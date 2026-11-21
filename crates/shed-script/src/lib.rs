//! shed's script engine: a JavaScript subset parsed, compiled and run on a machine of its own.

use std::fmt;
use std::sync::Arc;

use crate::ast::Pos;

pub mod ast;
mod builtins;
mod compile;
pub mod heap;
mod json;
mod lex;
mod parse;
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
    /// The limit that stopped the run (`FUEL`, `MEMORY`, `DEPTH`).
    pub code: Option<String>,
}

pub(crate) fn syntax<T>(pos: Pos, message: impl Into<String>) -> Result<T, ScriptError> {
    Err(ScriptError {
        name: "SyntaxError".into(),
        message: message.into(),
        line: pos.line,
        column: pos.column,
        code: None,
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
