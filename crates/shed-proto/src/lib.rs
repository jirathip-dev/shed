//! shed's API types and JSON-RPC method list.
//!
//! The wire format is newline-delimited JSON-RPC 2.0 over a Unix socket: one [`Request`] per line, one
//! [`Response`] per line. Every connection starts with [`Method::Hello`]; a client and server of
//! different products or [`VERSION`]s are refused with [`ErrorCode::Version`]. `docs/protocol.md` is generated
//! from these types (`SHED_BLESS=1 cargo test -p shed-proto`); CI fails when it drifts.
#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use schemars::generate::SchemaSettings;
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The protocol version: the shed version. Client and server must match exactly.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The JSON-RPC version string every message carries.
pub const JSONRPC: &str = "2.0";

/// A JSON-RPC request. Without an `id` it is a notification and gets no response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Request {
    /// Always `"2.0"`.
    pub jsonrpc: String,
    /// The request id, echoed in the response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    /// The method name; see [`Method`].
    pub method: String,
    /// The method's parameters.
    #[serde(default)]
    pub params: Value,
}

impl Request {
    /// A request for `method` with `params` and a numeric `id`.
    #[must_use]
    pub fn new(id: u64, method: Method, params: Value) -> Self {
        Self {
            jsonrpc: JSONRPC.to_owned(),
            id: Some(id.into()),
            method: method.name().to_owned(),
            params,
        }
    }
}

/// A JSON-RPC response: exactly one of `result` and `error` is set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Response {
    /// Always `"2.0"`.
    pub jsonrpc: String,
    /// The id of the request this answers (`null` when the request could not be read).
    pub id: Value,
    /// The result on success.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// The error on failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl Response {
    /// A success response.
    #[must_use]
    pub fn ok(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: JSONRPC.to_owned(),
            id,
            result: Some(result),
            error: None,
        }
    }

    /// An error response.
    #[must_use]
    pub fn err(id: Value, error: RpcError) -> Self {
        Self {
            jsonrpc: JSONRPC.to_owned(),
            id,
            result: None,
            error: Some(error),
        }
    }
}

/// A JSON-RPC error object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RpcError {
    /// The numeric code; see [`ErrorCode`].
    pub code: i64,
    /// `NAME: detail`, where `NAME` is the [`ErrorCode::name`].
    pub message: String,
}

impl RpcError {
    /// An error with `code` and a message `NAME: detail`.
    #[must_use]
    pub fn new(code: ErrorCode, detail: impl std::fmt::Display) -> Self {
        Self {
            code: code.code(),
            message: format!("{}: {detail}", code.name()),
        }
    }

    /// The version refusal: `SHED_VERSION: client shed x, server shed y; restart the server`.
    #[must_use]
    pub fn version(client: &str, server: &str) -> Self {
        Self::new(
            ErrorCode::Version,
            format!("client {client}, server {server}; restart the server"),
        )
    }
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// The error codes: JSON-RPC's own, then shed's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    /// The line is not valid JSON.
    Parse,
    /// The JSON is not a valid request.
    InvalidRequest,
    /// No such method.
    MethodNotFound,
    /// The parameters do not fit the method.
    InvalidParams,
    /// The server failed (saving state, reading the process table).
    Internal,
    /// Client and server versions differ.
    Version,
    /// A method other than `hello` came before the handshake.
    NoHello,
    /// No terminal has that name.
    NoSuchTerminal,
    /// Another terminal already has that name.
    NameTaken,
    /// The terminal is still running (`rm` needs it exited; `kill` it first).
    StillRunning,
    /// The program could not start.
    SpawnFailed,
    /// The client marked the terminal read-only (`terminal.readonly`): no input, keys, resize or attach.
    ReadOnly,
    /// The machine a `<machine>/<name>` names has no link up (`machine.links` says why).
    NoLink,
}

impl ErrorCode {
    /// Every code, in documentation order.
    pub const ALL: [Self; 13] = [
        Self::Parse,
        Self::InvalidRequest,
        Self::MethodNotFound,
        Self::InvalidParams,
        Self::Internal,
        Self::Version,
        Self::NoHello,
        Self::NoSuchTerminal,
        Self::NameTaken,
        Self::StillRunning,
        Self::SpawnFailed,
        Self::ReadOnly,
        Self::NoLink,
    ];

