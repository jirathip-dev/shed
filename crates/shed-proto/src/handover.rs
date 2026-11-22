//! The `handover` methods.

use std::path::PathBuf;

#[cfg(feature = "schema")]
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// `handover.check` params.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct HandoverCheckParams {
    /// The build the binary must be.
    pub build: String,
    /// The binary's path.
    pub candidate: PathBuf,
}

/// `handover` params.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct HandoverParams {
    /// The build the server's own binary must be now.
    pub build: String,
}

/// The answer to `handover.check` and `handover`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Handover {
    /// The server's build.
    pub from: String,
    /// The binary's build.
    pub to: String,
    /// The running terminals that cross.
    pub terminals: usize,
}
