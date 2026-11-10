//! Breaks `as_conversions`.

/// The byte as a character.
#[must_use]
pub fn letter(b: u8) -> char {
    b as char
}
