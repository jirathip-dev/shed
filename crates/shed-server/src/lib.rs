//! shed's per-user server: the socket, JSON-RPC, the events stream and machines over SSH.
//!
//! [`serve`] owns the terminals and answers newline-delimited JSON-RPC 2.0 ([`shed_proto`]) on a
//! Unix socket, one tokio task per connection. [`Client`] is the other end.
#![forbid(unsafe_code)]

pub mod acp;
pub mod agents;
pub mod config;
mod events;
pub mod hooks;
mod links;
pub mod machines;
mod providers;
mod registry;
mod screen;
mod stats;

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::io::{self, BufRead as _, BufReader, BufWriter, Read as _, Write as _};
use std::net::Shutdown;
use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use proto::{
    ErrorCode, Hello, Method, Request, ResizeParams, Response, RpcError, StatsParams, Target,
    TerminalParams, TerminalResizeParams, VERSION, WaitOutcome, WaitParams, WaitResult,
    WatchParams,
};
use registry::{Registry, Stream};
use rustix::process::{Resource, Rlimit, getrlimit, setrlimit};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use shed_core::proctree::{self, ProcessTable as _};
use shed_core::{Pattern, STATES, Terminal, Wake};
pub use shed_core::{clock, runner};
pub use shed_proto as proto;
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _};

/// Errors from the server and the client.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// `HOME` is not set.
    #[error("HOME is not set")]
    NoHome,
    /// `XDG_RUNTIME_DIR` is not set (Linux), so there is no default socket path.
    #[error("XDG_RUNTIME_DIR is not set; pass a directory with --dir")]
    NoRuntimeDir,
    /// A server already answers on the socket.
    #[error("a shed server already runs on {0}")]
    AlreadyRunning(PathBuf),
    /// The server answered with an error (including the `SHED_VERSION` refusal).
    #[error("{0}")]
    Rpc(RpcError),
    /// The server closed the connection or sent something that is not a response.
    #[error("bad response from the server: {0}")]
    BadResponse(String),
    /// The ACP agent could not start or failed the handshake.
    #[error("acp: {0}")]
    Acp(String),
    /// Installing, listing or removing an ACP agent failed.
    #[error("agents: {0}")]
    Agents(String),
    /// A JSON error.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// An I/O error.
    #[error(transparent)]
    Io(#[from] io::Error),
    /// The config file or a `machine add` argument is not valid.
    #[error("{0}")]
    Config(String),
    /// No machine has that name.
    #[error("no machine {0}; see `shed machine ls`")]
    NoMachine(String),
    /// The shed on a machine is another version.
    #[error(
        "SHED_VERSION: shed {here} here, {there} on {machine}; install the same version on both, e.g. \
         on {machine}: `cargo install --locked --git https://github.com/jirathip-dev/shed --tag v{here} \
         shed`, then restart its server"
    )]
    MachineVersion {
        /// The machine.
        machine: String,
        /// This shed's version.
        here: String,
        /// The version of the server on the machine.
        there: String,
    },
    /// `ssh` or the shed on a machine gave no answer.
    #[error(
        "no answer from shed on {machine}; check that `ssh {target} shed hello` works with key login \
         and that a server runs there (`shed serve --detach`)"
    )]
    Unreachable {
        /// The machine.
        machine: String,
        /// Its ssh target.
        target: String,
    },
}

/// Where a server listens and keeps its state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    /// The Unix socket.
    pub socket: PathBuf,
    /// The state file.
    pub state: PathBuf,
    /// The config file (machines).
    pub config: PathBuf,
}

impl Paths {
    /// `shed.sock` and `state.json`, both in `dir`.
    #[must_use]
    pub fn in_dir(dir: &Path) -> Self {
        Self {
            socket: dir.join("shed.sock"),
            state: dir.join("state.json"),
            config: dir.join("config.toml"),
        }
    }

