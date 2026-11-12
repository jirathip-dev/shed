//! The screen model: a VT parser feeding a visible grid and a bounded scrollback ring.
//!
//! Each cell keeps its character and pen (colours, bold, italic, underline, inverse) for
//! [`Screen::snapshot`]; the text reads drop the pen. Each character takes one cell; wide and combining
//! characters are not measured yet.

use std::collections::VecDeque;

use vte::{Params, Parser, Perform};

/// A terminal size in character cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    /// Rows (lines), at least 1.
    pub rows: u16,
    /// Columns, at least 1.
    pub cols: u16,
}

/// The states a program can report (`shed state`, or the OSC 7501 escape).
pub const STATES: [&str; 4] = ["working", "blocked", "done", "error"];

type Mapping = &'static [(&'static str, &'static str)];

const FOREIGN: [(&[u8], &str, Mapping); 1] = [(
    b"9999",
    "jan-osc9999",
    &[
        ("working", "working"),
        ("blocked", "blocked"),
        ("waiting", "blocked"),
        ("done", "done"),
    ],
)];

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

/// A state a program reported, with an optional note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// One of [`STATES`], or `idle` from the silence fallback.
    pub state: String,
    /// A free-text note.
    pub note: Option<String>,
    /// The foreign escape it came from (e.g. `jan-osc9999`); `None` for shed's own.
    pub source: Option<String>,
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

    /// Takes the bytes the screen owes the program (answers to cursor and device queries).
    pub fn take_replies(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.grid.replies)
    }

    /// Takes the last state the program reported with `ESC ] 7501 ; state=<s>[;note=<n>] BEL`, or another
    /// agent's state escape (Jan's OSC 9999), since the previous call. The escape never reaches the screen.
    pub fn take_report(&mut self) -> Option<Report> {
        self.grid.report.take()
    }

    /// The visible screen as plain text: one line per row, trailing spaces and blank rows dropped.
    #[must_use]
    pub fn text(&self) -> String {
        join(self.grid.visible())
    }

    /// The last `n` lines of scrollback followed by the visible screen, as plain text.
    #[must_use]
    pub fn history(&self, n: usize) -> String {
        let mut lines: Vec<String> = self.grid.history.iter().cloned().collect();
        lines.extend(self.grid.visible());
        trim_blank_tail(&mut lines);
        let skip = lines.len().saturating_sub(n);
        lines.drain(..skip);
        lines.join("\n")
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
            scrollback: g.history.len(),
            rows,
        }
    }

    /// The current size.
    #[must_use]
    pub fn size(&self) -> Size {
        self.grid.size()
    }

    /// Changes the size. Rows that no longer fit move into scrollback; lines are not re-wrapped.
    pub fn resize(&mut self, size: Size) {
        self.grid.resize(size);
    }
}

fn trim_blank_tail(lines: &mut Vec<String>) {
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
}

fn join(mut lines: Vec<String>) -> String {
    trim_blank_tail(&mut lines);
    lines.join("\n")
}

fn line_text(row: &[Cell]) -> String {
    let s: String = row.iter().map(|c| c.0).collect();
    s.trim_end().to_owned()
}

fn runs(row: &[Cell]) -> Vec<Run> {
    let end = row.iter().rposition(|c| *c != space()).map_or(0, |i| i + 1);
    let mut runs: Vec<Run> = Vec::new();
    for &(c, pen) in row.iter().take(end) {
        match runs.last_mut() {
            Some(run) if run.pen == pen => run.text.push(c),
            _ => runs.push(Run {
                text: c.to_string(),
                pen,
            }),
        }
    }
    runs
}

type Cell = (char, Pen);
fn space() -> Cell {
    (' ', Pen::default())
}
type Cells = Vec<Vec<Cell>>;

struct Grid {
    rows: usize,
    cols: usize,
    cells: Cells,
    row: usize,
    col: usize,
    wrap_next: bool,
    top: usize,
    bottom: usize,
    saved: (usize, usize),
    history: VecDeque<String>,
    history_bytes: usize,
    limit: (usize, usize),
    primary: Option<(Cells, (usize, usize))>,
    replies: Vec<u8>,
    report: Option<Report>,
    pen: Pen,
}

impl Grid {
    fn new(size: Size, limit: (usize, usize)) -> Self {
        let (rows, cols) = (usize::from(size.rows.max(1)), usize::from(size.cols.max(1)));
        Self {
            rows,
            cols,
            cells: vec![vec![space(); cols]; rows],
            row: 0,
            col: 0,
            wrap_next: false,
            top: 0,
            bottom: rows - 1,
            saved: (0, 0),
            history: VecDeque::new(),
            history_bytes: 0,
            limit,
            primary: None,
            replies: Vec::new(),
            report: None,
            pen: Pen::default(),
        }
    }

