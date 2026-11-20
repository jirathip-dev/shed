use std::collections::HashMap;

use crate::ScriptError;
use crate::ast::{
    Assign, Binary, Binding, Decl, Expr, Function, Item, Key, Logic, Pattern, Pos, Prop, Script,
    Stmt, Unary,
};
use crate::lex::{Lexer, Tok, Token};
use crate::syntax;

const DEPTH_MAX: usize = 200;
const REGEX: &str =
    "a regex literal is not in the subset: use text.includes(…) or text.matches(pattern)";
const RESERVED: &[&str] = &[
    "await", "break", "case", "catch", "const", "continue", "default", "else", "enum", "false",
    "for", "function", "if", "let", "new", "null", "return", "throw", "true", "try", "typeof",
    "while",
];

type Parsed<T> = Result<T, ScriptError>;

fn refused(word: &str) -> Option<&'static str> {
    Some(match word {
        "var" => "var is not in the subset: use let, or const",
        "this" => "this is not in the subset: pass the value as a parameter",
        "class" => "class is not in the subset: use an object of functions",
        "yield" => "yield is not in the subset: there are no generators, return an array",
        "super" => "super is not in the subset: there are no classes",
        "switch" => "switch is not in the subset: use if and else if",
        "do" => "do … while is not in the subset: use while",
        "with" => "with is not in the subset: name the object",
        "import" => "import is not in the subset: a script has only its globals",
        "export" => "export is not in the subset: return the value",
        "debugger" => "debugger is not in the subset: use console.log",
        "delete" => "delete is not in the subset: build the object without the key",
        "void" => "void is not in the subset: use undefined",
        "in" => "the in operator is not in the subset: use Object.hasOwn(object, key)",
        "instanceof" => "instanceof is not in the subset: use typeof or Array.isArray",
        "finally" => "finally is not in the subset: run the cleanup after the try",
        _ => return None,
    })
}

enum Op {
    Logic(Logic),
    Binary(Binary),
    Loose(bool),
}

fn binary_op(p: &str) -> Option<(u8, Op)> {
    Some(match p {
        "??" => (1, Op::Logic(Logic::Nullish)),
        "||" => (2, Op::Logic(Logic::Or)),
        "&&" => (3, Op::Logic(Logic::And)),
        "===" => (7, Op::Binary(Binary::Eq)),
        "!==" => (7, Op::Binary(Binary::Ne)),
        "==" => (7, Op::Loose(false)),
        "!=" => (7, Op::Loose(true)),
        "<" => (8, Op::Binary(Binary::Lt)),
        "<=" => (8, Op::Binary(Binary::Le)),
        ">" => (8, Op::Binary(Binary::Gt)),
        ">=" => (8, Op::Binary(Binary::Ge)),
        "+" => (10, Op::Binary(Binary::Add)),
        "-" => (10, Op::Binary(Binary::Sub)),
        "*" => (11, Op::Binary(Binary::Mul)),
        "/" => (11, Op::Binary(Binary::Div)),
        "%" => (11, Op::Binary(Binary::Rem)),
        "**" => (12, Op::Binary(Binary::Pow)),
        _ => return None,
    })
}

fn assign_op(p: &str) -> Option<Assign> {
    Some(match p {
        "=" => Assign::Set,
        "+=" => Assign::Add,
        "-=" => Assign::Sub,
        "*=" => Assign::Mul,
        "/=" => Assign::Div,
        "&&=" => Assign::And,
        "||=" => Assign::Or,
        "??=" => Assign::Nullish,
        _ => return None,
    })
}

fn bitwise(p: &str) -> bool {
    matches!(
        p,
        "&" | "|" | "^" | "~" | "<<" | ">>" | ">>>" | "&=" | "|=" | "^=" | "<<=" | ">>=" | ">>>="
    )
}

fn describe(tok: &Tok) -> String {
    match tok {
        Tok::Name(w) => w.clone(),
        Tok::Num(_) => "a number".into(),
        Tok::Str(_) => "a string".into(),
        Tok::Template(..) => "a template string".into(),
        Tok::Punct(p) => (*p).into(),
        Tok::Eof => "the end of the code".into(),
    }
}

fn head_pos(e: &Expr, or: Pos) -> Pos {
    match e {
        Expr::Name(pos, _) | Expr::Member(pos, ..) => *pos,
        _ => or,
    }
}

pub(crate) struct Parser<'a> {
    lex: Lexer<'a>,
    cur: Token,
    depth: usize,
    in_async: bool,
    loops: usize,
    arrows: HashMap<Pos, bool>,
}

