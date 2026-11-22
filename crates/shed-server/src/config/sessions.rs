//! Session, history and warm-spare settings.

use super::var;

/// How long shed keeps a session after its last turn: `$SHED_SESSIONS_KEEP` days of `shed serve`, else 30,
/// in seconds.
#[must_use]
pub fn sessions_keep() -> u64 {
    let days = var("SHED_SESSIONS_KEEP").and_then(|s| s.parse().ok());
    days.unwrap_or(30u64).saturating_mul(86_400)
}

/// How many of an agent's last sessions `agents` takes medians over: `$SHED_MEDIAN_OF`, else 20.
#[must_use]
pub fn median_of() -> usize {
    var("SHED_MEDIAN_OF")
        .and_then(|s| s.parse().ok())
        .unwrap_or(20)
}

/// How many spare ACP sessions `run --warm` keeps ready across recipes: `$SHED_WARM_MAX`, else 2.
#[must_use]
pub fn warm_max() -> usize {
    var("SHED_WARM_MAX")
        .and_then(|s| s.parse().ok())
        .unwrap_or(2)
}

/// How long an untaken spare session lives: `$SHED_WARM_IDLE` seconds, else 600.
#[must_use]
pub fn warm_idle() -> std::time::Duration {
    let secs = var("SHED_WARM_IDLE").and_then(|s| s.parse().ok());
    std::time::Duration::from_secs(secs.unwrap_or(600))
}
