//! What shed knows about each agent: the records ([`Catalog`], the built-in ones under the user's providers
//! folder), the agents installed and their timings, and the line formats of the stream driver,
//! `providers/<wire>.rs`.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Error;
use crate::drivers::stream::Wire;
use crate::proto::{AgentInfo, EventKind, Medians, Provider};
pub use shed_catalog::Catalog;
use std::time::UNIX_EPOCH;

type MakeWire = fn() -> Box<dyn Wire>;
include!(concat!(env!("OUT_DIR"), "/wires.rs"));

pub(crate) fn wire(name: &str) -> Option<Box<dyn Wire>> {
    WIRES.iter().find(|w| w.0 == name).map(|w| (w.1)())
}

#[derive(Serialize, Deserialize)]
struct Timed {
    agent: String,
    ready_ms: u64,
    first_token_ms: u64,
}

fn timings(state: &Path) -> PathBuf {
    state.with_file_name("timings.jsonl")
}

pub(crate) fn record_timing(
    state: &Path,
    agent: &str,
    ready_ms: u64,
    first_token_ms: u64,
) -> std::io::Result<()> {
    let agent = agent.to_owned();
    let mut line = serde_json::to_string(&Timed {
        agent,
        ready_ms,
        first_token_ms,
    })?;
    line.push('\n');
    let mut file = (fs::OpenOptions::new().create(true).append(true)).open(timings(state))?;
    file.write_all(line.as_bytes())
}

fn median(mut values: Vec<u64>) -> u64 {
    values.sort_unstable();
    let mid = values.len() / 2;
    let high = values.get(mid).copied().unwrap_or_default();
    let low = (values.len().is_multiple_of(2))
        .then(|| mid.checked_sub(1).and_then(|i| values.get(i)).copied())
        .flatten();
    low.map_or(high, |low| low.midpoint(high))
}

fn medians(state: &Path) -> BTreeMap<String, Medians> {
    let text = fs::read_to_string(timings(state)).unwrap_or_default();
    let mut by_agent: BTreeMap<String, Vec<(u64, u64)>> = BTreeMap::new();
    for t in text
        .lines()
        .filter_map(|l| serde_json::from_str::<Timed>(l).ok())
    {
        (by_agent.entry(t.agent).or_default()).push((t.ready_ms, t.first_token_ms));
    }
    let n = crate::config::median_of();
    let of = |all: Vec<(u64, u64)>| {
        let last: Vec<_> = all.into_iter().rev().take(n).collect();
        (!last.is_empty()).then(|| Medians {
            sessions: u64::try_from(last.len()).unwrap_or(u64::MAX),
            ready_ms: median(last.iter().map(|t| t.0).collect()),
            first_token_ms: median(last.iter().map(|t| t.1).collect()),
        })
    };
    (by_agent.into_iter())
        .filter_map(|(id, all)| Some((id, of(all)?)))
        .collect()
}

/// Each agent with a record in `dir` or installed beside the state file `state`, by id.
///
/// # Errors
/// [`crate::agents::list`]'s.
pub fn known(state: &Path, dir: &Path) -> Result<Vec<AgentInfo>, Error> {
    let catalog = Catalog::load(dir).0;
    let mut all: BTreeMap<_, _> = (catalog.iter())
        .map(|e| {
            (
                e.provider.id.clone(),
                (Some(e.provider.clone()), Some(e.source.clone()), None),
            )
        })
        .collect();
    for agent in crate::agents::list(state)? {
        let slot = all.entry(agent.id.clone()).or_default();
        slot.2 = Some(agent);
    }
    let mut medians = medians(state);
    let info = |(id, (provider, source, installed))| AgentInfo {
        medians: medians.remove(&id),
        id,
        source,
        provider,
        installed,
    };
    Ok(all.into_iter().map(info).collect())
}

/// Where a run's agent keeps its login: the record's `config_dir_var` in `vars`, else its `default_folder`.
#[must_use]
pub fn config_folder(record: &Provider, vars: &[(String, String)]) -> Option<PathBuf> {
    let var = record.config_dir_var.as_ref();
    let set = var.and_then(|v| vars.iter().rev().find(|(k, _)| k == v));
    let default = record.default_folder.as_deref().map(|d| {
        d.strip_prefix("~/")
            .map_or_else(|| d.into(), |rest| crate::accounts::home().join(rest))
    });
    set.map(|(_, v)| PathBuf::from(v)).or(default)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::path::{Path, PathBuf};

    use super::Catalog;
    use crate::proto::{Provider, Tokens};

    pub(crate) fn fake_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/providers")
    }

    pub(crate) fn fake(id: &str) -> Provider {
        Catalog::load(&fake_dir())
            .0
            .by_id(id)
            .cloned()
            .unwrap_or_default()
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
}