impl<'a> Parser<'a> {
    pub(crate) fn new(chars: &'a [char]) -> Parsed<Self> {
        let mut lex = Lexer::new(chars);
        let cur = lex.next()?;
        Ok(Self {
            lex,
            cur,
            depth: 0,
            in_async: true,
            loops: 0,
            arrows: HashMap::new(),
        })
    }

    pub(crate) fn script(mut self) -> Parsed<Script> {
        let mut body = Vec::new();
        while self.cur.tok != Tok::Eof {
            body.push(self.statement()?);
        }
        Ok(Script { body })
    }

    fn unexpected<T>(&self) -> Parsed<T> {
        if let Tok::Name(w) = &self.cur.tok
            && let Some(message) = refused(w)
        {
            return syntax(self.cur.pos, message);
        }
        if let Tok::Punct(p) = self.cur.tok
            && bitwise(p)
        {
            return syntax(
                self.cur.pos,
                format!("the bitwise operator {p} is not in the subset"),
            );
        }
        syntax(
            self.cur.pos,
            format!("unexpected {}", describe(&self.cur.tok)),
        )
    }

    fn advance(&mut self) -> Parsed<()> {
        self.cur = self.lex.next()?;
        Ok(())
    }

    fn peek(&self) -> Parsed<Token> {
        self.lex.clone().next()
    }

    fn is(&self, p: &str) -> bool {
        matches!(self.cur.tok, Tok::Punct(q) if q == p)
    }

    fn is_word(&self, w: &str) -> bool {
        matches!(&self.cur.tok, Tok::Name(n) if n == w)
    }

    fn eat(&mut self, p: &str) -> Parsed<bool> {
        let found = self.is(p);
        if found {
            self.advance()?;
        }
        Ok(found)
    }

    fn expect(&mut self, p: &str) -> Parsed<()> {
        if self.eat(p)? {
            return Ok(());
        }
        self.expected(p)
    }

    fn expected<T>(&self, p: &str) -> Parsed<T> {
        if self.cur.tok == Tok::Eof {
            return syntax(
                self.cur.pos,
                format!("the code ends where {p} was expected"),
            );
        }
        match self.unexpected() {
            Err(e) if e.message.starts_with("unexpected") => syntax(
                self.cur.pos,
                format!("expected {p} before {}", describe(&self.cur.tok)),
            ),
            other => other,
        }
    }

    fn semi(&mut self) -> Parsed<()> {
        if self.is(";") {
            return self.advance();
        }
        if self.is("}") || self.cur.tok == Tok::Eof || self.cur.newline_before {
            return Ok(());
        }
        if self.is(",") {
            return syntax(
                self.cur.pos,
                "the comma operator is not in the subset: use one statement each",
            );
        }
        self.unexpected()
    }

    fn nested<T>(&mut self, f: impl FnOnce(&mut Self) -> Parsed<T>) -> Parsed<T> {
        if self.depth >= DEPTH_MAX {
            return syntax(
                self.cur.pos,
                format!("nesting is deeper than {DEPTH_MAX} levels: split the code into functions"),
            );
        }
        self.depth += 1;
        let parsed = f(self);
        self.depth -= 1;
        parsed
    }

    fn name(&mut self) -> Parsed<(Pos, String)> {
        let pos = self.cur.pos;
        let Tok::Name(w) = &self.cur.tok else {
            return syntax(
                pos,
                format!("expected a name before {}", describe(&self.cur.tok)),
            );
        };
        if let Some(message) = refused(w) {
            return syntax(pos, message);
        }
        if RESERVED.contains(&w.as_str()) {
            return syntax(pos, format!("{w} is a reserved word, not a name"));
        }
        let w = w.clone();
        self.advance()?;
        Ok((pos, w))
    }

    fn statement(&mut self) -> Parsed<Stmt> {
        let pos = self.cur.pos;
        let word = match &self.cur.tok {
            Tok::Punct("{") => return Ok(Stmt::Block(self.block()?)),
            Tok::Punct(";") => {
                self.advance()?;
                return Ok(Stmt::Empty);
            }
            Tok::Name(w) => w.clone(),
            _ => return self.expression_statement(),
        };
        match word.as_str() {
            "const" | "let" => {
                let decl = if word == "const" {
                    Decl::Const
                } else {
                    Decl::Let
                };
                self.advance()?;
                let first = self.pattern()?;
                let stmt = self.declaration(decl, first)?;
                self.semi()?;
                Ok(stmt)
            }
            "function" => Ok(Stmt::Function(Box::new(self.function(false, true)?))),
            "async" if self.async_function()? => {
                self.advance()?;
                Ok(Stmt::Function(Box::new(self.function(true, true)?)))
            }
            "if" => {
                self.advance()?;
                let test = self.condition()?;
                let then = self.body()?;
                let otherwise = if self.is_word("else") {
                    self.advance()?;
                    Some(self.body()?)
                } else {
                    None
                };
                Ok(Stmt::If(test, then, otherwise))
            }
            "for" => self.for_statement(),
            "while" => {
                self.advance()?;
                let test = self.condition()?;
                Ok(Stmt::While(test, self.loop_body()?))
            }
            "break" | "continue" | "return" | "throw" => self.jump(&word, pos),
            "try" => self.try_statement(),
            "else" | "catch" => syntax(
                pos,
                format!(
                    "{word} without its {}",
                    if word == "else" { "if" } else { "try" }
                ),
            ),
            _ if refused(&word).is_none()
                && !RESERVED.contains(&word.as_str())
                && self.peek()?.tok == Tok::Punct(":") =>
            {
                syntax(
                    pos,
                    "a label is not in the subset: use a flag, or return from a function",
                )
            }
            _ => self.expression_statement(),
        }
    }

