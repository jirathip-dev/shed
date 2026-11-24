//! A command run inside `srt`: what it may read, write and reach, besides what shed always allows and denies.

use std::path::PathBuf;

#[cfg(feature = "schema")]
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// `run`'s and `exec`'s `sandbox`: the caller's folders and hosts; shed adds the system's, the agent's and its own denies.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Sandbox {
    /// Folders and files it may read, besides the system's own.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub read: Vec<PathBuf>,
    /// Folders and files it may write (and read).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub write: Vec<PathBuf>,
    /// Reads denied even inside an allowed folder.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deny: Vec<PathBuf>,
    /// Domains it may reach (`*.example.com` allowed); none: no network.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hosts: Vec<String>,
    /// Unix sockets it may connect to, besides the inside socket.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sockets: Vec<PathBuf>,
    /// macOS mach services it may look up, besides srt's defaults.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mach: Vec<String>,
    /// Its own temp folder; default one shed makes for it in its state folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tmp: Option<PathBuf>,
}

/// An agent record's `[sandbox]`: what the agent itself needs inside, `~` for home and `{config}` for the run's
/// config folder (its account's, else the record's `default_folder`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct SandboxNeeds {
    /// The variable `srt` takes the run's temp folder from.
    pub tmp_var: Option<String>,
    /// Folders and files it reads.
    pub read: Vec<String>,
    /// Folders and files it writes.
    pub write: Vec<String>,
    /// Domains it reaches.
    pub hosts: Vec<String>,
}
