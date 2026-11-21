use std::collections::VecDeque;

use super::paint::{line_text, write_line};
use super::{Pen, Report, Size};

pub(super) fn chars(glyph: &Glyph) -> impl Iterator<Item = char> + '_ {
    glyph.iter().copied().filter(|&c| c != '\0')
}

type Glyph = [char; 4];
pub(super) const TAIL: Glyph = ['\0'; 4];
pub(super) type Cell = (Glyph, Pen);
pub(super) fn glyph(c: char) -> Glyph {
    [c, '\0', '\0', '\0']
}
pub(super) fn space() -> Cell {
    (glyph(' '), Pen::default())
}
pub(super) type Cells = Vec<Vec<Cell>>;

fn reach(line: &mut Vec<Cell>, len: usize) {
    if line.len() < len {
        line.resize(len, space());
    }
}

pub(super) struct Grid {
    pub(super) rows: usize,
    pub(super) cols: usize,
    pub(super) cells: Cells,
    pub(super) row: usize,
    pub(super) col: usize,
    pub(super) wrap_next: bool,
    pub(super) top: usize,
    pub(super) bottom: usize,
    pub(super) saved: (usize, usize),
    pub(super) history: VecDeque<u8>,
    pub(super) lens: VecDeque<usize>,
    pub(super) scrolled: u64,
    pub(super) line: String,
    pub(super) limit: (usize, usize),
    pub(super) primary: Option<(Cells, (usize, usize))>,
    pub(super) replies: Vec<u8>,
    pub(super) answered: bool,
    pub(super) reports: Vec<Report>,
    pub(super) pen: Pen,
    pub(super) paste: bool,
}

impl Grid {
    pub(super) fn new(size: Size, limit: (usize, usize)) -> Self {
        let (rows, cols) = (usize::from(size.rows.max(1)), usize::from(size.cols.max(1)));
        Self {
            rows,
            cols,
            cells: vec![Vec::new(); rows],
            row: 0,
            col: 0,
            wrap_next: false,
            top: 0,
            bottom: rows - 1,
            saved: (0, 0),
            history: VecDeque::new(),
            lens: VecDeque::new(),
            scrolled: 0,
            line: String::new(),
            limit,
            primary: None,
            replies: Vec::new(),
            answered: false,
            reports: Vec::new(),
            pen: Pen::default(),
            paste: false,
        }
    }

    pub(super) fn size(&self) -> Size {
        let clamp = |n: usize| u16::try_from(n).unwrap_or(u16::MAX);
        Size {
            rows: clamp(self.rows),
            cols: clamp(self.cols),
        }
    }

    pub(super) fn scrollback(&self) -> Vec<String> {
        let mut at = 0;
        let line = |len: &usize| {
            let line: Vec<u8> = self.history.range(at..at + len).copied().collect();
            at += len;
            String::from_utf8_lossy(&line).into_owned()
        };
        self.lens.iter().map(line).collect()
    }

    pub(super) fn visible(&self) -> Vec<String> {
        self.cells.iter().map(|r| line_text(r)).collect()
    }

    fn blank(&self) -> Vec<Cell> {
        Vec::with_capacity(self.cols)
    }

    fn push_history(&mut self, row: usize) {
        let (lines, bytes) = self.limit;
        if self.primary.is_some() || lines == 0 {
            return;
        }
        write_line(
            &mut self.line,
            self.cells.get(row).map_or(&[], Vec::as_slice),
        );
        let len = self.line.len();
        self.scrolled = self.scrolled.saturating_add(1);
        if len > bytes {
            return;
        }
        while self.lens.len() >= lines || self.history.len() + len > bytes {
            let Some(old) = self.lens.pop_front() else {
                break;
            };
            self.history.drain(..old);
        }
        self.history.extend(self.line.as_bytes());
        self.lens.push_back(len);
    }

    pub(super) fn scroll_up(&mut self, n: usize) {
        for _ in 0..n.min(self.bottom - self.top + 1) {
            if self.top == 0 {
                self.push_history(0);
            }
            if let Some(rows) = self.cells.get_mut(self.top..=self.bottom) {
                rows.rotate_left(1);
                rows.last_mut().into_iter().for_each(Vec::clear);
            }
        }
    }

