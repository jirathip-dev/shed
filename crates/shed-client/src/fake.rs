//! A fake shed server for tests: a real socket in a temp folder, the answers or handlers a test gives it, the
//! state its built-in methods keep (terminals, files, cued agent replies, a live event log) and the calls it
//! saw. It shares no code with the client, so a client bug cannot hide in it.

use std::collections::{BTreeMap, VecDeque};
use std::io::{self, BufRead as _, BufReader, Write as _};
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;
use std::{fs, thread};

use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use shed_proto::{
    ErrorCode, Event, EventKind, EventsParams, ExecParams, ExecResult, FileParams, Hello, Input,
    LsParams, Method, Reported, Request, Response, RpcError, RunParams, SendParams, State,
    StateParams, Target, TerminalInfo,
};

type Handler = Arc<dyn Fn(&mut World, Value) -> Result<Value, RpcError> + Send + Sync>;
type Builtin = fn(&mut World, Value) -> Result<Value, RpcError>;

#[derive(Clone)]
enum Reply {
    Given(Result<Value, RpcError>),
    Handled(Handler),
}

#[derive(Default)]
struct Book {
    replies: Vec<(Method, Reply)>,
    seen: Vec<(Method, Value)>,
    world: World,
    conns: Vec<UnixStream>,
}

/// What the built-in methods keep and a handler is given: terminals, files, cued replies, exec answers and
/// the event log with its live `events` streams.
#[derive(Default)]
pub struct World {
    /// The terminals, by name.
    pub terminals: BTreeMap<String, TerminalInfo>,
    /// The files `file.read` and `file.write` use, by path.
    pub files: BTreeMap<PathBuf, Vec<u8>>,
    cues: BTreeMap<String, VecDeque<String>>,
    execs: Vec<(Vec<String>, ExecResult)>,
    kept: Vec<Event>,
    seq: u64,
    floor: u64,
    pids: i32,
    subs: Vec<(BTreeMap<String, String>, UnixStream)>,
}

/// A fake server on a socket in a temp folder of its own; dropping it stops it and removes the folder.
pub struct Server {
    dir: PathBuf,
    socket: PathBuf,
    book: Arc<Mutex<Book>>,
    stopped: Arc<AtomicBool>,
}

impl Server {
    /// Starts a server that answers `hello` as shed does, `run`, `get`, `ls`, `kill`, `rm`, `send`, `state`,
    /// `exec`, `file.read`, `file.write` and `events` from its [`World`], and every other method `MethodNotFound`.
    ///
    /// # Errors
    /// When the temp folder or the socket cannot be made.
    pub fn start() -> io::Result<Self> {
        let dir = folder()?;
        let socket = dir.join("shed.sock");
        let listener = match UnixListener::bind(&socket) {
            Ok(listener) => listener,
            Err(e) => {
                fs::remove_dir_all(&dir).ok();
                return Err(e);
            }
        };
        let (book, stopped) = (Arc::<Mutex<Book>>::default(), Arc::<AtomicBool>::default());
        let (shared, stop) = (Arc::clone(&book), Arc::clone(&stopped));
        thread::spawn(move || {
            for stream in listener.incoming().map_while(Result::ok) {
                if stop.load(Ordering::SeqCst) {
                    return;
                }
                if let Ok(conn) = stream.try_clone() {
                    lock(&shared).conns.push(conn);
                }
                let book = Arc::clone(&shared);
                thread::spawn(move || serve(stream, &book));
            }
        });
        Ok(Self {
            dir,
            socket,
            book,
            stopped,
        })
    }

    /// The socket to give the client.
    #[must_use]
    pub fn socket(&self) -> &Path {
        &self.socket
    }

    /// Answers `method` with `result` from now on; a streamed method's array is its items, one line each.
    pub fn answer(&self, method: Method, result: Value) {
        self.set(method, Reply::Given(Ok(result)));
    }

    /// Answers `method` with `error` from now on.
    pub fn fail(&self, method: Method, error: RpcError) {
        self.set(method, Reply::Given(Err(error)));
    }

    /// Answers `method` with `handler`'s result for its params from now on, over the built-in one.
    pub fn handle(
        &self,
        method: Method,
        handler: impl Fn(&mut World, Value) -> Result<Value, RpcError> + Send + Sync + 'static,
    ) {
        self.set(method, Reply::Handled(Arc::new(handler)));
    }

    /// Runs `f` on the server's [`World`], as no call is answered.
    pub fn with<R>(&self, f: impl FnOnce(&mut World) -> R) -> R {
        f(&mut lock(&self.book).world)
    }

    /// Every call so far but `hello`, in order, with its params.
    #[must_use]
    pub fn seen(&self) -> Vec<(Method, Value)> {
        lock(&self.book).seen.clone()
    }

