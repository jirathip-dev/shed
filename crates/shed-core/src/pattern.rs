//! A small regular-expression matcher for `wait --output`, so shed needs no regex crate.
//!
//! Supported: literals, `.`, classes `[a-z_]` and `[^…]`, `\d \w \s`, `\` escapes, the quantifiers
//! `* + ?` and the anchors `^ $`. No groups or alternation.

use crate::Error;

#[derive(Debug, Clone)]
enum Atom {
    Any,
    Char(char),
    Class(Vec<(char, char)>, bool),
}

impl Atom {
    fn matches(&self, c: char) -> bool {
        match self {
            Self::Any => true,
            Self::Char(want) => c == *want,
            Self::Class(ranges, negated) => {
                ranges.iter().any(|&(a, b)| (a..=b).contains(&c)) != *negated
            }
        }
    }
}

/// A compiled pattern; it matches a string when it matches anywhere in it.
#[derive(Debug, Clone)]
pub struct Pattern {
    start: bool,
    end: bool,
    atoms: Vec<(Atom, usize, usize)>,
}

fn escape(c: char) -> Atom {
    let class = |ranges: &[(char, char)]| Atom::Class(ranges.to_vec(), false);
    match c {
        'd' => class(&[('0', '9')]),
        'w' => class(&[('a', 'z'), ('A', 'Z'), ('0', '9'), ('_', '_')]),
        's' => class(&[(' ', ' '), ('\t', '\r')]),
        c => Atom::Char(c),
    }
}

impl Pattern {
    /// Compiles `re`.
    ///
    /// # Errors
    /// [`Error::Pattern`] for a dangling `\`, an unclosed `[`, or a quantifier with nothing before it.
    pub fn new(re: &str) -> Result<Self, Error> {
        let bad = |why: &str| Error::Pattern(format!("{re:?}: {why}"));
        let mut chars: Vec<char> = re.chars().collect();
        let start = chars.first() == Some(&'^');
        let end = chars.last() == Some(&'$') && !re.ends_with("\\$");
        if end {
            chars.pop();
        }
        let mut atoms: Vec<(Atom, usize, usize)> = Vec::new();
        let mut it = chars.into_iter().skip(usize::from(start)).peekable();
        while let Some(c) = it.next() {
            let atom = match c {
                '.' => Atom::Any,
                '\\' => escape(it.next().ok_or_else(|| bad("trailing \\"))?),
                '[' => {
                    let negated = it.next_if_eq(&'^').is_some();
                    let mut ranges = Vec::new();
                    loop {
                        let a = match it.next().ok_or_else(|| bad("unclosed ["))? {
                            ']' if !ranges.is_empty() => break,
                            '\\' => it.next().ok_or_else(|| bad("unclosed ["))?,
                            a => a,
                        };
                        let b = match it.peek() {
                            Some('-') => {
                                it.next();
                                match it.next().ok_or_else(|| bad("unclosed ["))? {
                                    ']' => {
                                        ranges.extend([(a, a), ('-', '-')]);
                                        break;
                                    }
                                    b => b,
                                }
                            }
                            _ => a,
                        };
                        ranges.push((a, b));
                    }
                    Atom::Class(ranges, negated)
                }
                '*' | '+' | '?' => {
                    let last = atoms.last_mut().filter(|a| (a.1, a.2) == (1, 1));
                    let last = last.ok_or_else(|| bad("nothing to repeat"))?;
                    (last.1, last.2) = match c {
                        '*' => (0, usize::MAX),
                        '+' => (1, usize::MAX),
                        _ => (0, 1),
                    };
                    continue;
                }
                c => Atom::Char(c),
            };
            atoms.push((atom, 1, 1));
        }
        Ok(Self { start, end, atoms })
    }

    /// Whether the pattern matches anywhere in `text`.
    #[must_use]
    pub fn is_match(&self, text: &str) -> bool {
        let chars: Vec<char> = text.chars().collect();
        let starts = if self.start { 0..=0 } else { 0..=chars.len() };
        starts
            .into_iter()
            .any(|i| chars.get(i..).is_some_and(|t| self.at(&self.atoms, t)))
    }

    fn at(&self, atoms: &[(Atom, usize, usize)], text: &[char]) -> bool {
        let Some(((atom, min, max), rest)) = atoms.split_first() else {
            return !self.end || text.is_empty();
        };
        let run = text
            .iter()
            .take(*max)
            .take_while(|&&c| atom.matches(c))
            .count();
        (*min..=run)
            .rev()
            .any(|n| text.get(n..).is_some_and(|t| self.at(rest, t)))
    }
}

#[cfg(test)]
mod tests {
    use super::Pattern;

    fn m(re: &str, text: &str) -> bool {
        Pattern::new(re).expect("valid pattern").is_match(text)
    }

    #[test]
    fn matches_the_supported_subset() {
        assert!(m("ok", "all ok here"));
        assert!(!m("^ok", "all ok"));
        assert!(m("^ok \\d+ tests$", "ok 12 tests"));
        assert!(!m("^ok \\d+ tests$", "ok 12 tests!"));
        assert!(m("a.c", "abc") && !m("a.c", "ac"));
        assert!(m("colou?r", "color") && m("colou?r", "colour"));
        assert!(m("x*y", "y") && m("^\\$ $", "$ "));
        assert!(m("[a-c]+[^0-9]$", "zzbcax"));
        assert!(!m("[a-c]+[^0-9]$", "bca1"));
        assert!(m("[-a]", "-") && m("[a-]", "-") && m("\\[x\\]", "[x]"));
        assert!(m("", "anything"));
    }

    #[test]
    fn rejects_broken_patterns() {
        for re in ["*a", "a\\", "[ab", "a**"] {
            assert!(Pattern::new(re).is_err(), "{re}");
        }
    }
}