    fn jump(&mut self, word: &str, pos: Pos) -> Parsed<Stmt> {
        match word {
            "break" | "continue" => {
                self.advance()?;
                if self.loops == 0 {
                    return syntax(pos, format!("{word} is only valid inside a loop"));
                }
                if matches!(self.cur.tok, Tok::Name(_)) && !self.cur.newline_before {
                    return syntax(
                        self.cur.pos,
                        "a label is not in the subset: use a flag, or return from a function",
                    );
                }
                self.semi()?;
                Ok(if word == "break" {
                    Stmt::Break(pos)
                } else {
                    Stmt::Continue(pos)
                })
            }
            "return" => {
                self.advance()?;
                let value = if self.is(";")
                    || self.is("}")
                    || self.cur.tok == Tok::Eof
                    || self.cur.newline_before
                {
                    None
                } else {
                    Some(self.expression()?)
                };
                self.semi()?;
                Ok(Stmt::Return(pos, value))
            }
            "throw" => {
                self.advance()?;
                if self.cur.newline_before {
                    return syntax(pos, "throw needs its value on the same line");
                }
                let value = self.expression()?;
                self.semi()?;
                Ok(Stmt::Throw(pos, value))
            }
            _ => self.expression_statement(),
        }
    }

    fn expression_statement(&mut self) -> Parsed<Stmt> {
        let e = self.expression()?;
        self.semi()?;
        Ok(Stmt::Expr(e))
    }

    fn declaration(&mut self, decl: Decl, first: Pattern) -> Parsed<Stmt> {
        let mut bindings = Vec::new();
        let mut target = first;
        loop {
            let pos = self.cur.pos;
            let init = if self.eat("=")? {
                Some(self.assign()?)
            } else {
                None
            };
            if init.is_none() && (decl == Decl::Const || !matches!(target, Pattern::Name(..))) {
                return syntax(pos, "this declaration needs a value: = …");
            }
            bindings.push((target, init));
            if !self.eat(",")? {
                return Ok(Stmt::Decl(decl, bindings));
            }
            target = self.pattern()?;
        }
    }

    fn condition(&mut self) -> Parsed<Expr> {
        self.expect("(")?;
        let test = self.nested(Self::expression)?;
        self.expect(")")?;
        Ok(test)
    }

    fn body(&mut self) -> Parsed<Box<Stmt>> {
        let stmt = if self.is("{") {
            Stmt::Block(self.block()?)
        } else {
            self.nested(Self::statement)?
        };
        Ok(Box::new(stmt))
    }

    fn loop_body(&mut self) -> Parsed<Box<Stmt>> {
        self.loops += 1;
        let body = self.body();
        self.loops -= 1;
        body
    }

    fn block(&mut self) -> Parsed<Vec<Stmt>> {
        let open = self.cur.pos;
        self.nested(|p| {
            p.expect("{")?;
            let mut body = Vec::new();
            while !p.eat("}")? {
                if p.cur.tok == Tok::Eof {
                    return syntax(open, "a { is not closed");
                }
                body.push(p.statement()?);
            }
            Ok(body)
        })
    }

