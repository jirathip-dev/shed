use std::sync::Arc;

use super::{arg, nth, place};
use crate::value::{Cell, Handle, Value};
use crate::vm::{Done, Flow, Run, count, full, type_error};

const FLAT_DEPTH: usize = 200;

impl Run {
    pub(crate) fn copy(&mut self, items: Vec<Value>) -> Done<Value> {
        self.touch(items.len())?;
        self.alloc(Cell::Array(items))
    }

    fn put(&mut self, array: Handle, items: Vec<Value>) -> Done {
        self.heap.put(array, Cell::Array(items)).map_err(full)?;
        Ok(())
    }

    pub(crate) fn array_method(&mut self, name: &str, this: Value, args: &[Value]) -> Done<Value> {
        let Some(handle) = this.handle() else {
            return Ok(Value::Undefined);
        };
        let mut items = self.items(this)?;
        self.touch(items.len())?;
        let len = items.len();
        let what = format!(".{name}");
        let at = |run: &Self, at: usize, default: f64| -> Done<usize> {
            Ok(place(run.number(args, at, default, &what)?, len))
        };
        Ok(match name {
            "at" => {
                let x = self.number(args, 0, 0.0, &what)?;
                nth(x, len)
                    .and_then(|i| items.get(i).copied())
                    .unwrap_or(Value::Undefined)
            }
            "push" => {
                self.heap.extend(handle, args).map_err(full)?;
                count(len + args.len())
            }
            "pop" | "shift" => {
                let v = if name == "pop" {
                    items.pop()
                } else {
                    (!items.is_empty()).then(|| items.remove(0))
                };
                self.put(handle, items)?;
                v.unwrap_or(Value::Undefined)
            }
            "unshift" => {
                let all = args.iter().copied().chain(items).collect();
                self.put(handle, all)?;
                count(len + args.len())
            }
            "reverse" => {
                items.reverse();
                self.put(handle, items)?;
                this
            }
            "slice" => {
                let (from, to) = (at(self, 0, 0.0)?, at(self, 1, f64::INFINITY)?);
                self.copy(items.get(from..to.max(from)).unwrap_or_default().to_vec())?
            }
            "concat" => {
                for v in args {
                    match self.cell(*v) {
                        Some(Cell::Array(more)) => items.extend_from_slice(more),
                        _ => items.push(*v),
                    }
                }
                self.copy(items)?
            }
            "join" => self.join(&items, args)?,
            "includes" | "indexOf" | "lastIndexOf" => self.find_item(name, &items, args)?,
            "flat" => {
                let depth = place(self.number(args, 0, 1.0, &what)?.max(0.0), FLAT_DEPTH);
                let mut out = Vec::with_capacity(len);
                self.flatten(&items, depth, &mut out)?;
                self.copy(out)?
            }
            _ => {
                let mut pairs = Vec::with_capacity(len);
                for (i, v) in items.into_iter().enumerate() {
                    pairs.push(self.alloc(Cell::Array(vec![count(i), v]))?);
                }
                self.copy(pairs)?
            }
        })
    }

    fn join(&mut self, items: &[Value], args: &[Value]) -> Done<Value> {
        let sep = match arg(args, 0) {
            Value::Undefined => ",".into(),
            _ => self.text_arg(args, 0, ".join")?,
        };
        let mut texts = Vec::with_capacity(items.len());
        for v in items {
            texts.push(match v {
                Value::Undefined | Value::Null => String::new(),
                v => self.texts(&[*v])?.concat(),
            });
        }
        let bytes = texts.iter().map(String::len).sum::<usize>();
        self.room(bytes.saturating_add(sep.len().saturating_mul(items.len())))?;
        self.touch(bytes / 64)?;
        self.string(texts.join(&sep))
    }

