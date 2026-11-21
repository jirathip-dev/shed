//! The machine: frames kept as data, the step loop in slices, fuel and depth, errors thrown and caught.

use std::cmp::Ordering;
use std::sync::Arc;

use serde_json::Value as Json;

use crate::ast::{Binary, Pos};
use crate::compile::{Code, Compiled, Op};
use crate::heap::{Full, Heap};
use crate::value::{Cell, Handle, Value};
use crate::{Program, ScriptError};

const SLICE: u64 = 10_000;
const JSON_DEPTH: usize = 200;

/// What a run may use before it is stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Instructions, one each.
    pub fuel: u64,
    /// Bytes held by live values.
    pub memory: usize,
    /// Frames in one task.
    pub depth: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            fuel: 50_000_000,
            memory: 64 << 20,
            depth: 200,
        }
    }
}

/// The limit that stopped a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Limit {
    /// Its steps.
    Fuel,
    /// Its live bytes.
    Memory,
    /// Its call depth.
    Depth,
}

/// What a run cost, exactly: the same program gives the same counts on every machine.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    /// Instructions run.
    pub steps: u64,
    /// The most that was live after a collection, or at the end.
    pub peak_bytes: usize,
    /// Collections finished.
    pub collections: u64,
    /// The cells and slots the largest collection visited.
    pub largest_collection: u64,
}

/// What one slice of a run came to.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// It has more to do: call [`Run::step`] again.
    More,
    /// It ended: what the program returned, as JSON, or why it stopped.
    Done(Result<Json, ScriptError>),
}

#[derive(Debug, Clone, Copy)]
struct Frame {
    code: usize,
    pc: usize,
    base: usize,
    env: Option<Handle>,
}

#[derive(Debug, Clone, Copy)]
struct Handler {
    frame: usize,
    pc: usize,
    stack: usize,
    env: Option<Handle>,
}

enum Flow {
    Throw(Value),
    Error(&'static str, String),
    Collect,
    Limit(Limit),
    Return(Value),
}

type Done<T = ()> = Result<T, Flow>;

fn type_error<T>(message: impl Into<String>) -> Done<T> {
    Err(Flow::Error("TypeError", message.into()))
}

fn full(full: Full) -> Flow {
    match full {
        Full::Collect => Flow::Collect,
        Full::Memory => Flow::Limit(Limit::Memory),
    }
}

fn ix(at: u32) -> usize {
    usize::try_from(at).unwrap_or(usize::MAX)
}

fn count(len: usize) -> Value {
    Value::Number(u32::try_from(len).map_or(f64::INFINITY, f64::from))
}

fn index(x: f64) -> Option<usize> {
    if x == 0.0 {
        return Some(0);
    }
    if !(1.0..4_294_967_296.0).contains(&x) || x.fract() != 0.0 {
        return None;
    }
    let bits = x.to_bits();
    let exp = ((bits >> 52) & 0x7ff).checked_sub(1023)?;
    let mantissa = (bits & ((1 << 52) - 1)) | (1 << 52);
    usize::try_from(mantissa >> 52_u64.checked_sub(exp)?).ok()
}

/// A number as JavaScript writes it: `1`, `0.5`, `1e+21`, `NaN`.
pub(crate) fn number_text(x: f64) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    if x == 0.0 {
        return "0".into();
    }
    let sign = if x < 0.0 { "-" } else { "" };
    let exp_form = format!("{:e}", x.abs());
    let (mantissa, exp) = exp_form.split_once('e').unwrap_or((&exp_form, "0"));
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let k = i64::try_from(digits.len()).unwrap_or(0);
    let e = exp.parse::<i64>().unwrap_or(0) + 1;
    let zeros = |m: i64| "0".repeat(usize::try_from(m).unwrap_or(0));
    let body = if k <= e && e <= 21 {
        format!("{digits}{}", zeros(e - k))
    } else if 0 < e && e <= 21 {
        let (int, frac) = digits.split_at(usize::try_from(e).unwrap_or(0).min(digits.len()));
        format!("{int}.{frac}")
    } else if -6 < e && e <= 0 {
        format!("0.{}{digits}", zeros(-e))
    } else {
        let (first, rest) = digits.split_at(1.min(digits.len()));
        let dot = if rest.is_empty() { "" } else { "." };
        let exp_sign = if e > 0 { "+" } else { "-" };
        format!("{first}{dot}{rest}e{exp_sign}{}", (e - 1).abs())
    };
    format!("{sign}{body}")
}

/// One run of a program: its heap, frames and counters. Plain data: drop it to cancel.
#[derive(Debug)]
pub struct Run {
    program: Arc<Compiled>,
    limits: Limits,
    heap: Heap,
    stack: Vec<Value>,
    frames: Vec<Frame>,
    handlers: Vec<Handler>,
    strings: Vec<Handle>,
    steps: u64,
    retries: u8,
    done: Option<Result<Json, ScriptError>>,
}

