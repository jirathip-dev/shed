//! `.gitignore` rules and globs, on paths relative to the folder a rule file is in.

pub(crate) struct Rule {
    base: String,
    pub(crate) negated: bool,
    dir_only: bool,
    anchored: bool,
    segments: Vec<Vec<char>>,
}

pub(crate) fn parse(text: &str, base: &str, out: &mut Vec<Rule>) {
    for line in text.lines() {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let negated = line.starts_with('!');
        let line = line.strip_prefix('!').unwrap_or(line);
        let line = line
            .strip_prefix('\\')
            .filter(|l| l.starts_with(['#', '!']))
            .unwrap_or(line);
        let dir_only = line.ends_with('/');
        let line = line.strip_suffix('/').unwrap_or(line);
        let anchored = line.contains('/');
        let line = line.strip_prefix('/').unwrap_or(line);
        out.push(Rule {
            base: base.to_owned(),
            negated,
            dir_only,
            anchored,
            segments: line.split('/').map(|s| s.chars().collect()).collect(),
        });
    }
}

pub(crate) fn ignored(rules: &[Rule], rel: &str, is_dir: bool) -> bool {
    last(rules, rel, is_dir).is_some_and(|r| !r.negated)
}

pub(crate) fn last<'r>(rules: &'r [Rule], rel: &str, is_dir: bool) -> Option<&'r Rule> {
    rules.iter().rev().find(|r| r.matches(rel, is_dir))
}

impl Rule {
    pub(crate) fn matches(&self, rel: &str, is_dir: bool) -> bool {
        if self.dir_only && !is_dir {
            return false;
        }
        let Some(rest) = rel.strip_prefix(&self.base) else {
            return false;
        };
        let parts: Vec<Vec<char>> = rest.split('/').map(|s| s.chars().collect()).collect();
        if self.anchored {
            return path(&self.segments, &parts);
        }
        match (self.segments.first(), parts.last()) {
            (Some(rule), Some(name)) => wild(rule, name),
            _ => false,
        }
    }
}

fn path(segments: &[Vec<char>], parts: &[Vec<char>]) -> bool {
    let Some((first, rest)) = segments.split_first() else {
        return parts.is_empty();
    };
    if first.as_slice() == ['*', '*'] {
        if rest.is_empty() {
            return !parts.is_empty();
        }
        return (0..=parts.len()).any(|skip| parts.get(skip..).is_some_and(|p| path(rest, p)));
    }
    parts
        .split_first()
        .is_some_and(|(part, more)| wild(first, part) && path(rest, more))
}

fn wild(pat: &[char], text: &[char]) -> bool {
    let (mut p, mut t) = (0, 0);
    let mut star = None;
    while let Some(&c) = text.get(t) {
        let step = match pat.get(p) {
            Some('*') => {
                star = Some((p, t));
                p += 1;
                continue;
            }
            Some('?') => Some(p + 1),
            Some('[') => match set(pat.get(p..).unwrap_or_default(), c) {
                Some((hit, used)) => hit.then_some(p + used),
                None => (c == '[').then_some(p + 1),
            },
            Some('\\') if pat.get(p + 1).is_some() => (pat.get(p + 1) == Some(&c)).then_some(p + 2),
            Some(&w) => (w == c).then_some(p + 1),
            None => None,
        };
        match (step, star) {
            (Some(next), _) => (p, t) = (next, t + 1),
            (None, Some((sp, st))) => {
                (p, t) = (sp + 1, st + 1);
                star = Some((sp, st + 1));
            }
            (None, None) => return false,
        }
    }
    pat.iter().skip(p).all(|&c| c == '*')
}

fn set(pat: &[char], c: char) -> Option<(bool, usize)> {
    let negated = matches!(pat.get(1), Some('!' | '^'));
    let first = 1 + usize::from(negated);
    let (mut i, mut hit) = (first, false);
    loop {
        let a = *pat.get(i)?;
        if a == ']' && i > first {
            return Some((hit != negated, i + 1));
        }
        match (pat.get(i + 1), pat.get(i + 2)) {
            (Some('-'), Some(&b)) if b != ']' => {
                hit |= (a..=b).contains(&c);
                i += 3;
            }
            _ => {
                hit |= a == c;
                i += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Rule, ignored, parse};

    fn rules(text: &str, base: &str) -> Vec<Rule> {
        let mut out = Vec::new();
        parse(text, base, &mut out);
        out
    }

    #[test]
    fn rules_follow_the_gitignore_manual() {
        for (rule, path, dir, want) in [
            ("*.o", "a/b/c.o", false, true),
            ("*.o", "c.oo", false, false),
            ("/build", "build", true, true),
            ("/build", "src/build", true, false),
            ("build/", "src/build", true, true),
            ("build/", "build", false, false),
            ("doc/frotz", "doc/frotz", true, true),
            ("doc/frotz", "a/doc/frotz", true, false),
            ("**/foo", "a/b/foo", false, true),
            ("**/foo", "foo", false, true),
            ("abc/**", "abc/x/y", false, true),
            ("abc/**", "abc", true, false),
            ("a/**/b", "a/b", false, true),
            ("a/**/b", "a/x/y/b", false, true),
            ("a?c", "abc", false, true),
            ("[a-c]x", "bx", false, true),
            ("[!a-c]x", "bx", false, false),
            ("\\#hash", "#hash", false, true),
            ("# comment", "# comment", false, false),
            ("\\!bang", "!bang", false, true),
            ("x\\*", "x*", false, true),
            ("x\\*", "xy", false, false),
        ] {
            assert_eq!(ignored(&rules(rule, ""), path, dir), want, "{rule} {path}");
        }
    }

    #[test]
    fn the_last_match_wins_and_negation_keeps_again() {
        let r = rules("*.log\n!keep.log\n", "");
        assert!(ignored(&r, "a.log", false));
        assert!(!ignored(&r, "keep.log", false));
        let mut r = rules("*.log\n", "");
        parse("!*.log\n", "sub/", &mut r);
        assert!(!ignored(&r, "sub/a.log", false) && ignored(&r, "a.log", false));
        assert!(!ignored(&rules("/x", "sub/"), "x", false));
        assert!(ignored(&rules("/x", "sub/"), "sub/x", false));
    }
}
