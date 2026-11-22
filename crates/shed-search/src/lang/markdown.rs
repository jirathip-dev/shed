use shed_proto::{Kind, Symbol};

use super::Row;

pub(super) fn outline(rows: &[Row<'_>]) -> Vec<Symbol> {
    let heads: Vec<(usize, usize, String)> = rows
        .iter()
        .enumerate()
        .filter_map(|(i, r)| {
            let level = r.text.chars().take_while(|&c| c == '#').count();
            let rest = r.text.get(level..)?;
            let heading =
                !r.skipped && r.indent <= 3 && (1..=6).contains(&level) && rest.starts_with(' ');
            heading.then(|| {
                (
                    i,
                    level,
                    rest.trim().trim_end_matches('#').trim().to_owned(),
                )
            })
        })
        .collect();
    let mut open: Vec<usize> = Vec::new();
    let mut out = Vec::with_capacity(heads.len());
    for (n, (at, level, name)) in heads.iter().enumerate() {
        let next = heads
            .iter()
            .skip(n.saturating_add(1))
            .find(|h| h.1 <= *level);
        let next = next.map_or(rows.len(), |h| h.0);
        let filled = |e: &usize| *e == *at || rows.get(*e).is_some_and(|r| !r.text.is_empty());
        let end = (*at..next).rev().find(filled).unwrap_or(*at);
        while open.last().is_some_and(|l| l >= level) {
            open.pop();
        }
        out.push(Symbol {
            kind: Kind::Section,
            name: name.clone(),
            start: at.saturating_add(1),
            end: end.saturating_add(1),
            depth: open.len(),
        });
        open.push(*level);
    }
    out
}
