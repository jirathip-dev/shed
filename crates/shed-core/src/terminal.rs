//! A terminal: one program on a PTY, its screen, and its process tree.

use crate::runner::ExitStatus;
use std::ffi::OsStr;
use std::fs::File;
use std::io::{ErrorKind::BrokenPipe, Read as _, Write as _};
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::os::unix::process::ExitStatusExt as _;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak};
use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::process::{Pid, Signal, WaitOptions, kill_process_group, waitpid};
use rustix::termios::{Winsize, tcgetwinsize, tcsetwinsize};
use tokio::io::AsyncWriteExt as _;
use tokio::runtime::{Builder, Handle, Runtime, RuntimeFlavor::MultiThread};
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::{mpsc, watch};

use crate::history::{History, Log};
use crate::proctree::{self, ProcessTable as _, Usage};
use crate::screen::{Report, Screen, Size, Snapshot};
use crate::{Error, keys};

const REAP: Duration = Duration::from_millis(20);
const WRITE_TIMEOUT: Duration = Duration::from_secs(1);
const OUTPUT_BEHIND: usize = 256;
const RULE_EVERY: Duration = Duration::from_millis(250);
const QUIET: Duration = Duration::from_millis(20);
const DETACH_WAIT: Duration = Duration::from_millis(100);

/// A screen rule: while one of `any` shows in the last `region` non-blank lines of the screen, the state is
/// `state`. Above priority 0 it outranks the reported state; at or below, it applies only without one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rule {
    /// Its name; the state's `from` is `rule:<id>`.
    pub id: String,
    /// The state it sets.
    pub state: String,
    /// Why it is blocked.
    pub kind: Option<String>,
    /// The highest matching rule wins; the reported state has priority 0.
    pub priority: i32,
    /// How many of the screen's last non-blank lines it looks at.
    pub region: usize,
    /// The texts it matches, any one of them.
    pub any: Vec<String>,
}

#[derive(Default)]
struct Rules {
    list: Vec<Rule>,
    seen: Option<u64>,
    at: Option<Instant>,
    hit: Option<Rule>,
    checks: u64,
}

/// Why [`Terminal::wait_until`] returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wake {
    /// The condition holds.
    Matched,
    /// The program exited first.
    Exited,
    /// The deadline passed first.
    Timeout,
}

struct Status {
    changes: u64,
    ended: bool,
    last_output: Instant,
    report: Option<Report>,
    records: Vec<Report>,
    idle_after: Option<Duration>,
    stuck: bool,
    exit: Option<Result<ExitStatus, std::io::ErrorKind>>,
}

impl Status {
    fn apply(&mut self, report: Report) {
        match (report.state.as_str(), report.id.clone()) {
            ("clear", None) => (self.report, self.records) = (None, Vec::new()),
            ("clear", Some(id)) => self.records.retain(|r| {
                r.id.as_ref()
                    .is_some_and(|r| *r != id && !r.starts_with(&format!("{id}/")))
            }),
            (_, None) => self.report = Some(report),
            (_, Some(id)) => {
                self.records.retain(|r| r.id.as_ref() != Some(&id));
                self.records.push(report);
                if self.records.len() > 256 {
                    self.records.remove(0);
                }
            }
        }
        self.end(self.ended);
    }

    fn end(&mut self, ended: bool) {
        self.ended = ended;
        let lasts = |r: &Report| !ended || !["working", "blocked"].contains(&r.state.as_str());
        self.report = self.report.take().filter(lasts);
        self.records.retain(lasts);
    }

    fn idle_at(&self) -> Option<Instant> {
        match self.report {
            Some(_) => None,
            None => self
                .idle_after
                .and_then(|d| self.last_output.checked_add(d)),
        }
    }
}

fn bump(watch: &watch::Sender<Status>, change: impl FnOnce(&mut Status)) {
    watch.send_modify(|status| {
        change(status);
        status.changes += 1;
    });
}

