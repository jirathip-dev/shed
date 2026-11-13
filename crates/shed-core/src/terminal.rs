//! A terminal: one program on a PTY, its screen, and its process tree.

use crate::runner::{Child, ExitStatus};
use std::ffi::OsStr;
use std::io::{ErrorKind::BrokenPipe, Read as _, Write as _};
use std::path::Path;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, SyncSender, sync_channel};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant};

use rustix::process::{Pid, Signal, kill_process_group};
use tokio::sync::{mpsc, watch};

use crate::history::{History, Log};
use crate::proctree::{self, ProcessTable as _, Usage};
use crate::screen::{Report, Screen, Size, Snapshot};
use crate::{Error, keys};

const REAP: Duration = Duration::from_millis(20);
const WRITE_TIMEOUT: Duration = Duration::from_secs(1);
const OUTPUT_BEHIND: usize = 256;

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
/// Its output feeds a [`Screen`] from a reader thread. Dropping the terminal SIGKILLs its process group.
pub struct Terminal {
    pty: Weak<pty_process::blocking::Pty>,
    input: Sender<Input>,
    screen: Arc<Mutex<Screen>>,
    outputs: Arc<Outputs>,
    watch: Arc<watch::Sender<Status>>,
    pid: i32,
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
        Self::spawn_with(cmd, cwd, env, size, &History::default())
    }

    /// [`spawn`](Self::spawn) keeping `history`: its caps bound the scrollback, the screen starts from
    /// the tail of its log, and the output is appended to the log.
    ///
    /// # Errors
    /// As [`spawn`](Self::spawn); [`Error::Io`] when the log cannot be read or opened.
    pub fn spawn_with<S, K, V>(
        cmd: &[S],
        cwd: &Path,
        env: impl IntoIterator<Item = (K, V)>,
        size: Size,
        history: &History,
    ) -> Result<Self, Error>
    where
        S: AsRef<OsStr>,
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        let (program, args) = cmd.split_first().ok_or(Error::EmptyCommand)?;
        let screen = Arc::new(Mutex::new(history.restore(size)?));
        let log = Log::open(history)?;
        let (pty, pts) = pty_process::blocking::open()?;
        pty.resize(pty_size(size))?;
        let child = crate::runner::pty(program, args, cwd, env, pts)?;
        let pid = i32::try_from(child.id())
            .map_err(|_| Error::Io(std::io::Error::other("pid out of range")))?;
        let reader = Arc::new(pty);
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
        std::thread::spawn(move || {
            let (pty, screen, outputs) =
                (Arc::clone(&reader), Arc::clone(&sink), Arc::clone(&subs));
            let watch = Arc::clone(&watcher);
            let (logs, logger) = stage(move |rx: Receiver<Vec<u8>>| {
                let mut log = log;
                batches(&rx).try_for_each(|bytes| log.as_mut()?.append(&bytes).ok())
            });
            let (chunks, feeder) =
                stage(move |rx| feed_loop(&rx, &pty, &screen, &outputs, &watch, Some(logs)));
            read_loop(&reader, &chunks);
            drop(chunks);
            feeder.join().ok();
            logger.join().ok();
            drop(reader);
            *lock(&subs) = None;
            bump(&watcher, |s| s.end(true));
        });
        let exits = Arc::clone(&watch);
        std::thread::spawn(move || reap(child, &exits));
        let (input, inputs) = std::sync::mpsc::channel();
        let (writer, watcher) = (Weak::clone(&pty), Arc::clone(&watch));
        std::thread::spawn(move || write_loop(&writer, &inputs, &watcher));
        Ok(Self {
            pty,
            input,
            screen,
            outputs,
            watch,
            pid,
        })
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

    /// The last `n` lines of scrollback and screen as plain text.
    #[must_use]
    pub fn history(&self, n: usize) -> String {
        lock(&self.screen).history(n)
    }

    /// The state the program last reported (`shed state`, the OSC 7501 escape or another agent's). Without one, `idle`
    /// once silent for the [`set_idle_after`](Self::set_idle_after) time; `stuck` while a [`write`](Self::write) waits.
    #[must_use]
    pub fn report(&self) -> Option<Report> {
        let status = self.watch.borrow();
        if status.stuck {
            return Some(Report {
                state: "stuck".to_owned(),
                note: Some("its input is not drained".to_owned()),
                ..Report::default()
            });
        }
        let idle = status.idle_at().is_some_and(|at| crate::clock::now() >= at);
        status.report.clone().or_else(|| {
            idle.then(|| Report {
                state: "idle".to_owned(),
                ..Report::default()
            })
        })
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
    /// output chunk and report and when the idle fallback comes due (no polling). Needs a tokio runtime
    /// with time.
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
            let wake = [deadline, idle].into_iter().flatten().min();
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

    fn open_pty(&self) -> Result<Arc<pty_process::blocking::Pty>, Error> {
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
        self.open_pty()?.resize(pty_size(size))?;
        screen.resize(size);
        bump(&self.watch, |_| {});
        Ok(())
    }

    /// The program's exit status, if it has exited, as the reaper thread recorded it (never blocks).
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
        if matches!(self.try_wait(), Ok(None))
            && let Some(group) = Pid::from_raw(self.pid)
        {
            kill_process_group(group, Signal::KILL).ok();
        }
    }
}

fn reap(mut child: Child, watch: &watch::Sender<Status>) {
    let exit = child.wait().map_err(|e| e.kind());
    bump(watch, |s| {
        s.exit = Some(exit);
        s.end(true);
    });
}

fn write_loop(
    pty: &Weak<pty_process::blocking::Pty>,
    inputs: &std::sync::mpsc::Receiver<Input>,
    watch: &watch::Sender<Status>,
) {
    for (bytes, done) in inputs {
        let wrote = pty
            .upgrade()
            .map_or_else(|| Err(BrokenPipe.into()), |pty| (&*pty).write_all(&bytes));
        done.send(wrote).ok();
        if watch.borrow().stuck {
            bump(watch, |s| s.stuck = false);
        }
    }
}

fn pty_size(size: Size) -> pty_process::Size {
    pty_process::Size::new(size.rows.max(1), size.cols.max(1))
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

fn read_loop(pty: &pty_process::blocking::Pty, chunks: &SyncSender<Vec<u8>>) {
    let mut buf = vec![0u8; 1 << 16];
    loop {
        match (&*pty).read(&mut buf) {
            Ok(0) => return,
            Ok(n) => {
                if chunks
                    .send(buf.get(..n).unwrap_or_default().to_vec())
                    .is_err()
                {
                    return;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return,
        }
    }
}

fn feed_loop(
    chunks: &Receiver<Vec<u8>>,
    pty: &pty_process::blocking::Pty,
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
        if !replies.is_empty() && (&*pty).write_all(&replies).is_err() {
            return;
        }
    }
}
