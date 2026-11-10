//! Every comment the rule allows: scripts/comment-rule.py must pass on it.

/// A public function.
#[must_use]
pub fn url() -> &'static str {
    // TODO(#65)
    "https://example.com/a//b /* not a comment */"
}

/// A public enum.
pub enum Shape {
    /// A variant of a public enum is public API.
    Square {
        /// So is its field.
        side: u8,
    },
}

/// A public trait.
pub trait Named {
    /// A trait item is public API.
    fn name(&self) -> String;
}

/// A public struct.
pub struct Point {
    /// A public field.
    pub x: u8,
}

impl Point {
    /// A public method.
    #[must_use]
    pub fn first(bytes: &[u8]) -> u8 {
        // SAFETY: the caller passes a non-empty slice.
        bytes[0]
    }
}
