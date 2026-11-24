//! The `events` stream: its params and the events it carries.

use std::collections::BTreeMap;
use std::path::PathBuf;

#[cfg(feature = "schema")]
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{AcpSession, AcpUsage, Reported, Resources};

/// `events` params.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct EventsParams {
    /// Only events of terminals carrying every one of these labels.
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    /// Replay the kept events whose `seq` is above this one first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<u64>,
}

/// An event names its terminal, carries its labels and its `parent` (absent without one) and counts up in
/// `seq`; `event` is `started`, `state` (the reported state changed), `exited` (by itself, with its `code`),
/// `killed` (by `kill`), `resources` (memory or CPU moved by more than 10 % since the last one) or `mem-limit`
/// (the tree passed `mem_max_mb`; `ended_by` is `kernel`, `shed` or `none`).
///
/// An ACP terminal (`shed run --acp`) adds its agent's events, emitted by its bridge: `turn` (a prompt), `text`
/// (a reply chunk), `tool_call` (started or changed), `plan`, `mode`, `approval_request` (answer with
/// `shed approve`), `approved`, `reply` (the turn's whole reply and its `stop_reason`), `session` (the session
/// is ready, or its model or slash commands changed) and `usage` (the agent's `usage_update`: tokens `used` of
/// the context window's `size`, and its `cost`). A resumed session's history (`run` with `resume`) comes first, from
/// shed's own transcript, its events marked `"replay": true` so a client does not count them as new; what the
/// agent replays is dropped. An idle agent hibernates (state `hibernated`, its processes stopped); the next
/// send wakes it on the same session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Event {
    /// Counts up from 1 for every event the server emits, so a reconnecting client can ask for what it
    /// missed with `since`.
    pub seq: u64,
    /// The terminal's name.
    pub name: String,
    /// The terminal's labels.
    pub labels: BTreeMap<String, String>,
    /// The terminal's parent, if it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// What happened.
    #[serde(flatten)]
    pub kind: EventKind,
    /// ACP: a resumed session's history, replayed from shed's transcript, not a new event.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub replay: bool,
}

/// What an [`Event`] reports; the `event` field names it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum EventKind {
    /// The program started (`run`, or a kept terminal back after a server restart).
    #[default]
    Started,
    /// The reported state changed.
    State(Box<Reported>),
    /// The program exited by itself.
    Exited {
        /// The exit status (128 + N for signal N).
        code: Option<i32>,
    },
    /// `kill` ended the process tree.
    Killed,
    /// The tree's memory or CPU moved by more than 10 % since the last such event.
    Resources(Resources),
    /// The process tree passed its memory limit.
    #[serde(rename = "mem-limit")]
    MemLimit {
        /// The limit, in MB.
        limit_mb: u64,
        /// The process over it: the largest the agent started, or the agent's own when it started none.
        process: String,
        /// Its memory, in MB.
        rss_mb: u64,
        /// `kernel` (the terminal's scope) or `shed` (the sampler) ended it; `none`: the agent's own, only reported.
        ended_by: String,
    },
    /// ACP: a prompt started a turn.
    Turn {
        /// The prompt text.
        prompt: String,
        /// The bridge's to the server: the agent's own pids as the turn began; never in the event stream.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        agent: Vec<i32>,
    },
    /// ACP: a chunk of the agent's reply.
    Text {
        /// The chunk.
        text: String,
    },
    /// ACP: a tool call started or changed.
    ToolCall {
        /// The agent's tool call id.
        id: String,
        /// What the tool does, in the agent's words.
        title: String,
        /// `pending`, `in_progress`, `completed` or `failed`.
        status: String,
        /// ACP's tool kind (`read`, `edit`, `execute`, …), when the agent sends one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<String>,
        /// The paths ACP's `locations` name in this update.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        locations: Vec<PathBuf>,
        /// ACP's `rawInput` in this update, left out over 4 KiB.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        raw_input: Option<Value>,
    },
    /// ACP: the agent's plan, as it sent it.
    Plan {
        /// The entries (`content`, `priority`, `status`).
        entries: Vec<Value>,
    },
    /// ACP: the session's mode changed.
    Mode {
        /// The mode id.
        mode: String,
    },
    /// ACP: the agent asks to run a tool; `shed approve` answers with one option.
    ApprovalRequest {
        /// The request's id, which `shed approve --request` names.
        #[serde(default)]
        id: String,
        /// The tool call's title.
        tool: String,
        /// The choices.
        options: Vec<ApprovalOption>,
        /// ACP's `toolCall` as the agent sent it, without `content`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        call: Option<Value>,
    },
    /// ACP: a permission request was answered (by `shed approve` or the `--policy`).
    Approved {
        /// The chosen option's id.
        option: String,
        /// `judge` when the `judge` policy answered.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        by: Option<String>,
        /// The judge's `allow` or `deny`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        decision: Option<String>,
        /// The judge's reason, or why it failed.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    /// ACP: the turn ended.
    Reply {
        /// The agent's whole reply text.
        text: String,
        /// Why it ended: `end_turn`, `max_tokens`, `max_turn_requests`, `refusal` or `cancelled`.
        stop_reason: String,
    },
    /// ACP: the session is ready, or its model or slash commands changed.
    Session(Box<AcpSession>),
    /// ACP: the agent's context window use and cost changed (`usage_update`).
    Usage(AcpUsage),
    /// The terminal runs on an account: from `start`, or after a `limit` on the account it `was` on or the owner's
    /// `account.use`.
    Account {
        /// The account it runs on now.
        account: String,
        /// The account it ran on before.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        was: Option<String>,
        /// `start`, `limit` or `owner`.
        why: String,
    },
}

impl EventKind {
    /// One of the ACP events `emit` takes.
    #[must_use]
    pub fn is_acp(&self) -> bool {
        !matches!(
            self,
            Self::Started
                | Self::State(_)
                | Self::Exited { .. }
                | Self::Killed
                | Self::Resources(_)
                | Self::MemLimit { .. }
                | Self::Account { .. }
        )
    }
}

/// One choice of an [`EventKind::ApprovalRequest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct ApprovalOption {
    /// The option id `shed approve` takes.
    pub id: String,
    /// Its label.
    pub name: String,
    /// `allow_once`, `allow_always`, `reject_once` or `reject_always`.
    pub kind: String,
}
