//! The `screen.watch` stream and the `terminal.*` methods a person's client uses.

#[cfg(feature = "schema")]
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// `screen.watch` params.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct WatchParams {
    /// The terminal's name.
    pub terminal: String,
    /// The `seq` of the last update the client applied: no full screen first if nothing changed since.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<u64>,
}

/// `screen.watch` writes one update per line. `seq` is the screen's change count; `rows`, `cols`, `cursor`
/// (`[row, col]`, from 0) and `scrollback` (lines) are always there; `lines` holds the rows that changed, each
/// as runs of cells with one pen: `fg` and `bg` (a palette index 0–255, or `[r, g, b]`; `null` is the default),
/// `bold`, `italic`, `underline` and `inverse`. Trailing blank default cells are dropped, so a blank row has no
/// runs. `full` marks an update that carries every row: the first, and the one after a resize.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct ScreenUpdate {
    /// The screen's change count it shows; pass it as `since` when watching again.
    pub seq: u64,
    /// Every row is in `lines` (the first update, and after a resize).
    pub full: bool,
    /// Rows.
    pub rows: u16,
    /// Columns.
    pub cols: u16,
    /// The cursor's row and column, from 0.
    pub cursor: [u16; 2],
    /// Lines in scrollback.
    pub scrollback: usize,
    /// The rows that changed.
    pub lines: Vec<Line>,
}

/// One row of a [`ScreenUpdate`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Line {
    /// The row, from 0.
    pub row: u16,
    /// Its cells, in runs of the same pen; trailing blank default cells dropped.
    pub runs: Vec<Run>,
}

/// Consecutive cells with the same pen; `null` colours are the terminal's defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[expect(
    clippy::struct_excessive_bools,
    reason = "the four SGR attributes are independent flags"
)]
pub struct Run {
    /// The cells' characters.
    pub text: String,
    /// Foreground.
    pub fg: Option<Color>,
    /// Background.
    pub bg: Option<Color>,
    /// Bold.
    pub bold: bool,
    /// Italic.
    pub italic: bool,
    /// Underline.
    pub underline: bool,
    /// Foreground and background swapped.
    pub inverse: bool,
}

/// A colour: a palette index (0–15 the named colours, up to 255 the xterm palette) or `[r, g, b]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(untagged)]
pub enum Color {
    /// A palette index.
    Index(u8),
    /// Red, green, blue.
    Rgb([u8; 3]),
}

/// `terminal.resize` params.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct TerminalResizeParams {
    /// The terminal's name; default the one this connection watches.
    #[serde(default, alias = "name", skip_serializing_if = "Option::is_none")]
    pub terminal: Option<String>,
    /// Columns, at least 1.
    pub cols: u16,
    /// Rows, at least 1.
    pub rows: u16,
}

/// `terminal.readonly` params.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct TerminalParams {
    /// The terminal's name.
    pub terminal: String,
}
