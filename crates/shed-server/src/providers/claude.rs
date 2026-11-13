use super::{Hooks, Limit, Provider};

pub struct Claude;

static LIMIT: Limit = Limit {
    says: &["hit your limit", "usage limit reached"],
    resets: &["resets "],
};

static HOOKS: Hooks = Hooks {
    agent: "claude",
    file: ".claude/settings.json",
    events: &[
        ("UserPromptSubmit", "working"),
        ("PostToolUse", "working"),
        ("PermissionRequest", "blocked"),
        ("Stop", "done"),
        ("StopFailure", "error"),
    ],
};

impl Provider for Claude {
    fn id(&self) -> &'static str {
        "claude-acp"
    }

    fn auth_failures(&self) -> &'static [&'static str] {
        &["Failed to authenticate", "OAuth session expired"]
    }

    fn hooks(&self) -> Option<&'static Hooks> {
        Some(&HOOKS)
    }

    fn limit(&self) -> Option<&'static Limit> {
        Some(&LIMIT)
    }
}
