//! A lane: `shed run --acp` starts [`bridge`] as an ordinary terminal's program. It runs the agent
//! through its driver and turns what the agent says into shed events (`emit`), reported states and a
//! JSONL transcript that `read` renders.
//!
//! The bridge reads its PTY in raw mode: a line framed by [`frame`](shed_proto::frame) (`send`, `approve`, `cancel`,
//! `model`), a plain line typed in `attach` (a prompt), or `Esc` / `Ctrl+C` alone (cancel).
//!
//! Its own tokio runtime runs the driver and the lane on this thread and, on one worker beside it, a
//! task reading the PTY into [`Input`]s and one draining the lane's outputs into the transcript, the
//! terminal and the server (as notifications), a batch at a time. The transcript is the session's own
//! (`sessions`), whatever terminal runs it.

pub mod sessions;
pub mod transcript;
pub(crate) mod writer;

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Map, Value};
use shed_accounts::{Store, choice, env::Injected, session};
use shed_core::proctree::{self, ProcessInfo, ProcessTable as _};
pub use shed_proto::Policy;
use shed_proto::{
    Account, AccountsParams, AcpSession, AcpUsage, ApprovalOption, ErrorCode, EventKind, Input,
    Method, Provider, Reported, RpcError, StateParams, Tokens,
};
use tokio::io::{AsyncBufReadExt as _, AsyncRead, AsyncWriteExt as _, BufReader};
use tokio::process::Child;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::time::Instant;
use transcript::{Preview, clean};
use writer::{Out, Writer};

use crate::Error;
use crate::drivers::{acp, stream};
use shed_client::Conn;

/// The states only an ACP lane reports, beside `working`, `done` and `error`.
pub const LANE_STATES: [&str; 4] = ["running", "waiting-approval", "rate-limited", "hibernated"];

pub(crate) const GO_ON: &str = "Your account hit its usage limit, so shed moved this session to another account. \
                     Go on where you left off.";

const MOVED: &str = "The owner moved this session to another account, so shed stopped your turn. \
                     Go on where you left off.";

