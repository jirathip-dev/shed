//! Agent specifics, one module per agent behind [`Provider`]: core code names no agent (#64,
//! `scripts/neutral-core.sh`). Adding one is a new file here plus one line in [`ALL`].

mod claude;
#[cfg(test)]
mod fake;

/// What shed knows about one agent beyond its ACP registry entry.
pub trait Provider: Sync {
    /// The ACP registry id it covers.
    fn id(&self) -> &'static str;
    /// Patterns that mark a reply or error line as an auth failure: the turn ends as `error`.
    fn auth_failures(&self) -> &'static [&'static str];
}

/// Every provider, one line each.
static ALL: &[&dyn Provider] = &[
    &claude::Claude,
    #[cfg(test)]
    &fake::Fake,
];

/// Agent `id`'s auth-failure patterns; none for an agent with no provider.
#[must_use]
pub fn auth_failures(id: &str) -> &'static [&'static str] {
    ALL.iter()
        .find(|p| p.id() == id)
        .map_or(&[], |p| p.auth_failures())
}

#[cfg(test)]
mod tests {
    use super::auth_failures;

    #[test]
    fn a_provider_is_one_file_and_one_registry_line() {
        assert_eq!(auth_failures("fake-acp"), ["fake: login gone"]);
        assert!(auth_failures("claude-acp").contains(&"OAuth session expired"));
        assert!(auth_failures("unknown-acp").is_empty());
    }
}