impl Run {
    /// A run of `program`, stopped by `limits`; nothing runs until [`Run::step`].
    #[must_use]
    pub fn new(program: &Program, limits: Limits) -> Self {
        let mut run = Self {
            program: Arc::clone(&program.compiled),
            limits,
            heap: Heap::new(limits.memory),
            stack: Vec::new(),
            frames: Vec::new(),
            handlers: Vec::new(),
            strings: Vec::new(),
            steps: 0,
            retries: 0,
            done: None,
        };
        let program = Arc::clone(&run.program);
        for text in &program.strings {
            let Ok(handle) = run.heap.alloc(Cell::Str(text.clone())) else {
                run.stop(Limit::Memory);
                return run;
            };
            run.strings.push(handle);
        }
        let locals = program.code.first().map_or(0, |c| c.locals);
        run.stack.resize(locals, Value::Undefined);
        run.frames.push(Frame {
            code: 0,
            pc: 0,
            base: 0,
            env: None,
        });
        run
    }

    /// Runs one slice: at most 10,000 steps or units of collecting.
    pub fn step(&mut self) -> Step {
        let mut units = 0;
        while self.done.is_none() && units < SLICE {
            if self.heap.collecting() || self.heap.wants_collection() {
                let roots = if self.heap.collecting() {
                    Vec::new()
                } else {
                    self.roots()
                };
                units += self.heap.collect(&roots, SLICE - units).max(1);
                continue;
            }
            if self.steps >= self.limits.fuel {
                self.stop(Limit::Fuel);
                break;
            }
            self.steps += 1;
            units += 1;
            match self.exec() {
                Ok(()) => self.retries = 0,
                Err(flow) => self.flow(flow),
            }
        }
        match &self.done {
            Some(done) => Step::Done(done.clone()),
            None => Step::More,
        }
    }

    /// What the run has cost so far.
    #[must_use]
    pub fn counts(&self) -> Counts {
        let heap = self.heap.counts();
        Counts {
            steps: self.steps,
            peak_bytes: heap.peak_bytes.max(self.heap.bytes()),
            collections: heap.collections,
            largest_collection: heap.largest_collection,
        }
    }

    fn roots(&self) -> Vec<Handle> {
        let refs = self.stack.iter().filter_map(|v| match v {
            Value::Ref(h) => Some(*h),
            _ => None,
        });
        let envs = self.frames.iter().filter_map(|f| f.env);
        let handlers = self.handlers.iter().filter_map(|h| h.env);
        refs.chain(envs)
            .chain(handlers)
            .chain(self.strings.iter().copied())
            .collect()
    }

    fn code(&self, at: usize) -> Option<&Code> {
        self.program.code.get(at)
    }

    fn pos(&self) -> Pos {
        self.frames
            .last()
            .and_then(|f| self.code(f.code)?.at.get(f.pc.checked_sub(1)?).copied())
            .unwrap_or(Pos { line: 1, column: 1 })
    }

    fn stop(&mut self, limit: Limit) {
        let (code, message) = match limit {
            Limit::Fuel => (
                "FUEL",
                format!(
                    "stopped after {} steps: a loop that does not end, or too much work for one script",
                    self.limits.fuel
                ),
            ),
            Limit::Memory => (
                "MEMORY",
                format!(
                    "stopped at {} bytes of live values: keep less, and return only what is needed",
                    self.limits.memory
                ),
            ),
            Limit::Depth => (
                "DEPTH",
                format!(
                    "stopped at {} nested calls: recursion too deep, use a loop",
                    self.limits.depth
                ),
            ),
        };
        let pos = self.pos();
        self.done = Some(Err(ScriptError {
            name: "LimitError".into(),
            message,
            line: pos.line,
            column: pos.column,
            code: Some(code.into()),
        }));
    }

    fn flow(&mut self, flow: Flow) {
        let thrown = match flow {
            Flow::Collect => return self.retry(),
            Flow::Limit(limit) => return self.stop(limit),
            Flow::Return(value) => {
                let pos = self.pos();
                self.done = Some(
                    self.json(value, "result", 0)
                        .map_err(|message| ScriptError {
                            name: "TypeError".into(),
                            message,
                            line: pos.line,
                            column: pos.column,
                            code: None,
                        }),
                );
                return;
            }
            Flow::Throw(value) => value,
            Flow::Error(name, message) => match self.error(name, &message) {
                Ok(value) => value,
                Err(Flow::Limit(limit)) => return self.stop(limit),
                Err(_) => return self.retry(),
            },
        };
        let Some(h) = self.handlers.pop() else {
            let pos = self.pos();
            self.done = Some(Err(self.uncaught(thrown, pos)));
            return;
        };
        self.frames.truncate(h.frame + 1);
        self.stack.truncate(h.stack);
        if let Some(frame) = self.frames.last_mut() {
            frame.env = h.env;
            frame.pc = h.pc;
        }
        self.stack.push(thrown);
    }

