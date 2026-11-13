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
