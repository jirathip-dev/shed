use std::path::Path;

use serde_json::{Value, json};
use shed_core::proctree::{self, ProcessTable as _};

use crate::common::temp_dir;
use crate::{
    events_until, fake_agent, first_turn, ok, reported, row, shed, sleepy_server, start_fake,
    start_with_history, until,
};

#[test]
fn top_and_ls_show_a_hibernated_agent_as_hibernated() {
    let dir = temp_dir("acp-top-hibernated");
    let _server = sleepy_server(&dir);
    start_with_history(&dir, "h", &dir.join("history.jsonl"), &[]);
    first_turn(&dir, "h");
    until(&dir, "h", "hibernated");
    for args in [&["top"][..], &["ls"]] {
        assert!(
            row(&dir, args, "h").contains(&"STATE=hibernated".to_owned()),
            "{args:?}"
        );
    }
    let top = ok(&dir, &["top"]);
    let totals = top.lines().last().unwrap_or_default();
    assert!(totals.starts_with("0 awake, 1 hibernated; agent "), "{top}");
}

fn processes(dir: &Path, name: &str) -> usize {
    let info: Value = serde_json::from_str(&ok(dir, &["get", name])).expect("JSON");
    let pid = i32::try_from(info["pid"].as_i64().expect("a pid")).expect("a pid");
    let procs = proctree::system().list().expect("the process table");
    proctree::tree(&procs, pid, &std::collections::BTreeSet::new()).len()
}

fn past_the_delay() {
    shed_server::clock::sleep(std::time::Duration::from_millis(4500));
}

fn settles_at(dir: &Path, name: &str, count: usize) {
    let settled = || processes(dir, name) == count;
    let within = shed_server::clock::PATIENCE;
    assert!(
        shed_server::clock::poll(within, settled),
        "{name}: never {count} processes"
    );
}

#[test]
fn an_idle_agent_hibernates_but_not_mid_turn_or_while_a_request_waits() {
    let dir = temp_dir("acp-hibernate");
    let history = dir.join("history.jsonl");
    let _server = sleepy_server(&dir);
    start_with_history(&dir, "h", &history, &[]);
    first_turn(&dir, "h");
    until(&dir, "h", "hibernated");
    settles_at(&dir, "h", 1);
    assert_eq!(
        reported(&dir, "h"),
        json!({"state": "hibernated", "from": "reported"})
    );
    let info: Value = serde_json::from_str(&ok(&dir, &["get", "h"])).expect("JSON");
    assert_eq!(
        (&info["state"], &info["session"]["id"]),
        (&json!("running"), &json!("sess-1"))
    );
    ok(&dir, &["send", "h", "slow"]);
    past_the_delay();
    assert_eq!(reported(&dir, "h")["state"], "working");
    settles_at(&dir, "h", 2);
    ok(&dir, &["cancel", "h"]);
    until(&dir, "h", "done");
    ok(&dir, &["send", "h", "tool"]);
    until(&dir, "h", "waiting-approval");
    past_the_delay();
    assert_eq!(reported(&dir, "h")["state"], "waiting-approval");
    settles_at(&dir, "h", 2);
    ok(&dir, &["approve", "h", "allow-once"]);
    until(&dir, "h", "done");
    until(&dir, "h", "hibernated");
    settles_at(&dir, "h", 1);
}

#[test]
fn meta_reaches_the_agent_unchanged_on_new_on_a_wake_and_on_resume() {
    let dir = temp_dir("acp-meta");
    let history = dir.join("history.jsonl");
    let meta = json!({"vendorX": {"opts": {"layers": ["a", "b"], "strict": true}}, "n": 1});
    let file = dir.join("meta.json");
    std::fs::write(&file, meta.to_string()).expect("write the meta");
    let _server = sleepy_server(&dir);
    let (agent, meta) = (fake_agent(), meta.to_string());
    let start = |name: &str, extra: &[&str]| {
        let run = ["run", "--name", name, "--acp", "fake", "--meta", &meta];
        let cmd = [
            "--",
            "python3",
            agent.to_str().expect("utf-8"),
            "--history",
            history.to_str().expect("utf-8"),
            "--meta",
            file.to_str().expect("utf-8"),
        ];
        ok(&dir, &[&run[..], extra, &cmd].concat());
    };
    start("a", &[]);
    first_turn(&dir, "a");
    until(&dir, "a", "hibernated");
    first_turn(&dir, "a");
    start("b", &["--resume", "sess-1"]);
    first_turn(&dir, "b");
    let info: Value = serde_json::from_str(&ok(&dir, &["get", "b"])).expect("JSON");
    assert_eq!(info["session"]["id"], "sess-1");
    for bad in ["[1]", "{"] {
        let out = shed(&dir, &["run", "--acp", "fake", "--meta", bad]);
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(
            !out.status.success() && err.contains("invalid value") && err.contains("--meta"),
            "{bad}: {err}"
        );
    }
}

