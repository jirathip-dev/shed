//! shed's per-user server: the socket, JSON-RPC, the events stream and machines over SSH.
//!
//! [`serve()`] owns the terminals and answers newline-delimited JSON-RPC 2.0 ([`shed_proto`]) on a
//! Unix socket, one tokio task per connection. `shed_client` is the other end.
#![forbid(unsafe_code)]

mod accounts;
pub mod agents;
mod call;
mod code;
pub mod config;
mod dispatch;
pub mod drivers;
mod events;
mod files;
mod flows;
pub mod handover;
pub mod hooks;
mod input;
pub mod lane;
pub mod limit;
mod links;
pub mod machines;
pub mod providers;
mod proxy;
mod registry;
pub mod relay;
mod screen;
mod script;
pub mod secrets;
pub mod serve;
mod stats;

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use registry::Registry;
pub use serve::serve;
pub use shed_core::{clock, proctree, runner};
pub use shed_proto as proto;
pub use shed_search as search;
use tokio::io::AsyncWriteExt as _;

/// Errors from the server and the client.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// `HOME` is not set.
    #[error("HOME is not set")]
    NoHome,
    /// `XDG_RUNTIME_DIR` is not set (Linux), so there is no default socket path.
    #[error("XDG_RUNTIME_DIR is not set; pass a directory with --dir")]
    NoRuntimeDir,
    /// A server already answers on the socket.
    #[error("a shed server already runs on {0}")]
    AlreadyRunning(PathBuf),
    /// A call to a server failed.
    #[error(transparent)]
    Client(#[from] shed_client::Error),
    /// The ACP agent could not start or failed the handshake.
    #[error("acp: {0}")]
    Acp(String),
    /// Installing, listing or removing an ACP agent failed.
    #[error("agents: {0}")]
    Agents(String),
    /// A JSON error.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// An I/O error.
    #[error(transparent)]
    Io(#[from] io::Error),
    /// The config file or a `machine add` argument is not valid.
    #[error("{0}")]
    Config(String),
    /// No machine has that name.
    #[error("no machine {0}; see `shed machine ls`")]
    NoMachine(String),
    /// The shed on a machine speaks another protocol.
    #[error(
        "SHED_VERSION: shed {here} here, {there} on {machine}; put both on main's latest release: \
         `shed self-update` on each (docs/self-update.md)"
    )]
    MachineVersion {
        /// The machine.
        machine: String,
        /// This shed's build.
        here: String,
        /// The build of the server on the machine.
        there: String,
    },
    /// `ssh` or the shed on a machine gave no answer.
    #[error(
        "no answer from shed on {machine}; check that `ssh {target} shed hello` works with key login \
         and that a server runs there (`shed serve --detach`)"
    )]
    Unreachable {
        /// The machine.
        machine: String,
        /// Its ssh target.
        target: String,
    },
}

/// Where a server listens and keeps its state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    /// The Unix socket.
    pub socket: PathBuf,
    /// The state file.
    pub state: PathBuf,
    /// The config file (machines).
    pub config: PathBuf,
}

impl Paths {
    /// The user's provider records, `providers/` beside the config file.
    #[must_use]
    pub fn providers(&self) -> PathBuf {
        self.config.with_file_name("providers")
    }

    /// `shed.sock` and `state.json`, both in `dir`.
    #[must_use]
    pub fn in_dir(dir: &Path) -> Self {
        Self {
            socket: dir.join("shed.sock"),
            state: dir.join("state.json"),
            config: dir.join("config.toml"),
        }
    }

    /// The per-user defaults. macOS: `~/Library/Caches/shed/shed.sock`, and `state.json` and
    /// `config.toml` in `~/Library/Application Support/shed/`. Elsewhere: `$XDG_RUNTIME_DIR/shed.sock`,
    /// `$XDG_STATE_HOME/shed/state.json` (default `~/.local/state`) and
    /// `$XDG_CONFIG_HOME/shed/config.toml` (default `~/.config`).
    ///
    /// # Errors
    /// [`Error::NoHome`] or [`Error::NoRuntimeDir`] when the variables they need are unset.
    pub fn user() -> Result<Self, Error> {
        let var = |k| config::var_os(k).filter(|v: &OsString| !v.is_empty());
        let home = var("HOME").map(PathBuf::from).ok_or(Error::NoHome);
        if cfg!(target_os = "macos") {
            let lib = home?.join("Library");
            return Ok(Self {
                socket: lib.join("Caches/shed/shed.sock"),
                state: lib.join("Application Support/shed/state.json"),
                config: lib.join("Application Support/shed/config.toml"),
            });
        }
        let runtime = var("XDG_RUNTIME_DIR").ok_or(Error::NoRuntimeDir)?;
        let home = home?;
        let xdg = |k, default| var(k).map_or_else(|| home.join(default), PathBuf::from);
        Ok(Self {
            socket: PathBuf::from(runtime).join("shed.sock"),
            state: xdg("XDG_STATE_HOME", ".local/state").join("shed/state.json"),
            config: xdg("XDG_CONFIG_HOME", ".config").join("shed/config.toml"),
        })
    }
}

type Shared = Arc<Mutex<Registry>>;

pub(crate) const HEARTBEAT: Duration = Duration::from_secs(15);
pub(crate) const WRITE_TIMEOUT: Duration = HEARTBEAT.saturating_mul(2);

async fn send(out: &mut tokio::net::unix::OwnedWriteHalf, bytes: &[u8]) -> bool {
    matches!(
        tokio::time::timeout(WRITE_TIMEOUT, out.write_all(bytes)).await,
        Ok(Ok(()))
    )
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
fn temp_dir(name: &str) -> PathBuf {
    struct Made(std::cell::RefCell<Vec<PathBuf>>);
    impl Drop for Made {
        fn drop(&mut self) {
            for dir in self.0.borrow().iter() {
                std::fs::remove_dir_all(dir).ok();
            }
        }
    }
    thread_local!(static MADE: Made = const { Made(std::cell::RefCell::new(Vec::new())) });
    let dir = std::env::temp_dir().join(format!("shed-{name}-{}", runner::pid()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).expect("temp dir");
    MADE.with(|m| m.0.borrow_mut().push(dir.clone()));
    dir
}
