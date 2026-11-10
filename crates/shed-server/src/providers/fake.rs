//! A test-only agent: a provider is this file plus its line in [`super::ALL`].

use super::Provider;

/// `fake-acp`.
pub struct Fake;

impl Provider for Fake {
    fn id(&self) -> &'static str {
        "fake-acp"
    }

    fn auth_failures(&self) -> &'static [&'static str] {
        &["fake: login gone"]
    }
}