    /// The numeric JSON-RPC code.
    #[must_use]
    pub fn code(self) -> i64 {
        match self {
            Self::Parse => -32700,
            Self::InvalidRequest => -32600,
            Self::MethodNotFound => -32601,
            Self::InvalidParams => -32602,
            Self::Internal => -32603,
            Self::Version => -32000,
            Self::NoHello => -32001,
            Self::NoSuchTerminal => -32002,
            Self::NameTaken => -32003,
            Self::StillRunning => -32004,
            Self::SpawnFailed => -32005,
            Self::ReadOnly => -32006,
            Self::NoLink => -32007,
        }
    }

    /// The name that starts the error message.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Parse => "PARSE",
            Self::InvalidRequest => "INVALID_REQUEST",
            Self::MethodNotFound => "NO_SUCH_METHOD",
            Self::InvalidParams => "INVALID_PARAMS",
            Self::Internal => "INTERNAL",
            Self::Version => "SHED_VERSION",
            Self::NoHello => "NO_HELLO",
            Self::NoSuchTerminal => "NO_SUCH_TERMINAL",
            Self::NameTaken => "NAME_TAKEN",
            Self::StillRunning => "STILL_RUNNING",
            Self::SpawnFailed => "SPAWN_FAILED",
            Self::ReadOnly => "READONLY",
            Self::NoLink => "NO_LINK",
        }
    }
}

/// The methods, with their parameter and result types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// The handshake: [`Hello`] → [`Hello`].
    Hello,
    /// Start a terminal: [`RunParams`] → [`TerminalInfo`].
    Run,
    /// List terminals: [`LsParams`] → `[TerminalInfo]`.
    Ls,
    /// One terminal: [`Target`] → [`TerminalInfo`].
    Get,
    /// Read the screen or scrollback: [`ReadParams`] → [`ReadResult`].
    Read,
    /// Type text: [`SendParams`] → `{}`.
    Send,
    /// Press keys: [`KeysParams`] → `{}`.
    Keys,
    /// End the process tree: [`Target`] → [`TerminalInfo`].
    Kill,
    /// Remove an exited terminal: [`Target`] → [`TerminalInfo`].
    Rm,
    /// Resize a terminal: [`ResizeParams`] → `{}`.
    Resize,
    /// Take over a terminal: [`Target`] → [`TerminalInfo`], then the connection streams raw output.
    Attach,
    /// Report a terminal's state: [`StateParams`] → [`TerminalInfo`].
    State,
    /// Block until a state or output matches: [`WaitParams`] → [`WaitResult`].
    Wait,
    /// Stream events: [`EventsParams`] → `{}`, then the connection streams [`Event`] lines.
    Events,
    /// Emit an ACP terminal's structured event: [`EmitParams`] → `{}`.
    Emit,
    /// Stream a terminal's screen: [`WatchParams`] → `{}`, then [`ScreenUpdate`] lines.
    ScreenWatch,
    /// Type text: [`SendParams`], `terminal` for `name` → `{}`.
    Input,
    /// Press keys: [`KeysParams`], `terminal` for `name` → `{}`.
    TerminalKeys,
    /// Resize: [`TerminalResizeParams`] → `{}`.
    TerminalResize,
    /// Refuse this connection's input to a terminal: [`TerminalParams`] → `{}`.
    ReadOnly,
    /// The machine's health: [`StatsParams`] → [`MachineStats`].
    MachineStats,
    /// Switch an ACP terminal's model: [`ModelParams`] → `{}`.
    Model,
    /// The held links to the machines: `{}` → `{machine: Link}`.
    MachineLinks,
    /// The API as an `OpenRPC` document: `{}` → [`openrpc`].
    Discover,
}

impl Method {
    /// Every method, in documentation order.
    pub const ALL: [Self; 24] = [
        Self::Hello,
        Self::Run,
        Self::Ls,
        Self::Get,
        Self::Read,
        Self::Send,
        Self::Keys,
        Self::Kill,
        Self::Rm,
        Self::Resize,
        Self::Attach,
        Self::State,
        Self::Wait,
        Self::Events,
        Self::Emit,
        Self::ScreenWatch,
        Self::Input,
        Self::TerminalKeys,
        Self::TerminalResize,
        Self::ReadOnly,
        Self::MachineStats,
        Self::Model,
        Self::MachineLinks,
        Self::Discover,
    ];