    /// The per-user defaults. macOS: `~/Library/Caches/shed/shed.sock`, and `state.json` and
    /// `config.toml` in `~/Library/Application Support/shed/`. Elsewhere: `$XDG_RUNTIME_DIR/shed.sock`,
    /// `$XDG_STATE_HOME/shed/state.json` (default `~/.local/state`) and
    /// `$XDG_CONFIG_HOME/shed/config.toml` (default `~/.config`).
    ///
    /// # Errors
    /// [`Error::NoHome`] or [`Error::NoRuntimeDir`] when the variables they need are unset.
    pub fn user() -> Result<Self, Error> {
        let var = |k| config::var_os(k).filter(|v: &OsString| !v.is_empty());
        let home = var("HOME").map(PathBuf::from).ok_or(Error::NoHome);
        if cfg!(target_os = "macos") {
            let lib = home?.join("Library");
            return Ok(Self {
                socket: lib.join("Caches/shed/shed.sock"),
                state: lib.join("Application Support/shed/state.json"),
                config: lib.join("Application Support/shed/config.toml"),
            });
        }
        let runtime = var("XDG_RUNTIME_DIR").ok_or(Error::NoRuntimeDir)?;
        let home = home?;
        let xdg = |k, default| var(k).map_or_else(|| home.join(default), PathBuf::from);
        Ok(Self {
            socket: PathBuf::from(runtime).join("shed.sock"),
            state: xdg("XDG_STATE_HOME", ".local/state").join("shed/state.json"),
            config: xdg("XDG_CONFIG_HOME", ".config").join("shed/config.toml"),
        })
    }
}

type Shared = Arc<Mutex<Registry>>;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> T + Send + 'static,
) -> Result<T, RpcError> {
    let joined = tokio::task::spawn_blocking(f).await;
    joined.map_err(|e| RpcError::new(ErrorCode::Internal, e))
}

fn watch(registry: &Shared, term: Arc<Terminal>) {
    let registry = Arc::clone(registry);
    tokio::spawn(async move {
        loop {
            let last = term.report();
            lock(&registry).refresh().ok();
            if !matches!(
                term.wait_until(|t| t.report() != last, None).await,
                Ok(Wake::Matched)
            ) {
                lock(&registry).refresh().ok();
                return;
            }
        }
    });
}

async fn sample(registry: Shared, every: Duration) {
    loop {
        tokio::time::sleep(every).await;
        if let Ok(Ok(procs)) = blocking(|| proctree::system().list()).await {
            lock(&registry).sample(&procs, shed_core::clock::now());
        }
    }
}

async fn record_stats(registry: &Mutex<Registry>) -> Duration {
    let (wait, dir) = {
        let reg = lock(registry);
        (reg.stats.until_due(shed_core::clock::unix_now()), reg.dir())
    };
    if wait > 0 {
        return Duration::from_secs(wait);
    }
    let Ok(sample) = blocking(move || stats::system(&dir)).await else {
        return Duration::from_secs(1);
    };
    let mut reg = lock(registry);
    reg.refresh().ok();
    reg.record(sample);
    Duration::from_secs(reg.stats.until_due(shed_core::clock::unix_now()).max(1))
}

/// Loads the state, binds the socket (mode 0600, its directory 0700), prints `shed ready <version>
/// <socket>` and serves until killed, sampling each terminal's memory and CPU every `sample_every`.
///
/// # Errors
/// [`Error::AlreadyRunning`] when another server answers on the socket or holds the state file; [`Error::Io`] or
/// [`Error::Json`] when the state file or socket cannot be set up.
pub fn serve(paths: &Paths, sample_every: Duration) -> Result<(), Error> {
    if UnixStream::connect(&paths.socket).is_ok() {
        return Err(Error::AlreadyRunning(paths.socket.clone()));
    }
    if let Some(dir) = paths.state.parent() {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
    }
    let held = fs::File::create(paths.state.with_extension("lock"))?;
    match held.try_lock() {
        Err(fs::TryLockError::WouldBlock) => {
            return Err(Error::AlreadyRunning(paths.socket.clone()));
        }
        locked => locked.map_err(io::Error::from)?,
    }
    raise_open_files();
    let workers = std::thread::available_parallelism().map_or(1, |n| n.get().min(4));
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers)
        .enable_all()
        .build()?;
    runtime.block_on(listen(paths, sample_every))
}

