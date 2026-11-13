mod claude;
mod codex;
#[cfg(test)]
mod fake;

pub trait Provider: Sync {
    fn id(&self) -> &'static str;
    fn auth_failures(&self) -> &'static [&'static str];
    fn hooks(&self) -> Option<&'static Hooks> {
        None
    }
    fn limit(&self) -> Option<&'static Limit> {
        None
    }
}

pub struct Limit {
    pub says: &'static [&'static str],
    pub resets: &'static [&'static str],
}

#[derive(Debug, PartialEq, Eq)]
pub struct Limited {
    pub resets: Option<String>,
}

impl Limit {
    #[must_use]
    pub fn hit(&self, text: &str) -> Option<Limited> {
        let line = text
            .lines()
            .find(|l| self.says.iter().any(|s| l.contains(s)))?;
        let reset = self.resets.iter().find_map(|r| line.split_once(r));
        let resets = reset.map(|(_, at)| at.trim().trim_end_matches('.').to_owned());
        Some(Limited { resets })
    }
}

pub struct Hooks {
    pub agent: &'static str,
    pub file: &'static str,
    pub events: &'static [(&'static str, &'static str)],
}

static ALL: &[&dyn Provider] = &[
    &claude::Claude,
    &codex::Codex,
    #[cfg(test)]
    &fake::Fake,
];

#[must_use]
pub fn auth_failures(id: &str) -> &'static [&'static str] {
    ALL.iter()
        .find(|p| p.id() == id)
        .map_or(&[], |p| p.auth_failures())
}

#[must_use]
pub fn limit(id: &str) -> Option<&'static Limit> {
    ALL.iter().find(|p| p.id() == id).and_then(|p| p.limit())
}

pub fn hook_tables() -> impl Iterator<Item = &'static Hooks> {
    ALL.iter().filter_map(|p| p.hooks())
}

#[cfg(test)]
mod tests {
    use super::{auth_failures, limit};

    #[test]
    fn a_provider_is_one_file_and_one_registry_line() {
        assert_eq!(auth_failures("fake-acp"), ["fake: login gone"]);
        assert!(auth_failures("claude-acp").contains(&"OAuth session expired"));
        assert!(auth_failures("unknown-acp").is_empty());
    }

    #[test]
    fn the_limit_messages_give_their_reset_time() {
        let hit = |id: &str, text: &str| limit(id).and_then(|l| l.hit(text)).map(|l| l.resets);
        let claude = "You've hit your limit · resets 3pm (Europe/London)";
        assert_eq!(
            hit("claude-acp", claude),
            Some(Some("3pm (Europe/London)".into()))
        );
        let codex = "You've hit your usage limit. Upgrade to Pro (https://chatgpt.com/explore/pro) \
            or try again at Nov 14th, 2026 3:05 PM.";
        assert_eq!(
            hit("codex-acp", codex),
            Some(Some("Nov 14th, 2026 3:05 PM".into()))
        );
        let bare = "You've hit your usage limit.";
        assert_eq!(hit("codex-acp", bare), Some(None));
        assert_eq!(hit("claude-acp", "all tests pass"), None);
        assert_eq!(hit("unknown-acp", claude), None);
    }
}