    fn for_statement(&mut self) -> Parsed<Stmt> {
        self.advance()?;
        if self.is_word("await") {
            return syntax(
                self.cur.pos,
                "for await is not in the subset: use Promise.all",
            );
        }
        self.expect("(")?;
        let init = if self.is_word("const") || self.is_word("let") {
            let decl = if self.is_word("const") {
                Decl::Const
            } else {
                Decl::Let
            };
            self.advance()?;
            let target = self.pattern()?;
            if self.is_word("of") {
                self.advance()?;
                let items = self.nested(Self::assign)?;
                self.expect(")")?;
                return Ok(Stmt::ForOf(decl, target, items, self.loop_body()?));
            }
            if self.is_word("in") {
                return syntax(
                    self.cur.pos,
                    "for … in is not in the subset: use for (const key of Object.keys(object))",
                );
            }
            Some(Box::new(self.declaration(decl, target)?))
        } else if self.is(";") {
            None
        } else {
            if matches!(self.cur.tok, Tok::Name(_)) {
                match &self.peek()?.tok {
                    Tok::Name(w) if w == "of" => {
                        return syntax(
                            self.cur.pos,
                            "for … of needs const or let: for (const x of xs)",
                        );
                    }
                    Tok::Name(w) if w == "in" => {
                        return syntax(
                            self.cur.pos,
                            "for … in is not in the subset: use for (const key of Object.keys(object))",
                        );
                    }
                    _ => {}
                }
            }
            Some(Box::new(Stmt::Expr(self.nested(Self::expression)?)))
        };
        self.expect(";")?;
        let test = if self.is(";") {
            None
        } else {
            Some(self.nested(Self::expression)?)
        };
        self.expect(";")?;
        let update = if self.is(")") {
            None
        } else {
            Some(self.nested(Self::expression)?)
        };
        self.expect(")")?;
        Ok(Stmt::For(init, test, update, self.loop_body()?))
    }

    fn try_statement(&mut self) -> Parsed<Stmt> {
        self.advance()?;
        let body = self.block()?;
        if self.is_word("finally") {
            return self.unexpected();
        }
        if !self.is_word("catch") {
            return syntax(self.cur.pos, "try needs a catch");
        }
        self.advance()?;
        let param = if self.eat("(")? {
            let p = self.pattern()?;
            self.expect(")")?;
            Some(p)
        } else {
            None
        };
        let handler = self.block()?;
        if self.is_word("finally") {
            return self.unexpected();
        }
        Ok(Stmt::Try(body, param, handler))
    }

    fn pattern(&mut self) -> Parsed<Pattern> {
        if self.is("[") {
            return self.nested(Self::array_pattern);
        }
        if self.is("{") {
            return self.nested(Self::object_pattern);
        }
        let (pos, name) = self.name()?;
        Ok(Pattern::Name(pos, name))
    }

    fn binding(&mut self) -> Parsed<Binding> {
        let target = self.pattern()?;
        let default = if self.eat("=")? {
            Some(self.assign()?)
        } else {
            None
        };
        Ok((target, default))
    }

    fn rest(&mut self, close: &str) -> Parsed<Option<Box<Pattern>>> {
        let rest = self.pattern()?;
        if self.is(",") {
            return syntax(self.cur.pos, "...rest must come last");
        }
        self.expect(close)?;
        Ok(Some(Box::new(rest)))
    }

    fn array_pattern(&mut self) -> Parsed<Pattern> {
        self.advance()?;
        let mut items = Vec::new();
        loop {
            if self.eat("]")? {
                return Ok(Pattern::Array(items, None));
            }
            if self.eat(",")? {
                items.push(None);
                continue;
            }
            if self.eat("...")? {
                return Ok(Pattern::Array(items, self.rest("]")?));
            }
            items.push(Some(self.binding()?));
            if !self.is("]") {
                self.expect(",")?;
            }
        }
    }

    fn object_pattern(&mut self) -> Parsed<Pattern> {
        self.advance()?;
        let mut props = Vec::new();
        loop {
            if self.eat("}")? {
                return Ok(Pattern::Object(props, None));
            }
            if self.eat("...")? {
                return Ok(Pattern::Object(props, self.rest("}")?));
            }
            let pos = self.cur.pos;
            let (key, shorthand) = self.key()?;
            let binding = if self.eat(":")? {
                self.binding()?
            } else if let Some(name) = shorthand {
                Self::check_name(pos, &name)?;
                let default = if self.eat("=")? {
                    Some(self.assign()?)
                } else {
                    None
                };
                (Pattern::Name(pos, name), default)
            } else {
                return syntax(
                    self.cur.pos,
                    format!("expected : before {}", describe(&self.cur.tok)),
                );
            };
            props.push((key, binding));
            if !self.is("}") {
                self.expect(",")?;
            }
        }
    }

    fn check_name(pos: Pos, name: &str) -> Parsed<()> {
        if let Some(message) = refused(name) {
            return syntax(pos, message);
        }
        if RESERVED.contains(&name) {
            return syntax(pos, format!("{name} is a reserved word, not a name"));
        }
        Ok(())
    }

