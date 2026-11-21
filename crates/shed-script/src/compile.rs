use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::{iter, mem, slice};

use crate::ScriptError;
use crate::ast::{
    Assign, Binary, Binding, Decl, Expr, Function, Item, Key, Logic, Pattern, Pos, Prop, Script,
    Stmt, Unary,
};
use crate::value::Builtin;
use crate::{builtins, syntax};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Op {
    Num(f64),
    Str(u32),
    Undefined,
    Null,
    Bool(bool),
    Pop,
    Dup,
    Dup2,
    Nip,
    Reg(u32),
    SetReg(u32),
    Load(u32, u32),
    Store(u32, u32),
    Global(u32),
    ConstSet(u32),
    Enter(u32),
    Leave,
    Renew,
    Closure(u32),
    Jump(u32),
    IfFalse(u32),
    IfTrue(u32),
    IfSome(u32),
    IfDefined(u32),
    Opt(u32),
    Not,
    Neg,
    Plus,
    Typeof,
    Inc(f64),
    NullCheck(bool),
    Binary(Binary),
    Get(u32),
    Set(u32),
    GetIndex,
    SetIndex,
    NewArray,
    Append,
    Spread,
    NewObject,
    Field(u32),
    FieldIndex,
    SpreadObject,
    Call(u32),
    CallSpread,
    Return,
    Throw,
    Try(u32),
    EndTry,
    Template(u32),
    Iter,
    Next(u32),
    Item(u32),
    RestArray(u32),
    RestObject(u32),
    Destructure(bool),
    NewError(u32),
    Native(Builtin),
}

#[derive(Debug, Default)]
pub(crate) struct Code {
    pub(crate) ops: Vec<Op>,
    pub(crate) at: Vec<Pos>,
    pub(crate) params: usize,
    pub(crate) rest: bool,
    pub(crate) locals: usize,
}

#[derive(Debug, Default)]
pub(crate) struct Compiled {
    pub(crate) code: Vec<Code>,
    pub(crate) strings: Vec<Arc<str>>,
}

pub(crate) fn n(at: usize) -> u32 {
    u32::try_from(at).unwrap_or(u32::MAX)
}

#[derive(Clone, Copy)]
enum Slot {
    Reg(u32),
    Heap(u32),
}

struct Bound {
    name: String,
    slot: Slot,
    constant: bool,
}

struct Block {
    names: Vec<Bound>,
    heap: bool,
}

struct Loop {
    blocks: usize,
    tries: usize,
    breaks: Vec<usize>,
    continues: Vec<usize>,
}

#[derive(Default)]
struct Func {
    code: Code,
    blocks: Vec<Block>,
    captured: HashSet<String>,
    regs: usize,
    loops: Vec<Loop>,
    tries: usize,
}

type Done = Result<(), ScriptError>;

pub(crate) struct Compiler {
    out: Compiled,
    cur: Func,
    outer: Vec<Func>,
    strings: HashMap<String, u32>,
    pos: Pos,
}

struct Refs<'a> {
    depth: usize,
    names: &'a mut HashSet<String>,
}

