//! Terminal size, memory and lifetime settings.

use super::var;

/// `RxC` (rows by columns, each at least 1) as `(rows, cols)`.
#[must_use]
pub fn size(s: &str) -> Option<(u16, u16)> {
    let (rows, cols) = s.split_once('x')?;
    let size = (rows.parse().ok()?, cols.parse().ok()?);
    (size.0 > 0 && size.1 > 0).then_some(size)
}

/// The size of a new terminal without `--size`: `$SHED_SIZE` (`RxC`) of `shed serve`, else 50x200.
#[must_use]
pub fn default_size() -> (u16, u16) {
    var("SHED_SIZE")
        .as_deref()
        .and_then(size)
        .unwrap_or((50, 200))
}

/// How long an exited terminal stays listed: `$SHED_PRUNE_AFTER` seconds of `shed serve`, else a day.
#[must_use]
pub fn prune_after() -> u64 {
    var("SHED_PRUNE_AFTER")
        .and_then(|s| s.parse().ok())
        .unwrap_or(86_400)
}

/// How long `shed state` waits for the server's answer: `$SHED_STATE_TIMEOUT` seconds, else 2.
#[must_use]
pub fn state_timeout() -> std::time::Duration {
    let secs = var("SHED_STATE_TIMEOUT").and_then(|s| s.parse().ok());
    let deadline = secs.and_then(|s| std::time::Duration::try_from_secs_f64(s).ok());
    let deadline = deadline.filter(|d| !d.is_zero());
    deadline.unwrap_or(std::time::Duration::from_secs(2))
}

/// A memory size, `<n>M`, `<n>G` or `<n>T` (1024-based), in MB; at least 1M.
#[must_use]
pub fn mem_mb(s: &str) -> Option<u64> {
    let at = s.len().checked_sub(1)?;
    let (n, unit) = (s.get(..at)?.parse::<u64>().ok()?, s.get(at..)?);
    let shift = ["M", "G", "T"]
        .iter()
        .position(|u| u.eq_ignore_ascii_case(unit))?;
    n.checked_shl(u32::try_from(shift * 10).ok()?)
        .filter(|&mb| mb > 0 && mb >> (shift * 10) == n)
}

/// The memory limit of a terminal run without one: `$SHED_MEM_MAX` of `shed serve` (`200M`, `4G`), else none.
#[must_use]
pub fn mem_max() -> Option<u64> {
    var("SHED_MEM_MAX").as_deref().and_then(mem_mb)
}
