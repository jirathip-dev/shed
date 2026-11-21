use std::sync::LazyLock;

use crate::ast::Stmt;
use crate::compile::{Compiled, Compiler, Op};
use crate::value::{Builtin, Value};
use crate::vm::{Done, Flow, Limit, Run, full, type_error};

mod array;
mod string;

const ROWS: &[(&str, &str)] = &[
    (
        "",
        "console JSON Math Object Array String Number Boolean Error parseInt parseFloat isNaN",
    ),
    ("console", "log info warn error debug"),
    ("JSON", "parse stringify"),
    ("Math", "min max floor ceil round trunc abs sign sqrt pow"),
    ("Object", "keys values entries fromEntries assign hasOwn"),
    ("Array", "isArray from"),
    ("String", "fromBytes"),
    ("Number", "isInteger isFinite"),
    (
        "string",
        "at charAt slice substring split trim trimStart trimEnd startsWith endsWith includes indexOf \
         lastIndexOf toLowerCase toUpperCase replace replaceAll padStart padEnd repeat concat \
         localeCompare matches toBytes",
    ),
    (
        "array",
        "at push pop shift unshift slice concat join includes indexOf lastIndexOf reverse flat \
         entries map filter forEach find findIndex some every reduce flatMap sort",
    ),
];

fn find(owner: &str, name: &str) -> Option<Builtin> {
    let (row, (_, names)) = ROWS.iter().enumerate().find(|(_, (o, _))| *o == owner)?;
    let at = names.split_whitespace().position(|m| m == name)?;
    Some(Builtin {
        owner: u8::try_from(row).ok()?,
        at: u8::try_from(at).ok()?,
    })
}

pub(crate) fn names(b: Builtin) -> (&'static str, &'static str) {
    ROWS.get(usize::from(b.owner))
        .and_then(|(owner, names)| Some((*owner, names.split_whitespace().nth(usize::from(b.at))?)))
        .unwrap_or_default()
}

pub(crate) fn global(name: &str) -> Option<Builtin> {
    match name {
        "console" | "JSON" | "Math" | "Object" | "Array" | "String" | "Number" | "Boolean"
        | "Error" | "parseInt" | "parseFloat" | "isNaN" => find("", name),
        _ => None,
    }
}

pub(crate) fn member(b: Builtin, name: &str) -> Value {
    match names(b) {
        ("", owner) => find(owner, name).map_or(Value::Undefined, Value::Native),
        _ => Value::Undefined,
    }
}

pub(crate) fn method(owner: &str, name: &str, this: Value) -> Value {
    match (find(owner, name), this.handle()) {
        (Some(b), Some(h)) => Value::Method(b, h),
        _ => Value::Undefined,
    }
}

pub(crate) fn kind(b: Builtin) -> &'static str {
    match names(b) {
        ("", "console" | "JSON" | "Math") => "object",
        _ => "function",
    }
}

pub(crate) fn hint(name: &str) -> &'static str {
    match name {
        "require" | "process" | "Buffer" | "globalThis" | "eval" | "Function" => {
            ": a script reaches the outside only through shed.<method>(params)"
        }
        "fetch" => ": a script has no network; shed.exec runs a command",
        "setTimeout" | "setInterval" | "Date" => {
            ": a script has no clock; shed.wait waits on a terminal"
        }
        "RegExp" => ": use text.matches(pattern)",
        _ => "",
    }
}

pub(crate) struct Prelude {
    pub(crate) compiled: Compiled,
    functions: Vec<(String, usize)>,
}

impl Prelude {
    pub(crate) fn function(&self, name: &str) -> Option<usize> {
        self.functions
            .iter()
            .find(|(f, _)| f == name)
            .map(|(_, at)| *at)
    }
}

static PRELUDE: LazyLock<Prelude> = LazyLock::new(|| {
    let compile = || {
        let script = crate::parse(include_str!("prelude.js")).ok()?;
        let compiled = Compiler::program(&script).ok()?;
        let main = &compiled.code.first()?.ops;
        if main.iter().any(|op| matches!(op, Op::Enter(_))) {
            return None;
        }
        let closures = main.iter().filter_map(|op| match op {
            Op::Closure(at) => usize::try_from(*at).ok(),
            _ => None,
        });
        let names = script.body.iter().filter_map(|s| match s {
            Stmt::Function(f) => f.name.clone(),
            _ => None,
        });
        let functions = names.zip(closures).collect();
        Some(Prelude {
            compiled,
            functions,
        })
    };
    compile().unwrap_or(Prelude {
        compiled: Compiled::default(),
        functions: Vec::new(),
    })
});

pub(crate) fn prelude() -> &'static Prelude {
    &PRELUDE
}

pub(crate) fn arg(args: &[Value], at: usize) -> Value {
    args.get(at).copied().unwrap_or(Value::Undefined)
}

fn int(x: f64) -> f64 {
    if x.is_nan() { 0.0 } else { x.trunc() }
}

