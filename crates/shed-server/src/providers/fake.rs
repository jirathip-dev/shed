use super::{Hooks, Provider};

pub struct Fake;

static HOOKS: Hooks = Hooks {
    agent: "fake",
    file: ".fake/settings.json",
    events: &[("Prompt", "working"), ("Stop", "done")],
};

impl Provider for Fake {
    fn id(&self) -> &'static str {
        "fake-acp"
    }

    fn auth_failures(&self) -> &'static [&'static str] {
        &["fake: login gone"]
    }

    fn hooks(&self) -> Option<&'static Hooks> {
        Some(&HOOKS)
    }
}
