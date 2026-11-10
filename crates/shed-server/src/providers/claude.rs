use super::Provider;

pub struct Claude;

impl Provider for Claude {
    fn id(&self) -> &'static str {
        "claude-acp"
    }

    fn auth_failures(&self) -> &'static [&'static str] {
        &["Failed to authenticate", "OAuth session expired"]
    }
}