/// A program running on a PTY, in its own session and process group.
///
/// Its output feeds a [`Screen`] from a reader thread. Dropping the terminal SIGKILLs its process group,
/// unless it was [`release`](Self::release)d.
pub struct Terminal {
    pty: Weak<File>,
    input: mpsc::UnboundedSender<Input>,
    screen: Arc<Mutex<Screen>>,
    outputs: Arc<Outputs>,
    watch: Arc<watch::Sender<Status>>,
    rules: Mutex<Rules>,
    pid: i32,
    released: AtomicBool,
    stop: Arc<Mutex<Option<UnixStream>>>,
    reader: Mutex<Option<std::thread::JoinHandle<()>>>,
    reaper: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

/// A terminal [`Terminal::detach`]ed from this process: its program runs on, and its output waits in the PTY.
pub struct Detached {
    /// The PTY's master.
    pub master: OwnedFd,
    /// [`Screen::repaint`] of its screen when it was detached.
    pub repaint: String,
}

type Outputs = Mutex<Option<Vec<mpsc::Sender<Arc<[u8]>>>>>;
type Input = (Vec<u8>, SyncSender<std::io::Result<()>>);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Terminal {
    /// Starts `cmd` (program and arguments) in `cwd` on a new PTY of `size`, with the default
    /// [`History`] caps and no log.
    ///
    /// The program inherits this process's environment plus `TERM=xterm-256color`, then `env`.
    ///
    /// # Errors
    /// [`Error::EmptyCommand`] when `cmd` is empty; [`Error::Pty`] when the PTY cannot be opened or the
    /// program cannot start.
    pub fn spawn<S, K, V>(
        cmd: &[S],
        cwd: &Path,
        env: impl IntoIterator<Item = (K, V)>,
        size: Size,
    ) -> Result<Self, Error>
    where
        S: AsRef<OsStr>,
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        Self::spawn_with(cmd, cwd, env, size, &History::default(), None)
    }