impl Refs<'_> {
    fn stmts(&mut self, body: &[Stmt]) {
        for s in body {
            self.stmt(s);
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Decl(_, bindings) => self.bindings(bindings),
            Stmt::Function(f) => self.function(f),
            Stmt::Expr(e) | Stmt::Return(_, Some(e)) | Stmt::Throw(_, e) => self.expr(e),
            Stmt::Block(body) => self.stmts(body),
            Stmt::If(c, a, b) => {
                self.expr(c);
                self.stmt(a);
                if let Some(b) = b {
                    self.stmt(b);
                }
            }
            Stmt::ForOf(_, p, e, body) => {
                self.pattern(p);
                self.expr(e);
                self.stmt(body);
            }
            Stmt::For(init, test, update, body) => {
                if let Some(init) = init {
                    self.stmt(init);
                }
                test.iter().chain(update).for_each(|e| self.expr(e));
                self.stmt(body);
            }
            Stmt::While(c, body) => {
                self.expr(c);
                self.stmt(body);
            }
            Stmt::Try(body, p, handler) => {
                self.stmts(body);
                if let Some(p) = p {
                    self.pattern(p);
                }
                self.stmts(handler);
            }
            Stmt::Break(_) | Stmt::Continue(_) | Stmt::Return(_, None) | Stmt::Empty => {}
        }
    }

    fn bindings(&mut self, bindings: &[Binding]) {
        for (p, e) in bindings {
            self.pattern(p);
            if let Some(e) = e {
                self.expr(e);
            }
        }
    }

    fn pattern(&mut self, p: &Pattern) {
        match p {
            Pattern::Name(..) => {}
            Pattern::Object(props, rest) => {
                for (key, binding) in props {
                    if let Key::Computed(e) = key {
                        self.expr(e);
                    }
                    self.bindings(slice::from_ref(binding));
                }
                if let Some(rest) = rest {
                    self.pattern(rest);
                }
            }
            Pattern::Array(items, rest) => {
                for binding in items.iter().flatten() {
                    self.bindings(slice::from_ref(binding));
                }
                if let Some(rest) = rest {
                    self.pattern(rest);
                }
            }
        }
    }

    fn function(&mut self, f: &Function) {
        self.depth += 1;
        self.bindings(&f.params);
        if let Some(rest) = &f.rest {
            self.pattern(rest);
        }
        self.stmts(&f.body);
        self.depth -= 1;
    }

    fn items(&mut self, items: &[Item]) {
        for item in items {
            let (Item::Value(e) | Item::Spread(e)) = item;
            self.expr(e);
        }
    }

    fn expr(&mut self, e: &Expr) {
        match e {
            Expr::Name(_, name) => {
                if self.depth > 0 {
                    self.names.insert(name.clone());
                }
            }
            Expr::Template(_, values) => values.iter().for_each(|e| self.expr(e)),
            Expr::Array(items) => self.items(items),
            Expr::Object(props) => {
                for prop in props {
                    match prop {
                        Prop::Value(key, e) => {
                            if let Key::Computed(k) = key {
                                self.expr(k);
                            }
                            self.expr(e);
                        }
                        Prop::Spread(e) => self.expr(e),
                    }
                }
            }
            Expr::Function(f) => self.function(f),
            Expr::Unary(_, _, x)
            | Expr::Update(_, _, _, x)
            | Expr::NullCheck(_, _, x)
            | Expr::Member(_, x, _, _)
            | Expr::Await(_, x) => self.expr(x),
            Expr::Binary(_, _, a, b)
            | Expr::Logic(_, _, a, b)
            | Expr::Assign(_, _, a, b)
            | Expr::Index(_, a, b, _) => {
                self.expr(a);
                self.expr(b);
            }
            Expr::Cond(c, a, b) => {
                self.expr(c);
                self.expr(a);
                self.expr(b);
            }
            Expr::Call(_, callee, args, _) => {
                self.expr(callee);
                self.items(args);
            }
            Expr::Num(_) | Expr::Str(_) | Expr::Bool(_) | Expr::Null => {}
        }
    }
}

struct Declared {
    pos: Pos,
    name: String,
    constant: bool,
    preset: Option<u32>,
}

fn declared(pos: Pos, name: &str, constant: bool, preset: Option<u32>) -> Declared {
    Declared {
        pos,
        name: name.into(),
        constant,
        preset,
    }
}

fn pattern_names(p: &Pattern, constant: bool, out: &mut Vec<Declared>) {
    match p {
        Pattern::Name(pos, name) => out.push(declared(*pos, name, constant, None)),
        Pattern::Object(props, rest) => {
            for (_, (p, _)) in props {
                pattern_names(p, constant, out);
            }
            if let Some(rest) = rest {
                pattern_names(rest, constant, out);
            }
        }
        Pattern::Array(items, rest) => {
            for (p, _) in items.iter().flatten() {
                pattern_names(p, constant, out);
            }
            if let Some(rest) = rest {
                pattern_names(rest, constant, out);
            }
        }
    }
}

fn binary(op: Assign) -> Option<Binary> {
    Some(match op {
        Assign::Add => Binary::Add,
        Assign::Sub => Binary::Sub,
        Assign::Mul => Binary::Mul,
        Assign::Div => Binary::Div,
        Assign::Set | Assign::And | Assign::Or | Assign::Nullish => return None,
    })
}

fn test(op: Logic) -> Op {
    match op {
        Logic::And => Op::IfFalse(0),
        Logic::Or => Op::IfTrue(0),
        Logic::Nullish => Op::IfSome(0),
    }
}

fn logic(op: Assign) -> Option<Logic> {
    match op {
        Assign::And => Some(Logic::And),
        Assign::Or => Some(Logic::Or),
        Assign::Nullish => Some(Logic::Nullish),
        _ => None,
    }
}

fn is_error_call(callee: &Expr) -> bool {
    matches!(callee, Expr::Name(_, name) if name == "Error")
}

