//! The one module that reads the clock or sleeps (CI bans `Instant::now`, `SystemTime::now` and
//! `thread::sleep` elsewhere).

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Now, on the monotonic clock.
#[must_use]
pub fn now() -> Instant {
    Instant::now()
}

/// Now, on tokio's clock, which a paused test runtime controls.
#[must_use]
pub fn tokio_now() -> tokio::time::Instant {
    tokio::time::Instant::now()
}

/// Whole seconds since the Unix epoch; 0 for a clock set before it.
#[must_use]
pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Blocks the calling thread for `d`.
pub fn sleep(d: Duration) {
    std::thread::sleep(d);
}