    fn find_item(&self, name: &str, items: &[Value], args: &[Value]) -> Done<Value> {
        let (want, len) = (arg(args, 0), items.len());
        let same = |v: &Value| {
            self.same(*v, want)
                || (name == "includes"
                    && matches!((v, want), (Value::Number(a), Value::Number(b)) if a.is_nan() && b.is_nan()))
        };
        let found = if name == "lastIndexOf" {
            let to = self.number(args, 1, f64::INFINITY, ".lastIndexOf")?;
            let head = match nth(to, len) {
                Some(to) => items.get(..=to.min(len.saturating_sub(1))),
                None if to >= 0.0 => Some(items),
                None => None,
            };
            head.and_then(|head| head.iter().rposition(same))
        } else {
            let from = place(self.number(args, 1, 0.0, &format!(".{name}"))?, len);
            items
                .get(from..)
                .and_then(|tail| tail.iter().position(same))
                .map(|i| i + from)
        };
        Ok(match (name, found) {
            ("includes", found) => Value::Bool(found.is_some()),
            (_, Some(i)) => count(i),
            _ => Value::Number(-1.0),
        })
    }

    fn flatten(&mut self, items: &[Value], depth: usize, out: &mut Vec<Value>) -> Done {
        for v in items {
            match self.cell(*v) {
                Some(Cell::Array(inner)) if depth > 0 => {
                    let inner = inner.clone();
                    self.touch(inner.len())?;
                    self.flatten(&inner, depth - 1, out)?;
                }
                _ => out.push(*v),
            }
        }
        Ok(())
    }

    fn fields(&mut self, v: Value, what: &str) -> Done<Vec<(Arc<str>, Value)>> {
        let fields = match self.cell(v) {
            Some(Cell::Object(fields)) => fields.clone(),
            Some(Cell::Array(items)) => items
                .iter()
                .enumerate()
                .map(|(i, v)| (Arc::from(i.to_string()), *v))
                .collect(),
            _ => return type_error(format!("{what} takes an object, not {}", self.a(v))),
        };
        self.touch(fields.len())?;
        Ok(fields)
    }

    pub(crate) fn object_call(&mut self, name: &str, args: &[Value]) -> Done<Value> {
        let what = format!("Object.{name}");
        let v = arg(args, 0);
        match name {
            "keys" | "values" | "entries" => {
                let fields = self.fields(v, &what)?;
                let mut out = Vec::with_capacity(fields.len());
                for (key, v) in fields {
                    let key = if name == "values" {
                        v
                    } else {
                        self.string(&*key)?
                    };
                    out.push(match name {
                        "entries" => self.alloc(Cell::Array(vec![key, v]))?,
                        _ => key,
                    });
                }
                self.alloc(Cell::Array(out))
            }
            "fromEntries" => {
                let mut fields: Vec<(Arc<str>, Value)> = Vec::new();
                for pair in self.items(v)? {
                    let (key, v) = match self.cell(pair) {
                        Some(Cell::Array(kv)) => (kv.first().copied(), kv.get(1).copied()),
                        _ => (None, None),
                    };
                    let key = self.key(key.unwrap_or(Value::Undefined))?;
                    let v = v.unwrap_or(Value::Undefined);
                    match fields.iter_mut().find(|(k, _)| *k == key) {
                        Some((_, old)) => *old = v,
                        None => fields.push((key, v)),
                    }
                }
                self.touch(fields.len())?;
                self.alloc(Cell::Object(fields))
            }
            "assign" => {
                if !matches!(self.cell(v), Some(Cell::Object(_))) {
                    return type_error(format!("Object.assign takes an object, not {}", self.a(v)));
                }
                for from in args.iter().skip(1) {
                    if matches!(from, Value::Undefined | Value::Null) {
                        continue;
                    }
                    for (key, field) in self.fields(*from, &what)? {
                        self.field(v, &key, field)?;
                    }
                }
                Ok(v)
            }
            "hasOwn" => {
                let key = self.key(arg(args, 1))?;
                Ok(Value::Bool(
                    self.fields(v, &what)?.iter().any(|(k, _)| *k == key),
                ))
            }
            _ => Err(Flow::Error(
                "TypeError",
                format!("{what} is not a function"),
            )),
        }
    }
}