    /// [`spawn`](Self::spawn) keeping `history`: its caps bound the scrollback, the screen starts from
    /// the tail of its log, and the output is appended to the log. With `stderr`, the program's stderr
    /// is that file instead of the PTY.
    ///
    /// # Errors
    /// As [`spawn`](Self::spawn); [`Error::Io`] when the log cannot be read or opened.
    pub fn spawn_with<S, K, V>(
        cmd: &[S],
        cwd: &Path,
        env: impl IntoIterator<Item = (K, V)>,
        size: Size,
        history: &History,
        stderr: Option<OwnedFd>,
    ) -> Result<Self, Error>
    where
        S: AsRef<OsStr>,
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        let (program, args) = cmd.split_first().ok_or(Error::EmptyCommand)?;
        let screen = history.restore(size, None)?;
        let log = Log::open(history)?;
        let (pty, pts) = pty_process::blocking::open()?;
        pty.resize(pty_process::Size::new(size.rows.max(1), size.cols.max(1)))?;
        let child = crate::runner::pty(program, args, cwd, env, pts, stderr)?;
        let pid = i32::try_from(child.id())
            .map_err(|_| Error::Io(std::io::Error::other("pid out of range")))?;
        Self::start(OwnedFd::from(pty).into(), pid, true, screen, log)
    }

    /// Takes over a terminal that another process started: `master` is its PTY's master, `pid` its program,
    /// which leads its session. The screen starts from `history` as in [`spawn_with`](Self::spawn_with).
    /// Not being its parent, the terminal learns of the program's end when the PTY closes, as exit 0. With
    /// `repaint`, it picks up a [`detach`](Self::detach)ed terminal of this process instead: the log tail,
    /// then `repaint`, and it waits on `pid` as its child.
    ///
    /// # Errors
    /// [`Error::Io`] when the PTY's size or the log cannot be read.
    pub fn adopt(
        master: OwnedFd,
        pid: i32,
        history: &History,
        repaint: Option<&str>,
    ) -> Result<Self, Error> {
        let ws = tcgetwinsize(&master).map_err(std::io::Error::from)?;
        let size = Size {
            rows: ws.ws_row,
            cols: ws.ws_col,
        };
        let mut screen = history.restore(size, repaint)?;
        screen.take_replies();
        screen.take_reports();
        let log = Log::open(history)?;
        Self::start(master.into(), pid, repaint.is_some(), screen, log)
    }

    fn start(
        master: File,
        pid: i32,
        child: bool,
        screen: Screen,
        log: Option<Log>,
    ) -> Result<Self, Error> {
        let runtime = runtime()?;
        let (input, inputs) = mpsc::unbounded_channel();
        let screen = Arc::new(Mutex::new(screen));
        let reader = Arc::new(master);
        let pty = Arc::downgrade(&reader);
        let outputs = Arc::new(Mutex::new(Some(Vec::new())));
        let watch = Arc::new(watch::Sender::new(Status {
            changes: 0,
            ended: false,
            last_output: crate::clock::now(),
            report: None,
            records: Vec::new(),
            idle_after: None,
            stuck: false,
            exit: None,
        }));
        let (sink, subs) = (Arc::clone(&screen), Arc::clone(&outputs));
        let watcher = Arc::clone(&watch);
        let adopted = !child;
        let replies = input.clone();
        let (stop, stopped) = UnixStream::pair()?;
        let stop = Arc::new(Mutex::new(Some(stop)));
        let held = Arc::clone(&stop);
        let reader = std::thread::spawn(move || {
            let (screen, outputs) = (Arc::clone(&sink), Arc::clone(&subs));
            let watch = Arc::clone(&watcher);
            let (logs, logger) = stage(move |rx: Receiver<Vec<u8>>| {
                let mut log = log;
                batches(&rx).try_for_each(|bytes| log.as_mut()?.append(&bytes).ok())
            });
            let (chunks, feeder) =
                stage(move |rx| feed_loop(&rx, &replies, &screen, &outputs, &watch, Some(logs)));
            let detached = read_loop(&reader, &stopped, &chunks);
            drop(chunks);
            *lock(&held) = None;
            feeder.join().ok();
            logger.join().ok();
            drop(reader);
            *lock(&subs) = None;
            if detached {
                return;
            }
            bump(&watcher, |s| {
                if adopted {
                    s.exit.get_or_insert(Ok(ExitStatus::from_raw(0)));
                }
                s.end(true);
            });
        });
        let pids = Pid::from_raw(pid).filter(|_| child);
        let reaping = pids.map(|pid| runtime.spawn(reap(pid, Arc::clone(&watch))));
        runtime.spawn(write_loop(Weak::clone(&pty), inputs, Arc::clone(&watch)));
        Ok(Self {
            pty,
            input,
            screen,
            outputs,
            watch,
            rules: Mutex::default(),
            pid,
            released: AtomicBool::new(false),
            stop,
            reader: Mutex::new(Some(reader)),
            reaper: Mutex::new(reaping),
        })
    }

    /// Stops the reader once the PTY is quiet (at most 100 ms), flushes the log and stops the reaper; the
    /// program runs on, and the terminal's own screen and output streams stop.
    ///
    /// # Errors
    /// [`Error::Io`] when the PTY is closed: the program has ended.
    pub fn detach(&self) -> Result<Detached, Error> {
        let master = self.master()?;
        self.release();
        let stop = lock(&self.stop).as_ref().map(UnixStream::try_clone);
        stop.ok_or_else(|| Error::Io(BrokenPipe.into()))??
            .write_all(b"x")?;
        if let Some(reader) = lock(&self.reader).take() {
            reader.join().ok();
        }
        if let Some(reaper) = lock(&self.reaper).take() {
            reaper.abort();
            while !reaper.is_finished() {
                crate::clock::sleep(Duration::from_millis(1));
            }
        }
        let repaint = lock(&self.screen).repaint();
        Ok(Detached { master, repaint })
    }

    /// A duplicate of the PTY's master, for a process that is to keep the terminal open.
    ///
    /// # Errors
    /// [`Error::Io`] when the PTY is closed or cannot be duplicated.
    pub fn master(&self) -> Result<OwnedFd, Error> {
        Ok(self.open_pty()?.try_clone()?.into())
    }

    /// Leaves the program running when the terminal is dropped.
    pub fn release(&self) {
        self.released.store(true, Ordering::Relaxed);
    }

    /// The program's process id; it also leads the terminal's session and process group.
    #[must_use]
    pub fn pid(&self) -> i32 {
        self.pid
    }

    /// The visible screen as plain text.
    #[must_use]
    pub fn screen(&self) -> String {
        lock(&self.screen).text()
    }

    /// The visible screen as runs, with the cursor and the scrollback length, and the
    /// [`changes`](Self::changes) count it shows.
    #[must_use]
    pub fn snapshot(&self) -> (u64, Snapshot) {
        let screen = lock(&self.screen);
        (self.changes(), screen.snapshot())
    }

    /// Counts up on every output chunk, report, resize and PTY close; the screen changes only with it.
    #[must_use]
    pub fn changes(&self) -> u64 {
        self.watch.borrow().changes
    }

    /// Whether the program has turned bracketed paste on.
    #[must_use]
    pub fn bracketed_paste(&self) -> bool {
        lock(&self.screen).bracketed_paste()
    }

    /// `f` of the screen, locked while it runs.
    pub fn with_screen<T>(&self, f: impl FnOnce(&Screen) -> T) -> T {
        f(&lock(&self.screen))
    }

    /// The state the program last reported (`shed state`, the OSC 7501 escape or another agent's), unless a
    /// [`set_rules`](Self::set_rules) rule outranks it. Without either, `idle` once silent for the
    /// [`set_idle_after`](Self::set_idle_after) time; `stuck` while a [`write`](Self::write) waits.
    #[must_use]
    pub fn report(&self) -> Option<Report> {
        let rule = self.screen_rule();
        let status = self.watch.borrow();
        if status.stuck {
            return Some(Report {
                state: "stuck".to_owned(),
                note: Some("its input is not drained".to_owned()),
                ..Report::default()
            });
        }
        let outranks = |r: &Rule| r.priority > 0 || status.report.is_none();
        if let Some(r) = rule.filter(|r| !status.ended && outranks(r)) {
            return Some(Report {
                state: r.state,
                kind: r.kind,
                from: Some(format!("rule:{}", r.id)),
                ..Report::default()
            });
        }
        let idle = status.idle_at().is_some_and(|at| crate::clock::now() >= at);
        let from = |from: &str| Some(from.to_owned());
        let reported = status.report.clone().map(|r| Report {
            from: from("reported"),
            ..r
        });
        reported.or_else(|| {
            idle.then(|| Report {
                state: "idle".to_owned(),
                from: from("silence"),
                ..Report::default()
            })
        })
    }

    /// Sets the screen rules [`report`](Self::report) checks, at most every 250 ms and only after the
    /// screen changed.
    pub fn set_rules(&self, rules: Vec<Rule>) {
        *lock(&self.rules) = Rules {
            list: rules,
            ..Rules::default()
        };
    }

    /// How many times the screen rules have been checked.
    #[must_use]
    pub fn rule_checks(&self) -> u64 {
        lock(&self.rules).checks
    }

    fn screen_rule(&self) -> Option<Rule> {
        let mut rules = lock(&self.rules);
        if rules.list.is_empty() {
            return None;
        }
        let (changes, now) = (self.changes(), crate::clock::now());
        let due = (rules.at).and_then(|at| at.checked_add(RULE_EVERY));
        if rules.seen != Some(changes) && due.is_none_or(|at| now >= at) {
            let text = self.screen();
            let lines: Vec<&str> = text.lines().collect();
            let end = lines.iter().rposition(|l| !l.trim().is_empty());
            let region = |n| lines.iter().take(end.map_or(0, |i| i + 1)).rev().take(n);
            let shows = |r: &&Rule| region(r.region).any(|l| r.any.iter().any(|s| l.contains(s)));
            let hit = rules.list.iter().filter(shows).max_by_key(|r| r.priority);
            rules.hit = hit.cloned();
            (rules.seen, rules.at, rules.checks) = (Some(changes), Some(now), rules.checks + 1);
        }
        rules.hit.clone()
    }

    fn rule_due(&self) -> Option<Instant> {
        let rules = lock(&self.rules);
        let stale = !rules.list.is_empty() && rules.seen != Some(self.changes());
        rules.at.filter(|_| stale)?.checked_add(RULE_EVERY)
    }

    /// The records the program reported with an `id`, least recently updated first.
    #[must_use]
    pub fn records(&self) -> Vec<Report> {
        self.watch.borrow().records.clone()
    }

    /// Records a reported state, as if the program had sent the escape.
    pub fn set_report(&self, report: Report) {
        bump(&self.watch, |s| s.apply(report));
    }

    /// Turns the silence fallback on (`Some`) or off: [`report`](Self::report) says `idle` after this
    /// long without output, until the program reports a state.
    pub fn set_idle_after(&self, after: Option<Duration>) {
        bump(&self.watch, |s| s.idle_after = after);
    }

    /// Waits until `done` holds, the program exits or `deadline` passes, checking `done` again on every
    /// output chunk and report and when the idle fallback or a screen rule check comes due (no polling).
    /// Needs a tokio runtime with time.
    ///
    /// # Errors
    /// [`Error::Io`] when the exit status cannot be read.
    pub async fn wait_until(
        &self,
        mut done: impl FnMut(&Self) -> bool,
        deadline: Option<Instant>,
    ) -> Result<Wake, Error> {
        let mut changed = self.watch.subscribe();
        loop {
            changed.mark_unchanged();
            if done(self) {
                return Ok(Wake::Matched);
            }
            if self.try_wait()?.is_some() {
                return Ok(Wake::Exited);
            }
            let now = crate::clock::now();
            if deadline.is_some_and(|d| now >= d) {
                return Ok(Wake::Timeout);
            }
            let idle = changed.borrow().idle_at().filter(|at| *at > now);
            let due = self.rule_due().filter(|at| *at > now);
            let wake = [deadline, idle, due].into_iter().flatten().min();
            let sleep = async {
                match wake {
                    Some(at) => tokio::time::sleep_until(at.into()).await,
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                _ = changed.changed() => {}
                () = sleep => {}
            }
        }
    }

    /// The program's raw output from now on, chunk by chunk, each chunk shared by all subscribers; the
    /// channel closes when the PTY does, or, for a receiver 256 chunks behind, after those chunks.
    #[must_use]
    pub fn subscribe(&self) -> mpsc::Receiver<Arc<[u8]>> {
        let (tx, rx) = mpsc::channel(OUTPUT_BEHIND);
        if let Some(subs) = lock(&self.outputs).as_mut() {
            subs.push(tx);
        }
        rx
    }

    /// Writes raw bytes to the program's input, waiting at most a second for the program to take them.
    ///
    /// # Errors
    /// [`Error::Stuck`] when the program has not taken them in time (they stay queued); [`Error::Io`] when closed.
    pub fn write(&self, bytes: &[u8]) -> Result<(), Error> {
        let (done, wrote) = sync_channel(1);
        self.input.send((bytes.to_vec(), done)).ok();
        match wrote.recv_timeout(WRITE_TIMEOUT) {
            Ok(wrote) => Ok(wrote?),
            Err(RecvTimeoutError::Disconnected) => Err(Error::Io(BrokenPipe.into())),
            Err(RecvTimeoutError::Timeout) => {
                let mut late = None;
                bump(&self.watch, |s| {
                    late = wrote.try_recv().ok();
                    s.stuck |= late.is_none();
                });
                Ok(late.ok_or(Error::Stuck)??)
            }
        }
    }

    fn open_pty(&self) -> Result<Arc<File>, Error> {
        (self.pty.upgrade()).ok_or_else(|| Error::Io(std::io::ErrorKind::BrokenPipe.into()))
    }

    /// Sends one key by name (`enter`, `ctrl+c`, `up`, …); see [`keys::encode`].
    ///
    /// # Errors
    /// [`Error::UnknownKey`] for a name not in the table; [`Error::Io`] when the PTY is closed.
    pub fn press(&self, key: &str) -> Result<(), Error> {
        self.write(&keys::encode(key)?)
    }

    /// Resizes the PTY (the program gets SIGWINCH) and the screen.
    ///
    /// # Errors
    /// [`Error::Pty`] when the PTY rejects the size; [`Error::Io`] when it is closed.
    pub fn resize(&self, size: Size) -> Result<(), Error> {
        let mut screen = lock(&self.screen);
        let ws = Winsize {
            ws_row: size.rows.max(1),
            ws_col: size.cols.max(1),
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        tcsetwinsize(&*self.open_pty()?, ws).map_err(std::io::Error::from)?;
        screen.resize(size);
        bump(&self.watch, |_| {});
        Ok(())
    }

    /// The program's exit status, if it has exited, as the reaper recorded it (never blocks).
    ///
    /// # Errors
    /// [`Error::Io`] when the status could not be read.
    pub fn try_wait(&self) -> Result<Option<ExitStatus>, Error> {
        let exit = self.watch.borrow().exit;
        Ok(exit.transpose().map_err(std::io::Error::from)?)
    }

    /// The live processes of the terminal's tree and their summed RSS and CPU time.
    ///
    /// # Errors
    /// [`Error::Io`] when the process table cannot be read.
    pub fn usage(&self) -> Result<Usage, Error> {
        let procs = proctree::system().list()?;
        Ok(proctree::usage(&proctree::tree(
            &procs,
            self.pid,
            &std::collections::BTreeSet::new(),
        )))
    }

    /// Ends the whole process tree: SIGTERM, then SIGKILL after `grace`; returns the program's exit status and drops its `working` and `blocked` reports.
    ///
    /// # Errors
    /// [`Error::Io`] when the exit status cannot be read.
    pub fn kill(&self, grace: Duration) -> Result<ExitStatus, Error> {
        proctree::kill_tree(&proctree::system(), self.pid, grace, || {});
        loop {
            if let Some(status) = self.try_wait()? {
                return Ok(status);
            }
            crate::clock::sleep(REAP);
        }
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        if !self.released.load(Ordering::Relaxed)
            && matches!(self.try_wait(), Ok(None))
            && let Some(group) = Pid::from_raw(self.pid)
        {
            kill_process_group(group, Signal::KILL).ok();
        }
    }
}

fn runtime() -> Result<Handle, Error> {
    static RUNTIME: OnceLock<Option<Runtime>> = OnceLock::new();
    let one = || {
        Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
    };
    let own = || {
        RUNTIME
            .get_or_init(|| one().ok())
            .as_ref()
            .map(Runtime::handle)
    };
    let pooled = Handle::try_current()
        .ok()
        .filter(|h| h.runtime_flavor() == MultiThread);
    let runtime = pooled.or_else(|| own().cloned());
    runtime.ok_or_else(|| Error::Io(std::io::Error::other("no runtime")))
}

fn wait_pid(pid: Pid, options: WaitOptions) -> std::io::Result<Option<ExitStatus>> {
    let status = waitpid(Some(pid), options)?;
    Ok(status.map(|(_, s)| ExitStatus::from_raw(s.as_raw())))
}

async fn reap(pid: Pid, watch: Arc<watch::Sender<Status>>) {
    let mut exits = signal(SignalKind::child()).ok();
    let mut exit = wait_pid(pid, WaitOptions::NOHANG);
    while let (Ok(None), Some(signals)) = (&exit, &mut exits) {
        exits = signals.recv().await.and(exits.take());
        exit = wait_pid(pid, WaitOptions::NOHANG);
    }
    let exit = match exit {
        Ok(None) => tokio::task::spawn_blocking(move || wait_pid(pid, WaitOptions::empty()))
            .await
            .unwrap_or_else(|e| Err(e.into())),
        done => done,
    };
    let exit = exit.and_then(|s| s.ok_or(BrokenPipe.into()));
    bump(&watch, |s| {
        s.exit = Some(exit.map_err(|e| e.kind()));
        s.end(true);
    });
}

async fn write_loop(
    pty: Weak<File>,
    mut inputs: mpsc::UnboundedReceiver<Input>,
    watch: Arc<watch::Sender<Status>>,
) {
    while let Some((bytes, done)) = inputs.recv().await {
        let wrote = match pty.upgrade() {
            Some(pty) => write_all(&pty, &bytes).await,
            None => Err(BrokenPipe.into()),
        };
        done.send(wrote).ok();
        if watch.borrow().stuck {
            bump(&watch, |s| s.stuck = false);
        }
    }
}

async fn write_all(pty: &File, bytes: &[u8]) -> std::io::Result<()> {
    let fd = OwnedFd::from(pty.try_clone()?);
    rustix::io::ioctl_fionbio(&fd, true)?;
    let tx = tokio::net::unix::pipe::Sender::from_owned_fd_unchecked(fd);
    let wrote = async { tx?.write_all(bytes).await }.await;
    rustix::io::ioctl_fionbio(pty, false)?;
    wrote
}

const DEPTH: usize = 256;

fn stage<T: Send + 'static, R: Send + 'static>(
    run: impl FnOnce(Receiver<T>) -> R + Send + 'static,
) -> (SyncSender<T>, std::thread::JoinHandle<R>) {
    let (tx, rx) = std::sync::mpsc::sync_channel(DEPTH);
    (tx, std::thread::spawn(move || run(rx)))
}

fn batches(rx: &Receiver<Vec<u8>>) -> impl Iterator<Item = Vec<u8>> + '_ {
    rx.iter().map(|mut chunk| {
        for more in rx.try_iter().take(DEPTH) {
            chunk.extend(more);
        }
        chunk
    })
}

