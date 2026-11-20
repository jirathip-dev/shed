use std::fmt;

use crate::ast::Pos;

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
}

pub(crate) fn syntax<T>(pos: Pos, message: impl Into<String>) -> Result<T, ScriptError> {
    Err(ScriptError {
        name: "SyntaxError".into(),
        message: message.into(),
        line: pos.line,
        column: pos.column,
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
