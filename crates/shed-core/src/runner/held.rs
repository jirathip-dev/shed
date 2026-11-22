//! A process held in a group of its own, for a conversation of lines or a run to its end.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;
use std::process::Stdio;

use rustix::process::{Pid, Signal, kill_process_group};

/// A process in a group of its own: dropping it ends the whole group.
pub struct Held {
    /// The process, its stdin and stdout piped.
    pub child: tokio::process::Child,
    group: Option<Pid>,
}

impl Drop for Held {
    fn drop(&mut self) {
        if let Some(group) = self.group {
            kill_process_group(group, Signal::KILL).ok();
        }
    }
}

/// Starts `cmd` in `cwd` with `env` over this process's variables in a process group of its own, stdin and stdout
/// piped, stderr piped too when `capture` and else ours.
///
/// # Errors
/// When `cmd` is empty or cannot start.
pub fn held(
    cmd: &[String],
    cwd: &Path,
    env: &BTreeMap<String, String>,
    capture: bool,
) -> io::Result<Held> {
    let (program, args) = cmd.split_first().ok_or(io::ErrorKind::InvalidInput)?;
    let child = tokio::process::Command::new(program)
        .args(args)
        .current_dir(cwd)
        .envs(env)
        .process_group(0)
        .kill_on_drop(true)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(if capture {
            Stdio::piped()
        } else {
            Stdio::inherit()
        })
        .spawn()?;
    let group = child
        .id()
        .and_then(|id| Pid::from_raw(i32::try_from(id).ok()?));
    Ok(Held { child, group })
}