    /// The wire name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Hello => "hello",
            Self::Run => "run",
            Self::Ls => "ls",
            Self::Get => "get",
            Self::Read => "read",
            Self::Send => "send",
            Self::Keys => "keys",
            Self::Kill => "kill",
            Self::Rm => "rm",
            Self::Resize => "resize",
            Self::Attach => "attach",
            Self::State => "state",
            Self::Wait => "wait",
            Self::Events => "events",
            Self::Emit => "emit",
            Self::ScreenWatch => "screen.watch",
            Self::Input => "terminal.input",
            Self::TerminalKeys => "terminal.keys",
            Self::TerminalResize => "terminal.resize",
            Self::ReadOnly => "terminal.readonly",
            Self::MachineStats => "machine.stats",
            Self::Model => "acp.model",
            Self::MachineLinks => "machine.links",
            Self::Discover => "rpc.discover",
        }
    }

    /// The method with this wire name.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == name)
    }

    /// One line on what the method does.
    #[must_use]
    pub fn summary(self) -> &'static str {
        match self {
            Self::Hello => {
                "The handshake; must come first on every connection. Another product or version is refused with `SHED_VERSION`."
            }
            Self::Run => {
                "Start a terminal. `name` defaults to the first free `tN`; `keep` brings it back as a shell in its cwd after a server restart; `rows` and `cols` default to `$SHED_SIZE` (`RxC`) of the server, else 50x200. ACP: `mcp_servers` (stdio, `http`, `sse`, in ACP's shapes) go to the agent; their env and header values are never saved or shown. `no_hibernate` keeps the agent running when idle. `parent` links it to the terminal that started it: when the parent's program ends the child is killed too, unless `outlive`; with `notify` a `[shed] <name> blocked|error|exited …` line is typed into the parent (a prompt for an ACP parent) when the child blocks, errors or ends, once per piece of news, held while the parent reports `working`."
            }
            Self::Ls => {
                "List terminals, by name; `labels` keeps those that carry every given label, `parent` the children of that terminal."
            }
            Self::Get => "One terminal.",
            Self::Read => {
                "The visible screen, or with `history` the scrollback and screen, as plain text; `lines` keeps the last N lines. ACP: the transcript as text, or with `events` its events (each with its `seq`), paged by `before` and `n`."
            }
            Self::Send => "Type text into the terminal.",
            Self::Keys => {
                "Press keys by name (`enter`, `esc`, `ctrl+c`, `up`, …); nothing is sent if any name is unknown."
            }
            Self::Kill => {
                "End the terminal's whole process tree (SIGTERM, then SIGKILL); returns the exited terminal."
            }
            Self::Rm => "Remove an exited terminal; returns it as it was.",
            Self::Resize => "Resize the terminal; the program gets SIGWINCH.",
            Self::Attach => {
                "Take over the terminal. After the response the server writes a redraw (clear, then the screen text) and the program's raw output, and closes the connection when the program's PTY closes. The client sends only notifications from then on (`send`, `keys`, `resize`); detaching is closing the connection."
            }
            Self::State => {
                "Report the terminal's state (`working`, `blocked`, `done` or `error`) with an optional note; a program can also print `ESC ] 7501 ; state=<s>[;note=<n>] BEL`, which never reaches the screen."
            }
            Self::Wait => {
                "Answer once the reported state is one of `until` (`idle` too, from the `idle_after` fallback) or a visible screen line matches the pattern `output` (`matched`), the program exits (`exited`) or `timeout` seconds pass (`timeout`). The server blocks; nothing polls. With `children`, `name` is a parent: `all` matches once no child is working (`terminal` is the parent), `any` on the next child that reports `done`, `blocked` or `error`, or exits (`terminal` is that child; `exited` when no child runs)."
            }
            Self::Events => {
                "Stream events. After the response the server writes one event per line (see Events) for the terminals carrying every one of `labels`, first those after `since` still in the ring of the last 1,000, and never closes the connection; closing it unsubscribes."
            }
            Self::Emit => {
                "Emit one of an ACP terminal's structured events (`turn`, `text`, `tool_call`, `plan`, `mode`, `approval_request`, `approved`, `reply`); its bridge (`shed run --acp`) calls this. Other events are refused."
            }
            Self::ScreenWatch => {
                "Stream a running terminal's screen. After the response the server writes screen updates (see Screen updates), at most 20 a second: the full screen first (none when `since` is the screen's current `seq`), then the rows that changed. It closes the connection when the program's PTY closes. The connection still takes requests; their responses are not written."
            }
            Self::Input => "Type text into the terminal, as `send`.",
            Self::TerminalKeys => "Press keys by name, as `keys`; `ctrl-x` is `ctrl+x`.",
            Self::TerminalResize => {
                "Resize the terminal, as `resize`; without `terminal`, the one this connection watches."
            }
            Self::ReadOnly => {
                "Mark the terminal read-only for this connection: from then on, until it closes, the connection's `send`, `keys`, `resize`, `attach`, `terminal.input`, `terminal.keys` and `terminal.resize` to the terminal are refused with `READONLY`. The connection may resize only the terminals it has not marked."
            }
            Self::MachineStats => {
                "The machine's health now: load per CPU, memory pressure (percent of memory not available), swap used, free space on the volume of the state file, CPUs, uptime and the terminals by state (the reported state while running, else `running`; `exited`). The server keeps one sample a minute for 24 h; `since` (unix seconds) adds those after it as `history`."
            }
            Self::Model => {
                "Switch an ACP terminal's model to one of its `session.models`: its bridge asks the agent (`session/set_config_option`, the `model` option) and emits `session` with the new `model`."
            }
            Self::MachineLinks => {
                "Each configured machine's held link: `up`, `reconnecting` or `down`, since when, and why it is down. A request naming `<machine>/<name>` (`get`, `read`, `send`, `keys`, `kill`, `rm`, `state`, `acp.model`, `wait`; `run` with `machine`, ACP ones bridged by that server) goes over the link; `ls` and `events` include every machine, their names so prefixed. Reads the config again, linking machines added since."
            }
            Self::Discover => {
                "The whole API as one OpenRPC document, as `shed schema` prints it (`docs/shed.openrpc.json`): every method's params and result, the streamed events and screen updates, and the error codes, as JSON Schema. Needs no `hello`."
            }
        }
    }
}

