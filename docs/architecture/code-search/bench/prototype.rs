use std::fmt::Write as _;
use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

struct Limits {
    files: usize,
    matches: usize,
    symbols: usize,
    bytes: u64,
    line: usize,
    time: Duration,
}

const FIXED: Limits = Limits {
    files: 20_000,
    matches: 250,
    symbols: 2_000,
    bytes: 256 << 20,
    line: 4096,
    time: Duration::from_secs(10),
};

const OFF: Limits = Limits {
    files: usize::MAX,
    matches: usize::MAX,
    symbols: usize::MAX,
    bytes: u64::MAX,
    line: usize::MAX,
    time: Duration::from_secs(3600),
};

static UNLIMITED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn limits() -> &'static Limits {
    if UNLIMITED.load(std::sync::atomic::Ordering::Relaxed) {
        &OFF
    } else {
        &FIXED
    }
}

#[derive(Debug)]
enum Error {
    Limit(&'static str, u64, String),
    Pattern(String),
    Io(String),
}

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
}

impl Parser<'_> {
    fn bad<T>(&self, why: &str) -> Result<T, Error> {
        Err(Error::Pattern(format!("{:?}: {why}", self.re)))
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn eat(&mut self, c: char) -> bool {
        let hit = self.peek() == Some(c);
        self.at += usize::from(hit);
        hit
    }

    fn alt(&mut self) -> Result<Node, Error> {
        let mut arms = vec![self.cat()?];
        while self.eat('|') {
            arms.push(self.cat()?);
        }
        Ok(if arms.len() == 1 { arms.remove(0) } else { Node::Alt(arms) })
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

    fn count(&mut self) -> Option<(usize, Option<usize>)> {
        let rest: String = self.chars[self.at..].iter().take(12).collect();
        let body = rest.strip_prefix('{')?;
        let (body, _) = body.split_once('}')?;
        let (min, max) = match body.split_once(',') {
            None => (body.parse().ok()?, Some(body.parse().ok()?)),
            Some((a, "")) => (a.parse().ok()?, None),
            Some((a, b)) => (a.parse().ok()?, Some(b.parse().ok()?)),
        };
        self.at += body.chars().count() + 2;
        Some((min, max))
    }

    fn repeat(&mut self, atom: Node) -> Result<Node, Error> {
        let (min, max) = match self.peek() {
            Some('*') => (0, None),
            Some('+') => (1, None),
            Some('?') => (0, Some(1)),
            Some('{') => match self.count() {
                Some(range) => {
                    self.at -= 1;
                    range
                }
                None => return Ok(atom),
            },
            _ => return Ok(atom),
        };
        self.at += 1;
        if matches!(atom, Node::Start | Node::End | Node::Word) {
            return self.bad("nothing to repeat");
        }
        if max.is_some_and(|max| max < min || max > 1000) || min > 1000 {
            return self.bad("a count over 1000 or out of order");
        }
        self.eat('?');
        if matches!(self.peek(), Some('*' | '+')) {
            return self.bad("nothing to repeat");
        }
        Ok(Node::Rep(Box::new(atom), min, max))
    }

    fn atom(&mut self) -> Result<Node, Error> {
        let Some(c) = self.peek() else {
            return self.bad("unexpected end");
        };
        self.at += 1;
        Ok(match c {
            '.' => Node::Any,
            '^' => Node::Start,
            '$' => Node::End,
            '*' | '+' | '?' => return self.bad("nothing to repeat"),
            '(' => {
                if self.eat('?') && !self.eat(':') {
                    return self.bad("only (?:…) groups; no lookaround, flags or names");
                }
                let inner = self.alt()?;
                if !self.eat(')') {
                    return self.bad("unclosed (");
                }
                inner
            }
            '[' => self.class()?,
            '\\' => {
                let Some(e) = self.peek() else {
                    return self.bad("trailing \\");
                };
                self.at += 1;
                match e {
                    'b' => Node::Word,
                    '1'..='9' => return self.bad("no backreferences"),
                    _ => escape(e),
                }
            }
            c => Node::Char(c),
        })
    }

    fn class(&mut self) -> Result<Node, Error> {
        let negated = self.eat('^');
        let mut ranges = Vec::new();
        loop {
            let Some(a) = self.peek() else {
                return self.bad("unclosed [");
            };
            self.at += 1;
            if a == ']' && !ranges.is_empty() {
                break;
            }
            if a == '\\' {
                let Some(e) = self.peek() else {
                    return self.bad("unclosed [");
                };
                self.at += 1;
                match escape(e) {
                    Node::Class(more, false) => ranges.extend(more),
                    _ => ranges.push((e, e)),
                }
                continue;
            }
            if self.peek() == Some('-') && self.chars.get(self.at + 1).is_some_and(|&b| b != ']') {
                let b = self.chars[self.at + 1];
                self.at += 2;
                ranges.push((a, b));
            } else {
                ranges.push((a, a));
            }
        }
        Ok(Node::Class(ranges, negated))
    }
}

fn escape(c: char) -> Node {
    let class = |ranges: &[(char, char)], negated| Node::Class(ranges.to_vec(), negated);
    let word = [('a', 'z'), ('A', 'Z'), ('0', '9'), ('_', '_')];
    match c {
        'd' => class(&[('0', '9')], false),
        'D' => class(&[('0', '9')], true),
        'w' => class(&word, false),
        'W' => class(&word, true),
        's' => class(&[(' ', ' '), ('\t', '\r')], false),
        'S' => class(&[(' ', ' '), ('\t', '\r')], true),
        'n' => Node::Char('\n'),
        't' => Node::Char('\t'),
        c => Node::Char(c),
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

const PROGRAM_MAX: usize = 4000;

struct Pattern {
    prog: Vec<Inst>,
    fold: bool,
    literals: Vec<String>,
}

fn emit(node: &Node, prog: &mut Vec<Inst>) {
    if prog.len() > PROGRAM_MAX {
        return;
    }
    match node {
        Node::Char(c) => prog.push(Inst::Char(*c)),
        Node::Any => prog.push(Inst::Any),
        Node::Class(r, n) => prog.push(Inst::Class(r.clone(), *n)),
        Node::Start => prog.push(Inst::Start),
        Node::End => prog.push(Inst::End),
        Node::Word => prog.push(Inst::Word),
        Node::Cat(seq) => seq.iter().for_each(|n| emit(n, prog)),
        Node::Alt(arms) => {
            let mut jumps = Vec::new();
            for (i, arm) in arms.iter().enumerate() {
                if i + 1 == arms.len() {
                    emit(arm, prog);
                    break;
                }
                let split = prog.len();
                prog.push(Inst::Split(split + 1, 0));
                emit(arm, prog);
                jumps.push(prog.len());
                prog.push(Inst::Jmp(0));
                prog[split] = Inst::Split(split + 1, prog.len());
            }
            let end = prog.len();
            for j in jumps {
                prog[j] = Inst::Jmp(end);
            }
        }
        Node::Rep(inner, min, max) => {
            for _ in 0..*min {
                emit(inner, prog);
            }
            match max {
                None => {
                    let split = prog.len();
                    prog.push(Inst::Split(split + 1, 0));
                    emit(inner, prog);
                    prog.push(Inst::Jmp(split));
                    prog[split] = Inst::Split(split + 1, prog.len());
                }
                Some(max) => {
                    let mut splits = Vec::new();
                    for _ in *min..*max {
                        splits.push(prog.len());
                        prog.push(Inst::Split(0, 0));
                        emit(inner, prog);
                    }
                    let end = prog.len();
                    for s in splits {
                        prog[s] = Inst::Split(s + 1, end);
                    }
                }
            }
        }
    }
}

fn literal_of(node: &Node) -> Option<String> {
    let Node::Cat(seq) = node else {
        return match node {
            Node::Char(c) => Some(c.to_string()),
            _ => None,
        };
    };
    let mut best = String::new();
    let mut run = String::new();
    for n in seq.iter().chain([&Node::Any]) {
        match n {
            Node::Char(c) => run.push(*c),
            Node::Cat(_) if literal_of(n).is_some() && run.is_empty() => {
                run = literal_of(n).unwrap_or_default();
                if run.len() > best.len() {
                    best = std::mem::take(&mut run);
                }
                run.clear();
            }
            _ => {
                if run.len() > best.len() {
                    best = std::mem::take(&mut run);
                }
                run.clear();
            }
        }
    }
    (!best.is_empty()).then_some(best)
}

fn literals_of(node: &Node) -> Vec<String> {
    let arms = match node {
        Node::Alt(arms) => arms.iter().map(literal_of).collect::<Option<Vec<_>>>(),
        Node::Cat(seq) if seq.len() == 1 => return literals_of(&seq[0]),
        node => literal_of(node).map(|l| vec![l]),
    };
    arms.unwrap_or_default()
}

fn plain(node: &Node) -> bool {
    match node {
        Node::Char(_) => true,
        Node::Cat(seq) => seq.iter().all(|n| matches!(n, Node::Char(_))),
        _ => false,
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn lower(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

impl Pattern {
    fn new(re: &str, fold: bool, word: bool) -> Result<(Self, bool), Error> {
        let mut p = Parser { re, chars: re.chars().collect(), at: 0 };
        let mut node = p.alt()?;
        if p.at < p.chars.len() {
            return p.bad("unmatched )");
        }
        let is_plain = plain(&node);
        let literals = literals_of(&node);
        if word {
            node = Node::Cat(vec![Node::Word, node, Node::Word]);
        }
        let mut prog = Vec::new();
        emit(&node, &mut prog);
        prog.push(Inst::Match);
        if prog.len() > PROGRAM_MAX {
            return p.bad("too large once counts are expanded");
        }
        Ok((Self { prog, fold, literals }, is_plain))
    }

    fn add(&self, list: &mut Vec<usize>, scratch: &mut Scratch, pc: usize, prev: Option<char>, next: Option<char>) {
        scratch.stack.push(pc);
        while let Some(pc) = scratch.stack.pop() {
            if scratch.seen[pc] == scratch.mark {
                continue;
            }
            scratch.seen[pc] = scratch.mark;
            match &self.prog[pc] {
                Inst::Jmp(to) => scratch.stack.push(*to),
                Inst::Split(a, b) => {
                    scratch.stack.push(*b);
                    scratch.stack.push(*a);
                }
                Inst::Start => {
                    if prev.is_none() {
                        scratch.stack.push(pc + 1);
                    }
                }
                Inst::End => {
                    if next.is_none() {
                        scratch.stack.push(pc + 1);
                    }
                }
                Inst::Word => {
                    if prev.is_some_and(is_word) != next.is_some_and(is_word) {
                        scratch.stack.push(pc + 1);
                    }
                }
                _ => list.push(pc),
            }
        }
    }

    fn is_match(&self, text: &str, scratch: &mut Scratch) -> bool {
        scratch.seen.resize(self.prog.len(), 0);
        let (mut now, mut then) = (std::mem::take(&mut scratch.now), std::mem::take(&mut scratch.then));
        now.clear();
        let mut chars = text.chars().peekable();
        let mut prev = None;
        let hit = loop {
            let next = chars.next();
            scratch.mark += 1;
            self.add(&mut now, scratch, 0, prev, next);
            let Some(c) = next else {
                break now.iter().any(|&pc| matches!(self.prog[pc], Inst::Match));
            };
            let after = chars.peek().copied();
            scratch.mark += 1;
            then.clear();
            let mut done = false;
            for &pc in &now {
                let hit = match &self.prog[pc] {
                    Inst::Match => {
                        done = true;
                        break;
                    }
                    Inst::Any => true,
                    Inst::Char(want) => c == *want || (self.fold && lower(c) == lower(*want)),
                    Inst::Class(ranges, negated) => {
                        let has = |c: char| ranges.iter().any(|&(a, b)| (a..=b).contains(&c));
                        let up = c.to_uppercase().next().unwrap_or(c);
                        (has(c) || (self.fold && (has(lower(c)) || has(up)))) != *negated
                    }
                    _ => false,
                };
                if hit {
                    self.add(&mut then, scratch, pc + 1, Some(c), after);
                }
            }
            if done {
                break true;
            }
            std::mem::swap(&mut now, &mut then);
            prev = Some(c);
        };
        (scratch.now, scratch.then) = (now, then);
        hit
    }
}

#[derive(Default)]
struct Scratch {
    seen: Vec<usize>,
    now: Vec<usize>,
    then: Vec<usize>,
    stack: Vec<usize>,
    mark: usize,
}

fn line_match(pattern: &Pattern, line: &[u8], scratch: &mut Scratch) -> bool {
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    match std::str::from_utf8(line) {
        Ok(text) => pattern.is_match(text, scratch),
        Err(_) => pattern.is_match(&String::from_utf8_lossy(line), scratch),
    }
}

struct Needle {
    bytes: Vec<u8>,
    skip: [usize; 256],
    fold: bool,
}

impl Needle {
    fn new(text: &str, fold: bool) -> Self {
        let fold = fold && text.is_ascii();
        let bytes: Vec<u8> = if fold { text.to_ascii_lowercase().into_bytes() } else { text.as_bytes().to_vec() };
        let mut skip = [bytes.len(); 256];
        for (i, &b) in bytes.iter().enumerate().take(bytes.len() - 1) {
            skip[usize::from(b)] = bytes.len() - 1 - i;
            if fold {
                skip[usize::from(b.to_ascii_uppercase())] = bytes.len() - 1 - i;
            }
        }
        Self { bytes, skip, fold }
    }

    fn find(&self, hay: &[u8], from: usize) -> Option<usize> {
        let m = self.bytes.len();
        let last = self.bytes[m - 1];
        let mut end = from + m - 1;
        while end < hay.len() {
            let c = hay[end];
            let c_cmp = if self.fold { c.to_ascii_lowercase() } else { c };
            if c_cmp == last {
                let window = &hay[end + 1 - m..end];
                let same = if self.fold {
                    window.iter().zip(&self.bytes).all(|(a, b)| a.to_ascii_lowercase() == *b)
                } else {
                    window == &self.bytes[..m - 1]
                };
                if same {
                    return Some(end + 1 - m);
                }
            }
            end += self.skip[usize::from(c)];
        }
        None
    }
}

enum Matcher {
    Literal(Needle, bool),
    Pattern(Pattern, Vec<Needle>),
}

impl Matcher {
    fn new(query: &str, fixed: bool, fold: bool, word: bool) -> Result<Self, Error> {
        if query.is_empty() {
            return Err(Error::Pattern("an empty pattern matches every line".into()));
        }
        if fixed && (!fold || query.is_ascii()) {
            return Ok(Self::Literal(Needle::new(query, fold), word));
        }
        let source = if fixed { quote(query) } else { query.to_owned() };
        let (pattern, plain) = Pattern::new(&source, fold, word)?;
        let ascii = !fold || pattern.literals.iter().all(|l| l.is_ascii());
        if plain && ascii {
            let literal = pattern.literals.first().cloned().unwrap_or_default();
            return Ok(Self::Literal(Needle::new(&literal, fold), word));
        }
        let needles = if ascii { pattern.literals.iter().map(|l| Needle::new(l, fold)).collect() } else { Vec::new() };
        Ok(Self::Pattern(pattern, needles))
    }

    fn lines(&self, hay: &[u8], mut hit: impl FnMut(usize, &[u8]) -> Result<(), Error>) -> Result<(), Error> {
        let mut found: Vec<(usize, usize)> = Vec::new();
        let mut scratch = Scratch::default();
        let bounds = |at: usize| {
            let start = hay[..at].iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
            let end = hay[at..].iter().position(|&b| b == b'\n').map_or(hay.len(), |i| at + i);
            (start, end)
        };
        match self {
            Self::Literal(needle, word) => {
                let mut from = 0;
                while let Some(at) = needle.find(hay, from) {
                    let (start, end) = bounds(at);
                    let after = at + needle.bytes.len();
                    let edge = |b: Option<&u8>| b.is_none_or(|b| !(b.is_ascii_alphanumeric() || *b == b'_' || *b >= 0x80));
                    if !*word || (edge(at.checked_sub(1).and_then(|i| hay.get(i))) && edge(hay.get(after))) {
                        found.push((start, end));
                        from = end + 1;
                    } else {
                        from = at + 1;
                    }
                }
            }
            Self::Pattern(pattern, needles) if needles.is_empty() => {
                let mut start = 0;
                for line in hay.split(|&b| b == b'\n') {
                    if line_match(pattern, line, &mut scratch) {
                        found.push((start, start + line.len()));
                    }
                    start += line.len() + 1;
                }
            }
            Self::Pattern(pattern, needles) => {
                for needle in needles {
                    let mut from = 0;
                    while let Some(at) = needle.find(hay, from) {
                        let (start, end) = bounds(at);
                        found.push((start, end));
                        from = end + 1;
                    }
                }
                found.sort_unstable();
                found.dedup();
                found.retain(|&(start, end)| line_match(pattern, &hay[start..end], &mut scratch));
            }
        }
        let (mut line, mut counted) = (1, 0);
        for (start, end) in found {
            line += hay[counted..start].iter().filter(|&&b| b == b'\n').count();
            counted = start;
            hit(line, &hay[start..end])?;
        }
        Ok(())
    }
}

fn quote(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if "\\.^$*+?()[]{}|".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn text_of(line: &[u8]) -> String {
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    String::from_utf8_lossy(line).into_owned()
}

fn wild(pat: &[char], text: &[char]) -> bool {
    let (mut p, mut t) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while t < text.len() {
        let step = match pat.get(p) {
            Some('*') => {
                star = Some((p, t));
                p += 1;
                continue;
            }
            Some('?') => Some(p + 1),
            Some('[') => match set(&pat[p..], text[t]) {
                Some((true, used)) => Some(p + used),
                Some((false, _)) => None,
                None => (text[t] == '[').then_some(p + 1),
            },
            Some('\\') if p + 1 < pat.len() => (pat[p + 1] == text[t]).then_some(p + 2),
            Some(&c) => (c == text[t]).then_some(p + 1),
            None => None,
        };
        match (step, star) {
            (Some(next), _) => {
                p = next;
                t += 1;
            }
            (None, Some((sp, st))) => {
                p = sp + 1;
                t = st + 1;
                star = Some((sp, st + 1));
            }
            (None, None) => return false,
        }
    }
    pat[p..].iter().all(|&c| c == '*')
}

fn set(pat: &[char], c: char) -> Option<(bool, usize)> {
    let negated = matches!(pat.get(1), Some('!' | '^'));
    let mut i = 1 + usize::from(negated);
    let mut hit = false;
    let first = i;
    loop {
        let a = *pat.get(i)?;
        if a == ']' && i > first {
            return Some((hit != negated, i + 1));
        }
        if pat.get(i + 1) == Some(&'-') && pat.get(i + 2).is_some_and(|&b| b != ']') {
            hit |= (a..=pat[i + 2]).contains(&c);
            i += 3;
        } else {
            hit |= a == c;
            i += 1;
        }
    }
}

struct Rule {
    base: String,
    negated: bool,
    dir_only: bool,
    anchored: bool,
    segments: Vec<Vec<char>>,
}

fn rules(text: &str, base: &str, out: &mut Vec<Rule>) {
    for line in text.lines() {
        let mut line = line.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let negated = line.starts_with('!');
        line = line.strip_prefix('!').unwrap_or(line);
        line = line.strip_prefix('\\').filter(|l| l.starts_with(['#', '!'])).unwrap_or(line);
        let dir_only = line.ends_with('/');
        line = line.strip_suffix('/').unwrap_or(line);
        let anchored = line.contains('/');
        line = line.strip_prefix('/').unwrap_or(line);
        let segments = line.split('/').map(|s| s.chars().collect()).collect();
        out.push(Rule { base: base.to_owned(), negated, dir_only, anchored, segments });
    }
}

fn path_match(segments: &[Vec<char>], parts: &[Vec<char>]) -> bool {
    match segments.split_first() {
        None => parts.is_empty(),
        Some((first, rest)) if first.iter().collect::<String>() == "**" => {
            if rest.is_empty() {
                return !parts.is_empty();
            }
            (0..=parts.len()).any(|skip| path_match(rest, &parts[skip..]))
        }
        Some((first, rest)) => parts.split_first().is_some_and(|(part, more)| wild(first, part) && path_match(rest, more)),
    }
}

impl Rule {
    fn matches(&self, rel: &str, is_dir: bool) -> bool {
        if self.dir_only && !is_dir {
            return false;
        }
        let Some(rest) = rel.strip_prefix(&self.base) else {
            return false;
        };
        let parts: Vec<Vec<char>> = rest.split('/').map(|s| s.chars().collect()).collect();
        if self.anchored {
            path_match(&self.segments, &parts)
        } else {
            parts.last().is_some_and(|name| wild(&self.segments[0], name))
        }
    }
}

fn ignored(rules: &[Rule], rel: &str, is_dir: bool) -> bool {
    rules.iter().rev().find(|r| r.matches(rel, is_dir)).is_some_and(|r| !r.negated)
}

struct Walk {
    hidden: bool,
    globs: Vec<Rule>,
    rules: Vec<Rule>,
    started: Instant,
    files: usize,
    bytes: u64,
    binary: usize,
}

impl Walk {
    fn new(root: &Path, hidden: bool, globs: &[String]) -> (Self, String) {
        let mut walk = Self { hidden, globs: Vec::new(), rules: Vec::new(), started: Instant::now(), files: 0, bytes: 0, binary: 0 };
        for g in globs {
            rules(g, "", &mut walk.globs);
        }
        let Some(top) = root.ancestors().find(|d| d.join(".git").exists()) else {
            return (walk, String::new());
        };
        let git = top.join(".git");
        let git_dir = match fs::read_to_string(&git) {
            Ok(file) => {
                let dir = top.join(file.trim().strip_prefix("gitdir: ").unwrap_or(""));
                let common = fs::read_to_string(dir.join("commondir")).unwrap_or_default();
                dir.join(common.trim())
            }
            Err(_) => git,
        };
        if let Ok(text) = fs::read_to_string(git_dir.join("info/exclude")) {
            rules(&text, "", &mut walk.rules);
        }
        let prefix = root.strip_prefix(top).unwrap_or(Path::new(""));
        let mut base = String::new();
        let mut dir = top.to_path_buf();
        for part in prefix.components() {
            if let Ok(text) = fs::read_to_string(dir.join(".gitignore")) {
                rules(&text, &base, &mut walk.rules);
            }
            dir.push(part);
            base.push_str(&part.as_os_str().to_string_lossy());
            base.push('/');
        }
        (walk, base)
    }

    fn wanted(&self, rel: &str) -> bool {
        let includes = self.globs.iter().any(|g| !g.negated);
        let hit = self.globs.iter().rev().find(|g| g.matches(rel, false));
        hit.map_or(!includes, |g| !g.negated)
    }

    fn check(&self) -> Result<(), Error> {
        if self.started.elapsed() > limits().time {
            return Err(Error::Limit("deadline", limits().time.as_secs(), String::new()));
        }
        Ok(())
    }

    fn dir(&mut self, dir: &Path, top_rel: &str, rel: &str, visit: &mut dyn FnMut(&mut Self, &str, Vec<u8>) -> Result<(), Error>) -> Result<(), Error> {
        self.check()?;
        let kept = self.rules.len();
        if let Ok(text) = fs::read_to_string(dir.join(".gitignore")) {
            rules(&text, top_rel, &mut self.rules);
        }
        let mut entries: Vec<_> = fs::read_dir(dir).map_err(|e| Error::Io(format!("{}: {e}", dir.display())))?.flatten().collect();
        entries.sort_by_key(fs::DirEntry::file_name);
        for entry in entries {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(kind) = entry.file_type() else { continue };
            if name == ".git" || (name.starts_with('.') && !self.hidden) || kind.is_symlink() {
                continue;
            }
            let in_top = format!("{top_rel}{name}");
            let in_root = format!("{rel}{name}");
            if ignored(&self.rules, &in_top, kind.is_dir()) {
                continue;
            }
            if kind.is_dir() {
                if self.globs.iter().any(|g| g.negated && g.matches(&in_root, true)) {
                    continue;
                }
                self.dir(&entry.path(), &format!("{in_top}/"), &format!("{in_root}/"), visit)?;
            } else if kind.is_file() && self.wanted(&in_root) {
                self.file(&entry.path(), &in_root, visit)?;
            }
        }
        self.rules.truncate(kept);
        Ok(())
    }

    fn file(&mut self, path: &Path, rel: &str, visit: &mut dyn FnMut(&mut Self, &str, Vec<u8>) -> Result<(), Error>) -> Result<(), Error> {
        self.check()?;
        self.files += 1;
        if self.files > limits().files {
            return Err(Error::Limit("files", limits().files as u64, rel.to_owned()));
        }
        let Ok(mut file) = fs::File::open(path) else { return Ok(()) };
        let size = file.metadata().map_or(0, |m| m.len());
        self.bytes += size;
        if self.bytes > limits().bytes {
            return Err(Error::Limit("bytes", limits().bytes, rel.to_owned()));
        }
        let mut bytes = Vec::with_capacity(size as usize + 1);
        if file.read_to_end(&mut bytes).is_err() {
            return Ok(());
        }
        if bytes.iter().take(8192).any(|&b| b == 0) {
            self.binary += 1;
            return Ok(());
        }
        visit(self, rel, bytes)
    }
}

fn walk(root: &Path, hidden: bool, globs: &[String], visit: &mut dyn FnMut(&mut Walk, &str, Vec<u8>) -> Result<(), Error>) -> Result<Walk, Error> {
    let root = fs::canonicalize(root).map_err(|e| Error::Io(format!("{}: {e}", root.display())))?;
    if root.is_file() {
        let mut w = Walk::new(root.parent().unwrap_or(&root), hidden, globs).0;
        let name = root.file_name().unwrap_or_default().to_string_lossy().into_owned();
        w.file(&root, &name, visit)?;
        return Ok(w);
    }
    let (mut w, top_rel) = Walk::new(&root, hidden, globs);
    w.dir(&root, &top_rel, "", visit)?;
    Ok(w)
}

#[derive(Clone, Copy, PartialEq)]
enum Lang {
    Rust,
    Script,
    Python,
    Markdown,
    Toml,
    Other,
}

impl Lang {
    fn of(path: &str) -> Self {
        match path.rsplit_once('.').map_or("", |(_, ext)| ext) {
            "rs" => Self::Rust,
            "ts" | "tsx" | "mts" | "cts" | "js" | "jsx" | "mjs" | "cjs" => Self::Script,
            "py" | "pyi" => Self::Python,
            "md" | "markdown" => Self::Markdown,
            "toml" => Self::Toml,
            _ => Self::Other,
        }
    }

    fn name(self, path: &str) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Script if path.contains(".ts") || path.ends_with(".mts") || path.ends_with(".cts") => "typescript",
            Self::Script => "javascript",
            Self::Python => "python",
            Self::Markdown => "markdown",
            Self::Toml => "toml",
            Self::Other => "text",
        }
    }
}

#[derive(Clone)]
struct Symbol {
    kind: &'static str,
    name: String,
    start: usize,
    end: usize,
    depth: usize,
    head: usize,
}

fn ident(s: &str) -> Option<String> {
    let name: String = s.chars().take_while(|&c| c.is_alphanumeric() || c == '_' || c == '$').collect();
    (!name.is_empty()).then_some(name)
}

fn strip_any<'a>(s: &'a str, words: &[&str]) -> &'a str {
    let mut s = s;
    while let Some(rest) = words.iter().find_map(|w| s.strip_prefix(w)) {
        s = rest.trim_start();
    }
    s
}

fn rust_head(t: &str) -> Option<(&'static str, String)> {
    let mut s = t;
    loop {
        let before = s;
        if s.starts_with("pub(") {
            s = s.split_once(')')?.1.trim_start();
        }
        s = strip_any(s, &["pub ", "async ", "unsafe ", "default "]);
        if let Some(rest) = s.strip_prefix("extern \"") {
            s = rest.split_once('"')?.1.trim_start();
        }
        if let Some(rest) = s.strip_prefix("const ") {
            if ["fn ", "unsafe ", "async ", "extern "].iter().any(|k| rest.starts_with(k)) {
                s = rest;
            }
        }
        if s == before {
            break;
        }
    }
    let kinds = [("fn ", "fn"), ("struct ", "struct"), ("union ", "struct"), ("enum ", "enum"), ("trait ", "trait"), ("type ", "type"), ("const ", "const"), ("static mut ", "const"), ("static ", "const"), ("macro_rules! ", "macro")];
    if let Some((rest, kind)) = kinds.iter().find_map(|(word, kind)| Some((s.strip_prefix(word)?, *kind))) {
        return Some((kind, ident(rest)?));
    }
    if let Some(rest) = s.strip_prefix("mod ") {
        return (!t.ends_with(';')).then_some(("mod", ident(rest)?));
    }
    let rest = s.strip_prefix("impl")?;
    if !rest.starts_with([' ', '<']) {
        return None;
    }
    let mut rest = rest.trim_start();
    if rest.starts_with('<') {
        let mut depth = 0;
        let close = rest.char_indices().find(|&(_, c)| {
            depth += i32::from(c == '<') - i32::from(c == '>');
            depth == 0
        })?;
        rest = rest[close.0 + 1..].trim_start();
    }
    let name = rest.split(" where").next()?.trim_end_matches('{').trim();
    (!name.is_empty()).then(|| ("impl", name.to_owned()))
}

fn script_head(t: &str, in_class: bool) -> Option<(&'static str, String)> {
    let s = strip_any(t, &["export ", "default ", "declare ", "abstract "]);
    if in_class {
        let s = strip_any(s, &["public ", "private ", "protected ", "static ", "readonly ", "override ", "async ", "get ", "set ", "*", "#"]);
        let name = ident(s)?;
        let rest = s[name.len()..].trim_start();
        let call = rest.starts_with(['(', '<']) && !["if", "for", "while", "switch", "catch", "return"].contains(&name.as_str());
        let arrow = rest.strip_prefix('=').is_some_and(|r| r.contains("=>") || r.trim_start().starts_with("function"));
        return (call || arrow).then_some(("fn", name));
    }
    let s = strip_any(s, &["async "]);
    if let Some(rest) = s.strip_prefix("function") {
        let rest = rest.trim_start_matches('*').trim_start();
        return Some(("fn", ident(rest).unwrap_or_else(|| "default".into())));
    }
    let kinds = [("class ", "class"), ("interface ", "interface"), ("const enum ", "enum"), ("enum ", "enum"), ("namespace ", "mod"), ("module ", "mod")];
    if let Some((rest, kind)) = kinds.iter().find_map(|(word, kind)| Some((s.strip_prefix(word)?, *kind))) {
        return Some((kind, ident(rest)?));
    }
    if let Some(rest) = s.strip_prefix("type ") {
        let name = ident(rest)?;
        return rest[name.len()..].trim_start().starts_with(['=', '<']).then_some(("type", name));
    }
    let rest = ["const ", "let ", "var "].iter().find_map(|w| s.strip_prefix(w))?;
    let name = ident(rest)?;
    let value = rest.split_once('=')?.1.trim_start();
    let func = value.contains("=>") || ["(", "async", "function", "<"].iter().any(|p| value.starts_with(p));
    Some((if func { "fn" } else { "const" }, name))
}

fn python_head(t: &str) -> Option<(&'static str, String)> {
    let s = strip_any(t, &["async "]);
    if let Some(rest) = s.strip_prefix("def ") {
        return Some(("fn", ident(rest)?));
    }
    Some(("class", ident(s.strip_prefix("class ")?)?))
}

fn comment(lang: Lang, t: &str) -> bool {
    let marks: &[&str] = match lang {
        Lang::Rust => &["//", "#[", "#!["],
        Lang::Script => &["//", "/*", "*", "@"],
        Lang::Python => &["#", "@"],
        Lang::Markdown => &[],
        Lang::Toml => &["#"],
        Lang::Other => &["#", "//", "/*", "*", "--", ";"],
    };
    marks.iter().any(|m| t.starts_with(m))
}

fn block_end(lang: Lang, lines: &[(usize, &str)], masked: &[bool], at: usize) -> usize {
    let (indent, head) = lines[at];
    if lang != Lang::Python && (head.ends_with(';') || (head.contains('{') && head.trim_end_matches([',', ';']).ends_with('}'))) {
        return at;
    }
    let mut last = at;
    let mut body = false;
    let mut opened = lang != Lang::Python && head.ends_with(['{', '(', '[']);
    for (i, &(ind, t)) in lines.iter().enumerate().skip(at + 1) {
        if t.is_empty() || masked[i] {
            if masked[i] {
                last = i;
            }
            continue;
        }
        if ind > indent {
            last = i;
            body = true;
            continue;
        }
        let closer = t.starts_with(['}', ')', ']']);
        let opener = t.ends_with(['{', '(', '[']) || (lang == Lang::Python && t.ends_with(':'));
        if closer && opener {
            last = i;
            continue;
        }
        if closer && lang != Lang::Python {
            let next = lines[i + 1..].iter().find(|l| !l.1.is_empty());
            if next.is_some_and(|l| l.0 == indent && (l.1 == "{" || l.1.starts_with("where"))) {
                last = i;
                continue;
            }
            return i;
        }
        if closer && !body {
            last = i;
            continue;
        }
        if lang != Lang::Python && (t == "{" || opened || (!body && t.starts_with("where"))) {
            opened |= t == "{";
            last = i;
            continue;
        }
        if comment(lang, t) && lang == Lang::Python && ind == indent && !t.starts_with('@') && body {
            continue;
        }
        break;
    }
    while last > at && comment(lang, lines[last].1) && lines[last].0 <= indent {
        last -= 1;
    }
    last
}

fn mask(lang: Lang, lines: &[(usize, &str)]) -> Vec<bool> {
    let mut inside = false;
    let mut out = Vec::with_capacity(lines.len());
    for &(_, t) in lines {
        let toggles = match lang {
            Lang::Markdown => usize::from(t.starts_with("```") || t.starts_with("~~~")),
            Lang::Python => t.matches("\"\"\"").count() + t.matches("'''").count(),
            _ => 0,
        };
        out.push(inside || (lang == Lang::Markdown && toggles > 0));
        if toggles % 2 == 1 {
            inside = !inside;
        }
    }
    out
}

fn markdown(lines: &[(usize, &str)], masked: &[bool]) -> Vec<Symbol> {
    let mut heads: Vec<(usize, usize, String)> = Vec::new();
    for (i, &(indent, t)) in lines.iter().enumerate() {
        let level = t.chars().take_while(|&c| c == '#').count();
        if masked[i] || indent > 3 || !(1..=6).contains(&level) || !t[level..].starts_with(' ') {
            continue;
        }
        heads.push((i, level, t[level..].trim().trim_end_matches('#').trim().to_owned()));
    }
    let mut out = Vec::new();
    let mut open: Vec<usize> = Vec::new();
    for (n, (at, level, name)) in heads.iter().enumerate() {
        let next = heads[n + 1..].iter().find(|h| h.1 <= *level).map_or(lines.len(), |h| h.0);
        let mut end = next - 1;
        while end > *at && lines[end].1.is_empty() {
            end -= 1;
        }
        while open.last().is_some_and(|&l| l >= *level) {
            open.pop();
        }
        out.push(Symbol { kind: "section", name: name.clone(), start: at + 1, end: end + 1, depth: open.len(), head: at + 1 });
        open.push(*level);
    }
    out
}

fn other_head(lines: &[(usize, &str)], at: usize) -> Option<(&'static str, String)> {
    let (indent, t) = lines[at];
    if indent > 0 || comment(Lang::Other, t) || !t.starts_with(|c: char| c.is_alphanumeric() || c == '_' || c == '"') {
        return None;
    }
    let mut after = lines[at + 1..].iter().filter(|l| !l.1.is_empty() && l.1 != "{");
    if after.next().is_none_or(|l| l.0 == 0) {
        return None;
    }
    let name = t.trim_end_matches(['{', ':', '(', '[', '=', ' ']);
    let name: String = name.chars().take(80).collect();
    (!name.is_empty()).then_some(("block", name))
}

fn outline(lang: Lang, text: &str) -> Vec<Symbol> {
    let lines: Vec<(usize, &str)> = text.lines().map(|l| (l.len() - l.trim_start().len(), l.trim())).collect();
    let masked = mask(lang, &lines);
    if lang == Lang::Markdown {
        return markdown(&lines, &masked);
    }
    let mut out: Vec<Symbol> = Vec::new();
    let mut open: Vec<(usize, &'static str, usize)> = Vec::new();
    let mut section: Option<usize> = None;
    for (i, &(indent, t)) in lines.iter().enumerate() {
        while open.last().is_some_and(|o| o.0 < i) {
            open.pop();
        }
        if t.is_empty() || masked[i] {
            continue;
        }
        let parent = open.last().map(|o| o.1);
        if parent.is_some_and(|k| !["impl", "trait", "mod", "class"].contains(&k)) {
            continue;
        }
        let head = match lang {
            Lang::Rust => rust_head(t).filter(|_| !t.starts_with("//")),
            Lang::Script => {
                let member = parent == Some("class") && open.last().is_some_and(|o| lines[o.2 + 1..i].iter().all(|l| l.1.is_empty() || l.0 >= indent));
                script_head(t, member).filter(|_| !comment(lang, t) && (parent != Some("class") || member))
            }
            Lang::Python => python_head(t),
            Lang::Toml => (indent == 0 && t.starts_with('[') && t.ends_with(']')).then(|| ("section", t.trim_matches(['[', ']']).to_owned())),
            _ => other_head(&lines, i),
        };
        let Some((kind, name)) = head else { continue };
        if kind == "section" {
            if let Some(prev) = section.replace(out.len()) {
                out[prev].end = lines[..i].iter().rposition(|l| !l.1.is_empty() && !comment(lang, l.1)).map_or(i, |l| l + 1);
            }
            out.push(Symbol { kind, name, start: i + 1, end: lines.len(), depth: 0, head: i + 1 });
            continue;
        }
        if section.is_some() {
            continue;
        }
        let end = block_end(lang, &lines, &masked, i);
        let floor = out.last().map_or(0, |s| if s.end <= i { s.end } else { s.head });
        let mut start = i;
        while start > floor && !lines[start - 1].1.is_empty() && comment(lang, lines[start - 1].1) {
            start -= 1;
        }
        out.push(Symbol { kind, name, start: start + 1, end: end + 1, depth: open.len(), head: i + 1 });
        open.push((end, kind, i));
    }
    if let Some(last) = section {
        out[last].end = lines.iter().rposition(|l| !l.1.is_empty()).map_or(1, |l| l + 1);
    }
    out
}

fn label(s: &Symbol) -> String {
    format!("{} {} [{}-{}]", s.kind, s.name, s.start, s.end)
}

fn json_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if u32::from(c) < 0x20 => _ = write!(out, "\\u{:04x}", u32::from(c)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn json_symbol(s: &Symbol) -> String {
    format!("{{\"kind\":\"{}\",\"name\":{},\"start\":{},\"end\":{},\"depth\":{}}}", s.kind, json_str(&s.name), s.start, s.end, s.depth)
}

struct Options {
    json: bool,
    fixed: bool,
    fold: bool,
    word: bool,
    count: bool,
    hidden: bool,
    flat: bool,
    top: usize,
    globs: Vec<String>,
}

struct FileHits {
    path: String,
    lang: &'static str,
    count: usize,
    groups: Vec<(Vec<Symbol>, Vec<(usize, String)>)>,
}

fn grep(root: &Path, query: &str, o: &Options) -> Result<String, Error> {
    let matcher = Matcher::new(query, o.fixed, o.fold, o.word)?;
    let mut files: Vec<FileHits> = Vec::new();
    let mut total = 0;
    let walked = walk(root, o.hidden, &o.globs, &mut |_, rel, bytes| {
        let mut hits: Vec<(usize, String)> = Vec::new();
        matcher.lines(&bytes, |line, text| {
            if !o.count && text.len() > limits().line {
                return Err(Error::Limit("line", limits().line as u64, format!("{rel}:{line}")));
            }
            hits.push((line, if o.count { String::new() } else { text_of(text) }));
            Ok(())
        })?;
        if hits.is_empty() {
            return Ok(());
        }
        total += if o.count { 1 } else { hits.len() };
        if total > limits().matches {
            return Err(Error::Limit("matches", limits().matches as u64, rel.to_owned()));
        }
        let lang = Lang::of(rel);
        let mut groups: Vec<(Vec<Symbol>, Vec<(usize, String)>)> = Vec::new();
        let count = hits.len();
        if !o.count {
            let symbols = if o.flat { Vec::new() } else { outline(lang, &String::from_utf8_lossy(&bytes)) };
            for (line, text) in hits {
                let scope: Vec<Symbol> = symbols.iter().filter(|s| s.start <= line && line <= s.end).cloned().collect();
                let key = scope.last().map(|s| s.head);
                match groups.iter_mut().find(|g| g.0.last().map(|s| s.head) == key) {
                    Some(group) => group.1.push((line, text)),
                    None => groups.push((scope, vec![(line, text)])),
                }
            }
        }
        files.push(FileHits { path: rel.to_owned(), lang: lang.name(rel), count, groups });
        Ok(())
    })?;
    let matches: usize = files.iter().map(|f| f.count).sum();
    let mut out = String::new();
    if o.json {
        let list: Vec<String> = files
            .iter()
            .map(|f| {
                let groups: Vec<String> = f
                    .groups
                    .iter()
                    .map(|(scope, lines)| {
                        let scope: Vec<String> = scope.iter().map(json_symbol).collect();
                        let lines: Vec<String> = lines.iter().map(|(n, t)| format!("{{\"n\":{n},\"text\":{}}}", json_str(t))).collect();
                        format!("{{\"scope\":[{}],\"lines\":[{}]}}", scope.join(","), lines.join(","))
                    })
                    .collect();
                format!("{{\"path\":{},\"lang\":\"{}\",\"count\":{},\"groups\":[{}]}}", json_str(&f.path), f.lang, f.count, groups.join(","))
            })
            .collect();
        _ = writeln!(out, "{{\"files\":[{}],\"matches\":{matches},\"searched\":{}}}", list.join(","), walked.files);
        return Ok(out);
    }
    for f in &files {
        if o.count {
            _ = writeln!(out, "{}: {}", f.path, f.count);
            continue;
        }
        _ = writeln!(out, "{}", f.path);
        for (scope, lines) in &f.groups {
            let names: Vec<String> = scope.iter().map(|s| format!("{} {}", s.kind, s.name)).collect();
            match scope.last() {
                Some(s) => _ = writeln!(out, "  {} [{}-{}]", names.join(" > "), s.start, s.end),
                None if o.flat => {}
                None => _ = writeln!(out, "  (top level)"),
            }
            for (n, text) in lines {
                _ = writeln!(out, "    {n}: {}", text.trim_end());
            }
        }
    }
    let plural = |n: usize, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
    let found = if matches == 1 { "1 match".to_owned() } else { format!("{matches} matches") };
    _ = writeln!(out, "{found} in {}, {} searched", plural(files.len(), "file"), plural(walked.files, "file"));
    Ok(out)
}

fn outline_cmd(path: &Path, o: &Options) -> Result<String, Error> {
    let mut out = String::new();
    walk(path, true, &[], &mut |_, rel, bytes| {
        let text = String::from_utf8_lossy(&bytes);
        let lang = Lang::of(rel);
        let symbols = outline(lang, &text);
        if symbols.len() > limits().symbols {
            return Err(Error::Limit("symbols", limits().symbols as u64, rel.to_owned()));
        }
        let lines = text.lines().count();
        if o.json {
            let list: Vec<String> = symbols.iter().map(json_symbol).collect();
            _ = writeln!(out, "{{\"path\":{},\"lang\":\"{}\",\"lines\":{lines},\"symbols\":[{}]}}", json_str(rel), lang.name(rel), list.join(","));
            return Ok(());
        }
        let count = if symbols.len() == 1 { "1 symbol".to_owned() } else { format!("{} symbols", symbols.len()) };
        _ = writeln!(out, "{rel} ({}, {lines} lines, {count})", lang.name(rel));
        for s in &symbols {
            _ = writeln!(out, "{}{}", "  ".repeat(s.depth + 1), label(s));
        }
        Ok(())
    })?;
    Ok(out)
}

struct Found {
    score: u32,
    path: String,
    lang: &'static str,
    lines: usize,
    why: Vec<String>,
    symbols: Vec<Symbol>,
}

fn is_test(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    path.split('/').any(|d| ["tests", "test", "__tests__", "spec"].contains(&d))
        || name.starts_with("test_")
        || ["_test.", ".test.", ".spec."].iter().any(|m| name.contains(m))
}

fn find(root: &Path, query: &str, o: &Options) -> Result<String, Error> {
    let mut terms: Vec<String> = Vec::new();
    for term in query.split_whitespace().map(str::to_lowercase) {
        if !terms.contains(&term) {
            terms.push(term);
        }
    }
    if terms.is_empty() {
        return Err(Error::Pattern("give at least one word".into()));
    }
    let needles: Vec<Needle> = terms.iter().map(|t| Needle::new(t, true)).collect();
    let mut found: Vec<Found> = Vec::new();
    let walked = walk(root, o.hidden, &o.globs, &mut |_, rel, bytes| {
        let lower_path = rel.to_lowercase();
        let (dirs, name) = lower_path.rsplit_once('/').unwrap_or(("", &lower_path));
        let stem = name.split('.').next().unwrap_or(name);
        let lang = Lang::of(rel);
        let mut outlined: Option<Vec<Symbol>> = None;
        let (mut score, mut hit) = (0, 0);
        let (mut is_name, mut has_name, mut in_path, mut text) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let mut named: Vec<(bool, Symbol)> = Vec::new();
        for (term, needle) in terms.iter().zip(&needles) {
            let before = score;
            if stem == term {
                score += 40;
                is_name.push(term.as_str());
            } else if name.contains(term.as_str()) {
                score += 24;
                has_name.push(term.as_str());
            }
            if dirs.contains(term.as_str()) {
                score += if dirs.split('/').any(|d| d == term) { 12 } else { 6 };
                in_path.push(term.as_str());
            }
            let (mut lines, mut from) = (0, 0);
            while let Some(at) = needle.find(&bytes, from) {
                lines += 1;
                from = bytes[at..].iter().position(|&b| b == b'\n').map_or(bytes.len(), |i| at + i + 1);
            }
            if lines > 0 {
                let symbols = outlined.get_or_insert_with(|| outline(lang, &String::from_utf8_lossy(&bytes)));
                let mut mine: Vec<&Symbol> = symbols.iter().filter(|s| s.name.to_lowercase().contains(term.as_str())).collect();
                mine.sort_by_key(|s| s.name.to_lowercase() != *term);
                for s in mine.iter().take(3) {
                    let exact = s.name.to_lowercase() == *term;
                    score += if exact { 16 } else { 6 };
                    if !named.iter().any(|n| n.1.head == s.head) {
                        named.push((exact, (*s).clone()));
                    }
                }
                score += u32::try_from(lines.min(10)).unwrap_or(10);
                text.push(format!("{term} {lines}"));
            }
            hit += u32::from(score > before);
        }
        if score == 0 {
            return Ok(());
        }
        let mut why = Vec::new();
        for (what, terms) in [("name is", is_name), ("name has", has_name), ("path has", in_path)] {
            if !terms.is_empty() {
                why.push(format!("{what} {}", terms.join(", ")));
            }
        }
        if !text.is_empty() {
            why.push(format!("lines: {}", text.join(", ")));
        }
        score *= hit;
        if is_test(rel) {
            score /= 2;
            why.push("a test".into());
        }
        named.sort_by_key(|n| !n.0);
        named.truncate(5);
        let lines = bytes.iter().filter(|&&b| b == b'\n').count();
        let symbols = named.into_iter().map(|n| n.1).collect();
        found.push(Found { score, path: rel.to_owned(), lang: lang.name(rel), lines, why, symbols });
        Ok(())
    })?;
    found.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.path.cmp(&b.path)));
    let total = found.len();
    found.truncate(o.top);
    let mut out = String::new();
    if o.json {
        let list: Vec<String> = found
            .iter()
            .map(|f| {
                let why: Vec<String> = f.why.iter().map(|w| json_str(w)).collect();
                let symbols: Vec<String> = f.symbols.iter().map(json_symbol).collect();
                format!("{{\"path\":{},\"lang\":\"{}\",\"lines\":{},\"score\":{},\"why\":[{}],\"symbols\":[{}]}}", json_str(&f.path), f.lang, f.lines, f.score, why.join(","), symbols.join(","))
            })
            .collect();
        _ = writeln!(out, "{{\"files\":[{}],\"total\":{total},\"searched\":{}}}", list.join(","), walked.files);
        return Ok(out);
    }
    for (n, f) in found.iter().enumerate() {
        _ = writeln!(out, "{}. {} ({}, {} lines)", n + 1, f.path, f.lang, f.lines);
        _ = writeln!(out, "     why: {}", f.why.join("; "));
        for s in &f.symbols {
            _ = writeln!(out, "     {}", label(s));
        }
    }
    _ = writeln!(out, "{} of {total} files, {} searched", found.len(), walked.files);
    Ok(out)
}

fn main() {
    let mut o = Options { json: false, fixed: false, fold: false, word: false, count: false, hidden: false, flat: false, top: 10, globs: Vec::new() };
    let mut rest: Vec<String> = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--json" => o.json = true,
            "-F" | "--fixed" => o.fixed = true,
            "-i" | "--ignore-case" => o.fold = true,
            "-w" | "--word" => o.word = true,
            "--count" => o.count = true,
            "--hidden" => o.hidden = true,
            "--flat" => o.flat = true,
            "--no-limits" => UNLIMITED.store(true, std::sync::atomic::Ordering::Relaxed),
            "--top" => o.top = args.next().and_then(|n| n.parse().ok()).unwrap_or(10),
            "-g" | "--glob" => o.globs.extend(args.next()),
            _ => rest.push(arg),
        }
    }
    let part = |i: usize| rest.get(i).map_or("", String::as_str);
    let root = PathBuf::from(rest.get(2).map_or(".", String::as_str));
    let done = match part(0) {
        "grep" => grep(&root, part(1), &o),
        "find" => find(&root, part(1), &o),
        "outline" => outline_cmd(Path::new(part(1)), &o),
        _ => Err(Error::Io("usage: prototype grep|find|outline …".into())),
    };
    match done {
        Ok(out) => print!("{out}"),
        Err(Error::Limit(which, max, at)) => {
            eprintln!("LIMIT {which}: over {max} at {at}");
            std::process::exit(3);
        }
        Err(Error::Pattern(why)) => {
            eprintln!("INVALID_PARAMS pattern {why}");
            std::process::exit(2);
        }
        Err(Error::Io(why)) => {
            eprintln!("IO {why}");
            std::process::exit(1);
        }
    }
}
