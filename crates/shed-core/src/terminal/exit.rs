//! The program's end: its exit status, its process tree's usage, and killing it.

use std::io::ErrorKind::BrokenPipe;
use std::os::unix::process::ExitStatusExt as _;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use rustix::process::{Pid, WaitOptions, waitpid};
use tokio::runtime::{Builder, Handle, Runtime, RuntimeFlavor::MultiThread};
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::watch;

use super::Terminal;
use super::status::{Status, bump};
use crate::Error;
use crate::proctree::{self, ProcessTable as _, Usage};
use crate::runner::ExitStatus;

const REAP: Duration = Duration::from_millis(20);

impl Terminal {
    /// The program's exit status, if it has exited, as the reaper recorded it (never blocks).
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

pub(super) fn runtime() -> Result<Handle, Error> {
    static RUNTIME: OnceLock<Option<Runtime>> = OnceLock::new();
    let one = || {
        Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
    };
    let own = || {
        RUNTIME
            .get_or_init(|| one().ok())
            .as_ref()
            .map(Runtime::handle)
    };
    let pooled = Handle::try_current()
        .ok()
        .filter(|h| h.runtime_flavor() == MultiThread);
    let runtime = pooled.or_else(|| own().cloned());
    runtime.ok_or_else(|| Error::Io(std::io::Error::other("no runtime")))
}

fn wait_pid(pid: Pid, options: WaitOptions) -> std::io::Result<Option<ExitStatus>> {
    let status = waitpid(Some(pid), options)?;
    Ok(status.map(|(_, s)| ExitStatus::from_raw(s.as_raw())))
}

pub(super) async fn reap(pid: Pid, watch: Arc<watch::Sender<Status>>) {
    let mut exits = signal(SignalKind::child()).ok();
    let mut exit = wait_pid(pid, WaitOptions::NOHANG);
    while let (Ok(None), Some(signals)) = (&exit, &mut exits) {
        exits = signals.recv().await.and(exits.take());
        exit = wait_pid(pid, WaitOptions::NOHANG);
    }
    let exit = match exit {
        Ok(None) => tokio::task::spawn_blocking(move || wait_pid(pid, WaitOptions::empty()))
            .await
            .unwrap_or_else(|e| Err(e.into())),
        done => done,
    };
    let exit = exit.and_then(|s| s.ok_or(BrokenPipe.into()));
    bump(&watch, |s| {
        s.exit = Some(exit.map_err(|e| e.kind()));
        s.end(true);
    });
}
