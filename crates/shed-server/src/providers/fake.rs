use super::Provider;

pub struct Fake;

impl Provider for Fake {
    fn id(&self) -> &'static str {
        "fake-acp"
    }

    fn auth_failures(&self) -> &'static [&'static str] {
        &["fake: login gone"]
    }
}