/// `hello` params and result: the sender's product and version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Hello {
    /// The product, `shed`; a peer that sends none is refused like another version.
    #[serde(default)]
    pub product: String,
    /// The shed version.
    pub version: String,
}

impl Hello {
    /// This build's hello: `shed` and [`VERSION`].
    #[must_use]
    pub fn ours() -> Self {
        Self {
            product: "shed".to_owned(),
            version: VERSION.to_owned(),
        }
    }

    /// `shed 1.2.3`, or `unnamed 1.2.3` without a product.
    #[must_use]
    pub fn label(&self) -> String {
        let product = if self.product.is_empty() {
            "unnamed"
        } else {
            &self.product
        };
        format!("{product} {}", self.version)
    }
}

/// How an ACP bridge answers the agent's permission requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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
    /// The ACP agent the program bridges (`shed run --acp`): `read` renders its transcript.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acp: Option<String>,
    /// ACP: how the bridge answers permission requests (default `ask`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<Policy>,
    /// ACP: load this agent session (`session/load`, `$SHED_ACP_RESUME` to the bridge) instead of a new one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume: Option<String>,
    /// ACP: MCP servers for the agent, in ACP's shapes: stdio `{name, command, args, env}`, or
    /// `{type: "http" | "sse", name, url, headers}`. Kept in memory only; send them again to resume.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcp_servers: Vec<Value>,
    /// ACP: never hibernate the agent when idle.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_hibernate: bool,
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
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LsParams {
    /// Only terminals carrying every one of these labels.
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    /// Only the children of this terminal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
}

/// `get`, `kill` and `rm` params: the terminal's name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Target {
    /// The terminal's name.
    pub name: String,
}

/// `read` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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
}

/// `read` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReadResult {
    /// Plain text, one line per row, trailing blanks dropped.
    pub text: String,
    /// With `events`: the transcript's events, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<Logged>,
}

/// One event of an ACP terminal's transcript.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Logged {
    /// Its place in the transcript, from 1.
    pub seq: u64,
    /// The event.
    #[serde(flatten)]
    pub kind: EventKind,
}

/// `send` and `terminal.input` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SendParams {
    /// The terminal's name (`terminal` in `terminal.input`).
    #[serde(alias = "terminal")]
    pub name: String,
    /// The text, sent as UTF-8.
    pub text: String,
}

