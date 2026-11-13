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
    program: &str,
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
/// to one byte past `limit`.
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
    let group = child
        .id()
        .and_then(|id| Pid::from_raw(i32::try_from(id).ok()?));
    let end = move || group.map(|g| kill_process_group(g, Signal::KILL));
    let (input, out, err) = (child.stdin.take(), child.stdout.take(), child.stderr.take());
    let feed = async move {
        if let Some(mut pipe) = input {
            pipe.write_all(&stdin).await.ok();
        }
    };
    let ran = async {
        let status = child.wait().await;
        end();
        status
    };
    let all = async { tokio::join!(feed, capture(out, limit), capture(err, limit), ran) };
    let done = tokio::time::timeout(timeout, all).await;
    end();
    let ((), stdout, stderr, status) = done.map_err(|_| io::ErrorKind::TimedOut)?;
    Ok((status?, stdout?, stderr?))
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
) -> pty_process::Result<Child>
where
    S: AsRef<OsStr>,
    K: AsRef<OsStr>,
    V: AsRef<OsStr>,
{
    pty_process::blocking::Command::new(program)
        .args(args)
        .current_dir(cwd)
        .env("TERM", "xterm-256color")
        .envs(env)
        .spawn(pts)
}

/// This process's id.
#[must_use]
pub fn pid() -> u32 {
    std::process::id()
}