    fn key(&mut self) -> Parsed<(Key, Option<String>)> {
        let key = match &self.cur.tok {
            Tok::Name(w) => {
                let w = w.clone();
                self.advance()?;
                return Ok((Key::Name(w.clone()), Some(w)));
            }
            Tok::Str(s) => Key::Name(s.clone()),
            Tok::Num(n) => Key::Name(n.to_string()),
            Tok::Punct("[") => {
                self.advance()?;
                let key = self.nested(Self::assign)?;
                self.expect("]")?;
                return Ok((Key::Computed(Box::new(key)), None));
            }
            _ => {
                return syntax(
                    self.cur.pos,
                    format!(
                        "expected a property name before {}",
                        describe(&self.cur.tok)
                    ),
                );
            }
        };
        self.advance()?;
        Ok((key, None))
    }

    fn starts_key(tok: &Tok) -> bool {
        matches!(
            tok,
            Tok::Name(_) | Tok::Str(_) | Tok::Num(_) | Tok::Punct("[")
        )
    }

    fn async_function(&self) -> Parsed<bool> {
        let next = self.peek()?;
        Ok(!next.newline_before && next.tok == Tok::Name("function".into()))
    }

    fn function(&mut self, is_async: bool, named: bool) -> Parsed<Function> {
        let pos = self.cur.pos;
        self.advance()?;
        if self.is("*") {
            return syntax(
                pos,
                "a generator (function*) is not in the subset: return an array",
            );
        }
        let name = if named || matches!(self.cur.tok, Tok::Name(_)) {
            Some(self.name()?.1)
        } else {
            None
        };
        self.function_rest(pos, name, is_async)
    }

    fn function_rest(
        &mut self,
        pos: Pos,
        name: Option<String>,
        is_async: bool,
    ) -> Parsed<Function> {
        let (params, rest) = self.params()?;
        let body = self.function_body(is_async, Self::block)?;
        Ok(Function {
            pos,
            name,
            is_async,
            arrow: false,
            params,
            rest,
            body,
        })
    }

    fn function_body(
        &mut self,
        is_async: bool,
        f: impl FnOnce(&mut Self) -> Parsed<Vec<Stmt>>,
    ) -> Parsed<Vec<Stmt>> {
        let outer = (self.in_async, self.loops);
        self.in_async = is_async;
        self.loops = 0;
        let body = f(self);
        (self.in_async, self.loops) = outer;
        body
    }

    fn params(&mut self) -> Parsed<(Vec<Binding>, Option<Pattern>)> {
        self.nested(|p| {
            p.expect("(")?;
            let mut params = Vec::new();
            loop {
                if p.eat(")")? {
                    return Ok((params, None));
                }
                if p.eat("...")? {
                    let rest = p.rest(")")?;
                    return Ok((params, rest.map(|r| *r)));
                }
                params.push(p.binding()?);
                if !p.is(")") {
                    p.expect(",")?;
                }
            }
        })
    }

    fn expression(&mut self) -> Parsed<Expr> {
        let e = self.assign()?;
        if self.is(",") {
            return syntax(
                self.cur.pos,
                "the comma operator is not in the subset: use one statement each",
            );
        }
        Ok(e)
    }

    fn assign(&mut self) -> Parsed<Expr> {
        if let Some(f) = self.arrow()? {
            return Ok(Expr::Function(Box::new(f)));
        }
        let target = self.conditional()?;
        let pos = self.cur.pos;
        let Tok::Punct(p) = self.cur.tok else {
            return Ok(target);
        };
        let Some(op) = assign_op(p) else {
            if bitwise(p) {
                return self.unexpected();
            }
            if matches!(p, "%=" | "**=") {
                return syntax(
                    pos,
                    format!(
                        "the operator {p} is not in the subset: write x = x {} y",
                        p.trim_end_matches('=')
                    ),
                );
            }
            return Ok(target);
        };
        Self::check_target(pos, &target, p)?;
        self.advance()?;
        let value = self.nested(Self::assign)?;
        Ok(Expr::Assign(pos, op, Box::new(target), Box::new(value)))
    }

    fn check_target(pos: Pos, target: &Expr, op: &str) -> Parsed<()> {
        match target {
            Expr::Name(at, name) => Self::check_name(*at, name),
            Expr::Member(.., false) | Expr::Index(.., false) => Ok(()),
            Expr::Array(_) | Expr::Object(_) => syntax(
                pos,
                "destructuring assignment is not in the subset: declare with const [a, b] = … or const { a } = …",
            ),
            _ => syntax(pos, format!("the left of {op} is not a name or a property")),
        }
    }

