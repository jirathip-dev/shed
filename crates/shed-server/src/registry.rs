//! The terminals a server owns, and the state file that outlives it.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _};
use std::os::unix::process::ExitStatusExt as _;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use shed_core::proctree::{self, ProcessInfo};
use shed_core::{History, Report, STATES, Size, Terminal};
use shed_proto::{
    EmitParams, ErrorCode, EventKind, EventsParams, KeysParams, Logged, LsParams, MachineStats,
    ModelParams, ReadParams, ReadResult, Reported, ResizeParams, Resources, RpcError, RunParams,
    Sample, SendParams, State, StateParams, TerminalInfo,
};

use tokio::io::AsyncWriteExt as _;
use tokio::net::unix::OwnedWriteHalf;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::events::{Events, Subscriber};
use crate::stats::Stats;

/// The size new terminals start at.
const SIZE: Size = Size { rows: 24, cols: 80 };
/// How long `kill` waits after SIGTERM before SIGKILL.
pub(crate) const GRACE: Duration = Duration::from_secs(2);
/// The state file's format version.
const FILE_VERSION: u32 = 1;

/// What `attach`, `events` and `screen.watch` stream after their response, until the source ends or a
/// write fails.
pub(crate) enum Stream {
    /// `attach`: a redraw of the screen, then the raw output.
    Output(Vec<u8>, UnboundedReceiver<Vec<u8>>),
    /// `events`: the replayed events, then the event lines.
    Events(Subscriber),
    /// `screen.watch`: the terminal, and the change count the client has, if any.
    Screen(Arc<Terminal>, Option<u64>),
}

impl Stream {
    /// Writes the stream to `out`.
    pub(crate) async fn run(self, mut out: OwnedWriteHalf) {
        match self {
            Self::Output(first, rx) => pump(&mut out, first, rx).await,
            Self::Events(sub) => sub.stream(&mut out).await,
            Self::Screen(term, since) => crate::screen::watch(&term, since, &mut out).await,
        }
    }
}

/// Writes `bytes`, then all that `rx` holds at each wake as one write.
async fn pump(out: &mut OwnedWriteHalf, mut bytes: Vec<u8>, mut rx: UnboundedReceiver<Vec<u8>>) {
    let mut batch = Vec::new();
    while out.write_all(&bytes).await.is_ok() && rx.recv_many(&mut batch, 1024).await > 0 {
        bytes = batch.concat();
        batch.clear();
    }
}

/// The state file: every terminal's last known info.
#[derive(Serialize, Deserialize)]
struct StateFile {
    version: u32,
    terminals: Vec<TerminalInfo>,
}

struct Entry {
    info: TerminalInfo,
    /// The live (or exited but still readable) terminal; `None` for one lost in a restart.
    term: Option<Arc<Terminal>>,
    /// `kill` is ending it: its exit is a `killed` event, not `exited`.
    killing: bool,
    /// The tree's use at the last sample, and the use the last `resources` event reported.
    resources: Option<Resources>,
    emitted: Option<Resources>,
    /// When the tree was last sampled and its CPU time then.
    sampled: (Instant, Duration),
}

impl Entry {
    fn new(info: TerminalInfo, term: Option<Arc<Terminal>>) -> Self {
        Self {
            info,
            term,
            killing: false,
            resources: None,
            emitted: None,
            sampled: (Instant::now(), Duration::ZERO),
        }
    }

    /// The info with the last sample, as `ls` and `get` show it.
    fn view(&self) -> TerminalInfo {
        TerminalInfo {
            resources: self.resources,
            ..self.info.clone()
        }
    }
}

/// The server's terminals by name, saved to `path` after every change.
pub(crate) struct Registry {
    entries: BTreeMap<String, Entry>,
    path: PathBuf,
    events: Events,
    /// The machine samples kept for `machine.stats`.
    pub(crate) stats: Stats,
}

