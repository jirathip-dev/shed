//! A crate with a doc comment on a private item: scripts/comment-rule.py must fail on it.

/// Public API, so this doc is allowed.
pub fn answer() -> u8 {
    half() * 2
}

/// A private item, so this doc breaks the rule.
fn half() -> u8 {
    21
}
