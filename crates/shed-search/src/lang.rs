//! Outlines by line rules: one header rule per language, one shared rule for a symbol's range.

mod markdown;
mod python;
mod rust;
mod script;

use shed_proto::{Kind, Lang, Symbol};

/// The language of a path, by its extension.
#[must_use]
pub fn of(path: &str) -> Lang {
    match path.rsplit_once('.').map_or("", |(_, ext)| ext) {
        "rs" => Lang::Rust,
        "ts" | "tsx" | "mts" | "cts" => Lang::Typescript,
        "js" | "jsx" | "mjs" | "cjs" => Lang::Javascript,
        "py" | "pyi" => Lang::Python,
        "md" | "markdown" => Lang::Markdown,
        "toml" => Lang::Toml,
        _ => Lang::Text,
    }
}

struct Row<'a> {
    indent: usize,
    text: &'a str,
    skipped: bool,
}

struct Open {
    end: usize,
    kind: Kind,
    head: usize,
}

/// A text's symbols in file order; pure, text in and symbols out.
#[must_use]
pub fn outline(lang: Lang, text: &str) -> Vec<Symbol> {
    let rows = rows(lang, text);
    if lang == Lang::Markdown {
        return markdown::outline(&rows);
    }
    let mut out: Vec<(Symbol, usize)> = Vec::new();
    let mut open: Vec<Open> = Vec::new();
    let mut section: Option<usize> = None;
    for (i, r) in rows.iter().enumerate() {
        while open.last().is_some_and(|o| o.end < i) {
            open.pop();
        }
        if r.text.is_empty() || r.skipped {
            continue;
        }
        let parent = open.last();
        let nests = |k| matches!(k, Kind::Impl | Kind::Trait | Kind::Mod | Kind::Class);
        if parent.is_some_and(|o| !nests(o.kind)) {
            continue;
        }
        let head = match lang {
            Lang::Rust => rust::head(r.text).filter(|_| !r.text.starts_with("//")),
            Lang::Typescript | Lang::Javascript => {
                let class = parent.filter(|o| o.kind == Kind::Class);
                let member = class.is_some_and(|o| {
                    let mut between = rows.iter().take(i).skip(o.head.saturating_add(1));
                    between.all(|l| l.text.is_empty() || l.indent >= r.indent)
                });
                let fits = class.is_none() || member;
                script::head(r.text, member).filter(|_| fits && !comment(lang, r.text))
            }
            Lang::Python => python::head(r.text),
            Lang::Toml => table(r),
            Lang::Markdown | Lang::Text => block(&rows, i),
        };
        let Some((kind, name)) = head else { continue };
        if kind == Kind::Section {
            if let Some(prev) = section.replace(out.len()) {
                let mut above = rows.iter().take(i);
                let end = above.rposition(|l| !l.text.is_empty() && !comment(lang, l.text));
                if let Some((s, _)) = out.get_mut(prev) {
                    s.end = end.map_or(i, |l| l.saturating_add(1));
                }
            }
            let (start, end) = (i.saturating_add(1), rows.len());
            out.push((
                Symbol {
                    kind,
                    name,
                    start,
                    end,
                    depth: 0,
                },
                start,
            ));
            continue;
        }
        if section.is_some() {
            continue;
        }
        let end = block_end(lang, &rows, i);
        let floor = out
            .last()
            .map_or(0, |(s, head)| if s.end <= i { s.end } else { *head });
        let mut start = i;
        while start > floor
            && rows
                .get(start.saturating_sub(1))
                .is_some_and(|l| !l.text.is_empty() && comment(lang, l.text))
        {
            start = start.saturating_sub(1);
        }
        let depth = open.len();
        let (start, head) = (start.saturating_add(1), i.saturating_add(1));
        let end1 = end.saturating_add(1);
        out.push((
            Symbol {
                kind,
                name,
                start,
                end: end1,
                depth,
            },
            head,
        ));
        open.push(Open { end, kind, head: i });
    }
    if let Some((s, _)) = section.and_then(|last| out.get_mut(last)) {
        s.end = rows
            .iter()
            .rposition(|l| !l.text.is_empty())
            .map_or(1, |l| l.saturating_add(1));
    }
    out.into_iter().map(|(s, _)| s).collect()
}