    fn arrow(&mut self) -> Parsed<Option<Function>> {
        let pos = self.cur.pos;
        let is_async = self.is_word("async");
        let mut lex = self.lex.clone();
        let after = lex.clone();
        let Ok(next) = lex.next() else {
            return Ok(None);
        };
        if is_async && !next.newline_before && matches!(next.tok, Tok::Name(_) | Tok::Punct("(")) {
            let arrow = if next.tok == Tok::Punct("(") {
                self.arrow_at(lex, next.pos)
            } else {
                matches!(
                    lex.next(),
                    Ok(Token {
                        tok: Tok::Punct("=>"),
                        ..
                    })
                )
            };
            if !arrow {
                return Ok(None);
            }
            self.advance()?;
        } else if self.is("(") {
            if !self.arrow_at(after, pos) {
                return Ok(None);
            }
        } else if !matches!(self.cur.tok, Tok::Name(_)) || next.tok != Tok::Punct("=>") {
            return Ok(None);
        }
        let (params, rest) = if self.is("(") {
            self.params()?
        } else {
            let (at, name) = self.name()?;
            (vec![(Pattern::Name(at, name), None)], None)
        };
        if self.cur.newline_before {
            return syntax(self.cur.pos, "=> must be on the line of its parameters");
        }
        self.expect("=>")?;
        let body = self.function_body(is_async, |p| {
            if p.is("{") {
                return p.block();
            }
            let at = p.cur.pos;
            Ok(vec![Stmt::Return(at, Some(p.nested(Self::assign)?))])
        })?;
        Ok(Some(Function {
            pos,
            name: None,
            is_async,
            arrow: true,
            params,
            rest,
            body,
        }))
    }

    fn arrow_at(&mut self, lex: Lexer<'a>, open: Pos) -> bool {
        if !self.arrows.contains_key(&open) {
            self.scan_arrows(lex, open);
        }
        self.arrows.get(&open) == Some(&true)
    }

    fn scan_arrows(&mut self, mut lex: Lexer<'a>, open: Pos) {
        let mut stack = vec![Some(open)];
        let mut templates = Vec::new();
        while let Some(&top) = stack.last() {
            if stack.len() > DEPTH_MAX {
                return;
            }
            let Ok(token) = lex.next() else { return };
            match token.tok {
                Tok::Punct("(") => stack.push(Some(token.pos)),
                Tok::Punct("[" | "{") => stack.push(None),
                Tok::Template(_, false) => {
                    templates.push(stack.len());
                    stack.push(None);
                }
                Tok::Punct("}") if templates.last() == Some(&(stack.len() - 1)) => {
                    match lex.template_rest() {
                        Ok(Token {
                            tok: Tok::Template(_, true),
                            ..
                        }) => {
                            templates.pop();
                            stack.pop();
                        }
                        Ok(_) => {}
                        Err(_) => return,
                    }
                }
                Tok::Punct(")" | "]" | "}") => {
                    stack.pop();
                    if let Some(at) = top {
                        let next = lex.clone().next();
                        self.arrows.insert(
                            at,
                            matches!(
                                next,
                                Ok(Token {
                                    tok: Tok::Punct("=>"),
                                    ..
                                })
                            ),
                        );
                    }
                }
                Tok::Eof => return,
                _ => {}
            }
        }
    }

    fn conditional(&mut self) -> Parsed<Expr> {
        let test = self.binary(0)?;
        if !self.eat("?")? {
            return Ok(test);
        }
        let then = self.nested(Self::assign)?;
        self.expect(":")?;
        let otherwise = self.nested(Self::assign)?;
        Ok(Expr::Cond(
            Box::new(test),
            Box::new(then),
            Box::new(otherwise),
        ))
    }

    fn binary(&mut self, min: u8) -> Parsed<Expr> {
        let prefixed = matches!(self.cur.tok, Tok::Punct("!" | "-" | "+"))
            || self.is_word("typeof")
            || self.is_word("await");
        let mut left = self.unary()?;
        let mut first = true;
        loop {
            let pos = self.cur.pos;
            let (prec, op) = match &self.cur.tok {
                Tok::Punct(p) if bitwise(p) && !p.ends_with('=') => return self.unexpected(),
                Tok::Punct(p) => match binary_op(p) {
                    Some(found) => found,
                    None => return Ok(left),
                },
                Tok::Name(w) if w == "in" || w == "instanceof" => return self.unexpected(),
                _ => return Ok(left),
            };
            if prec < min {
                return Ok(left);
            }
            if first && prefixed && matches!(op, Op::Binary(Binary::Pow)) {
                return syntax(pos, "put the left of ** in parentheses: (-a) ** b");
            }
            first = false;
            self.advance()?;
            let right = if matches!(op, Op::Binary(Binary::Pow)) {
                self.nested(|p| p.binary(prec))?
            } else {
                self.binary(prec + 1)?
            };
            left = match op {
                Op::Logic(op) => Expr::Logic(pos, op, Box::new(left), Box::new(right)),
                Op::Binary(op) => Expr::Binary(pos, op, Box::new(left), Box::new(right)),
                Op::Loose(negated) => match (left, right) {
                    (value, Expr::Null) | (Expr::Null, value) => {
                        Expr::NullCheck(pos, negated, Box::new(value))
                    }
                    _ => {
                        let op = if negated { "!=" } else { "==" };
                        return syntax(
                            pos,
                            format!(
                                "{op} is only for null in the subset: use {op}= (or {op} null)"
                            ),
                        );
                    }
                },
            };
        }
    }

