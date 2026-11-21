use std::sync::Arc;

use serde_json::Value as Json;

use crate::value::{Cell, Value};
use crate::vm::{Done, Flow, Run, number_text};

const DEPTH: usize = 200;

impl Run {
    pub(crate) fn json(&self, value: Value, path: &str, depth: usize) -> Result<Json, String> {
        Ok(self
            .to_json(value, path, depth, false)?
            .unwrap_or(Json::Null))
    }

    pub(crate) fn to_json(
        &self,
        value: Value,
        path: &str,
        depth: usize,
        lenient: bool,
    ) -> Result<Option<Json>, String> {
        if depth > DEPTH {
            return Err(format!(
                "{path} is nested over {DEPTH} deep (does it hold itself?)"
            ));
        }
        Ok(Some(match value {
            Value::Undefined => return Ok(None),
            Value::Null => Json::Null,
            Value::Bool(b) => Json::Bool(b),
            Value::Number(x) if x.is_finite() => {
                if x.fract() == 0.0 && x.abs() <= 9_007_199_254_740_992.0 {
                    number_text(x).parse::<i64>().map_or(Json::Null, Json::from)
                } else {
                    serde_json::Number::from_f64(x).map_or(Json::Null, Json::Number)
                }
            }
            Value::Number(_) if lenient => Json::Null,
            Value::Number(x) => {
                return Err(format!(
                    "{path} is {}: JSON has no such number",
                    number_text(x)
                ));
            }
            _ => match self.cell(value) {
                Some(Cell::Str(s)) => Json::String(s.to_string()),
                Some(Cell::Array(items)) => Json::Array(
                    items
                        .iter()
                        .enumerate()
                        .map(|(at, v)| {
                            let path = format!("{path}[{at}]");
                            Ok(self
                                .to_json(*v, &path, depth + 1, lenient)?
                                .unwrap_or(Json::Null))
                        })
                        .collect::<Result<_, String>>()?,
                ),
                Some(Cell::Object(fields)) => {
                    let mut map = serde_json::Map::new();
                    for (key, v) in fields {
                        let path = format!("{path}.{key}");
                        if let Some(v) = self.to_json(*v, &path, depth + 1, lenient)? {
                            map.insert(key.to_string(), v);
                        }
                    }
                    Json::Object(map)
                }
                _ if lenient => return Ok(None),
                _ => return Err(format!("{path} is {}: JSON cannot carry it", self.a(value))),
            },
        }))
    }

    pub(crate) fn json_value(&mut self, json: &Json) -> Done<Value> {
        Ok(match json {
            Json::Null => Value::Null,
            Json::Bool(b) => Value::Bool(*b),
            Json::Number(n) => Value::Number(n.as_f64().unwrap_or(f64::NAN)),
            Json::String(s) => self.string(s.as_str())?,
            Json::Array(items) => {
                let mut out = Vec::with_capacity(items.len());
                for v in items {
                    out.push(self.json_value(v)?);
                }
                self.alloc(Cell::Array(out))?
            }
            Json::Object(map) => {
                let mut fields = Vec::with_capacity(map.len());
                for (key, v) in map {
                    fields.push((Arc::from(key.as_str()), self.json_value(v)?));
                }
                self.alloc(Cell::Object(fields))?
            }
        })
    }

    pub(crate) fn json_call(&mut self, name: &str, args: &[Value]) -> Done<Value> {
        let arg = |at: usize| args.get(at).copied().unwrap_or(Value::Undefined);
        if name == "parse" {
            let text = self
                .str(arg(0))
                .map(str::to_owned)
                .ok_or_else(|| Flow::Error("TypeError", "JSON.parse takes a string".into()))?;
            self.room(text.len())?;
            self.touch(text.len() / 64)?;
            let json = serde_json::from_str(&text)
                .map_err(|e| Flow::Error("SyntaxError", format!("JSON.parse: {e}")))?;
            return self.json_value(&json);
        }
        if !matches!(arg(1), Value::Undefined | Value::Null) {
            return Err(Flow::Error(
                "TypeError",
                "JSON.stringify takes no replacer: pass null, and map the value first".into(),
            ));
        }
        let indent = match (arg(2), self.str(arg(2))) {
            (Value::Number(n), _) => {
                " ".repeat(crate::vm::index(n.clamp(0.0, 10.0).trunc()).unwrap_or(0))
            }
            (_, Some(s)) => s.chars().take(10).collect(),
            _ => String::new(),
        };
        let json = self
            .to_json(arg(0), "the value", 0, true)
            .map_err(|m| Flow::Error("TypeError", m))?;
        match json {
            Some(json) => {
                let text = write(&json, &indent);
                self.touch(text.len() / 64)?;
                self.string(text)
            }
            None => Ok(Value::Undefined),
        }
    }
}

pub(crate) fn write(json: &Json, indent: &str) -> String {
    let mut out = String::new();
    put(&mut out, json, indent, 0);
    out
}

fn put(out: &mut String, json: &Json, indent: &str, depth: usize) {
    let (open, close, items): (char, char, Vec<(Option<&String>, &Json)>) = match json {
        Json::Array(items) => ('[', ']', items.iter().map(|v| (None, v)).collect()),
        Json::Object(map) => ('{', '}', map.iter().map(|(k, v)| (Some(k), v)).collect()),
        Json::Number(n) => return out.push_str(&number_text(n.as_f64().unwrap_or(0.0))),
        other => return out.push_str(&other.to_string()),
    };
    out.push(open);
    let line = |out: &mut String, depth: usize| {
        if !indent.is_empty() {
            out.push('\n');
            out.push_str(&indent.repeat(depth));
        }
    };
    for (at, (key, v)) in items.iter().enumerate() {
        if at > 0 {
            out.push(',');
        }
        line(out, depth + 1);
        if let Some(key) = key {
            out.push_str(&Json::String((*key).clone()).to_string());
            out.push(':');
            if !indent.is_empty() {
                out.push(' ');
            }
        }
        put(out, v, indent, depth + 1);
    }
    if !items.is_empty() {
        line(out, depth);
    }
    out.push(close);
}