/// `keys` and `terminal.keys` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct KeysParams {
    /// The terminal's name (`terminal` in `terminal.keys`).
    #[serde(alias = "terminal")]
    pub name: String,
    /// Key names, pressed in order.
    pub keys: Vec<String>,
}

/// `resize` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ResizeParams {
    /// The terminal's name.
    pub name: String,
    /// Rows, at least 1.
    pub rows: u16,
    /// Columns, at least 1.
    pub cols: u16,
}

/// `state` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StateParams {
    /// The terminal's name.
    pub name: String,
    /// The reported state and note.
    #[serde(flatten)]
    pub report: Reported,
}

/// A state a terminal's program reported.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Reported {
    /// `working`, `blocked`, `done` or `error`; `idle` from the `idle_after` fallback.
    pub state: String,
    /// A free-text note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// The other agent's escape the state came from, e.g. `jan-osc9999`; absent for shed's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_source: Option<String>,
}

/// `wait` params.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WaitParams {
    /// The terminal's name; with `children`, the parent's.
    pub name: String,
    /// Wait on the terminal's children instead of the terminal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub children: Option<Children>,
    /// Reported states that end the wait.
    #[serde(default)]
    pub until: Vec<String>,
    /// A pattern (`.`, `[…]`, `\d \w \s`, `* + ?`, `^ $`) that ends the wait when a screen line matches.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    /// Give up after this many seconds; none waits for ever.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<f64>,
}

/// Which children end a `wait`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Children {
    /// No child is working: each has exited or reports something other than `working`.
    All,
    /// The next child that reports `done`, `blocked` or `error`, or exits; the result names it.
    Any,
}

/// How a `wait` ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WaitResult {
    /// How the wait ended.
    pub outcome: WaitOutcome,
    /// The terminal at that moment.
    pub terminal: TerminalInfo,
}

/// `events` params.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EventsParams {
    /// Only events of terminals carrying every one of these labels.
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    /// Replay the kept events whose `seq` is above this one first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<u64>,
}

/// One event, as `events` streams it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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
    /// ACP: a resumed session's history, replayed (`session/load`), not a new event.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub replay: bool,
}

/// What an [`Event`] reports; the `event` field names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum EventKind {
    /// The program started (`run`, or a kept terminal back after a server restart).
    Started,
    /// The reported state changed.
    State(Reported),
    /// The program exited by itself.
    Exited {
        /// The exit status (128 + N for signal N).
        code: Option<i32>,
    },
    /// `kill` ended the process tree.
    Killed,
    /// The tree's memory or CPU moved by more than 10 % since the last such event.
    Resources(Resources),
    /// ACP: a prompt started a turn.
    Turn {
        /// The prompt text.
        prompt: String,
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
        /// The tool call's title.
        tool: String,
        /// The choices.
        options: Vec<ApprovalOption>,
    },
    /// ACP: a permission request was answered (by `shed approve` or the `--policy`).
    Approved {
        /// The chosen option's id.
        option: String,
    },
    /// ACP: the turn ended.
    Reply {
        /// The agent's whole reply text.
        text: String,
        /// Why it ended: `end_turn`, `max_tokens`, `max_turn_requests`, `refusal` or `cancelled`.
        stop_reason: String,
    },
    /// ACP: the session is ready, or its model or slash commands changed.
    Session(AcpSession),
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
        )
    }
}

/// One choice of an [`EventKind::ApprovalRequest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ApprovalOption {
    /// The option id `shed approve` takes.
    pub id: String,
    /// Its label.
    pub name: String,
    /// `allow_once`, `allow_always`, `reject_once` or `reject_always`.
    pub kind: String,
}

/// An ACP terminal's agent session, as its bridge last reported it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AcpSession {
    /// The agent's session id, for `run --acp … --resume`.
    pub id: String,
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
}

/// `emit` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ModelParams {
    /// The terminal's name.
    pub name: String,
    /// The model id, one of the session's `models`.
    pub model: String,
}

/// `machine.stats` params.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StatsParams {
    /// Add the kept samples taken after this time (unix seconds) as `history`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<u64>,
}