    /// Closes every connection, as a shed restart onto its own binary does: streams end, the terminals and
    /// the event log live on.
    pub fn restart(&self) {
        let mut book = lock(&self.book);
        book.world.subs.clear();
        for conn in book.conns.drain(..) {
            conn.shutdown(Shutdown::Both).ok();
        }
    }

    fn set(&self, method: Method, reply: Reply) {
        let replies = &mut lock(&self.book).replies;
        replies.retain(|(m, _)| *m != method);
        replies.push((method, reply));
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
        self.restart();
        UnixStream::connect(&self.socket).ok();
        fs::remove_dir_all(&self.dir).ok();
    }
}

impl World {
    /// Starts a terminal as `run` does, emitting `started`.
    ///
    /// # Errors
    /// `NAME_TAKEN` when a running terminal has its name.
    pub fn run(&mut self, params: RunParams) -> Result<TerminalInfo, RpcError> {
        let next = self.terminals.len() + 1;
        let name = params.name.unwrap_or_else(|| format!("t{next}"));
        if self
            .terminal(&name)
            .is_ok_and(|t| t.state == State::Running)
        {
            return Err(RpcError::new(ErrorCode::NameTaken, name));
        }
        self.pids += 1;
        let info = TerminalInfo {
            name: name.clone(),
            labels: params.labels,
            cwd: params.cwd,
            cmd: params.cmd,
            keep: params.keep,
            state: State::Running,
            exit: None,
            pid: Some(self.pids),
            reported: None,
            records: Vec::new(),
            idle_after: params.idle_after,
            resources: None,
            acp: params.acp,
            session: None,
            parent: params.parent,
            mem_max_mb: params.mem_max_mb,
            redacted: false,
            account: params.account,
            auto: false,
            sandboxed: params.sandbox.is_some(),
        };
        self.terminals.insert(name.clone(), info.clone());
        self.emit(&name, EventKind::Started);
        Ok(info)
    }

    /// Terminal `name`.
    ///
    /// # Errors
    /// `NO_SUCH_TERMINAL` when there is none.
    pub fn terminal(&mut self, name: &str) -> Result<&mut TerminalInfo, RpcError> {
        let gone = || RpcError::new(ErrorCode::NoSuchTerminal, name);
        self.terminals.get_mut(name).ok_or_else(gone)
    }

    /// Terminal `name` reports `report`, as `state` does, emitting `state`.
    ///
    /// # Errors
    /// As [`World::terminal`].
    pub fn report(&mut self, name: &str, report: Reported) -> Result<TerminalInfo, RpcError> {
        let terminal = self.terminal(name)?;
        terminal.reported = Some(report.clone());
        let info = terminal.clone();
        self.emit(name, EventKind::State(Box::new(report)));
        Ok(info)
    }

    /// Terminal `name`'s program exits by itself with `code`, emitting `exited`.
    ///
    /// # Errors
    /// As [`World::terminal`].
    pub fn exit(&mut self, name: &str, code: i32) -> Result<TerminalInfo, RpcError> {
        let code = Some(code);
        self.end(name, code, EventKind::Exited { code })
    }

    /// Terminal `name`'s agent answers each prompt `send` gives it with the next of `replies` (the last
    /// repeated): `turn`, `text`, `reply`, then the state `done`.
    pub fn cue(&mut self, name: &str, replies: &[&str]) {
        let replies = replies.iter().map(|r| (*r).to_owned()).collect();
        self.cues.insert(name.to_owned(), replies);
    }

    /// `exec` of exactly `cmd` gives `result`; any other program fails `SPAWN_FAILED`.
    pub fn on_exec(&mut self, cmd: &[&str], result: ExecResult) {
        let cmd: Vec<String> = cmd.iter().map(|c| (*c).to_owned()).collect();
        self.execs.retain(|(c, _)| *c != cmd);
        self.execs.push((cmd, result));
    }

    /// Emits terminal `name`'s event to every live `events` stream whose labels it carries and keeps it; its seq.
    pub fn emit(&mut self, name: &str, kind: EventKind) -> u64 {
        self.seq += 1;
        let terminal = self.terminals.get(name);
        let event = Event {
            seq: self.seq,
            name: name.to_owned(),
            labels: terminal.map(|t| t.labels.clone()).unwrap_or_default(),
            parent: terminal.and_then(|t| t.parent.as_ref().map(|p| p.name.clone())),
            kind,
            replay: false,
        };
        let line = json!(event).to_string();
        self.subs
            .retain_mut(|(labels, w)| !carries(&event, labels) || writeln!(w, "{line}").is_ok());
        self.kept.push(event);
        self.seq
    }

