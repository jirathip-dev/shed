//! shed's API types and JSON-RPC method list.
//!
//! The wire format is newline-delimited JSON-RPC 2.0 over a Unix socket: one [`Request`] per line, one
//! [`Response`] per line. Every connection starts with [`Method::Hello`]; a client and server of
//! different products or of protocol ranges that don't overlap are refused with [`ErrorCode::Version`]. `docs/protocol.md` is generated
//! from these types (`SHED_BLESS=1 cargo test -p shed-proto --features schema`); CI fails when it drifts.
#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::path::PathBuf;

#[cfg(feature = "schema")]
use schemars::generate::SchemaSettings;
#[cfg(feature = "schema")]
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(feature = "schema")]
use serde_json::json;

mod code;
pub use code::{Kind, Lang, OutlineParams, OutlineResult, Span, Symbol};

/// The shed version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The git commit this build was made from, or `unknown` outside a git checkout.
pub const BUILD: &str = env!("SHED_BUILD");

/// `<semver> (<sha>)`, as `shed --version`, `shed hello` and the server's ready line print it.
pub const LONG_VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), " (", env!("SHED_BUILD"), ")");

/// The wire protocol version; bump it with any change to the messages.
pub const PROTOCOL: u32 = 2;

/// The oldest protocol this build still speaks: a server answers a client on it in its shapes, a client accepts a server on it.
pub const OLDEST_PROTOCOL: u32 = 1;

/// The JSON-RPC version string every message carries.
pub const JSONRPC: &str = "2.0";

