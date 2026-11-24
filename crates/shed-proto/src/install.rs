//! The `install` methods: releases of shed's own binary and the calling tool's, on the main host.

use std::collections::BTreeMap;
use std::path::PathBuf;

#[cfg(feature = "schema")]
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The calling tool, named at `hello`: the one binary besides shed's own the `install` methods move.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Tool {
    /// Its name, as its release assets start (`drover`); the last part of `exe`.
    pub name: String,
    /// Its live binary's absolute path.
    pub exe: PathBuf,
}

/// Where a release is: `<releases>/<tag>/SHA256SUMS` and `<releases>/<tag>/<tool>-<arch>-<os>`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Source {
    /// `shed` or the caller's [`Tool`] name.
    pub tool: String,
    /// The releases URL.
    pub releases: String,
    /// The release tag, the caller's choice.
    pub tag: String,
}

/// `install.find` result: a release's assets.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Release {
    /// The tag.
    pub tag: String,
    /// Each asset's sha256, from `SHA256SUMS`.
    pub assets: BTreeMap<String, String>,
}

/// `install.stage` params.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Stage {
    /// The release.
    pub source: Source,
    /// What `<dest>.new --version` must contain.
    pub build: String,
    /// The live binary.
    pub dest: PathBuf,
}

/// `install.swap` params.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Swap {
    /// The live binary.
    pub dest: PathBuf,
    /// The build `<dest>.new` must be.
    pub build: String,
}

/// `install.ls` and `install.back` params.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Dest {
    /// The live binary.
    pub dest: PathBuf,
}

/// An install's three files, each by its `--version` line; none when missing or not answering.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Install {
    /// `<dest>`.
    pub live: Option<String>,
    /// `<dest>.new`.
    pub staged: Option<String>,
    /// `<dest>.prev`.
    pub prev: Option<String>,
}
