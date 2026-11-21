//! `shed serve` end to end: a real server in a temp dir, driven over its socket.
//!
//! Every method's request and response is checked against `tests/golden/<method>.jsonl` (the temp
//! dir, version and pids normalised). Regenerate with `just generate` (the tests in `golden.rs`).
#![cfg(test)]

#[path = "../common/mod.rs"]
mod common;
mod events;
mod golden;
mod input;
mod protocol;
mod screen;

use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::net::UnixStream;
use std::path::Path;

use common::eventually;
use serde_json::{Value, json};
use shed_server::Paths;
use shed_server::proto::Hello;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn ours() -> Value {
    serde_json::to_value(Hello::ours()).expect("hello json")
}

struct Conn {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

impl Conn {
    fn open(dir: &Path) -> Self {
        let writer = UnixStream::connect(Paths::in_dir(dir).socket).expect("connect");
        Self {
            reader: BufReader::new(writer.try_clone().expect("clone")),
            writer,
        }
    }

    fn line(&mut self, request: &str) -> String {
        self.writer
            .write_all(format!("{request}\n").as_bytes())
            .expect("write");
        let mut reply = String::new();
        self.reader.read_line(&mut reply).expect("read");
        reply.trim_end().to_owned()
    }

    fn call(&mut self, id: u64, method: &str, params: &Value) -> (String, String, Value) {
        let request = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let request = request.to_string();
        let response = self.line(&request);
        let value = serde_json::from_str(&response).expect("response is JSON");
        (request, response, value)
    }

    fn next(&mut self) -> String {
        self.reader
            .get_ref()
            .set_read_timeout(Some(std::time::Duration::from_secs(10)))
            .expect("timeout");
        let mut line = String::new();
        self.reader.read_line(&mut line).expect("an event line");
        line.trim_end().to_owned()
    }

    fn event(&mut self) -> Value {
        serde_json::from_str(&self.next()).expect("event is JSON")
    }

    fn events(dir: &Path, params: &Value) -> Self {
        let mut c = Self::open(dir);
        c.result("hello", &ours());
        assert_eq!(c.result("events", params), json!({}));
        c
    }

    fn result(&mut self, method: &str, params: &Value) -> Value {
        let (_, line, response) = self.call(0, method, params);
        response
            .get("result")
            .cloned()
            .unwrap_or_else(|| panic!("{method} failed: {line}"))
    }
}

fn hello(dir: &Path) -> Conn {
    let mut c = Conn::open(dir);
    c.result("hello", &ours());
    c
}

fn shell(c: &mut Conn, dir: &Path, name: &str) {
    c.result("run", &json!({"name": name, "cwd": dir, "cmd": ["sh"]}));
    eventually("the prompt", || {
        c.result("read", &json!({"name": name}))["text"] == "$"
    });
}

fn rows(update: &Value) -> Vec<(u64, String)> {
    let lines = update["lines"].as_array().expect("lines");
    lines
        .iter()
        .map(|l| {
            let runs = l["runs"].as_array().expect("runs");
            let text = runs.iter().map(|r| r["text"].as_str().expect("text"));
            (l["row"].as_u64().expect("row"), text.collect())
        })
        .collect()
}
