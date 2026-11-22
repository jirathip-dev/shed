//! The screen model: a VT parser feeding a visible grid and a bounded scrollback ring.
//!
//! Each cell keeps its character and pen (colours, bold, italic, underline, inverse) for
//! [`Screen::snapshot`]; the text reads drop the pen. A wide character takes two cells and a combining
//! one joins the cell before it, as `unicode-width` measures them.

mod grid;
mod paint;
pub mod report;
mod vt;

pub use report::{Report, STATES};

use std::fmt::Write as _;

use vte::Parser;

use grid::{Grid, TAIL, chars, space};
use paint::{join, paint, runs, sgr, trim_blank_tail};

/// A terminal size in character cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    /// Rows (lines), at least 1.
    pub rows: u16,
    /// Columns, at least 1.
    pub cols: u16,
}

/// A colour: a palette index (0–15 the named colours, up to 255 the xterm palette) or 24-bit RGB.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    /// A palette index.
    Index(u8),
    /// Red, green, blue.
    Rgb(u8, u8, u8),
}

/// How a cell is drawn, as SGR set it; `None` colours are the terminal's defaults.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "the four SGR attributes are independent flags"
)]
pub struct Pen {
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

/// Consecutive cells of one row with the same pen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    /// The cells' characters.
    pub text: String,
    /// Their pen.
    pub pen: Pen,
}

/// The visible screen as runs, for drawing it elsewhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// The size.
    pub size: Size,
    /// The cursor's row and column, from 0.
    pub cursor: (u16, u16),
    /// The lines in scrollback.
    pub scrollback: usize,
    /// One list of runs per row; trailing blank cells in the default pen are dropped.
    pub rows: Vec<Vec<Run>>,
}

/// A VT screen: feed it the program's output and read back plain text.
pub struct Screen {
    parser: Parser,
    grid: Grid,
}

impl Screen {
    /// A blank screen of `size` that keeps at most `scrollback` lines that scrolled off the top.
    #[must_use]
    pub fn new(size: Size, scrollback: usize) -> Self {
        Self::bounded(size, scrollback, usize::MAX)
    }

    /// A blank screen of `size` whose scrollback keeps at most `lines` lines and `bytes` bytes of text;
    /// the oldest whole lines go first, so no UTF-8 character is ever split.
    #[must_use]
    pub fn bounded(size: Size, lines: usize, bytes: usize) -> Self {
        Self {
            parser: Parser::new(),
            grid: Grid::new(size, (lines, bytes)),
        }
    }

