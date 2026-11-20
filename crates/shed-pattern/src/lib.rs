//! shed's pattern matcher, for `wait --output` and code search, on std alone.
//!
//! Supported: literals, `.`, classes `[a-z_]` and `[^…]` (`\d \w \s` inside too), `\d \w \s \D \W \S`,
//! `\n \t`, `\` before punctuation, groups `( … )` and `(?: … )`, alternation `a|b`, the counts `* + ?`,
//! `{n}`, `{n,}` and `{n,m}` up to 1,000 (a lazy `?` after one is accepted), `\b`, and `^ $` anywhere.
//! A pattern compiles to a program run as a Pike VM without captures, so one line costs at most the
//! program's size times the line's length, whatever the pattern.

use std::fmt;

/// A pattern that does not compile, or a construct shed lacks, by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "bad pattern {}", self.0)
    }
}

impl std::error::Error for Error {}

const PROGRAM_MAX: usize = 4000;
const COUNT_MAX: usize = 1000;
const DEPTH_MAX: usize = 100;
const DIGIT: &[(char, char)] = &[('0', '9')];
const WORD: &[(char, char)] = &[('a', 'z'), ('A', 'Z'), ('0', '9'), ('_', '_')];
const SPACE: &[(char, char)] = &[(' ', ' '), ('\t', '\r')];

#[derive(Debug, Clone)]
enum Node {
    Char(char),
    Any,
    Class(Vec<(char, char)>, bool),
    Start,
    End,
    Word,
    Cat(Vec<Node>),
    Alt(Vec<Node>),
    Rep(Box<Node>, usize, Option<usize>),
}

struct Parser<'a> {
    re: &'a str,
    chars: Vec<char>,
    at: usize,
    depth: usize,
}