async fn listen(paths: &Paths, sample_every: Duration) -> Result<(), Error> {
    let registry = Arc::new(Mutex::new(Registry::load(&paths.state, &paths.socket)?));
    for term in lock(&registry).running() {
        watch(&registry, term);
    }
    tokio::spawn(sample(Arc::clone(&registry), sample_every));
    let first = record_stats(&registry).await;
    let recorder = Arc::clone(&registry);
    tokio::spawn(async move {
        let mut wait = first;
        loop {
            tokio::time::sleep(wait).await;
            wait = record_stats(&recorder).await;
        }
    });
    let control = paths.socket.parent().unwrap_or(Path::new("."));
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(control)?;
    let links = Arc::new(links::Links::new(paths.config.clone(), control.to_owned()));
    links.sync(&registry);
    match fs::remove_file(&paths.socket) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e.into()),
        _ => {}
    }
    let listener = tokio::net::UnixListener::bind(&paths.socket)?;
    fs::set_permissions(&paths.socket, fs::Permissions::from_mode(0o600))?;
    writeln!(
        io::stdout(),
        "shed ready {VERSION} {}",
        paths.socket.display()
    )
    .ok();
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                tokio::spawn(connection(
                    Arc::clone(&registry),
                    Arc::clone(&links),
                    stream,
                ));
            }
            Err(e) => {
                writeln!(io::stderr(), "shed serve: accept: {e}").ok();
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
}

fn raise_open_files() {
    let old = getrlimit(Resource::Nofile);
    let cap = if cfg!(target_os = "macos") {
        runner::output("/usr/sbin/sysctl", ["-n", "kern.maxfilesperproc"], &[])
            .ok()
            .and_then(|out| String::from_utf8(out.stdout).ok()?.trim().parse().ok())
    } else {
        None
    };
    let target = match (old.maximum, cap) {
        (Some(hard), Some(cap)) => Some(hard.min(cap)),
        (hard, None) => hard,
        (None, cap) => cap,
    };
    if (old.current).is_none_or(|soft| target.is_some_and(|target| soft >= target)) {
        return;
    }
    let new = Rlimit {
        current: target,
        ..old
    };
    if let Err(e) = setrlimit(Resource::Nofile, new) {
        let show = |n: Option<u64>| n.map_or_else(|| "unlimited".to_owned(), |n| n.to_string());
        let (from, to) = (show(old.current), show(new.current));
        writeln!(
            io::stderr(),
            "shed serve: open-file limit {from} -> {to}: {e}"
        )
        .ok();
    }
}

#[derive(Default)]
struct Session {
    hello: bool,
    stream: Option<Stream>,
    watching: Option<String>,
    readonly: BTreeSet<String>,
    links: Arc<links::Links>,
}

fn readonly<'a>(method: Method, request: &'a Request, session: &'a Session) -> Option<&'a str> {
    let input = matches!(
        method,
        Method::Send
            | Method::Model
            | Method::Keys
            | Method::Resize
            | Method::Input
            | Method::TerminalKeys
            | Method::TerminalResize
            | Method::Attach
    );
    let mut named = ["terminal", "name"].map(|k| request.params.get(k).and_then(Value::as_str));
    if named == [None, None] {
        named[0] = session.watching.as_deref();
    }
    (named.into_iter().flatten()).find(|n| input && session.readonly.contains(*n))
}

async fn connection(registry: Shared, held: Arc<links::Links>, stream: tokio::net::UnixStream) {
    let mut session = Session {
        links: held,
        ..Session::default()
    };
    let (read, out) = stream.into_split();
    let (mut out, mut streaming) = (Some(out), tokio::task::JoinSet::new());
    let mut lines = tokio::io::BufReader::new(read).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        let response = respond(&registry, &mut session, &line).await;
        if let (Some(response), Some(out)) = (response, &mut out) {
            let mut bytes = serde_json::to_vec(&response).unwrap_or_default();
            bytes.push(b'\n');
            if out.write_all(&bytes).await.is_err() {
                return;
            }
        }
        if let Some(stream) = session.stream.take()
            && let Some(out) = out.take()
        {
            streaming.spawn(stream.run(out));
        }
    }
}

async fn respond(registry: &Shared, session: &mut Session, line: &str) -> Option<Response> {
    let request = match serde_json::from_str::<Request>(line).or_else(|_| invalid(line)) {
        Ok(r) if r.jsonrpc == proto::JSONRPC => r,
        Ok(r) => {
            let e = RpcError::new(ErrorCode::InvalidRequest, "jsonrpc must be \"2.0\"");
            return Some(Response::err(r.id.unwrap_or_default(), e));
        }
        Err((id, e)) => return Some(Response::err(id, e)),
    };
    let result = dispatch(registry, session, &request).await;
    let id = request.id?;
    Some(match result {
        Ok(v) => Response::ok(id, v),
        Err(e) => Response::err(id, e),
    })
}