    fn unary(&mut self) -> Parsed<Expr> {
        let pos = self.cur.pos;
        let op = match &self.cur.tok {
            Tok::Punct("!") => Some(Unary::Not),
            Tok::Punct("-") => Some(Unary::Neg),
            Tok::Punct("+") => Some(Unary::Plus),
            Tok::Punct("~") => return self.unexpected(),
            Tok::Name(w) if w == "typeof" => Some(Unary::Typeof),
            _ => None,
        };
        if let Some(op) = op {
            self.advance()?;
            let value = self.nested(Self::unary)?;
            return Ok(Expr::Unary(pos, op, Box::new(value)));
        }
        if self.is("++") || self.is("--") {
            let increment = self.is("++");
            self.advance()?;
            let target = self.nested(Self::unary)?;
            Self::check_target(pos, &target, if increment { "++" } else { "--" })?;
            return Ok(Expr::Update(pos, true, increment, Box::new(target)));
        }
        if self.is_word("await") {
            if !self.in_async {
                return syntax(pos, "await is only valid in an async function");
            }
            self.advance()?;
            let value = self.nested(Self::unary)?;
            return Ok(Expr::Await(pos, Box::new(value)));
        }
        let value = self.call_member()?;
        if (self.is("++") || self.is("--")) && !self.cur.newline_before {
            let increment = self.is("++");
            let at = self.cur.pos;
            Self::check_target(at, &value, if increment { "++" } else { "--" })?;
            self.advance()?;
            return Ok(Expr::Update(at, false, increment, Box::new(value)));
        }
        Ok(value)
    }

    fn call_member(&mut self) -> Parsed<Expr> {
        let mut e = self.primary()?;
        loop {
            let pos = self.cur.pos;
            e = match &self.cur.tok {
                Tok::Punct(".") => {
                    self.advance()?;
                    let (at, name) = self.property()?;
                    Expr::Member(at, Box::new(e), name, false)
                }
                Tok::Punct("?.") => {
                    self.advance()?;
                    if self.is("(") {
                        let args = self.arguments()?;
                        Expr::Call(head_pos(&e, pos), Box::new(e), args, true)
                    } else if self.is("[") {
                        Expr::Index(pos, Box::new(e), Box::new(self.index()?), true)
                    } else {
                        let (at, name) = self.property()?;
                        Expr::Member(at, Box::new(e), name, true)
                    }
                }
                Tok::Punct("[") => Expr::Index(pos, Box::new(e), Box::new(self.index()?), false),
                Tok::Punct("(") => {
                    let args = self.arguments()?;
                    Expr::Call(head_pos(&e, pos), Box::new(e), args, false)
                }
                Tok::Template(..) => {
                    return syntax(
                        pos,
                        "a tagged template is not in the subset: call the function with a template string",
                    );
                }
                _ => return Ok(e),
            };
        }
    }

    fn property(&mut self) -> Parsed<(Pos, String)> {
        let pos = self.cur.pos;
        let Tok::Name(w) = &self.cur.tok else {
            return syntax(
                pos,
                format!(
                    "expected a property name before {}",
                    describe(&self.cur.tok)
                ),
            );
        };
        let w = w.clone();
        self.advance()?;
        Ok((pos, w))
    }

    fn index(&mut self) -> Parsed<Expr> {
        self.nested(|p| {
            p.advance()?;
            let key = p.expression()?;
            p.expect("]")?;
            Ok(key)
        })
    }

    fn arguments(&mut self) -> Parsed<Vec<Item>> {
        self.nested(|p| {
            p.advance()?;
            p.items(")")
        })
    }

    fn items(&mut self, close: &str) -> Parsed<Vec<Item>> {
        let mut items = Vec::new();
        while !self.eat(close)? {
            if self.is(",") {
                return syntax(
                    self.cur.pos,
                    "an array hole is not in the subset: write undefined",
                );
            }
            items.push(if self.eat("...")? {
                Item::Spread(self.assign()?)
            } else {
                Item::Value(self.assign()?)
            });
            if !self.is(close) {
                self.expect(",")?;
            }
        }
        Ok(items)
    }

