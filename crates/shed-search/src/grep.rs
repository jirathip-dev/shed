use shed_proto::{GrepLine, Group, Symbol};

use crate::matcher::{Hit, text};

pub(crate) fn group(symbols: &[Symbol], hay: &[u8], hits: Vec<Hit>, max: usize) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    for hit in hits {
        let line = clip(
            text(hay, &hit.bytes).into_owned(),
            hit.n,
            hit.bytes.len(),
            max,
        );
        let scope: Vec<Symbol> = symbols
            .iter()
            .filter(|s| s.start <= hit.n && hit.n <= s.end)
            .cloned()
            .collect();
        match groups.iter_mut().find(|g| g.scope.last() == scope.last()) {
            Some(g) => g.lines.push(line),
            None => groups.push(Group {
                scope,
                lines: vec![line],
            }),
        }
    }
    groups
}

fn clip(mut text: String, n: usize, length: usize, max: usize) -> GrepLine {
    let clipped = length > max;
    if clipped {
        text.truncate(text.floor_char_boundary(max));
    }
    GrepLine {
        n,
        text,
        clipped,
        length: clipped.then_some(length),
    }
}