    fn size(&self) -> Size {
        let clamp = |n: usize| u16::try_from(n).unwrap_or(u16::MAX);
        Size {
            rows: clamp(self.rows),
            cols: clamp(self.cols),
        }
    }

    fn visible(&self) -> Vec<String> {
        self.cells.iter().map(|r| line_text(r)).collect()
    }

    fn blank(&self) -> Vec<Cell> {
        vec![space(); self.cols]
    }

    fn push_history(&mut self, line: String) {
        let (lines, bytes) = self.limit;
        if self.primary.is_some() || lines == 0 || line.len() > bytes {
            return;
        }
        while self.history.len() >= lines || self.history_bytes + line.len() > bytes {
            let Some(old) = self.history.pop_front() else {
                break;
            };
            self.history_bytes -= old.len();
        }
        self.history_bytes += line.len();
        self.history.push_back(line);
    }

    fn scroll_up(&mut self, n: usize) {
        for _ in 0..n.min(self.bottom - self.top + 1) {
            let line = self.cells.remove(self.top);
            if self.top == 0 {
                self.push_history(line_text(&line));
            }
            self.cells.insert(self.bottom, self.blank());
        }
    }

    fn scroll_down(&mut self, n: usize) {
        for _ in 0..n.min(self.bottom - self.top + 1) {
            self.cells.remove(self.bottom);
            self.cells.insert(self.top, self.blank());
        }
    }

    fn linefeed(&mut self) {
        if self.row == self.bottom {
            self.scroll_up(1);
        } else if self.row + 1 < self.rows {
            self.row += 1;
        }
    }

    fn reverse_index(&mut self) {
        if self.row == self.top {
            self.scroll_down(1);
        } else {
            self.row = self.row.saturating_sub(1);
        }
    }

    fn goto(&mut self, row: usize, col: usize) {
        self.row = row.min(self.rows - 1);
        self.col = col.min(self.cols - 1);
        self.wrap_next = false;
    }

    fn erase(&mut self, row: usize, from: usize, to: usize) {
        let to = to.min(self.cols);
        if let Some(cells) = self.cells.get_mut(row).and_then(|r| r.get_mut(from..to)) {
            cells.fill(space());
        }
    }

    fn erase_display(&mut self, mode: u16) {
        let (row, col) = (self.row, self.col);
        match mode {
            0 => {
                self.erase(row, col, self.cols);
                for r in row + 1..self.rows {
                    self.erase(r, 0, self.cols);
                }
            }
            1 => {
                for r in 0..row {
                    self.erase(r, 0, self.cols);
                }
                self.erase(row, 0, col + 1);
            }
            2 | 3 => {
                for r in 0..self.rows {
                    self.erase(r, 0, self.cols);
                }
                if mode == 3 {
                    self.history.clear();
                    self.history_bytes = 0;
                }
            }
            _ => {}
        }
    }

    fn lines_at_cursor(&mut self, n: usize, insert: bool) {
        if self.row < self.top || self.row > self.bottom {
            return;
        }
        for _ in 0..n.min(self.bottom - self.row + 1) {
            if insert {
                self.cells.remove(self.bottom);
                self.cells.insert(self.row, self.blank());
            } else {
                self.cells.remove(self.row);
                self.cells.insert(self.bottom, self.blank());
            }
        }
    }

    fn chars_at_cursor(&mut self, n: usize, insert: bool) {
        let (col, cols) = (self.col, self.cols);
        let Some(line) = self.cells.get_mut(self.row).and_then(|l| l.get_mut(col..)) else {
            return;
        };
        let n = n.min(cols - col);
        if insert {
            line.rotate_right(n);
            line.iter_mut().take(n).for_each(|c| *c = space());
        } else {
            line.rotate_left(n);
            line.iter_mut().rev().take(n).for_each(|c| *c = space());
        }
    }

    fn set_alt(&mut self, on: bool) {
        if on && self.primary.is_none() {
            let blank = vec![self.blank(); self.rows];
            let cells = std::mem::replace(&mut self.cells, blank);
            self.primary = Some((cells, (self.row, self.col)));
        } else if !on && let Some((cells, (row, col))) = self.primary.take() {
            self.cells = cells;
            self.goto(row, col);
        }
    }

    fn resize(&mut self, size: Size) {
        let (rows, cols) = (usize::from(size.rows.max(1)), usize::from(size.cols.max(1)));
        let fit = |cells: &mut Cells| {
            for line in cells.iter_mut() {
                line.resize(cols, space());
            }
        };
        fit(&mut self.cells);
        self.cols = cols;
        while self.cells.len() > rows
            && self.cells.len() - 1 > self.row
            && self.cells.last().is_some_and(|l| line_text(l).is_empty())
        {
            self.cells.pop();
        }
        while self.cells.len() > rows {
            let line = self.cells.remove(0);
            self.push_history(line_text(&line));
            self.row = self.row.saturating_sub(1);
        }
        self.cells.resize(rows, vec![space(); cols]);
        if let Some((cells, _)) = &mut self.primary {
            fit(cells);
            cells.resize(rows, vec![space(); cols]);
        }
        self.rows = rows;
        self.top = 0;
        self.bottom = rows - 1;
        self.goto(self.row, self.col);
    }

