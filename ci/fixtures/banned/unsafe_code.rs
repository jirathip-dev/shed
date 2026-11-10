//! Breaks `unsafe_code`.

/// The first byte of a slice the caller keeps non-empty.
#[must_use]
pub fn first(v: &[u8]) -> u8 {
    unsafe { *v.get_unchecked(0) }
}
