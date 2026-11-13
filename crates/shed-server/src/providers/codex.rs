use super::{Limit, Provider};

pub struct Codex;

static LIMIT: Limit = Limit {
    says: &["hit your usage limit"],
    resets: &[
        "try again at ",
        "Try again at ",
        "try again in ",
        "Try again in ",
    ],
};

impl Provider for Codex {
    fn id(&self) -> &'static str {
        "codex-acp"
    }

    fn auth_failures(&self) -> &'static [&'static str] {
        &[]
    }

    fn limit(&self) -> Option<&'static Limit> {
        Some(&LIMIT)
    }
}