/// Where the bridge of terminal `name` lends its PTY's master to a server on `socket` that starts again.
#[must_use]
pub fn pty_socket(socket: &Path, name: &str) -> PathBuf {
    socket.with_file_name(format!("pty-{name}.sock"))
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Heard {
    Session {
        model: Option<String>,
        models: Option<Vec<[String; 2]>>,
        commands: Option<Vec<[String; 3]>>,
    },
    Prompted,
    Text(String),
    Tool(Tool),
    Plan(Vec<Value>),
    Mode(String),
    Usage(Option<AcpUsage>),
    Limit(Option<String>),
    Ask(Ask),
    TurnEnd {
        stop: String,
        error: Option<String>,
        meta: Option<Map<String, Value>>,
    },
    Skipped(String),
    Note(String),
}

#[derive(Deserialize)]
pub(crate) struct Tool {
    pub id: String,
    pub title: Option<String>,
    pub kind: Option<String>,
    pub status: Option<String>,
    #[serde(default)]
    pub locations: Vec<PathBuf>,
    pub raw_input: Option<Value>,
}

#[derive(Deserialize)]
pub(crate) struct Ask {
    pub id: String,
    pub title: Option<String>,
    pub tool_call: String,
    pub options: Vec<ApprovalOption>,
    pub call: Option<Value>,
}

pub(crate) enum Stop {
    No,
    Yes,
}

pub(crate) struct Switch {
    to: String,
    deliver: Option<Input>,
    why: &'static str,
}

#[derive(Default)]
pub(crate) struct Start {
    pub session: String,
    pub resumed: bool,
    pub deliver: Option<Input>,
}

pub(crate) struct Opened {
    pub id: String,
    pub resumable: bool,
    pub mcp: Vec<Value>,
    pub session: Heard,
    pub pid: Option<i32>,
    pub stirred: Arc<AtomicBool>,
}

pub(crate) trait Driver {
    const NAME: &'static str;
    async fn open(&mut self, cwd: &Path, start: &Start) -> Result<Opened, String>;
    async fn heard(&mut self) -> Option<Heard>;
    fn gone(&self) -> impl Future<Output = ()> + 'static;
    fn prompt(&mut self, text: &str) -> Result<(), String>;
    fn cancel(&mut self) -> Result<Stop, String>;
    fn model(&mut self, model: &str) -> Result<Stop, String>;
    fn approve(&mut self, request: &str, option: Option<&str>) -> Result<(), String>;
    fn forget(&mut self);
}

pub(crate) struct Lane {
    out: UnboundedSender<Out>,
    policy: Policy,
    turns: usize,
    prompted: bool,
    asks: VecDeque<(String, Vec<ApprovalOption>)>,
    titles: BTreeMap<String, (String, Option<String>)>,
    reply: String,
    rules: Provider,
    heard: Instant,
    stirred: Arc<AtomicBool>,
    idle: Duration,
    last: (String, Option<String>),
    shown: AcpSession,
    tokens: Option<(Tokens, Tokens)>,
    pid: Option<i32>,
    own: BTreeSet<i32>,
    seen: BTreeSet<i32>,
    open: Vec<String>,
    ended: Option<String>,
    child: Option<String>,
    restart: bool,
    sleep_after: Option<Duration>,
    hibernate_set: Option<String>,
    auto: Option<(String, PathBuf)>,
    switch: Option<Switch>,
}

impl Lane {
    pub(crate) fn new(
        out: UnboundedSender<Out>,
        policy: Policy,
        rules: Provider,
        idle: Duration,
    ) -> Self {
        Self {
            out,
            policy,
            turns: 0,
            prompted: false,
            asks: VecDeque::new(),
            titles: BTreeMap::new(),
            reply: String::new(),
            rules,
            heard: shed_core::clock::tokio_now(),
            stirred: Arc::default(),
            idle,
            last: ("idle".to_owned(), None),
            shown: AcpSession::default(),
            tokens: None,
            pid: None,
            own: BTreeSet::new(),
            seen: BTreeSet::new(),
            open: Vec::new(),
            ended: None,
            child: None,
            restart: false,
            sleep_after: None,
            hibernate_set: None,
            auto: None,
            switch: None,
        }
    }

    fn send(&self, out: Out) -> Result<(), String> {
        let gone = |_| "the writer task is gone".to_owned();
        self.out.send(out).map_err(gone)
    }

    fn emit(&self, kind: EventKind) {
        self.send(Out::Emit(kind)).ok();
    }

    fn state(&mut self, state: &str, note: Option<String>) {
        self.heard = shed_core::clock::tokio_now();
        if state != "hibernated" {
            self.last = (state.to_owned(), note.clone());
        }
        self.send(Out::State(Reported {
            state: state.to_owned(),
            note,
            ..Reported::default()
        }))
        .ok();
    }

    fn say(&self, line: String) -> Result<(), String> {
        self.send(Out::Say(line))
    }

    fn auth(&self, text: &str) -> Option<String> {
        self.rules.auth_failure(text).map(|l| format!("auth: {l}"))
    }

    fn fail(&mut self, why: String, driver: &mut impl Driver) -> Result<(), String> {
        self.turns = 0;
        let cancelled = self.input(Input::Cancel, driver);
        driver.forget();
        if let Some((_, resets)) = self.rules.limit_hit(&why) {
            self.limited(resets);
            return cancelled;
        }
        let auth = self.auth(&why);
        if auth.is_some() {
            let (text, stop_reason) = (std::mem::take(&mut self.reply), "auth_error".to_owned());
            self.emit(EventKind::Reply { text, stop_reason });
        }
        self.state("error", Some(auth.unwrap_or(why)));
        cancelled
    }

    fn pick(&self, leave: bool) -> Result<String, RpcError> {
        let none = || RpcError::new(ErrorCode::NoAccount, "no auto account with session files");
        let auto = self
            .auto
            .as_ref()
            .filter(|_| !self.rules.session_files.is_empty());
        let (on, socket) = auto.ok_or_else(none)?;
        let io = |e: shed_client::Error| RpcError::new(ErrorCode::Io, e.to_string());
        let params = AccountsParams {
            provider: Some(self.rules.agent.clone()),
        };
        let all: Vec<Account> = (Conn::connect(socket).map_err(io)?)
            .call(Method::AccountLs, &params)
            .map_err(io)?;
        let leave = leave.then_some(on.as_str());
        choice::pick(&all, leave, shed_core::clock::unix_now())
    }

    fn limited(&mut self, resets: Option<String>) {
        self.state("rate-limited", resets);
        match self.pick(true) {
            Ok(to) => {
                let deliver = Some(Input::Prompt(GO_ON.to_owned()));
                self.switch = Some(Switch {
                    to,
                    deliver,
                    why: "limit",
                });
            }
            Err(e) if e.code == ErrorCode::AccountLimited.code() => {
                self.state("rate-limited", Some(e.message));
            }
            Err(_) => {}
        }
    }

    fn moving(&mut self, to: String) {
        let deliver = if self.last.0 == "rate-limited" {
            Some(GO_ON)
        } else {
            (self.turns > 0).then_some(MOVED)
        };
        let deliver = deliver.map(|text| Input::Prompt(text.to_owned()));
        self.switch = Some(Switch {
            to,
            deliver,
            why: "owner",
        });
    }

    fn switched(
        &mut self,
        sw: &Switch,
        (socket, state): (&Path, &Path),
    ) -> Result<Injected, String> {
        let (store, rules, to) = (Store::beside(state), &self.rules, sw.to.as_str());
        let injected = Injected::for_run(&store, rules, to, &BTreeMap::new());
        let injected = injected.map_err(|e| e.message)?;
        let was = (self.auto.as_ref()).map_or(shed_accounts::store::DEFAULT, |a| a.0.as_str());
        let was = was.to_owned();
        let home = PathBuf::from(crate::config::var("HOME").unwrap_or_default());
        let from = session::folder(&store, rules, &was, &home);
        if let (Some(from), Ok(Some(into))) = (from, store.folder(rules, to)) {
            let copied = session::copy(rules, &self.shown.id, &from, &into);
            copied.map_err(|e| format!("moving the session to {to}: {e}"))?;
        }
        self.auto = Some((to.to_owned(), socket.to_owned()));
        self.last = ("idle".to_owned(), None);
        let (account, why) = (to.to_owned(), sw.why.to_owned());
        self.emit(EventKind::Account {
            account,
            was: Some(was),
            why,
        });
        Ok(injected)
    }

    fn settle(&mut self) {
        if !self.asks.is_empty() || ["error", "rate-limited"].contains(&self.last.0.as_str()) {
            return;
        }
        let tool = (self.open.last()).and_then(|id| self.titles.get(id));
        let (state, note) = match (tool, &self.ended, &self.child) {
            (Some((title, _)), ..) => ("running", Some(title.clone())),
            _ if self.turns > 0 => ("working", None),
            (None, Some(_), Some(child)) => ("running", Some(child.clone())),
            (None, Some(why), None) => ("done", Some(why.clone())),
            (None, None, _) => return,
        };
        if self.last != (state.to_owned(), note.clone()) {
            self.state(state, note);
        }
    }

    fn tree(&mut self) -> Option<(i32, Vec<ProcessInfo>)> {
        let pid = self.pid?;
        let procs = proctree::system().list().ok()?;
        let tree = proctree::tree(&procs, pid, &BTreeSet::new());
        self.seen = tree.iter().map(|p| p.pid).collect();
        Some((pid, tree))
    }

    fn spawned(&mut self) -> Option<String> {
        let (pid, tree) = self.tree()?;
        let new: Vec<_> = (tree.iter())
            .filter(|p| p.pid != pid && !self.own.contains(&p.pid))
            .collect();
        let top = new.iter().find(|p| !new.iter().any(|q| q.pid == p.ppid))?;
        let (cmd, age) = proctree::command(top.pid)?;
        let cmd: String = cmd.chars().take(60).collect();
        Some(format!("running: {cmd} ({}m)", age.as_secs() / 60))
    }

    fn input(&mut self, input: Input, driver: &mut impl Driver) -> Result<(), String> {
        self.heard = shed_core::clock::tokio_now();
        if matches!(input, Input::Prompt(_))
            && self.last.0 == "rate-limited"
            && let Ok(to) = self.pick(false)
            && self.auto.as_ref().is_some_and(|(on, _)| *on != to)
        {
            self.switch = Some(Switch {
                to,
                deliver: Some(input),
                why: "limit",
            });
            return Ok(());
        }
        match input {
            Input::Prompt(text) => {
                self.reply.clear();
                (self.ended, self.child) = (None, None);
                self.open.clear();
                self.own.clone_from(&self.seen);
                self.emit(EventKind::Turn {
                    prompt: text.clone(),
                    agent: self.own.iter().copied().collect(),
                });
                self.state("working", None);
                self.prompted = true;
                driver.prompt(&text)?;
                self.turns += 1;
            }
            Input::Approve {
                option: choice,
                request,
            } => {
                let at = match &request {
                    Some(r) => self.asks.iter().position(|a| a.0 == *r),
                    None => (!self.asks.is_empty()).then_some(0),
                };
                let Some((id, options)) = at.and_then(|at| self.asks.remove(at)) else {
                    let which = request.map_or_else(String::new, |r| format!(" {r:?}"));
                    return self.say(format!("shed: no permission request{which} is waiting"));
                };
                let Some(option) = options
                    .iter()
                    .find(|o| o.id == choice)
                    .or_else(|| options.iter().find(|o| o.kind == choice))
                    .map(|o| o.id.clone())
                else {
                    let ids: Vec<&str> = options.iter().map(|o| o.id.as_str()).collect();
                    let ids = ids.join(", ");
                    self.asks.insert(at.unwrap_or(0), (id, options));
                    return self.say(format!("shed: no option {choice:?}; one of {ids}"));
                };
                driver.approve(&id, Some(&option))?;
                self.emit(EventKind::Approved { option });
                self.settle();
            }
            Input::Cancel => {
                if let Stop::Yes = driver.cancel()? {
                    self.restart = true;
                }
                if self.restart && self.turns > 0 {
                    self.turns = 0;
                    for id in std::mem::take(&mut self.open) {
                        let (title, kind) = self.titles.get(&id).cloned().unwrap_or_default();
                        let (status, locations, raw_input) =
                            ("failed".to_owned(), Vec::new(), None);
                        let call = EventKind::ToolCall {
                            id,
                            title,
                            status,
                            kind,
                            locations,
                            raw_input,
                        };
                        self.emit(call);
                    }
                    let text = std::mem::take(&mut self.reply);
                    let stop_reason = "cancelled".to_owned();
                    self.emit(EventKind::Reply { text, stop_reason });
                    (self.ended, self.child) = (Some("cancelled".to_owned()), None);
                    self.settle();
                }
                self.asks.clear();
            }
            Input::Name(name) => return self.send(Out::Name(name)),
            Input::Account(to) => self.moving(to),
            Input::Model(model) => {
                if let Stop::Yes = driver.model(&model)? {
                    self.restart = true;
                    return self.set_model(model);
                }
            }
        }
        Ok(())
    }

    fn hear(&mut self, heard: Heard, driver: &mut impl Driver) -> Result<(), String> {
        match heard {
            Heard::Text(chunk) => {
                self.reply.push_str(&chunk);
                self.emit(EventKind::Text { text: chunk });
                if self.turns > 0 && self.auth(&self.reply).is_some() {
                    return self.fail(self.reply.clone(), driver);
                }
            }
            Heard::Tool(tool) => self.tool(tool),
            Heard::Plan(entries) => self.emit(EventKind::Plan { entries }),
            Heard::Mode(mode) => self.emit(EventKind::Mode { mode }),
            Heard::Session {
                model,
                models,
                commands,
            } => return self.session(model, models, commands),
            Heard::Usage(usage) => {
                self.shown.usage = usage;
                self.show_usage();
            }
            Heard::Limit(resets) => self.limited(resets),
            Heard::Ask(ask) => return self.ask(ask, driver),
            Heard::TurnEnd { stop, error, meta } => {
                self.turns = self.turns.saturating_sub(1);
                match error {
                    Some(why) => return self.fail(why, driver),
                    None => self.turn_end(stop, meta.as_ref()),
                }
            }
            Heard::Skipped(why) => {
                return self.say(clean(&format!("shed: skipped a session/update: {why}")));
            }
            Heard::Note(line) => return self.say(line),
            Heard::Prompted => {}
        }
        Ok(())
    }

    fn turn_end(&mut self, stop_reason: String, meta: Option<&Map<String, Value>>) {
        let text = std::mem::take(&mut self.reply);
        let limited = self.rules.limit_hit(&text).map(|(_, at)| at);
        let stop = stop_reason.clone();
        if let Some(turn) = meta.and_then(|m| self.rules.tokens(m)) {
            let total = self.tokens.map_or(turn, |(_, t)| t.plus(turn));
            self.tokens = Some((turn, total));
            self.show_usage();
        }
        self.emit(EventKind::Reply { text, stop_reason });
        if let Some(resets) = limited {
            self.limited(resets);
        } else {
            (self.ended, self.child) = (Some(stop), self.spawned());
            self.settle();
        }
    }

    fn tool(&mut self, tool: Tool) {
        let id = tool.id;
        let (title, kind) = (self.titles.get(&id).cloned()).unwrap_or((id.clone(), None));
        let title = tool.title.unwrap_or(title);
        let kind = tool.kind.or(kind);
        self.titles
            .insert(id.clone(), (title.clone(), kind.clone()));
        let given = tool.status.is_some();
        let status = tool.status.unwrap_or_else(|| "pending".to_owned());
        if given {
            self.open.retain(|o| *o != id);
            if ["pending", "in_progress"].contains(&status.as_str()) {
                self.open.push(id.clone());
            }
        }
        let raw_input = tool.raw_input.filter(|v| v.to_string().len() <= 4096);
        self.emit(EventKind::ToolCall {
            id,
            title,
            status,
            kind,
            locations: tool.locations,
            raw_input,
        });
        if given {
            self.settle();
        }
    }

    fn session(
        &mut self,
        model: Option<String>,
        models: Option<Vec<[String; 2]>>,
        commands: Option<Vec<[String; 3]>>,
    ) -> Result<(), String> {
        if let Some(commands) = commands {
            if commands == self.shown.commands {
                return Ok(());
            }
            self.shown.commands = commands;
        }
        if let Some(models) = models {
            (self.shown.model, self.shown.models) = (model, models);
        }
        let shown = EventKind::Session(Box::new(self.shown.clone()));
        self.send(Out::Emit(shown))
    }

    fn set_model(&mut self, model: String) -> Result<(), String> {
        let models = Some(self.shown.models.clone());
        self.session(Some(model), models, None)
    }

    fn show_usage(&mut self) {
        if let Some(usage) = &mut self.shown.usage {
            (usage.turn, usage.total) = (self.tokens.map(|t| t.0), self.tokens.map(|t| t.1));
            let usage = usage.clone();
            self.emit(EventKind::Usage(usage));
        }
    }

    fn ask(&mut self, ask: Ask, driver: &mut impl Driver) -> Result<(), String> {
        let tool = (ask.title)
            .or_else(|| (self.titles.get(&ask.tool_call)).map(|t| t.0.clone()))
            .unwrap_or_else(|| "tool".to_owned());
        self.emit(EventKind::ApprovalRequest {
            id: ask.id.clone(),
            tool: tool.clone(),
            options: ask.options.clone(),
            call: ask.call,
        });
        let kinds = match self.policy {
            Policy::Ask => {
                self.asks.push_back((ask.id, ask.options));
                self.state("waiting-approval", Some(tool));
                return Ok(());
            }
            Policy::Allow => ["allow_once", "allow_always"],
            Policy::Deny => ["reject_once", "reject_always"],
        };
        let pick = kinds
            .iter()
            .find_map(|k| ask.options.iter().find(|o| o.kind == *k));
        let option = pick.map(|o| o.id.clone());
        driver.approve(&ask.id, option.as_deref())?;
        if let Some(option) = option {
            self.emit(EventKind::Approved { option });
        }
        Ok(())
    }

    async fn asleep(&mut self, inputs: &mut UnboundedReceiver<Input>) -> Option<Input> {
        if std::mem::take(&mut self.restart) {
            let after = self.sleep_after.unwrap_or(Duration::MAX);
            if let Ok(input) = tokio::time::timeout(after, inputs.recv()).await {
                return input;
            }
        }
        self.state("hibernated", None);
        inputs.recv().await
    }

    pub(crate) async fn drive<D: Driver>(
        &mut self,
        driver: &mut D,
        inputs: &mut UnboundedReceiver<Input>,
        cwd: &Path,
        start: Start,
    ) -> Result<bool, String> {
        let opened = driver.open(cwd, &start).await?;
        (self.pid, self.stirred) = (opened.pid, opened.stirred);
        self.send(Out::Session(opened.id.clone(), start.resumed))?;
        let api = |v: &str| crate::config::var(v).is_some_and(|v| !v.is_empty());
        self.shown.cache_lifetime = self.rules.cache_lifetime(api);
        (self.shown.id, self.shown.mcp_servers) = (opened.id, opened.mcp);
        D::NAME.clone_into(&mut self.shown.driver);
        self.hear(opened.session, driver)?;
        (self.ended, self.child) = (None, None);
        self.tree();
        let ready = format!("shed: ACP session ready ({}); send a prompt", self.shown.id);
        self.say(ready)?;
        if let Some(input) = start.deliver {
            if !matches!(input, Input::Prompt(_)) {
                let (state, note) = self.last.clone();
                self.state(&state, note);
            }
            self.input(input, driver)?;
        }
        let set = self.hibernate_set.clone();
        self.sleep_after =
            hibernate_after(set, self.shown.cache_lifetime).filter(|_| opened.resumable);
        let (mut typing, mut polled) = (true, shed_core::clock::tokio_now());
        let gone = driver.gone();
        tokio::pin!(gone);
        loop {
            let owner = self.switch.as_ref().is_some_and(|s| s.why == "owner");
            if owner && self.turns > 0 && self.open.is_empty() {
                self.turns = 0;
                let text = std::mem::take(&mut self.reply);
                let stop_reason = "cancelled".to_owned();
                self.emit(EventKind::Reply { text, stop_reason });
            }
            if (self.restart || self.switch.is_some()) && self.turns == 0 {
                return Ok(true);
            }
            tokio::select! {
                biased;
                heard = driver.heard() => match heard {
                    Some(heard) => self.hear(heard, driver)?,
                    None => return Ok(false),
                },
                input = inputs.recv(), if typing => match input {
                    Some(input) => self.input(input, driver)?,
                    None => typing = false,
                },
                () = &mut gone => return Ok(false),
                () = tokio::time::sleep(Duration::from_millis(100)) => {
                    if self.stirred.swap(false, Ordering::Relaxed) {
                        self.heard = shed_core::clock::tokio_now();
                    }
                    if self.ended.is_some() && polled.elapsed() >= Duration::from_secs(5) {
                        (self.child, polled) = (self.spawned(), shed_core::clock::tokio_now());
                        self.settle();
                    }
                    let quiet = self.heard.elapsed();
                    if !self.asks.is_empty() {
                    } else if self.turns > 0 && self.open.is_empty() && quiet >= self.idle {
                        self.fail("no response".to_owned(), driver)?;
                    } else if self.turns == 0 && typing && (start.resumed || self.prompted)
                        && ["done", "idle"].contains(&self.last.0.as_str())
                        && self.sleep_after.is_some_and(|d| quiet >= d)
                    {
                        return Ok(true);
                    }
                }
            }
        }
    }
}

pub(crate) fn turn_idle_timeout() -> Duration {
    let secs =
        crate::config::var("SHED_TURN_IDLE_TIMEOUT").map_or(1800, |s| s.parse().unwrap_or(1800));
    Duration::from_secs(secs)
}

fn hibernate_after(set: Option<String>, cache_lifetime: Option<u64>) -> Option<Duration> {
    let fits = cache_lifetime.map_or(600, |c| c.min(600));
    let secs = set.map_or(fits, |s| s.parse().unwrap_or(600));
    (secs > 0).then(|| Duration::from_secs(secs))
}

/// Runs the ACP adapter `cmd` in `cwd` for terminal `$SHED` of the server on `socket` (state file
/// `state`) as agent `agent`: `initialize`, then `session/load` of `$SHED_ACP_RESUME` (shed's transcript of
/// it replayed first, the agent's own replay dropped) or `session/new`, both with the MCP servers in
/// `$SHED_ACP_MCP` and the `_meta` in `$SHED_ACP_META`, then prompts, updates and permission requests until the agent exits, reading its limit and expired-login
/// lines by `rules`. Returns the agent's exit code.
///
/// After `$SHED_ACP_HIBERNATE_AFTER` seconds `done` (default 600, or the record's shorter prompt cache
/// lifetime on this login; `0` never) it stops the agent's process tree and reports `hibernated`, keeping
/// the session (MCP servers included) in memory; the next input starts the agent again, loads the session
/// and then delivers it.
///
/// With `stream` it runs the stream driver instead, by the record's `stream` table: `cmd` is then the
/// agent's command, and a cancel or a model switch stops the agent until the next input. `why`, the reason
/// a lane stays on ACP, is its first line.
///
/// A failure to start is reported as the terminal's state `error`, with the reason as its note.
///
/// # Errors
/// [`Error::Acp`] when `$SHED` is unset, the agent cannot start, or the handshake fails (including
/// an agent that does not speak ACP v1, or a resume the agent cannot load); [`Error::Io`] when its
/// runtime or terminal cannot be set up.
pub fn bridge(
    cmd: &[String],
    cwd: &Path,
    (socket, state): (&Path, &Path),
    (agent, policy, rules): (&str, Policy, Provider),
    (stream, why): (bool, Option<&str>),
) -> Result<i32, Error> {
    let name = crate::config::var("SHED").ok_or_else(|| Error::Acp("$SHED is unset".to_owned()))?;
    let launch = if stream {
        Launch::Stream(Box::new(stream::Launch::from_env(
            cmd,
            rules.clone(),
            state,
        )?))
    } else {
        Launch::Acp(acp::Launch::from_env()?)
    };
    let at = (socket, state);
    let result = run(cmd, cwd, at, (agent, policy, rules), (&name, launch, why));
    if let (Err(e), Ok(mut server)) = (&result, Conn::connect(socket)) {
        let report = Reported {
            state: "error".to_owned(),
            note: Some(e.to_string()),
            ..Reported::default()
        };
        let _: Option<serde_json::Value> = server
            .call(Method::State, &StateParams { name, report })
            .ok();
    }
    result
}

pub(crate) fn stop(mut child: Child) {
    if let Some(pid) = child.id().and_then(|p| i32::try_from(p).ok()) {
        let table = proctree::system();
        let reap = || drop(child.try_wait());
        proctree::kill_tree(&table, pid, Duration::from_secs(2), reap);
    }
}

pub(crate) fn echo(stderr: impl AsyncRead + Send + Unpin + 'static, stirred: Arc<AtomicBool>) {
    tokio::spawn(async move {
        let (mut from, mut to, mut line) =
            (BufReader::new(stderr), tokio::io::stderr(), Vec::new());
        while from.read_until(b'\n', &mut line).await.is_ok_and(|n| n > 0) {
            stirred.store(true, Ordering::Relaxed);
            to.write_all(&line).await.ok();
            line.clear();
        }
    });
}

enum Launch {
    Acp(acp::Launch),
    Stream(Box<stream::Launch>),
}

impl Launch {
    fn set_account(&mut self, account: Injected) {
        match self {
            Self::Acp(launch) => launch.account = account,
            Self::Stream(launch) => launch.account = account,
        }
    }
}

fn run(
    cmd: &[String],
    cwd: &Path,
    (socket, state): (&Path, &Path),
    (agent, policy, rules): (&str, Policy, Provider),
    (name, mut launch, why): (&str, Launch, Option<&str>),
) -> Result<i32, Error> {
    let reported = Arc::default();
    let lent = pty_socket(socket, name);
    let taken = crate::handover::take_over(io::stderr(), 1).map(|(fds, _)| fds.into_iter().next());
    if let Ok(Some(master)) = taken {
        rustix::stdio::dup2_stderr(io::stdin()).map_err(io::Error::from)?;
        writer::lend(master, &lent, Arc::clone(&reported))?;
    }
    let tty = File::options().write(true).open("/dev/tty").ok();
    let tty = tty.filter(|t| rustix::io::ioctl_fionbio(t, true).is_ok());
    let (out, outs) = unbounded_channel();
    let writer = Writer {
        transcript: None,
        state: state.to_owned(),
        record: shed_proto::SessionRecord {
            agent: agent.to_owned(),
            cwd: cwd.to_owned(),
            ..shed_proto::SessionRecord::default()
        },
        tty,
        reported,
        server: None,
        socket: socket.to_owned(),
        name: name.to_owned(),
        preview: Preview::default(),
    };
    let (typed, mut inputs) = unbounded_channel();
    let resume = crate::config::var("SHED_ACP_RESUME");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()?;
    let writer = runtime.spawn(writer.run(outs));
    runtime.spawn(writer::read_input(typed));
    let mut lane = Lane::new(out, policy, rules, turn_idle_timeout());
    lane.hibernate_set = crate::config::var("SHED_ACP_HIBERNATE_AFTER");
    let auto = crate::config::var(crate::drivers::launch::AUTO_ACCOUNT);
    lane.auto = auto.map(|on| (on, socket.to_owned()));
    if let Some(why) = why {
        lane.say(format!("shed: on ACP: {why}")).ok();
    }
    let result = runtime.block_on(async move {
        let resumed = resume.is_some();
        let session = resume.unwrap_or_default();
        let mut start = Start {
            session,
            resumed,
            deliver: None,
        };
        loop {
            let (mut child, slept) = match &mut launch {
                Launch::Acp(launch) => {
                    let drive =
                        async |d: &mut acp::Acp| lane.drive(d, &mut inputs, cwd, start).await;
                    acp::run(cmd, cwd, launch, drive).await?
                }
                Launch::Stream(launch) => {
                    let drive =
                        async |d: &mut stream::Agent| lane.drive(d, &mut inputs, cwd, start).await;
                    stream::run(launch, drive).await?
                }
            };
            if !slept {
                return Ok(child.wait().await?.code().unwrap_or(1));
            }
            stop(child);
            let mut deliver = None;
            if lane.switch.is_none() {
                match lane.asleep(&mut inputs).await {
                    Some(Input::Account(to)) => lane.moving(to),
                    Some(input) => deliver = Some(input),
                    None => return Ok(0),
                }
            }
            if let Some(sw) = lane.switch.take() {
                deliver = match lane.switched(&sw, (socket, state)) {
                    Ok(account) => {
                        launch.set_account(account);
                        sw.deliver
                    }
                    Err(why) => {
                        let was = lane.last.0.clone();
                        lane.state(&was, Some(why));
                        None
                    }
                };
            }
            let session = lane.shown.id.clone();
            start = Start {
                session,
                resumed: true,
                deliver,
            };
        }
    });
    runtime.block_on(writer).ok();
    runtime.shutdown_background();
    std::fs::remove_file(lent).ok();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hibernate_delay_fits_a_shorter_cache_and_the_variable_overrides_it() {
        let secs = |set: Option<&str>, cache| {
            hibernate_after(set.map(str::to_owned), cache).map(|d| d.as_secs())
        };
        assert_eq!(secs(None, None), Some(600));
        assert_eq!(secs(None, Some(3600)), Some(600));
        assert_eq!(secs(None, Some(300)), Some(300));
        assert_eq!(secs(Some("2"), Some(300)), Some(2));
        assert_eq!(secs(Some("900"), Some(300)), Some(900));
        assert_eq!(secs(Some("junk"), Some(300)), Some(600));
        assert_eq!(secs(Some("0"), Some(300)), None);
    }
}
