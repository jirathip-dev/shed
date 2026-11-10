//! Process trees: find a terminal's processes, sample their memory and CPU, and end them.
//!
//! Each terminal's program leads its own session and process group, so the tree is the group plus
//! any descendants that left it. The process table comes from `/proc` on Linux and `ps` elsewhere
//! (macOS), behind [`ProcessTable`].

use std::collections::BTreeSet;
use std::io;
use std::time::{Duration, Instant};

use rustix::process::{Pid, Signal, kill_process, kill_process_group};

/// One live process, as the process table reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessInfo {
    /// Process id.
    pub pid: i32,
    /// Parent process id.
    pub ppid: i32,
    /// Process group id.
    pub pgid: i32,
    /// Resident memory in bytes.
    pub rss_bytes: u64,
    /// User plus system CPU time used so far.
    pub cpu: Duration,
    /// Exited but not yet reaped.
    pub zombie: bool,
}

/// A source of the system's process list.
pub trait ProcessTable {
    /// Every process visible to this user.
    ///
    /// # Errors
    /// When the process list cannot be read.
    fn list(&self) -> io::Result<Vec<ProcessInfo>>;
}

/// Linux: reads `/proc/<pid>/stat`.
#[cfg(target_os = "linux")]
pub struct Procfs;

#[cfg(target_os = "linux")]
impl ProcessTable for Procfs {
    fn list(&self) -> io::Result<Vec<ProcessInfo>> {
        let page = rustix::param::page_size() as u64;
        let ticks = rustix::param::clock_ticks_per_second().max(1);
        let mut out = Vec::new();
        for entry in std::fs::read_dir("/proc")? {
            let Ok(entry) = entry else { continue };
            if !entry
                .file_name()
                .to_string_lossy()
                .bytes()
                .all(|b| b.is_ascii_digit())
            {
                continue;
            }
            if let Ok(stat) = std::fs::read_to_string(entry.path().join("stat"))
                && let Some(p) = parse_stat(&stat, page, ticks)
            {
                out.push(p);
            }
        }
        Ok(out)
    }
}

#[cfg(any(target_os = "linux", test))]
fn parse_stat(stat: &str, page: u64, ticks: u64) -> Option<ProcessInfo> {
    let (pid, rest) = stat.split_once(" (")?;
    let fields: Vec<&str> = rest.rsplit_once(") ")?.1.split(' ').collect();
    let num = |i: usize| fields.get(i).and_then(|f| f.parse::<u64>().ok());
    let cpu_ticks = num(11)? + num(12)?;
    Some(ProcessInfo {
        pid: pid.trim().parse().ok()?,
        ppid: fields.get(1)?.parse().ok()?,
        pgid: fields.get(2)?.parse().ok()?,
        rss_bytes: num(21)? * page,
        cpu: Duration::from_millis(cpu_ticks * 1000 / ticks),
        zombie: fields.first() == Some(&"Z"),
    })
}

/// macOS and other Unixes: runs `ps`.
#[cfg(not(target_os = "linux"))]
pub struct Ps;

#[cfg(not(target_os = "linux"))]
impl ProcessTable for Ps {
    fn list(&self) -> io::Result<Vec<ProcessInfo>> {
        let out = std::process::Command::new("ps")
            .args(["-A", "-o", "pid=,ppid=,pgid=,rss=,time=,stat="])
            .env("LC_ALL", "C")
            .output()?;
        if !out.status.success() {
            return Err(io::Error::other(format!("ps failed: {}", out.status)));
        }
        Ok(String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(parse_ps)
            .collect())
    }
}

#[cfg(any(not(target_os = "linux"), test))]
fn parse_ps(line: &str) -> Option<ProcessInfo> {
    let f: Vec<&str> = line.split_whitespace().collect();
    let [id, parent, group, rss, time, stat] = f.as_slice() else {
        return None;
    };
    Some(ProcessInfo {
        pid: id.parse().ok()?,
        ppid: parent.parse().ok()?,
        pgid: group.parse().ok()?,
        rss_bytes: rss.parse::<u64>().ok()? * 1024,
        cpu: parse_cpu_time(time)?,
        zombie: stat.starts_with('Z'),
    })
}

#[cfg(any(not(target_os = "linux"), test))]
fn parse_cpu_time(s: &str) -> Option<Duration> {
    let (days, clock) = s
        .split_once('-')
        .map_or(Some((0, s)), |(d, c)| Some((d.parse::<u64>().ok()?, c)))?;
    let (clock, frac) = clock.split_once('.').unwrap_or((clock, "0"));
    let mut secs = 0u64;
    for part in clock.split(':') {
        secs = secs * 60 + part.parse::<u64>().ok()?;
    }
    let centis: u64 = format!("{frac:0<2}").get(..2)?.parse().ok()?;
    Some(Duration::from_secs(days * 86_400 + secs) + Duration::from_millis(centis * 10))
}

/// The platform's process table.
#[must_use]
pub fn system() -> impl ProcessTable {
    #[cfg(target_os = "linux")]
    return Procfs;
    #[cfg(not(target_os = "linux"))]
    return Ps;
}

/// A tree's summed resource use.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    /// Live processes in the tree.
    pub processes: usize,
    /// Summed resident memory in bytes.
    pub rss_bytes: u64,
    /// Summed CPU time of the live processes.
    pub cpu: Duration,
}

