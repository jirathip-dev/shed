//! A query to the matched lines of one file: needles first, the engine only on lines that hold one.

use std::borrow::Cow;
use std::ops::Range;

use shed_pattern::{Flags, Pattern};
use shed_proto::GrepParams;

use crate::Error;
use crate::needle::Needle;

const CHECK_EVERY: usize = 1024;

pub(crate) struct Matcher {
    pattern: Pattern,
    needles: Vec<Needle>,
    literal: bool,
}

pub(crate) struct Hit {
    pub(crate) n: usize,
    pub(crate) bytes: Range<usize>,
}

impl Matcher {
    pub(crate) fn new(p: &GrepParams) -> Result<Self, Error> {
        if p.pattern.is_empty() {
            return Err(Error::Query("an empty pattern matches every line".into()));
        }
        let flags = Flags {
            ignore_case: p.ignore_case,
            word: p.word,
            fixed: p.fixed,
        };
        let pattern = Pattern::with(&p.pattern, flags).map_err(|e| Error::Query(e.to_string()))?;
        let texts = pattern.needles();
        let usable = !p.ignore_case || texts.iter().all(|t| t.is_ascii());
        let needles = if usable {
            texts
                .iter()
                .map(|t| Needle::new(t, p.ignore_case))
                .collect()
        } else {
            Vec::new()
        };
        Ok(Self {
            literal: pattern.is_literal() && !p.word && usable,
            pattern,
            needles,
        })
    }

    pub(crate) fn lines(&self, hay: &[u8], expired: &dyn Fn() -> bool) -> Option<Vec<Hit>> {
        let mut found: Vec<Range<usize>> = Vec::new();
        if self.needles.is_empty() {
            let mut start = 0;
            for (i, line) in hay.split(|&b| b == b'\n').enumerate() {
                if line.is_empty() && start == hay.len() {
                    break;
                }
                if i % CHECK_EVERY == CHECK_EVERY - 1 && expired() {
                    return None;
                }
                found.push(start..start + line.len());
                start += line.len() + 1;
            }
        } else {
            for needle in &self.needles {
                let mut from = 0;
                while let Some(at) = needle.find(hay, from) {
                    let line = bounds(hay, at);
                    from = line.end + 1;
                    if at + needle.len() <= line.end {
                        found.push(line);
                    }
                }
            }
            found.sort_unstable_by_key(|r| r.start);
            found.dedup();
        }
        let mut hits = Vec::new();
        let (mut n, mut counted) = (1, 0);
        for (i, line) in found.into_iter().enumerate() {
            if i % CHECK_EVERY == CHECK_EVERY - 1 && expired() {
                return None;
            }
            let bytes = trim_cr(hay, line);
            if !self.literal && !self.pattern.is_match(&text(hay, &bytes)) {
                continue;
            }
            n += hay.get(counted..bytes.start).map_or(0, |gap| {
                gap.iter().fold(0, |n, &b| n + usize::from(b == b'\n'))
            });
            counted = bytes.start;
            hits.push(Hit { n, bytes });
        }
        Some(hits)
    }
}

pub(crate) fn text<'h>(hay: &'h [u8], bytes: &Range<usize>) -> Cow<'h, str> {
    String::from_utf8_lossy(hay.get(bytes.clone()).unwrap_or_default())
}

fn bounds(hay: &[u8], at: usize) -> Range<usize> {
    let before = hay.get(..at).unwrap_or_default();
    let after = hay.get(at..).unwrap_or_default();
    let start = before
        .iter()
        .rposition(|&b| b == b'\n')
        .map_or(0, |i| i + 1);
    let end = after
        .iter()
        .position(|&b| b == b'\n')
        .map_or(hay.len(), |i| at + i);
    start..end
}

fn trim_cr(hay: &[u8], line: Range<usize>) -> Range<usize> {
    let cr = line.end > line.start && hay.get(line.end - 1) == Some(&b'\r');
    line.start..line.end - usize::from(cr)
}
