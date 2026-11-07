//! The ACP benches (`scripts/bench-acp.sh` runs them): `fake_acp.py` streams 1,000 `session/update`s
//! through the bridge to a socket client (at a PR's base and head), and a 1,000-update `session/load`
//! replay and the hibernation bench (at the head).

mod common;

use std::io::BufRead as _;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

use common::{Server, temp_dir};
use serde_json::Value;
use shed_core::proctree::{self, ProcessTable as _};
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
fn events(dir: &Path, name: &str) -> impl Iterator<Item = Value> {
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

#[test]
#[ignore = "a timing bench: scripts/bench-acp.sh"]
fn throughput() {
    let dir = temp_dir("acp-bench");
    let _server = Server::start(&dir);
    start(&dir, "b", &[], &[]);
    let ready = [
        "wait",
        "b",
        "--output",
        "ACP session ready",
        "--timeout",
        "10",
    ];
    shed(&dir, &ready);
    let mut client = Client::connect(&Paths::in_dir(&dir).socket).expect("connect");
    // The best of 3 turns: from the prompt to its reply at a socket client.
    let us = (0..3)
        .map(|_| {
            let mut events = events(&dir, "b");
            let text = frame(&Input::Prompt(format!("flood {UPDATES}")));
            let start = Instant::now();
            let params = SendParams {
                name: "b".to_owned(),
                text,
            };
            let _: Value = client.call(Method::Send, &params).expect("send");
            assert_eq!(texts_until(&mut events, "reply"), UPDATES);
            start.elapsed().as_micros()
        })
        .min()
        .unwrap_or_default();
    println!("acp throughput: {us} us");
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

/// The output of a `shed` call that must succeed.
fn out(dir: &Path, args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_shed"))
        .arg("--dir")
        .arg(dir)
        .args(args)
        .output()
        .expect("shed runs");
    assert!(out.status.success(), "shed {args:?} failed: {out:?}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The summed RSS (KiB) and process count of terminal `name`'s tree: its bridge and its agent's.
fn rss(dir: &Path, name: &str) -> (u64, usize) {
    let info: Value = serde_json::from_str(&out(dir, &["get", name])).expect("JSON");
    let pid = i32::try_from(info["pid"].as_i64().expect("a pid")).expect("a pid");
    let procs = proctree::system().list().expect("the process table");
    let usage = proctree::usage(&proctree::tree(
        &procs,
        pid,
        &std::collections::BTreeSet::new(),
    ));
    (usage.rss_bytes / 1024, usage.processes)
}

/// Sends `prompt` to `name` (asleep); returns the ms to its first update that is not a replay, then
/// waits for the reply.
fn wake(dir: &Path, name: &str, prompt: &str) -> u128 {
    let mut events = events(dir, name).filter(|e| e["replay"] != true);
    let began = Instant::now();
    out(dir, &["send", name, prompt]);
    let first =
        events.find(|e| ["text", "tool_call", "plan"].contains(&e["event"].as_str().unwrap_or("")));
    assert!(first.is_some(), "no update");
    let ms = began.elapsed().as_millis();
    out(
        dir,
        &["wait", name, "--until", "done", "error", "--timeout", "300"],
    );
    ms
}

#[test]
#[ignore = "a memory bench: scripts/bench-acp.sh"]
fn hibernate() {
    let n: usize = std::env::var("SHED_BENCH_AGENTS").map_or(10, |n| n.parse().expect("a count"));
    let mut totals = Vec::new();
    for after in ["0", "1"] {
        let dir = temp_dir(&format!("acp-hibernate-{after}"));
        let _server = Server::with_env(&dir, "3600", &[("SHED_ACP_HIBERNATE_AFTER", after)]);
        let names: Vec<String> = (0..n).map(|i| format!("a{i}")).collect();
        for name in &names {
            let history = dir.join(format!("{name}.jsonl"));
            start(
                &dir,
                name,
                &[],
                &["--history", history.to_str().expect("utf-8")],
            );
            out(
                &dir,
                &[
                    "wait",
                    name,
                    "--output",
                    "ACP session ready",
                    "--timeout",
                    "10",
                ],
            );
            out(&dir, &["send", name, "hi"]);
            out(&dir, &["wait", name, "--until", "done", "--timeout", "10"]);
        }
        // Idle for the same time either way: past the delay, then until every agent sleeps.
        std::thread::sleep(std::time::Duration::from_secs(2));
        if after != "0" {
            for name in &names {
                out(
                    &dir,
                    &["wait", name, "--until", "hibernated", "--timeout", "10"],
                );
            }
        }
        let total: u64 = names.iter().map(|name| rss(&dir, name).0).sum();
        let state = if after == "0" { "off" } else { "on" };
        println!("acp hibernate: {n} idle agents, hibernation {state}: {total} KiB");
        totals.push(total);
        if after != "0" {
            println!(
                "acp hibernate: fake agent wake {} ms",
                wake(&dir, "a0", "again")
            );
        }
    }
    assert!(
        totals[1] < totals[0],
        "hibernated agents take no less memory: {totals:?}"
    );
}

/// `SHED_BENCH_LIVE` (an adapter command, e.g. `npx -y @agentclientprotocol/claude-agent-acp@0.100.0`,
/// logged in): its RSS awake and hibernated, and its wake time; the conversation goes on after it.
#[test]
#[ignore = "a live bench: needs SHED_BENCH_LIVE and a logged-in agent"]
fn hibernate_live() {
    let live = std::env::var("SHED_BENCH_LIVE").expect("SHED_BENCH_LIVE");
    let dir = temp_dir("acp-hibernate-live");
    let _server = Server::with_env(&dir, "3600", &[("SHED_ACP_HIBERNATE_AFTER", "5")]);
    let cwd = dir.join("work");
    std::fs::create_dir_all(&cwd).expect("a work dir");
    let run = [
        "run",
        "--name",
        "c",
        "--label",
        "run=c",
        "--acp",
        "claude-acp",
    ];
    let cwd = ["--cwd", cwd.to_str().expect("utf-8"), "--"];
    out(
        &dir,
        &[&run[..], &cwd, &live.split_whitespace().collect::<Vec<_>>()].concat(),
    );
    out(
        &dir,
        &[
            "wait",
            "c",
            "--output",
            "ACP session ready",
            "--timeout",
            "120",
        ],
    );
    out(
        &dir,
        &[
            "send",
            "c",
            "Remember the word lantern. Reply with just OK.",
        ],
    );
    out(
        &dir,
        &["wait", "c", "--until", "done", "error", "--timeout", "300"],
    );
    let (awake, procs) = rss(&dir, "c");
    println!("acp hibernate live: awake {awake} KiB in {procs} processes");
    out(
        &dir,
        &["wait", "c", "--until", "hibernated", "--timeout", "60"],
    );
    let (asleep, procs) = rss(&dir, "c");
    println!("acp hibernate live: hibernated {asleep} KiB in {procs} processes");
    assert_eq!(procs, 1, "only the bridge is left");
    let ms = wake(&dir, "c", "Which word did I ask you to remember? One word.");
    println!("acp hibernate live: wake {ms} ms");
    println!("acp hibernate live: read\n{}", out(&dir, &["read", "c"]));
}