    fn retry(&mut self) {
        self.retries += 1;
        if self.retries > 2 {
            return self.stop(Limit::Memory);
        }
        self.steps -= 1;
        if let Some(frame) = self.frames.last_mut() {
            frame.pc -= 1;
        }
        if !self.heap.collecting() {
            let roots = self.roots();
            self.heap.collect(&roots, 1);
        }
    }

    fn alloc(&mut self, cell: Cell) -> Done<Value> {
        self.heap.alloc(cell).map(Value::Ref).map_err(full)
    }

    fn string(&mut self, text: impl Into<Box<str>>) -> Done<Value> {
        self.alloc(Cell::Str(text.into()))
    }

    fn error(&mut self, name: &str, message: &str) -> Done<Value> {
        let pos = self.pos();
        let name = self.string(name)?;
        let message = self.string(message)?;
        self.alloc(Cell::Object(vec![
            ("name".into(), name),
            ("message".into(), message),
            ("line".into(), Value::Number(pos.line.into())),
            ("column".into(), Value::Number(pos.column.into())),
        ]))
    }

    fn uncaught(&self, value: Value, pos: Pos) -> ScriptError {
        let field = |key: &str| match self.cell(value) {
            Some(Cell::Object(fields)) => fields
                .iter()
                .find(|(k, _)| &**k == key)
                .and_then(|(_, v)| self.str(*v))
                .map(str::to_owned),
            _ => None,
        };
        let (name, message) = match (field("name"), field("message")) {
            (Some(name), Some(message)) => (name, message),
            _ => (
                "Error".into(),
                self.text(value)
                    .unwrap_or_else(|_| "a value was thrown".into()),
            ),
        };
        ScriptError {
            name,
            message,
            line: pos.line,
            column: pos.column,
            code: None,
        }
    }

    fn cell(&self, value: Value) -> Option<&Cell> {
        match value {
            Value::Ref(h) => self.heap.get(h),
            _ => None,
        }
    }

    fn str(&self, value: Value) -> Option<&str> {
        match self.cell(value) {
            Some(Cell::Str(s)) => Some(s),
            _ => None,
        }
    }