/// A JSON-RPC request. Without an `id` it is a notification and gets no response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
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

    /// The protocol refusal: `SHED_VERSION: client <sha> protocol <a>-<b> / server <sha> protocol <c>-<d>; update the <side>`.
    #[must_use]
    pub fn version(client: &Hello, server: &Hello) -> Self {
        let side = if client.protocol < server.oldest() {
            "the client"
        } else {
            "the server and restart shed serve"
        };
        Self::new(
            ErrorCode::Version,
            format!(
                "client {} protocol {}-{} / server {} protocol {}-{}; update {side}",
                client.label(),
                client.oldest(),
                client.protocol,
                server.label(),
                server.oldest(),
                server.protocol
            ),
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
    /// `exec` or a file call failed: not found, denied, timed out, or its reply over [`REPLY_LIMIT`].
    Io,
    /// A person's unsent line held the `send` past `draft_wait`; nothing was written.
    DraftHeld,
    /// The server will not hand over (`handover.check`, `handover`); the message says why, and nothing changed.
    HandoverRefused,
    /// A `code.*` query went over one of its fixed limits; the message names it, its value and where it stood.
    Limit,
}

impl ErrorCode {
    /// Every code, in documentation order.
    pub const ALL: [Self; 17] = [
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
        Self::Io,
        Self::DraftHeld,
        Self::HandoverRefused,
        Self::Limit,
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
            Self::Io => -32008,
            Self::DraftHeld => -32009,
            Self::HandoverRefused => -32010,
            Self::Limit => -32011,
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
            Self::Io => "IO",
            Self::DraftHeld => "DRAFT_HELD",
            Self::HandoverRefused => "HANDOVER_REFUSED",
            Self::Limit => "LIMIT",
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
    /// Run a program without a terminal: [`ExecParams`] → [`ExecResult`].
    Exec,
    /// Read a file: [`FileParams`] → its bytes.
    FileRead,
    /// Write a file: [`FileParams`] with `data` → `{}`.
    FileWrite,
    /// A file's symbols: [`OutlineParams`] → [`OutlineResult`].
    CodeOutline,
    /// Keep the connection alive: `{}` → `{}`, usually as a notification.
    Heartbeat,
    /// The agents shed knows: `{}` → `[AgentInfo]`.
    Agents,
    /// Apply an agent's limit and auth rules to text: [`MatchParams`] → [`Matched`].
    AgentsMatch,
    /// The API as an `OpenRPC` document: `{}` → `openrpc()` (the `schema` feature).
    Discover,
    /// Would the server hand over to a binary: [`HandoverCheckParams`] → [`Handover`].
    HandoverCheck,
    /// Hand over to the server's own binary: [`HandoverParams`] → [`Handover`].
    Handover,
}

impl Method {
    /// Every method, in documentation order.
    pub const ALL: [Self; 33] = [
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
        Self::Exec,
        Self::FileRead,
        Self::FileWrite,
        Self::CodeOutline,
        Self::Heartbeat,
        Self::Agents,
        Self::AgentsMatch,
        Self::Discover,
        Self::HandoverCheck,
        Self::Handover,
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
            Self::Exec => "exec",
            Self::FileRead => "file.read",
            Self::FileWrite => "file.write",
            Self::CodeOutline => "code.outline",
            Self::Heartbeat => "heartbeat",
            Self::Agents => "agents",
            Self::AgentsMatch => "agents.match",
            Self::Discover => "rpc.discover",
            Self::HandoverCheck => "handover.check",
            Self::Handover => "handover",
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
                "The handshake; must come first on every connection. Another product, or a protocol range that does not overlap the server's, is refused with `SHED_VERSION`."
            }
            Self::Run => {
                "Start a terminal. `name` defaults to the first free `tN`; `keep` brings it back as a shell in its cwd after a server restart; `rows` and `cols` default to `$SHED_SIZE` (`RxC`) of the server, else 50x200. ACP: `mcp_servers` (stdio, `http`, `sse`, in ACP's shapes) go to the agent; their env and header values are never saved or shown. `meta` (an object) goes unchanged as the session's `_meta`. `no_hibernate` keeps the agent running when idle. `mem_max_mb` (default the server's `$SHED_MEM_MAX`, e.g. `4G`) limits the process tree's memory, and a `mem-limit` event (with `notify`, a `[shed] <name> mem-limit …` line to the parent) says what was ended: on Linux with a user systemd that has the memory controller the program runs in its own scope with `MemoryMax` and the kernel ends a process (the agent's children are ranked first); elsewhere, once the tree is over for two samples in a row, the sampler ends the largest process the agent started (SIGTERM, SIGKILL 2 s later), and an agent over it by itself is only reported. The sampler is a guard against sustained use, not a hard limit: a fast allocation can pass it between samples. `warm` takes the ready spare session of the same recipe (agent, command, cwd, policy, MCP servers, meta, size), renamed to `name` with these labels, and starts the next spare; spares are listed as `.warm-N`, at most `$SHED_WARM_MAX` (default 2) of them, each killed after `$SHED_WARM_IDLE` seconds (default 600) unused. `parent` links it to the terminal that started it: when the parent's program ends the child is killed too, unless `outlive`; with `notify` a `[shed] <name> blocked|error|exited …` line is typed into the parent (a prompt for an ACP parent) when the child blocks, errors or ends, once per piece of news, held while the parent reports `working`."
            }
            Self::Ls => {
                "List terminals, by name; `labels` keeps those that carry every given label, `parent` the children of that terminal."
            }
            Self::Get => "One terminal.",
            Self::Read => {
                "The visible screen, or with `history` the scrollback and screen, as plain text; `lines` keeps the last N lines. Every read returns a `cursor`; with `since: <cursor>` only what is new: the lines that scrolled off since (`lost: N` when scrollback dropped N of them first), then the screen only when it changed (`changed`), so nothing new is an empty `text`. ACP: the transcript as text, or with `events` its events (each with its `seq`), paged by `before` and `n`, or with `since` only those after its `seq`."
            }
            Self::Send => {
                "Paste text into the terminal, then press enter: bracketed when the program has turned bracketed paste on, unless `no_paste`; no enter with `no_enter`. While a person has an unsent line (typed through `terminal.input`), the send waits until they press enter or clear it (`ctrl+u`, `ctrl+c`), or `DRAFT_HELD` once `draft_wait` seconds (default 60, 0 off) pass after their last key, writing nothing. Sends go in one at a time, each whole. ACP: the text goes to the bridge as it is."
            }
            Self::Keys => {
                "Press keys by name (`enter`, `esc`, `ctrl+c`, `up`, …); nothing is sent if any name is unknown."
            }
            Self::Kill => {
                "End the terminal's whole process tree (SIGTERM, then SIGKILL); returns the exited terminal."
            }
            Self::Rm => "Remove an exited terminal, its ACP session closed; returns it as it was.",
            Self::Resize => "Resize the terminal; the program gets SIGWINCH.",
            Self::Attach => {
                "Take over the terminal. After the response the server writes a redraw (clear, then the screen text) and the program's raw output, and closes the connection when the program's PTY closes. The client sends only notifications from then on (`terminal.input`, `keys`, `resize`); detaching is closing the connection."
            }
            Self::State => {
                "Report the terminal's state (`working`, `blocked`, `done` or `error`) with an optional note; a program can also print `ESC ] 7501 ; state=<s>[;note=<n>] BEL`, which never reaches the screen. The terminal also speaks the Program Status Protocol (OSC 7501, revision 0.4): `state=<s>[:<key>=<value>]…` with `idle` and `clear`, `id`, `kind`, `progress`, `app` and base64 `title` and `msg`, and answers the probe `ESC ] 7501 ; ?`. A report with an `id` is a record of its own (`records`); on exit `working` and `blocked` ones are dropped."
            }
            Self::Wait => {
                "Answer once the reported state is one of `until` (`idle` too, from the `idle_after` fallback; `blocked` only of `kind`, if given) or a visible screen line matches the pattern `output` (`matched`), the program exits (`exited`) or `timeout` seconds pass (`timeout`). The server blocks; nothing polls. With `children`, `name` is a parent: `all` matches once no child is working (`terminal` is the parent), `any` on the next child that reports `done`, `blocked` or `error`, or exits (`terminal` is that child; `exited` when no child runs)."
            }
            Self::Events => {
                "Stream events. After the response the server writes one event per line (see Events) for the terminals carrying every one of `labels`, first those after `since` still kept (the last 4,096 events other than `text`, and each terminal's last 256 `text` chunks), and never closes the connection; closing it unsubscribes. When events after `since` are gone it first writes `{\"jsonrpc\":\"2.0\",\"method\":\"reset\",\"params\":{\"seq\":N}}`: those up to `N` are lost. A client more than 4,096 events behind gets `{\"jsonrpc\":\"2.0\",\"method\":\"lagged\",\"params\":{\"seq\":N}}` and the connection is closed; it resumes with `since`. A connection that sent `heartbeat` gets `{\"jsonrpc\":\"2.0\",\"method\":\"heartbeat\"}` lines when idle."
            }
            Self::Emit => {
                "Emit one of an ACP terminal's structured events (`turn`, `text`, `tool_call`, `plan`, `mode`, `approval_request`, `approved`, `reply`); its bridge (`shed run --acp`) calls this. Other events are refused."
            }
            Self::ScreenWatch => {
                "Stream a running terminal's screen. After the response the server writes screen updates (see Screen updates), at most 20 a second: the full screen first (none when `since` is the screen's current `seq`), then the rows that changed. It closes the connection when the program's PTY closes. The connection still takes requests; their responses are not written."
            }
            Self::Input => {
                "Type text into the terminal as a person: written as it is, and a line left unsent holds `send`s until enter, `ctrl+u` or `ctrl+c`."
            }
            Self::TerminalKeys => {
                "Press keys by name as a person, as `keys`; `ctrl-x` is `ctrl+x`. Like `terminal.input`, a line left unsent holds `send`s."
            }
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
            Self::Exec => {
                "Run a program without a terminal in a process group of its own, `stdin` fed to it, and answer with its exit code (128 + the signal when one ended it) and the exact bytes it wrote. The group is ended when the program returns or `timeout` seconds pass (`IO`); output over `REPLY_LIMIT` bytes is `IO`, never cut. `machine` runs it there, over the held link."
            }
            Self::FileRead => {
                "A file's bytes, or with `lines` only that span's; over `REPLY_LIMIT` bytes is `IO`. `machine` reads it there."
            }
            Self::FileWrite => {
                "Replace a file with `data`: written to a temp file beside it, then renamed over it, its mode kept. `machine` writes it there."
            }
            Self::CodeOutline => {
                "A file's functions, types, sections and blocks in file order, each with its kind, name, lines and depth, by line rules for Rust, TypeScript/JavaScript, Python, Markdown and TOML and by indentation for other files. Over 2,000 symbols or 256 MiB is `LIMIT`. `machine` reads it there."
            }
            Self::Heartbeat => {
                "Keep the connection alive. Once a connection sends one, the server closes it when nothing comes from it for 30 s (two missed beats of 15 s), and writes a `heartbeat` line every 15 s into its idle `events` stream. A write the client does not take within 30 s also closes any connection."
            }
            Self::Agents => {
                "Each agent shed has a provider record for or has installed, by id, as `shed agents --json` prints it: the record (its limit and login-expired lines, hooks and program facts), its `source` (`built-in` or the user file in `<config>/providers/` that adds or overrides it) and the installed adapter. A bad user file is skipped (`shed doctor` names it)."
            }
            Self::AgentsMatch => {
                "Apply agent `id`'s limit and auth rules to `text` (a pane's screen, a reply): the first line that says a usage limit was hit with its reset time, and the first that says the login expired, as an ACP lane would read them. An agent with no record matches nothing."
            }
            Self::Discover => {
                "The whole API as one OpenRPC document, as `shed schema` prints it (`docs/shed.openrpc.json`): every method's params and result, the streamed events and screen updates, and the error codes, as JSON Schema. Needs no `hello`."
            }
            Self::HandoverCheck => {
                "Would the server hand over to the binary at `candidate`: it runs `<candidate> serve --preflight`, which must name `build`, read the state file and inherit this server's handover format, and speak the protocol of every live ACP bridge. Changes nothing; `HANDOVER_REFUSED` says why not."
            }
            Self::Handover => {
                "Hand over to the binary the server was started from, which must be `build`: after the same check as `handover.check` it answers, waits up to 10 s for the other requests in flight, then replaces itself in place on the same process id, keeping the socket and every running terminal (screen, children and exit statuses). Every connection is closed: a closed connection means reconnect and ask again; `events` with `since` gets `reset`. A refused or failed handover leaves the server serving as it was."
            }
        }
    }
}

