//! ACP client mode. `shed run --acp` starts [`bridge`] as an ordinary terminal's program: it runs the
//! agent's ACP adapter on stdio, speaks the Agent Client Protocol (v1) to it through the
//! `agent-client-protocol` SDK, and turns the session into shed events (`emit`), reported states and a
//! JSONL transcript that `read` renders.
//!
//! The bridge reads its PTY in raw mode: a line framed by [`frame`] (`send`, `approve`, `cancel`,
//! `model`), a plain line typed in `attach` (a prompt), or `Esc` / `Ctrl+C` alone (cancel).
//!
//! Its own tokio runtime runs the SDK connection and the session task on this thread and, on one worker
//! beside it, a task reading the PTY into [`Input`]s and one draining the session's outputs into the
//! transcript, the terminal and the server (as notifications), a batch at a time.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::{self, File};
use std::io::{self, BufWriter, Write as _};
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::v1::schema::{
    AvailableCommandInput, CancelNotification, ContentBlock, ContentChunk, Implementation,
    InitializeRequest, LoadSessionRequest, McpServer, McpServerHttp, McpServerSse,
    NewSessionRequest, PromptRequest, PromptResponse, RequestPermissionOutcome,
    RequestPermissionRequest, RequestPermissionResponse, SelectedPermissionOutcome,
    SessionConfigKind as Kind, SessionConfigOption, SessionConfigOptionCategory as Category,
    SessionConfigOptionValue, SessionConfigSelectOption, SessionConfigSelectOptions as Choices,
    SessionId, SessionNotification, SessionUpdate, SetSessionConfigOptionRequest, TextContent,
};
use agent_client_protocol::v1::{ConnectionTo, Responder};
use agent_client_protocol::{Agent, ErrorCode, JsonRpcMessage as _, Lines, UntypedMessage};
use futures::{sink, stream};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use shed_proto::{
    AcpSession, ApprovalOption, EmitParams, EventKind, Method, Reported, StateParams,
};
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::{Client, Error};

type AcpResult<T> = Result<T, agent_client_protocol::Error>;
type Asked = Responder<RequestPermissionResponse>;

pub use shed_proto::Policy;

/// One framed input to the bridge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Input {
    /// Start a turn (`session/prompt`).
    Prompt(String),
    /// Answer the oldest permission request with this option id or kind.
    Approve(String),
    /// Cancel the turn (`session/cancel`).
    Cancel,
    /// Switch to this model (`session/set_config_option`, the `model` option).
    Model(String),
}

/// The bytes that deliver `input` to a bridge through its terminal: `RS`, the JSON, a newline.
#[must_use]
pub fn frame(input: &Input) -> String {
    // An enum of strings always serialises.
    format!("\x1e{}\n", serde_json::to_string(input).unwrap_or_default())
}

/// The transcript of terminal `name` of the server whose state file is `state`.
#[must_use]
pub fn transcript(state: &Path, name: &str) -> PathBuf {
    state.with_file_name("acp").join(format!("{name}.jsonl"))
}

/// Control characters (but newlines and tabs) as `?`, so agent text cannot drive a terminal.
fn clean(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\n' | '\t' => c,
            c if c.is_control() => '?',
            c => c,
        })
        .collect()
}

/// A transcript (JSONL events) as text: each prompt (`> …`), one line per tool call with its last
/// status, and each turn's final reply.
#[must_use]
pub fn render(transcript: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut tools: BTreeMap<String, usize> = BTreeMap::new();
    for kind in transcript
        .lines()
        .filter_map(|l| serde_json::from_str::<EventKind>(l).ok())
    {
        match kind {
            EventKind::Turn { prompt } => lines.push(format!("> {prompt}")),
            EventKind::ToolCall {
                id, title, status, ..
            } => {
                let line = format!("[tool] {title} ({status})");
                if let Some(&at) = tools.get(&id) {
                    lines[at] = line;
                } else {
                    tools.insert(id, lines.len());
                    lines.push(line);
                }
            }
            EventKind::Reply { text, .. } if !text.is_empty() => lines.push(text),
            _ => {}
        }
    }
    clean(&lines.join("\n"))
}

/// A transcript drawn for a reader: each block styled and wrapped, every tool call on one line at its
/// first place with its last status. Only ACP's own fields are read; an unknown status gets no glyph.
#[derive(Default)]
pub struct Preview {
    blocks: Vec<Result<EventKind, String>>,
    tools: BTreeMap<String, usize>,
}

impl Preview {
    /// Adds an event; `false` when it draws nothing.
    pub fn push(&mut self, kind: EventKind) -> bool {
        match &kind {
            EventKind::ToolCall { id, .. } if self.tools.contains_key(id) => {
                let at = self.tools[id];
                self.blocks[at] = Ok(kind);
                return true;
            }
            EventKind::ToolCall { id, .. } => _ = self.tools.insert(id.clone(), self.blocks.len()),
            EventKind::Reply { text, .. } if text.is_empty() => return false,
            EventKind::Turn { .. }
            | EventKind::Reply { .. }
            | EventKind::Plan { .. }
            | EventKind::ApprovalRequest { .. }
            | EventKind::Approved { .. } => {}
            _ => return false,
        }
        self.blocks.push(Ok(kind));
        true
    }