    fn primary(&mut self) -> Parsed<Expr> {
        let pos = self.cur.pos;
        let word = match &self.cur.tok {
            Tok::Num(n) => {
                let n = *n;
                self.advance()?;
                return Ok(Expr::Num(n));
            }
            Tok::Str(s) => {
                let s = s.clone();
                self.advance()?;
                return Ok(Expr::Str(s));
            }
            Tok::Template(..) => return self.nested(Self::template),
            Tok::Punct("(") => {
                return self.nested(|p| {
                    p.advance()?;
                    let e = p.expression()?;
                    p.expect(")")?;
                    Ok(e)
                });
            }
            Tok::Punct("[") => {
                return self.nested(|p| {
                    p.advance()?;
                    Ok(Expr::Array(p.items("]")?))
                });
            }
            Tok::Punct("{") => return self.nested(Self::object),
            Tok::Punct("/" | "/=") => return syntax(pos, REGEX),
            Tok::Name(w) => w.clone(),
            Tok::Eof => return syntax(pos, "the code ends early"),
            Tok::Punct(_) => return self.unexpected(),
        };
        let e = match word.as_str() {
            "true" => Expr::Bool(true),
            "false" => Expr::Bool(false),
            "null" => Expr::Null,
            "function" => return Ok(Expr::Function(Box::new(self.function(false, false)?))),
            "async" if self.async_function()? => {
                self.advance()?;
                return Ok(Expr::Function(Box::new(self.function(true, false)?)));
            }
            "new" => return self.new_error(),
            _ => return Ok(Expr::Name(pos, self.name()?.1)),
        };
        self.advance()?;
        Ok(e)
    }

    fn new_error(&mut self) -> Parsed<Expr> {
        let pos = self.cur.pos;
        self.advance()?;
        if self.is_word("Error") && self.peek()?.tok == Tok::Punct("(") {
            let at = self.cur.pos;
            self.advance()?;
            let args = self.arguments()?;
            return Ok(Expr::Call(
                at,
                Box::new(Expr::Name(at, "Error".into())),
                args,
                false,
            ));
        }
        syntax(
            pos,
            "new is not in the subset but for new Error(message): use a function that returns an object",
        )
    }

    fn template(&mut self) -> Parsed<Expr> {
        let mut texts = Vec::new();
        let mut values = Vec::new();
        loop {
            let Tok::Template(text, tail) = &self.cur.tok else {
                return self.unexpected();
            };
            let tail = *tail;
            texts.push(text.clone());
            self.advance()?;
            if tail {
                return Ok(Expr::Template(texts, values));
            }
            values.push(self.expression()?);
            if !self.is("}") {
                return syntax(
                    self.cur.pos,
                    format!(
                        "expected }} to close ${{ before {}",
                        describe(&self.cur.tok)
                    ),
                );
            }
            self.cur = self.lex.template_rest()?;
        }
    }

    fn object(&mut self) -> Parsed<Expr> {
        self.advance()?;
        let mut props = Vec::new();
        while !self.eat("}")? {
            props.push(self.prop()?);
            if !self.is("}") {
                self.expect(",")?;
            }
        }
        Ok(Expr::Object(props))
    }

    fn prop(&mut self) -> Parsed<Prop> {
        let pos = self.cur.pos;
        if self.eat("...")? {
            return Ok(Prop::Spread(self.assign()?));
        }
        if self.is("*") {
            return syntax(
                pos,
                "a generator (function*) is not in the subset: return an array",
            );
        }
        let next = self.peek()?;
        let mut is_async = false;
        if Self::starts_key(&next.tok) || next.tok == Tok::Punct("*") {
            if self.is_word("get") {
                return syntax(
                    pos,
                    "a getter (get x() {}) is not in the subset: use a plain property",
                );
            }
            if self.is_word("set") {
                return syntax(
                    pos,
                    "a setter (set x(v) {}) is not in the subset: assign the property",
                );
            }
            if self.is_word("async") && !next.newline_before {
                self.advance()?;
                is_async = true;
                if self.is("*") {
                    return syntax(
                        pos,
                        "a generator (function*) is not in the subset: return an array",
                    );
                }
            }
        }
        let key_pos = self.cur.pos;
        let (key, shorthand) = self.key()?;
        if self.is("(") {
            let name = match &key {
                Key::Name(n) => Some(n.clone()),
                Key::Computed(_) => None,
            };
            let f = self.function_rest(key_pos, name, is_async)?;
            return Ok(Prop::Value(key, Expr::Function(Box::new(f))));
        }
        if is_async {
            return self.expected("(");
        }
        if self.eat(":")? {
            return Ok(Prop::Value(key, self.assign()?));
        }
        match shorthand {
            Some(name) if self.is(",") || self.is("}") => {
                Self::check_name(key_pos, &name)?;
                Ok(Prop::Value(key, Expr::Name(key_pos, name)))
            }
            _ => self.expected(":"),
        }
    }
}