impl Compiler {
    pub(crate) fn program(script: &Script) -> Result<Compiled, ScriptError> {
        let mut c = Self {
            out: Compiled::default(),
            cur: Func::default(),
            outer: Vec::new(),
            strings: HashMap::new(),
            pos: Pos { line: 1, column: 1 },
        };
        c.out.code.push(Code::default());
        let code = c.body(&[], None, &script.body)?;
        if let Some(main) = c.out.code.first_mut() {
            *main = code;
        }
        Ok(c.out)
    }

    fn func(&mut self) -> &mut Func {
        &mut self.cur
    }

    fn emit(&mut self, op: Op) {
        let pos = self.pos;
        let code = &mut self.func().code;
        code.ops.push(op);
        code.at.push(pos);
    }

    fn jump(&mut self, op: Op) -> usize {
        self.emit(op);
        self.func().code.ops.len() - 1
    }

    fn here(&mut self) -> u32 {
        n(self.func().code.ops.len())
    }

    fn patch(&mut self, at: usize) {
        let to = self.here();
        self.patch_to(at, to);
    }

    fn patch_to(&mut self, at: usize, to: u32) {
        if let Some(
            Op::Jump(t)
            | Op::IfFalse(t)
            | Op::IfTrue(t)
            | Op::IfSome(t)
            | Op::IfDefined(t)
            | Op::Opt(t)
            | Op::Try(t)
            | Op::Next(t),
        ) = self.func().code.ops.get_mut(at)
        {
            *t = to;
        }
    }

    fn string(&mut self, s: &str) -> u32 {
        if let Some(&at) = self.strings.get(s) {
            return at;
        }
        let at = n(self.out.strings.len());
        self.out.strings.push(s.into());
        self.strings.insert(s.into(), at);
        at
    }

    fn reg(&mut self) -> u32 {
        let f = self.func();
        f.regs += 1;
        f.code.locals = f.code.locals.max(f.regs);
        n(f.regs - 1)
    }

    fn body(
        &mut self,
        params: &[Binding],
        rest: Option<&Pattern>,
        body: &[Stmt],
    ) -> Result<Code, ScriptError> {
        let mut captured = HashSet::new();
        let mut refs = Refs {
            depth: 0,
            names: &mut captured,
        };
        refs.bindings(params);
        if let Some(rest) = rest {
            refs.pattern(rest);
        }
        refs.stmts(body);
        let args = params.len() + usize::from(rest.is_some());
        let parent = mem::replace(
            &mut self.cur,
            Func {
                code: Code {
                    params: params.len(),
                    rest: rest.is_some(),
                    locals: args,
                    ..Code::default()
                },
                blocks: Vec::new(),
                captured,
                regs: args,
                loops: Vec::new(),
                tries: 0,
            },
        );
        self.outer.push(parent);
        let mut names = Vec::new();
        for (at, (p, default)) in params.iter().enumerate() {
            match p {
                Pattern::Name(pos, name) if default.is_none() => {
                    names.push(declared(*pos, name, false, Some(n(at))));
                }
                _ => pattern_names(p, false, &mut names),
            }
        }
        if let Some(rest) = rest {
            match rest {
                Pattern::Name(pos, name) => {
                    names.push(declared(*pos, name, false, Some(n(params.len()))));
                }
                _ => pattern_names(rest, false, &mut names),
            }
        }
        self.open(names, body)?;
        for (at, (p, default)) in params.iter().enumerate() {
            self.param(n(at), p, default.as_ref())?;
        }
        if let Some(rest) = rest {
            self.param(n(params.len()), rest, None)?;
        }
        self.stmts(body)?;
        self.emit(Op::Undefined);
        self.emit(Op::Return);
        let parent = self.outer.pop().unwrap_or_default();
        Ok(mem::replace(&mut self.cur, parent).code)
    }

    fn param(&mut self, at: u32, p: &Pattern, default: Option<&Expr>) -> Done {
        if let (Pattern::Name(_, name), None) = (p, default)
            && matches!(self.resolve(name), Some((Slot::Reg(r), 0, _)) if r == at)
        {
            return Ok(());
        }
        self.emit(Op::Reg(at));
        self.default(default)?;
        self.bind(p)
    }

    fn function(&mut self, f: &Function) -> Result<u32, ScriptError> {
        self.pos = f.pos;
        let at = self.out.code.len();
        self.out.code.push(Code::default());
        let code = self.body(&f.params, f.rest.as_ref(), &f.body)?;
        if let Some(slot) = self.out.code.get_mut(at) {
            *slot = code;
        }
        Ok(n(at))
    }