    /// The last `rows` lines, `width` columns wide, with SGR colours when `color`.
    #[must_use]
    pub fn draw(&self, width: usize, color: bool, rows: usize) -> Vec<String> {
        let mut lines = Vec::new();
        for block in self.blocks.iter().rev() {
            if lines.len() >= rows {
                break;
            }
            let mut out = Vec::new();
            let mut put = |sgr: &str, first: &str, rest: &str, text: &str| {
                let room = width.saturating_sub(first.chars().count()).max(1);
                let text = clean(text);
                for (i, l) in text.split('\n').flat_map(|l| wrap(l, room)).enumerate() {
                    let lead = if i == 0 { first } else { rest };
                    out.push(match (color, sgr) {
                        (false, _) => format!("{lead}{l}"),
                        (true, "") => format!("{lead}{}", code(&l)),
                        (true, _) => format!("{lead}\x1b[{sgr}m{l}\x1b[0m"),
                    });
                }
            };
            match block {
                Err(line) => put("", "", "", line),
                Ok(EventKind::Turn { prompt }) => put("1", "› ", "  ", prompt),
                Ok(EventKind::Reply { text, .. }) => markdown(text, &mut put),
                Ok(EventKind::ToolCall {
                    title,
                    status,
                    kind,
                    ..
                }) => {
                    let glyph = mark(status).0;
                    let kind = kind.as_deref().map(|k| format!("{k} ")).unwrap_or_default();
                    let line = format!("{glyph}{kind}{}", title.lines().next().unwrap_or(""));
                    put("2", "", "", &line.chars().take(width).collect::<String>());
                }
                Ok(EventKind::Plan { entries }) => {
                    for e in entries {
                        let mark = mark(e["status"].as_str().unwrap_or("")).1;
                        put("", mark, "    ", e["content"].as_str().unwrap_or(""));
                    }
                }
                Ok(EventKind::ApprovalRequest { tool, options }) => {
                    let options = options.iter().map(|o| format!("{} ({})", o.id, o.name));
                    let options = options.collect::<Vec<_>>().join(" | ");
                    let tool = tool.lines().next().unwrap_or("");
                    put("33", "? ", "  ", &format!("{tool}: {options}"));
                }
                Ok(EventKind::Approved { option }) => {
                    put("2", "", "", &format!("approved {option}"));
                }
                Ok(_) => {}
            }
            lines.extend(out.into_iter().rev());
        }
        lines.truncate(rows);
        lines.reverse();
        lines
    }
}

/// A status's tool glyph and plan checkbox; an unknown one has no glyph.
fn mark(status: &str) -> (&'static str, &'static str) {
    let at = ["pending", "in_progress", "completed", "failed"]
        .iter()
        .position(|s| *s == status);
    at.map_or(("", "[ ] "), |i| {
        (
            ["… ", "… ", "✓ ", "✗ "][i],
            ["[ ] ", "[~] ", "[x] ", "[!] "][i],
        )
    })
}

/// A reply's markdown, lightly: headings bold, list items indented, fenced code indented and dim
/// under its language.
fn markdown(text: &str, put: &mut impl FnMut(&str, &str, &str, &str)) {
    let mut fence = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        let numbered = trimmed.split_once(". ").map(|(n, _)| n);
        let item = trimmed
            .strip_prefix(['-', '*', '+'])
            .filter(|r| r.starts_with(' '));
        if let Some(lang) = trimmed.strip_prefix("```") {
            fence = !fence;
            if fence && !lang.is_empty() {
                put("2", "    ", "    ", lang);
            }
        } else if fence {
            put("2", "    ", "    ", line);
        } else if let Some(h) = trimmed.strip_prefix('#') {
            put("1", "", "", h.trim_start_matches('#').trim_start());
        } else if let Some(item) = item {
            put("", "  •", "    ", item);
        } else if numbered.is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())) {
            put("", "  ", "     ", trimmed);
        } else {
            put("", "", "", line);
        }
    }
}

/// A line's inline code (`` `x` ``) highlighted.
fn code(line: &str) -> String {
    let parts = line.split('`').enumerate();
    let parts = parts.map(|(i, p)| match i % 2 {
        1 => format!("\x1b[36m{p}\x1b[39m"),
        _ => p.to_owned(),
    });
    parts.collect()
}

/// `line` wrapped at spaces to `width` columns; a longer word is cut.
fn wrap(line: &str, width: usize) -> Vec<String> {
    let mut lines = vec![String::new()];
    for (i, word) in line.split(' ').enumerate() {
        let last = lines.len() - 1;
        if i > 0 && lines[last].chars().count() + 1 + word.chars().count() <= width {
            lines[last].push(' ');
        } else if i > 0 {
            lines.push(String::new());
        }
        for (j, chunk) in word.chars().collect::<Vec<_>>().chunks(width).enumerate() {
            if j > 0 {
                lines.push(String::new());
            }
            let last = lines.len() - 1;
            lines[last].extend(chunk);
        }
    }
    lines
}

/// Columns, rows and colour for drawing to stdout: 80 × 24 unless it is a terminal, and colour only
/// on a terminal without `NO_COLOR`.
#[must_use]
pub fn stdout_style() -> (usize, usize, bool) {
    let tty = rustix::termios::isatty(io::stdout());
    let size = (rustix::termios::tcgetwinsize(io::stdout()).ok()).filter(|_| tty);
    let size = size.map_or((80, 24), |s| (s.ws_col.max(20), s.ws_row.max(2)));
    (
        size.0.into(),
        size.1.into(),
        tty && std::env::var_os("NO_COLOR").is_none(),
    )
}

