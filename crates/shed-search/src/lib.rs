//! shed's code search: grep and outlines by line rules, on shed-proto's types. Reads files in `walk` alone,
//! starts no process, reads no clock and no environment, writes nothing.
#![forbid(unsafe_code)]

mod grep;
mod ignore;
pub mod lang;
mod matcher;
mod needle;
pub mod text;
mod walk;

use std::fmt;
use std::time::Duration;

pub use grep::grep;

use shed_proto::{OutlineParams, OutlineResult};

/// The fixed limits a query may not go over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Files read, after the ignore rules and globs.
    pub files: usize,
    /// Rows in a grep result: matched lines, or files with `count`.
    pub matches: usize,
    /// Symbols in one outline.
    pub symbols: usize,
    /// Bytes of the files read, by their size, before each read.
    pub bytes: u64,
    /// Bytes of a matched line returned; a longer one comes back clipped.
    pub line: usize,
    /// Wall time; the caller's `expired` reads the clock.
    pub deadline: Duration,
}

/// The limits the server applies.
pub const LIMITS: Limits = Limits {
    files: 20_000,
    matches: 250,
    symbols: 2000,
    bytes: 256 << 20,
    line: 4096,
    deadline: Duration::from_secs(10),
};

/// Why a query has no result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A pattern that does not compile, or an empty one.
    Query(String),
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
    /// [`Limits::files`].
    Files,
    /// [`Limits::matches`].
    Matches,
    /// [`Limits::symbols`].
    Symbols,
    /// [`Limits::bytes`].
    Bytes,
    /// [`Limits::deadline`], in seconds.
    Deadline,
}

impl fmt::Display for Over {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self.limit {
            Limit::Files => "files",
            Limit::Matches => "matches",
            Limit::Symbols => "symbols",
            Limit::Bytes => "bytes",
            Limit::Deadline => "deadline",
        };
        write!(f, "{name} over {} at {}", self.max, self.at)?;
        if self.limit != Limit::Symbols {
            f.write_str("; narrow the path, add a glob, or ask for a count")?;
        }
        Ok(())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Limit(over) => over.fmt(f),
            Self::Query(why) | Self::Io(why) => f.write_str(why),
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