/// `hello` params and result: the sender's product, version, build and protocol range.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Hello {
    /// The product, `shed`; a peer that sends none is refused like another protocol.
    #[serde(default)]
    pub product: String,
    /// The shed version.
    pub version: String,
    /// The git commit the sender was built from.
    #[serde(default)]
    pub build: String,
    /// The sender's [`PROTOCOL`]; a peer that sends none counts as protocol 0. A server answers a client on an older protocol it still serves with that protocol.
    #[serde(default)]
    pub protocol: u32,
    /// The oldest protocol the sender still speaks: a server's oldest served, a client's oldest accepted server; none means `protocol` alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oldest: Option<u32>,
}

impl Hello {
    /// This build's hello: `shed`, [`VERSION`], [`BUILD`], [`PROTOCOL`] and [`OLDEST_PROTOCOL`].
    #[must_use]
    pub fn ours() -> Self {
        Self {
            product: "shed".to_owned(),
            version: VERSION.to_owned(),
            build: BUILD.to_owned(),
            protocol: PROTOCOL,
            oldest: Some(OLDEST_PROTOCOL),
        }
    }

    /// The server's answer to a compatible `client`: ours, in the client's shapes when it speaks an older protocol.
    #[must_use]
    pub fn answer(client: &Self) -> Self {
        if client.protocol < PROTOCOL {
            Self {
                protocol: client.protocol,
                oldest: None,
                ..Self::ours()
            }
        } else {
            Self::ours()
        }
    }

