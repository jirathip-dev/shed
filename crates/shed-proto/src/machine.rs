//! The machine: `exec`, files, health and the held links.

use std::collections::BTreeMap;
use std::path::PathBuf;

#[cfg(feature = "schema")]
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Span;

/// The most bytes one `exec` or `file.read` reply carries: 8 MiB.
pub const REPLY_LIMIT: u64 = 8 << 20;

/// `exec` params.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct ExecParams {
    /// The program and its arguments.
    pub cmd: Vec<String>,
    /// Its working directory; default the server's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    /// End it after this many seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<f64>,
    /// Bytes fed to its stdin, then closed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stdin: Vec<u8>,
    /// Run it on this machine, over its held link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
}

/// `exec` result.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct ExecResult {
    /// The exit code, or 128 + the signal that ended it.
    pub code: i32,
    /// What it wrote to stdout.
    pub stdout: Vec<u8>,
    /// What it wrote to stderr.
    pub stderr: Vec<u8>,
}

/// `file.read` and `file.write` params.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct FileParams {
    /// The file.
    pub path: PathBuf,
    /// `file.write`: its new bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Vec<u8>>,
    /// `file.read`: only these lines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lines: Option<Span>,
    /// The machine it is on, over its held link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
}

/// `machine.stats` params.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct StatsParams {
    /// Add the kept samples taken after this time (unix seconds) as `history`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<u64>,
}

/// One sample of a machine's health.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Sample {
    /// When it was taken, unix seconds.
    pub at: u64,
    /// The 1-minute load average divided by `cpus`.
    pub load_per_cpu: f64,
    /// Percent of memory not available.
    pub mem_pressure_pct: u64,
    /// Swap in use, MiB.
    pub swap_used_mb: u64,
    /// Free space on the volume of the state file, MiB.
    pub disk_free_mb: u64,
    /// Logical CPUs.
    pub cpus: u64,
    /// Seconds since boot.
    pub uptime_s: u64,
    /// Terminals by state: the reported state while running, else `running`; `exited`.
    pub terminals: BTreeMap<String, u64>,
}

/// A held link's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum LinkState {
    /// Connected.
    Up,
    /// Dropped, retried with backoff.
    Reconnecting,
    /// Never connected, or refused (another version); retried with backoff.
    Down,
}

/// One machine's held link, as `machine.links` reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Link {
    /// Up, reconnecting or down.
    pub state: LinkState,
    /// Since when, unix seconds.
    pub since: u64,
    /// Why it is down: the last connect's error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// `machine.stats` result: a sample taken now, and the kept ones after `since`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct MachineStats {
    /// Now.
    #[serde(flatten)]
    pub now: Sample,
    /// The kept samples (one a minute, 24 h) after `since`, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<Sample>,
}
