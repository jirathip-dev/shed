//! A terminal: one program on a PTY, its screen, and its process tree.

use crate::runner::{Child, ExitStatus};
use std::ffi::OsStr;
use std::io::{Read as _, Write as _};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use rustix::process::{Pid, Signal, kill_process_group};
use tokio::sync::{mpsc, watch};

use crate::history::{History, Log};
use crate::proctree::{self, ProcessTable as _, Usage};
use crate::screen::{Report, Screen, Size, Snapshot};
use crate::{Error, keys};

const REAP: Duration = Duration::from_millis(20);

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
    closed: bool,
    ended: bool,
    last_output: Instant,
    report: Option<Report>,
    records: Vec<Report>,
    idle_after: Option<Duration>,
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
    pty: Arc<pty_process::blocking::Pty>,
    child: Mutex<Child>,
    screen: Arc<Mutex<Screen>>,
    outputs: Arc<Outputs>,
    watch: Arc<watch::Sender<Status>>,
    pid: i32,
}

type Outputs = Mutex<Option<Vec<mpsc::UnboundedSender<Vec<u8>>>>>;

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
        let pty = Arc::new(pty);
        let outputs = Arc::new(Mutex::new(Some(Vec::new())));
        let watch = Arc::new(watch::Sender::new(Status {
            changes: 0,
            closed: false,
            ended: false,
            last_output: crate::clock::now(),
            report: None,
            records: Vec::new(),
            idle_after: None,
        }));
        let (reader, sink, subs) = (Arc::clone(&pty), Arc::clone(&screen), Arc::clone(&outputs));
        let watcher = Arc::clone(&watch);
        std::thread::spawn(move || {
            read_loop(&reader, &sink, &subs, &watcher, log);
            *lock(&subs) = None;
            bump(&watcher, |s| {
                s.closed = true;
                s.end(true);
            });
        });
        Ok(Self {
            pty,
            child: Mutex::new(child),
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

    /// The last `n` lines of scrollback and screen as plain text.
    #[must_use]
    pub fn history(&self, n: usize) -> String {
        lock(&self.screen).history(n)
    }

    /// The state the program last reported (`shed state`, the OSC 7501 escape or another agent's). Without one, `idle`
    /// once the program has been silent for the [`set_idle_after`](Self::set_idle_after) time.
    #[must_use]
    pub fn report(&self) -> Option<Report> {
        let status = self.watch.borrow();
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
            let (idle, closed) = {
                let status = changed.borrow();
                (status.idle_at().filter(|at| *at > now), status.closed)
            };
            let reap = closed.then(|| now + REAP);
            let wake = [deadline, idle, reap].into_iter().flatten().min();
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

    /// The program's raw output from now on, chunk by chunk; the channel closes when the PTY does.
    #[must_use]
    pub fn subscribe(&self) -> mpsc::UnboundedReceiver<Vec<u8>> {
        let (tx, rx) = mpsc::unbounded_channel();
        if let Some(subs) = lock(&self.outputs).as_mut() {
            subs.push(tx);
        }
        rx
    }

    /// Writes raw bytes to the program's input.
    ///
    /// # Errors
    /// [`Error::Io`] when the PTY is closed.
    pub fn write(&self, bytes: &[u8]) -> Result<(), Error> {
        Ok((&*self.pty).write_all(bytes)?)
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
    /// [`Error::Pty`] when the PTY rejects the size.
    pub fn resize(&self, size: Size) -> Result<(), Error> {
        let mut screen = lock(&self.screen);
        self.pty.resize(pty_size(size))?;
        screen.resize(size);
        bump(&self.watch, |_| {});
        Ok(())
    }

    /// The program's exit status, if it has exited; once it has, its `working` and `blocked` reports are dropped.
    ///
    /// # Errors
    /// [`Error::Io`] when the status cannot be read.
    pub fn try_wait(&self) -> Result<Option<ExitStatus>, Error> {
        let status = lock(&self.child).try_wait()?;
        if status.is_some() && !self.watch.borrow().ended {
            bump(&self.watch, |s| s.end(true));
        }
        Ok(status)
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

    /// Ends the whole process tree: SIGTERM, then SIGKILL after `grace`; returns the program's exit status.
    ///
    /// # Errors
    /// [`Error::Io`] when the process table or the exit status cannot be read.
    pub fn kill(&self, grace: Duration) -> Result<ExitStatus, Error> {
        let mut child = lock(&self.child);
        proctree::kill_tree(&proctree::system(), self.pid, grace, || {
            child.try_wait().ok();
        })?;
        Ok(child.wait()?)
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let child = self.child.get_mut().unwrap_or_else(PoisonError::into_inner);
        if matches!(child.try_wait(), Ok(None))
            && let Some(group) = Pid::from_raw(self.pid)
        {
            kill_process_group(group, Signal::KILL).ok();
            child.wait().ok();
        }
    }
}

fn pty_size(size: Size) -> pty_process::Size {
    pty_process::Size::new(size.rows.max(1), size.cols.max(1))
}

fn read_loop(
    pty: &pty_process::blocking::Pty,
    screen: &Mutex<Screen>,
    outputs: &Outputs,
    watch: &watch::Sender<Status>,
    mut log: Option<Log>,
) {
    let mut buf = [0u8; 8192];
    loop {
        match (&*pty).read(&mut buf) {
            Ok(0) => return,
            Ok(n) => {
                let chunk = buf.get(..n).unwrap_or_default();
                let replies = {
                    let mut screen = lock(screen);
                    screen.feed(chunk);
                    let reports = screen.take_reports();
                    bump(watch, |s| {
                        s.last_output = crate::clock::now();
                        for r in reports {
                            s.apply(r);
                        }
                    });
                    screen.take_replies()
                };
                if let Some(subs) = lock(outputs).as_mut() {
                    subs.retain(|tx| tx.send(chunk.to_vec()).is_ok());
                }
                if log.as_mut().is_some_and(|l| l.append(chunk).is_err()) {
                    log = None;
                }
                if !replies.is_empty() && (&*pty).write_all(&replies).is_err() {
                    return;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return,
        }
    }
}
