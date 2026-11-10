//! Breaks `unwrap_used`.

/// One.
///
/// # Panics
/// Never.
#[must_use]
pub fn one() -> u8 {
    "1".parse().unwrap()
}
