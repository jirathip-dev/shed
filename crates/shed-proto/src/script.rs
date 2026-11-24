//! The `script` params and reply, and how each method may be called inside a script; re-exported at the crate root.

use std::path::PathBuf;

#[cfg(feature = "schema")]
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Method;

/// `script` params.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct ScriptParams {
    /// The program: the body of an async function, in shed's JavaScript subset; at most 256 KiB.
    pub code: String,
    /// Stop it after this many seconds: 60 unless given, at most 600.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<f64>,
    /// Run it on this machine's server, over its held link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
    /// Who sends it, as the sender says; the guard gets it unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller: Option<Caller>,
    /// Parse and compile only: nothing runs, no call is made, no guard is asked.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub check: bool,
}

/// What a script's sender says about itself. shed proves none of it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Caller {
    /// The terminal it runs in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal: Option<String>,
    /// Its working directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    /// Its role (`impl`, `rev`, `orchestrator`, …), for the guard to refuse by; it never allows more.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
}

/// `script` result: `result` when the program returned, `error` when it did not; never both.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct ScriptResult {
    /// What the program returned, as JSON; `null` when it returned nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// Why it stopped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ScriptError>,
    /// One entry per `console` call, in order.
    pub logs: Vec<String>,
    /// How many `shed` calls it made, refused ones included.
    pub calls: u64,
    /// Every call of a guarded method it made, in call order, with what came of it.
    pub acting: Vec<Acting>,
    /// How long it took, queueing and guards included.
    pub ms: u64,
}

/// One acting call: `result` when it was answered, `error` when the guard refused it, it failed or it was
/// still in flight when the run stopped (`CANCELLED`); never both. Over `REPLY_LIMIT` only `method` and
/// `error` (or `result: true`) are kept.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Acting {
    /// The method's wire name.
    pub method: String,
    /// Its params, as the script gave them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
    /// What it answered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// Its refusal (`GUARD_REFUSED`, …), shed's error or `CANCELLED`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ActingError>,
}

/// Why an acting call came to nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct ActingError {
    /// `GUARD_REFUSED`, `GUARD_FAILED`, `GUARD_TIMEOUT`, `NO_GUARD`, `CANCELLED` or shed's own code.
    pub code: String,
    /// One line.
    pub message: String,
}

/// A script's error: where, and what.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct ScriptError {
    /// `SyntaxError`, `ReferenceError`, `TypeError`, `RangeError`, `Error`, `ShedError`, `GuardError` or `LimitError`.
    pub name: String,
    /// One line.
    pub message: String,
    /// The line, from 1.
    pub line: u32,
    /// The column, from 1, in code points.
    pub column: u32,
    /// A shed error's name (`NO_SUCH_TERMINAL`), a guard's verdict (`GUARD_REFUSED`) or the limit (`FUEL`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// The shed method that failed or was refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
}

/// How a method may be called inside a script.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Called as it is, with no guard.
    Read,
    /// The guard is asked first.
    Guarded,
    /// `NOT_IN_SCRIPT`: it streams, belongs to a person's keyboard or the connection, or replaces the server.
    Refused,
}

impl Access {
    /// Its name in `shed.methods()`: `read`, `guarded` or `refused`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Guarded => "guarded",
            Self::Refused => "refused",
        }
    }
}

impl Method {
    /// How the method may be called inside a script.
    #[must_use]
    pub const fn in_script(self) -> Access {
        match self {
            Self::Ls
            | Self::Get
            | Self::Read
            | Self::Wait
            | Self::MachineStats
            | Self::MachineLinks
            | Self::FlowReport
            | Self::Agents
            | Self::AgentsMatch
            | Self::Discover
            | Self::TerminalProcs => Access::Read,
            Self::Run
            | Self::Send
            | Self::Keys
            | Self::Kill
            | Self::Rm
            | Self::Resize
            | Self::State
            | Self::Model
            | Self::Exec
            | Self::FileWrite
            | Self::FileRead
            | Self::CodeOutline
            | Self::CodeGrep
            | Self::CodeFind
            | Self::MachineAdd
            | Self::MachineRm
            | Self::AccountAdd
            | Self::AccountRm
            | Self::AccountLs
            | Self::AccountStatus
            | Self::AccountUse
            | Self::ProxyKeys
            | Self::ProxyRevoke => Access::Guarded,
            Self::Hello
            | Self::Events
            | Self::Attach
            | Self::ScreenWatch
            | Self::Input
            | Self::AccountLogin
            | Self::TerminalKeys
            | Self::TerminalResize
            | Self::ReadOnly
            | Self::Heartbeat
            | Self::Emit
            | Self::HandoverCheck
            | Self::Handover
            | Self::InstallFind
            | Self::InstallLs
            | Self::InstallStage
            | Self::InstallSwap
            | Self::InstallBack
            | Self::ProxyRelay
            | Self::Script => Access::Refused,
        }
    }
}
