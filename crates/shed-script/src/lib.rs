//! shed's script engine: a JavaScript subset parsed into a tree, every left-out construct a `SyntaxError`.

pub mod ast;
mod error;
mod lex;
mod parse;

pub use error::ScriptError;

/// Parses a program, the body of an async function, into its tree.
///
/// # Errors
///
/// A `SyntaxError` at the line and column of the first construct outside the subset.
pub fn parse(code: &str) -> Result<ast::Script, ScriptError> {
    let chars: Vec<char> = code.chars().collect();
    parse::Parser::new(&chars)?.script()
}
