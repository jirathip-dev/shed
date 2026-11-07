//! The ACP benches (`scripts/bench-acp.sh` runs them): `fake_acp.py` streams 1,000 `session/update`s
//! through the bridge to a socket client, and 10,000 canned ones that time shed alone (both at a PR's
//! base and head), and a 1,000-update `session/load` replay (at the head).

mod common;

use std::io::BufRead as _;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

use common::{Server, temp_dir};
use serde_json::Value;
use shed_server::acp::{Input, frame};
use shed_server::proto::{EventsParams, Method, SendParams};
use shed_server::{Client, Paths};

/// Updates per turn or replay.
const UPDATES: usize = 1_000;

fn shed(dir: &Path, args: &[&str]) {
    let out = Command::new(env!("CARGO_BIN_EXE_shed"))
        .arg("--dir")
        .arg(dir)
        .args(args)
        .output()
        .expect("shed runs");
    assert!(out.status.success(), "shed {args:?} failed: {out:?}");
}

/// Starts the fake agent as terminal `name`, labelled `run=<name>`, `extra` passed to `run` and `fake`
/// to the agent.
fn start(dir: &Path, name: &str, extra: &[&str], fake: &[&str]) {
    let agent = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake_acp.py");
    let label = format!("run={name}");
    let run = ["run", "--name", name, "--label", &label, "--acp", "fake"];
    let cmd = ["--", "python3", agent.to_str().expect("utf-8")];
    shed(dir, &[&run[..], extra, &cmd, fake].concat());
}

/// The events of terminals labelled `run=<name>` from now on.
fn events(dir: &Path, name: &str) -> impl Iterator<Item = Value> + use<> {
    let mut client = Client::connect(&Paths::in_dir(dir).socket).expect("connect");
    let params = EventsParams {
        labels: [("run".to_owned(), name.to_owned())].into(),
        since: None,
    };
    let _: Value = client.call(Method::Events, &params).expect("events");
    let lines = client.split().0.lines();
    lines.map(|l| serde_json::from_str(&l.expect("an event line")).expect("event JSON"))
}

/// Reads `events` until one is `last`; returns how many were `text`.
fn texts_until(events: &mut impl Iterator<Item = Value>, last: &str) -> usize {
    let mut texts = 0;
    for e in events {
        if e["event"] == last {
            return texts;
        }
        texts += usize::from(e["event"] == "text");
    }
    panic!("the events ended before {last}");
}

/// The best of 3 turns of `updates` one-character chunks from fake agent `name` (started with `fake`),
/// at a socket client, in ns to the reply. `count` reads the client's events until the reply and returns
/// how many were text and when the time starts: the prompt's `sent`, or the first update.
fn best_turn<E>(
    name: &str,
    fake: &[&str],
    updates: usize,
    events: impl Fn(&Path) -> E,
    count: impl Fn(&mut E, Instant) -> (usize, Instant),
) -> u128 {
    let dir = temp_dir(&format!("acp-bench-{name}"));
    let _server = Server::start(&dir);
    start(&dir, name, &[], fake);
    let ready = [
        "wait",
        name,
        "--output",
        "ACP session ready",
        "--timeout",
        "10",
    ];
    shed(&dir, &ready);
    let mut client = Client::connect(&Paths::in_dir(&dir).socket).expect("connect");
    (0..3)
        .map(|_| {
            let mut events = events(&dir);
            let text = frame(&Input::Prompt(format!("flood {updates}")));
            let sent = Instant::now();
            let params = SendParams {
                name: name.to_owned(),
                text,
            };
            let _: Value = client.call(Method::Send, &params).expect("send");
            let (texts, first) = count(&mut events, sent);
            assert_eq!(texts, updates);
            first.elapsed().as_nanos()
        })
        .min()
        .unwrap_or_default()
}

#[test]
#[ignore = "a timing bench: scripts/bench-acp.sh"]
fn throughput() {
    let count = |e: &mut _, sent| (texts_until(e, "reply"), sent);
    let ns = best_turn("b", &[], UPDATES, |d| events(d, "b"), count);
    println!("acp throughput: {} us", ns / 1_000);
}

/// Shed's own share: the agent writes 10,000 canned update lines at once and the client counts raw event
/// lines without parsing them, so nearly all the time from the first update to the reply is the bridge
/// and the server (the turn's start, before the agent's 0.2 s pause, is not timed).
#[test]
#[ignore = "a timing bench: scripts/bench-acp.sh"]
fn shed_only() {
    const CANNED: usize = 10_000;
    let lines = |dir: &Path| {
        let mut client = Client::connect(&Paths::in_dir(dir).socket).expect("connect");
        let labels = [("run".to_owned(), "c".to_owned())].into();
        let params = EventsParams {
            labels,
            since: None,
        };
        let _: Value = client.call(Method::Events, &params).expect("events");
        client.split().0.lines()
    };
    let count = |lines: &mut std::io::Lines<_>, sent| {
        let (mut texts, mut first) = (0, sent);
        for line in lines.map(|l| l.expect("an event line")) {
            if line.contains(r#""event":"reply""#) {
                return (texts, first);
            }
            if line.contains(r#""event":"text""#) {
                texts += 1;
                first = if texts == 1 { Instant::now() } else { first };
            }
        }
        panic!("the events ended before the reply");
    };
    let ns = best_turn("c", &["--canned"], CANNED, lines, count);
    println!("acp shed-only: {} ns per update", ns / (CANNED as u128 - 1));
}

#[test]
#[ignore = "a timing bench: scripts/bench-acp.sh"]
fn replay() {
    let dir = temp_dir("acp-replay");
    let history = dir.join("history.jsonl");
    let chunk =
        r#"{"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "x"}}"#;
    std::fs::write(&history, format!("{chunk}\n").repeat(UPDATES)).expect("the history");
    let _server = Server::start(&dir);
    let mut events = events(&dir, "r");
    let began = Instant::now();
    let history = history.to_str().expect("utf-8");
    start(&dir, "r", &["--resume", "sess-1"], &["--history", history]);
    // The session event follows the replay.
    assert_eq!(texts_until(&mut events, "session"), UPDATES);
    println!("acp replay: {} us", began.elapsed().as_micros());
}
