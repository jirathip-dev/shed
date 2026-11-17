//! What shed knows about each agent, as data: the built-in `providers/<name>.toml` files, each overridden
//! key by key by `<name>.toml` in the user's providers folder, which can also add agents.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use toml::Table;

use crate::Error;
use crate::proto::{AgentInfo, Provider};

const BUILT_IN: &[(&str, &str)] = include!(concat!(env!("OUT_DIR"), "/providers.rs"));

fn parse(text: &str) -> Result<Table, String> {
    text.parse().map_err(|e: toml::de::Error| e.to_string())
}

fn record(table: &Table) -> Result<Provider, String> {
    table
        .clone()
        .try_into()
        .map_err(|e: toml::de::Error| e.to_string())
}

/// The provider records with where each came from (`built-in` or the user file), and an error for each
/// user file in `dir` that is unreadable, not TOML, or not a record; those are skipped.
#[must_use]
pub fn load(dir: &Path) -> (Vec<(Provider, String)>, Vec<String>) {
    let built_in = BUILT_IN.iter().filter_map(|(name, text)| {
        let source = "built-in".to_owned();
        Some(((*name).to_owned(), (parse(text).ok()?, source)))
    });
    let mut tables: BTreeMap<String, (Table, String)> = built_in.collect();
    let mut bad = Vec::new();
    let mut files: Vec<_> = fs::read_dir(dir).into_iter().flatten().flatten().collect();
    files.sort_by_key(fs::DirEntry::path);
    for path in files.iter().map(fs::DirEntry::path) {
        let Some(name) = path
            .file_stem()
            .filter(|_| path.extension() == Some("toml".as_ref()))
        else {
            continue;
        };
        let name = name.to_string_lossy().into_owned();
        let read = fs::read_to_string(&path).map_err(|e| e.to_string());
        let merged = read.and_then(|t| parse(&t)).and_then(|user| {
            let mut table = tables.get(&name).map(|t| t.0.clone()).unwrap_or_default();
            table.extend(user);
            record(&table).map(|_| table)
        });
        match merged {
            Ok(table) => {
                tables.insert(name, (table, path.display().to_string()));
            }
            Err(e) => bad.push(format!("{}: {e}", path.display())),
        }
    }
    let records = tables.into_values();
    let records = records.filter_map(|(t, source)| Some((record(&t).ok()?, source)));
    (records.collect(), bad)
}

/// Agent `id`'s record (with no rules when it has none).
#[must_use]
pub fn find(dir: &Path, id: &str) -> Provider {
    let found = load(dir).0.into_iter().find(|(p, _)| p.id == id);
    found.map(|(p, _)| p).unwrap_or_default()
}

/// Each agent with a record in `dir` or installed beside the state file `state`, by id.
///
/// # Errors
/// [`crate::agents::list`]'s.
pub fn known(state: &Path, dir: &Path) -> Result<Vec<AgentInfo>, Error> {
    let mut all: BTreeMap<_, _> = (load(dir).0.into_iter())
        .map(|(p, source)| (p.id.clone(), (Some(p), Some(source), None)))
        .collect();
    for agent in crate::agents::list(state)? {
        let slot = all.entry(agent.id.clone()).or_default();
        slot.2 = Some(agent);
    }
    let info = |(id, (provider, source, installed))| AgentInfo {
        id,
        source,
        provider,
        installed,
    };
    Ok(all.into_iter().map(info).collect())
}