    fn open(&mut self, mut names: Vec<Declared>, body: &[Stmt]) -> Done {
        for s in body {
            match s {
                Stmt::Decl(decl, bindings) => {
                    for (p, _) in bindings {
                        pattern_names(p, *decl == Decl::Const, &mut names);
                    }
                }
                Stmt::Function(f) => {
                    if let Some(name) = &f.name {
                        names.push(declared(f.pos, name, false, None));
                    }
                }
                _ => {}
            }
        }
        let mut block = Block {
            names: Vec::new(),
            heap: false,
        };
        let mut slots = 0;
        for Declared {
            pos,
            name,
            constant,
            preset,
        } in names
        {
            if block.names.iter().any(|b| b.name == name) {
                return syntax(pos, format!("{name} is declared twice in one block"));
            }
            let slot = if self.func().captured.contains(&name) {
                slots += 1;
                Slot::Heap(slots - 1)
            } else {
                match preset {
                    Some(r) => Slot::Reg(r),
                    None => Slot::Reg(self.reg()),
                }
            };
            block.names.push(Bound {
                name,
                slot,
                constant,
            });
        }
        if slots > 0 {
            self.emit(Op::Enter(slots));
            block.heap = true;
        }
        self.func().blocks.push(block);
        for s in body {
            if let Stmt::Function(f) = s
                && let Some(name) = &f.name
            {
                let at = self.function(f)?;
                self.emit(Op::Closure(at));
                self.init(name);
            }
        }
        Ok(())
    }

    fn close(&mut self) {
        if self.func().blocks.pop().is_some_and(|b| b.heap) {
            self.emit(Op::Leave);
        }
    }

    fn block(&mut self, body: &[Stmt]) -> Done {
        self.open(Vec::new(), body)?;
        self.stmts(body)?;
        self.close();
        Ok(())
    }

    fn scoped(&mut self, s: &Stmt) -> Done {
        match s {
            Stmt::Block(body) => self.block(body),
            _ => self.block(slice::from_ref(s)),
        }
    }

    fn stmts(&mut self, body: &[Stmt]) -> Done {
        body.iter().try_for_each(|s| self.stmt(s))
    }

    fn stmt(&mut self, s: &Stmt) -> Done {
        match s {
            Stmt::Decl(_, bindings) => {
                for (p, init) in bindings {
                    match init {
                        Some(e) => self.expr(e)?,
                        None => self.emit(Op::Undefined),
                    }
                    self.bind(p)?;
                }
                Ok(())
            }
            Stmt::Function(_) | Stmt::Empty => Ok(()),
            Stmt::Expr(e) => self.effect(e),
            Stmt::Block(body) => self.block(body),
            Stmt::If(c, a, b) => {
                self.expr(c)?;
                let skip = self.jump(Op::IfFalse(0));
                self.scoped(a)?;
                if let Some(b) = b {
                    let end = self.jump(Op::Jump(0));
                    self.patch(skip);
                    self.scoped(b)?;
                    self.patch(end);
                } else {
                    self.patch(skip);
                }
                Ok(())
            }
            Stmt::While(c, body) => {
                let top = self.here();
                self.enter_loop();
                self.expr(c)?;
                let exit = self.jump(Op::IfFalse(0));
                self.scoped(body)?;
                self.emit(Op::Jump(top));
                self.patch(exit);
                let end = self.here();
                self.exit_loop(top, end);
                Ok(())
            }
            Stmt::For(init, test, update, body) => {
                self.for_loop(init.as_deref(), test.as_ref(), update.as_ref(), body)
            }
            Stmt::ForOf(decl, p, iterable, body) => self.for_of(*decl, p, iterable, body),
            Stmt::Break(pos) | Stmt::Continue(pos) => {
                self.pos = *pos;
                self.jump_out(matches!(s, Stmt::Break(_)));
                Ok(())
            }
            Stmt::Return(pos, e) => {
                self.pos = *pos;
                match e {
                    Some(e) => self.expr(e)?,
                    None => self.emit(Op::Undefined),
                }
                self.pos = *pos;
                self.emit(Op::Return);
                Ok(())
            }
            Stmt::Throw(pos, e) => {
                self.expr(e)?;
                self.pos = *pos;
                self.emit(Op::Throw);
                Ok(())
            }
            Stmt::Try(body, p, handler) => self.try_catch(body, p.as_ref(), handler),
        }
    }

