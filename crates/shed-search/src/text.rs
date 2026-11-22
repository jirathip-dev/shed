//! A result as plain text.

use std::fmt::Write as _;

use shed_proto::{FindResult, GrepResult, OutlineResult, Symbol};

/// Grep: each file, its groups under their symbols, its lines; with `count` a file and its count a line.
#[must_use]
pub fn grep(r: &GrepResult) -> String {
    let mut out = String::new();
    for f in &r.files {
        if f.groups.is_empty() {
            writeln!(out, "{}: {}", f.path, f.count).ok();
            continue;
        }
        writeln!(out, "{}", f.path).ok();
        for g in &f.groups {
            let names: Vec<String> = g
                .scope
                .iter()
                .map(|s| format!("{} {}", s.kind.name(), s.name))
                .collect();
            match g.scope.last() {
                Some(s) => writeln!(out, "  {} [{}-{}]", names.join(" > "), s.start, s.end),
                None => writeln!(out, "  (top level)"),
            }
            .ok();
            for l in &g.lines {
                let clipped = l
                    .length
                    .map(|n| format!(" … ({n} bytes)"))
                    .unwrap_or_default();
                writeln!(out, "    {}: {}{clipped}", l.n, l.text.trim_end()).ok();
            }
        }
    }
    let found = plural(r.matches, "match", "matches");
    let files = plural(r.files.len(), "file", "files");
    let searched = plural(r.searched, "file", "files");
    writeln!(out, "{found} in {files}, {searched} searched").ok();
    out
}

/// Find: each file ranked, its reasons and its symbols, then how many of how many.
#[must_use]
pub fn find(r: &FindResult) -> String {
    let mut out = String::new();
    for (n, f) in r.files.iter().enumerate() {
        let (path, lang, lines) = (&f.path, f.lang.name(), f.lines);
        writeln!(out, "{}. {path} ({lang}, {lines} lines)", n + 1).ok();
        writeln!(out, "     why: {}", f.why.join("; ")).ok();
        for s in &f.symbols {
            writeln!(out, "     {}", label(s)).ok();
        }
    }
    let files = plural(r.total, "file", "files");
    let searched = r.searched;
    writeln!(out, "{} of {files}, {searched} searched", r.files.len()).ok();
    out
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

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
