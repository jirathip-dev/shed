use crate::ScriptError;
use crate::ast::Pos;
use crate::syntax;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Tok {
    Name(String),
    Num(f64),
    Str(String),
    Template(String, bool),
    Punct(&'static str),
    Eof,
}

#[derive(Debug, Clone)]
pub(crate) struct Token {
    pub(crate) tok: Tok,
    pub(crate) pos: Pos,
    pub(crate) newline_before: bool,
}

const PUNCTS: &[&str] = &[
    ">>>=", "...", "===", "!==", "**=", "<<=", ">>=", ">>>", "&&=", "||=", "??=", "=>", "==", "!=",
    "<=", ">=", "&&", "||", "??", "?.", "++", "--", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=",
    "**", "<<", ">>", "{", "}", "(", ")", "[", "]", ";", ",", "<", ">", "+", "-", "*", "/", "%",
    "&", "|", "^", "!", "~", "?", ":", "=", ".",
];

#[derive(Clone)]
pub(crate) struct Lexer<'a> {
    chars: &'a [char],
    at: usize,
    line: u32,
    column: u32,
}

fn is_name_start(c: char) -> bool {
    c == '$' || c == '_' || c.is_alphabetic()
}

fn is_name_part(c: char) -> bool {
    is_name_start(c) || c.is_alphanumeric()
}