fn not_found(name: &str) -> RpcError {
    RpcError::new(ErrorCode::NoSuchTerminal, name)
}

fn internal(e: impl std::fmt::Display) -> RpcError {
    RpcError::new(ErrorCode::Internal, e)
}

/// More than 10 % away from the last reported value.
fn moved(old: u64, new: u64) -> bool {
    new.abs_diff(old) * 10 > old
}

fn exit_code(status: ExitStatus) -> Option<i32> {
    status.code().or_else(|| status.signal().map(|s| 128 + s))
}

/// Terminal `name`'s history: the default caps, and its output log beside the state file at `state`.
fn history(state: &Path, name: &str) -> History {
    History {
        log: Some(state.with_file_name("history").join(format!("{name}.log"))),
        ..History::default()
    }
}

/// Starts `cmd` with `$SHED` set to the terminal's name, so `shed state` inside needs no name. The
/// screen starts from the tail of the terminal's log, if any.
fn spawn(
    cmd: &[String],
    cwd: &Path,
    name: &str,
    idle_after: Option<u64>,
    history: &History,
    acp: &[(&str, &str)],
) -> Result<Terminal, shed_core::Error> {
    let env = [("SHED", name)].into_iter().chain(acp.iter().copied());
    let term = Terminal::spawn_with(cmd, cwd, env, SIZE, history)?;
    term.set_idle_after(idle_after.map(Duration::from_secs));
    Ok(term)
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

impl Registry {
    /// Loads the state file at `path` (none yet: no terminals). Running kept terminals come back as
    /// `$SHELL` (or `/bin/sh`) in their cwd; other running ones are listed as exited. The result is saved at once.
    pub(crate) fn load(path: &Path) -> io::Result<Self> {
        let file: StateFile = match fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => StateFile {
                version: FILE_VERSION,
                terminals: Vec::new(),
            },
            Err(e) => return Err(e),
        };
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_owned());
        let mut entries = BTreeMap::new();
        let mut events = Events::default();
        for mut info in file.terminals {
            let mut term = None;
            if info.state == State::Running {
                if info.keep {
                    let cmd = vec![shell.clone()];
                    let history = history(path, &info.name);
                    term = spawn(&cmd, &info.cwd, &info.name, info.idle_after, &history, &[])
                        .ok()
                        .map(Arc::new);
                    if term.is_some() {
                        info.cmd = cmd;
                    }
                }
                if term.is_none() {
                    (info.state, info.exit) = (State::Exited, None);
                }
            }
            info.pid = term.as_ref().map(|t| t.pid());
            if term.is_some() {
                events.emit(&info.name, &info.labels, EventKind::Started);
            }
            entries.insert(info.name.clone(), Entry::new(info, term));
        }
        // Logs and transcripts of terminals the file no longer names (removed, or lost before a save).
        for dir in ["history", "acp"] {
            for file in fs::read_dir(path.with_file_name(dir)).into_iter().flatten() {
                let Ok(file) = file else { continue };
                let stem = file
                    .path()
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned());
                if stem.is_some_and(|s| !entries.contains_key(&s)) {
                    let _ = fs::remove_file(file.path());
                }
            }
        }
        let registry = Self {
            entries,
            path: path.to_owned(),
            events,
            stats: Stats::load(&path.with_file_name("stats.json")),
        };
        registry.save()?;
        Ok(registry)
    }

    /// Writes the state file atomically (temp file, fsync, rename), mode 0600.
    fn save(&self) -> io::Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir)?;
        }
        let file = StateFile {
            version: FILE_VERSION,
            terminals: self.entries.values().map(|e| e.info.clone()).collect(),
        };
        let tmp = self.path.with_extension("json.tmp");
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        serde_json::to_writer_pretty(&mut f, &file)?;
        f.write_all(b"\n")?;
        f.sync_all()?;
        fs::rename(&tmp, &self.path)
    }

    fn saved<T>(&self, value: T) -> Result<T, RpcError> {
        self.save().map_err(internal)?;
        Ok(value)
    }

    /// Copies each terminal's reported state and marks those whose program has exited, emitting the
    /// events and saving when any changed.
    pub(crate) fn refresh(&mut self) -> Result<(), RpcError> {
        let mut changed = false;
        for entry in self.entries.values_mut() {
            let Some(term) = &entry.term else { continue };
            let reported = term.report().map(|r| Reported {
                state: r.state,
                note: r.note,
                state_source: r.source,
            });
            if entry.info.reported != reported {
                entry.info.reported.clone_from(&reported);
                if let Some(r) = reported {
                    let (name, labels) = (&entry.info.name, &entry.info.labels);
                    self.events.emit(name, labels, EventKind::State(r));
                }
                changed = true;
            }
            if entry.info.state == State::Running
                && let Some(status) = term.try_wait().map_err(internal)?
            {
                Self::exited(&mut self.events, entry, status);
                changed = true;
            }
        }
        if changed {
            self.save().map_err(internal)?;
        }
        Ok(())
    }

    /// Marks a running entry exited and emits `killed` or `exited`.
    fn exited(events: &mut Events, entry: &mut Entry, status: ExitStatus) {
        let code = exit_code(status);
        (entry.info.state, entry.info.exit) = (State::Exited, code);
        entry.info.pid = None;
        entry.resources = None;
        let kind = if entry.killing {
            EventKind::Killed
        } else {
            EventKind::Exited { code }
        };
        events.emit(&entry.info.name, &entry.info.labels, kind);
    }

    /// Records each running tree's memory and CPU from `procs`, emitting `resources` when either moved
    /// by more than 10 % since the last one.
    pub(crate) fn sample(&mut self, procs: &[ProcessInfo], now: Instant) {
        for entry in self.entries.values_mut() {
            let Some(pid) = entry.info.pid else { continue };
            let tree = proctree::tree(procs, pid, &std::collections::BTreeSet::new());
            if tree.is_empty() {
                continue;
            }
            let usage = proctree::usage(&tree);
            let (then, cpu) = entry.sampled;
            let wall = now.saturating_duration_since(then).as_micros().max(1);
            let busy = usage.cpu.saturating_sub(cpu).as_micros();
            entry.sampled = (now, usage.cpu);
            let cpu_pct = u64::try_from((busy * 100 + wall / 2) / wall).unwrap_or(u64::MAX);
            let now = Resources {
                rss_mb: usage.rss_bytes / (1 << 20),
                cpu_pct,
            };
            entry.resources = Some(now);
            if entry
                .emitted
                .is_none_or(|old| moved(old.rss_mb, now.rss_mb) || moved(old.cpu_pct, now.cpu_pct))
            {
                entry.emitted = Some(now);
                let (name, labels) = (&entry.info.name, &entry.info.labels);
                self.events.emit(name, labels, EventKind::Resources(now));
            }
        }
    }

    /// The state file's directory.
    pub(crate) fn dir(&self) -> PathBuf {
        self.path.parent().unwrap_or(Path::new(".")).to_owned()
    }

    /// Terminals by state: the reported state while running, else `running`; `exited`.
    fn counts(&self) -> BTreeMap<String, u64> {
        let mut counts = BTreeMap::new();
        for e in self.entries.values() {
            let state = match (e.info.state, &e.info.reported) {
                (State::Exited, _) => "exited",
                (State::Running, Some(r)) => &r.state,
                (State::Running, None) => "running",
            };
            *counts.entry(state.to_owned()).or_default() += 1;
        }
        counts
    }

    /// Keeps `sample` with the terminal counts.
    pub(crate) fn record(&mut self, mut sample: Sample) {
        sample.terminals = self.counts();
        self.stats.push(sample);
    }

    /// `machine.stats`: `now` with the terminal counts, and the kept samples after `since`.
    pub(crate) fn machine_stats(&self, mut now: Sample, since: Option<u64>) -> MachineStats {
        now.terminals = self.counts();
        let history = since.map(|t| self.stats.since(t)).unwrap_or_default();
        MachineStats { now, history }
    }

    /// `events`: the replay and the stream.
    pub(crate) fn subscribe(&mut self, p: EventsParams) -> Stream {
        Stream::Events(self.events.subscribe(p))
    }

    /// The live terminals, to watch.
    pub(crate) fn running(&self) -> Vec<Arc<Terminal>> {
        self.entries
            .values()
            .filter(|e| e.info.state == State::Running)
            .filter_map(|e| e.term.clone())
            .collect()
    }

    fn entry(&self, name: &str) -> Result<&Entry, RpcError> {
        self.entries.get(name).ok_or_else(|| not_found(name))
    }

    /// A running terminal.
    pub(crate) fn live(&self, name: &str) -> Result<&Arc<Terminal>, RpcError> {
        let entry = self.entry(name)?;
        match (&entry.term, entry.info.state) {
            (Some(term), State::Running) => Ok(term),
            _ => Err(RpcError::new(
                ErrorCode::NoSuchTerminal,
                format!("{name} has exited"),
            )),
        }
    }

    /// `run`: starts a terminal.
    pub(crate) fn run(&mut self, p: RunParams) -> Result<TerminalInfo, RpcError> {
        let bad = |what: &str| RpcError::new(ErrorCode::InvalidParams, what);
        if p.cmd.is_empty() {
            return Err(bad("empty cmd"));
        }
        if !p.cwd.is_absolute() {
            return Err(bad("cwd must be absolute"));
        }
        if p.labels.keys().any(|k| k.is_empty() || k.contains('=')) {
            return Err(bad("label keys must be non-empty and have no '='"));
        }
        crate::acp::mcp_servers(&p.mcp_servers).map_err(|e| bad(&e))?;
        let name = match p.name {
            Some(name) if !valid_name(&name) => return Err(bad("name must match [A-Za-z0-9._-]+")),
            Some(name) if self.entries.contains_key(&name) => {
                return Err(RpcError::new(ErrorCode::NameTaken, name));
            }
            Some(name) => name,
            // len + 1 candidates for len names: one is always free, so `find` returns Some.
            None => (1..=self.entries.len() + 1)
                .map(|n| format!("t{n}"))
                .find(|n| !self.entries.contains_key(n))
                .unwrap_or_default(),
        };
        // A new terminal starts a fresh log.
        let history = history(&self.path, &name);
        if let Some(log) = &history.log {
            let _ = fs::remove_file(log);
        }
        // The MCP servers (secrets and all) reach the bridge in its environment only.
        let mcp = serde_json::to_string(&p.mcp_servers).map_err(internal)?;
        let resume = p.resume.as_deref().map(|id| ("SHED_ACP_RESUME", id));
        let mcp = (!p.mcp_servers.is_empty()).then_some(("SHED_ACP_MCP", mcp.as_str()));
        let awake = p.no_hibernate.then_some(("SHED_ACP_HIBERNATE_AFTER", "0"));
        let acp: Vec<_> = resume.into_iter().chain(mcp).chain(awake).collect();
        let term = spawn(&p.cmd, &p.cwd, &name, p.idle_after, &history, &acp)
            .map_err(|e| RpcError::new(ErrorCode::SpawnFailed, format!("{name}: {e}")))?;
        let info = TerminalInfo {
            name: name.clone(),
            labels: p.labels,
            cwd: p.cwd,
            cmd: p.cmd,
            keep: p.keep,
            state: State::Running,
            exit: None,
            pid: Some(term.pid()),
            reported: None,
            idle_after: p.idle_after,
            resources: None,
            acp: p.acp,
            session: None,
        };
        self.events.emit(&name, &info.labels, EventKind::Started);
        let entry = Entry::new(info.clone(), Some(Arc::new(term)));
        self.entries.insert(name, entry);
        self.saved(info)
    }

    /// `ls`: terminals carrying every label in `p`, by name.
    pub(crate) fn ls(&self, p: &LsParams) -> Vec<TerminalInfo> {
        self.entries
            .values()
            .filter(|e| {
                p.labels
                    .iter()
                    .all(|(k, v)| e.info.labels.get(k) == Some(v))
            })
            .map(Entry::view)
            .collect()
    }

    /// `get`: one terminal.
    pub(crate) fn get(&self, name: &str) -> Result<TerminalInfo, RpcError> {
        Ok(self.entry(name)?.view())
    }

    /// `read`: screen or history text, or an ACP terminal's transcript rendered; for a terminal lost in
    /// a restart, from the tail of its log.
    pub(crate) fn read(&self, p: &ReadParams) -> Result<ReadResult, RpcError> {
        let entry = self.entry(&p.name)?;
        let (n, mut events) = (p.lines.unwrap_or(usize::MAX), Vec::new());
        let acp = entry.info.acp.is_some();
        let file = || fs::read_to_string(crate::acp::transcript(&self.path, &p.name));
        let mut text = match &entry.term {
            _ if acp && !p.events => crate::acp::render(&file().unwrap_or_default()),
            _ if acp => {
                let (file, before) = (file().unwrap_or_default(), p.before.unwrap_or(u64::MAX));
                let lines = (file.lines().zip(1..)).take_while(|&(_, seq)| seq < before);
                let parsed =
                    lines.filter_map(|(l, seq)| Some((seq, serde_json::from_str(l).ok()?)));
                events = parsed.map(|(seq, kind)| Logged { seq, kind }).collect();
                events.drain(..events.len().saturating_sub(p.n.unwrap_or(usize::MAX)));
                String::new()
            }
            Some(term) if p.history => term.history(n),
            Some(term) => term.screen(),
            None => {
                let screen = history(&self.path, &p.name)
                    .restore(SIZE)
                    .map_err(internal)?;
                if p.history {
                    screen.history(n)
                } else {
                    screen.text()
                }
            }
        };
        if acp || !p.history {
            let lines: Vec<&str> = text.lines().collect();
            text = lines[lines.len().saturating_sub(n)..].join("\n");
        }
        Ok(ReadResult { text, events })
    }

    /// `send`: types text.
    pub(crate) fn send(&self, p: &SendParams) -> Result<(), RpcError> {
        self.live(&p.name)?
            .write(p.text.as_bytes())
            .map_err(internal)
    }

    /// `keys`: presses keys; sends nothing if any name is unknown.
    pub(crate) fn keys(&self, p: &KeysParams) -> Result<(), RpcError> {
        let term = self.live(&p.name)?;
        let bytes = p
            .keys
            .iter()
            .map(|k| shed_core::keys::encode(k))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| RpcError::new(ErrorCode::InvalidParams, e))?;
        term.write(&bytes.concat()).map_err(internal)
    }

    /// `resize`: resizes a running terminal.
    pub(crate) fn resize(&self, p: &ResizeParams) -> Result<(), RpcError> {
        if p.rows == 0 || p.cols == 0 {
            return Err(RpcError::new(
                ErrorCode::InvalidParams,
                "rows and cols must be at least 1",
            ));
        }
        let size = Size {
            rows: p.rows,
            cols: p.cols,
        };
        self.live(&p.name)?.resize(size).map_err(internal)
    }

    /// `attach`: a running terminal, a redraw of its screen, and its raw output from now on.
    pub(crate) fn attach(&self, name: &str) -> Result<(TerminalInfo, Stream), RpcError> {
        let term = self.live(name)?;
        let output = term.subscribe();
        let redraw = format!("\x1b[H\x1b[2J{}", term.screen().replace('\n', "\r\n"));
        Ok((self.get(name)?, Stream::Output(redraw.into_bytes(), output)))
    }

    /// `state`: records a running terminal's reported state.
    pub(crate) fn state(&mut self, p: StateParams) -> Result<TerminalInfo, RpcError> {
        // An ACP bridge also reports its agent `hibernated`.
        let acp = self.entry(&p.name)?.info.acp.is_some() && p.report.state == "hibernated";
        if !acp && !STATES.contains(&p.report.state.as_str()) {
            return Err(RpcError::new(
                ErrorCode::InvalidParams,
                "state must be working, blocked, done or error",
            ));
        }
        self.live(&p.name)?.set_report(Report {
            state: p.report.state,
            note: p.report.note,
            source: None,
        });
        self.refresh()?;
        self.get(&p.name)
    }

    /// `emit`: an ACP terminal's structured event.
    pub(crate) fn emit(&mut self, p: EmitParams) -> Result<(), RpcError> {
        if !p.kind.is_acp() {
            return Err(RpcError::new(
                ErrorCode::InvalidParams,
                "emit takes only ACP events",
            ));
        }
        self.live(&p.name)?;
        let entry = self.entries.get_mut(&p.name);
        let info = &mut entry.ok_or_else(|| not_found(&p.name))?.info;
        let session = match &p.kind {
            EventKind::Session(s) => info.session.replace(s.clone()).is_none_or(|old| old != *s),
            _ => false,
        };
        let labels = info.labels.clone();
        self.events.emit_as(&p.name, &labels, p.kind, p.replay);
        // Saved, so a lost terminal's session id outlives a restart for `run --resume`.
        if session {
            self.save().map_err(internal)?;
        }
        Ok(())
    }

    /// `acp.model`: asks an ACP terminal's bridge to switch the agent's model.
    pub(crate) fn model(&self, p: ModelParams) -> Result<(), RpcError> {
        let why = format!("{} is not an ACP terminal", p.name);
        let acp = self.entry(&p.name)?.info.acp.as_ref();
        acp.ok_or_else(|| RpcError::new(ErrorCode::InvalidParams, why))?;
        let text = crate::acp::frame(&crate::acp::Input::Model(p.model));
        self.send(&SendParams { name: p.name, text })
    }

    /// The terminal to wait on (`None` when lost in a restart) and its info; the caller waits without
    /// holding the registry.
    pub(crate) fn to_wait(
        &self,
        name: &str,
    ) -> Result<(Option<Arc<Terminal>>, TerminalInfo), RpcError> {
        let entry = self.entry(name)?;
        Ok((entry.term.clone(), entry.info.clone()))
    }

    /// The terminal to kill, if it still runs after a refresh, marked so its exit is `killed`; the
    /// caller kills it without holding the registry.
    pub(crate) fn mark_kill(&mut self, name: &str) -> Result<Option<Arc<Terminal>>, RpcError> {
        self.refresh()?;
        let entry = self.entries.get_mut(name).ok_or_else(|| not_found(name))?;
        entry.killing = entry.info.state == State::Running;
        Ok(entry.term.clone().filter(|_| entry.killing))
    }

    /// Records the exit of a terminal `kill` ended; returns its info.
    pub(crate) fn killed(
        &mut self,
        name: &str,
        term: &Arc<Terminal>,
        status: ExitStatus,
    ) -> Result<TerminalInfo, RpcError> {
        let entry = self.entries.get_mut(name).ok_or_else(|| not_found(name))?;
        if entry.info.state == State::Running
            && entry.term.as_ref().is_some_and(|t| Arc::ptr_eq(t, term))
        {
            Self::exited(&mut self.events, entry, status);
        }
        let info = entry.view();
        self.saved(info)
    }

    /// `rm`: removes an exited terminal.
    pub(crate) fn rm(&mut self, name: &str) -> Result<TerminalInfo, RpcError> {
        if self.entry(name)?.info.state == State::Running {
            return Err(RpcError::new(ErrorCode::StillRunning, name));
        }
        let entry = self.entries.remove(name).ok_or_else(|| not_found(name))?;
        if let Some(log) = history(&self.path, name).log {
            let _ = fs::remove_file(log);
        }
        let _ = fs::remove_file(crate::acp::transcript(&self.path, name));
        self.saved(entry.info)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("shed-registry-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// R2: the state file each past release wrote (M1; M2's report and idle fallback; M2c's ACP agent;
    /// #27's state source) loads, and so does one with keys this release does not know.
    #[test]
    fn state_files_of_every_past_release_load() {
        let terminal = |name: &str, extra: &str| {
            format!(
                r#"{{"name":"{name}","labels":{{"issue":"7"}},"cwd":"/home/user","cmd":["make"],"keep":false,"state":"exited","exit":0,"pid":null{extra}}}"#
            )
        };
        let terminals = [
            terminal("m1", ""),
            terminal(
                "m2",
                r#","reported":{"state":"done","note":"ok"},"idle_after":30"#,
            ),
            terminal("m2c", r#","acp":"claude-acp""#),
            terminal(
                "osc",
                r#","reported":{"state":"blocked","state_source":"jan-osc9999"}"#,
            ),
            terminal(
                "next",
                r#","retired_key":true,"reported":{"state":"done","color":"red"}"#,
            ),
        ];
        let path = dir("releases").join("state.json");
        let file = format!(
            r#"{{"version":2,"future":[],"terminals":[{}]}}"#,
            terminals.join(",")
        );
        fs::write(&path, file).unwrap();
        let registry = Registry::load(&path).unwrap();
        let names: Vec<String> = registry
            .ls(&LsParams::default())
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert_eq!(names, ["m1", "m2", "m2c", "next", "osc"]);
        assert_eq!(registry.get("m2").unwrap().idle_after, Some(30));
        assert_eq!(
            registry.get("m2c").unwrap().acp.as_deref(),
            Some("claude-acp")
        );
        let osc = registry.get("osc").unwrap().reported.unwrap();
        assert_eq!(osc.state_source.as_deref(), Some("jan-osc9999"));
    }

    /// R13: a state file that is not JSON stops the load with the reason and is left as it was, never
    /// replaced by an empty one.
    #[test]
    fn a_corrupt_state_file_is_refused_and_kept() {
        let path = dir("corrupt").join("state.json");
        fs::write(&path, "{\"version\":1,\"terminals\":[").unwrap();
        let e = Registry::load(&path).err().expect("refused");
        assert!(e.to_string().contains("EOF"), "{e}");
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "{\"version\":1,\"terminals\":["
        );
    }

    /// R24: the load removes logs and transcripts no terminal names, and `rm` removes the terminal's.
    #[test]
    fn leftovers_go_on_load_and_a_removed_terminal_leaves_nothing() {
        let root = dir("leftovers");
        let path = root.join("state.json");
        let file = r#"{"version":1,"terminals":[{"name":"kept.1","labels":{},"cwd":"/home/user","cmd":["make"],"keep":false,"state":"exited","exit":0,"pid":null}]}"#;
        fs::write(&path, file).unwrap();
        let files = [
            "history/kept.1.log",
            "acp/kept.1.jsonl",
            "history/gone.log",
            "history/gone.log.tmp",
            "acp/gone.jsonl",
        ];
        for f in files {
            fs::create_dir_all(root.join(f).parent().unwrap()).unwrap();
            fs::write(root.join(f), "x").unwrap();
        }
        let exists = || files.map(|f| root.join(f).exists());
        let mut registry = Registry::load(&path).unwrap();
        assert_eq!(exists(), [true, true, false, false, false]);
        registry.rm("kept.1").unwrap();
        assert_eq!(exists(), [false; 5]);
    }
}
