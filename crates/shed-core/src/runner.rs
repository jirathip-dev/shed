//! The one module that starts processes; every other module asks it (CI bans `Command::new` elsewhere).

use std::ffi::OsStr;
use std::io;
use std::os::fd::OwnedFd;
use std::os::unix::process::CommandExt as _;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::Duration;

use rustix::process::{Pid, Signal, kill_process_group};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWriteExt as _};

pub use std::process::{Child, ExitCode, ExitStatus};

/// Runs `program` with `args` and `envs` to its end, stdout and stderr captured, stdin empty.
///
/// # Errors
/// When `program` cannot start.
pub fn output<A: AsRef<OsStr>>(
    program: impl AsRef<OsStr>,
    args: impl IntoIterator<Item = A>,
    envs: &[(&str, &str)],
) -> io::Result<Output> {
    Command::new(program)
        .args(args)
        .envs(envs.iter().copied())
        .output()
}

/// Starts `program` with `args` in a process group of its own: stdin and stderr closed, stdout piped.
///
/// # Errors
/// When `program` cannot start.
pub fn detached<A: AsRef<OsStr>>(
    program: &Path,
    args: impl IntoIterator<Item = A>,
) -> io::Result<Child> {
    Command::new(program)
        .args(args)
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
}

/// Starts `program` with `args`, its stdin and stdout both `fd`.
///
/// # Errors
/// When `fd` cannot be duplicated or `program` cannot start.
pub fn over<A: AsRef<OsStr>>(
    program: &str,
    args: impl IntoIterator<Item = A>,
    fd: OwnedFd,
) -> io::Result<Child> {
    Command::new(program)
        .args(args)
        .stdin(Stdio::from(fd.try_clone()?))
        .stdout(Stdio::from(fd))
        .spawn()
}

/// Starts `program` with `args` in `cwd` without the `unset` variables, stdin, stdout and stderr piped.
///
/// # Errors
/// When `program` cannot start.
pub fn piped<A: AsRef<OsStr>>(
    program: &str,
    args: impl IntoIterator<Item = A>,
    cwd: &Path,
    unset: &[&str],
) -> io::Result<tokio::process::Child> {
    let mut command = tokio::process::Command::new(program);
    for name in unset {
        command.env_remove(name);
    }
    command
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
}

/// Runs `cmd` in `cwd` without a terminal, in a process group of its own that is ended when it
/// returns or after `timeout`, `stdin` fed to it; its status, and stdout and stderr each read up
/// to one byte past `limit`. Dropping the future ends the group too.
///
/// # Errors
/// When `cmd` is empty or cannot start, or `TimedOut` after `timeout`.
pub async fn exec(
    cmd: &[String],
    cwd: &Path,
    stdin: Vec<u8>,
    timeout: Duration,
    limit: u64,
) -> io::Result<(ExitStatus, Vec<u8>, Vec<u8>)> {
    let (program, args) = cmd.split_first().ok_or(io::ErrorKind::InvalidInput)?;
    let mut child = tokio::process::Command::new(program)
        .args(args)
        .current_dir(cwd)
        .process_group(0)
        .kill_on_drop(true)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let group = Group::of(&child);
    let (input, out, err) = (child.stdin.take(), child.stdout.take(), child.stderr.take());
    let feed = async move {
        if let Some(mut pipe) = input {
            pipe.write_all(&stdin).await.ok();
        }
    };
    let ran = async {
        let status = child.wait().await;
        group.end();
        status
    };
    let all = async { tokio::join!(feed, capture(out, limit), capture(err, limit), ran) };
    let done = tokio::time::timeout(timeout, all).await;
    let ((), stdout, stderr, status) = done.map_err(|_| io::ErrorKind::TimedOut)?;
    Ok((status?, stdout?, stderr?))
}

struct Group(Option<Pid>);

impl Group {
    fn of(child: &tokio::process::Child) -> Self {
        Self(
            child
                .id()
                .and_then(|id| Pid::from_raw(i32::try_from(id).ok()?)),
        )
    }

    fn end(&self) {
        if let Some(group) = self.0 {
            kill_process_group(group, Signal::KILL).ok();
        }
    }
}

impl Drop for Group {
    fn drop(&mut self) {
        self.end();
    }
}

/// A process started by [`held`]: dropping it ends its whole group.
pub struct Held {
    _child: tokio::process::Child,
    _group: Group,
}

/// Starts `cmd` in a process group of its own, its stdin and stdout piped and its stderr ours, for a
/// conversation of lines; the group is ended when the [`Held`] is dropped.
///
/// # Errors
/// When `cmd` is empty or cannot start.
pub fn held(
    cmd: &[String],
) -> io::Result<(
    Held,
    tokio::process::ChildStdin,
    tokio::process::ChildStdout,
)> {
    let (program, args) = cmd.split_first().ok_or(io::ErrorKind::InvalidInput)?;
    let mut child = tokio::process::Command::new(program)
        .args(args)
        .process_group(0)
        .kill_on_drop(true)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    let group = Group::of(&child);
    let pipes = child.stdin.take().zip(child.stdout.take());
    let (stdin, stdout) = pipes.ok_or(io::ErrorKind::BrokenPipe)?;
    let held = Held {
        _child: child,
        _group: group,
    };
    Ok((held, stdin, stdout))
}

async fn capture(pipe: Option<impl AsyncRead + Unpin>, limit: u64) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    if let Some(pipe) = pipe {
        let mut pipe = pipe.take(limit.saturating_add(1));
        pipe.read_to_end(&mut bytes).await?;
    }
    Ok(bytes)
}

pub(crate) fn pty<S, K, V>(
    program: &S,
    args: &[S],
    cwd: &Path,
    env: impl IntoIterator<Item = (K, V)>,
    pts: pty_process::blocking::Pts,
    stderr: Option<OwnedFd>,
) -> pty_process::Result<Child>
where
    S: AsRef<OsStr>,
    K: AsRef<OsStr>,
    V: AsRef<OsStr>,
{
    let command = pty_process::blocking::Command::new(program)
        .args(args)
        .current_dir(cwd)
        .env("TERM", "xterm-256color")
        .envs(env);
    match stderr {
        Some(fd) => command.stderr(fd).spawn(pts),
        None => command.spawn(pts),
    }
}

/// Replaces this process with `program` and `args`, its stdin `stdin`; returns only when it could not.
#[must_use]
pub fn replace<A: AsRef<OsStr>>(program: &Path, args: &[A], stdin: OwnedFd) -> io::Error {
    Command::new(program).args(args).stdin(stdin).exec()
}

/// This process's id.
#[must_use]
pub fn pid() -> u32 {
    std::process::id()
}