impl Parser<'_> {
    fn bad<T>(&self, why: &str) -> Result<T, Error> {
        Err(Error(format!("{:?}: {why}", self.re)))
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn ahead(&self) -> Option<char> {
        self.chars.get(self.at + 1).copied()
    }

    fn next(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.at += 1;
        }
        c
    }

    fn eat(&mut self, c: char) -> bool {
        let hit = self.peek() == Some(c);
        if hit {
            self.at += 1;
        }
        hit
    }

    fn alt(&mut self) -> Result<Node, Error> {
        let mut arms = vec![self.cat()?];
        while self.eat('|') {
            arms.push(self.cat()?);
        }
        Ok(if arms.len() == 1 {
            Node::Cat(arms)
        } else {
            Node::Alt(arms)
        })
    }

    fn cat(&mut self) -> Result<Node, Error> {
        let mut seq = Vec::new();
        while let Some(c) = self.peek() {
            if c == '|' || c == ')' {
                break;
            }
            let atom = self.atom()?;
            seq.push(self.repeat(atom)?);
        }
        Ok(Node::Cat(seq))
    }

    fn count(&self) -> Option<(usize, Option<usize>, usize)> {
        let rest: String = self.chars.iter().skip(self.at).take(12).collect();
        let (body, _) = rest.strip_prefix('{')?.split_once('}')?;
        let num = |s: &str| s.parse::<usize>().ok();
        let (min, max) = match body.split_once(',') {
            None => (num(body)?, Some(num(body)?)),
            Some((a, "")) => (num(a)?, None),
            Some((a, b)) => (num(a)?, Some(num(b)?)),
        };
        Some((min, max, body.chars().count() + 2))
    }

    fn repeat(&mut self, atom: Node) -> Result<Node, Error> {
        let (min, max, len) = match self.peek() {
            Some('*') => (0, None, 1),
            Some('+') => (1, None, 1),
            Some('?') => (0, Some(1), 1),
            Some('{') => match self.count() {
                Some(count) => count,
                None => return Ok(atom),
            },
            _ => return Ok(atom),
        };
        self.at += len;
        if matches!(atom, Node::Start | Node::End | Node::Word) {
            return self.bad("nothing to repeat");
        }
        if min > COUNT_MAX || max.is_some_and(|max| max < min || max > COUNT_MAX) {
            return self.bad("a count over 1000 or out of order");
        }
        self.eat('?');
        if matches!(self.peek(), Some('*' | '+' | '?')) || self.count().is_some() {
            return self.bad("nothing to repeat");
        }
        Ok(Node::Rep(Box::new(atom), min, max))
    }

    fn atom(&mut self) -> Result<Node, Error> {
        let Some(c) = self.next() else {
            return self.bad("unexpected end");
        };
        Ok(match c {
            '.' => Node::Any,
            '^' => Node::Start,
            '$' => Node::End,
            '*' | '+' | '?' => return self.bad("nothing to repeat"),
            '(' => self.group()?,
            '[' => self.class()?,
            '\\' => match self.next() {
                None => return self.bad("trailing \\"),
                Some('b') => Node::Word,
                Some(e) => self.escape(e)?,
            },
            c => Node::Char(c),
        })
    }

    fn group(&mut self) -> Result<Node, Error> {
        if self.eat('?') && !self.eat(':') {
            let why = match (self.peek(), self.ahead()) {
                (Some('=' | '!'), _) => "lookahead (?= (?! is not supported",
                (Some('<'), Some('=' | '!')) => "lookbehind (?<= (?<! is not supported",
                (Some('<' | 'P'), _) => "named groups (?<name> are not supported",
                _ => "inline flags (?i) are not supported",
            };
            return self.bad(why);
        }
        self.depth += 1;
        if self.depth > DEPTH_MAX {
            return self.bad("groups nested over 100 deep");
        }
        let inner = self.alt()?;
        self.depth -= 1;
        if !self.eat(')') {
            return self.bad("unclosed (");
        }
        Ok(inner)
    }

    fn escape(&self, e: char) -> Result<Node, Error> {
        let class = |ranges: &[(char, char)], negated| Node::Class(ranges.to_vec(), negated);
        Ok(match e {
            'd' => class(DIGIT, false),
            'D' => class(DIGIT, true),
            'w' => class(WORD, false),
            'W' => class(WORD, true),
            's' => class(SPACE, false),
            'S' => class(SPACE, true),
            'n' => Node::Char('\n'),
            't' => Node::Char('\t'),
            '1'..='9' => return self.bad("backreferences \\1 are not supported"),
            e if e.is_ascii_alphanumeric() => {
                return self.bad(&format!("the escape \\{e} is not supported"));
            }
            e => Node::Char(e),
        })
    }

    fn class(&mut self) -> Result<Node, Error> {
        let negated = self.eat('^');
        let mut ranges = Vec::new();
        loop {
            let Some(a) = self.next() else {
                return self.bad("unclosed [");
            };
            match a {
                ']' if !ranges.is_empty() => break,
                '[' if self.peek() == Some(':') => {
                    return self.bad("POSIX classes [:name:] are not supported");
                }
                '\\' => {
                    let Some(e) = self.next() else {
                        return self.bad("unclosed [");
                    };
                    match self.escape(e)? {
                        Node::Class(more, false) => ranges.extend(more),
                        Node::Char(c) => ranges.push((c, c)),
                        _ => return self.bad(&format!("\\{e} inside [ ] is not supported")),
                    }
                }
                a => match (self.peek(), self.ahead()) {
                    (Some('-'), Some(b)) if b != ']' => {
                        if b < a {
                            return self.bad(&format!("the range {a}-{b} is out of order"));
                        }
                        self.at += 2;
                        ranges.push((a, b));
                    }
                    _ => ranges.push((a, a)),
                },
            }
        }
        Ok(Node::Class(ranges, negated))
    }
}

#[derive(Debug, Clone)]
enum Inst {
    Char(char),
    Any,
    Class(Vec<(char, char)>, bool),
    Start,
    End,
    Word,
    Split(usize, usize),
    Jmp(usize),
    Match,
}

fn patch(prog: &mut [Inst], at: usize, inst: Inst) {
    if let Some(slot) = prog.get_mut(at) {
        *slot = inst;
    }
}

fn emit(node: &Node, prog: &mut Vec<Inst>) {
    if prog.len() > PROGRAM_MAX {
        return;
    }
    match node {
        Node::Char(c) => prog.push(Inst::Char(*c)),
        Node::Any => prog.push(Inst::Any),
        Node::Class(ranges, negated) => prog.push(Inst::Class(ranges.clone(), *negated)),
        Node::Start => prog.push(Inst::Start),
        Node::End => prog.push(Inst::End),
        Node::Word => prog.push(Inst::Word),
        Node::Cat(seq) => seq.iter().for_each(|n| emit(n, prog)),
        Node::Alt(arms) => {
            let Some((last, rest)) = arms.split_last() else {
                return;
            };
            let mut jumps = Vec::new();
            for arm in rest {
                let split = prog.len();
                prog.push(Inst::Split(split + 1, 0));
                emit(arm, prog);
                jumps.push(prog.len());
                prog.push(Inst::Jmp(0));
                let end = prog.len();
                patch(prog, split, Inst::Split(split + 1, end));
            }
            emit(last, prog);
            let end = prog.len();
            for jump in jumps {
                patch(prog, jump, Inst::Jmp(end));
            }
        }
        Node::Rep(inner, min, max) => {
            for _ in 0..*min {
                emit(inner, prog);
            }
            if let Some(max) = max {
                let mut splits = Vec::new();
                for _ in *min..*max {
                    splits.push(prog.len());
                    prog.push(Inst::Split(0, 0));
                    emit(inner, prog);
                }
                let end = prog.len();
                for split in splits {
                    patch(prog, split, Inst::Split(split + 1, end));
                }
            } else {
                let split = prog.len();
                prog.push(Inst::Split(split + 1, 0));
                emit(inner, prog);
                prog.push(Inst::Jmp(split));
                let end = prog.len();
                patch(prog, split, Inst::Split(split + 1, end));
            }
        }
    }
}