    fn enter_loop(&mut self) {
        let f = self.func();
        let (blocks, tries) = (f.blocks.len(), f.tries);
        f.loops.push(Loop {
            blocks,
            tries,
            breaks: Vec::new(),
            continues: Vec::new(),
        });
    }

    fn exit_loop(&mut self, continue_to: u32, break_to: u32) {
        if let Some(l) = self.func().loops.pop() {
            for at in l.continues {
                self.patch_to(at, continue_to);
            }
            for at in l.breaks {
                self.patch_to(at, break_to);
            }
        }
    }

    fn jump_out(&mut self, is_break: bool) {
        let f = self.func();
        let Some(l) = f.loops.last() else {
            return;
        };
        let leaves = f.blocks.iter().skip(l.blocks).filter(|b| b.heap).count();
        let tries = f.tries.saturating_sub(l.tries);
        for _ in 0..leaves {
            self.emit(Op::Leave);
        }
        for _ in 0..tries {
            self.emit(Op::EndTry);
        }
        let at = self.jump(Op::Jump(0));
        if let Some(l) = self.func().loops.last_mut() {
            if is_break {
                &mut l.breaks
            } else {
                &mut l.continues
            }
            .push(at);
        }
    }

    fn for_loop(
        &mut self,
        init: Option<&Stmt>,
        test: Option<&Expr>,
        update: Option<&Expr>,
        body: &Stmt,
    ) -> Done {
        self.open(Vec::new(), init.map(slice::from_ref).unwrap_or_default())?;
        let renew = self.func().blocks.last().is_some_and(|b| b.heap);
        if let Some(init) = init {
            self.stmt(init)?;
        }
        let top = self.here();
        self.enter_loop();
        let exit = match test {
            Some(test) => {
                self.expr(test)?;
                Some(self.jump(Op::IfFalse(0)))
            }
            None => None,
        };
        self.scoped(body)?;
        let next = self.here();
        if renew {
            self.emit(Op::Renew);
        }
        if let Some(update) = update {
            self.effect(update)?;
        }
        self.emit(Op::Jump(top));
        if let Some(exit) = exit {
            self.patch(exit);
        }
        let end = self.here();
        self.exit_loop(next, end);
        self.close();
        Ok(())
    }

    fn for_of(&mut self, decl: Decl, p: &Pattern, iterable: &Expr, body: &Stmt) -> Done {
        self.expr(iterable)?;
        self.emit(Op::Iter);
        let top = self.here();
        let exit = self.jump(Op::Next(0));
        self.enter_loop();
        let mut names = Vec::new();
        pattern_names(p, decl == Decl::Const, &mut names);
        self.open(names, &[])?;
        self.bind(p)?;
        self.scoped(body)?;
        self.close();
        let next = self.here();
        self.emit(Op::Jump(top));
        let pops = self.here();
        self.emit(Op::Pop);
        self.emit(Op::Pop);
        self.exit_loop(next, pops);
        self.patch(exit);
        Ok(())
    }

    fn try_catch(&mut self, body: &[Stmt], p: Option<&Pattern>, handler: &[Stmt]) -> Done {
        let handle = self.jump(Op::Try(0));
        self.func().tries += 1;
        self.block(body)?;
        self.func().tries -= 1;
        self.emit(Op::EndTry);
        let end = self.jump(Op::Jump(0));
        self.patch(handle);
        let mut names = Vec::new();
        if let Some(p) = p {
            pattern_names(p, false, &mut names);
        }
        self.open(names, handler)?;
        match p {
            Some(p) => self.bind(p)?,
            None => self.emit(Op::Pop),
        }
        self.stmts(handler)?;
        self.close();
        self.patch(end);
        Ok(())
    }

    fn resolve(&self, name: &str) -> Option<(Slot, u32, bool)> {
        let mut hops = 0;
        for (depth, f) in iter::once(&self.cur)
            .chain(self.outer.iter().rev())
            .enumerate()
        {
            for block in f.blocks.iter().rev() {
                if let Some(b) = block.names.iter().find(|b| b.name == name) {
                    if depth > 0 && matches!(b.slot, Slot::Reg(_)) {
                        return None;
                    }
                    return Some((b.slot, hops, b.constant));
                }
                hops += u32::from(block.heap);
            }
        }
        None
    }

    fn load(&mut self, name: &str) {
        let op = match self.resolve(name) {
            Some((Slot::Reg(r), ..)) => Op::Reg(r),
            Some((Slot::Heap(slot), hops, _)) => Op::Load(hops, slot),
            None => match name {
                "undefined" => Op::Undefined,
                "NaN" => Op::Num(f64::NAN),
                "Infinity" => Op::Num(f64::INFINITY),
                _ => {
                    builtins::global(name).map_or_else(|| Op::Global(self.string(name)), Op::Native)
                }
            },
        };
        self.emit(op);
    }

