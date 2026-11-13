//! A real `shed serve` against a subscriber that never reads and a reconnect after a flood of text (#74).
#![cfg(test)]

mod common;

use std::io::{BufRead as _, Read as _, Write as _};
use std::process::Command;
use std::time::Duration;

use common::{Server, temp_dir};
use serde_json::{Value, json};
use shed_server::proto::{Event, EventKind, EventsParams, Method};
use shed_server::{Client, Paths, notification};

fn serve(name: &str) -> (Server, std::path::PathBuf) {
    let dir = temp_dir(name);
    (Server::start(&dir), Paths::in_dir(&dir).socket)
}

fn rss_kb(pid: u32) -> u64 {
    let ps = Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .expect("ps");
    let rss = String::from_utf8_lossy(&ps.stdout).trim().to_owned();
    rss.parse().expect("an RSS in KB")
}

fn run(client: &mut Client, name: &str) {
    let run = json!({"name": name, "cmd": ["sleep", "100"], "cwd": "/", "labels": {"lane": name}});
    let _: Value = client.call(Method::Run, &run).expect("run");
}

fn emit(name: &str, kind: &Value) -> Vec<u8> {
    let mut params = kind.clone();
    params["name"] = name.into();
    notification(Method::Emit, &params).expect("an emit line")
}

#[test]
fn a_subscriber_that_never_reads_100k_events_is_dropped_and_serve_stays_under_10_mb() {
    let (server, socket) = serve("never-reads");
    let mut client = Client::connect(&socket).expect("connect");
    run(&mut client, "t");
    let mut subscriber = Client::connect(&socket).expect("connect");
    let _: Value = subscriber
        .call(Method::Events, &EventsParams::default())
        .expect("events");
    let (mut never_read, _writer) = subscriber.split();
    let before = rss_kb(server.0.id());
    let tool = json!({"event": "tool_call", "id": "call-1", "title": "Read crates/shed-server/src/events.rs", "status": "in_progress"});
    let line = emit("t", &tool);
    let (mut emitter, _) = Client::connect(&socket).expect("connect").split();
    for _ in 0..100 {
        emitter
            .get_mut()
            .write_all(&line.repeat(1_000))
            .expect("1,000 emits");
    }
    let _: Value = client
        .call(Method::Get, &json!({"name": "t"}))
        .expect("get");
    let after = rss_kb(server.0.id());
    never_read
        .get_ref()
        .set_read_timeout(Some(Duration::from_secs(10)))
        .ok();
    let mut buffered = Vec::new();
    never_read
        .read_to_end(&mut buffered)
        .expect("the server closed it");
    let grew = after.saturating_sub(before);
    assert!(grew < 10 * 1024, "serve's RSS grew {grew} KB");
    assert!(buffered.len() < 1 << 20, "{} bytes held", buffered.len());
}

#[test]
fn a_reconnect_with_since_after_5000_text_chunks_from_other_lanes_gets_every_state_event() {
    let (_server, socket) = serve("text-flood");
    let mut client = Client::connect(&socket).expect("connect");
    for name in ["a", "b", "c"] {
        run(&mut client, name);
    }
    let text = json!({"event": "text", "text": "a chunk of the agent's reply"});
    let a = [
        json!({"event": "turn", "prompt": "go"}),
        json!({"event": "tool_call", "id": "1", "title": "Run tests", "status": "pending"}),
        json!({"event": "mode", "mode": "plan"}),
        json!({"event": "reply", "text": "done", "stop_reason": "end_turn"}),
    ];
    let (mut emitter, _) = Client::connect(&socket).expect("connect").split();
    for kind in &a {
        let chunks = [emit("b", &text), emit("c", &text)].concat().repeat(625);
        let burst = [emit("a", kind), chunks].concat();
        emitter.get_mut().write_all(&burst).expect("emits");
    }
    let _: Value = client
        .call(Method::Get, &json!({"name": "a"}))
        .expect("get");
    let mut events = Client::connect(&socket).expect("connect");
    let params = EventsParams {
        labels: [("lane".to_owned(), "a".to_owned())].into(),
        since: Some(0),
    };
    let _: Value = events.call(Method::Events, &params).expect("events");
    let reader = events.split().0;
    let wait = Some(Duration::from_secs(10));
    reader.get_ref().set_read_timeout(wait).expect("timeout");
    let mut got = Vec::new();
    for line in reader.lines().map_while(Result::ok) {
        let event: Event = serde_json::from_str(&line).expect("an event");
        let reply = matches!(event.kind, EventKind::Reply { .. });
        got.push(serde_json::to_value(&event.kind).expect("json")["event"].clone());
        if reply {
            break;
        }
    }
    assert_eq!(got, ["started", "turn", "tool_call", "mode", "reply"]);
}
