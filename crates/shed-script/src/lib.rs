//! shed's script engine: a JavaScript subset parsed, compiled and run on a machine of its own.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::ast::Pos;

pub mod ast;
mod builtins;
mod compile;
pub mod heap;
mod json;
mod lex;
mod parse;
mod task;
pub mod value;
pub mod vm;

/// Parses a program, the body of an async function, into its tree.
///
/// # Errors
///
/// A `SyntaxError` at the line and column of the first construct outside the subset.
pub fn parse(code: &str) -> Result<ast::Script, ScriptError> {
    let chars: Vec<char> = code.chars().collect();
    parse::Parser::new(&chars)?.script()
}

/// A program parsed and compiled: code and constants, no state; each [`vm::Run`] has its own.
#[derive(Debug, Clone)]
pub struct Program {
    compiled: Arc<compile::Compiled>,
}

impl Program {
    /// Parses and compiles a program, the body of an async function.
    ///
    /// # Errors
    ///
    /// A `SyntaxError` at the line and column of the first construct outside the subset.
    pub fn parse(code: &str) -> Result<Self, ScriptError> {
        let script = parse(code)?;
        Ok(Self {
            compiled: Arc::new(compile::Compiler::program(&script)?),
        })
    }
}

/// What a script can reach: shed's methods, by wire name. The engine's only way out.
pub trait Host: Sync {
    /// Starts one call; the engine polls it with the others in flight and drops it to cancel.
    fn call(&self, method: &str, params: serde_json::Value) -> Call<'_>;
}

/// One call in flight, as the host started it.
pub type Call<'a> = Pin<Box<dyn Future<Output = Result<serde_json::Value, HostError>> + Send + 'a>>;

/// A call's failure as the engine throws it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostError {
    /// The error's class: `ShedError` or `GuardError`.
    pub name: String,
    /// shed's name for it (`NO_SUCH_TERMINAL`) or the guard's verdict (`GUARD_REFUSED`).
    pub code: String,
    /// One line, written to be acted on.
    pub message: String,
}

/// A script's error: its class, one line to act on, and where it was thrown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptError {
    /// `SyntaxError`, `ReferenceError`, `TypeError`, `RangeError`, `Error`, `ShedError`, `GuardError` or `LimitError`.
    pub name: String,
    /// One line, written to be acted on.
    pub message: String,
    /// The line, from 1.
    pub line: u32,
    /// The column, from 1, in code points.
    pub column: u32,
    /// A shed error's name, a guard's verdict or the limit that stopped the run (`FUEL`).
    pub code: Option<String>,
    /// The shed method that failed or was refused.
    pub method: Option<String>,
}

pub(crate) fn syntax<T>(pos: Pos, message: impl Into<String>) -> Result<T, ScriptError> {
    Err(ScriptError {
        name: "SyntaxError".into(),
        message: message.into(),
        line: pos.line,
        column: pos.column,
        code: None,
        method: None,
    })
}

impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {}:{}: {}",
            self.name, self.line, self.column, self.message
        )
    }
}

impl std::error::Error for ScriptError {}