pub(crate) fn nth(x: f64, len: usize) -> Option<usize> {
    let x = int(x);
    crate::vm::index(if x < 0.0 { x + len_f(len) } else { x })
}

fn len_f(len: usize) -> f64 {
    f64::from(u32::try_from(len).unwrap_or(u32::MAX))
}

pub(crate) fn place(x: f64, len: usize) -> usize {
    let len_f = f64::from(u32::try_from(len).unwrap_or(u32::MAX));
    let x = int(x);
    let at = if x < 0.0 {
        (len_f + x).max(0.0)
    } else {
        x.min(len_f)
    };
    crate::vm::index(at).unwrap_or(len)
}

impl Run {
    pub(crate) fn room(&self, bytes: usize) -> Done {
        self.heap.fits(bytes.saturating_add(32)).map_err(full)
    }

    pub(crate) fn touch(&mut self, units: usize) -> Done {
        let units = u64::try_from(units).unwrap_or(u64::MAX);
        self.steps = self.steps.saturating_add(units);
        if self.steps > self.limits.fuel {
            self.steps = self.limits.fuel;
            return Err(Flow::Limit(Limit::Fuel));
        }
        Ok(())
    }

    pub(crate) fn number(&self, args: &[Value], at: usize, default: f64, what: &str) -> Done<f64> {
        match arg(args, at) {
            Value::Undefined => Ok(default),
            Value::Number(x) => Ok(x),
            v => type_error(format!(
                "{what} takes a number, not {}: convert with Number(x)",
                self.a(v)
            )),
        }
    }

    pub(crate) fn text_arg(&self, args: &[Value], at: usize, what: &str) -> Done<String> {
        let v = arg(args, at);
        self.str(v).map(str::to_owned).ok_or_else(|| {
            Flow::Error(
                "TypeError",
                format!("{what} takes a string, not {}", self.a(v)),
            )
        })
    }

    pub(crate) fn texts(&self, values: &[Value]) -> Done<Vec<String>> {
        values
            .iter()
            .map(|v| self.text(*v).map_err(|m| Flow::Error("TypeError", m)))
            .collect()
    }

    pub(crate) fn builtin(
        &mut self,
        owner: &str,
        name: &str,
        this: Value,
        args: &[Value],
    ) -> Done<Value> {
        match owner {
            "" => self.global_call(name, args),
            "console" => self.log(args),
            "JSON" => self.json_call(name, args),
            "Math" => self.math(name, args),
            "string" => self.string_method(name, this, args),
            "array" => self.array_method(name, this, args),
            _ => self.namespace_call(owner, name, args),
        }
    }

    fn log(&mut self, args: &[Value]) -> Done<Value> {
        let line = self.texts(args)?.join(" ");
        self.logged = self.logged.saturating_add(line.len());
        if self.logged > self.limits.logs {
            return Err(Flow::Limit(Limit::Logs));
        }
        self.logs.push(line);
        Ok(Value::Undefined)
    }

    fn global_call(&mut self, name: &str, args: &[Value]) -> Done<Value> {
        let v = arg(args, 0);
        Ok(match name {
            "String" => {
                let text = self.texts(&[v])?.concat();
                self.string(text)?
            }
            "Number" => Value::Number(self.to_number(v)),
            "Boolean" => Value::Bool(self.truthy(v)),
            "isNaN" => Value::Bool(self.to_number(v).is_nan()),
            "Error" => {
                let message = match v {
                    Value::Undefined => String::new(),
                    v => self.texts(&[v])?.concat(),
                };
                self.error("Error", &message)?
            }
            "parseInt" | "parseFloat" => {
                let text = self.texts(&[v])?.concat();
                let radix = int(self.number(args, 1, 0.0, "parseInt's radix")?);
                Value::Number(if name == "parseInt" {
                    parse_int(&text, radix.rem_euclid(4_294_967_296.0))
                } else {
                    parse_float(&text)
                })
            }
            _ => return type_error(format!("{name} is not a function")),
        })
    }

    pub(crate) fn to_number(&self, v: Value) -> f64 {
        match v {
            Value::Null => 0.0,
            Value::Bool(b) => f64::from(u8::from(b)),
            Value::Number(x) => x,
            _ => self.str(v).map_or(f64::NAN, |s| {
                let s = s.trim_matches(string::space);
                let digits = |radix| {
                    s.get(2..)
                        .filter(|d| !d.is_empty())
                        .map_or(f64::NAN, |d| parse_int(d, radix))
                };
                match s.get(..2).map(str::to_ascii_lowercase).as_deref() {
                    _ if s.is_empty() => 0.0,
                    Some("0x") => digits(16.0),
                    Some("0o") => digits(8.0),
                    Some("0b") => digits(2.0),
                    _ if s.trim_start_matches(['+', '-']) == "Infinity" => {
                        if s.starts_with('-') {
                            f64::NEG_INFINITY
                        } else {
                            f64::INFINITY
                        }
                    }
                    _ if s.chars().any(|c| !"0123456789.eE+-".contains(c)) => f64::NAN,
                    _ => s.parse().unwrap_or(f64::NAN),
                }
            }),
        }
    }

