use std::path::Path;
use std::process::Command;

use serde_json::{Value, json};
use shed_server::Paths;

use crate::common::{self, Server, temp_dir};
use crate::{SHED, events_until, ok, start_fake, until};

#[test]
fn a_usage_update_and_a_quota_block_show_in_get_top_and_as_an_event() {
    let dir = temp_dir("acp-usage");
    let _server = Server::start(&dir);
    let providers = Paths::in_dir(&dir).providers();
    std::fs::create_dir_all(&providers).expect("providers dir");
    let claude = include_str!("../../../shed-catalog/providers/claude.toml");
    let quota = claude
        .split_once("[quota]")
        .expect("claude-acp has a quota table")
        .1;
    let record = format!(
        "id = \"fake\"\n[quota]{}",
        quota.split("\n[").next().unwrap_or("")
    );
    std::fs::write(providers.join("fake.toml"), record).expect("write the record");
    start_fake(&dir, "g", "fake");
    ok(&dir, &["send", "g", "hi"]);
    ok(&dir, &["wait", "g", "--until", "done", "--timeout", "10"]);
    let info: Value = serde_json::from_str(&ok(&dir, &["get", "g"])).expect("JSON");
    assert_eq!(info["session"].get("usage"), None, "{info}");
    let column = |n: usize| {
        let top = ok(&dir, &["top"]);
        let cells = |l: &str| l.split_whitespace().nth(n).map(str::to_owned);
        let row = top.lines().find(|l| l.starts_with("g "));
        (top.lines().next().and_then(cells), row.and_then(cells))
    };
    let tokens = || column(7);
    assert_eq!(tokens(), (Some("TOKENS".into()), Some("-".into())));
    ok(&dir, &["send", "g", "usage"]);
    ok(&dir, &["wait", "g", "--until", "done", "--timeout", "10"]);
    let usage =
        json!({"used": 53_000, "size": 200_000, "cost": {"amount": 0.045, "currency": "USD"}});
    let events = events_until(&dir, "g", |e| e["event"] == "usage");
    let event: Value = serde_json::from_str(events.last().expect("a usage event")).expect("JSON");
    let fields = ["used", "size", "cost"].map(|k| event[k].clone());
    assert_eq!(fields, ["used", "size", "cost"].map(|k| usage[k].clone()));
    let info: Value = serde_json::from_str(&ok(&dir, &["get", "g"])).expect("JSON");
    assert_eq!(info["session"]["usage"], usage);
    assert_eq!(
        info["session"]["models"],
        json!([["fast", "Fast"], ["smart", "Smart"]])
    );
    ok(&dir, &["model", "g", "smart"]);
    common::eventually("the switch", || {
        let info: Value = serde_json::from_str(&ok(&dir, &["get", "g"])).expect("JSON");
        info["session"]["model"] == "smart" && info["session"]["usage"] == usage
    });
    assert_eq!(tokens(), (Some("TOKENS".into()), Some("53k/200k".into())));
    assert_eq!(column(8), (Some("CACHE".into()), Some("-".into())));
    assert!(!ok(&dir, &["ls"]).contains("TOKENS"));
    assert!(!ok(&dir, &["ls"]).contains("CACHE"));

    let turn = json!({"input": 100, "cache_read": 900, "cache_write": 1000, "output": 50});
    let total = json!({"input": 200, "cache_read": 1800, "cache_write": 2000, "output": 100});
    for _ in 0..2 {
        ok(&dir, &["send", "g", "quota"]);
        ok(&dir, &["wait", "g", "--until", "done", "--timeout", "10"]);
    }
    let events = events_until(&dir, "g", |e| e["event"] == "usage" && e["total"] == total);
    let event: Value = serde_json::from_str(events.last().expect("a usage event")).expect("JSON");
    assert_eq!((&event["turn"], &event["used"]), (&turn, &json!(53_000)));
    let info: Value = serde_json::from_str(&ok(&dir, &["get", "g"])).expect("JSON");
    assert_eq!(info["session"]["usage"]["turn"], turn);
    assert_eq!(info["session"]["usage"]["total"], total);
    assert_eq!(column(8), (Some("CACHE".into()), Some("45%".into())));
}

fn medians(dir: &Path, of: &str) -> Value {
    let out = Command::new(SHED)
        .arg("--dir")
        .arg(dir)
        .args(["--json", "agents"])
        .env("SHED_MEDIAN_OF", of)
        .output()
        .expect("shed runs");
    assert!(out.status.success(), "{out:?}");
    let list: Value = serde_json::from_slice(&out.stdout).expect("JSON");
    let fake = list
        .as_array()
        .and_then(|l| l.iter().find(|a| a["id"] == "fake"));
    fake.expect("the fake agent")["medians"].clone()
}

#[test]
fn each_session_records_start_to_ready_and_first_token_and_agents_gives_their_medians() {
    let dir = temp_dir("acp-timings");
    let _server = Server::start(&dir);
    let providers = Paths::in_dir(&dir).providers();
    std::fs::create_dir_all(&providers).expect("providers dir");
    std::fs::write(providers.join("fake.toml"), "id = \"fake\"\n").expect("write the record");
    let mut ready = Vec::new();
    let mut first = Vec::new();
    for (name, prompt) in [("a", "busy"), ("b", "hi"), ("c", "hi")] {
        start_fake(&dir, name, "fake");
        let ls: Value = serde_json::from_str(&ok(&dir, &["ls", "--json"])).expect("JSON");
        let row = |ls: &Value| {
            ls.as_array()
                .and_then(|l| l.iter().find(|t| t["name"] == name))
                .cloned()
        };
        let session = row(&ls).expect("listed")["session"].clone();
        assert!(session["ready_ms"].as_u64().is_some(), "{session}");
        assert_eq!(session.get("first_token_ms"), None, "{session}");
        ok(&dir, &["send", name, prompt]);
        until(&dir, name, "done");
        ok(&dir, &["send", name, "hi"]);
        until(&dir, name, "done");
        let ls: Value = serde_json::from_str(&ok(&dir, &["ls", "--json"])).expect("JSON");
        let session = row(&ls).expect("listed")["session"].clone();
        ready.push(session["ready_ms"].as_u64().expect("ready_ms"));
        first.push(session["first_token_ms"].as_u64().expect("first_token_ms"));
        ok(&dir, &["kill", name]);
    }
    assert!(
        first[0] >= 2000,
        "busy's first text comes after its 2.4 s of progress: {first:?}"
    );
    assert!(first[1] < 2000 && first[2] < 2000, "{first:?}");
    let middle = |v: &[u64]| {
        let mut v = v.to_vec();
        v.sort_unstable();
        v[1]
    };
    assert_eq!(
        medians(&dir, "20"),
        json!({"sessions": 3, "ready_ms": middle(&ready), "first_token_ms": middle(&first)})
    );
    assert_eq!(
        medians(&dir, "2"),
        json!({"sessions": 2, "ready_ms": ready[1].midpoint(ready[2]),
            "first_token_ms": first[1].midpoint(first[2])})
    );
}