fn flatten<'a>(node: &'a Node, out: &mut Vec<&'a Node>) {
    match node {
        Node::Cat(seq) => seq.iter().for_each(|n| flatten(n, out)),
        node => out.push(node),
    }
}

fn needle(node: &Node) -> Option<String> {
    let mut seq = Vec::new();
    flatten(node, &mut seq);
    let mut best = String::new();
    let mut run = String::new();
    for n in seq.into_iter().map(Some).chain([None]) {
        if let Some(Node::Char(c)) = n {
            run.push(*c);
        } else if run.chars().count() > best.chars().count() {
            best = std::mem::take(&mut run);
        } else {
            run.clear();
        }
    }
    (!best.is_empty()).then_some(best)
}

fn needles(node: &Node) -> Vec<String> {
    let mut seq = Vec::new();
    flatten(node, &mut seq);
    match seq.as_slice() {
        [Node::Alt(arms)] => arms
            .iter()
            .map(needle)
            .collect::<Option<Vec<_>>>()
            .unwrap_or_default(),
        _ => needle(node).into_iter().collect(),
    }
}

fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

struct Threads {
    pcs: Vec<usize>,
    seen: Vec<bool>,
    stack: Vec<usize>,
}

impl Threads {
    fn new(len: usize) -> Self {
        Self {
            pcs: Vec::new(),
            seen: vec![false; len],
            stack: Vec::new(),
        }
    }

    fn clear(&mut self) {
        self.pcs.clear();
        self.seen.fill(false);
    }
}

/// A compiled pattern; it matches a line when it matches anywhere in it.
#[derive(Debug, Clone)]
pub struct Pattern {
    prog: Vec<Inst>,
    needles: Vec<String>,
}

impl Pattern {
    /// Compiles `re`.
    ///
    /// # Errors
    /// [`Error`] for a pattern that does not parse, a count over 1,000, a program over 4,000 steps, or a
    /// construct shed lacks (backreferences, lookaround, inline flags, named groups), named.
    pub fn new(re: &str) -> Result<Self, Error> {
        let mut parser = Parser {
            re,
            chars: re.chars().collect(),
            at: 0,
            depth: 0,
        };
        let node = parser.alt()?;
        if parser.peek().is_some() {
            return parser.bad("unmatched )");
        }
        let mut prog = Vec::new();
        emit(&node, &mut prog);
        prog.push(Inst::Match);
        if prog.len() > PROGRAM_MAX {
            return parser.bad("over 4000 steps once its counts are expanded");
        }
        Ok(Self {
            prog,
            needles: needles(&node),
        })
    }

    /// The fixed text every match holds one of; empty when none is known.
    #[must_use]
    pub fn needles(&self) -> &[String] {
        &self.needles
    }

    /// Whether the pattern matches anywhere in `text`, in time linear in its length.
    #[must_use]
    pub fn is_match(&self, text: &str) -> bool {
        let mut now = Threads::new(self.prog.len());
        let mut then = Threads::new(self.prog.len());
        let mut chars = text.chars().peekable();
        let mut prev = None;
        loop {
            let cur = chars.next();
            self.add(&mut now, 0, prev, cur);
            if now.seen.last() == Some(&true) {
                return true;
            }
            let Some(c) = cur else {
                return false;
            };
            let after = chars.peek().copied();
            then.clear();
            for &pc in &now.pcs {
                let step = match self.prog.get(pc) {
                    Some(Inst::Any) => true,
                    Some(Inst::Char(want)) => c == *want,
                    Some(Inst::Class(ranges, negated)) => {
                        ranges.iter().any(|&(a, b)| (a..=b).contains(&c)) != *negated
                    }
                    _ => false,
                };
                if step {
                    self.add(&mut then, pc + 1, Some(c), after);
                }
            }
            std::mem::swap(&mut now, &mut then);
            prev = Some(c);
        }
    }

