use shed_proto::{FileMatches, GrepLine, GrepParams, GrepResult, Group, Symbol};

use crate::matcher::{Hit, Matcher, text};
use crate::walk::{Tree, walk};
use crate::{Error, Limit, Limits, Over, lang};

/// The lines under `p.root` that match, grouped by the symbols around them, files in name order.
///
/// # Errors
/// [`Error::Query`] for a pattern that does not compile, [`Error::Io`] for a missing root,
/// [`Error::Limit`] over `limits` or once `expired` is true; nothing is returned then.
pub fn grep(
    p: &GrepParams,
    limits: &Limits,
    expired: &dyn Fn() -> bool,
) -> Result<GrepResult, Error> {
    let matcher = Matcher::new(p)?;
    let (mut files, mut rows) = (Vec::new(), 0);
    let tree = Tree {
        root: &p.root,
        hidden: p.hidden,
        globs: &p.glob,
        limits,
        expired,
    };
    let searched = walk(&tree, &mut |rel, hay| {
        let over = |limit, max| {
            Error::Limit(Over {
                limit,
                max,
                at: rel.to_owned(),
            })
        };
        let deadline = || over(Limit::Deadline, limits.deadline.as_secs());
        let hits = matcher.lines(hay, expired).ok_or_else(deadline)?;
        if hits.is_empty() {
            return Ok(());
        }
        rows += if p.count { 1 } else { hits.len() };
        if rows > limits.matches {
            return Err(over(
                Limit::Matches,
                u64::try_from(limits.matches).unwrap_or(u64::MAX),
            ));
        }
        let lang = lang::of(rel);
        let count = hits.len();
        let groups = if p.count {
            Vec::new()
        } else {
            let symbols = lang::outline(lang, &String::from_utf8_lossy(hay));
            group(&symbols, hay, hits, limits.line)
        };
        files.push(FileMatches {
            path: rel.to_owned(),
            lang,
            count,
            groups,
        });
        Ok(())
    })?;
    Ok(GrepResult {
        matches: files.iter().map(|f| f.count).sum(),
        files,
        searched,
    })
}

fn group(symbols: &[Symbol], hay: &[u8], hits: Vec<Hit>, max: usize) -> Vec<Group> {
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
