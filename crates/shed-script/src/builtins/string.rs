use std::cmp::Ordering;

use shed_pattern::Pattern;

use super::{arg, nth, place};
use crate::Host;
use crate::value::{Cell, Value};
use crate::vm::{Done, Flow, Run, count, type_error};

pub(crate) fn space(c: char) -> bool {
    (c.is_whitespace() && c != '\u{85}') || c == '\u{feff}'
}

fn byte_at(s: &str, chars: usize) -> usize {
    s.char_indices().nth(chars).map_or(s.len(), |(at, _)| at)
}

fn chars_in(s: &str, bytes: usize) -> usize {
    s.get(..bytes).map_or(0, |p| p.chars().count())
}

impl<H: Host> Run<'_, H> {
    pub(crate) fn string_method(&mut self, name: &str, this: Value, args: &[Value]) -> Done<Value> {
        let s = self.str(this).unwrap_or_default().to_owned();
        self.touch(s.len() / 64)?;
        let len = s.chars().count();
        let what = format!(".{name}");
        let number = |run: &Self, at, default| run.number(args, at, default, &what);
        let out: String = match name {
            "at" | "charAt" => {
                let x = number(self, 0, 0.0)?;
                let at = if name == "at" {
                    nth(x, len)
                } else {
                    crate::vm::index(x.trunc())
                };
                match at.and_then(|at| s.chars().nth(at)) {
                    Some(c) => c.into(),
                    None if name == "at" => return Ok(Value::Undefined),
                    None => String::new(),
                }
            }
            "slice" | "substring" => {
                let (mut from, mut to) = (number(self, 0, 0.0)?, number(self, 1, f64::INFINITY)?);
                if name == "substring" {
                    (from, to) = (
                        from.max(0.0).min(to.max(0.0)),
                        from.max(0.0).max(to.max(0.0)),
                    );
                }
                let (from, to) = (place(from, len), place(to, len));
                s.chars().skip(from).take(to.saturating_sub(from)).collect()
            }
            "split" => return self.split(&s, args),
            "trim" => s.trim_matches(space).into(),
            "trimStart" => s.trim_start_matches(space).into(),
            "trimEnd" => s.trim_end_matches(space).into(),
            "startsWith" | "endsWith" | "includes" | "indexOf" | "lastIndexOf" => {
                return self.search(name, &s, len, args);
            }
            "toLowerCase" => s.to_lowercase(),
            "toUpperCase" => s.to_uppercase(),
            "replace" | "replaceAll" | "padStart" | "padEnd" | "repeat" => {
                self.grow(name, s, len, args)?
            }
            "concat" => {
                let rest = self.texts(args)?.concat();
                self.room(s.len() + rest.len())?;
                s + &rest
            }
            "localeCompare" => {
                let t = self.text_arg(args, 0, &what)?;
                return Ok(Value::Number(match s.cmp(&t) {
                    Ordering::Less => -1.0,
                    Ordering::Equal => 0.0,
                    Ordering::Greater => 1.0,
                }));
            }
            "matches" => {
                let pattern = self.text_arg(args, 0, &what)?;
                let pattern = Pattern::new(&pattern)
                    .map_err(|e| Flow::Error("SyntaxError", e.to_string()))?;
                return Ok(Value::Bool(pattern.is_match(&s)));
            }
            _ => {
                self.room(s.len().saturating_mul(16))?;
                let bytes = s.bytes().map(|b| Value::Number(b.into())).collect();
                return self.alloc(Cell::Array(bytes));
            }
        };
        self.string(out)
    }

    fn search(&self, name: &str, s: &str, len: usize, args: &[Value]) -> Done<Value> {
        let what = format!(".{name}");
        let found = |at: Option<usize>| match at {
            Some(bytes) => count(chars_in(s, bytes)),
            None => Value::Number(-1.0),
        };
        let t = self.text_arg(args, 0, &what)?;
        let default = if matches!(name, "endsWith" | "lastIndexOf") {
            f64::INFINITY
        } else {
            0.0
        };
        let at = byte_at(
            s,
            place(self.number(args, 1, default, &what)?.max(0.0), len),
        );
        let (head, tail) = s.split_at_checked(at).unwrap_or((s, ""));
        Ok(match name {
            "startsWith" => Value::Bool(tail.starts_with(&*t)),
            "endsWith" => Value::Bool(head.ends_with(&*t)),
            "includes" => Value::Bool(tail.contains(&*t)),
            "indexOf" => found(tail.find(&*t).map(|b| b + at)),
            _ => found(
                s.get(..(at + t.len()).min(s.len()))
                    .and_then(|h| h.rfind(&*t)),
            ),
        })
    }

    fn grow(&mut self, name: &str, s: String, len: usize, args: &[Value]) -> Done<String> {
        let what = format!(".{name}");
        Ok(match name {
            "replace" | "replaceAll" => {
                let from = self.text_arg(args, 0, &format!("{what} (a string, not a regex)"))?;
                let to = self.text_arg(args, 1, &what)?;
                let hits = if from.is_empty() {
                    len + 1
                } else {
                    s.matches(&*from).count()
                };
                let times = if name == "replace" { 1 } else { hits };
                self.room(
                    s.len()
                        .saturating_add(to.len().saturating_mul(times.min(hits))),
                )?;
                s.replacen(&*from, &to, times)
            }
            "padStart" | "padEnd" => {
                let want = place(self.number(args, 0, 0.0, &what)?.max(0.0), usize::MAX >> 1);
                let fill = match arg(args, 1) {
                    Value::Undefined => " ".into(),
                    _ => self.text_arg(args, 1, &what)?,
                };
                if want <= len || fill.is_empty() {
                    s
                } else {
                    self.room(want.saturating_mul(4))?;
                    let pad: String = fill.chars().cycle().take(want - len).collect();
                    if name == "padStart" {
                        pad + &s
                    } else {
                        s + &pad
                    }
                }
            }
            _ => {
                let times = self.number(args, 0, 0.0, &what)?;
                let Some(times) = crate::vm::index(times.trunc()).filter(|_| times >= 0.0) else {
                    return Err(Flow::Error(
                        "RangeError",
                        format!(
                            "repeat({}): give a count from 0",
                            crate::vm::number_text(times)
                        ),
                    ));
                };
                self.room(s.len().saturating_mul(times))?;
                self.touch(s.len().saturating_mul(times) / 64)?;
                s.repeat(times)
            }
        })
    }

    fn split(&mut self, s: &str, args: &[Value]) -> Done<Value> {
        let limit = match arg(args, 1) {
            Value::Undefined => usize::MAX,
            _ => place(
                self.number(args, 1, 0.0, ".split")?.max(0.0),
                usize::MAX >> 1,
            ),
        };
        let parts: Vec<String> = match arg(args, 0) {
            Value::Undefined => vec![s.to_owned()],
            v => match self.str(v) {
                Some("") => s.chars().map(String::from).collect(),
                Some(sep) => s.split(sep).map(String::from).collect(),
                None => return type_error(format!(".split takes a string, not {}", self.a(v))),
            },
        };
        self.room(s.len().saturating_add(parts.len().saturating_mul(48)))?;
        let mut items = Vec::with_capacity(parts.len().min(limit));
        for part in parts.into_iter().take(limit) {
            items.push(self.string(part)?);
        }
        self.alloc(Cell::Array(items))
    }

    pub(crate) fn byte_values(&self, v: Value) -> Option<Vec<u8>> {
        match self.cell(v)? {
            Cell::Array(items) => items
                .iter()
                .map(|b| match b {
                    Value::Number(x) => crate::vm::index(*x).and_then(|b| u8::try_from(b).ok()),
                    _ => None,
                })
                .collect(),
            _ => None,
        }
    }
}
