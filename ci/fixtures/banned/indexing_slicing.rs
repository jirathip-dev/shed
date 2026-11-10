//! Breaks `indexing_slicing`.

/// The first byte.
#[must_use]
pub fn first(v: &[u8]) -> u8 {
    v[0]
}
