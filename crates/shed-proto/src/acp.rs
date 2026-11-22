//! An ACP terminal's session, usage and the methods its bridge calls.

use std::path::PathBuf;

#[cfg(feature = "schema")]
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::EventKind;

/// An ACP terminal's agent session, as its bridge last reported it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct AcpSession {
    /// The agent's session id, for `run --acp … --resume`.
    pub id: String,
    /// How the lane talks to its agent: `acp`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub driver: String,
    /// The current model, when the agent offers models.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The models `acp.model` switches to: id and name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<[String; 2]>,
    /// The `run`'s `mcp_servers`, each env and header value as `***`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcp_servers: Vec<Value>,
    /// The agent's slash commands (a prompt `/name …` runs one): name, description, input hint.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<[String; 3]>,
    /// The agent's last `usage_update`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<AcpUsage>,
    /// When the last turn ended, unix seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_ended: Option<u64>,
    /// The prompt cache's likely lifetime on this login in seconds, from the provider record's `cache`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_lifetime: Option<u64>,
    /// While idle, `warm` or `cold`: what the prompt cache likely is, by the idle time against `cache_lifetime`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_likely: Option<String>,
    /// Milliseconds from `run` to the session being ready.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ready_ms: Option<u64>,
    /// Milliseconds from the first prompt to the first chunk of its reply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_token_ms: Option<u64>,
}

impl AcpSession {
    /// Whether the prompt cache is likely still warm at `now` (unix seconds), or `None` with no turn or lifetime.
    #[must_use]
    pub fn cache_warm(&self, now: u64) -> Option<bool> {
        Some(now.saturating_sub(self.turn_ended?) < self.cache_lifetime?)
    }
}

/// A session shed ran, `sessions/<id>.json` beside the state file: `agents sessions` lists these, `run
/// --resume <id>` continues one. Its transcript is `sessions/<id>.jsonl`; in a file name each byte of `id`
/// outside `[A-Za-z0-9._-]` is `%XX`. Kept `$SHED_SESSIONS_KEEP` days (default 30) after its last turn; `rm`
/// of its terminal deletes both at once.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct SessionRecord {
    /// The agent's session id.
    pub id: String,
    /// The agent id of `run --acp`.
    pub agent: String,
    /// The folder it ran in.
    pub cwd: PathBuf,
    /// The first prompt's first line, at most 80 characters.
    pub title: String,
    /// When its last turn ended, else when it started, unix seconds.
    pub updated: u64,
    /// How many turns it has had.
    pub turns: u64,
    /// The transcript's path; filled in by `agents sessions`, not stored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript: Option<PathBuf>,
}

/// An ACP session's context window use and cost, as the agent last reported it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct AcpUsage {
    /// Tokens in context.
    pub used: u64,
    /// The context window, in tokens.
    pub size: u64,
    /// The session's total cost so far (`amount`, `currency`), when the agent reports it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<Value>,
    /// The last turn's tokens, when the agent's prompt responses carry them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<Tokens>,
    /// The session's tokens over its turns so far, when the agent's prompt responses carry them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<Tokens>,
}

/// Tokens a turn or a session spent, as the agent's prompt responses report them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Tokens {
    /// Input tokens neither read from nor written to the prompt cache.
    pub input: u64,
    /// Input tokens read from the prompt cache.
    pub cache_read: u64,
    /// Input tokens written to the prompt cache.
    pub cache_write: u64,
    /// Output tokens.
    pub output: u64,
}

impl Tokens {
    /// Both counts added up.
    #[must_use]
    pub fn plus(self, o: Self) -> Self {
        Self {
            input: self.input.saturating_add(o.input),
            cache_read: self.cache_read.saturating_add(o.cache_read),
            cache_write: self.cache_write.saturating_add(o.cache_write),
            output: self.output.saturating_add(o.output),
        }
    }

    /// Cache reads as a percentage of all input tokens, or `None` with no input.
    #[must_use]
    pub fn cache_share(&self) -> Option<u64> {
        let all = (self.input.saturating_add(self.cache_read)).saturating_add(self.cache_write);
        self.cache_read.saturating_mul(100).checked_div(all)
    }
}

/// `emit` params.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct EmitParams {
    /// The terminal's name.
    pub name: String,
    /// The event.
    #[serde(flatten)]
    pub kind: EventKind,
    /// A resumed session's history, replayed.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub replay: bool,
}

/// `acp.model` params.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct ModelParams {
    /// The terminal's name.
    pub name: String,
    /// The model id, one of the session's `models`.
    pub model: String,
}
