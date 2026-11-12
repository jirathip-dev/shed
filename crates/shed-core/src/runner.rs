//! The one module that starts processes; every other module asks it (CI bans `Command::new` elsewhere).

use std::ffi::OsStr;
use std::io;
use std::os::fd::OwnedFd;
use std::os::unix::process::CommandExt as _;
use std::path::Path;
use std::process::{Command, Output, Stdio};

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