    fn kind(&self, value: Value) -> &'static str {
        match (value, self.cell(value)) {
            (Value::Undefined, _) => "undefined",
            (Value::Null, _) => "null",
            (Value::Bool(_), _) => "boolean",
            (Value::Number(_), _) => "number",
            (_, Some(Cell::Str(_))) => "string",
            (_, Some(Cell::Array(_))) => "array",
            (_, Some(Cell::Closure { .. })) => "function",
            _ => "object",
        }
    }

    fn a(&self, value: Value) -> String {
        match self.kind(value) {
            kind @ ("undefined" | "null") => kind.into(),
            kind @ ("array" | "object") => format!("an {kind}"),
            kind => format!("a {kind}"),
        }
    }

    fn truthy(&self, value: Value) -> bool {
        match value {
            Value::Undefined | Value::Null => false,
            Value::Bool(b) => b,
            Value::Number(x) => x != 0.0 && !x.is_nan(),
            Value::Ref(_) => self.str(value).is_none_or(|s| !s.is_empty()),
        }
    }

    fn same(&self, a: Value, b: Value) -> bool {
        match (self.str(a), self.str(b)) {
            (Some(x), Some(y)) => x == y,
            _ => a == b,
        }
    }

    fn text(&self, value: Value) -> Result<String, String> {
        Ok(match value {
            Value::Undefined => "undefined".into(),
            Value::Null => "null".into(),
            Value::Bool(b) => b.to_string(),
            Value::Number(x) => number_text(x),
            Value::Ref(_) => match self.str(value) {
                Some(s) => s.into(),
                None => self.json(value, "the value", 0)?.to_string(),
            },
        })
    }

    fn json(&self, value: Value, path: &str, depth: usize) -> Result<Json, String> {
        if depth > JSON_DEPTH {
            return Err(format!(
                "{path} is nested over {JSON_DEPTH} deep (does it hold itself?)"
            ));
        }
        Ok(match value {
            Value::Undefined | Value::Null => Json::Null,
            Value::Bool(b) => Json::Bool(b),
            Value::Number(x) if x.is_finite() => {
                if x.fract() == 0.0 && x.abs() <= 9_007_199_254_740_992.0 {
                    number_text(x).parse::<i64>().map_or(Json::Null, Json::from)
                } else {
                    serde_json::Number::from_f64(x).map_or(Json::Null, Json::Number)
                }
            }
            Value::Number(x) => {
                return Err(format!(
                    "{path} is {}: JSON has no such number",
                    number_text(x)
                ));
            }
            Value::Ref(_) => match self.cell(value) {
                Some(Cell::Str(s)) => Json::String(s.to_string()),
                Some(Cell::Array(items)) => Json::Array(
                    items
                        .iter()
                        .enumerate()
                        .map(|(at, v)| self.json(*v, &format!("{path}[{at}]"), depth + 1))
                        .collect::<Result<_, _>>()?,
                ),
                Some(Cell::Object(fields)) => {
                    let mut map = serde_json::Map::new();
                    for (key, v) in fields {
                        if *v != Value::Undefined {
                            map.insert(
                                key.to_string(),
                                self.json(*v, &format!("{path}.{key}"), depth + 1)?,
                            );
                        }
                    }
                    Json::Object(map)
                }
                _ => return Err(format!("{path} is {}: JSON cannot carry it", self.a(value))),
            },
        })
    }

    fn pop(&mut self) -> Value {
        self.stack.pop().unwrap_or(Value::Undefined)
    }

    fn peek(&self, depth: usize) -> Value {
        self.stack
            .len()
            .checked_sub(depth + 1)
            .and_then(|at| self.stack.get(at))
            .copied()
            .unwrap_or(Value::Undefined)
    }

    fn replace(&mut self, popped: usize, value: Value) {
        self.stack.truncate(self.stack.len().saturating_sub(popped));
        self.stack.push(value);
    }

    fn frame(&mut self) -> Done<&mut Frame> {
        self.frames
            .last_mut()
            .ok_or_else(|| Flow::Error("Error", "the machine has no frame".into()))
    }

    fn env(&self, hops: u32) -> Done<Handle> {
        let mut env = self.frames.last().and_then(|f| f.env);
        for _ in 0..hops {
            env = match env.and_then(|h| self.heap.get(h)) {
                Some(Cell::Scope { parent, .. }) => *parent,
                _ => None,
            };
        }
        env.ok_or_else(|| Flow::Error("Error", "a scope is missing".into()))
    }

    fn name(&self, at: u32) -> Box<str> {
        self.program
            .strings
            .get(ix(at))
            .cloned()
            .unwrap_or_default()
    }

    fn exec(&mut self) -> Done {
        let frame = self.frame()?;
        let (code, pc, base) = (frame.code, frame.pc, frame.base);
        frame.pc += 1;
        let op = self
            .code(code)
            .and_then(|c| c.ops.get(pc))
            .copied()
            .ok_or_else(|| Flow::Error("Error", "the code ended without a return".into()))?;
        match op {
            Op::Num(_)
            | Op::Str(_)
            | Op::Undefined
            | Op::Null
            | Op::Bool(_)
            | Op::Pop
            | Op::Dup
            | Op::Dup2
            | Op::Nip
            | Op::Reg(_)
            | Op::SetReg(_) => {
                self.stack_op(op, base);
                Ok(())
            }
            Op::Load(..)
            | Op::Store(..)
            | Op::Global(_)
            | Op::ConstSet(_)
            | Op::Enter(_)
            | Op::Leave
            | Op::Renew
            | Op::Closure(_) => self.scope_op(op),
            Op::Jump(_)
            | Op::IfFalse(_)
            | Op::IfTrue(_)
            | Op::IfSome(_)
            | Op::IfDefined(_)
            | Op::Opt(_)
            | Op::Not
            | Op::Neg
            | Op::Plus
            | Op::Inc(_)
            | Op::Typeof
            | Op::NullCheck(_)
            | Op::Binary(_) => self.flow_op(op),
            Op::Get(_)
            | Op::GetIndex
            | Op::Set(_)
            | Op::SetIndex
            | Op::NewArray
            | Op::Append
            | Op::Spread
            | Op::NewObject
            | Op::Field(_)
            | Op::FieldIndex
            | Op::SpreadObject => self.object_op(op),
            Op::Call(_) | Op::CallSpread | Op::Return | Op::Throw | Op::Try(_) | Op::EndTry => {
                self.control_op(op)
            }
            Op::Template(_)
            | Op::Iter
            | Op::Next(_)
            | Op::Destructure(_)
            | Op::Item(_)
            | Op::RestArray(_)
            | Op::RestObject(_)
            | Op::NewError(_) => self.build_op(op),
        }
    }

    fn stack_op(&mut self, op: Op, base: usize) {
        match op {
            Op::Num(x) => self.stack.push(Value::Number(x)),
            Op::Str(at) => {
                let s = self
                    .strings
                    .get(ix(at))
                    .copied()
                    .map_or(Value::Undefined, Value::Ref);
                self.stack.push(s);
            }
            Op::Undefined => self.stack.push(Value::Undefined),
            Op::Null => self.stack.push(Value::Null),
            Op::Bool(b) => self.stack.push(Value::Bool(b)),
            Op::Pop => {
                self.pop();
            }
            Op::Dup => self.stack.push(self.peek(0)),
            Op::Dup2 => {
                let (a, b) = (self.peek(1), self.peek(0));
                self.stack.extend([a, b]);
            }
            Op::Nip => {
                let top = self.pop();
                self.replace(1, top);
            }
            Op::Reg(r) => {
                let v = self
                    .stack
                    .get(base + ix(r))
                    .copied()
                    .unwrap_or(Value::Undefined);
                self.stack.push(v);
            }
            Op::SetReg(r) => {
                let v = self.pop();
                if let Some(slot) = self.stack.get_mut(base + ix(r)) {
                    *slot = v;
                }
            }
            _ => {}
        }
    }

    fn scope_op(&mut self, op: Op) -> Done {
        match op {
            Op::Load(hops, slot) => {
                let env = self.env(hops)?;
                let v = match self.heap.get(env) {
                    Some(Cell::Scope { slots, .. }) => slots.get(ix(slot)).copied(),
                    _ => None,
                };
                self.stack.push(v.unwrap_or(Value::Undefined));
            }
            Op::Store(hops, slot) => {
                let env = self.env(hops)?;
                let v = self.pop();
                if let Some(Cell::Scope { slots, .. }) = self.heap.cell_mut(env)
                    && let Some(at) = slots.get_mut(ix(slot))
                {
                    *at = v;
                }
            }
            Op::Global(at) => {
                return Err(Flow::Error(
                    "ReferenceError",
                    format!("{} is not defined", self.name(at)),
                ));
            }
            Op::ConstSet(at) => {
                return type_error(format!(
                    "{} is a constant: declare it with let to assign to it",
                    self.name(at)
                ));
            }
            Op::Enter(slots) => {
                let parent = self.frames.last().and_then(|f| f.env);
                let scope = self.heap.alloc(Cell::Scope {
                    parent,
                    slots: vec![Value::Undefined; ix(slots)],
                });
                self.frame()?.env = Some(scope.map_err(full)?);
            }
            Op::Leave => {
                let env = self.env(1).ok();
                self.frame()?.env = env;
            }
            Op::Renew => {
                let env = self.env(0)?;
                let copy = self
                    .heap
                    .get(env)
                    .cloned()
                    .ok_or_else(|| Flow::Error("Error", "a scope is missing".into()))?;
                let copy = self.heap.alloc(copy).map_err(full)?;
                self.frame()?.env = Some(copy);
            }
            Op::Closure(function) => {
                let scope = self.frames.last().and_then(|f| f.env);
                let closure = self.alloc(Cell::Closure {
                    function: ix(function),
                    scope,
                })?;
                self.stack.push(closure);
            }
            _ => {}
        }
        Ok(())
    }

    fn flow_op(&mut self, op: Op) -> Done {
        match op {
            Op::Jump(to) => self.frame()?.pc = ix(to),
            Op::IfFalse(to) | Op::IfTrue(to) | Op::IfSome(to) | Op::IfDefined(to) => {
                let v = self.pop();
                let jump = match op {
                    Op::IfFalse(_) => !self.truthy(v),
                    Op::IfTrue(_) => self.truthy(v),
                    Op::IfSome(_) => !matches!(v, Value::Undefined | Value::Null),
                    _ => v != Value::Undefined,
                };
                if jump {
                    self.frame()?.pc = ix(to);
                }
            }
            Op::Opt(to) => {
                if matches!(self.peek(0), Value::Undefined | Value::Null) {
                    self.replace(1, Value::Undefined);
                    self.frame()?.pc = ix(to);
                }
            }
            Op::Not => {
                let v = self.pop();
                self.stack.push(Value::Bool(!self.truthy(v)));
            }
            Op::Neg | Op::Plus | Op::Inc(_) => {
                let Value::Number(x) = self.peek(0) else {
                    return type_error(format!(
                        "{} is not a number: no implicit conversion here, use Number(x)",
                        self.kind(self.peek(0))
                    ));
                };
                let v = match op {
                    Op::Neg => -x,
                    Op::Inc(delta) => x + delta,
                    _ => x,
                };
                self.replace(1, Value::Number(v));
            }
            Op::Typeof => {
                let kind = match self.kind(self.peek(0)) {
                    "null" | "array" => "object",
                    kind => kind,
                };
                let s = self.string(kind)?;
                self.replace(1, s);
            }
            Op::NullCheck(ne) => {
                let v = self.pop();
                self.stack.push(Value::Bool(
                    matches!(v, Value::Undefined | Value::Null) != ne,
                ));
            }
            Op::Binary(op) => {
                let v = self.binary(op, self.peek(1), self.peek(0))?;
                self.replace(2, v);
            }
            _ => {}
        }
        Ok(())
    }

    fn object_op(&mut self, op: Op) -> Done {
        match op {
            Op::Get(at) => {
                let name = self.name(at);
                let v = self.get(self.peek(0), &name)?;
                self.replace(1, v);
            }
            Op::GetIndex => {
                let (object, key) = (self.peek(1), self.peek(0));
                let v = self.get_key(object, key)?;
                self.replace(2, v);
            }
            Op::Set(at) => {
                let (object, v) = (self.peek(1), self.peek(0));
                let name = self.name(at);
                self.set(object, &name, None, v)?;
                self.replace(2, v);
            }
            Op::SetIndex => {
                let (object, key, v) = (self.peek(2), self.peek(1), self.peek(0));
                if let (Value::Number(x), Some(Cell::Array(_))) = (key, self.cell(object)) {
                    self.set(object, "", Some(x), v)?;
                } else {
                    let key = self.key(key)?;
                    self.set(object, &key, None, v)?;
                }
                self.replace(3, v);
            }
            Op::NewArray => {
                let array = self.alloc(Cell::Array(Vec::new()))?;
                self.stack.push(array);
            }
            Op::Append | Op::Spread => {
                let (array, v) = (self.peek(1), self.peek(0));
                let items = if op == Op::Spread {
                    self.items(v)?
                } else {
                    vec![v]
                };
                if let Value::Ref(array) = array {
                    self.heap.extend(array, &items).map_err(full)?;
                }
                self.pop();
            }
            Op::NewObject => {
                let object = self.alloc(Cell::Object(Vec::new()))?;
                self.stack.push(object);
            }
            Op::Field(at) => {
                let (object, v) = (self.peek(1), self.peek(0));
                self.field(object, &self.name(at), v)?;
                self.pop();
            }
            Op::FieldIndex => {
                let (object, key, v) = (self.peek(2), self.peek(1), self.peek(0));
                let key = self.key(key)?;
                self.field(object, &key, v)?;
                self.replace(3, object);
            }
            Op::SpreadObject => {
                let (object, from) = (self.peek(1), self.peek(0));
                let fields = match self.cell(from) {
                    Some(Cell::Object(fields)) => fields.clone(),
                    _ if matches!(from, Value::Undefined | Value::Null) => Vec::new(),
                    _ => {
                        return type_error(format!(
                            "...{} in an object: spread an object",
                            self.a(from)
                        ));
                    }
                };
                for (key, v) in fields {
                    self.field(object, &key, v)?;
                }
                self.pop();
            }
            _ => {}
        }
        Ok(())
    }

    fn control_op(&mut self, op: Op) -> Done {
        match op {
            Op::Call(argc) => {
                let callee = self.stack.len().saturating_sub(ix(argc) + 1);
                let values = self
                    .stack
                    .get(callee + 1..)
                    .map(<[Value]>::to_vec)
                    .unwrap_or_default();
                self.call(callee, &values)?;
            }
            Op::CallSpread => {
                let at = self.stack.len().saturating_sub(2);
                let args = match self.cell(self.peek(0)) {
                    Some(Cell::Array(items)) => items.clone(),
                    _ => Vec::new(),
                };
                self.call(at, &args)?;
            }
            Op::Return => {
                let v = self.pop();
                if self.frames.len() == 1 {
                    return Err(Flow::Return(v));
                }
                let frame = self.frames.pop();
                let depth = self.frames.len();
                while self.handlers.last().is_some_and(|h| h.frame >= depth) {
                    self.handlers.pop();
                }
                match frame {
                    Some(frame) if depth > 0 => {
                        self.stack.truncate(frame.base.saturating_sub(1));
                        self.stack.push(v);
                    }
                    _ => return Err(Flow::Return(v)),
                }
            }
            Op::Throw => {
                let v = self.peek(0);
                return Err(Flow::Throw(v));
            }
            Op::Try(to) => {
                let frame = self.frames.len().saturating_sub(1);
                let env = self.frames.last().and_then(|f| f.env);
                self.handlers.push(Handler {
                    frame,
                    pc: ix(to),
                    stack: self.stack.len(),
                    env,
                });
            }
            Op::EndTry => {
                self.handlers.pop();
            }
            _ => {}
        }
        Ok(())
    }

    fn build_op(&mut self, op: Op) -> Done {
        match op {
            Op::Template(parts) => {
                let parts = ix(parts);
                let mut out = String::new();
                for depth in (0..parts).rev() {
                    let text = self
                        .text(self.peek(depth))
                        .map_err(|m| Flow::Error("TypeError", m))?;
                    out.push_str(&text);
                }
                let s = self.string(out)?;
                self.replace(parts, s);
            }
            Op::Iter => {
                let v = self.peek(0);
                if !matches!(self.cell(v), Some(Cell::Array(_) | Cell::Str(_))) {
                    return type_error(format!(
                        "for … of {}: it takes an array or a string (Object.keys(o) for an object's keys)",
                        self.a(v)
                    ));
                }
                self.stack.push(Value::Number(0.0));
            }
            Op::Next(exit) => {
                let Value::Number(at) = self.peek(0) else {
                    return type_error("for … of lost its place");
                };
                if let Some(v) = self.element(self.peek(1), index(at).unwrap_or(usize::MAX))? {
                    self.replace(1, Value::Number(at + 1.0));
                    self.stack.push(v);
                } else {
                    {
                        self.replace(2, Value::Undefined);
                        self.pop();
                        self.frame()?.pc = ix(exit);
                    }
                }
            }
            Op::Destructure(array) => {
                let from = self.peek(0);
                if array && !matches!(self.cell(from), Some(Cell::Array(_) | Cell::Str(_))) {
                    return type_error(format!("[…] destructures an array, not {}", self.a(from)));
                }
                if matches!(from, Value::Undefined | Value::Null) {
                    return type_error(format!("{{…}} cannot destructure {}", self.a(from)));
                }
            }
            Op::Item(at) => {
                let from = self.peek(0);
                let v = self.element(from, ix(at))?.unwrap_or(Value::Undefined);
                self.stack.push(v);
            }
            Op::RestArray(start) => {
                let items = self.items(self.peek(0))?;
                let rest = self.alloc(Cell::Array(
                    items.get(ix(start)..).unwrap_or_default().to_vec(),
                ))?;
                self.stack.push(rest);
            }
            Op::RestObject(keys) => {
                let keys = ix(keys);
                let mut skip = Vec::new();
                for depth in 0..keys {
                    skip.push(self.key(self.peek(depth))?);
                }
                let from = self.peek(keys);
                let fields = match self.cell(from) {
                    Some(Cell::Object(fields)) => fields
                        .iter()
                        .filter(|(k, _)| !skip.iter().any(|s| **s == **k))
                        .cloned()
                        .collect(),
                    _ => Vec::new(),
                };
                let rest = self.alloc(Cell::Object(fields))?;
                self.replace(keys + 1, rest);
            }
            Op::NewError(argc) => {
                let argc = ix(argc);
                let message = match argc {
                    0 => String::new(),
                    _ => match self.peek(argc - 1) {
                        Value::Undefined => String::new(),
                        v => self.text(v).map_err(|m| Flow::Error("TypeError", m))?,
                    },
                };
                let error = self.error("Error", &message)?;
                self.replace(argc, error);
            }
            _ => {}
        }
        Ok(())
    }

    fn call(&mut self, at: usize, args: &[Value]) -> Done {
        let callee = self.stack.get(at).copied().unwrap_or(Value::Undefined);
        let Some(&Cell::Closure { function, scope }) = self.cell(callee) else {
            return type_error(format!("{} is not a function", self.kind(callee)));
        };
        let (params, rest, locals) = self
            .code(function)
            .map_or((0, false, 0), |c| (c.params, c.rest, c.locals));
        if self.frames.len() >= self.limits.depth {
            return Err(Flow::Limit(Limit::Depth));
        }
        let extra = if rest {
            let extra = args.get(params..).unwrap_or_default().to_vec();
            Some(self.alloc(Cell::Array(extra))?)
        } else {
            None
        };
        let base = at + 1;
        self.stack.truncate(base);
        self.stack.extend(args.iter().take(params).copied());
        self.stack.resize(base + params, Value::Undefined);
        self.stack.extend(extra);
        self.stack.resize(base + locals, Value::Undefined);
        self.frames.push(Frame {
            code: function,
            pc: 0,
            base,
            env: scope,
        });
        Ok(())
    }

    fn binary(&mut self, op: Binary, a: Value, b: Value) -> Done<Value> {
        if let (Value::Number(x), Value::Number(y)) = (a, b) {
            return Ok(match op {
                Binary::Add => Value::Number(x + y),
                Binary::Sub => Value::Number(x - y),
                Binary::Mul => Value::Number(x * y),
                Binary::Div => Value::Number(x / y),
                Binary::Rem => Value::Number(x % y),
                Binary::Pow => Value::Number(x.powf(y)),
                Binary::Eq => Value::Bool(x.partial_cmp(&y) == Some(Ordering::Equal)),
                Binary::Ne => Value::Bool(x.partial_cmp(&y) != Some(Ordering::Equal)),
                Binary::Lt => Value::Bool(x < y),
                Binary::Le => Value::Bool(x <= y),
                Binary::Gt => Value::Bool(x > y),
                Binary::Ge => Value::Bool(x >= y),
            });
        }
        match op {
            Binary::Eq => return Ok(Value::Bool(self.same(a, b))),
            Binary::Ne => return Ok(Value::Bool(!self.same(a, b))),
            _ => {}
        }
        if let (Some(x), Some(y)) = (self.str(a), self.str(b)) {
            let v = match op {
                Binary::Lt => x < y,
                Binary::Le => x <= y,
                Binary::Gt => x > y,
                Binary::Ge => x >= y,
                _ => false,
            };
            if op != Binary::Add {
                return if matches!(op, Binary::Lt | Binary::Le | Binary::Gt | Binary::Ge) {
                    Ok(Value::Bool(v))
                } else {
                    type_error(format!(
                        "{} between strings: convert with Number(x)",
                        symbol(op)
                    ))
                };
            }
        }
        if op == Binary::Add && (self.str(a).is_some() || self.str(b).is_some()) {
            let joined = [a, b]
                .iter()
                .map(|v| self.text(*v))
                .collect::<Result<String, String>>()
                .map_err(|m| Flow::Error("TypeError", m))?;
            return self.string(joined);
        }
        type_error(format!(
            "{} between {} and {}: no implicit conversion here, use String(x) or Number(x)",
            symbol(op),
            self.a(a),
            self.a(b)
        ))
    }

    fn key(&self, key: Value) -> Done<String> {
        match key {
            Value::Number(x) => Ok(number_text(x)),
            _ => match self.str(key) {
                Some(s) => Ok(s.into()),
                None => type_error(format!(
                    "{} is not a key: use a string or a number",
                    self.a(key)
                )),
            },
        }
    }

    fn get_key(&mut self, object: Value, key: Value) -> Done<Value> {
        if let (Value::Number(x), Some(Cell::Array(_) | Cell::Str(_))) = (key, self.cell(object)) {
            return Ok(self
                .element(object, index(x).unwrap_or(usize::MAX))?
                .unwrap_or(Value::Undefined));
        }
        let key = self.key(key)?;
        self.get(object, &key)
    }

    fn get(&self, object: Value, name: &str) -> Done<Value> {
        Ok(match self.cell(object) {
            Some(Cell::Object(fields)) => fields
                .iter()
                .find(|(k, _)| &**k == name)
                .map_or(Value::Undefined, |(_, v)| *v),
            Some(Cell::Array(items)) if name == "length" => count(items.len()),
            Some(Cell::Str(s)) if name == "length" => count(s.chars().count()),
            Some(Cell::Array(items)) => name
                .parse::<usize>()
                .ok()
                .and_then(|at| items.get(at).copied())
                .unwrap_or(Value::Undefined),
            _ if matches!(object, Value::Undefined | Value::Null) => {
                return type_error(format!(
                    "cannot read .{name} of {}: guard it with ?.",
                    self.kind(object)
                ));
            }
            _ => Value::Undefined,
        })
    }

    fn set(&mut self, object: Value, name: &str, at: Option<f64>, v: Value) -> Done {
        let Value::Ref(handle) = object else {
            return type_error(format!("cannot set .{name} on {}", self.kind(object)));
        };
        match (self.heap.cell_mut(handle), at) {
            (Some(Cell::Object(_)), _) => self.field(object, name, v),
            (Some(Cell::Array(items)), Some(x)) => {
                let len = items.len();
                match index(x) {
                    Some(i) if i < len => {
                        if let Some(slot) = items.get_mut(i) {
                            *slot = v;
                        }
                        Ok(())
                    }
                    Some(i) if i == len => self.heap.extend(handle, &[v]).map(|_| ()).map_err(full),
                    _ => Err(Flow::Error(
                        "RangeError",
                        format!(
                            "index {} of an array of {len}: set one that exists, or the one at its length",
                            number_text(x)
                        ),
                    )),
                }
            }
            _ => type_error(format!("cannot set .{name} on {}", self.a(object))),
        }
    }

    fn field(&mut self, object: Value, key: &str, v: Value) -> Done {
        if let Value::Ref(handle) = object {
            self.heap.set_field(handle, key, v).map_err(full)?;
        }
        Ok(())
    }

    fn element(&mut self, from: Value, at: usize) -> Done<Option<Value>> {
        match self.cell(from) {
            Some(Cell::Array(items)) => Ok(items.get(at).copied()),
            Some(Cell::Str(s)) => match s.chars().nth(at) {
                Some(c) => Ok(Some(self.string(c.to_string())?)),
                None => Ok(None),
            },
            _ => Ok(None),
        }
    }

    fn items(&mut self, from: Value) -> Done<Vec<Value>> {
        match self.cell(from) {
            Some(Cell::Array(items)) => Ok(items.clone()),
            Some(Cell::Str(s)) => {
                let chars: Vec<char> = s.chars().collect();
                chars
                    .into_iter()
                    .map(|c| self.string(c.to_string()))
                    .collect()
            }
            _ => type_error(format!("...{}: spread an array or a string", self.a(from))),
        }
    }
}

fn symbol(op: Binary) -> &'static str {
    match op {
        Binary::Add => "+",
        Binary::Sub => "-",
        Binary::Mul => "*",
        Binary::Div => "/",
        Binary::Rem => "%",
        Binary::Pow => "**",
        Binary::Eq => "===",
        Binary::Ne => "!==",
        Binary::Lt => "<",
        Binary::Le => "<=",
        Binary::Gt => ">",
        Binary::Ge => ">=",
    }
}