#[cfg(test)]
pub(crate) mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::{find, load};
    use crate::proto::{Provider, Tokens};

    pub(crate) fn fake_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/providers")
    }

    pub(crate) fn fake(id: &str) -> Provider {
        find(&fake_dir(), id)
    }

    #[test]
    fn a_provider_is_one_data_file_and_no_rust() {
        let (records, bad) = load(&fake_dir());
        assert!(bad.is_empty(), "{bad:?}");
        let file = fake_dir().join("fake.toml").display().to_string();
        let sources: Vec<(&str, &str)> = records
            .iter()
            .map(|(p, s)| (p.id.as_str(), s.as_str()))
            .collect();
        assert_eq!(
            sources,
            [
                ("claude-acp", "built-in"),
                ("codex-acp", "built-in"),
                ("fake-acp", file.as_str())
            ]
        );
        let p = fake("fake-acp");
        assert_eq!(
            p.auth_failure("ok\nfake: login gone\n"),
            Some("fake: login gone")
        );
        assert_eq!(
            p.hooks.map(|h| h.file),
            Some(".fake/settings.json".to_owned())
        );
        assert!(
            find(&fake_dir(), "claude-acp")
                .auth_failures
                .contains(&"OAuth session expired".to_owned())
        );
        assert_eq!(find(&fake_dir(), "unknown-acp"), Provider::default());
    }

    #[test]
    fn the_limit_messages_give_their_reset_time() {
        let hit = |id: &str, text: &str| fake(id).limit_hit(text).map(|(_, at)| at);
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

    #[test]
    fn claude_turn_tokens_are_read_from_the_quota_meta_and_a_malformed_one_is_none() {
        let claude = fake("claude-acp");
        let meta = |counts: serde_json::Value| {
            let meta = serde_json::json!({"quota": {"token_count": counts}});
            meta.as_object().cloned().unwrap_or_default()
        };
        let counts = serde_json::json!({"inputTokens": 3, "cachedInputTokens": 90,
            "cachedWriteTokens": 7, "outputTokens": 5});
        let tokens = claude.tokens(&meta(counts.clone()));
        assert_eq!(
            tokens,
            Some(Tokens {
                input: 3,
                cache_read: 90,
                cache_write: 7,
                output: 5
            })
        );
        assert_eq!(tokens.and_then(|t| t.cache_share()), Some(90));
        let mut bad = counts.clone();
        bad["outputTokens"] = "5".into();
        assert_eq!(claude.tokens(&meta(bad)), None);
        assert_eq!(claude.tokens(&meta(serde_json::json!([1]))), None);
        assert_eq!(claude.tokens(&serde_json::Map::new()), None);
        assert_eq!(fake("codex-acp").tokens(&meta(counts)), None);
    }

    #[test]
    fn an_api_login_cache_is_likely_cold_after_five_idle_minutes_a_plan_one_after_sixty() {
        let claude = fake("claude-acp");
        let api = claude.cache_lifetime(|v| v == "ANTHROPIC_API_KEY");
        let plan = claude.cache_lifetime(|_| false);
        assert_eq!((api, plan), (Some(300), Some(3600)));
        let warm = |lifetime, idle: u64| {
            let session = crate::proto::AcpSession {
                turn_ended: Some(1000),
                cache_lifetime: lifetime,
                ..Default::default()
            };
            session.cache_warm(1000 + idle)
        };
        assert_eq!((warm(api, 299), warm(api, 300)), (Some(true), Some(false)));
        assert_eq!(
            (warm(plan, 3599), warm(plan, 3600)),
            (Some(true), Some(false))
        );
        assert_eq!(fake("codex-acp").cache_lifetime(|_| true), None);
        assert_eq!(warm(None, 0), None);
    }

    #[test]
    fn a_user_file_overrides_its_keys_and_a_bad_one_is_skipped() {
        let dir = crate::temp_dir("providers");
        let banner = "limit = [\n  \"out of tokens\"\n]\n# a comment\nresets = [\"until \"]\n";
        fs::write(dir.join("claude.toml"), banner).expect("write");
        fs::write(dir.join("codex.toml"), "limit = \"not a list\"\n").expect("write");
        fs::write(
            dir.join("new.toml"),
            "id = \"new-acp\"\n[hooks]\nfile = 1\n",
        )
        .expect("write");
        fs::write(dir.join("notes.txt"), "not a record").expect("write");
        let (records, bad) = load(&dir);
        let claude = find(&dir, "claude-acp");
        assert_eq!(
            claude.limit_hit("you are out of tokens until 5pm."),
            Some(("you are out of tokens until 5pm.", Some("5pm".to_owned())))
        );
        assert_eq!(claude.limit_hit("You've hit your limit"), None);
        assert_eq!(
            claude.auth_failure("OAuth session expired"),
            Some("OAuth session expired")
        );
        assert_eq!(find(&dir, "codex-acp").limit, ["hit your usage limit"]);
        assert_eq!(records.len(), 2, "new-acp is skipped");
        assert_eq!(bad.len(), 2, "{bad:?}");
        assert!(
            bad[0].starts_with(&dir.join("codex.toml").display().to_string()),
            "{bad:?}"
        );
        assert!(bad[1].contains("new.toml"), "{bad:?}");
    }
}