    /// Whether a peer that sent this hello is `shed` with a protocol range that overlaps ours.
    #[must_use]
    pub fn compatible(&self) -> bool {
        self.product == "shed" && self.oldest() <= PROTOCOL && OLDEST_PROTOCOL <= self.protocol
    }

    fn oldest(&self) -> u32 {
        self.oldest.unwrap_or(self.protocol)
    }

    /// The build's sha (`unknown` when it sent none), after the product unless that is `shed`.
    #[must_use]
    pub fn label(&self) -> String {
        let build = if self.build.is_empty() {
            "unknown"
        } else {
            &self.build
        };
        match self.product.as_str() {
            "shed" => build.to_owned(),
            "" => format!("unnamed {build}"),
            product => format!("{product} {build}"),
        }
    }
}

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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct KeysParams {
    /// The terminal's name (`terminal` in `terminal.keys`).
    #[serde(alias = "terminal")]
    pub name: String,
    /// Key names, pressed in order.
    pub keys: Vec<String>,
}

/// `resize` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum EventKind {
    /// The program started (`run`, or a kept terminal back after a server restart).
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct ModelParams {
    /// The terminal's name.
    pub name: String,
    /// The model id, one of the session's `models`.
    pub model: String,
}

/// The most bytes one `exec` or `file.read` reply carries: 8 MiB.
pub const REPLY_LIMIT: u64 = 8 << 20;