    fn store(&mut self, name: &str, init: bool) {
        let op = match self.resolve(name) {
            Some((_, _, true)) if !init => Op::ConstSet(self.string(name)),
            Some((Slot::Reg(r), ..)) => Op::SetReg(r),
            Some((Slot::Heap(slot), hops, _)) => Op::Store(hops, slot),
            None if builtins::global(name).is_some() => Op::ConstSet(self.string(name)),
            None => Op::Global(self.string(name)),
        };
        self.emit(op);
    }

    fn init(&mut self, name: &str) {
        self.store(name, true);
    }

    fn default(&mut self, default: Option<&Expr>) -> Done {
        if let Some(default) = default {
            self.emit(Op::Dup);
            let skip = self.jump(Op::IfDefined(0));
            self.emit(Op::Pop);
            self.expr(default)?;
            self.patch(skip);
        }
        Ok(())
    }

    fn bind(&mut self, p: &Pattern) -> Done {
        match p {
            Pattern::Name(pos, name) => {
                self.pos = *pos;
                self.init(name);
            }
            Pattern::Object(props, rest) => {
                self.emit(Op::Destructure(false));
                let mut keys = Vec::new();
                for (key, (sub, default)) in props {
                    self.emit(Op::Dup);
                    match key {
                        Key::Name(k) => {
                            let k = self.string(k);
                            self.emit(Op::Get(k));
                            keys.push(Op::Str(k));
                        }
                        Key::Computed(e) => {
                            self.expr(e)?;
                            if rest.is_some() {
                                let t = self.reg();
                                self.emit(Op::Dup);
                                self.emit(Op::SetReg(t));
                                keys.push(Op::Reg(t));
                            }
                            self.emit(Op::GetIndex);
                        }
                    }
                    self.default(default.as_ref())?;
                    self.bind(sub)?;
                }
                match rest {
                    Some(rest) => {
                        let count = n(keys.len());
                        for k in keys {
                            self.emit(k);
                        }
                        self.emit(Op::RestObject(count));
                        self.bind(rest)?;
                    }
                    None => self.emit(Op::Pop),
                }
            }
            Pattern::Array(items, rest) => {
                self.emit(Op::Destructure(true));
                for (at, item) in items.iter().enumerate() {
                    if let Some((sub, default)) = item {
                        self.emit(Op::Item(n(at)));
                        self.default(default.as_ref())?;
                        self.bind(sub)?;
                    }
                }
                if let Some(rest) = rest {
                    self.emit(Op::RestArray(n(items.len())));
                    self.bind(rest)?;
                }
                self.emit(Op::Pop);
            }
        }
        Ok(())
    }

    fn effect(&mut self, e: &Expr) -> Done {
        match e {
            Expr::Assign(pos, op, target, value) => self.assign(*pos, *op, target, value, false),
            Expr::Update(pos, prefix, inc, target) => {
                self.update(*pos, *prefix, *inc, target, false)
            }
            _ => {
                self.expr(e)?;
                self.emit(Op::Pop);
                Ok(())
            }
        }
    }

    fn items(&mut self, items: &[Item]) -> Done {
        for item in items {
            match item {
                Item::Value(e) => {
                    self.expr(e)?;
                    self.emit(Op::Append);
                }
                Item::Spread(e) => {
                    self.expr(e)?;
                    self.emit(Op::Spread);
                }
            }
        }
        Ok(())
    }

