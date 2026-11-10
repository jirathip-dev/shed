//! The one module that starts processes; every other module asks it (CI bans `Command::new` elsewhere).

use std::ffi::OsStr;
use std::io;
use std::os::fd::OwnedFd;
use std::os::unix::process::CommandExt as _;
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};

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

/// Starts `program` with `args` in `cwd` without the `unset` variable, stdin and stdout piped.
///
/// # Errors
/// When `program` cannot start.
pub fn piped<A: AsRef<OsStr>>(
    program: &str,
    args: impl IntoIterator<Item = A>,
    cwd: &Path,
    unset: &str,
) -> io::Result<tokio::process::Child> {
    tokio::process::Command::new(program)
        .args(args)
        .current_dir(cwd)
        .env_remove(unset)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
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