fn read_loop(pty: &File, stop: &UnixStream, chunks: &SyncSender<Vec<u8>>) -> bool {
    let mut buf = vec![0u8; 1 << 16];
    let (quiet, mut until) = (Timespec::try_from(QUIET).ok(), None);
    loop {
        let mut fds = [
            PollFd::new(pty, PollFlags::IN),
            PollFd::new(stop, PollFlags::IN),
        ];
        let polled = poll(&mut fds, until.and(quiet.as_ref()));
        let [out, asked] = fds.map(|fd| !fd.revents().is_empty());
        let now = crate::clock::now();
        if asked && until.is_none() {
            (&*stop).read_exact(&mut [0]).ok();
            until = now.checked_add(DETACH_WAIT);
        }
        if until.is_some_and(|at| now >= at || polled.is_ok_and(|n| n == 0)) {
            return true;
        }
        if !out {
            continue;
        }
        match (&*pty).read(&mut buf) {
            Ok(0) => return false,
            Ok(n) => {
                if chunks
                    .send(buf.get(..n).unwrap_or_default().to_vec())
                    .is_err()
                {
                    return false;
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
                ) => {}
            Err(_) => return false,
        }
    }
}

fn feed_loop(
    chunks: &Receiver<Vec<u8>>,
    pty: &mpsc::UnboundedSender<Input>,
    screen: &Mutex<Screen>,
    outputs: &Outputs,
    watch: &watch::Sender<Status>,
    mut log: Option<SyncSender<Vec<u8>>>,
) {
    for chunk in batches(chunks) {
        let (relayed, replies) = {
            let mut screen = lock(screen);
            let relayed = screen.relay(&chunk);
            let reports = screen.take_reports();
            bump(watch, |s| {
                s.last_output = crate::clock::now();
                for r in reports {
                    s.apply(r);
                }
            });
            (relayed, screen.take_replies())
        };
        if let Some(subs) = lock(outputs).as_mut().filter(|s| !s.is_empty()) {
            let shared: Arc<[u8]> = relayed.as_slice().into();
            subs.retain(|tx| tx.try_send(Arc::clone(&shared)).is_ok());
        }
        if log.as_ref().is_some_and(|l| l.send(relayed).is_err()) {
            log = None;
        }
        if !replies.is_empty() && pty.send((replies, sync_channel(1).0)).is_err() {
            return;
        }
    }
}