    fn expr(&mut self, e: &Expr) -> Done {
        match e {
            Expr::Num(x) => self.emit(Op::Num(*x)),
            Expr::Str(s) => {
                let s = self.string(s);
                self.emit(Op::Str(s));
            }
            Expr::Template(texts, values) => self.template(texts, values)?,
            Expr::Bool(b) => self.emit(Op::Bool(*b)),
            Expr::Null => self.emit(Op::Null),
            Expr::Name(pos, name) => {
                self.pos = *pos;
                self.load(name);
            }
            Expr::Array(items) => {
                self.emit(Op::NewArray);
                self.items(items)?;
            }
            Expr::Object(props) => self.object(props)?,
            Expr::Function(f) => {
                let at = self.function(f)?;
                self.emit(Op::Closure(at));
            }
            Expr::Unary(pos, op, x) => {
                if let (Unary::Typeof, Expr::Name(_, name)) = (op, &**x)
                    && self.resolve(name).is_none()
                    && builtins::global(name).is_none()
                    && !matches!(name.as_str(), "undefined" | "NaN" | "Infinity")
                {
                    let s = self.string("undefined");
                    self.emit(Op::Str(s));
                    return Ok(());
                }
                self.expr(x)?;
                self.pos = *pos;
                self.emit(match op {
                    Unary::Not => Op::Not,
                    Unary::Neg => Op::Neg,
                    Unary::Plus => Op::Plus,
                    Unary::Typeof => Op::Typeof,
                });
            }
            Expr::Update(pos, prefix, inc, target) => {
                self.update(*pos, *prefix, *inc, target, true)?;
            }
            Expr::Binary(pos, op, a, b) => {
                self.expr(a)?;
                self.expr(b)?;
                self.pos = *pos;
                self.emit(Op::Binary(*op));
            }
            Expr::NullCheck(pos, ne, x) => {
                self.expr(x)?;
                self.pos = *pos;
                self.emit(Op::NullCheck(*ne));
            }
            Expr::Logic(pos, op, a, b) => {
                self.expr(a)?;
                self.pos = *pos;
                self.emit(Op::Dup);
                let end = self.jump(test(*op));
                self.emit(Op::Pop);
                self.expr(b)?;
                self.patch(end);
            }
            Expr::Cond(c, a, b) => {
                self.expr(c)?;
                let other = self.jump(Op::IfFalse(0));
                self.expr(a)?;
                let end = self.jump(Op::Jump(0));
                self.patch(other);
                self.expr(b)?;
                self.patch(end);
            }
            Expr::Assign(pos, op, target, value) => self.assign(*pos, *op, target, value, true)?,
            Expr::Member(..) | Expr::Index(..) | Expr::Call(..) => {
                let mut exits = Vec::new();
                self.link(e, &mut exits)?;
                for at in exits {
                    self.patch(at);
                }
            }
            Expr::Await(_, x) => self.expr(x)?,
        }
        Ok(())
    }

    fn template(&mut self, texts: &[String], values: &[Expr]) -> Done {
        let mut parts = 0;
        for (at, text) in texts.iter().enumerate() {
            if !text.is_empty() {
                let s = self.string(text);
                self.emit(Op::Str(s));
                parts += 1;
            }
            if let Some(value) = values.get(at) {
                self.expr(value)?;
                parts += 1;
            }
        }
        self.emit(Op::Template(parts));
        Ok(())
    }

    fn object(&mut self, props: &[Prop]) -> Done {
        self.emit(Op::NewObject);
        for prop in props {
            match prop {
                Prop::Value(Key::Name(k), value) => {
                    self.expr(value)?;
                    let k = self.string(k);
                    self.emit(Op::Field(k));
                }
                Prop::Value(Key::Computed(k), value) => {
                    self.expr(k)?;
                    self.expr(value)?;
                    self.emit(Op::FieldIndex);
                }
                Prop::Spread(value) => {
                    self.expr(value)?;
                    self.emit(Op::SpreadObject);
                }
            }
        }
        Ok(())
    }

    fn link(&mut self, e: &Expr, exits: &mut Vec<usize>) -> Done {
        match e {
            Expr::Member(pos, object, name, optional) => {
                self.link(object, exits)?;
                self.pos = *pos;
                if *optional {
                    exits.push(self.jump(Op::Opt(0)));
                }
                let name = self.string(name);
                self.emit(Op::Get(name));
            }
            Expr::Index(pos, object, key, optional) => {
                self.link(object, exits)?;
                self.pos = *pos;
                if *optional {
                    exits.push(self.jump(Op::Opt(0)));
                }
                self.expr(key)?;
                self.pos = *pos;
                self.emit(Op::GetIndex);
            }
            Expr::Call(pos, callee, args, optional) => {
                let spread = args.iter().any(|a| matches!(a, Item::Spread(_)));
                if is_error_call(callee) && self.resolve("Error").is_none() && !spread {
                    for arg in args {
                        let (Item::Value(arg) | Item::Spread(arg)) = arg;
                        self.expr(arg)?;
                    }
                    self.pos = *pos;
                    self.emit(Op::NewError(n(args.len())));
                    return Ok(());
                }
                self.link(callee, exits)?;
                self.pos = *pos;
                if *optional {
                    exits.push(self.jump(Op::Opt(0)));
                }
                if spread {
                    self.emit(Op::NewArray);
                    self.items(args)?;
                    self.pos = *pos;
                    self.emit(Op::CallSpread);
                } else {
                    for arg in args {
                        let (Item::Value(arg) | Item::Spread(arg)) = arg;
                        self.expr(arg)?;
                    }
                    self.pos = *pos;
                    self.emit(Op::Call(n(args.len())));
                }
            }
            _ => self.expr(e)?,
        }
        Ok(())
    }

