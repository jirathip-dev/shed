//! shed's core: terminals, the screen model, the state machine, persistence and process trees.
//!
//! No sockets, and no dependency on any other shed crate (CI checks this).
#![forbid(unsafe_code)]

pub mod history;
pub mod keys;
pub mod pattern;
pub mod proctree;
pub mod runner;
pub mod screen;
pub mod terminal;

pub use history::History;
pub use pattern::Pattern;
pub use proctree::Usage;
pub use screen::{Color, Pen, Report, Run, STATES, Screen, Size, Snapshot};
pub use terminal::{Terminal, Wake};

/// Errors from shed-core.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A terminal was asked to run an empty command.
    #[error("empty command")]
    EmptyCommand,
    /// A key name that is not in the key table.
    #[error("unknown key: {0:?}")]
    UnknownKey(String),
    /// A `wait --output` pattern that does not compile.
    #[error("bad pattern {0}")]
    Pattern(String),
    /// Opening, sizing or spawning on the PTY failed.
    #[error("pty: {0}")]
    Pty(#[from] pty_process::Error),
    /// An I/O error.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
