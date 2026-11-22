//! The `code.*` params and results, re-exported at the crate root.

use std::path::PathBuf;

#[cfg(feature = "schema")]
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One symbol of an outline: lines are 1-based and inclusive, `start` the first comment or attribute above it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Symbol {
    /// What it is.
    pub kind: Kind,
    /// Its name: an identifier, an impl's `Trait for Type`, a heading's or table's text.
    pub name: String,
    /// Its first line.
    pub start: usize,
    /// Its last line.
    pub end: usize,
    /// How many symbols hold it.
    pub depth: usize,
}

/// A symbol's kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A function or method.
    Fn,
    /// A struct or union.
    Struct,
    /// An enum.
    Enum,
    /// A trait.
    Trait,
    /// An impl block.
    Impl,
    /// A module or namespace.
    Mod,
    /// A constant or static.
    Const,
    /// A type alias.
    Type,
    /// A macro.
    Macro,
    /// A class.
    Class,
    /// An interface.
    Interface,
    /// A Markdown heading or a TOML table.
    Section,
    /// A line at column 0 with deeper lines under it, in a language shed has no rules for.
    Block,
}

/// A file's language, by its extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    /// `rs`.
    Rust,
    /// `ts tsx mts cts`.
    Typescript,
    /// `js jsx mjs cjs`.
    Javascript,
    /// `py pyi`.
    Python,
    /// `md markdown`.
    Markdown,
    /// `toml`.
    Toml,
    /// Anything else: blocks by indentation.
    Text,
}

/// `code.outline` params.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct OutlineParams {
    /// The file.
    pub path: PathBuf,
    /// The machine it is on, over its held link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
}

/// `code.outline` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct OutlineResult {
    /// The file, as asked for.
    pub path: String,
    /// Its language.
    pub lang: Lang,
    /// Its lines.
    pub lines: usize,
    /// Its symbols in file order.
    pub symbols: Vec<Symbol>,
}

/// `code.grep` params.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[expect(
    clippy::struct_excessive_bools,
    reason = "the wire shape: one flag per grep option"
)]
pub struct GrepParams {
    /// A folder or one file.
    pub root: PathBuf,
    /// shed's pattern syntax, or plain text with `fixed`.
    pub pattern: String,
    /// The pattern is plain text.
    #[serde(default)]
    pub fixed: bool,
    /// Letters match in either case.
    #[serde(default)]
    pub ignore_case: bool,
    /// A match starts and ends at a word boundary.
    #[serde(default)]
    pub word: bool,
    /// Per-file counts only: no lines, no outline.
    #[serde(default)]
    pub count: bool,
    /// Also dot files and dot folders, never `.git`.
    #[serde(default)]
    pub hidden: bool,
    /// `*.rs` keeps, `!tests/` drops, in `.gitignore` syntax.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub glob: Vec<String>,
    /// The machine it is on, over its held link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
}

/// `code.grep` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct GrepResult {
    /// The files with a match, in name order, paths relative to `root` with `/`.
    pub files: Vec<FileMatches>,
    /// Matched lines in all.
    pub matches: usize,
    /// Files read.
    pub searched: usize,
}

/// One file's matches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct FileMatches {
    /// Relative to `root`, with `/`.
    pub path: String,
    /// Its language.
    pub lang: Lang,
    /// Its matched lines.
    pub count: usize,
    /// The lines by the symbol that holds them; empty with `count`.
    pub groups: Vec<Group>,
}

/// Matched lines under one symbol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Group {
    /// The symbols around the lines, outermost first; empty at top level.
    pub scope: Vec<Symbol>,
    /// The lines, in file order.
    pub lines: Vec<GrepLine>,
}

/// One matched line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct GrepLine {
    /// 1-based.
    pub n: usize,
    /// The line as it is in the file, without its line end.
    pub text: String,
    /// `text` is only the line's first 4,096 bytes.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub clipped: bool,
    /// With `clipped`, the line's full length in bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length: Option<usize>,
}

/// `code.find` params.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct FindParams {
    /// A folder or one file.
    pub root: PathBuf,
    /// Words; a file is ranked by how many of them it answers, and where.
    pub query: String,
    /// How many files to return: default 10, at most 50.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top: Option<usize>,
    /// Also dot files and dot folders, never `.git`.
    #[serde(default)]
    pub hidden: bool,
    /// `*.rs` keeps, `!tests/` drops, in `.gitignore` syntax.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub glob: Vec<String>,
    /// The machine it is on, over its held link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
}

/// `code.find` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct FindResult {
    /// The best files, best first, ties by path.
    pub files: Vec<Found>,
    /// Files that earned anything; `files` is the first `top` of them.
    pub total: usize,
    /// Files read.
    pub searched: usize,
}

/// One ranked file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Found {
    /// Relative to `root`, with `/`.
    pub path: String,
    /// Its language.
    pub lang: Lang,
    /// Its lines.
    pub lines: usize,
    /// Its rank's score.
    pub score: u32,
    /// What earned it: `name is pattern`, `path has lane`, `lines: wait 89, output 6`, `a test`.
    pub why: Vec<String>,
    /// At most 5 symbols whose name holds a word, exact names first.
    pub symbols: Vec<Symbol>,
}

/// A span of lines, 1-based and inclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Span {
    /// The first line.
    pub start: usize,
    /// The last line.
    pub end: usize,
}

impl Kind {
    /// The wire name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Fn => "fn",
            Self::Struct => "struct",
            Self::Enum => "enum",
            Self::Trait => "trait",
            Self::Impl => "impl",
            Self::Mod => "mod",
            Self::Const => "const",
            Self::Type => "type",
            Self::Macro => "macro",
            Self::Class => "class",
            Self::Interface => "interface",
            Self::Section => "section",
            Self::Block => "block",
        }
    }
}

impl Lang {
    /// The wire name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Typescript => "typescript",
            Self::Javascript => "javascript",
            Self::Python => "python",
            Self::Markdown => "markdown",
            Self::Toml => "toml",
            Self::Text => "text",
        }
    }
}