/// `exec` params.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct ExecParams {
    /// The program and its arguments.
    pub cmd: Vec<String>,
    /// Its working directory; default the server's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    /// End it after this many seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<f64>,
    /// Bytes fed to its stdin, then closed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stdin: Vec<u8>,
    /// Run it on this machine, over its held link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
}

/// `exec` result.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct ExecResult {
    /// The exit code, or 128 + the signal that ended it.
    pub code: i32,
    /// What it wrote to stdout.
    pub stdout: Vec<u8>,
    /// What it wrote to stderr.
    pub stderr: Vec<u8>,
}

/// `file.read` and `file.write` params.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct FileParams {
    /// The file.
    pub path: PathBuf,
    /// `file.write`: its new bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Vec<u8>>,
    /// `file.read`: only these lines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lines: Option<Span>,
    /// The machine it is on, over its held link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
}

/// `machine.stats` params.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct StatsParams {
    /// Add the kept samples taken after this time (unix seconds) as `history`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<u64>,
}

/// One sample of a machine's health.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Link {
    /// Up, reconnecting or down.
    pub state: LinkState,
    /// Since when, unix seconds.
    pub since: u64,
    /// Why it is down: the last connect's error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// What shed knows about one agent, as data: a built-in file, overridden key by key, or added, by a file of
/// the same name in `<config>/providers/` (`docs/acp.md`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct Provider {
    /// The ACP agent id (`shed agents add <id>`).
    pub id: String,
    /// The PTY agent's name, as `shed setup` takes it.
    pub agent: String,
    /// Text in a line that says the login expired.
    pub auth_failures: Vec<String>,
    /// Text in a line that says a usage limit was hit.
    pub limit: Vec<String>,
    /// Markers in a limit line that the reset time follows.
    pub resets: Vec<String>,
    /// The `shed state` hooks `shed setup` writes.
    pub hooks: Option<Hooks>,
    /// The command that logs in.
    pub login: Option<String>,
    /// The command that shows whether it is logged in.
    pub status: Option<String>,
    /// The command that logs out.
    pub logout: Option<String>,
    /// The variable that moves its config folder.
    pub config_dir_var: Option<String>,
    /// The variable that names its settings.
    pub settings_var: Option<String>,
    /// The arguments that set the effort, `{effort}` standing for one of `efforts`.
    pub effort_args: Vec<String>,
    /// The effort levels it takes.
    pub efforts: Vec<String>,
    /// The screens that set its state, for a terminal labelled `agent=<agent>`.
    pub rules: Vec<Rule>,
    /// Where its prompt responses' `_meta` carries the turn's tokens.
    pub quota: Option<Quota>,
    /// How long its prompt cache lives on each login.
    pub cache: Option<Cache>,
    /// Which driver runs its lanes.
    pub driver: DriverKind,
    /// The stream driver's data: the agent's own JSON-lines mode.
    pub stream: Option<Stream>,
}

