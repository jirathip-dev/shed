use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};

use shed_core::clock::unix_now;
use shed_proto::Sample;

const EVERY: u64 = 60;
const DAY: u64 = 86_400;

fn stdout<A: AsRef<std::ffi::OsStr>>(program: &str, args: impl IntoIterator<Item = A>) -> String {
    shed_core::runner::output(program, args, &[])
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

pub(crate) fn system(dir: &Path) -> Sample {
    let cpus =
        std::thread::available_parallelism().map_or(1, |n| u32::try_from(n.get()).unwrap_or(1));
    let (load, mem_pressure_pct, swap_used_mb, uptime_s) = probe();
    let df = stdout("df", [Path::new("-Pk"), dir]);
    let free_kb = df.lines().nth(1).and_then(|l| l.split_whitespace().nth(3));
    Sample {
        at: unix_now(),
        load_per_cpu: load / f64::from(cpus),
        mem_pressure_pct,
        swap_used_mb,
        disk_free_mb: free_kb.and_then(|v| v.parse().ok()).unwrap_or(0) / 1024,
        cpus: cpus.into(),
        uptime_s,
        terminals: std::collections::BTreeMap::new(),
    }
}

#[cfg(target_os = "linux")]
fn probe() -> (f64, u64, u64, u64) {
    let read = |p: &str| fs::read_to_string(p).unwrap_or_default();
    let meminfo = read("/proc/meminfo");
    let kb = |key: &str| {
        let line = meminfo
            .lines()
            .find_map(|l| l.strip_prefix(key)?.strip_prefix(':'));
        let value = line.and_then(|l| l.split_whitespace().next()?.parse::<u64>().ok());
        value.unwrap_or(0)
    };
    let total = kb("MemTotal").max(1);
    let uptime = read("/proc/uptime");
    (
        read("/proc/loadavg")
            .split(' ')
            .next()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.0),
        100 - kb("MemAvailable").min(total) * 100 / total,
        kb("SwapTotal").saturating_sub(kb("SwapFree")) / 1024,
        uptime
            .split(['.', ' '])
            .next()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0),
    )
}

#[cfg(not(target_os = "linux"))]
fn probe() -> (f64, u64, u64, u64) {
    let keys = [
        "vm.loadavg",
        "kern.memorystatus_level",
        "vm.swapusage",
        "kern.boottime",
    ];
    parse_sysctl(
        &stdout("sysctl", std::iter::once("-n").chain(keys)),
        unix_now(),
    )
}

#[cfg(any(not(target_os = "linux"), test))]
fn parse_sysctl(text: &str, now: u64) -> (f64, u64, u64, u64) {
    let mut lines = text.lines();
    let mut next = || lines.next().unwrap_or_default();
    let load = next()
        .split_whitespace()
        .nth(1)
        .and_then(|v| v.parse().ok());
    let free = next().trim().parse::<u64>().map_or(100, |f| f.min(100));
    let after = |line: &str, key: &str, end: char| {
        let value = line.split(key).nth(1)?.split(end).next()?;
        value.parse::<u64>().ok()
    };
    let swap = after(next(), "used = ", '.');
    let boot = after(next(), "sec = ", ',');
    (
        load.unwrap_or(0.0),
        100 - free,
        swap.unwrap_or(0),
        boot.map_or(0, |b| now.saturating_sub(b)),
    )
}

pub(crate) struct Stats {
    samples: VecDeque<Sample>,
    path: PathBuf,
}

impl Stats {
    pub(crate) fn load(path: &Path) -> Self {
        let saved = fs::read(path).ok();
        Self {
            samples: saved
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or_default(),
            path: path.to_owned(),
        }
    }

    pub(crate) fn until_due(&self, now: u64) -> u64 {
        self.samples
            .back()
            .map_or(0, |s| (s.at + EVERY).saturating_sub(now))
    }

    pub(crate) fn push(&mut self, sample: Sample) {
        let cutoff = sample.at.saturating_sub(DAY);
        self.samples.push_back(sample);
        while self.samples.front().is_some_and(|s| s.at <= cutoff) {
            self.samples.pop_front();
        }
        let tmp = self.path.with_extension("json.tmp");
        if serde_json::to_vec(&self.samples).is_ok_and(|b| fs::write(&tmp, b).is_ok()) {
            fs::rename(&tmp, &self.path).ok();
        }
    }

    pub(crate) fn since(&self, since: u64) -> Vec<Sample> {
        self.samples
            .iter()
            .filter(|s| s.at > since)
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_sample_a_minute_on_a_fake_clock_and_24_hours_kept() {
        let path = crate::temp_dir("stats").join("stats.json");
        let mut stats = Stats::load(&path);
        let t0 = 1_762_000_000;
        for now in (t0..t0 + 25 * 3600).step_by(10) {
            if stats.until_due(now) == 0 {
                stats.push(Sample {
                    at: now,
                    ..Sample::default()
                });
            }
        }
        let kept = stats.since(0);
        assert_eq!(kept.len(), 1_440);
        assert!(kept.windows(2).all(|w| w[1].at - w[0].at == 60));
        let last = kept[1_439].at;
        assert_eq!(last, t0 + 25 * 3600 - 60);
        assert_eq!(kept[0].at, last - 1_439 * 60);
        assert_eq!(stats.since(last - 600).len(), 10);
        assert_eq!(Stats::load(&path).since(0), kept, "saved and loaded back");
    }

    #[test]
    fn parses_macos_sysctl() {
        let text = "{ 3.20 2.10 1.90 }\n58\ntotal = 2048.00M  used = 1024.50M  free = 1023.50M  (encrypted)\n\
                    { sec = 1762000000, usec = 5 } Thu Nov  6 10:00:00 2026\n";
        assert_eq!(parse_sysctl(text, 1_762_003_600), (3.2, 42, 1024, 3600));
        assert_eq!(parse_sysctl("", 5), (0.0, 0, 0, 0));
    }
}