    pub(super) fn scroll_down(&mut self, n: usize) {
        for _ in 0..n.min(self.bottom - self.top + 1) {
            self.cells.remove(self.bottom);
            self.cells.insert(self.top, self.blank());
        }
    }

    pub(super) fn linefeed(&mut self) {
        if self.row == self.bottom {
            self.scroll_up(1);
        } else if self.row + 1 < self.rows {
            self.row += 1;
        }
    }

    pub(super) fn reverse_index(&mut self) {
        if self.row == self.top {
            self.scroll_down(1);
        } else {
            self.row = self.row.saturating_sub(1);
        }
    }

    pub(super) fn goto(&mut self, row: usize, col: usize) {
        self.row = row.min(self.rows - 1);
        self.col = col.min(self.cols - 1);
        self.wrap_next = false;
    }

    pub(super) fn erase(&mut self, row: usize, from: usize, to: usize) {
        let Some(line) = self.cells.get_mut(row) else {
            return;
        };
        if to >= line.len() {
            line.truncate(from);
        } else if let Some(cells) = line.get_mut(from..to) {
            cells.fill(space());
        }
    }

    pub(super) fn erase_display(&mut self, mode: u16) {
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
                    self.lens.clear();
                }
            }
            _ => {}
        }
    }

    pub(super) fn lines_at_cursor(&mut self, n: usize, insert: bool) {
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

    pub(super) fn chars_at_cursor(&mut self, n: usize, insert: bool) {
        let (col, cols) = (self.col, self.cols);
        let Some(line) = self.cells.get_mut(self.row) else {
            return;
        };
        reach(line, cols);
        let Some(line) = line.get_mut(col..) else {
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

    pub(super) fn set_alt(&mut self, on: bool) {
        if on && self.primary.is_none() {
            let blank = vec![self.blank(); self.rows];
            let cells = std::mem::replace(&mut self.cells, blank);
            self.primary = Some((cells, (self.row, self.col)));
        } else if !on && let Some((cells, (row, col))) = self.primary.take() {
            self.cells = cells;
            self.goto(row, col);
        }
    }

    pub(super) fn resize(&mut self, size: Size) {
        let (rows, cols) = (usize::from(size.rows.max(1)), usize::from(size.cols.max(1)));
        let fit = |cells: &mut Cells| {
            for line in cells.iter_mut() {
                line.truncate(cols);
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
            self.push_history(0);
            self.cells.remove(0);
            self.row = self.row.saturating_sub(1);
        }
        self.cells.resize(rows, Vec::new());
        if let Some((cells, _)) = &mut self.primary {
            fit(cells);
            cells.resize(rows, Vec::new());
        }
        self.rows = rows;
        self.top = 0;
        self.bottom = rows - 1;
        self.goto(self.row, self.col);
    }

    pub(super) fn reply(&mut self, bytes: &[u8]) {
        self.replies.extend_from_slice(bytes);
        self.answered = true;
    }

    pub(super) fn put(&mut self, c: char, span: usize) {
        let (col, pen) = (self.col, self.pen);
        let Some(line) = self.cells.get_mut(self.row) else {
            return;
        };
        if span == 1 && line.len() == col {
            return line.push((glyph(c), pen));
        }
        if line.get(col).is_some_and(|c| c.0 == TAIL)
            && let Some(head) = line.get_mut(col.saturating_sub(1))
        {
            *head = space();
        }
        if let Some(tail) = line.get_mut(col + span).filter(|c| c.0 == TAIL) {
            *tail = space();
        }
        reach(line, col + span);
        for (i, cell) in line.iter_mut().skip(col).take(span).enumerate() {
            *cell = (if i == 0 { glyph(c) } else { TAIL }, pen);
        }
    }

    pub(super) fn combine(&mut self, c: char) {
        let Some(mut col) = self.col.checked_sub(usize::from(!self.wrap_next)) else {
            return;
        };
        let Some(line) = self.cells.get_mut(self.row) else {
            return;
        };
        if line.get(col).is_some_and(|c| c.0 == TAIL) {
            col = col.saturating_sub(1);
        }
        reach(line, col + 1);
        let cell = line.get_mut(col);
        if let Some(slot) = cell.and_then(|c| c.0.iter_mut().find(|s| **s == '\0')) {
            *slot = c;
        }
    }
}