/// The driver a provider's lanes run on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum DriverKind {
    /// Its ACP adapter.
    #[default]
    Acp,
    /// Its own JSON-lines mode, by the record's `stream` table, when a lane can be served whole.
    Stream,
}

/// How the stream driver starts an agent in its own JSON-lines mode.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct Stream {
    /// The provider's line format: `providers/<wire>.rs`.
    pub wire: String,
    /// The agent's command line, before the arguments of one start.
    pub command: Vec<String>,
    /// Text that names the ACP adapter when it is the last argument of a caller's own command.
    pub adapter: Option<String>,
    /// The words an adapter is started with; `command` replaces them and the adapter.
    pub launchers: Vec<String>,
    /// The arguments that start a session under the id shed made (`{session}`).
    pub new: Vec<String>,
    /// The arguments that start the agent on a session it has (`{session}`).
    pub resume: Vec<String>,
    /// The arguments that set the model (`{model}`).
    pub model: Vec<String>,
    /// The models `shed model` offers: id and name.
    pub models: Vec<[String; 2]>,
    /// The arguments that pass the MCP servers (`{file}`: a private file with the wire's text).
    pub mcp: Vec<String>,
    /// Each policy's arguments; a policy without an entry stays on ACP.
    pub policy: BTreeMap<String, Vec<String>>,
    /// Each `--meta` option's flag, by its dotted path; an option without an entry stays on ACP.
    pub meta: BTreeMap<String, String>,
    /// Each tool's ACP kind and the input key that is its title.
    pub tools: BTreeMap<String, [String; 2]>,
    /// The tool, and its input key, whose list is the plan.
    pub plan: Option<[String; 2]>,
}

/// The prompt cache's lifetime on a plan and on an API key, and the variables that mean an API key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Cache {
    /// Seconds on a subscription plan, the login without an API variable.
    pub plan: u64,
    /// Seconds on an API key.
    pub api: u64,
    /// Variables that, set and not empty in the agent's environment, mean an API key.
    pub api_vars: Vec<String>,
}

/// A screen rule: while one of `any` shows in the last `region` non-blank lines of the screen, the state is
/// `state`. Above priority 0 it outranks the agent's own report; at or below, it applies only without one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Rule {
    /// Its name; `get` shows `rule:<id>` as the state's `from`.
    pub id: String,
    /// The state it sets.
    pub state: String,
    /// Why it is blocked: `permission`, `question` or `auth`.
    #[serde(default)]
    pub kind: Option<String>,
    /// The highest matching rule wins; the agent's own report has priority 0.
    pub priority: i32,
    /// How many of the screen's last non-blank lines it looks at.
    pub region: usize,
    /// The texts it matches, any one of them.
    pub any: Vec<String>,
    /// The agent version its golden screen was captured from.
    pub version: String,
}

/// The keys of a turn's tokens in a prompt response's `_meta`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Quota {
    /// The keys from `_meta` down to the object of counters.
    pub path: Vec<String>,
    /// The input tokens' key.
    pub input: String,
    /// The cache reads' key.
    pub cache_read: String,
    /// The cache writes' key.
    pub cache_write: String,
    /// The output tokens' key.
    pub output: String,
}

