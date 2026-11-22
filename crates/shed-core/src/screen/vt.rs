use unicode_width::UnicodeWidthChar as _;
use vte::{Params, Perform};

use super::grid::Grid;
use super::report::{FOREIGN, PROBE_REPLY, STATES, program_status};
use super::{Color, Pen, Report};

impl Grid {
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
        let width = c.width().unwrap_or(1);
        if width == 0 {
            return self.combine(c);
        }
        if self.wrap_next || (width > 1 && self.col > 0 && self.col + width > self.cols) {
            self.col = 0;
            self.linefeed();
            self.wrap_next = false;
        }
        let span = width.min(self.cols - self.col);
        self.put(c, span);
        if self.col + span == self.cols {
            self.col = self.cols - 1;
            self.wrap_next = true;
        } else {
            self.col += span;
        }
    }

    fn terminated(&self) -> bool {
        self.answered
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
            if matches!(action, 'h' | 'l') && params.iter().any(|p| p.first() == Some(&2004)) {
                self.paste = action == 'h';
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
            'n' if n == 6 => self.reply(format!("\x1b[{};{}R", row + 1, col + 1).as_bytes()),
            'n' if n == 5 => self.reply(b"\x1b[0n"),
            'c' => self.reply(b"\x1b[?1;2c"),
            _ => {}
        }
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], bell_terminated: bool) {
        let [code, fields @ ..] = params else { return };
        let text = String::from_utf8_lossy(&fields.join(&b';')).into_owned();
        if let Some((_, source, states)) = FOREIGN.iter().find(|(c, ..)| c == code) {
            let json: Option<serde_json::Value> = serde_json::from_str(&text).ok();
            let state = json.as_ref().and_then(|j| j.get("state")?.as_str());
            if let Some((_, ours)) = states.iter().find(|(theirs, _)| Some(*theirs) == state) {
                self.reports.push(Report {
                    state: (*ours).to_owned(),
                    source: Some((*source).to_owned()),
                    ..Report::default()
                });
            }
            return;
        }
        if *code != b"7501".as_slice() {
            return;
        }
        if text == "?" {
            let st: &[u8] = if bell_terminated { b"\x07" } else { b"\x1b\\" };
            self.reply(&[PROBE_REPLY, st].concat());
        } else if let Some((state, note)) = text.split_once(";note=") {
            if let Some(state) = state.strip_prefix("state=")
                && STATES.contains(&state)
            {
                self.reports.push(Report {
                    state: state.to_owned(),
                    note: Some(note.to_owned()),
                    ..Report::default()
                });
            }
        } else if text.len() <= 4096
            && let Some(report) = program_status(&text)
        {
            self.reports.push(report);
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
                let (limit, reports) = (self.limit, std::mem::take(&mut self.reports));
                *self = Self::new(self.size(), limit);
                self.reports = reports;
            }
            _ => {}
        }
    }
}
