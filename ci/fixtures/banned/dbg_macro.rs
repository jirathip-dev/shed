//! Breaks `dbg_macro`.

/// One, shown.
#[must_use]
pub fn one() -> u8 {
    dbg!(1)
}
