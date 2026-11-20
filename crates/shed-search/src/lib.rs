//! shed's code search: outlines by line rules, on shed-proto's types. Reads files in `walk` alone,
//! starts no process, reads no clock and no environment, writes nothing.
#![forbid(unsafe_code)]

pub mod lang;
pub mod text;
mod walk;

use std::fmt;

use shed_proto::{OutlineParams, OutlineResult};

/// The fixed limits a query may not go over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Symbols in one outline.
    pub symbols: usize,
    /// Bytes of a file read, by its size, before the read.
    pub bytes: u64,
}

/// The limits the server applies.
pub const LIMITS: Limits = Limits {
    symbols: 2000,
    bytes: 256 << 20,
};

/// Why a query has no result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// It went over a limit.
    Limit(Over),
    /// A file could not be read.
    Io(String),
}

/// The limit a query went over, its value and where it stood.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Over {
    /// The limit.
    pub limit: Limit,
    /// Its value.
    pub max: u64,
    /// The file the query was at.
    pub at: String,
}

/// A limit, by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Limit {
    /// [`Limits::symbols`].
    Symbols,
    /// [`Limits::bytes`].
    Bytes,
}

impl fmt::Display for Over {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self.limit {
            Limit::Symbols => "symbols",
            Limit::Bytes => "bytes",
        };
        write!(f, "{name} over {} at {}", self.max, self.at)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Limit(over) => over.fmt(f),
            Self::Io(why) => f.write_str(why),
        }
    }
}

/// A file's symbols in file order.
///
/// # Errors
/// [`Error::Io`] when the file cannot be read, [`Error::Limit`] over `limits`.
pub fn outline(p: &OutlineParams, limits: &Limits) -> Result<OutlineResult, Error> {
    let path = p.path.display().to_string();
    let bytes = walk::read(&p.path, limits.bytes)?;
    let text = String::from_utf8_lossy(&bytes);
    let lang = lang::of(&path);
    let symbols = lang::outline(lang, &text);
    if symbols.len() > limits.symbols {
        let max = u64::try_from(limits.symbols).unwrap_or(u64::MAX);
        let limit = Limit::Symbols;
        return Err(Error::Limit(Over {
            limit,
            max,
            at: path,
        }));
    }
    Ok(OutlineResult {
        lines: text.lines().count(),
        path,
        lang,
        symbols,
    })
}
