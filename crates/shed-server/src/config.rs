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
