//! The terminal methods' params and results, and a terminal as it is listed.

use std::collections::BTreeMap;
use std::path::PathBuf;

#[cfg(feature = "schema")]
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{AcpSession, EventKind};

/// How an ACP bridge answers the agent's permission requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum Policy {
    /// Pick the first `allow_once` (else `allow_always`) option.
    Allow,
    /// Pick the first `reject_once` (else `reject_always`) option.
    Deny,
    /// Report `blocked` and wait for `shed approve`.
    Ask,
}

/// `run` params.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct RunParams {
    /// The terminal's name, unique per server; `[A-Za-z0-9._-]+`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Labels, `k=v`.
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    /// The working directory (absolute).
    pub cwd: PathBuf,
    /// Come back as a shell after a server restart.
    #[serde(default)]
    pub keep: bool,
    /// Report `idle` after this many seconds without output, until the program reports a state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_after: Option<u64>,
    /// The program and its arguments; with `acp`, the agent's ACP adapter (empty: the installed
    /// agent's), which the server wraps in its own bridge.
    pub cmd: Vec<String>,
    /// ACP: the model, in the words of the agent's record for the driver the server picks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// ACP: the effort, one of the record's `efforts`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// Variables for the program, over the server's own. Kept in memory only; never saved or shown.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// The ACP agent the program bridges (`shed run --acp`): `read` renders its transcript.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acp: Option<String>,
    /// ACP: how the bridge answers permission requests (default `ask`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<Policy>,
    /// ACP: continue this session (`$SHED_ACP_RESUME` to the bridge): shed's transcript of it is replayed, then
    /// the agent loads it (`session/load`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume: Option<String>,
    /// ACP: MCP servers for the agent, in ACP's shapes: stdio `{name, command, args, env}`, or
    /// `{type: "http" | "sse", name, url, headers}`. Kept in memory only; send them again to resume.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcp_servers: Vec<Value>,
    /// ACP: the session's options, sent unchanged as `_meta` on `session/new` and `session/load`;
    /// which keys exist is the agent's business. Callers: drover (herdsman#923) and cooee.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<serde_json::Map<String, Value>>,
    /// ACP: never hibernate the agent when idle.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_hibernate: bool,
    /// ACP: take this recipe's ready spare session if there is one, and keep a spare ready for the next run.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub warm: bool,
    /// The most memory the terminal's process tree may hold, in MB; default the server's `$SHED_MEM_MAX`, else none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mem_max_mb: Option<u64>,
    /// Start it on this machine, over its held link (`machine.links`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
    /// The terminal that started it (`shed run` sends `$SHED`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<Parent>,
    /// Rows; default the server's `$SHED_SIZE`, else 50.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<u16>,
    /// Columns; default the server's `$SHED_SIZE`, else 200.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cols: Option<u16>,
}

/// A child's link to the terminal that started it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Parent {
    /// The parent terminal's name.
    pub name: String,
    /// Keep running when the parent's program ends; otherwise the child is killed with it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub outlive: bool,
    /// Type a `[shed]` note into the parent when the child blocks, errors or ends, while the parent is not
    /// working.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub notify: bool,
}

/// `ls` params.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct LsParams {
    /// Only terminals carrying every one of these labels.
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    /// Only the children of this terminal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
}

/// `get`, `kill` and `rm` params: the terminal's name.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Target {
    /// The terminal's name.
    pub name: String,
}

/// `read` params.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct ReadParams {
    /// The terminal's name.
    pub name: String,
    /// Only the last N lines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lines: Option<usize>,
    /// Scrollback and screen instead of the screen alone.
    #[serde(default)]
    pub history: bool,
    /// ACP: the transcript's events instead of text.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub events: bool,
    /// With `events`: only those before this `seq`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<u64>,
    /// With `events`: only the last N.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub n: Option<usize>,
    /// Only what came after this `cursor` of an earlier `read` (ACP: the events after its `seq`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<Cursor>,
}

/// A place in a terminal's output, as `read` returns it.
#[derive(Debug, Clone, Default, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Cursor {
    /// The lines that have scrolled off the screen so far (ACP: the transcript's last `seq`).
    pub seq: u64,
    /// A hash of the visible screen's text (ACP: 0).
    pub screen: u32,
}

/// `read` result.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct ReadResult {
    /// Plain text, one line per row, trailing blanks dropped.
    pub text: String,
    /// With `events`: the transcript's events, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<Logged>,
    /// Where this read ends: the next `read`'s `since`.
    pub cursor: Cursor,
    /// With `since`: lines that scrolled off after it but are no longer kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lost: Option<u64>,
    /// With `since`: the screen changed, and `text` ends with it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub changed: bool,
}

/// One event of an ACP terminal's transcript.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Logged {
    /// Its place in the transcript, from 1.
    pub seq: u64,
    /// The event.
    #[serde(flatten)]
    pub kind: EventKind,
}

/// `send` and `terminal.input` params.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct SendParams {
    /// The terminal's name (`terminal` in `terminal.input`).
    #[serde(alias = "terminal")]
    pub name: String,
    /// The text, sent as UTF-8.
    pub text: String,
    /// `send`: no enter after the text.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_enter: bool,
    /// `send`: the text as typed keys even when the program has turned bracketed paste on.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_paste: bool,
    /// `send`: seconds after a person's last key that their unsent line holds the send (default 60, 0 off).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft_wait: Option<u64>,
}

/// `keys` and `terminal.keys` params.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct KeysParams {
    /// The terminal's name (`terminal` in `terminal.keys`).
    #[serde(alias = "terminal")]
    pub name: String,
    /// Key names, pressed in order.
    pub keys: Vec<String>,
}

