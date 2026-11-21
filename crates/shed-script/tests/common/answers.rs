//! A fixed script's recorded answers, `<name>.host.json`: in call order, `{at, result}` or `{at, error}`.

use std::path::Path;

use serde_json::Value as Json;
use shed_script::HostError;

use crate::common::{Answer, Fixture};

pub fn fixture(script: &Path) -> Fixture {
    let path = script.with_extension("host.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Fixture::default();
    };
    let list: Vec<Json> = serde_json::from_str(&text).unwrap();
    let answers = list.iter().map(answer).collect();
    Fixture {
        answers,
        ..Fixture::default()
    }
}

fn answer(entry: &Json) -> Answer {
    let at = entry["at"].as_u64().unwrap();
    let text = |e: &Json, key: &str| e[key].as_str().unwrap().to_owned();
    match entry.get("error") {
        Some(e) => (
            at,
            Err(HostError {
                name: text(e, "name"),
                code: text(e, "code"),
                message: text(e, "message"),
            }),
        ),
        None => (at, Ok(entry["result"].clone())),
    }
}