fn invalid(line: &str) -> Result<Request, (Value, RpcError)> {
    let parse = |e| (Value::Null, RpcError::new(ErrorCode::Parse, e));
    let value: Value = serde_json::from_str(line).map_err(parse)?;
    let id = value.get("id").cloned().unwrap_or(Value::Null);
    serde_json::from_value(value).map_err(|e| (id, RpcError::new(ErrorCode::InvalidRequest, e)))
}

fn params<T: DeserializeOwned>(request: &Request) -> Result<T, RpcError> {
    let empty = Value::Object(serde_json::Map::new());
    let p = Some(&request.params).filter(|p| !p.is_null());
    T::deserialize(p.unwrap_or(&empty)).map_err(|e| RpcError::new(ErrorCode::InvalidParams, e))
}

fn json<T: Serialize>(value: &T) -> Result<Value, RpcError> {
    serde_json::to_value(value).map_err(|e| RpcError::new(ErrorCode::Internal, e))
}

async fn linked(
    registry: &Shared,
    links: &links::Links,
    method: Method,
    request: &Request,
) -> Option<Result<Value, RpcError>> {
    if let Some((machine, params)) = links::remote(method, request) {
        return Some(links.call(&machine, method, params).await);
    }
    if method == Method::MachineLinks {
        links.sync(registry);
        return Some(json(&links.report()));
    }
    if method != Method::Ls {
        return None;
    }
    let list = params(request).and_then(|p| {
        let mut reg = lock(registry);
        reg.refresh()?;
        json(&reg.ls(&p))
    });
    let Ok(Value::Array(mut list)) = list else {
        return Some(list);
    };
    for machine in links.up() {
        if let Ok(Value::Array(more)) = links.call(&machine, method, request.params.clone()).await {
            list.extend(more);
        }
    }
    Some(Ok(Value::Array(list)))
}

async fn dispatch(
    registry: &Shared,
    session: &mut Session,
    request: &Request,
) -> Result<Value, RpcError> {
    let method = Method::parse(&request.method)
        .ok_or_else(|| RpcError::new(ErrorCode::MethodNotFound, &request.method))?;
    if method == Method::Hello {
        let p: Hello = params(request)?;
        if p != Hello::ours() {
            return Err(RpcError::version(&p.label(), &Hello::ours().label()));
        }
        session.hello = true;
        return json(&Hello::ours());
    }
    if !session.hello && method != Method::Discover {
        return Err(RpcError::new(
            ErrorCode::NoHello,
            "send hello with your version first",
        ));
    }
    if let Some(name) = readonly(method, request, session) {
        return Err(RpcError::new(ErrorCode::ReadOnly, name));
    }
    if let Some(answer) = linked(registry, &session.links, method, request).await {
        return answer;
    }
    if method == Method::Kill {
        let Target { name } = params(request)?;
        let term = lock(registry).mark_kill(&name)?;
        let Some(term) = term else {
            return json(&lock(registry).get(&name)?);
        };
        let killing = Arc::clone(&term);
        let status = blocking(move || killing.kill(registry::GRACE)).await?;
        let status = status.map_err(|e| RpcError::new(ErrorCode::Internal, e))?;
        return json(&lock(registry).killed(&name, &term, status)?);
    }
    if method == Method::Wait {
        return json(&wait(registry, &params(request)?).await?);
    }
    if method == Method::MachineStats {
        return machine_stats(registry, params(request)?).await;
    }
    let mut reg = lock(registry);
    reg.refresh()?;
    match method {
        Method::Run => {
            let info = reg.run(params(request)?)?;
            if let (Some(term), _) = reg.to_wait(&info.name)? {
                watch(registry, term);
            }
            json(&info)
        }
        Method::Get => json(&reg.get(&params::<Target>(request)?.name)?),
        Method::Read => json(&reg.read(&params(request)?)?),
        Method::Send | Method::Input => reg.send(&params(request)?).map(|()| empty()),
        Method::Keys | Method::TerminalKeys => reg.keys(&params(request)?).map(|()| empty()),
        Method::Rm => json(&reg.rm(&params::<Target>(request)?.name)?),
        Method::Resize => reg.resize(&params(request)?).map(|()| empty()),
        Method::Attach => {
            let (info, attached) = reg.attach(&params::<Target>(request)?.name)?;
            session.stream = Some(attached);
            json(&info)
        }
        Method::Events => {
            session.stream = Some(reg.subscribe(params(request)?));
            Ok(empty())
        }
        Method::ScreenWatch => {
            let p: WatchParams = params(request)?;
            let term = Arc::clone(reg.live(&p.terminal)?);
            session.stream = Some(Stream::Screen(term, p.since));
            session.watching = Some(p.terminal);
            Ok(empty())
        }
        Method::TerminalResize => {
            let p: TerminalResizeParams = params(request)?;
            let Some(name) = p.terminal.or_else(|| session.watching.clone()) else {
                let e = "give terminal, or watch one first";
                return Err(RpcError::new(ErrorCode::InvalidParams, e));
            };
            let (rows, cols) = (p.rows, p.cols);
            reg.resize(&ResizeParams { name, rows, cols })
                .map(|()| empty())
        }
        Method::ReadOnly => {
            let TerminalParams { terminal } = params(request)?;
            reg.get(&terminal)?;
            session.readonly.insert(terminal);
            Ok(empty())
        }
        Method::State => json(&reg.state(params(request)?)?),
        Method::Emit => reg.emit(params(request)?).map(|()| empty()),
        Method::Model => reg.model(params(request)?).map(|()| empty()),
        Method::Discover => Ok(proto::openrpc()),
        Method::Hello
        | Method::Kill
        | Method::Wait
        | Method::MachineStats
        | Method::Ls
        | Method::MachineLinks => Err(RpcError::new(ErrorCode::Internal, "handled above")),
    }
}

