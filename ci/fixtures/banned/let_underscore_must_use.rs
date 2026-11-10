//! Breaks `let_underscore_must_use`.

/// Parses and drops it.
pub fn drop_it() {
    let _ = "1".parse::<u8>();
}
