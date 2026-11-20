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