/// `resize` params.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct ResizeParams {
    /// The terminal's name.
    pub name: String,
    /// Rows, at least 1.
    pub rows: u16,
    /// Columns, at least 1.
    pub cols: u16,
}

/// `state` params.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct StateParams {
    /// The terminal's name.
    pub name: String,
    /// The reported state and note.
    #[serde(flatten)]
    pub report: Reported,
}

/// A state a terminal's program reported.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Reported {
    /// `working`, `blocked`, `done` or `error`; `idle` from OSC 7501 or the `idle_after` fallback.
    pub state: String,
    /// A free-text note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// The other agent's escape the state came from, e.g. `jan-osc9999`; absent for shed's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_source: Option<String>,
    /// Where the state came from: `reported`, `rule:<id>` (a screen rule) or `silence` (`idle_after`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// OSC 7501's record path, e.g. `build/tests`; absent for the terminal's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Why it is blocked: `permission`, `question` or `auth`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Percent done, 0–100, while working or blocked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<u8>,
    /// The program's stable name, e.g. `make`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    /// A short label (OSC 7501's `title`, decoded).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// A one-line description (OSC 7501's `msg`, decoded).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub msg: Option<String>,
}

/// `wait` params.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct WaitParams {
    /// The terminal's name; with `children`, the parent's.
    pub name: String,
    /// Wait on the terminal's children instead of the terminal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub children: Option<Children>,
    /// Reported states that end the wait.
    #[serde(default)]
    pub until: Vec<String>,
    /// With `blocked` in `until`: only a block of this kind (`permission`, `question` or `auth`) ends the wait.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// A pattern (`.`, `[…]`, `\d \w \s`, `( | )`, `* + ? {n,m}`, `\b`, `^ $`) that ends the wait when a screen line matches.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    /// Give up after this many seconds; none waits for ever.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<f64>,
}

/// Which children end a `wait`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum Children {
    /// No child is working: each has exited or reports something other than `working`.
    All,
    /// The next child that reports `done`, `blocked` or `error`, or exits; the result names it.
    Any,
}

/// How a `wait` ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum WaitOutcome {
    /// A state in `until` or the `output` pattern matched.
    Matched,
    /// The program exited first.
    Exited,
    /// The timeout passed first.
    Timeout,
}

/// `wait` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct WaitResult {
    /// How the wait ended.
    pub outcome: WaitOutcome,
    /// The terminal at that moment.
    pub terminal: TerminalInfo,
}

/// A terminal's process tree's resource use at the last sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Resources {
    /// Summed resident memory, in MiB.
    pub rss_mb: u64,
    /// CPU use since the previous sample, in percent of one core.
    pub cpu_pct: u64,
    /// ACP: the agent's own processes' memory, in MiB: the bridge, and the adapter and agent as the turn began.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_mb: Option<u64>,
    /// ACP: the memory of everything else in the tree, what the agent started, in MiB.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_mb: Option<u64>,
}

/// Whether a terminal's program runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum State {
    /// The program runs.
    Running,
    /// The program has exited, or was lost in a server restart.
    Exited,
}

/// A terminal, as `run`, `ls`, `get`, `kill` and `rm` return it: `state` is `running` or `exited`; `exit` is the
/// exit status (128 + N for signal N), `null` while running or when the program was lost in a server restart;
/// `pid` is set while running; `reported` is the state the program last reported (absent until then), with OSC
/// 7501's `kind`, `progress`, `app`, `title` and `msg` when it sent them, or `stuck` while the program has left
/// input unread for over a second (`send`, `keys` and `acp.model` then fail, their input queued); `records` are
/// its OSC 7501 reports with an `id`, one per id; `idle_after` is set when the `idle` fallback is on;
/// `resources` is the process tree's memory (MiB) and CPU (percent of one core) at the last sample, while
/// running; an ACP terminal's `session` is its agent session (`id`, for `run`'s `resume`; the current `model`
/// and the `models`, `[id, name]`; `run`'s `mcp_servers` with each env and header value as `***`; the agent's
/// slash `commands`, `[name, description, hint]`; its last `usage`: `used` and `size` of the context window in
/// tokens, and the `cost` when sent); `parent` names the terminal that started it (absent without one), with
/// `outlive` and `notify` as `run` set them; `redacted` (a login's terminal) means `read`, `screen.watch`,
/// `wait`'s `output` and its events show its text with anything like a token as `[redacted]`, and `attach` is refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct TerminalInfo {
    /// The unique name.
    pub name: String,
    /// Labels, `k=v`.
    pub labels: BTreeMap<String, String>,
    /// The working directory.
    pub cwd: PathBuf,
    /// The program and its arguments.
    pub cmd: Vec<String>,
    /// Comes back as a shell after a server restart.
    pub keep: bool,
    /// Running or exited.
    pub state: State,
    /// The exit status once exited (128 + N for signal N); `null` while running or when lost.
    pub exit: Option<i32>,
    /// The program's pid while running.
    pub pid: Option<i32>,
    /// The state the program last reported, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reported: Option<Reported>,
    /// The OSC 7501 records the program reported with an `id`, least recently updated first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub records: Vec<Reported>,
    /// The `idle` fallback's silence, in seconds, if on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_after: Option<u64>,
    /// The process tree's resource use at the last sample, while running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resources: Option<Resources>,
    /// The ACP agent the program bridges, for an ACP terminal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acp: Option<String>,
    /// The ACP agent session, from the bridge's last `session` event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<AcpSession>,
    /// The terminal that started it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<Parent>,
    /// The most memory its process tree may hold, in MB, if limited.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mem_max_mb: Option<u64>,
    /// Its screen passes through `redact` on every read, and `attach` is refused.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub redacted: bool,
}
