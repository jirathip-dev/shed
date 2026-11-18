//! The one module that reads the environment (CI bans `env::var` elsewhere).

use std::ffi::OsString;

/// The variable `key`, when set to UTF-8.
#[must_use]
pub fn var(key: &str) -> Option<String> {
    std::env::var(key).ok()
}

/// The variable `key`, when set.
#[must_use]
pub fn var_os(key: &str) -> Option<OsString> {
    std::env::var_os(key)
}

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

/// How many of an agent's last sessions `agents` takes medians over: `$SHED_MEDIAN_OF`, else 20.
#[must_use]
pub fn median_of() -> usize {
    var("SHED_MEDIAN_OF")
        .and_then(|s| s.parse().ok())
        .unwrap_or(20)
}

/// How many spare ACP sessions `run --warm` keeps ready across recipes: `$SHED_WARM_MAX`, else 2.
#[must_use]
pub fn warm_max() -> usize {
    var("SHED_WARM_MAX")
        .and_then(|s| s.parse().ok())
        .unwrap_or(2)
}

/// How long an untaken spare session lives: `$SHED_WARM_IDLE` seconds, else 600.
#[must_use]
pub fn warm_idle() -> std::time::Duration {
    let secs = var("SHED_WARM_IDLE").and_then(|s| s.parse().ok());
    std::time::Duration::from_secs(secs.unwrap_or(600))
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
