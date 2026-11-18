//! Serve's handover to a new build in place: the check, the drain, the fd queue and the exec.

use std::ffi::OsString;
use std::io::{self, IoSlice, IoSliceMut};
use std::mem::MaybeUninit;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};
use std::time::Duration;

use rustix::io::{FdFlags, fcntl_setfd};
use rustix::net::{RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, SendAncillaryBuffer};
use rustix::net::{SendAncillaryMessage, SendFlags, recv, recvmsg, sendmsg};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use shed_core::{Detached, Terminal, runner};
use shed_proto::{BUILD, ErrorCode, Handover, HandoverCheckParams, HandoverParams, Method};
use shed_proto::{OLDEST_PROTOCOL, Request, RpcError};
use tokio::sync::mpsc::UnboundedSender;

use crate::{Error, Paths, Shared, lock};

/// The format of the fd queue and the state file's `handover` section.
pub const FORMAT: u32 = 1;
const BATCH: usize = 64;
const DRAIN: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(5);

static DRAINING: AtomicBool = AtomicBool::new(false);
static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);

/// How serve starts itself again, built once by the CLI before any swap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Restart {
    /// The binary serve was started from.
    pub exe: PathBuf,
    /// Its arguments for the new image: `serve … --inherit`.
    pub inherit: Vec<OsString>,
    /// Its arguments for the check: `serve … --preflight`.
    pub preflight: Vec<OsString>,
}

#[derive(Serialize, Deserialize)]
struct Preflight {
    build: String,
    oldest: u32,
    inherits: Vec<u32>,
}

pub(crate) struct Control {
    pub(crate) again: Restart,
    pub(crate) asked: UnboundedSender<()>,
}

pub(crate) struct Inherited {
    pub(crate) listener: UnixListener,
    pub(crate) masters: Vec<OwnedFd>,
    pub(crate) early: Vec<UnixStream>,
}

/// The JSON line `shed serve --preflight` prints once the state file parses: this build, the oldest
/// protocol it serves and the handover formats it inherits.
///
/// # Errors
/// [`Error::Io`] or [`Error::Json`] when the state file cannot be read.
pub fn preflight(paths: &Paths) -> Result<String, Error> {
    crate::registry::Registry::readable(&paths.state)?;
    let (build, oldest, inherits) = (BUILD.to_owned(), OLDEST_PROTOCOL, vec![FORMAT]);
    Ok(serde_json::to_string(&Preflight {
        build,
        oldest,
        inherits,
    })?)
}

fn refused(why: impl std::fmt::Display) -> RpcError {
    RpcError::new(ErrorCode::HandoverRefused, why)
}

pub(crate) struct Busy;

impl Drop for Busy {
    fn drop(&mut self) {
        IN_FLIGHT.fetch_sub(1, SeqCst);
    }
}

pub(crate) async fn enter(line: &str) -> Option<Busy> {
    let method = serde_json::from_str::<Value>(line).ok();
    if method.as_ref().and_then(|v| v.get("method")?.as_str()) == Some(Method::Wait.name()) {
        return None;
    }
    loop {
        IN_FLIGHT.fetch_add(1, SeqCst);
        if !DRAINING.load(SeqCst) {
            return Some(Busy);
        }
        drop(Busy);
        tokio::time::sleep(POLL).await;
    }
}

async fn drain() -> bool {
    DRAINING.store(true, SeqCst);
    let until = shed_core::clock::now().checked_add(DRAIN);
    while IN_FLIGHT.load(SeqCst) > 0 {
        if until.is_none_or(|at| shed_core::clock::now() >= at) {
            return false;
        }
        tokio::time::sleep(POLL).await;
    }
    true
}

pub(crate) async fn answer(
    registry: &Shared,
    method: Method,
    request: &Request,
) -> Result<Value, RpcError> {
    let control = lock(registry).control.clone();
    let control = control.ok_or_else(|| refused("not a server"))?;
    let p: HandoverCheckParams = if method == Method::Handover {
        let build = crate::params::<HandoverParams>(request)?.build;
        let candidate = control.again.exe.clone();
        HandoverCheckParams { build, candidate }
    } else {
        crate::params(request)?
    };
    let (program, args) = (p.candidate.clone(), control.again.preflight.clone());
    let ran = crate::blocking(move || runner::output(program, args, &[])).await?;
    let ran = ran.map_err(|e| refused(format!("preflight: {e}")))?;
    let said = String::from_utf8_lossy(&ran.stdout);
    let found = (said.lines().last()).and_then(|l| serde_json::from_str::<Preflight>(l).ok());
    let Some(found) = found.filter(|_| ran.status.success()) else {
        let err = String::from_utf8_lossy(&ran.stderr);
        return Err(refused(format!(
            "preflight: {}: {}",
            ran.status,
            err.trim()
        )));
    };
    let (to, oldest) = (found.build, found.oldest);
    if to != p.build {
        return Err(refused(format!(
            "the binary is build {to}, not {}",
            p.build
        )));
    }
    if !found.inherits.contains(&FORMAT) {
        let formats = found.inherits;
        return Err(refused(format!(
            "the binary inherits {formats:?}, not {FORMAT}"
        )));
    }
    let terminals = {
        let reg = lock(registry);
        if let Some((name, speaks)) = reg.bridges().find(|(_, s)| *s < oldest) {
            let why =
                format!("ACP terminal {name} speaks protocol {speaks}, the binary {oldest} up");
            return Err(refused(why));
        }
        reg.running().len()
    };
    if method == Method::Handover {
        control.asked.send(()).ok();
    }
    let from = BUILD.to_owned();
    crate::json(&Handover {
        from,
        to,
        terminals,
    })
}

