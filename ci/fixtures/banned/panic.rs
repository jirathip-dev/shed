//! Breaks `panic`.

/// Never returns.
///
/// # Panics
/// Always.
pub fn stop() {
    panic!("stop");
}