/// The live processes of the tree led by `root`: its process group, any descendants of those, and any
/// of `known` still alive (orphans that left the group).
#[must_use]
pub fn tree(procs: &[ProcessInfo], root: i32, known: &BTreeSet<i32>) -> Vec<ProcessInfo> {
    let live: Vec<&ProcessInfo> = procs.iter().filter(|p| !p.zombie).collect();
    let mut pids: BTreeSet<i32> = live
        .iter()
        .filter(|p| p.pid == root || p.pgid == root || known.contains(&p.pid))
        .map(|p| p.pid)
        .collect();
    loop {
        let before = pids.len();
        let children: Vec<i32> = live
            .iter()
            .filter(|p| pids.contains(&p.ppid))
            .map(|p| p.pid)
            .collect();
        pids.extend(children);
        if pids.len() == before {
            break;
        }
    }
    live.into_iter()
        .filter(|p| pids.contains(&p.pid))
        .cloned()
        .collect()
}

/// Sums the resource use of `procs`.
#[must_use]
pub fn usage(procs: &[ProcessInfo]) -> Usage {
    Usage {
        processes: procs.len(),
        rss_bytes: procs.iter().map(|p| p.rss_bytes).sum(),
        cpu: procs.iter().map(|p| p.cpu).sum(),
    }
}

fn signal_tree(root: i32, pids: &BTreeSet<i32>, sig: Signal) {
    if let Some(group) = Pid::from_raw(root) {
        let _ = kill_process_group(group, sig);
    }
    for pid in pids.iter().filter_map(|&p| Pid::from_raw(p)) {
        let _ = kill_process(pid, sig);
    }
}

/// Ends the tree led by `root`: SIGTERM to every process, then SIGKILL to what is left after `grace`.
///
/// `reap` is called while waiting so the caller can collect its exited child; the call returns once
/// no process of the tree is alive, or a second after SIGKILL.
///
/// # Errors
/// When the process table cannot be read.
pub fn kill_tree(
    table: &impl ProcessTable,
    root: i32,
    grace: Duration,
    mut reap: impl FnMut(),
) -> io::Result<()> {
    let mut known: BTreeSet<i32> = tree(&table.list()?, root, &BTreeSet::new())
        .iter()
        .map(|p| p.pid)
        .collect();
    signal_tree(root, &known, Signal::TERM);
    let mut killed = false;
    let mut deadline = Instant::now() + grace;
    loop {
        reap();
        let live = tree(&table.list()?, root, &known);
        if live.is_empty() {
            return Ok(());
        }
        known.extend(live.iter().map(|p| p.pid));
        if Instant::now() >= deadline {
            if killed {
                return Ok(());
            }
            signal_tree(root, &known, Signal::KILL);
            killed = true;
            deadline = Instant::now() + Duration::from_secs(1);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(id: i32, parent: i32, group: i32) -> ProcessInfo {
        ProcessInfo {
            pid: id,
            ppid: parent,
            pgid: group,
            rss_bytes: 1024,
            cpu: Duration::from_millis(10),
            zombie: false,
        }
    }

    #[test]
    fn tree_takes_the_group_descendants_and_known_orphans() {
        let procs = [
            p(1, 0, 1),
            p(10, 1, 10),
            p(11, 10, 10),
            p(12, 11, 12),
            p(13, 1, 13),
            p(14, 1, 14),
            p(15, 14, 15),
        ];
        let pids = |known: &[i32]| -> Vec<i32> {
            tree(&procs, 10, &known.iter().copied().collect())
                .iter()
                .map(|p| p.pid)
                .collect()
        };
        assert_eq!(pids(&[]), [10, 11, 12]);
        assert_eq!(pids(&[14]), [10, 11, 12, 14, 15]);
        let mut zombie = procs.to_vec();
        zombie[2].zombie = true;
        assert_eq!(tree(&zombie, 10, &BTreeSet::new()).len(), 1);
        assert_eq!(
            usage(&procs[1..3]),
            Usage {
                processes: 2,
                rss_bytes: 2048,
                cpu: Duration::from_millis(20)
            }
        );
    }

    #[test]
    fn parses_proc_stat() {
        let stat = "42 (a b) c) S 7 42 42 0 -1 0 0 0 0 0 150 50 0 0 20 0 1 0 100 1000 3 18446744073709551615";
        let info = parse_stat(stat, 4096, 100);
        assert_eq!(
            info,
            Some(ProcessInfo {
                pid: 42,
                ppid: 7,
                pgid: 42,
                rss_bytes: 3 * 4096,
                cpu: Duration::from_secs(2),
                zombie: false
            })
        );
    }

    #[test]
    fn parses_ps_lines() {
        let info = parse_ps("  42     7    42   1500   0:01.50 Ss+");
        assert_eq!(
            info,
            Some(ProcessInfo {
                pid: 42,
                ppid: 7,
                pgid: 42,
                rss_bytes: 1500 * 1024,
                cpu: Duration::from_millis(1500),
                zombie: false
            })
        );
        assert_eq!(parse_ps("1 0 1 0 0:00.00 Z").map(|p| p.zombie), Some(true));
        assert_eq!(parse_cpu_time("01:02:03"), Some(Duration::from_secs(3723)));
        assert_eq!(
            parse_cpu_time("2-00:00:01"),
            Some(Duration::from_secs(172_801))
        );
        assert_eq!(parse_ps("garbage"), None);
    }
}