    /// Drops every kept event and skips `lost` seqs, as a shed whose ring moved on does: an `events` with an
    /// older `since` gets `reset` first. The new floor.
    pub fn forget(&mut self, lost: u64) -> u64 {
        self.seq += lost;
        self.floor = self.seq;
        self.kept.clear();
        self.floor
    }

    /// Ends every live `events` stream.
    pub fn close_streams(&mut self) {
        for (_, stream) in self.subs.drain(..) {
            stream.shutdown(Shutdown::Both).ok();
        }
    }

    fn end(
        &mut self,
        name: &str,
        exit: Option<i32>,
        kind: EventKind,
    ) -> Result<TerminalInfo, RpcError> {
        let terminal = self.terminal(name)?;
        (terminal.state, terminal.exit, terminal.pid) = (State::Exited, exit, None);
        let info = terminal.clone();
        self.emit(name, kind);
        Ok(info)
    }

    fn ls(&self, params: &LsParams) -> Vec<TerminalInfo> {
        let shown = self.terminals.values().filter(|t| {
            let parent = t.parent.as_ref().map(|p| &p.name);
            within(&t.labels, &params.labels)
                && params.parent.as_ref().is_none_or(|p| parent == Some(p))
        });
        shown.cloned().collect()
    }

    fn rm(&mut self, name: &str) -> Result<TerminalInfo, RpcError> {
        if self.terminal(name)?.state == State::Running {
            return Err(RpcError::new(ErrorCode::StillRunning, name));
        }
        self.terminal(name).cloned().inspect(|_| {
            self.terminals.remove(name);
        })
    }

    fn send(&mut self, params: &SendParams) -> Result<(), RpcError> {
        let name = params.name.as_str();
        self.terminal(name)?;
        let framed = params.text.strip_prefix('\x1e').unwrap_or_default();
        let Ok(Input::Prompt(prompt)) = serde_json::from_str(framed.trim_end()) else {
            return Ok(());
        };
        let Some(cue) = self.cues.get_mut(name) else {
            return Ok(());
        };
        let Some(text) = (if cue.len() > 1 {
            cue.pop_front()
        } else {
            cue.front().cloned()
        }) else {
            return Ok(());
        };
        let agent = Vec::new();
        self.emit(name, EventKind::Turn { prompt, agent });
        let chunk = text.clone();
        self.emit(name, EventKind::Text { text: chunk });
        let stop_reason = "end_turn".to_owned();
        self.emit(name, EventKind::Reply { text, stop_reason });
        let state = "done".to_owned();
        self.report(
            name,
            Reported {
                state,
                ..Reported::default()
            },
        )
        .map(drop)
    }

    fn exec(&self, params: &ExecParams) -> Result<ExecResult, RpcError> {
        let given = self.execs.iter().find(|(cmd, _)| *cmd == params.cmd);
        let failed = || RpcError::new(ErrorCode::SpawnFailed, params.cmd.join(" "));
        given.map(|(_, result)| result.clone()).ok_or_else(failed)
    }

    fn file_read(&self, params: &FileParams) -> Result<Vec<u8>, RpcError> {
        let missing = || RpcError::new(ErrorCode::Io, params.path.display());
        self.files.get(&params.path).cloned().ok_or_else(missing)
    }

    fn file_write(&mut self, params: FileParams) -> Result<(), RpcError> {
        let none = || RpcError::new(ErrorCode::InvalidParams, "file.write needs `data`");
        self.files
            .insert(params.path, params.data.ok_or_else(none)?);
        Ok(())
    }
}

fn within(labels: &BTreeMap<String, String>, wanted: &BTreeMap<String, String>) -> bool {
    wanted.iter().all(|(k, v)| labels.get(k) == Some(v))
}

fn carries(event: &Event, wanted: &BTreeMap<String, String>) -> bool {
    within(&event.labels, wanted)
}

fn params<P: DeserializeOwned>(params: Value) -> Result<P, RpcError> {
    serde_json::from_value(params).map_err(|e| RpcError::new(ErrorCode::InvalidParams, e))
}

fn builtin(method: Method) -> Option<Builtin> {
    let answer: Builtin = match method {
        Method::Run => |w, p| w.run(params(p)?).map(|t| json!(t)),
        Method::Get => |w, p| w.terminal(&params::<Target>(p)?.name).map(|t| json!(t)),
        Method::Ls => |w, p| Ok(json!(w.ls(&params(p)?))),
        Method::Kill => |w, p| {
            let name = params::<Target>(p)?.name;
            w.end(&name, Some(143), EventKind::Killed).map(|t| json!(t))
        },
        Method::Rm => |w, p| w.rm(&params::<Target>(p)?.name).map(|t| json!(t)),
        Method::Send => |w, p| w.send(&params(p)?).map(|()| json!({})),
        Method::State => |w, p| {
            let StateParams { name, report } = params(p)?;
            w.report(&name, report).map(|t| json!(t))
        },
        Method::Exec => |w, p| w.exec(&params(p)?).map(|r| json!(r)),
        Method::FileRead => |w, p| w.file_read(&params(p)?).map(|b| json!(b)),
        Method::FileWrite => |w, p| w.file_write(params(p)?).map(|()| json!({})),
        _ => return None,
    };
    Some(answer)
}