async fn machine_stats(registry: &Shared, p: StatsParams) -> Result<Value, RpcError> {
    let dir = lock(registry).dir();
    let now = blocking(move || stats::system(&dir)).await?;
    let mut reg = lock(registry);
    reg.refresh()?;
    json(&reg.machine_stats(now, p.since))
}

fn empty() -> Value {
    Value::Object(serde_json::Map::new())
}

async fn wait(registry: &Mutex<Registry>, p: &WaitParams) -> Result<WaitResult, RpcError> {
    let bad = |e: &dyn std::fmt::Display| RpcError::new(ErrorCode::InvalidParams, e);
    if p.until.is_empty() && p.output.is_none() {
        return Err(bad(&"give until, output or both"));
    }
    if let Some(s) = p
        .until
        .iter()
        .find(|s| !["idle", "hibernated"].contains(&s.as_str()) && !STATES.contains(&s.as_str()))
    {
        return Err(bad(&format!("unknown state {s:?}")));
    }
    let pattern = p.output.as_deref().map(Pattern::new).transpose();
    let pattern = pattern.map_err(|e| bad(&e))?;
    let timeout = p.timeout.map(Duration::try_from_secs_f64).transpose();
    let deadline = timeout
        .map_err(|e| bad(&e))?
        .and_then(|d| shed_core::clock::now().checked_add(d));
    let (term, info) = {
        let mut reg = lock(registry);
        reg.refresh()?;
        reg.to_wait(&p.name)?
    };
    let reported = |state: &str| p.until.iter().any(|s| s == state);
    let outcome = match term {
        Some(term) => {
            let done = |t: &shed_core::Terminal| {
                t.report().is_some_and(|r| reported(&r.state))
                    || pattern
                        .as_ref()
                        .is_some_and(|re| t.screen().lines().any(|l| re.is_match(l)))
            };
            match term
                .wait_until(done, deadline)
                .await
                .map_err(|e| RpcError::new(ErrorCode::Internal, e))?
            {
                Wake::Matched => WaitOutcome::Matched,
                Wake::Exited => WaitOutcome::Exited,
                Wake::Timeout => WaitOutcome::Timeout,
            }
        }
        None if info.reported.is_some_and(|r| reported(&r.state)) => WaitOutcome::Matched,
        None => WaitOutcome::Exited,
    };
    let mut reg = lock(registry);
    reg.refresh()?;
    Ok(WaitResult {
        outcome,
        terminal: reg.get(&p.name)?,
    })
}

/// A connection to a server, past the version handshake.
pub struct Client {
    reader: BufReader<UnixStream>,
    writer: BufWriter<UnixStream>,
    next_id: u64,
}

impl Client {
    /// Connects to the server on `socket` and checks it runs this version.
    ///
    /// # Errors
    /// [`Error::Rpc`] with `SHED_VERSION: client x, server y; restart the server` on a version
    /// mismatch; [`Error::Io`] when nothing listens.
    pub fn connect(socket: &Path) -> Result<Self, Error> {
        Self::over(UnixStream::connect(socket)?)
    }