/// One sample of a machine's health.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Sample {
    /// When it was taken, unix seconds.
    pub at: u64,
    /// The 1-minute load average divided by `cpus`.
    pub load_per_cpu: f64,
    /// Percent of memory not available.
    pub mem_pressure_pct: u64,
    /// Swap in use, MiB.
    pub swap_used_mb: u64,
    /// Free space on the volume of the state file, MiB.
    pub disk_free_mb: u64,
    /// Logical CPUs.
    pub cpus: u64,
    /// Seconds since boot.
    pub uptime_s: u64,
    /// Terminals by state: the reported state while running, else `running`; `exited`.
    pub terminals: BTreeMap<String, u64>,
}

/// A held link's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LinkState {
    /// Connected.
    Up,
    /// Dropped, retried with backoff.
    Reconnecting,
    /// Never connected, or refused (another version); retried with backoff.
    Down,
}

/// One machine's held link, as `machine.links` reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Link {
    /// Up, reconnecting or down.
    pub state: LinkState,
    /// Since when, unix seconds.
    pub since: u64,
    /// Why it is down: the last connect's error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// `machine.stats` result: a sample taken now, and the kept ones after `since`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MachineStats {
    /// Now.
    #[serde(flatten)]
    pub now: Sample,
    /// The kept samples (one a minute, 24 h) after `since`, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<Sample>,
}

/// A terminal's process tree's resource use at the last sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Resources {
    /// Summed resident memory, in MiB.
    pub rss_mb: u64,
    /// CPU use since the previous sample, in percent of one core.
    pub cpu_pct: u64,
}

/// Whether a terminal's program runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum State {
    /// The program runs.
    Running,
    /// The program has exited, or was lost in a server restart.
    Exited,
}

/// One terminal, as `run`, `ls`, `get`, `kill` and `rm` return it and the state file stores it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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
}

/// `screen.watch` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WatchParams {
    /// The terminal's name.
    pub terminal: String,
    /// The `seq` of the last update the client applied: no full screen first if nothing changed since.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<u64>,
}

/// One `screen.watch` update.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ScreenUpdate {
    /// The screen's change count it shows; pass it as `since` when watching again.
    pub seq: u64,
    /// Every row is in `lines` (the first update, and after a resize).
    pub full: bool,
    /// Rows.
    pub rows: u16,
    /// Columns.
    pub cols: u16,
    /// The cursor's row and column, from 0.
    pub cursor: [u16; 2],
    /// Lines in scrollback.
    pub scrollback: usize,
    /// The rows that changed.
    pub lines: Vec<Line>,
}

/// One row of a [`ScreenUpdate`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Line {
    /// The row, from 0.
    pub row: u16,
    /// Its cells, in runs of the same pen; trailing blank default cells dropped.
    pub runs: Vec<Run>,
}

/// Consecutive cells with the same pen; `null` colours are the terminal's defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "the four SGR attributes are independent flags"
)]
pub struct Run {
    /// The cells' characters.
    pub text: String,
    /// Foreground.
    pub fg: Option<Color>,
    /// Background.
    pub bg: Option<Color>,
    /// Bold.
    pub bold: bool,
    /// Italic.
    pub italic: bool,
    /// Underline.
    pub underline: bool,
    /// Foreground and background swapped.
    pub inverse: bool,
}

/// A colour: a palette index (0–15 the named colours, up to 255 the xterm palette) or `[r, g, b]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Color {
    /// A palette index.
    Index(u8),
    /// Red, green, blue.
    Rgb([u8; 3]),
}

/// `terminal.resize` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TerminalResizeParams {
    /// The terminal's name; default the one this connection watches.
    #[serde(default, alias = "name", skip_serializing_if = "Option::is_none")]
    pub terminal: Option<String>,
    /// Columns, at least 1.
    pub cols: u16,
    /// Rows, at least 1.
    pub rows: u16,
}

/// `terminal.readonly` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TerminalParams {
    /// The terminal's name.
    pub terminal: String,
}