    fn add(&self, to: &mut Threads, pc: usize, prev: Option<char>, next: Option<char>) {
        to.stack.push(pc);
        while let Some(pc) = to.stack.pop() {
            match to.seen.get_mut(pc) {
                Some(seen) if !*seen => *seen = true,
                _ => continue,
            }
            let holds = match self.prog.get(pc) {
                Some(Inst::Jmp(target)) => {
                    to.stack.push(*target);
                    continue;
                }
                Some(Inst::Split(a, b)) => {
                    to.stack.extend([*b, *a]);
                    continue;
                }
                Some(Inst::Start) => prev.is_none(),
                Some(Inst::End) => next.is_none(),
                Some(Inst::Word) => prev.is_some_and(is_word) != next.is_some_and(is_word),
                _ => {
                    to.pcs.push(pc);
                    continue;
                }
            };
            if holds {
                to.stack.push(pc + 1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Pattern;
    use std::sync::mpsc;
    use std::time::Duration;

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

    #[test]
    fn matches_the_new_constructs() {
        assert!(m("TODO|FIXME|HACK", "a FIXME here") && !m("TODO|FIXME", "done"));
        assert!(m("Method::(Exec|FileRead)\\b", "Method::FileRead)"));
        assert!(!m("Method::(Exec|FileRead)\\b", "Method::FileReader"));
        assert!(m(
            "^\\s*pub (async )?fn [a-z_]+\\(&mut self",
            "    pub async fn go(&mut self)"
        ));
        assert!(m("(?:ab)+c", "xababc") && !m("^(?:ab)+c", "ac"));
        assert!(m("\\d{2,}\\.\\d+", "v10.2") && !m("\\d{2,}\\.\\d+", "v1.2"));
        assert!(m("^[0-9a-f]{4}$", "beef") && !m("^[0-9a-f]{4}$", "beefs"));
        assert!(m("^a{2,3}$", "aaa") && !m("^a{2,3}$", "aaaa") && m("^a{2}x{1,}", "aax"));
        assert!(m("[\\d_]+", "_") && m("\\D\\W\\S", "a b") && !m("\\bcat\\b", "concat"));
        assert!(m("a(b|^c)", "ab") && !m("a(b|^c)", "ac") && m("x$|^y", "yz"));
        assert!(m("a{", "a{") && m("a{x}", "a{x}") && m("a}", "a}"));
        assert!(m("a+?b", "aab") && m("(a*)*b", "b") && m("(|x)y", "y"));
    }

    #[test]
    fn names_what_it_refuses() {
        for (re, why) in [
            ("(a)\\1", "backreferences"),
            ("a(?=b)", "lookahead"),
            ("(?<=a)b", "lookbehind"),
            ("(?<x>a)", "named groups"),
            ("(?i)a", "inline flags"),
            ("[[:alpha:]]", "POSIX classes"),
            ("\\p{L}", "the escape \\p"),
            ("[\\D]", "\\D inside [ ]"),
            ("[z-a]", "out of order"),
            ("a{1001}", "over 1000"),
            ("(a{1000}){5}", "over 4000 steps"),
            ("(a", "unclosed ("),
            ("a)", "unmatched )"),
            ("a{2}{3}", "nothing to repeat"),
        ] {
            let err = Pattern::new(re).expect_err(re).to_string();
            assert!(err.contains(why), "{re}: {err}");
        }
        let deep = format!("{}a{}", "(".repeat(101), ")".repeat(101));
        let err = Pattern::new(&deep).expect_err("too deep").to_string();
        assert!(err.contains("nested over 100"), "{err}");
    }

    #[test]
    fn a_nested_count_returns_within_a_second() {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let text = format!("{}b", "a".repeat(10_000));
            let hit = Pattern::new("(a+)+$").map(|p| p.is_match(&text));
            tx.send(hit).ok();
        });
        let hit = rx.recv_timeout(Duration::from_secs(1));
        assert_eq!(hit, Ok(Ok(false)));
    }

    #[test]
    fn needles_are_the_fixed_text_every_match_holds() {
        for (re, want) in [
            ("Pattern::new", &["Pattern::new"][..]),
            ("fn \\w+_limit", &["_limit"]),
            ("TODO|FIXME|HACK", &["TODO", "FIXME", "HACK"]),
            ("(TODO|FIXME)", &["TODO", "FIXME"]),
            ("Method::(Exec|FileRead)\\b", &["Method::"]),
            ("^\\s*pub (async )?fn [a-z_]+", &["pub "]),
            ("a(bc)d", &["abcd"]),
            ("TODO|\\d+", &[]),
            ("[0-9a-f]{40}", &[]),
            ("", &[]),
        ] {
            let p = Pattern::new(re).expect("valid pattern");
            assert_eq!(p.needles(), want, "{re}");
        }
    }
}
