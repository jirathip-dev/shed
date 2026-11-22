//! Input and output: writes and keys to the program, its raw output to the screen, the log and subscribers.

use std::fs::File;
use std::io::{ErrorKind::BrokenPipe, Read as _};
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::termios::{Winsize, tcsetwinsize};
use tokio::io::AsyncWriteExt as _;
use tokio::sync::{mpsc, watch};

use super::status::{Status, bump};
use super::{Input, Outputs, Terminal, lock};
use crate::screen::{Screen, Size};
use crate::{Error, keys};

const WRITE_TIMEOUT: Duration = Duration::from_secs(1);
const OUTPUT_BEHIND: usize = 256;
const QUIET: Duration = Duration::from_millis(20);
const DETACH_WAIT: Duration = Duration::from_millis(100);

impl Terminal {
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
}

pub(super) async fn write_loop(
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

pub(super) fn stage<T: Send + 'static, R: Send + 'static>(
    run: impl FnOnce(Receiver<T>) -> R + Send + 'static,
) -> (SyncSender<T>, std::thread::JoinHandle<R>) {
    let (tx, rx) = std::sync::mpsc::sync_channel(DEPTH);
    (tx, std::thread::spawn(move || run(rx)))
}

pub(super) fn batches(rx: &Receiver<Vec<u8>>) -> impl Iterator<Item = Vec<u8>> + '_ {
    rx.iter().map(|mut chunk| {
        for more in rx.try_iter().take(DEPTH) {
            chunk.extend(more);
        }
        chunk
    })
}

pub(super) fn read_loop(pty: &File, stop: &UnixStream, chunks: &SyncSender<Vec<u8>>) -> bool {
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

pub(super) fn feed_loop(
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
