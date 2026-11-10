mod claude;
#[cfg(test)]
mod fake;

pub trait Provider: Sync {
    fn id(&self) -> &'static str;
    fn auth_failures(&self) -> &'static [&'static str];
}

static ALL: &[&dyn Provider] = &[
    &claude::Claude,
    #[cfg(test)]
    &fake::Fake,
];

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
