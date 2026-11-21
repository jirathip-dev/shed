//! A terminal: one program on a PTY, its screen, and its process tree.

pub mod exit;
pub mod io;
pub mod status;

pub use status::{Rule, Wake};

use std::ffi::OsStr;
use std::fs::File;
use std::io::{ErrorKind::BrokenPipe, Write as _};
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::os::unix::process::ExitStatusExt as _;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use rustix::process::{Pid, Signal, kill_process_group};
use rustix::termios::tcgetwinsize;
use tokio::sync::{mpsc, watch};

use crate::Error;
use crate::history::{History, Log};
use crate::runner::ExitStatus;
use crate::screen::{Screen, Size, Snapshot};
use exit::{reap, runtime};
use io::{batches, feed_loop, read_loop, stage, write_loop};
use status::{Rules, Status, bump};

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

    fn open_pty(&self) -> Result<Arc<File>, Error> {
        (self.pty.upgrade()).ok_or_else(|| Error::Io(std::io::ErrorKind::BrokenPipe.into()))
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