fn is_newline(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

impl<'a> Lexer<'a> {
    pub(crate) fn new(chars: &'a [char]) -> Self {
        Self {
            chars,
            at: 0,
            line: 1,
            column: 1,
        }
    }

    fn pos(&self) -> Pos {
        Pos {
            line: self.line,
            column: self.column,
        }
    }

    fn peek_at(&self, ahead: usize) -> Option<char> {
        self.chars.get(self.at + ahead).copied()
    }

    fn peek(&self) -> Option<char> {
        self.peek_at(0)
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.at += 1;
        if is_newline(c) && !(c == '\r' && self.peek() == Some('\n')) {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        Some(c)
    }

    fn starts_with(&self, text: &str) -> bool {
        text.chars()
            .enumerate()
            .all(|(i, c)| self.peek_at(i) == Some(c))
    }

    fn skip_space(&mut self) -> Result<bool, ScriptError> {
        let mut newline = false;
        loop {
            match self.peek() {
                Some(c) if is_newline(c) => {
                    newline = true;
                    self.bump();
                }
                Some(c) if c.is_whitespace() || c == '\u{feff}' => {
                    self.bump();
                }
                Some('/') if self.peek_at(1) == Some('/') => {
                    while self.peek().is_some_and(|c| !is_newline(c)) {
                        self.bump();
                    }
                }
                Some('/') if self.peek_at(1) == Some('*') => {
                    let start = self.pos();
                    self.bump();
                    self.bump();
                    while !self.starts_with("*/") {
                        match self.bump() {
                            Some(c) if is_newline(c) => newline = true,
                            Some(_) => {}
                            None => return syntax(start, "a /* comment is not closed"),
                        }
                    }
                    self.bump();
                    self.bump();
                }
                _ => return Ok(newline),
            }
        }
    }

    pub(crate) fn next(&mut self) -> Result<Token, ScriptError> {
        let newline_before = self.skip_space()?;
        let pos = self.pos();
        let tok = match self.peek() {
            None => Tok::Eof,
            Some(c) if is_name_start(c) => {
                let mut name = String::new();
                while let Some(c) = self.peek().filter(|&c| is_name_part(c)) {
                    name.push(c);
                    self.bump();
                }
                Tok::Name(name)
            }
            Some(c) if c.is_ascii_digit() => self.number(pos)?,
            Some('.') if self.peek_at(1).is_some_and(|c| c.is_ascii_digit()) => self.number(pos)?,
            Some(q @ ('"' | '\'')) => {
                self.bump();
                Tok::Str(self.text(pos, q)?)
            }
            Some('`') => {
                self.bump();
                self.template_chunk(pos)?
            }
            Some(c) => self.punct(pos, c)?,
        };
        Ok(Token {
            tok,
            pos,
            newline_before,
        })
    }

    pub(crate) fn template_rest(&mut self) -> Result<Token, ScriptError> {
        let pos = self.pos();
        let tok = self.template_chunk(pos)?;
        Ok(Token {
            tok,
            pos,
            newline_before: false,
        })
    }

    fn punct(&mut self, pos: Pos, c: char) -> Result<Tok, ScriptError> {
        if c == '?'
            && self.peek_at(1) == Some('.')
            && self.peek_at(2).is_some_and(|d| d.is_ascii_digit())
        {
            self.bump();
            return Ok(Tok::Punct("?"));
        }
        let Some(&p) = PUNCTS.iter().find(|p| self.starts_with(p)) else {
            return syntax(pos, format!("{c:?} is not in the subset"));
        };
        for _ in 0..p.chars().count() {
            self.bump();
        }
        Ok(Tok::Punct(p))
    }

    fn number(&mut self, pos: Pos) -> Result<Tok, ScriptError> {
        let radix = match (self.peek(), self.peek_at(1).map(|c| c.to_ascii_lowercase())) {
            (Some('0'), Some('x')) => 16,
            (Some('0'), Some('o')) => 8,
            (Some('0'), Some('b')) => 2,
            _ => 10,
        };
        let mut raw = String::new();
        let value = if radix == 10 {
            let mut text = String::new();
            while let Some(c) = self.peek() {
                let exponent_sign = matches!(c, '+' | '-') && text.ends_with(['e', 'E']);
                if !(c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '_') || exponent_sign) {
                    break;
                }
                if c != '_' {
                    text.push(c);
                }
                raw.push(c);
                self.bump();
            }
            if raw.len() > 1
                && raw.starts_with('0')
                && raw
                    .chars()
                    .nth(1)
                    .is_some_and(|c| c.is_ascii_digit() || c == '_')
            {
                return syntax(pos, "a number may not start with 0: write 0o10 for octal");
            }
            text.parse::<f64>()
                .or_else(|_| syntax(pos, format!("{text} is not a number")))?
        } else {
            self.bump();
            self.bump();
            let mut value = None;
            while let Some(c) = self.peek() {
                if c != '_' {
                    let Some(digit) = c.to_digit(radix) else {
                        break;
                    };
                    value = Some(value.unwrap_or(0.0) * f64::from(radix) + f64::from(digit));
                }
                raw.push(c);
                self.bump();
            }
            match value {
                Some(value) => value,
                None => return syntax(pos, "a number has no digits after its 0x, 0o or 0b"),
            }
        };
        let chars: Vec<char> = raw.chars().collect();
        let digit = |at: Option<usize>| {
            at.and_then(|at| chars.get(at))
                .is_some_and(|c| c.is_digit(radix))
        };
        if (0..chars.len()).any(|at| {
            chars.get(at) == Some(&'_') && !(digit(at.checked_sub(1)) && digit(Some(at + 1)))
        }) {
            return syntax(pos, "a _ in a number goes between two digits");
        }
        match self.peek() {
            Some('n') => syntax(
                pos,
                "a BigInt literal (1n) is not in the subset: use a number",
            ),
            Some(c) if is_name_part(c) => {
                syntax(self.pos(), format!("{c:?} cannot follow a number"))
            }
            _ => Ok(Tok::Num(value)),
        }
    }

    fn text(&mut self, start: Pos, quote: char) -> Result<String, ScriptError> {
        let mut text = String::new();
        loop {
            match self.peek() {
                None => return syntax(start, "a string is not closed"),
                Some(c) if c == quote => {
                    self.bump();
                    return Ok(text);
                }
                Some('\n' | '\r') => {
                    return syntax(
                        start,
                        "a string is not closed on its line: use a template string for several lines",
                    );
                }
                Some('\\') => self.escape(&mut text)?,
                Some(c) => {
                    text.push(c);
                    self.bump();
                }
            }
        }
    }

    fn template_chunk(&mut self, start: Pos) -> Result<Tok, ScriptError> {
        let mut text = String::new();
        loop {
            match self.peek() {
                None => return syntax(start, "a template string is not closed"),
                Some('`') => {
                    self.bump();
                    return Ok(Tok::Template(text, true));
                }
                Some('$') if self.peek_at(1) == Some('{') => {
                    self.bump();
                    self.bump();
                    return Ok(Tok::Template(text, false));
                }
                Some('\\') => self.escape(&mut text)?,
                Some('\r') => {
                    self.bump();
                    if self.peek() == Some('\n') {
                        self.bump();
                    }
                    text.push('\n');
                }
                Some(c) => {
                    text.push(c);
                    self.bump();
                }
            }
        }
    }

    fn escape(&mut self, text: &mut String) -> Result<(), ScriptError> {
        let pos = self.pos();
        self.bump();
        let Some(c) = self.bump() else {
            return syntax(pos, "an escape at the end of the code");
        };
        let decoded = match c {
            'n' => '\n',
            't' => '\t',
            'r' => '\r',
            'b' => '\u{8}',
            'f' => '\u{c}',
            'v' => '\u{b}',
            '0' if !self.peek().is_some_and(|d| d.is_ascii_digit()) => '\0',
            'x' => self.hex(pos, 2)?,
            'u' if self.peek() == Some('{') => {
                self.bump();
                let mut code = 0u32;
                while let Some(d) = self.peek().and_then(|d| d.to_digit(16)) {
                    self.bump();
                    code = code.saturating_mul(16).saturating_add(d);
                }
                if self.bump() != Some('}') {
                    return syntax(pos, "a \\u{…} escape is not closed");
                }
                Self::code_point(pos, code)?
            }
            'u' => self.hex(pos, 4)?,
            '\r' => {
                if self.peek() == Some('\n') {
                    self.bump();
                }
                return Ok(());
            }
            c if is_newline(c) => return Ok(()),
            c if c.is_ascii_digit() => {
                return syntax(pos, "an octal escape is not in the subset: use \\x or \\u");
            }
            c => c,
        };
        text.push(decoded);
        Ok(())
    }

    fn hex(&mut self, pos: Pos, digits: usize) -> Result<char, ScriptError> {
        let mut code = 0;
        for _ in 0..digits {
            let Some(d) = self.peek().and_then(|d| d.to_digit(16)) else {
                return syntax(pos, "an escape needs hex digits");
            };
            self.bump();
            code = code * 16 + d;
        }
        Self::code_point(pos, code)
    }

    fn code_point(pos: Pos, code: u32) -> Result<char, ScriptError> {
        char::from_u32(code).map_or_else(
            || {
                syntax(
                    pos,
                    "an escape names no character (a lone surrogate or past 10FFFF)",
                )
            },
            Ok,
        )
    }
}
