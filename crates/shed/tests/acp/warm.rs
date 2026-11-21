use std::io::BufRead as _;
use std::path::Path;

use serde_json::{Value, json};
use shed_client::Conn;
use shed_server::Paths;
use shed_server::proto::{EventsParams, Method};

use crate::common::{Server, temp_dir};
use crate::{events_until, fake_agent, first_turn, ok, until};

fn spares(dir: &Path) -> Vec<(String, Value)> {
    let ls: Value = serde_json::from_str(&ok(dir, &["ls", "--json"])).expect("JSON");
    let ls = ls.as_array().expect("a list").iter();
    let running = ls.filter(|t| t["state"] == "running");
    let spare = running.filter(|t| t["name"].as_str().is_some_and(|n| n.starts_with(".warm-")));
    spare
        .map(|t| {
            (
                t["name"].as_str().expect("a name").to_owned(),
                t["pid"].clone(),
            )
        })
        .collect()
}

fn start_warm(dir: &Path, name: &str, cwd: &Path, history: &Path) {
    let agent = fake_agent();
    let label = format!("run={name}");
    let cwd = cwd.to_str().expect("utf-8");
    let run = ["run", "--name", name, "--label", &label, "--cwd", cwd];
    let cmd = ["--acp", "fake", "--warm", "--", "python3"];
    let history = history.to_str().expect("utf-8");
    let fake = [
        agent.to_str().expect("utf-8"),
        "--history",
        history,
        "--unique",
    ];
    ok(dir, &[&run[..], &cmd, &fake].concat());
}

#[test]
fn a_warm_spare_is_taken_renamed_and_replaced_within_its_limits_and_hibernates_as_one_session() {
    let dir = temp_dir("acp-warm");
    let history = dir.join("history.jsonl");
    let env = [
        ("SHED_ACP_HIBERNATE_AFTER", "2"),
        ("SHED_WARM_MAX", "1"),
        ("SHED_WARM_IDLE", "8"),
    ];
    let _server = Server::with_env(&dir, "0.3", &env);
    start_warm(&dir, "a", &dir, &history);
    let first = spares(&dir);
    assert_eq!(first.len(), 1, "{first:?}");
    let (spare, pid) = first[0].clone();
    assert_eq!(spare, ".warm-1");
    ok(
        &dir,
        &[
            "wait",
            &spare,
            "--output",
            "ACP session ready",
            "--timeout",
            "10",
        ],
    );
    start_warm(&dir, "b", &dir, &history);
    let b: Value = serde_json::from_str(&ok(&dir, &["get", "b"])).expect("JSON");
    assert_eq!(b["pid"], pid, "b is the spare");
    let id = b["session"]["id"].clone();
    assert!(id.as_str().is_some_and(|id| id.starts_with("sess-")), "{b}");
    assert_eq!(
        (&b["labels"]["run"], &b["session"]["ready_ms"]),
        (&json!("b"), &json!(0))
    );
    let next = spares(&dir);
    assert_eq!(next.len(), 1, "{next:?}");
    assert_eq!(next[0].0, ".warm-2");
    assert_ne!(next[0].1, pid);
    let elsewhere = dir.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).expect("a second folder");
    start_warm(&dir, "c", &elsewhere, &history);
    assert_eq!(spares(&dir), next, "at most SHED_WARM_MAX spares");

    first_turn(&dir, "b");
    until(&dir, "b", "hibernated");
    ok(&dir, &["send", "b", "more"]);
    until(&dir, "b", "done");
    let b: Value = serde_json::from_str(&ok(&dir, &["get", "b"])).expect("JSON");
    assert_eq!(b["session"]["id"], id);
    assert_eq!(
        ok(&dir, &["read", "b"]),
        "> hi\nyou said: hi\n> more\nyou said: more\n"
    );
    events_until(&dir, "b", |e| {
        e["event"] == "reply" && e["text"] == "you said: more"
    });

    let mut client = Conn::connect(&Paths::in_dir(&dir).socket).expect("connect");
    let all = EventsParams {
        labels: std::collections::BTreeMap::new(),
        since: Some(0),
    };
    let _: Value = client.call(Method::Events, &all).expect("events");
    let (reader, _writer) = client.split();
    let timeout = Some(std::time::Duration::from_secs(15));
    reader.get_ref().set_read_timeout(timeout).expect("timeout");
    let killed = reader.lines().map(|l| l.expect("an event line")).find(|l| {
        let e: Value = serde_json::from_str(l).expect("event JSON");
        e["event"] == "killed" && e["name"] == ".warm-2"
    });
    assert!(killed.is_some(), "the idle spare is killed");
    assert_eq!(spares(&dir), [], "no spare left");
    for name in ["a", "b", "c"] {
        let info: Value = serde_json::from_str(&ok(&dir, &["get", name])).expect("JSON");
        assert_eq!(info["state"], "running", "{name}");
    }
}
