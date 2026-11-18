//! A terminal's memory limit: its own systemd scope with `MemoryMax` where the user's systemd has the
//! memory controller (Linux), else the sampler's guard.

use std::sync::OnceLock;

/// Whether this host starts each limited terminal in a scope of its own; probed once.
#[must_use]
pub fn scoped() -> bool {
    static FOUND: OnceLock<bool> = OnceLock::new();
    *FOUND.get_or_init(|| {
        let check = r#"cat "/sys/fs/cgroup$(cut -d: -f3 /proc/self/cgroup)/memory.max""#;
        let args = in_scope(&["sh".to_owned(), "-c".to_owned(), check.to_owned()], 1024);
        cfg!(target_os = "linux")
            && crate::runner::output("systemd-run", args.iter().skip(1), &[])
                .is_ok_and(|o| o.stdout.trim_ascii() == b"1073741824")
    })
}

/// How this host limits a terminal's memory, for `shed doctor`.
#[must_use]
pub fn mechanism() -> &'static str {
    if scoped() {
        "a systemd scope per terminal (MemoryMax, the kernel ends a process)"
    } else {
        "the sampler (two samples over ends the largest process the agent started)"
    }
}

pub(crate) fn in_scope(cmd: &[String], mb: u64) -> Vec<String> {
    let max = format!("MemoryMax={mb}M");
    let systemd = "systemd-run --user --scope --quiet -p OOMPolicy=continue -p MemorySwapMax=0 -p";
    let systemd = systemd.split(' ').map(str::to_owned);
    let max = [max, "--".to_owned()];
    systemd.chain(max).chain(cmd.iter().cloned()).collect()
}

pub(crate) fn oom_kills(pid: i32) -> Option<u64> {
    let cgroup = std::fs::read_to_string(format!("/proc/{pid}/cgroup")).ok()?;
    let path = cgroup.lines().find_map(|l| l.strip_prefix("0::"))?;
    let scope = std::path::Path::new(path).extension() == Some("scope".as_ref());
    let path = scope.then_some(path)?;
    let events = std::fs::read_to_string(format!("/sys/fs/cgroup{path}/memory.events")).ok()?;
    let kills = events.lines().find_map(|l| l.strip_prefix("oom_kill "))?;
    kills.parse().ok()
}

pub(crate) fn kill_first(pid: i32) {
    std::fs::write(format!("/proc/{pid}/oom_score_adj"), "1000").ok();
}