pub(crate) async fn run(
    registry: &Shared,
    listener: &tokio::net::UnixListener,
) -> (String, Vec<(String, Arc<Terminal>)>) {
    if !drain().await {
        DRAINING.store(false, SeqCst);
        return (
            "busy: a request outlived the 10 s drain".to_owned(),
            Vec::new(),
        );
    }
    let (frozen, saved, again) = lock(registry).freeze();
    let stdin = || listener.as_fd().try_clone_to_owned();
    let why = match saved.and_then(|()| Ok((queue(listener, &frozen)?, stdin()?))) {
        Ok((_queue, stdin)) => match again {
            Some(c) => {
                let failed = runner::replace(&c.again.exe, &c.again.inherit, stdin);
                format!("exec: {failed}; restart serve when quiet, its signals may be reset")
            }
            None => "not a server".to_owned(),
        },
        Err(e) => format!("queue: {e}"),
    };
    let back = lock(registry).thaw(frozen);
    DRAINING.store(false, SeqCst);
    (why, back)
}

fn queue(
    listener: &tokio::net::UnixListener,
    frozen: &[(String, Detached)],
) -> io::Result<UnixStream> {
    let at = listener.local_addr()?;
    let queue = UnixStream::connect(at.as_pathname().ok_or(io::ErrorKind::AddrNotAvailable)?)?;
    queue.set_nonblocking(true)?;
    let fds: Vec<_> = frozen.iter().map(|(_, d)| d.master.as_fd()).collect();
    let mut batches = fds.chunks(BATCH).peekable();
    loop {
        let batch = batches.next().unwrap_or_default();
        let more = batches.peek().is_some();
        hand_over(&queue, batch, if more { b"+" } else { b"." })?;
        if !more {
            return Ok(queue);
        }
    }
}

pub(crate) fn take(stdin: impl AsFd) -> Option<Inherited> {
    let listener = UnixListener::from(stdin.as_fd().try_clone_to_owned().ok()?);
    listener.set_nonblocking(true).ok()?;
    let mut early = Vec::new();
    let queue = loop {
        let (conn, _) = listener.accept().ok()?;
        let mut first = [0];
        let peeked = recv(&conn, &mut first, RecvFlags::PEEK | RecvFlags::DONTWAIT);
        if peeked.is_ok() && matches!(&first, b"+" | b".") {
            break conn;
        }
        conn.set_nonblocking(true).ok()?;
        early.push(conn);
    };
    let mut masters = Vec::new();
    loop {
        let (more, said) = take_over(&queue, 1).ok()?;
        masters.extend(more);
        if said != b"+" {
            break;
        }
    }
    Some(Inherited {
        listener,
        masters,
        early,
    })
}

pub(crate) fn hand_over(to: impl AsFd, fds: &[BorrowedFd<'_>], said: &[u8]) -> io::Result<()> {
    let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(BATCH))];
    let mut ancillary = SendAncillaryBuffer::new(&mut space);
    if !fds.is_empty() {
        ancillary.push(SendAncillaryMessage::ScmRights(fds));
    }
    sendmsg(
        to,
        &[IoSlice::new(said)],
        &mut ancillary,
        SendFlags::empty(),
    )?;
    Ok(())
}

pub(crate) fn take_over(from: impl AsFd, max: usize) -> io::Result<(Vec<OwnedFd>, Vec<u8>)> {
    let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(BATCH))];
    let mut ancillary = RecvAncillaryBuffer::new(&mut space);
    let mut said = vec![0; max];
    let got = recvmsg(
        from,
        &mut [IoSliceMut::new(&mut said)],
        &mut ancillary,
        RecvFlags::empty(),
    )?;
    said.truncate(got.bytes);
    let mut fds = Vec::new();
    for message in ancillary.drain() {
        if let RecvAncillaryMessage::ScmRights(more) = message {
            fds.extend(more);
        }
    }
    for fd in &fds {
        fcntl_setfd(fd, FdFlags::CLOEXEC)?;
    }
    Ok((fds, said))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::Registry;
    use std::sync::Mutex;

    #[tokio::test(flavor = "multi_thread")]
    async fn a_failed_exec_leaves_every_terminal_running_in_the_same_process() {
        let dir = crate::temp_dir("handover-exec");
        let socket = dir.join("shed.sock");
        let reg = Registry::load(&dir.join("state.json"), &socket, Vec::new());
        let registry = Arc::new(Mutex::new(reg.expect("a registry")));
        let run_sh = serde_json::json!({"name": "sh", "cwd": "/", "cmd": ["sh"]});
        lock(&registry)
            .run(serde_json::from_value(run_sh).expect("params"))
            .expect("run");
        let pid = lock(&registry).live("sh").expect("running").pid();
        let listener = tokio::net::UnixListener::bind(&socket).expect("bind");
        let again = Restart {
            exe: dir.join("gone\0"),
            inherit: Vec::new(),
            preflight: Vec::new(),
        };
        let asked = tokio::sync::mpsc::unbounded_channel().0;
        lock(&registry).control = Some(Arc::new(Control { again, asked }));
        let (why, back) = run(&registry, &listener).await;
        assert!(why.starts_with("exec: "), "{why}");
        assert_eq!(back.len(), 1);
        let term = Arc::clone(lock(&registry).live("sh").expect("still running"));
        assert_eq!(term.pid(), pid);
        term.write(b"echo $((6 * 7))\r").expect("input");
        let until = shed_core::clock::now().checked_add(Duration::from_secs(10));
        let woke = term.wait_until(|t| t.screen().contains("42"), until).await;
        assert_eq!(woke.expect("wait"), shed_core::Wake::Matched);
        assert!(!DRAINING.load(SeqCst), "requests go on");
    }
}
