use std::fmt::Write as _;

use super::grid::{Cell, Cells, chars, glyph, space};
use super::{Color, Pen, Run};

pub(super) fn sgr(pen: &Pen) -> String {
    let color = |c: Option<Color>, base: u8| match c {
        None => String::new(),
        Some(Color::Index(i)) => format!(";{base};5;{i}"),
        Some(Color::Rgb(r, g, b)) => format!(";{base};2;{r};{g};{b}"),
    };
    let flags = [(pen.bold, ";1"), (pen.italic, ";3"), (pen.underline, ";4")];
    let flags = flags.into_iter().chain([(pen.inverse, ";7")]);
    let on: String = flags.filter(|(on, _)| *on).map(|(_, f)| f).collect();
    format!("\x1b[0{on}{}{}m", color(pen.fg, 38), color(pen.bg, 48))
}

pub(super) fn paint(out: &mut String, cells: &Cells) {
    let mut pen = None;
    for (r, row) in cells.iter().enumerate() {
        let end = row.iter().rposition(|c| *c != space()).map_or(0, |i| i + 1);
        if end > 0 {
            write!(out, "\x1b[{};1H", r + 1).ok();
        }
        for (glyph, p) in row.iter().take(end) {
            if pen != Some(*p) {
                out.push_str(&sgr(p));
                pen = Some(*p);
            }
            out.extend(chars(glyph));
        }
    }
}

pub(super) fn trim_blank_tail(lines: &mut Vec<String>) {
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
}

pub(super) fn join(mut lines: Vec<String>) -> String {
    trim_blank_tail(&mut lines);
    lines.join("\n")
}
pub(super) fn line_text(row: &[Cell]) -> String {
    let mut s = String::new();
    write_line(&mut s, row);
    s
}

pub(super) fn write_line(s: &mut String, row: &[Cell]) {
    let end = row
        .iter()
        .rposition(|c| c.0 != glyph(' '))
        .map_or(0, |i| i + 1);
    s.clear();
    s.reserve(end);
    for (glyph, _) in row.iter().take(end) {
        match glyph {
            [c, '\0', ..] if *c != '\0' => s.push(*c),
            _ => s.extend(chars(glyph)),
        }
    }
    s.truncate(s.trim_end().len());
}

pub(super) fn runs(row: &[Cell]) -> Vec<Run> {
    let end = row.iter().rposition(|c| *c != space()).map_or(0, |i| i + 1);
    let mut runs: Vec<Run> = Vec::new();
    for (glyph, pen) in row.iter().take(end) {
        match runs.last_mut() {
            Some(run) if run.pen == *pen => run.text.extend(chars(glyph)),
            _ => runs.push(Run {
                text: chars(glyph).collect(),
                pen: *pen,
            }),
        }
    }
    runs
}