#[test]
fn a_prompt_wakes_the_agent_and_its_history_is_not_repeated() {
    let dir = temp_dir("acp-wake");
    let history = dir.join("history.jsonl");
    let _server = sleepy_server(&dir);
    start_with_history(&dir, "h", &history, &[]);
    ok(&dir, &["send", "h", "hi"]);
    until(&dir, "h", "done");
    until(&dir, "h", "hibernated");
    ok(&dir, &["send", "h", "more"]);
    until(&dir, "h", "done");
    assert_eq!(processes(&dir, "h"), 2, "awake again");
    assert_eq!(
        ok(&dir, &["read", "h"]),
        "> hi\nyou said: hi\n> more\nyou said: more\n"
    );
    let events = events_until(&dir, "h", |e| {
        e["event"] == "reply" && e["text"] == "you said: more"
    });
    let acp: Vec<Value> = events
        .iter()
        .map(|l| serde_json::from_str::<Value>(l).expect("JSON"))
        .filter(|e| ["turn", "reply"].contains(&e["event"].as_str().unwrap_or_default()))
        .map(|e| {
            json!([
                e["event"],
                e.get("prompt").or(e.get("text")),
                e.get("replay")
            ])
        })
        .collect();
    assert_eq!(
        acp,
        [
            json!(["turn", "hi", null]),
            json!(["reply", "you said: hi", null]),
            json!(["turn", "more", null]),
            json!(["reply", "you said: more", null]),
        ]
    );
}

#[test]
fn sends_during_a_wake_are_delivered_in_order() {
    let dir = temp_dir("acp-wake-order");
    let history = dir.join("history.jsonl");
    let _server = sleepy_server(&dir);
    start_with_history(&dir, "h", &history, &[]);
    first_turn(&dir, "h");
    until(&dir, "h", "hibernated");
    ok(&dir, &["model", "h", "smart"]);
    ok(&dir, &["send", "h", "one"]);
    ok(&dir, &["send", "h", "two"]);
    events_until(&dir, "h", |e| {
        e["event"] == "reply" && e["text"] == "[smart] you said: two"
    });
    let read = ok(&dir, &["read", "h"]);
    let (prompts, replies): (Vec<&str>, Vec<&str>) =
        read.lines().partition(|l| l.starts_with("> "));
    assert_eq!(prompts, ["> hi", "> one", "> two"], "{read}");
    assert_eq!(
        replies,
        [
            "you said: hi",
            "[smart] you said: one",
            "[smart] you said: two"
        ],
        "{read}"
    );
}

#[test]
fn no_hibernation_without_load_session_with_no_hibernate_or_before_a_turn() {
    let dir = temp_dir("acp-awake");
    let _server = sleepy_server(&dir);
    start_fake(&dir, "a", "fake");
    start_with_history(&dir, "b", &dir.join("b.jsonl"), &["--no-hibernate"]);
    start_with_history(&dir, "c", &dir.join("c.jsonl"), &[]);
    for name in ["a", "b"] {
        first_turn(&dir, name);
    }
    past_the_delay();
    for name in ["a", "b"] {
        assert_eq!(reported(&dir, name)["state"], "done", "{name}");
        assert_eq!(processes(&dir, name), 2, "{name}");
    }
    assert_ne!(reported(&dir, "c")["state"], "hibernated");
    assert_eq!(processes(&dir, "c"), 2);
}