    /// Feeds program output through the VT parser.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.grid, bytes);
    }

    /// Feeds program output like [`feed`](Self::feed) and returns it without the queries the screen
    /// answered, so an attached client's terminal does not answer them a second time.
    pub fn relay(&mut self, mut bytes: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(bytes.len());
        while !bytes.is_empty() {
            self.grid.answered = false;
            let n = self.parser.advance_until_terminated(&mut self.grid, bytes);
            let (done, rest) = bytes.split_at_checked(n).unwrap_or((bytes, &[]));
            let answered = self
                .grid
                .answered
                .then(|| done.iter().rposition(|&b| b == 0x1b));
            let keep = answered.flatten().unwrap_or(done.len());
            out.extend_from_slice(done.get(..keep).unwrap_or(done));
            bytes = rest;
        }
        out
    }

    /// Takes the bytes the screen owes the program (answers to cursor and device queries).
    pub fn take_replies(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.grid.replies)
    }

    /// Takes the states the program reported since the previous call, oldest first: OSC 7501 (`state=<s>[:<key>=<value>]…`
    /// or shed's `state=<s>;note=<n>`), or another agent's state escape (Jan's OSC 9999). The escape never
    /// reaches the screen.
    pub fn take_reports(&mut self) -> Vec<Report> {
        std::mem::take(&mut self.grid.reports)
    }

    /// The visible screen as plain text: one line per row, trailing spaces and blank rows dropped.
    #[must_use]
    pub fn text(&self) -> String {
        join(self.grid.visible())
    }

    /// The last `n` lines of scrollback followed by the visible screen, as plain text.
    #[must_use]
    pub fn history(&self, n: usize) -> String {
        let mut lines = self.grid.scrollback();
        lines.extend(self.grid.visible());
        trim_blank_tail(&mut lines);
        let skip = lines.len().saturating_sub(n);
        lines.drain(..skip);
        lines.join("\n")
    }

    /// How many lines have scrolled off the top so far, kept or not.
    #[must_use]
    pub fn scrolled(&self) -> u64 {
        self.grid.scrolled
    }

    /// The kept lines that scrolled off after the first `seq`, and how many after it are no longer kept.
    #[must_use]
    pub fn scrolled_since(&self, seq: u64) -> (Vec<String>, u64) {
        let mut lines = self.grid.scrollback();
        let new = self.grid.scrolled.saturating_sub(seq);
        let fresh = usize::try_from(new).unwrap_or(usize::MAX);
        let lost = new.saturating_sub(u64::try_from(lines.len()).unwrap_or(u64::MAX));
        lines.drain(..lines.len().saturating_sub(fresh));
        (lines, lost)
    }

    /// The visible screen as runs, with the cursor and the scrollback length.
    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        let g = &self.grid;
        let clamp = |n: usize| u16::try_from(n).unwrap_or(u16::MAX);
        let rows = g.cells.iter().map(|row| runs(row)).collect();
        Snapshot {
            size: g.size(),
            cursor: (clamp(g.row), clamp(g.col)),
            scrollback: g.lens.len(),
            rows,
        }
    }

    /// The current size.
    #[must_use]
    pub fn size(&self) -> Size {
        self.grid.size()
    }

    /// Whether the program has turned bracketed paste on (`ESC [ ? 2004 h`).
    #[must_use]
    pub fn bracketed_paste(&self) -> bool {
        self.grid.paste
    }

    /// Changes the size. Rows that no longer fit move into scrollback; lines are not re-wrapped.
    pub fn resize(&mut self, size: Size) {
        self.grid.resize(size);
    }

    /// VT text that gives a blank screen of this size the same rows, pens, cursor and modes.
    #[must_use]
    pub fn repaint(&self) -> String {
        let g = &self.grid;
        let mut out = String::from("\x1b[?1049l\x1b[r\x1b[H\x1b[2J");
        if let Some((cells, (row, col))) = &g.primary {
            paint(&mut out, cells);
            write!(out, "\x1b[{};{}H\x1b[?1049h", row + 1, col + 1).ok();
        }
        paint(&mut out, &g.cells);
        if g.top > 0 || g.bottom + 1 < g.rows {
            write!(out, "\x1b[{};{}r", g.top + 1, g.bottom + 1).ok();
        }
        let (saved, row) = (g.saved, g.cells.get(g.row));
        write!(out, "\x1b[{};{}H\x1b7", saved.0 + 1, saved.1 + 1).ok();
        let tail = row.and_then(|l| l.get(g.col)).is_some_and(|c| c.0 == TAIL);
        let col = g.col - usize::from(g.wrap_next && tail);
        write!(out, "\x1b[{};{}H", g.row + 1, col + 1).ok();
        if g.wrap_next {
            let (glyph, pen) = row.and_then(|l| l.get(col)).copied().unwrap_or_else(space);
            out.push_str(&sgr(&pen));
            out.extend(chars(&glyph));
        }
        out.push_str(&sgr(&g.pen));
        out.push_str(if g.paste {
            "\x1b[?2004h"
        } else {
            "\x1b[?2004l"
        });
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(rows: u16, cols: u16, scrollback: usize, input: &str) -> Screen {
        let mut s = Screen::new(Size { rows, cols }, scrollback);
        s.feed(input.as_bytes());
        s
    }

    #[test]
    fn a_repaint_fed_to_a_blank_screen_gives_the_same_screen() {
        let mut s = screen(4, 6, 10, "\x1b[1;31mred\x1b[0m 世e\u{301}\r\n");
        s.feed("\x1b[48;2;1;2;3mbg\x1b[0m\x1b[2;4H\x1b7\x1b[?2004h\x1b[?1049h".as_bytes());
        s.feed("alt\x1b[38;5;200;4m\x1b[2;3r\x1b[4;1Habcdef".as_bytes());
        let mut t = screen(4, 6, 10, "");
        t.feed(s.repaint().as_bytes());
        assert_eq!((t.snapshot(), t.repaint()), (s.snapshot(), s.repaint()));
        for more in ["x\x1b8y", "\x1b[?1049l", "z\r\n\r\n\r\n"] {
            s.feed(more.as_bytes());
            t.feed(more.as_bytes());
            assert_eq!(t.snapshot(), s.snapshot(), "after {more:?}");
        }
        assert!(t.bracketed_paste());
    }

    #[test]
    fn prints_lines_and_drops_trailing_blanks() {
        assert_eq!(screen(5, 20, 10, "hello\r\nworld  ").text(), "hello\nworld");
    }

    #[test]
    fn wraps_at_the_last_column() {
        assert_eq!(screen(3, 4, 10, "abcdef").text(), "abcd\nef");
    }

    #[test]
    fn cursor_moves_and_erases() {
        let s = screen(3, 10, 10, "xxxxx\x1b[1;3HAB\x1b[K\r\n\x1b[2Cz");
        assert_eq!(s.text(), "xxAB\n  z");
        assert_eq!(screen(3, 10, 10, "abc\r\ndef\x1b[2J").text(), "");
    }

    #[test]
    fn scrollback_is_bounded() {
        let s = screen(2, 10, 3, "1\r\n2\r\n3\r\n4\r\n5\r\n6");
        assert_eq!(s.text(), "5\n6");
        assert_eq!(s.history(100), "2\n3\n4\n5\n6");
        assert_eq!(s.history(2), "5\n6");
    }

    #[test]
    fn scrolled_since_counts_the_lines_trimmed_unread() {
        let mut s = screen(2, 10, 3, "1\r\n2\r\n");
        assert_eq!(
            (s.scrolled(), s.scrolled_since(0)),
            (1, (vec!["1".into()], 0))
        );
        s.feed(b"3\r\n4\r\n5\r\n6\r\n");
        assert_eq!(
            s.scrolled_since(1),
            (vec!["3".into(), "4".into(), "5".into()], 1)
        );
        assert_eq!(s.scrolled_since(s.scrolled()), (Vec::new(), 0));
    }

    #[test]
    fn scrollback_bytes_are_bounded_by_whole_lines() {
        let mut s = Screen::bounded(Size { rows: 1, cols: 10 }, 100, 7);
        s.feed("é1\r\né2\r\né3\r\né4".as_bytes());
        assert_eq!(s.history(100), "é2\né3\né4");
        assert_eq!(s.grid.history.len(), 6);
        s.feed(b"\x1b[3J");
        assert_eq!((s.grid.history.len(), s.grid.lens.len()), (0, 0));
    }

    #[test]
    fn alternate_screen_keeps_the_primary_and_no_history() {
        let mut s = screen(2, 10, 10, "main");
        s.feed(b"\x1b[?1049h\x1b[Halt\r\n1\r\n2\r\n3");
        assert_eq!(s.text(), "2\n3");
        s.feed(b"\x1b[?1049l");
        assert_eq!(s.text(), "main");
        assert_eq!(s.history(10), "main");
    }

    #[test]
    fn resize_moves_rows_into_scrollback() {
        let mut s = screen(4, 10, 10, "a\r\nb\r\nc\r\nd");
        s.resize(Size { rows: 2, cols: 1 });
        assert_eq!(s.text(), "c\nd");
        assert_eq!(s.history(10), "a\nb\nc\nd");
        assert_eq!(s.size(), Size { rows: 2, cols: 1 });
    }

    #[test]
    fn osc_7501_reports_state_and_never_reaches_the_screen() {
        let mut s = screen(3, 20, 0, "a\x1b]7501;state=working\x07b");
        assert_eq!(s.text(), "ab");
        let working = Report {
            state: "working".into(),
            ..Report::default()
        };
        assert_eq!(s.take_reports(), vec![working]);
        assert_eq!(s.take_reports(), vec![]);
        s.feed(b"\x1b]7501;state=blocked;note=needs a; reply\x1b\\c");
        s.feed(b"\x1b]7501;state=bogus\x07\x1b]2;title\x07");
        assert_eq!(s.text(), "abc");
        let blocked = Report {
            state: "blocked".into(),
            note: Some("needs a; reply".into()),
            ..Report::default()
        };
        assert_eq!(s.take_reports(), vec![blocked]);
    }

    fn program_status(osc: &str) -> Vec<Report> {
        let mut s = screen(3, 20, 0, &format!("\x1b]7501;{osc}\x1b\\"));
        assert_eq!(s.text(), "");
        s.take_reports()
    }

    #[test]
    fn osc_7501_reads_every_state_and_key() {
        for state in ["idle", "working", "done", "blocked", "error", "clear"] {
            let got = program_status(&format!("state={state}"));
            assert_eq!(got.first().map(|r| r.state.as_str()), Some(state));
        }
        let full = "state=blocked: kind=question :progress=40:app=make:id=build/tests:title=RG9jcw==:msg=T3ZlcndyaXRlIGNvbmZpZy50b21sPw==:x=1:bad";
        let blocked = Report {
            state: "blocked".into(),
            id: Some("build/tests".into()),
            kind: Some("question".into()),
            progress: Some(40),
            app: Some("make".into()),
            title: Some("Docs".into()),
            msg: Some("Overwrite config.toml?".into()),
            ..Report::default()
        };
        assert_eq!(program_status(full), vec![blocked]);
        let loose =
            program_status("state=done:kind=auth:progress=50:app=a b:state=working:progress=101");
        let working = Report {
            state: "working".into(),
            ..Report::default()
        };
        assert_eq!(loose, vec![working]);
    }

    #[test]
    fn osc_7501_drops_garbled_reports() {
        for osc in [
            "state=bogus",
            "kind=auth",
            "state=done:id=a//b",
            "state=done:id=a/b/c/d/e/f/g/h/i",
            "state=done:msg=QUFBQ",
            "state=done:msg=QQ=Q",
            "state=done:msg=Cg==",
            "state=done:title=/w==",
            &format!("state=done:title={}", "QUFB".repeat(65)),
        ] {
            assert_eq!(program_status(osc), vec![], "{osc}");
        }
    }

    #[test]
    fn osc_7501_probe_gets_the_spec_reply_in_the_same_terminator() {
        let reply = b"\x1b]7501;?:states=idle,working,done,blocked,error,clear:kinds=permission,question,auth";
        let mut s = screen(3, 20, 0, "\x1b]7501;?\x07");
        assert_eq!(s.take_replies(), [&reply[..], b"\x07"].concat());
        s.feed(b"\x1b]7501;?\x1b\\");
        assert_eq!(s.take_replies(), [&reply[..], b"\x1b\\"].concat());
        assert_eq!((s.take_reports(), s.text()), (vec![], String::new()));
    }

    #[test]
    fn jan_osc_9999_maps_waiting_to_blocked_and_ignores_garbage() {
        let mut s = screen(
            3,
            20,
            0,
            "\x1b]9999;{\"agentType\":\"jan\",\"state\":\"waiting\"}\x07",
        );
        let report: Vec<_> = s
            .take_reports()
            .into_iter()
            .map(|r| (r.state, r.source))
            .collect();
        assert_eq!(report, vec![("blocked".into(), Some("jan-osc9999".into()))]);
        s.feed(b"\x1b]9999;{\"state\":\x07\x1b]9999;[1]\x07\x1b]9999;{\"state\":7}\x07x");
        assert_eq!((s.take_reports(), s.text()), (vec![], "x".into()));
    }

    #[test]
    fn sgr_sets_the_pen_and_snapshot_groups_cells_into_runs() {
        let s = screen(
            3,
            20,
            5,
            "a\x1b[1;31mb\x1b[38;5;200;48:2::1:2:3;7mc\x1b[22;27;39;49;3;4md\x1b[mE \r\n\r\nz\x1b[1;2H",
        );
        let red_bold = Pen {
            fg: Some(Color::Index(1)),
            bold: true,
            ..Pen::default()
        };
        let mixed = Pen {
            fg: Some(Color::Index(200)),
            bg: Some(Color::Rgb(1, 2, 3)),
            inverse: true,
            ..red_bold
        };
        let plain = |text: &str| Run {
            text: text.into(),
            pen: Pen::default(),
        };
        let snap = s.snapshot();
        let first = vec![
            plain("a"),
            Run {
                text: "b".into(),
                pen: red_bold,
            },
            Run {
                text: "c".into(),
                pen: mixed,
            },
            Run {
                text: "d".into(),
                pen: Pen {
                    italic: true,
                    underline: true,
                    ..Pen::default()
                },
            },
            plain("E"),
        ];
        assert_eq!(snap.rows, vec![first, vec![], vec![plain("z")]]);
        assert_eq!((snap.cursor, snap.scrollback), ((0, 1), 0));
        assert_eq!(snap.size, Size { rows: 3, cols: 20 });
        assert_eq!(s.text(), "abcdE\n\nz");
    }

    #[test]
    fn wide_and_combining_characters_keep_the_box_edge_in_its_column() {
        let s = screen(
            4,
            10,
            0,
            "┌──────┐\r\n│日本語\x1b[2;8H│\r\n│😀e\u{301}x\x1b[3;8H│\r\n└──────┘",
        );
        assert_eq!(s.text(), "┌──────┐\n│日本語│\n│😀e\u{301}x  │\n└──────┘");
    }

    #[test]
    fn a_wide_character_at_the_last_column_wraps_whole() {
        assert_eq!(screen(3, 5, 0, "abcd日x").text(), "abcd\n日x");
        assert_eq!(screen(3, 5, 0, "abcde\u{301}日").text(), "abcde\u{301}\n日");
    }

    #[test]
    fn rows_past_their_written_cells_read_as_blanks() {
        let mut s = screen(
            3,
            10,
            5,
            "ab\x1b[1;6Hx\x1b[1;3H\x1b[2@\x1b[2;4H\u{301}\x1b[3;9Hyz",
        );
        s.feed(b"\x1b[3;2H\x1b[3P\x1b[1;9H\x1b[X");
        assert_eq!(s.text(), "ab     x\n   \u{301}\n     yz");
        s.resize(Size { rows: 2, cols: 12 });
        s.feed(b"\x1b[2;12Hq\x1b[1;1H\x1b[K");
        assert_eq!(s.text(), "\n     yz    q");
        assert_eq!(s.history(5), "ab     x\n\n     yz    q");
    }

    #[test]
    fn answers_cursor_position_queries() {
        let mut s = screen(5, 10, 0, "ab\x1b[6n");
        assert_eq!(s.take_replies(), b"\x1b[1;3R");
        assert!(s.take_replies().is_empty());
    }
}