fn folder() -> io::Result<PathBuf> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    loop {
        let n = NEXT.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("shed-fake-{n}"));
        match fs::create_dir(&dir) {
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            made => return made.map(|()| dir),
        }
    }
}

fn lock(book: &Mutex<Book>) -> MutexGuard<'_, Book> {
    book.lock().unwrap_or_else(PoisonError::into_inner)
}

fn serve(stream: UnixStream, book: &Mutex<Book>) -> Option<()> {
    let mut writer = stream.try_clone().ok()?;
    for line in BufReader::new(stream).lines() {
        let request: Request = serde_json::from_str(&line.ok()?).ok()?;
        let method = Method::ALL.into_iter().find(|m| m.name() == request.method);
        let given = |m| lock(book).replies.iter().any(|(r, _)| *r == m);
        if method == Some(Method::Events) && !given(Method::Events) {
            return subscribe(book, request.id?, request.params, writer);
        }
        let reply = reply(book, method, &request.method, request.params);
        let Some(id) = request.id else { continue };
        let streamed = matches!(
            method,
            Some(Method::Events | Method::ScreenWatch | Method::Attach)
        );
        let (result, items) = match reply {
            Ok(Value::Array(items)) if streamed => (Ok(json!({})), items),
            reply => (reply, Vec::new()),
        };
        let response = match result {
            Ok(value) => Response::ok(id, value),
            Err(error) => Response::err(id, error),
        };
        writeln!(writer, "{}", serde_json::to_string(&response).ok()?).ok()?;
        if streamed {
            for item in items {
                writeln!(writer, "{item}").ok()?;
            }
            return writer.shutdown(Shutdown::Both).ok();
        }
    }
    Some(())
}

fn subscribe(book: &Mutex<Book>, id: Value, raw: Value, mut writer: UnixStream) -> Option<()> {
    let mut book = lock(book);
    book.seen.push((Method::Events, raw.clone()));
    let wanted: EventsParams = match params(raw) {
        Ok(wanted) => wanted,
        Err(e) => {
            let refused = serde_json::to_string(&Response::err(id, e)).ok()?;
            return writeln!(writer, "{refused}").ok();
        }
    };
    writer
        .set_write_timeout(Some(Duration::from_secs(5)))
        .ok()?;
    let world = &mut book.world;
    let mut lines = vec![serde_json::to_string(&Response::ok(id, json!({}))).ok()?];
    if let Some(since) = wanted.since {
        if since < world.floor {
            let reset =
                json!({"jsonrpc": "2.0", "method": "reset", "params": {"seq": world.floor}});
            lines.push(reset.to_string());
        }
        let kept = world.kept.iter();
        let replay = kept.filter(|e| e.seq > since && carries(e, &wanted.labels));
        lines.extend(replay.map(|e| json!(e).to_string()));
    }
    for line in lines {
        writeln!(writer, "{line}").ok()?;
    }
    world.subs.push((wanted.labels, writer));
    Some(())
}

fn reply(
    book: &Mutex<Book>,
    method: Option<Method>,
    name: &str,
    params: Value,
) -> Result<Value, RpcError> {
    let unknown = || RpcError::new(ErrorCode::MethodNotFound, name);
    let Some(method) = method else {
        return Err(unknown());
    };
    let book = &mut *lock(book);
    let given = book.replies.iter().find(|(m, _)| *m == method);
    let given = given.map(|(_, reply)| reply.clone());
    if method == Method::Hello {
        let hello = serde_json::from_value(params.clone()).unwrap_or_else(|_| Hello::ours());
        let ours = || Ok(json!(Hello::answer(&hello)));
        return given.map_or_else(ours, |r| answer(r, &mut book.world, params));
    }
    book.seen.push((method, params.clone()));
    match (given, builtin(method)) {
        (Some(r), _) => answer(r, &mut book.world, params),
        (None, Some(builtin)) => builtin(&mut book.world, params),
        (None, None) => Err(unknown()),
    }
}

fn answer(reply: Reply, world: &mut World, params: Value) -> Result<Value, RpcError> {
    match reply {
        Reply::Given(given) => given,
        Reply::Handled(handler) => handler(world, params),
    }
}