    fn math(&mut self, name: &str, args: &[Value]) -> Done<Value> {
        let what = format!("Math.{name}");
        let mut xs = Vec::with_capacity(args.len());
        for at in 0..args.len().max(1) {
            xs.push(self.number(args, at, f64::NAN, &what)?);
        }
        let x = xs.first().copied().unwrap_or(f64::NAN);
        Ok(Value::Number(match name {
            "min" | "max" if args.is_empty() => {
                if name == "min" {
                    f64::INFINITY
                } else {
                    f64::NEG_INFINITY
                }
            }
            "min" | "max" => xs.iter().copied().fold(x, |a, b| {
                let lower = b.total_cmp(&a).is_lt();
                match () {
                    () if a.is_nan() || b.is_nan() => f64::NAN,
                    () if lower == (name == "min") => b,
                    () => a,
                }
            }),
            "floor" => x.floor(),
            "ceil" => x.ceil(),
            "round" => {
                let f = x.floor();
                let r = if x - f >= 0.5 { f + 1.0 } else { f };
                if r == 0.0 { r.copysign(x) } else { r }
            }
            "trunc" => x.trunc(),
            "abs" => x.abs(),
            "sign" if x == 0.0 || x.is_nan() => x,
            "sign" => x.signum(),
            "sqrt" => x.sqrt(),
            _ => {
                let y = self.number(args, 1, f64::NAN, &what)?;
                if y.is_nan() || (x.abs().total_cmp(&1.0).is_eq() && y.is_infinite()) {
                    f64::NAN
                } else {
                    x.powf(y)
                }
            }
        }))
    }

    fn namespace_call(&mut self, owner: &str, name: &str, args: &[Value]) -> Done<Value> {
        let v = arg(args, 0);
        Ok(match (owner, name) {
            ("Number", "isInteger") => {
                Value::Bool(matches!(v, Value::Number(x) if x.is_finite() && x.fract() == 0.0))
            }
            ("Number", _) => Value::Bool(matches!(v, Value::Number(x) if x.is_finite())),
            ("String", _) => {
                let bytes = self.byte_values(v).ok_or_else(|| {
                    Flow::Error(
                        "TypeError",
                        "String.fromBytes takes an array of bytes, 0 to 255".into(),
                    )
                })?;
                self.room(bytes.len())?;
                self.touch(bytes.len() / 64)?;
                self.string(String::from_utf8_lossy(&bytes))?
            }
            ("Array", "isArray") => Value::Bool(self.kind(v) == "array"),
            ("Array", _) => {
                let items = self.items(v)?;
                self.copy(items)?
            }
            _ => self.object_call(name, args)?,
        })
    }
}

fn parse_int(text: &str, radix: f64) -> f64 {
    let s = text.trim_start_matches(string::space);
    let (sign, s) = match s.strip_prefix('-') {
        Some(rest) => (-1.0, rest),
        None => (1.0, s.strip_prefix('+').unwrap_or(s)),
    };
    let hex = s.get(..2).is_some_and(|p| p.eq_ignore_ascii_case("0x"));
    let (radix, s) = match radix {
        0.0 if hex => (16, s.get(2..).unwrap_or_default()),
        0.0 => (10, s),
        16.0 if hex => (16, s.get(2..).unwrap_or_default()),
        r if (2.0..=36.0).contains(&r) => (
            crate::vm::index(r)
                .and_then(|r| u32::try_from(r).ok())
                .unwrap_or(10),
            s,
        ),
        _ => return f64::NAN,
    };
    let mut out = None;
    for digit in s.chars().map_while(|c| c.to_digit(radix)) {
        out = Some(out.unwrap_or(0.0) * f64::from(radix) + f64::from(digit));
    }
    out.map_or(f64::NAN, |x| sign * x)
}

fn parse_float(text: &str) -> f64 {
    let s = text.trim_start_matches(string::space);
    let body = s.trim_start_matches(['+', '-']);
    if body.starts_with("Infinity") {
        return if s.starts_with('-') {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
    }
    let mut end = s.len() - body.len().min(s.len());
    let digits = |from: usize| {
        s.get(from..).map_or(0, |r| {
            r.len() - r.trim_start_matches(|c: char| c.is_ascii_digit()).len()
        })
    };
    end += digits(end);
    if s.get(end..).is_some_and(|r| r.starts_with('.')) {
        end += 1 + digits(end + 1);
    }
    if s.get(end..).is_some_and(|r| r.starts_with(['e', 'E'])) {
        let sign = usize::from(s.get(end + 1..).is_some_and(|r| r.starts_with(['+', '-'])));
        let exp = digits(end + 1 + sign);
        if exp > 0 {
            end += 1 + sign + exp;
        }
    }
    s.get(..end)
        .and_then(|p| p.parse().ok())
        .unwrap_or(f64::NAN)
}
