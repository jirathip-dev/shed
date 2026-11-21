//! The worked examples, run by the engine against the answers Node recorded, give the reply Node made.
#![cfg(test)]

#[path = "common/answers.rs"]
mod answers;
mod common;

use std::path::Path;

use serde_json::{Map, Value as Json, json};
use shed_script::Program;
use shed_script::vm::{Limits, Run};

const GUARDED: &[&str] = &[
    "run",
    "send",
    "keys",
    "kill",
    "rm",
    "resize",
    "state",
    "acp.model",
    "exec",
    "file.write",
    "file.read",
];

fn reply(name: &str) -> String {
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name);
    let program = Program::parse(&std::fs::read_to_string(&script).unwrap()).unwrap();
    let host = answers::fixture(&script);
    let mut run = Run::new(&program, Limits::default(), &host);
    let outcome = common::drive(&mut run, &host);
    let state = host.state.lock().unwrap();
    let acting: Vec<Json> = state
        .calls
        .iter()
        .zip(&host.answers)
        .filter(|((method, _), _)| GUARDED.contains(&method.as_str()))
        .map(|((method, params), (_, answer))| match answer {
            Ok(result) => json!({"method": method, "params": params, "result": result}),
            Err(e) => json!({"method": method, "params": params, "error": {"code": e.code, "message": e.message}}),
        })
        .collect();
    let mut out = Map::new();
    match outcome {
        Ok(result) => {
            out.insert("result".into(), result);
        }
        Err(e) => {
            let mut error =
                json!({"name": e.name, "message": e.message, "line": e.line, "column": e.column});
            for (key, value) in [("code", e.code), ("method", e.method)] {
                if let Some(value) = value {
                    error[key] = json!(value);
                }
            }
            out.insert("error".into(), error);
        }
    }
    out.insert("logs".into(), json!(run.logs()));
    out.insert("calls".into(), json!(run.counts().calls));
    out.insert("acting".into(), json!(acting));
    out.insert("ms".into(), json!(state.now));
    format!("{}\n", Json::Object(out))
}

#[test]
fn examples_match_their_goldens() {
    for name in ["blocked.js", "start.js", "errors.js", "refused.js"] {
        let golden = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/golden")
            .join(name.replace(".js", ".reply.json"));
        assert_eq!(
            reply(name),
            std::fs::read_to_string(golden).unwrap(),
            "{name}"
        );
    }
}