/// `run`'s `mcp_servers` as the SDK's entries; an error names the first unknown or malformed one.
///
/// # Errors
/// The entry's index and name, never its values.
pub fn mcp_servers(entries: &[Value]) -> Result<Vec<McpServer>, String> {
    let parse = |(i, e): (usize, &Value)| {
        let known = (e.get("type")).is_none_or(|t| ["stdio", "http", "sse"].iter().any(|k| t == k));
        let server = serde_json::from_value(e.clone()).ok().filter(|_| known);
        let name = e.get("name").unwrap_or(&Value::Null);
        server
            .ok_or_else(|| format!("mcp_servers[{i}] {name}: not an ACP stdio, http or sse server"))
    };
    entries.iter().enumerate().map(parse).collect()
}

/// `servers` as JSON, each env and header value as `***`.
fn redacted(servers: &[McpServer]) -> Vec<Value> {
    let mut shown = servers.to_vec();
    for server in &mut shown {
        let values: Vec<&mut String> = match server {
            McpServer::Stdio(s) => s.env.iter_mut().map(|e| &mut e.value).collect(),
            McpServer::Http(McpServerHttp { headers, .. })
            | McpServer::Sse(McpServerSse { headers, .. }) => {
                headers.iter_mut().map(|h| &mut h.value).collect()
            }
            _ => Vec::new(),
        };
        for value in values {
            "***".clone_into(value);
        }
    }
    let shown = serde_json::to_value(shown).unwrap_or_default();
    serde_json::from_value(shown).unwrap_or_default()
}

/// What the connection hands the session task, in the order the agent sent it.
enum FromAgent {
    /// A `session/update`, or why it does not parse (a kind this SDK does not know).
    Update(Result<Box<SessionUpdate>, String>),
    Permission(Box<RequestPermissionRequest>, Asked),
    /// The response to our prompt number `.0`.
    Prompt(u64, AcpResult<PromptResponse>),
    /// The config options after `session/set_config_option`.
    Config(AcpResult<Vec<SessionConfigOption>>),
}

/// A `session/update`'s update: a text chunk (most of a turn) read straight from the JSON, else typed.
fn update(params: &Value) -> Result<Box<SessionUpdate>, String> {
    let u = &params["update"];
    let chunk = u["sessionUpdate"] == "agent_message_chunk" && u["content"]["type"] == "text";
    if let Some(text) = u["content"]["text"].as_str().filter(|_| chunk) {
        let chunk = ContentChunk::new(text.into());
        return Ok(Box::new(SessionUpdate::AgentMessageChunk(chunk)));
    }
    let n = SessionNotification::parse_message("session/update", params);
    n.map(|n| Box::new(n.update)).map_err(|e| e.to_string())
}

/// What the session task hands the writer task, in order.
enum Out {
    /// An event; `true` when replayed, then `true` to keep it in the transcript.
    Emit(EventKind, bool, bool),
    State(Reported),
    /// A line for the terminal only.
    Say(String),
}

/// A typed value's wire name (`end_turn`, `allow_once`, …).
fn wire<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value)
        .unwrap_or_default()
        .trim_matches('"')
        .to_owned()
}

fn acp_error(why: impl Into<String>) -> agent_client_protocol::Error {
    agent_client_protocol::Error::new(ErrorCode::INTERNAL_ERROR, why)
}

/// The session task's state.
struct Session {
    cx: ConnectionTo<Agent>,
    /// Hands responses to our requests back to the task, in order with the agent's messages.
    back: UnboundedSender<FromAgent>,
    out: UnboundedSender<Out>,
    id: SessionId,
    policy: Policy,
    /// Our `session/prompt` requests in flight, by our number.
    prompts: BTreeSet<u64>,
    next: u64,
    /// Permission requests waiting for `approve`: their responder and options.
    asks: VecDeque<(Asked, Vec<ApprovalOption>)>,
    /// Tool call titles and kinds by id, for updates that carry none.
    titles: BTreeMap<String, (String, Option<String>)>,
    /// The current turn's reply so far.
    reply: String,
    auth: &'static [&'static str],
    heard: Instant,
    /// Replaying a resumed session's history.
    replay: bool,
    /// Loading the session again on a wake: its replay is already in the transcript.
    waking: bool,
    /// The last state reported but `hibernated`, restored by a wake that starts no turn.
    last: (String, Option<String>),
    /// The agent's `model` config option, if it has one.
    model: Option<String>,
    shown: AcpSession,
    mcp: Vec<McpServer>,
}

impl Session {
    /// Hands `out` to the writer task; an error once it is gone.
    fn send(&self, out: Out) -> AcpResult<()> {
        let gone = |_| acp_error("the writer task is gone");
        self.out.send(out).map_err(gone)
    }

    fn emit(&self, kind: EventKind) {
        let _ = self.send(Out::Emit(kind, self.replay, !self.waking));
    }

    fn state(&mut self, state: &str, note: Option<String>) {
        if state != "hibernated" {
            self.last = (state.to_owned(), note.clone());
        }
        let (state, state_source) = (state.to_owned(), None);
        let _ = self.send(Out::State(Reported {
            state,
            note,
            state_source,
        }));
    }

    fn say(&self, line: String) -> AcpResult<()> {
        self.send(Out::Say(line))
    }

    /// `auth: <line>` for the first line of `text` with one of the agent's auth-failure patterns.
    fn auth(&self, text: &str) -> Option<String> {
        let bad = |l: &&str| self.auth.iter().any(|p| l.contains(p));
        text.lines().find(bad).map(|l| format!("auth: {l}"))
    }