    /// Speaks the protocol over `writer`, a connected stream (a socket, or one end of a socketpair
    /// whose other end is `ssh … shed connect`), and checks the server runs this version.
    ///
    /// # Errors
    /// As [`Client::connect`].
    pub fn over(writer: UnixStream) -> Result<Self, Error> {
        let mut client = Self {
            reader: BufReader::new(writer.try_clone()?),
            writer: BufWriter::new(writer),
            next_id: 1,
        };
        let server: Hello = client.call(Method::Hello, &Hello::ours())?;
        if server != Hello::ours() {
            let (here, there) = (Hello::ours().label(), server.label());
            return Err(Error::Rpc(RpcError::version(&here, &there)));
        }
        Ok(client)
    }

    /// Calls `method` with `params` and decodes its result.
    ///
    /// # Errors
    /// [`Error::Rpc`] when the server answers with an error; [`Error::BadResponse`],
    /// [`Error::Json`] or [`Error::Io`] when the exchange itself fails.
    pub fn call<P: Serialize, R: DeserializeOwned>(
        &mut self,
        method: Method,
        params: &P,
    ) -> Result<R, Error> {
        let request = Request::new(self.next_id, method, serde_json::to_value(params)?);
        self.next_id += 1;
        serde_json::to_writer(&mut self.writer, &request)?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()?;
        let mut reply = String::new();
        if self.reader.read_line(&mut reply)? == 0 {
            return Err(Error::BadResponse("connection closed".to_owned()));
        }
        let response: Response = serde_json::from_str(&reply)?;
        if response.id != request.id.unwrap_or_default() {
            return Err(Error::BadResponse(format!("id {}", response.id)));
        }
        match (response.result, response.error) {
            (_, Some(e)) => Err(Error::Rpc(e)),
            (Some(result), None) => Ok(serde_json::from_value(result)?),
            (None, None) => Err(Error::BadResponse("neither result nor error".to_owned())),
        }
    }

    /// The connection's reader (with anything it buffered) and writer: after `attach`, the reader
    /// carries the terminal's output and the writer takes [`notification`]s.
    #[must_use]
    pub fn split(self) -> (BufReader<UnixStream>, UnixStream) {
        (self.reader, self.writer.into_parts().0)
    }
}

/// One notification line (a request without an id, so no response) for `method` with `params`.
///
/// # Errors
/// [`Error::Json`] when `params` cannot be serialised.
pub fn notification<P: Serialize>(method: Method, params: &P) -> Result<Vec<u8>, Error> {
    let name = method.name();
    let mut line = format!(r#"{{"jsonrpc":"2.0","method":"{name}","params":"#).into_bytes();
    serde_json::to_writer(&mut line, params)?;
    line.extend_from_slice(b"}\n");
    Ok(line)
}

/// `shed connect`: carries the protocol between stdin/stdout and the server on `socket` (for
/// `ssh host shed connect`) until either side closes.
///
/// # Errors
/// [`Error::Io`] when nothing listens on `socket` or the copy fails.
pub fn relay(socket: &Path) -> Result<(), Error> {
    let mut stream = UnixStream::connect(socket)?;
    let mut up = stream.try_clone()?;
    std::thread::spawn(move || {
        io::copy(&mut io::stdin().lock(), &mut up).ok();
        up.shutdown(Shutdown::Write).ok();
    });
    let mut out = io::stdout().lock();
    let mut buf = [0u8; 8192];
    loop {
        let n = stream.read(&mut buf)?;
        if n == 0 {
            return Ok(());
        }
        out.write_all(buf.get(..n).unwrap_or_default())?;
        out.flush()?;
    }
}

#[cfg(test)]
fn temp_dir(name: &str) -> PathBuf {
    struct Made(std::cell::RefCell<Vec<PathBuf>>);
    impl Drop for Made {
        fn drop(&mut self) {
            for dir in self.0.borrow().iter() {
                fs::remove_dir_all(dir).ok();
            }
        }
    }
    thread_local!(static MADE: Made = const { Made(std::cell::RefCell::new(Vec::new())) });
    let dir = std::env::temp_dir().join(format!("shed-{name}-{}", runner::pid()));
    fs::remove_dir_all(&dir).ok();
    fs::create_dir_all(&dir).expect("temp dir");
    MADE.with(|m| m.0.borrow_mut().push(dir.clone()));
    dir
}
