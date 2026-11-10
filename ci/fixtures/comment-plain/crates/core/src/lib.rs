//! A crate with a plain comment: scripts/comment-rule.py must fail on it.

/// Public API, so this doc is allowed.
pub fn answer() -> u8 {
    // a plain comment
    42
}