    /// Ends the turn as `error` with the note `why`, cancelling it at the agent. An auth failure in
    /// `why` makes the note `auth: <line>`, after a `reply` with the stop reason `auth_error`.
    fn fail(&mut self, why: String) -> AcpResult<()> {
        let cancelled = self.input(Input::Cancel);
        self.prompts.clear();
        let auth = self.auth(&why);
        if auth.is_some() {
            let (text, stop_reason) = (std::mem::take(&mut self.reply), "auth_error".to_owned());
            self.emit(EventKind::Reply { text, stop_reason });
        }
        self.state("error", Some(auth.unwrap_or(why)));
        cancelled
    }

    /// Answers a permission request with option `id`, or `cancelled`.
    fn answer(asked: Asked, id: Option<String>) -> AcpResult<()> {
        let outcome = match id {
            Some(id) => RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(id)),
            None => RequestPermissionOutcome::Cancelled,
        };
        asked.respond(RequestPermissionResponse::new(outcome))
    }

    /// One framed or typed input.
    fn input(&mut self, input: Input) -> AcpResult<()> {
        self.heard = Instant::now();
        let back = self.back.clone();
        match input {
            Input::Prompt(text) => {
                self.reply.clear();
                self.emit(EventKind::Turn {
                    prompt: text.clone(),
                });
                self.state("working", None);
                let blocks = vec![ContentBlock::Text(TextContent::new(text))];
                let n = self.next;
                self.next += 1;
                self.cx
                    .send_request(PromptRequest::new(self.id.clone(), blocks))
                    .on_receiving_result(async move |r| {
                        let _ = back.send(FromAgent::Prompt(n, r));
                        Ok(())
                    })?;
                self.prompts.insert(n);
            }
            Input::Approve(choice) => {
                let Some((asked, options)) = self.asks.pop_front() else {
                    return self.say("shed: no permission request is waiting".to_owned());
                };
                let Some(option) = options
                    .iter()
                    .find(|o| o.id == choice)
                    .or_else(|| options.iter().find(|o| o.kind == choice))
                    .map(|o| o.id.clone())
                else {
                    let ids: Vec<&str> = options.iter().map(|o| o.id.as_str()).collect();
                    let ids = ids.join(", ");
                    self.asks.push_front((asked, options));
                    return self.say(format!("shed: no option {choice:?}; one of {ids}"));
                };
                Self::answer(asked, Some(option.clone()))?;
                self.emit(EventKind::Approved { option });
                let state = if self.asks.is_empty() {
                    "working"
                } else {
                    "blocked"
                };
                self.state(state, None);
            }
            Input::Cancel => {
                if !self.prompts.is_empty() {
                    let cancel = CancelNotification::new(self.id.clone());
                    self.cx.send_notification(cancel)?;
                }
                while let Some((asked, _)) = self.asks.pop_front() {
                    Self::answer(asked, None)?;
                }
            }
            Input::Model(model) => {
                let Some(option) = self.model.clone() else {
                    return self.say("shed: the agent offers no models".to_owned());
                };
                let model = SessionConfigOptionValue::value_id(model);
                let set = SetSessionConfigOptionRequest::new(self.id.clone(), option, model);
                self.cx
                    .send_request(set)
                    .on_receiving_result(async move |r| {
                        let _ = back.send(FromAgent::Config(r.map(|r| r.config_options)));
                        Ok(())
                    })?;
            }
        }
        Ok(())
    }

    /// One message from the agent.
    fn message(&mut self, m: FromAgent) -> AcpResult<()> {
        self.heard = Instant::now();
        match m {
            FromAgent::Update(Ok(u)) => self.update(*u),
            FromAgent::Update(Err(why)) => {
                self.say(clean(&format!("shed: skipped a session/update: {why}")))
            }
            FromAgent::Permission(request, asked) => self.permission(*request, asked),
            FromAgent::Prompt(n, r) if self.prompts.remove(&n) => match r {
                Err(e) => self.fail(e.message),
                Ok(r) => {
                    let (text, stop_reason) =
                        (std::mem::take(&mut self.reply), wire(&r.stop_reason));
                    self.emit(EventKind::Reply {
                        text,
                        stop_reason: stop_reason.clone(),
                    });
                    self.state("done", Some(stop_reason));
                    Ok(())
                }
            },
            FromAgent::Prompt(..) => Ok(()),
            FromAgent::Config(Ok(options)) => self.models(&options),
            FromAgent::Config(Err(e)) => self.say(format!("shed: model: {e}")),
        }
    }

    fn update(&mut self, u: SessionUpdate) -> AcpResult<()> {
        let text = |c: ContentBlock| match c {
            ContentBlock::Text(t) => Some(t.text),
            _ => None,
        };
        match u {
            SessionUpdate::AgentMessageChunk(c) => {
                if let Some(chunk) = text(c.content) {
                    self.reply.push_str(&chunk);
                    self.emit(EventKind::Text { text: chunk });
                    if !self.prompts.is_empty() && self.auth(&self.reply).is_some() {
                        return self.fail(self.reply.clone());
                    }
                }
            }
            // A resumed session's prompts; each ends the reply before it.
            SessionUpdate::UserMessageChunk(c) if self.replay => {
                if let Some(prompt) = text(c.content) {
                    self.end_replayed_turn();
                    self.emit(EventKind::Turn { prompt });
                }
            }
            SessionUpdate::ToolCall(call) => {
                return self.update(SessionUpdate::ToolCallUpdate(call.into()));
            }
            SessionUpdate::ToolCallUpdate(call) => {
                let id = call.tool_call_id.to_string();
                let (title, kind) = (self.titles.get(&id).cloned()).unwrap_or((id.clone(), None));
                let title = call.fields.title.unwrap_or(title);
                let kind = (call.fields.kind.map(|k| wire(&k)).filter(|k| k != "other")).or(kind);
                self.titles
                    .insert(id.clone(), (title.clone(), kind.clone()));
                let status = wire(&call.fields.status.unwrap_or_default());
                let call = EventKind::ToolCall {
                    id,
                    title,
                    status,
                    kind,
                };
                self.emit(call);
            }
            SessionUpdate::Plan(plan) => {
                let entries = plan.entries.iter().map(serde_json::to_value);
                let entries = entries.collect::<Result<_, _>>().unwrap_or_default();
                self.emit(EventKind::Plan { entries });
            }
            SessionUpdate::CurrentModeUpdate(m) => {
                let mode = m.current_mode_id.to_string();
                self.emit(EventKind::Mode { mode });
            }
            SessionUpdate::ConfigOptionUpdate(c) => return self.models(&c.config_options),
            SessionUpdate::AvailableCommandsUpdate(u) => {
                let commands: Vec<_> = (u.available_commands.into_iter())
                    .map(|c| match c.input {
                        Some(AvailableCommandInput::Unstructured(i)) => {
                            [c.name, c.description, i.hint]
                        }
                        _ => [c.name, c.description, String::new()],
                    })
                    .collect();
                if commands != self.shown.commands {
                    self.shown.commands = commands;
                    return self.models(&[]);
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Emits a replayed turn's reply, if any.
    fn end_replayed_turn(&mut self) {
        if !self.reply.is_empty() {
            let (text, stop_reason) = (std::mem::take(&mut self.reply), "end_turn".to_owned());
            self.emit(EventKind::Reply { text, stop_reason });
        }
    }

    /// Takes the agent's `model` option (a select) from its config options (if any), then emits the
    /// session.
    fn models(&mut self, options: &[SessionConfigOption]) -> AcpResult<()> {
        let model = options.iter().find_map(|o| match (&o.category, &o.kind) {
            (Some(Category::Model), Kind::Select(s)) => Some((o, s)),
            _ => None,
        });
        if let Some((option, select)) = model {
            let choices: Vec<_> = match &select.options {
                Choices::Ungrouped(o) => o.iter().collect(),
                Choices::Grouped(g) => g.iter().flat_map(|g| &g.options).collect(),
                _ => Vec::new(),
            };
            self.model = Some(option.id.to_string());
            self.shown.model = Some(select.current_value.to_string());
            let pair = |c: &&SessionConfigSelectOption| [c.value.to_string(), c.name.clone()];
            self.shown.models = choices.iter().map(pair).collect();
        }
        self.send(Out::Emit(
            EventKind::Session(self.shown.clone()),
            self.replay,
            !self.waking,
        ))
    }

    fn permission(&mut self, p: RequestPermissionRequest, asked: Asked) -> AcpResult<()> {
        let call = p.tool_call;
        let tool = (call.fields.title)
            .or_else(|| (self.titles.get(&call.tool_call_id.to_string())).map(|t| t.0.clone()))
            .unwrap_or_else(|| "tool".to_owned());
        let options: Vec<ApprovalOption> = (p.options.into_iter())
            .map(|o| ApprovalOption {
                id: o.option_id.to_string(),
                name: o.name,
                kind: wire(&o.kind),
            })
            .collect();
        self.emit(EventKind::ApprovalRequest {
            tool: tool.clone(),
            options: options.clone(),
        });
        let kinds = match self.policy {
            Policy::Ask => {
                self.asks.push_back((asked, options));
                self.state("blocked", Some(tool));
                return Ok(());
            }
            Policy::Allow => ["allow_once", "allow_always"],
            Policy::Deny => ["reject_once", "reject_always"],
        };
        let pick = kinds
            .iter()
            .find_map(|k| options.iter().find(|o| o.kind == *k));
        let option = pick.map(|o| o.id.clone());
        Self::answer(asked, option.clone())?;
        if let Some(option) = option {
            self.emit(EventKind::Approved { option });
        }
        Ok(())
    }
}

/// Seconds of idle before an ACP agent hibernates: `$SHED_ACP_HIBERNATE_AFTER` (default 600, `0` never).
fn hibernate_after() -> Option<Duration> {
    let secs = std::env::var("SHED_ACP_HIBERNATE_AFTER").map_or(600, |s| s.parse().unwrap_or(600));
    (secs > 0).then(|| Duration::from_secs(secs))
}

/// The session task: the handshake (loading the session again and then handing it `wake` when it
/// wakes), then the agent's messages, the inputs and the watchdog, one at a time, until the agent
/// closes its stdout (`None`) or the session has been idle long enough to hibernate (the session back).
async fn drive(
    mut s: Session,
    mut from_agent: UnboundedReceiver<FromAgent>,
    inputs: &mut UnboundedReceiver<Input>,
    cwd: &Path,
    resume: Option<String>,
    wake: Option<Input>,
) -> AcpResult<Option<Session>> {
    let cx = s.cx.clone();
    let refused = |e| {
        let closed = cx.is_incoming_closed();
        acp_error(if closed {
            "the agent closed its stdout".to_owned()
        } else {
            format!("agent refused: {e}")
        })
    };
    let init = InitializeRequest::new(ProtocolVersion::V1)
        .client_info(Implementation::new("shed", shed_proto::VERSION));
    let init = cx.send_request(init).block_task().await.map_err(refused)?;
    let can = &init.agent_capabilities.mcp_capabilities;
    for server in &s.mcp {
        let (kind, name) = match server {
            McpServer::Http(h) if !can.http => ("http", &h.name),
            McpServer::Sse(h) if !can.sse => ("sse", &h.name),
            _ => continue,
        };
        let why = format!("MCP server {name}: the agent takes no {kind} servers (mcpCapabilities)");
        return Err(acp_error(why));
    }
    let resumed = resume.is_some();
    let options = match resume {
        Some(_) if !init.agent_capabilities.load_session => {
            let why = "the agent cannot resume a session (no loadSession)";
            return Err(acp_error(why));
        }
        Some(id) => {
            (s.id, s.replay, s.waking) = (SessionId::new(id), true, wake.is_some());
            let load = LoadSessionRequest::new(s.id.clone(), cwd).mcp_servers(s.mcp.clone());
            let loaded = cx.send_request(load).block_task().await.map_err(refused)?;
            // The updates before the response, all queued by now, are the history.
            while let Ok(m) = from_agent.try_recv() {
                s.message(m)?;
            }
            s.end_replayed_turn();
            (s.replay, s.waking) = (false, false);
            loaded.config_options
        }
        None => {
            let new = NewSessionRequest::new(cwd).mcp_servers(s.mcp.clone());
            let new = cx.send_request(new).block_task().await.map_err(refused)?;
            s.id = new.session_id;
            new.config_options
        }
    };
    (s.shown.id, s.shown.mcp_servers) = (s.id.to_string(), redacted(&s.mcp));
    s.models(&options.unwrap_or_default())?;
    s.say(format!("shed: ACP session ready ({}); send a prompt", s.id))?;
    if let Some(input) = wake {
        if !matches!(input, Input::Prompt(_)) {
            let (state, note) = s.last.clone();
            s.state(&state, note);
        }
        s.input(input)?;
    }
    let idle = std::env::var("SHED_TURN_IDLE_TIMEOUT").map_or(120, |s| s.parse().unwrap_or(120));
    // Only an agent that can load its session again hibernates, once the session has a turn to load
    // (claude-agent-acp keeps no session before its first prompt).
    let sleep_after = hibernate_after().filter(|_| init.agent_capabilities.load_session);
    let mut typing = true;
    loop {
        tokio::select! {
            biased;
            Some(m) = from_agent.recv() => s.message(m)?,
            input = inputs.recv(), if typing => match input {
                Some(input) => s.input(input)?,
                None => typing = false,
            },
            () = cx.incoming_closed() => return Ok(None),
            // Ends a turn that heard nothing for `idle` seconds, but while a request waits for `approve`;
            // hibernates when no turn runs and no request waits.
            () = tokio::time::sleep(Duration::from_millis(100)) => {
                let quiet = s.heard.elapsed();
                if !s.asks.is_empty() {
                    // A request waits for `approve`.
                } else if !s.prompts.is_empty() && quiet.as_secs() >= idle {
                    s.fail("no response".to_owned())?;
                } else if s.prompts.is_empty() && typing && (resumed || s.next > 0)
                    && sleep_after.is_some_and(|d| quiet >= d)
                {
                    return Ok(Some(s));
                }
            }
        }
    }
}

/// The writer task's end: the transcript, the terminal and the server.
struct Writer {
    transcript: BufWriter<File>,
    server: Option<tokio::net::UnixStream>,
    socket: PathBuf,
    name: String,
    /// What the terminal shows, redrawn whole on each change.
    preview: Preview,
}

impl Writer {
    /// Writes each output to the transcript, the server and the terminal, flushing (in that order)
    /// whenever the session task has nothing more queued.
    async fn run(mut self, mut outs: UnboundedReceiver<Out>) {
        let mut batch = Vec::new();
        while outs.recv_many(&mut batch, 4096).await > 0 {
            let (mut redraw, mut calls) = (false, Vec::new());
            for out in batch.drain(..) {
                let name = self.name.clone();
                let call = match out {
                    Out::Emit(kind, replay, keep) => {
                        if keep {
                            let line = serde_json::to_string(&kind).unwrap_or_default();
                            let _ = writeln!(self.transcript, "{line}");
                            redraw |= self.preview.push(kind.clone());
                        }
                        let params = EmitParams { name, kind, replay };
                        crate::notification(Method::Emit, &params)
                    }
                    Out::State(report) => {
                        let params = StateParams { name, report };
                        crate::notification(Method::State, &params)
                    }
                    Out::Say(line) => {
                        self.preview.blocks.push(Err(line));
                        redraw = true;
                        Ok(Vec::new())
                    }
                };
                calls.extend(call.unwrap_or_default());
            }
            let _ = self.transcript.flush();
            self.send(&calls).await;
            if redraw {
                let (cols, rows, color) = stdout_style();
                let lines = self.preview.draw(cols, color, rows - 1).join("\r\n");
                let mut stdout = io::stdout().lock();
                let _ = write!(stdout, "\x1b[H\x1b[2J{lines}\r\n").and_then(|()| stdout.flush());
            }
        }
    }

    /// Sends notification lines, reconnecting once when the connection broke (a server restart). The
    /// agent's session goes on when the server is gone.
    async fn send(&mut self, calls: &[u8]) {
        for _ in 0..2 {
            if self.server.is_none() {
                let socket = Client::connect(&self.socket).ok().map(|c| c.split().1);
                let socket = socket.filter(|s| s.set_nonblocking(true).is_ok());
                self.server = socket.and_then(|s| tokio::net::UnixStream::from_std(s).ok());
            }
            let Some(server) = &mut self.server else {
                return;
            };
            if server.write_all(calls).await.is_ok() {
                return;
            }
            self.server = None;
        }
    }
}

/// Reads the terminal's input (raw mode) and hands each framed or typed line, and each lone
/// `Esc` / `Ctrl+C`, to the session task.
async fn read_input(inputs: UnboundedSender<Input>) {
    let mut stdin = tokio::io::stdin();
    let mut line = Vec::new();
    let mut buf = [0u8; 4096];
    while let Ok(n @ 1..) = stdin.read(&mut buf).await {
        for &b in &buf[..n] {
            let input = match b {
                0x1b | 0x03 => {
                    line.clear();
                    Some(Input::Cancel)
                }
                b'\r' | b'\n' => {
                    let text = String::from_utf8_lossy(&line).into_owned();
                    line.clear();
                    match text.strip_prefix('\x1e') {
                        Some(json) => serde_json::from_str(json).ok(),
                        None if text.trim().is_empty() => None,
                        None => Some(Input::Prompt(text)),
                    }
                }
                0x7f => {
                    line.pop();
                    None
                }
                b => {
                    line.push(b);
                    None
                }
            };
            if let Some(input) = input
                && inputs.send(input).is_err()
            {
                return;
            }
        }
    }
}

/// Runs the ACP adapter `cmd` in `cwd` for terminal `$SHED` of the server on `socket` (state file
/// `state`): `initialize`, then `session/load` of `$SHED_ACP_RESUME` (its history replayed) or
/// `session/new`, both with the MCP servers in `$SHED_ACP_MCP`, then prompts, updates and permission
/// requests until the agent exits. Returns the agent's exit code.
///
/// After `$SHED_ACP_HIBERNATE_AFTER` idle seconds (default 600, `0` never) it stops the agent's process
/// tree and reports `hibernated`, keeping the session (MCP servers included) in memory; the next input
/// starts the agent again, loads the session and then delivers it.
///
/// A failure to start is reported as the terminal's state `error`, with the reason as its note.
///
/// # Errors
/// [`Error::Acp`] when `$SHED` is unset, the agent cannot start, or the handshake fails (including
/// an agent that does not speak ACP v1, or a resume the agent cannot load); [`Error::Io`] when the
/// transcript cannot be created.
pub fn bridge(
    cmd: &[String],
    cwd: &Path,
    socket: &Path,
    state: &Path,
    policy: Policy,
    agent_id: &str,
) -> Result<i32, Error> {
    let name = std::env::var("SHED").map_err(|_| Error::Acp("$SHED is unset".to_owned()))?;
    let result = run(cmd, cwd, socket, state, policy, agent_id, &name);
    if let (Err(e), Ok(mut server)) = (&result, Client::connect(socket)) {
        let report = Reported {
            state: "error".to_owned(),
            note: Some(e.to_string()),
            state_source: None,
        };
        let _: Result<serde_json::Value, _> =
            server.call(Method::State, &StateParams { name, report });
    }
    result
}

/// Creates terminal `name`'s transcript, empty.
fn open_transcript(state: &Path, name: &str) -> io::Result<File> {
    let path = transcript(state, name);
    if let Some(dir) = path.parent() {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
    }
    fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .mode(0o600)
        .open(&path)
}

/// Speaks ACP to `child` over its stdio: runs `main` (the session task) with the connection and the
/// sender and receiver of the agent's messages, until it returns.
async fn connect(
    child: &mut tokio::process::Child,
    main: impl AsyncFnOnce(
        ConnectionTo<Agent>,
        UnboundedSender<FromAgent>,
        UnboundedReceiver<FromAgent>,
    ) -> AcpResult<Option<Session>>,
) -> Result<Option<Session>, Error> {
    let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        return Err(Error::Acp("the agent's stdio is not piped".to_owned()));
    };
    let incoming = stream::unfold(BufReader::new(stdout).lines(), async |mut lines| {
        Some((lines.next_line().await.transpose()?, lines))
    });
    let outgoing = sink::unfold(stdin, async |mut stdin, line: String| {
        stdin.write_all(format!("{line}\n").as_bytes()).await?;
        stdin.flush().await?;
        Ok::<_, io::Error>(stdin)
    });
    let (back, from_agent) = unbounded_channel();
    let (updates, asks) = (back.clone(), back.clone());
    agent_client_protocol::Client
        .v1()
        .name("shed")
        .on_receive_notification(
            async move |n: UntypedMessage, _| {
                if SessionNotification::matches_method(n.method()) {
                    let _ = updates.send(FromAgent::Update(update(n.params())));
                }
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async move |r: RequestPermissionRequest, asked, _| {
                let _ = asks.send(FromAgent::Permission(Box::new(r), asked));
                Ok(())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(
            Lines::new(Box::pin(outgoing), Box::pin(incoming)),
            async move |cx| main(cx, back, from_agent).await,
        )
        .await
        .map_err(|e| Error::Acp(e.to_string()))
}

fn run(
    cmd: &[String],
    cwd: &Path,
    socket: &Path,
    state: &Path,
    policy: Policy,
    agent_id: &str,
    name: &str,
) -> Result<i32, Error> {
    let mcp = std::env::var("SHED_ACP_MCP").unwrap_or_default();
    let mcp = serde_json::from_str::<Vec<Value>>(&mcp).unwrap_or_default();
    let mcp = mcp_servers(&mcp).map_err(Error::Acp)?;
    let (out, outs) = unbounded_channel();
    let writer = Writer {
        transcript: BufWriter::new(open_transcript(state, name)?),
        server: None,
        socket: socket.to_owned(),
        name: name.to_owned(),
        preview: Preview::default(),
    };
    let (typed, mut inputs) = unbounded_channel();
    // A woken session keeps its state; a new one starts empty.
    let session = move |cx, back, kept: Option<Session>| match kept {
        Some(s) => Session { cx, back, ..s },
        None => Session {
            cx,
            back,
            out: out.clone(),
            id: SessionId::new(""),
            policy,
            prompts: BTreeSet::new(),
            next: 0,
            asks: VecDeque::new(),
            titles: BTreeMap::new(),
            reply: String::new(),
            auth: crate::agents::auth_failures(agent_id),
            heard: Instant::now(),
            replay: false,
            waking: false,
            last: ("idle".to_owned(), None),
            model: None,
            shown: AcpSession::default(),
            mcp: mcp.clone(),
        },
    };
    let mut resume = std::env::var("SHED_ACP_RESUME").ok();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()?;
    let writer = runtime.spawn(writer.run(outs));
    runtime.spawn(read_input(typed));
    let result = runtime.block_on(async move {
        let (program, args) = cmd
            .split_first()
            .ok_or_else(|| Error::Acp("empty agent command".to_owned()))?;
        let (mut kept, mut wake) = (None, None);
        loop {
            let mut child = tokio::process::Command::new(program)
                .args(args)
                .current_dir(cwd)
                .env_remove("SHED_ACP_MCP")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .map_err(|e| Error::Acp(format!("{program}: {e}")))?;
            let (rx, loaded, first, prev) = (&mut inputs, resume.take(), wake.take(), kept.take());
            let slept = connect(&mut child, async |cx, back, from_agent| {
                let s = session(cx, back, prev);
                drive(s, from_agent, rx, cwd, loaded, first).await
            })
            .await?;
            let Some(mut s) = slept else {
                return Ok(child.wait().await?.code().unwrap_or(1));
            };
            if let Some(pid) = child.id().and_then(|p| i32::try_from(p).ok()) {
                let table = shed_core::proctree::system();
                let reap = || drop(child.try_wait());
                shed_core::proctree::kill_tree(&table, pid, Duration::from_secs(2), reap)?;
            }
            s.state("hibernated", None);
            // Inputs from now on queue up in order; the first one wakes the agent.
            let Some(input) = inputs.recv().await else {
                return Ok(0);
            };
            (resume, wake) = (Some(s.id.to_string()), Some(input));
            kept = Some(s);
        }
    });
    // The session's sender is gone with it: the writer drains what is queued and ends. The stdin read
    // never ends on its own, so the runtime does not wait for it.
    let _ = runtime.block_on(writer);
    runtime.shutdown_background();
    result
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    #[test]
    fn the_preview_draws_an_unknown_status_plain_and_keeps_each_call_on_its_first_line() {
        let call = |id: &str, status: &str, kind: Option<&str>| EventKind::ToolCall {
            id: id.into(),
            title: "line one\nline two".into(),
            status: status.into(),
            kind: kind.map(Into::into),
        };
        let mut preview = Preview::default();
        assert!(preview.push(call("a", "pending", Some("execute"))));
        assert!(preview.push(call("b", "teleported", Some("warp"))));
        assert!(!preview.push(EventKind::Text { text: "x".into() }));
        assert!(preview.push(call("a", "failed", None)));
        assert_eq!(preview.draw(12, false, 9), ["✗ line one", "warp line on"]);
        assert_eq!(preview.draw(40, false, 1), ["warp line one"]);
        let styled = preview.draw(40, true, 9);
        assert_eq!(styled[0], "\x1b[2m✗ line one\x1b[0m");
    }

    #[test]
    fn render_keeps_prompts_one_line_per_tool_call_and_replies() {
        let lines = [
            json!({"event": "turn", "prompt": "fix it"}),
            json!({"event": "text", "text": "Look"}),
            json!({"event": "tool_call", "id": "a", "title": "Edit", "status": "pending"}),
            json!({"event": "tool_call", "id": "b", "title": "Run", "status": "pending"}),
            json!({"event": "tool_call", "id": "a", "title": "Edit", "status": "completed"}),
            json!({"event": "reply", "text": "Done\u{1b}[2J", "stop_reason": "end_turn"}),
        ];
        let jsonl: Vec<String> = lines.iter().map(Value::to_string).collect();
        assert_eq!(
            render(&jsonl.join("\n")),
            "> fix it\n[tool] Edit (completed)\n[tool] Run (pending)\nDone?[2J"
        );
    }

    /// A text chunk read straight from the JSON is the update the SDK makes of it; any other goes
    /// through the SDK, and one it does not know is an error.
    #[test]
    fn a_text_chunk_skips_the_sdk_parse_and_matches_it() {
        let params = |update: Value| json!({"sessionId": "s", "update": update});
        let text = params(json!({"sessionUpdate": "agent_message_chunk",
            "content": {"type": "text", "text": "hi"}}));
        let typed = SessionNotification::parse_message("session/update", &text).unwrap();
        assert_eq!(update(&text), Ok(Box::new(typed.update)));
        let image = params(json!({"sessionUpdate": "agent_message_chunk",
            "content": {"type": "image", "data": "", "mimeType": "image/png"}}));
        let Ok(image) = update(&image).map(|u| *u) else {
            panic!("an image chunk parses");
        };
        assert!(matches!(image, SessionUpdate::AgentMessageChunk(c)
            if matches!(c.content, ContentBlock::Image(_))));
        assert!(update(&params(json!({"sessionUpdate": "brand_new_kind"}))).is_err());
    }

    #[test]
    fn frames_are_one_line_of_json() {
        let framed = frame(&Input::Prompt("a\nb".to_owned()));
        assert_eq!(framed, "\x1e{\"prompt\":\"a\\nb\"}\n");
        assert_eq!(frame(&Input::Cancel), "\x1e\"cancel\"\n");
    }
}
