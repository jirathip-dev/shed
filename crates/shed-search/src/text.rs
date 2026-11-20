//! A result as plain text.

use shed_proto::{OutlineResult, Symbol};

/// An outline: the file, its language, lines and symbol count, then a symbol a line, indented by depth.
#[must_use]
pub fn outline(r: &OutlineResult) -> String {
    let count = match r.symbols.len() {
        1 => "1 symbol".to_owned(),
        n => format!("{n} symbols"),
    };
    let mut out = format!(
        "{} ({}, {} lines, {count})\n",
        r.path,
        r.lang.name(),
        r.lines
    );
    for s in &r.symbols {
        out.push_str(&"  ".repeat(s.depth.saturating_add(1)));
        out.push_str(&label(s));
        out.push('\n');
    }
    out
}

fn label(s: &Symbol) -> String {
    format!("{} {} [{}-{}]", s.kind.name(), s.name, s.start, s.end)
}