impl Method {
    fn schemas(self, g: &mut SchemaGenerator) -> [Schema; 2] {
        fn pair<P: JsonSchema, R: JsonSchema>(g: &mut SchemaGenerator) -> [Schema; 2] {
            [P::json_schema(g), g.subschema_for::<R>()]
        }
        match self {
            Self::Hello => pair::<Hello, Hello>(g),
            Self::Run => pair::<RunParams, TerminalInfo>(g),
            Self::Ls => pair::<LsParams, Vec<TerminalInfo>>(g),
            Self::Get | Self::Attach | Self::Kill | Self::Rm => pair::<Target, TerminalInfo>(g),
            Self::Read => pair::<ReadParams, ReadResult>(g),
            Self::Send => pair::<SendParams, Empty>(g),
            Self::Keys => pair::<KeysParams, Empty>(g),
            Self::Resize => pair::<ResizeParams, Empty>(g),
            Self::State => pair::<StateParams, TerminalInfo>(g),
            Self::Wait => pair::<WaitParams, WaitResult>(g),
            Self::Events => pair::<EventsParams, Empty>(g),
            Self::Emit => pair::<EmitParams, Empty>(g),
            Self::ScreenWatch => pair::<WatchParams, Empty>(g),
            Self::Input => terminal_for_name(pair::<SendParams, Empty>(g)),
            Self::TerminalKeys => terminal_for_name(pair::<KeysParams, Empty>(g)),
            Self::TerminalResize => pair::<TerminalResizeParams, Empty>(g),
            Self::ReadOnly => pair::<TerminalParams, Empty>(g),
            Self::MachineStats => pair::<StatsParams, MachineStats>(g),
            Self::Model => pair::<ModelParams, Empty>(g),
            Self::MachineLinks => pair::<Empty, BTreeMap<String, Link>>(g),
            Self::Discover => [Empty::json_schema(g), Schema::from(true)],
        }
    }

    fn stream(self, g: &mut SchemaGenerator) -> Option<Schema> {
        match self {
            Self::Events => Some(g.subschema_for::<Event>()),
            Self::ScreenWatch => Some(g.subschema_for::<ScreenUpdate>()),
            _ => None,
        }
    }
}

#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
struct Empty {}

fn terminal_for_name([mut params, result]: [Schema; 2]) -> [Schema; 2] {
    for (key, value) in params.as_object_mut().into_iter().flatten() {
        match (key.as_str(), value) {
            ("properties", Value::Object(props)) => {
                if let Some(name) = props.remove("name") {
                    props.insert("terminal".to_owned(), name);
                }
            }
            ("required", Value::Array(names)) => {
                for name in names.iter_mut().filter(|n| *n == "name") {
                    *name = "terminal".into();
                }
            }
            _ => {}
        }
    }
    [params, result]
}

/// The whole API as one `OpenRPC` document (`shed schema`, `rpc.discover`): every method's params and result,
/// the streamed [`Event`]s and [`ScreenUpdate`]s (`x-stream`) and the error codes, as JSON Schema.
#[must_use]
pub fn openrpc() -> Value {
    let mut g = SchemaSettings::draft2020_12()
        .with(|s| s.definitions_path = "/components/schemas".into())
        .into_generator();
    let methods: Vec<Value> = Method::ALL.map(|m| describe(m, &mut g)).into();
    let errors: serde_json::Map<String, Value> = ErrorCode::ALL
        .into_iter()
        .map(|c| {
            (
                c.name().to_owned(),
                json!({"code": c.code(), "message": c.name()}),
            )
        })
        .collect();
    json!({"openrpc": "1.3.2", "info": {"title": "shed", "version": VERSION}, "methods": methods,
        "components": {"schemas": g.take_definitions(true), "errors": errors}})
}

fn describe(method: Method, g: &mut SchemaGenerator) -> Value {
    let [params, result] = method.schemas(g);
    let required = params.get("required").and_then(Value::as_array);
    let descriptors: Vec<Value> = params
        .get("properties")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .map(|(name, schema)| {
            let required = required.is_some_and(|r| r.contains(&name.as_str().into()));
            json!({"name": name, "required": required, "schema": schema})
        })
        .collect();
    let extras = [
        (
            "x-params",
            params.get("oneOf").map(|_| Value::from(params.clone())),
        ),
        ("x-stream", method.stream(g).map(Value::from)),
    ];
    let mut doc = json!({
        "name": method.name(),
        "description": method.summary(),
        "paramStructure": "by-name",
        "params": descriptors,
        "result": {"name": "result", "schema": result},
    });
    if let Value::Object(map) = &mut doc {
        map.extend(
            extras
                .into_iter()
                .filter_map(|(k, v)| Some((k.to_owned(), v?))),
        );
    }
    doc
}
