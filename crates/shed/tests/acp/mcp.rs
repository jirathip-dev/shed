use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::common::{Server, temp_dir};
use crate::{
    events_until, fake_agent, first_turn, is_state, ok, reported, shed, sleepy_server, until,
};

fn mcp_file(name: &str, servers: &Value) -> PathBuf {
    let path = temp_dir(name).join("mcp.json");
    std::fs::write(&path, servers.to_string()).expect("write the MCP servers");
    path
}

fn start_with_mcp(dir: &Path, name: &str, file: &Path, extra: &[&str], fake: &[&str]) {
    let (agent, file) = (fake_agent(), file.to_str().expect("utf-8"));
    let run = ["run", "--name", name, "--label", "run=mcp", "--acp", "fake"];
    let cmd = [
        "--",
        "python3",
        agent.to_str().expect("utf-8"),
        "--mcp",
        file,
    ];
    ok(
        dir,
        &[&run[..], &["--mcp", file], extra, &cmd, fake].concat(),
    );
}

const SECRET: &str = "s3cret-mcp-value";

#[test]
fn mcp_servers_pass_through_on_new_and_resume_with_their_values_hidden() {
    let dir = temp_dir("acp-mcp");
    let history = dir.join("history.jsonl");
    let history = history.to_str().expect("utf-8");
    let servers = |value: &str| {
        json!([
            {"name": "files", "command": "/bin/echo", "args": ["hi"],
             "env": [{"name": "TOKEN", "value": value}]},
            {"type": "http", "name": "web", "url": "https://mcp.example/",
             "headers": [{"name": "Authorization", "value": value}]},
        ])
    };
    let file = mcp_file("acp-mcp-in", &servers(SECRET));
    let _server = Server::start(&dir);
    let fake = ["--history", history, "--mcp-caps", "http"];
    let ready = |name| {
        [
            "wait",
            name,
            "--output",
            "ACP session ready",
            "--timeout",
            "10",
        ]
    };
    let turn = |name, prompt| {
        ok(&dir, &["send", name, prompt]);
        ok(&dir, &["wait", name, "--until", "done", "--timeout", "10"]);
    };
    start_with_mcp(&dir, "m1", &file, &[], &fake);
    ok(&dir, &ready("m1"));
    turn("m1", "hi");
    start_with_mcp(&dir, "m2", &file, &["--resume", "sess-1"], &fake);
    ok(&dir, &ready("m2"));
    turn("m2", "more");
    let info: Value = serde_json::from_str(&ok(&dir, &["get", "m2"])).expect("JSON");
    assert_eq!(info["session"]["mcp_servers"], servers("***"));

    let events = events_until(&dir, "mcp", |e| e["name"] == "m2" && is_state("done")(e));
    let mut shown = events.join("\n");
    for name in ["m1", "m2"] {
        shown.push_str(&ok(&dir, &["get", name]));
    }
    assert_hidden(&dir, shown);
}

fn assert_hidden(dir: &Path, mut shown: String) {
    shown.push_str(&ok(dir, &["ls", "--json"]));
    assert!(shown.contains("TOKEN") && shown.contains("***"), "{shown}");
    assert!(!shown.contains(SECRET), "{shown}");
    let mut dirs = vec![dir.to_owned()];
    while let Some(d) = dirs.pop() {
        for entry in std::fs::read_dir(&d).expect("read the state dir") {
            let path = entry.expect("an entry").path();
            if path.is_dir() {
                dirs.push(path);
            } else if let Ok(bytes) = std::fs::read(&path) {
                let text = String::from_utf8_lossy(&bytes);
                assert!(!text.contains(SECRET), "{} has the secret", path.display());
            }
        }
    }
}

#[test]
fn sse_servers_pass_only_when_the_agent_advertises_them() {
    let dir = temp_dir("acp-mcp-sse");
    let _server = Server::start(&dir);
    let servers = json!([{"type": "sse", "name": "events", "url": "https://mcp.example/sse",
                          "headers": []}]);
    let file = mcp_file("acp-mcp-sse-in", &servers);
    start_with_mcp(&dir, "s1", &file, &[], &["--mcp-caps", "sse"]);
    ok(
        &dir,
        &[
            "wait",
            "s1",
            "--output",
            "ACP session ready",
            "--timeout",
            "10",
        ],
    );
    start_with_mcp(&dir, "s2", &file, &[], &["--mcp-caps", "http"]);
    ok(&dir, &["wait", "s2", "--until", "error", "--timeout", "10"]);
    let why = "acp: MCP server events: the agent takes no sse servers (mcpCapabilities)";
    assert_eq!(
        reported(&dir, "s2"),
        json!({"state": "error", "note": why, "from": "reported"})
    );
}

#[test]
fn unknown_or_malformed_mcp_servers_are_refused() {
    let dir = temp_dir("acp-mcp-bad");
    let _server = Server::start(&dir);
    let agent = fake_agent();
    let bad = [
        (
            json!([{"type": "ws", "name": "x", "command": "c"}]),
            r#"mcp_servers[0] "x""#,
        ),
        (
            json!([{"name": "f", "command": "c", "args": [], "env": []}, {"name": "y"}]),
            r#"mcp_servers[1] "y""#,
        ),
        (json!({"name": "z"}), "not a JSON list"),
    ];
    for (servers, why) in bad {
        let file = mcp_file("acp-mcp-bad-in", &servers);
        let file = file.to_str().expect("utf-8");
        let run = ["run", "--acp", "fake", "--mcp", file, "--", "python3"];
        let out = shed(
            &dir,
            &[&run[..], &[agent.to_str().expect("utf-8")]].concat(),
        );
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(
            !out.status.success() && err.contains(why),
            "{servers}: {err}"
        );
    }
    let ls: Value = serde_json::from_str(&ok(&dir, &["ls", "--json"])).expect("JSON");
    assert_eq!(ls, json!([]));
}

#[test]
fn mcp_secrets_stay_in_memory_through_a_hibernation() {
    let dir = temp_dir("acp-hibernate-mcp");
    let history = dir.join("history.jsonl");
    let history = history.to_str().expect("utf-8");
    let servers = json!([{"name": "files", "command": "/bin/echo", "args": [],
                          "env": [{"name": "TOKEN", "value": SECRET}]}]);
    let file = mcp_file("acp-hibernate-mcp-in", &servers);
    let _server = sleepy_server(&dir);
    start_with_mcp(&dir, "m", &file, &[], &["--history", history]);
    first_turn(&dir, "m");
    until(&dir, "m", "hibernated");
    ok(&dir, &["send", "m", "again"]);
    until(&dir, "m", "done");
    let read = ok(&dir, &["read", "m"]);
    assert_eq!(read, "> hi\nyou said: hi\n> again\nyou said: again\n");
    let events = events_until(&dir, "mcp", |e| {
        e["event"] == "reply" && e["text"] == "you said: again"
    });
    assert_hidden(&dir, events.join("\n") + &ok(&dir, &["get", "m"]));
}
