//! Breaks `allow_attributes_without_reason`.

/// Nothing.
#[allow(clippy::needless_return)]
pub fn nothing() {
    return;
}