impl Provider {
    /// The first line of `text` that says a usage limit was hit, and the reset time after a marker in it.
    #[must_use]
    pub fn limit_hit<'t>(&self, text: &'t str) -> Option<(&'t str, Option<String>)> {
        let line = text
            .lines()
            .find(|l| self.limit.iter().any(|s| l.contains(s)))?;
        let at = self.resets.iter().find_map(|r| line.split_once(r));
        Some((
            line,
            at.map(|(_, at)| at.trim().trim_end_matches('.').to_owned()),
        ))
    }

    /// The turn's tokens in a prompt response's `meta`, or `None` when they are absent or not counts.
    #[must_use]
    pub fn tokens(&self, meta: &serde_json::Map<String, Value>) -> Option<Tokens> {
        let quota = self.quota.as_ref()?;
        let (first, rest) = quota.path.split_first()?;
        let counts = rest.iter().try_fold(meta.get(first)?, |v, k| v.get(k))?;
        let count = |key: &String| counts.get(key).and_then(Value::as_u64);
        Some(Tokens {
            input: count(&quota.input)?,
            cache_read: count(&quota.cache_read)?,
            cache_write: count(&quota.cache_write)?,
            output: count(&quota.output)?,
        })
    }

    /// The prompt cache's likely lifetime in seconds, by whether `set` says one of the API variables is set.
    #[must_use]
    pub fn cache_lifetime(&self, set: impl Fn(&str) -> bool) -> Option<u64> {
        let cache = self.cache.as_ref()?;
        let api = cache.api_vars.iter().any(|v| set(v));
        Some(if api { cache.api } else { cache.plan })
    }

    /// The first line of `text` that says the login expired.
    #[must_use]
    pub fn auth_failure<'t>(&self, text: &'t str) -> Option<&'t str> {
        text.lines()
            .find(|l| self.auth_failures.iter().any(|s| l.contains(s)))
    }
}

/// A PTY agent's hook table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Hooks {
    /// Its config file, under the home folder.
    pub file: String,
    /// Each hook event and the state it reports.
    pub events: Vec<[String; 2]>,
}

/// One installed ACP agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Agent {
    /// The registry id.
    pub id: String,
    /// The pinned version.
    pub version: String,
    /// The distribution it came from: `binary`, `npx` (installed once with npm, run by `node`) or `uvx`.
    pub distribution: String,
    /// The ACP adapter's command line (`env K=V …` first when the registry sets variables).
    pub cmd: Vec<String>,
}

/// One `agents` entry: an agent shed has a provider record for, installed, or both.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct AgentInfo {
    /// The ACP agent id.
    pub id: String,
    /// Where the record came from: `built-in` or the user file's path.
    pub source: Option<String>,
    /// The provider record.
    pub provider: Option<Provider>,
    /// The installed adapter.
    pub installed: Option<Agent>,
    /// The medians of its last sessions' timings, when any session got a first token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub medians: Option<Medians>,
}

/// An agent's medians over its last `$SHED_MEDIAN_OF` (default 20) ACP sessions that got a first token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Medians {
    /// How many sessions they are over.
    pub sessions: u64,
    /// The median of [`AcpSession::ready_ms`].
    pub ready_ms: u64,
    /// The median of [`AcpSession::first_token_ms`].
    pub first_token_ms: u64,
}

/// `agents.match` params: an agent's id and text to apply its rules to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct MatchParams {
    /// The ACP agent id.
    pub id: String,
    /// The text, e.g. a pane's screen.
    pub text: String,
}

/// `agents.match` result: the lines the agent's rules find.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Matched {
    /// The first line that says a usage limit was hit (the state `rate-limited`).
    pub limit: Option<String>,
    /// The reset time that line gives.
    pub resets: Option<String>,
    /// The first line that says the login expired.
    pub auth: Option<String>,
}

/// `machine.stats` result: a sample taken now, and the kept ones after `since`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct MachineStats {
    /// Now.
    #[serde(flatten)]
    pub now: Sample,
    /// The kept samples (one a minute, 24 h) after `since`, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<Sample>,
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
/// `outlive` and `notify` as `run` set them.
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
}