fn rows(lang: Lang, text: &str) -> Vec<Row<'_>> {
    let mut inside = false;
    text.lines()
        .map(|line| {
            let text = line.trim();
            let toggles = match lang {
                Lang::Markdown => usize::from(text.starts_with("```") || text.starts_with("~~~")),
                Lang::Python => text.matches("\"\"\"").count() + text.matches("'''").count(),
                _ => 0,
            };
            let skipped = inside || (lang == Lang::Markdown && toggles > 0);
            if toggles % 2 == 1 {
                inside = !inside;
            }
            let indent = line.len() - line.trim_start().len();
            Row {
                indent,
                text,
                skipped,
            }
        })
        .collect()
}

fn comment(lang: Lang, t: &str) -> bool {
    let marks: &[&str] = match lang {
        Lang::Rust => &["//", "#[", "#!["],
        Lang::Typescript | Lang::Javascript => &["//", "/*", "*", "@"],
        Lang::Python => &["#", "@"],
        Lang::Markdown => &[],
        Lang::Toml => &["#"],
        Lang::Text => &["#", "//", "/*", "*", "--", ";"],
    };
    marks.iter().any(|m| t.starts_with(m))
}

fn block_end(lang: Lang, rows: &[Row<'_>], at: usize) -> usize {
    let Some(&Row {
        indent, text: head, ..
    }) = rows.get(at)
    else {
        return at;
    };
    let py = lang == Lang::Python;
    let one_line = head.ends_with(';')
        || (head.contains('{') && head.trim_end_matches([',', ';']).ends_with('}'));
    if !py && one_line {
        return at;
    }
    let (mut last, mut body) = (at, false);
    let mut bracketed = !py && head.ends_with(['{', '(', '[']);
    for (i, r) in rows.iter().enumerate().skip(at.saturating_add(1)) {
        let t = r.text;
        if r.skipped {
            last = i;
        }
        if t.is_empty() || r.skipped {
            continue;
        }
        if r.indent > indent {
            (last, body) = (i, true);
            continue;
        }
        let closer = t.starts_with(['}', ')', ']']);
        let opener = t.ends_with(['{', '(', '[']) || (py && t.ends_with(':'));
        if closer && (opener || (py && !body)) {
            last = i;
            continue;
        }
        if closer && !py {
            let next = rows
                .iter()
                .skip(i.saturating_add(1))
                .find(|l| !l.text.is_empty());
            let goes_on = |l: &Row<'_>| l.text == "{" || l.text.starts_with("where");
            if next.is_some_and(|l| l.indent == indent && goes_on(l)) {
                last = i;
                continue;
            }
            return i;
        }
        if !py && (t == "{" || bracketed || (!body && t.starts_with("where"))) {
            bracketed |= t == "{";
            last = i;
            continue;
        }
        if py && body && r.indent == indent && comment(lang, t) && !t.starts_with('@') {
            continue;
        }
        break;
    }
    while last > at
        && rows
            .get(last)
            .is_some_and(|r| r.indent <= indent && comment(lang, r.text))
    {
        last = last.saturating_sub(1);
    }
    last
}

fn table(r: &Row<'_>) -> Option<(Kind, String)> {
    let t = r.text;
    let name = t.trim_matches(['[', ']']).to_owned();
    (r.indent == 0 && t.starts_with('[') && t.ends_with(']')).then_some((Kind::Section, name))
}

fn block(rows: &[Row<'_>], at: usize) -> Option<(Kind, String)> {
    let r = rows.get(at)?;
    let named = r
        .text
        .starts_with(|c: char| c.is_alphanumeric() || c == '_' || c == '"');
    if r.indent > 0 || comment(Lang::Text, r.text) || !named {
        return None;
    }
    let next = rows
        .iter()
        .skip(at.saturating_add(1))
        .find(|l| !l.text.is_empty() && l.text != "{");
    if next.is_none_or(|l| l.indent == 0) {
        return None;
    }
    let name: String = r
        .text
        .trim_end_matches(['{', ':', '(', '[', '=', ' '])
        .chars()
        .take(80)
        .collect();
    (!name.is_empty()).then_some((Kind::Block, name))
}

fn ident(s: &str) -> Option<String> {
    let name: String = s
        .chars()
        .take_while(|&c| c.is_alphanumeric() || c == '_' || c == '$')
        .collect();
    (!name.is_empty()).then_some(name)
}

fn strip_any<'a>(mut s: &'a str, words: &[&str]) -> &'a str {
    while let Some(rest) = words.iter().find_map(|w| s.strip_prefix(w)) {
        s = rest.trim_start();
    }
    s
}