    fn sgr(&mut self, params: &Params) {
        let mut it = params.iter().peekable();
        while let Some(p) = it.next() {
            let pen = &mut self.pen;
            let index = |n: u16| Some(Color::Index(u8::try_from(n).unwrap_or(0)));
            let [first, tail @ ..] = p else {
                continue;
            };
            match *first {
                0 => *pen = Pen::default(),
                1 => pen.bold = true,
                3 => pen.italic = true,
                4 => pen.underline = true,
                7 => pen.inverse = true,
                22 => pen.bold = false,
                23 => pen.italic = false,
                24 => pen.underline = false,
                27 => pen.inverse = false,
                n @ 30..=37 => pen.fg = index(n - 30),
                n @ 90..=97 => pen.fg = index(n - 82),
                39 => pen.fg = None,
                n @ 40..=47 => pen.bg = index(n - 40),
                n @ 100..=107 => pen.bg = index(n - 92),
                49 => pen.bg = None,
                n @ (38 | 48) => {
                    let mut rest: Vec<u16> = if tail.is_empty() {
                        let take = match it.peek().and_then(|v| v.first()) {
                            Some(5) => 2,
                            Some(2) => 4,
                            _ => 0,
                        };
                        it.by_ref()
                            .take(take)
                            .filter_map(|v| v.first().copied())
                            .collect()
                    } else {
                        tail.to_vec()
                    };
                    if rest.len() == 5 {
                        rest.remove(1);
                    }
                    let byte = |v: u16| u8::try_from(v).unwrap_or(u8::MAX);
                    let color = match *rest.as_slice() {
                        [5, i] => Some(Color::Index(byte(i))),
                        [2, r, g, b] => Some(Color::Rgb(byte(r), byte(g), byte(b))),
                        _ => continue,
                    };
                    *if n == 38 { &mut pen.fg } else { &mut pen.bg } = color;
                }
                _ => {}
            }
        }
    }
}

fn param(params: &Params, i: usize, default: u16) -> usize {
    let v = params
        .iter()
        .nth(i)
        .and_then(|p| p.first().copied())
        .unwrap_or(0);
    usize::from(if v == 0 { default } else { v })
}