/// `screen.watch` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct WatchParams {
    /// The terminal's name.
    pub terminal: String,
    /// The `seq` of the last update the client applied: no full screen first if nothing changed since.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<u64>,
}

/// `screen.watch` writes one update per line. `seq` is the screen's change count; `rows`, `cols`, `cursor`
/// (`[row, col]`, from 0) and `scrollback` (lines) are always there; `lines` holds the rows that changed, each
/// as runs of cells with one pen: `fg` and `bg` (a palette index 0–255, or `[r, g, b]`; `null` is the default),
/// `bold`, `italic`, `underline` and `inverse`. Trailing blank default cells are dropped, so a blank row has no
/// runs. `full` marks an update that carries every row: the first, and the one after a resize.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Line {
    /// The row, from 0.
    pub row: u16,
    /// Its cells, in runs of the same pen; trailing blank default cells dropped.
    pub runs: Vec<Run>,
}

/// Consecutive cells with the same pen; `null` colours are the terminal's defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(untagged)]
pub enum Color {
    /// A palette index.
    Index(u8),
    /// Red, green, blue.
    Rgb([u8; 3]),
}

/// `terminal.resize` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct TerminalResizeParams {
    /// The terminal's name; default the one this connection watches.
    #[serde(default, alias = "name", skip_serializing_if = "Option::is_none")]
    pub terminal: Option<String>,
    /// Columns, at least 1.
    pub cols: u16,
    /// Rows, at least 1.
    pub rows: u16,
}

/// `handover.check` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct HandoverCheckParams {
    /// The build the binary must be.
    pub build: String,
    /// The binary's path.
    pub candidate: PathBuf,
}

/// `handover` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct HandoverParams {
    /// The build the server's own binary must be now.
    pub build: String,
}

/// The answer to `handover.check` and `handover`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Handover {
    /// The server's build.
    pub from: String,
    /// The binary's build.
    pub to: String,
    /// The running terminals that cross.
    pub terminals: usize,
}

/// `terminal.readonly` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct TerminalParams {
    /// The terminal's name.
    pub terminal: String,
}

#[cfg(feature = "schema")]
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
            Self::Exec => pair::<ExecParams, ExecResult>(g),
            Self::FileRead => pair::<FileParams, Vec<u8>>(g),
            Self::FileWrite => pair::<FileParams, Empty>(g),
            Self::CodeOutline => pair::<OutlineParams, OutlineResult>(g),
            Self::Heartbeat => pair::<Empty, Empty>(g),
            Self::Agents => pair::<Empty, Vec<AgentInfo>>(g),
            Self::AgentsMatch => pair::<MatchParams, Matched>(g),
            Self::Discover => [Empty::json_schema(g), Schema::from(true)],
            Self::HandoverCheck => pair::<HandoverCheckParams, Handover>(g),
            Self::Handover => pair::<HandoverParams, Handover>(g),
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

#[cfg(feature = "schema")]
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
struct Empty {}

#[cfg(feature = "schema")]
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

#[cfg(feature = "schema")]
/// The whole API as one `OpenRPC` document (`shed schema`, `rpc.discover`): every method's params and result,
/// the streamed [`Event`]s and [`ScreenUpdate`]s (`x-stream`) and the error codes, as JSON Schema.
#[must_use]
pub fn openrpc() -> Value {
    let mut g = SchemaSettings::draft2020_12()
        .with(|s| s.definitions_path = "/components/schemas".into())
        .into_generator();
    let methods: Vec<Value> = Method::ALL.map(|m| describe(m, &mut g)).into();
    let record = g.subschema_for::<SessionRecord>();
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
        "x-files": {"sessions/<id>.json": record}, "components": {"schemas": g.take_definitions(true), "errors": errors}})
}

#[cfg(feature = "schema")]
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