    fn assign(&mut self, pos: Pos, op: Assign, target: &Expr, value: &Expr, used: bool) -> Done {
        self.pos = pos;
        match target {
            Expr::Name(_, name) => {
                if let Some(l) = logic(op) {
                    self.load(name);
                    self.emit(Op::Dup);
                    let end = self.jump(test(l));
                    self.emit(Op::Pop);
                    self.expr(value)?;
                    self.emit(Op::Dup);
                    self.pos = pos;
                    self.store(name, false);
                    self.patch(end);
                    if !used {
                        self.emit(Op::Pop);
                    }
                    return Ok(());
                }
                if let Some(b) = binary(op) {
                    self.load(name);
                    self.expr(value)?;
                    self.pos = pos;
                    self.emit(Op::Binary(b));
                } else {
                    self.expr(value)?;
                }
                if used {
                    self.emit(Op::Dup);
                }
                self.pos = pos;
                self.store(name, false);
                return Ok(());
            }
            Expr::Member(_, object, name, _) => {
                self.expr(object)?;
                let name = self.string(name);
                self.modify(pos, op, (Op::Dup, Op::Get(name), Op::Set(name)), 1, value)?;
            }
            Expr::Index(_, object, key, _) => {
                self.expr(object)?;
                self.expr(key)?;
                self.modify(pos, op, (Op::Dup2, Op::GetIndex, Op::SetIndex), 2, value)?;
            }
            _ => return syntax(pos, "the left of = is not a name or a property"),
        }
        if !used {
            self.emit(Op::Pop);
        }
        Ok(())
    }

    fn modify(
        &mut self,
        pos: Pos,
        op: Assign,
        (dup, get, set): (Op, Op, Op),
        held: usize,
        value: &Expr,
    ) -> Done {
        self.pos = pos;
        if op == Assign::Set {
            self.expr(value)?;
            self.pos = pos;
            self.emit(set);
            return Ok(());
        }
        self.emit(dup);
        self.emit(get);
        if let Some(l) = logic(op) {
            self.emit(Op::Dup);
            let skip = self.jump(test(l));
            self.emit(Op::Pop);
            self.expr(value)?;
            self.pos = pos;
            self.emit(set);
            let end = self.jump(Op::Jump(0));
            self.patch(skip);
            for _ in 0..held {
                self.emit(Op::Nip);
            }
            self.patch(end);
            return Ok(());
        }
        self.expr(value)?;
        self.pos = pos;
        if let Some(b) = binary(op) {
            self.emit(Op::Binary(b));
        }
        self.emit(set);
        Ok(())
    }

    fn update(&mut self, pos: Pos, prefix: bool, inc: bool, target: &Expr, used: bool) -> Done {
        let delta = if inc { 1.0 } else { -1.0 };
        let old = used && !prefix;
        self.pos = pos;
        let (get, set) = match target {
            Expr::Name(_, name) => {
                self.load(name);
                if old {
                    self.emit(Op::Inc(0.0));
                    self.emit(Op::Dup);
                }
                self.emit(Op::Inc(delta));
                if used && prefix {
                    self.emit(Op::Dup);
                }
                self.store(name, false);
                return Ok(());
            }
            Expr::Member(_, object, name, _) => {
                self.expr(object)?;
                self.pos = pos;
                self.emit(Op::Dup);
                let name = self.string(name);
                (Op::Get(name), Op::Set(name))
            }
            Expr::Index(_, object, key, _) => {
                self.expr(object)?;
                self.expr(key)?;
                self.pos = pos;
                self.emit(Op::Dup2);
                (Op::GetIndex, Op::SetIndex)
            }
            _ => return syntax(pos, "the target of ++ or -- is not a name or a property"),
        };
        self.emit(get);
        let saved = if old {
            let t = self.reg();
            self.emit(Op::Inc(0.0));
            self.emit(Op::Dup);
            self.emit(Op::SetReg(t));
            Some(t)
        } else {
            None
        };
        self.emit(Op::Inc(delta));
        self.emit(set);
        if let Some(t) = saved {
            self.emit(Op::Pop);
            self.emit(Op::Reg(t));
        } else if !used {
            self.emit(Op::Pop);
        }
        Ok(())
    }
}