impl Perform for Grid {
    fn print(&mut self, c: char) {
        if self.wrap_next {
            self.col = 0;
            self.linefeed();
            self.wrap_next = false;
        }
        if let Some(cell) = self
            .cells
            .get_mut(self.row)
            .and_then(|r| r.get_mut(self.col))
        {
            *cell = (c, self.pen);
        }
        if self.col + 1 == self.cols {
            self.wrap_next = true;
        } else {
            self.col += 1;
        }
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            b'\n' | 0x0b | 0x0c => self.linefeed(),
            b'\r' => self.col = 0,
            0x08 => self.col = self.col.saturating_sub(1),
            b'\t' => self.col = ((self.col / 8 + 1) * 8).min(self.cols - 1),
            _ => return,
        }
        self.wrap_next = false;
    }

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], _ignore: bool, action: char) {
        let n = param(params, 0, 1);
        let (row, col) = (self.row, self.col);
        if intermediates == b"?" {
            if matches!(action, 'h' | 'l')
                && params
                    .iter()
                    .any(|p| matches!(p.first(), Some(47 | 1047 | 1049)))
            {
                self.set_alt(action == 'h');
            }
            return;
        }
        if !intermediates.is_empty() {
            return;
        }
        match action {
            'A' => self.goto(row.saturating_sub(n), col),
            'B' | 'e' => self.goto(row + n, col),
            'C' | 'a' => self.goto(row, col + n),
            'D' => self.goto(row, col.saturating_sub(n)),
            'E' => self.goto(row + n, 0),
            'F' => self.goto(row.saturating_sub(n), 0),
            'G' | '`' => self.goto(row, n - 1),
            'd' => self.goto(n - 1, col),
            'H' | 'f' => self.goto(n - 1, param(params, 1, 1) - 1),
            'J' => self.erase_display(
                params
                    .iter()
                    .next()
                    .and_then(|p| p.first().copied())
                    .unwrap_or(0),
            ),
            'K' => match params
                .iter()
                .next()
                .and_then(|p| p.first().copied())
                .unwrap_or(0)
            {
                0 => self.erase(row, col, self.cols),
                1 => self.erase(row, 0, col + 1),
                2 => self.erase(row, 0, self.cols),
                _ => {}
            },
            'L' => self.lines_at_cursor(n, true),
            'M' => self.lines_at_cursor(n, false),
            '@' => self.chars_at_cursor(n, true),
            'P' => self.chars_at_cursor(n, false),
            'X' => self.erase(row, col, col + n),
            'S' => self.scroll_up(n),
            'T' => self.scroll_down(n),
            'r' => {
                let (top, bottom) = (n - 1, param(params, 1, self.size().rows) - 1);
                if top < bottom && bottom < self.rows {
                    (self.top, self.bottom) = (top, bottom);
                    self.goto(0, 0);
                }
            }
            'm' => self.sgr(params),
            's' => self.saved = (row, col),
            'u' => self.goto(self.saved.0, self.saved.1),
            'n' if n == 6 => self
                .replies
                .extend(format!("\x1b[{};{}R", row + 1, col + 1).bytes()),
            'n' if n == 5 => self.replies.extend(b"\x1b[0n"),
            'c' => self.replies.extend(b"\x1b[?1;2c"),
            _ => {}
        }
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        let [code, fields @ ..] = params else { return };
        let text = String::from_utf8_lossy(&fields.join(&b';')).into_owned();
        if let Some((_, source, states)) = FOREIGN.iter().find(|(c, ..)| c == code) {
            let json: Option<serde_json::Value> = serde_json::from_str(&text).ok();
            let state = json.as_ref().and_then(|j| j.get("state")?.as_str());
            if let Some((_, ours)) = states.iter().find(|(theirs, _)| Some(*theirs) == state) {
                self.report = Some(Report {
                    state: (*ours).to_owned(),
                    note: None,
                    source: Some((*source).to_owned()),
                });
            }
            return;
        }
        if *code != b"7501".as_slice() {
            return;
        }
        let (state, note) = match text.split_once(";note=") {
            Some((state, note)) => (state, Some(note.to_owned())),
            None => (text.as_str(), None),
        };
        if let Some(state) = state.strip_prefix("state=")
            && STATES.contains(&state)
        {
            self.report = Some(Report {
                state: state.to_owned(),
                note,
                source: None,
            });
        }
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], _ignore: bool, byte: u8) {
        if !intermediates.is_empty() {
            return;
        }
        match byte {
            b'7' => self.saved = (self.row, self.col),
            b'8' => self.goto(self.saved.0, self.saved.1),
            b'D' => self.linefeed(),
            b'E' => {
                self.col = 0;
                self.linefeed();
            }
            b'M' => self.reverse_index(),
            b'c' => {
                let (limit, report) = (self.limit, self.report.take());
                *self = Self::new(self.size(), limit);
                self.report = report;
            }
            _ => {}
        }
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
    fn scrollback_bytes_are_bounded_by_whole_lines() {
        let mut s = Screen::bounded(Size { rows: 1, cols: 10 }, 100, 7);
        s.feed("é1\r\né2\r\né3\r\né4".as_bytes());
        assert_eq!(s.history(100), "é2\né3\né4");
        assert_eq!(s.grid.history_bytes, 6);
        s.feed(b"\x1b[3J");
        assert_eq!(s.grid.history_bytes, 0);
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
            note: None,
            source: None,
        };
        assert_eq!(s.take_report(), Some(working));
        assert_eq!(s.take_report(), None);
        s.feed(b"\x1b]7501;state=blocked;note=needs a; reply\x1b\\c");
        s.feed(b"\x1b]7501;state=bogus\x07\x1b]2;title\x07");
        assert_eq!(s.text(), "abc");
        let blocked = Report {
            state: "blocked".into(),
            note: Some("needs a; reply".into()),
            source: None,
        };
        assert_eq!(s.take_report(), Some(blocked));
    }

    #[test]
    fn jan_osc_9999_maps_waiting_to_blocked_and_ignores_garbage() {
        let mut s = screen(
            3,
            20,
            0,
            "\x1b]9999;{\"agentType\":\"jan\",\"state\":\"waiting\"}\x07",
        );
        let report = s.take_report().map(|r| (r.state, r.source));
        assert_eq!(report, Some(("blocked".into(), Some("jan-osc9999".into()))));
        s.feed(b"\x1b]9999;{\"state\":\x07\x1b]9999;[1]\x07\x1b]9999;{\"state\":7}\x07x");
        assert_eq!((s.take_report(), s.text()), (None, "x".into()));
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
    fn answers_cursor_position_queries() {
        let mut s = screen(5, 10, 0, "ab\x1b[6n");
        assert_eq!(s.take_replies(), b"\x1b[1;3R");
        assert!(s.take_replies().is_empty());
    }
}
