use serde_json::{Value, json};

use crate::common::{Server, temp_dir};
use crate::{events_until, fake_agent, is_state, ok, reported, shed, start_fake};

#[test]
fn cancel_ends_a_turn_mid_way() {
    let dir = temp_dir("acp-cancel");
    let _server = Server::start(&dir);
    let agent = fake_agent();
    let agent = agent.to_str().expect("utf-8");
    let run = [
        "run",
        "--name",
        "c",
        "--label",
        "run=cancel",
        "--acp",
        "fake",
        "--",
    ];
    ok(&dir, &[&run[..], &["python3", agent]].concat());
    ok(
        &dir,
        &[
            "wait",
            "c",
            "--output",
            "ACP session ready",
            "--timeout",
            "10",
        ],
    );
    for cancel in [&["cancel", "c"][..], &["keys", "c", "esc"][..]] {
        ok(&dir, &["send", "c", "slow"]);
        let seen = std::cell::Cell::new(0);
        events_until(&dir, "cancel", |e| {
            if e["event"] == "text" {
                seen.set(seen.get() + 1);
            }
            e["event"] == "text" && seen.get() == if cancel[0] == "cancel" { 1 } else { 2 }
        });
        ok(&dir, cancel);
        ok(&dir, &["wait", "c", "--until", "done", "--timeout", "10"]);
        let info: Value = serde_json::from_str(&ok(&dir, &["get", "c"])).expect("JSON");
        assert_eq!(
            info["reported"],
            json!({"state": "done", "note": "cancelled", "from": "reported"})
        );
    }
    assert_eq!(
        ok(&dir, &["read", "c"]),
        "> slow\nthinking\n> slow\nthinking\n"
    );
}

#[test]
fn the_deny_policy_rejects_permission_requests() {
    let dir = temp_dir("acp-deny");
    let _server = Server::start(&dir);
    let agent = fake_agent();
    let agent = agent.to_str().expect("utf-8");
    let args = [
        "run", "--name", "d", "--label", "run=deny", "--acp", "fake", "--policy", "deny", "--",
    ];
    ok(&dir, &[&args[..], &["python3", agent]].concat());
    ok(
        &dir,
        &[
            "wait",
            "d",
            "--output",
            "ACP session ready",
            "--timeout",
            "10",
        ],
    );
    ok(&dir, &["send", "d", "tool"]);
    ok(&dir, &["wait", "d", "--until", "done", "--timeout", "10"]);
    let events = events_until(&dir, "deny", is_state("done"));
    let kinds: Vec<Value> = events
        .iter()
        .map(|l| serde_json::from_str::<Value>(l).expect("JSON"))
        .filter(|e| e["event"] == "approved" || e["event"] == "state")
        .map(|e| json!([e["event"], e.get("option").or(e.get("state"))]))
        .collect();
    assert_eq!(
        kinds,
        [
            json!(["state", "working"]),
            json!(["state", "running"]),
            json!(["approved", "reject-once"]),
            json!(["state", "working"]),
            json!(["state", "done"])
        ]
    );
    assert_eq!(
        ok(&dir, &["read", "d"]),
        "> tool\n[tool] Write notes.txt (failed)\ndenied\n"
    );
}

#[test]
fn two_requests_in_flight_are_each_answered_by_id_and_tool_calls_carry_locations_and_raw_input() {
    let dir = temp_dir("acp-two");
    let _server = Server::start(&dir);
    let agent = fake_agent();
    let agent = agent.to_str().expect("utf-8");
    let args = [
        "run", "--name", "w", "--label", "run=two", "--acp", "fake", "--",
    ];
    ok(&dir, &[&args[..], &["python3", agent]].concat());
    ok(
        &dir,
        &[
            "wait",
            "w",
            "--output",
            "ACP session ready",
            "--timeout",
            "10",
        ],
    );
    ok(&dir, &["send", "w", "pair"]);
    let asked = std::cell::Cell::new(0);
    let events = events_until(&dir, "two", |e| {
        asked.set(asked.get() + usize::from(e["event"] == "approval_request"));
        asked.get() == 2
    });
    let events: Vec<Value> = events
        .iter()
        .map(|l| serde_json::from_str(l).expect("JSON"))
        .collect();
    let of = |kind: &str| -> Vec<&Value> { events.iter().filter(|e| e["event"] == kind).collect() };
    let calls = of("tool_call");
    assert_eq!(calls[0]["locations"], json!(["/work/call-a.rs"]));
    assert_eq!(calls[0].get("raw_input"), None, "over 4 KiB: {}", calls[0]);
    assert_eq!(calls[1]["locations"], json!(["/work/call-b.rs"]));
    assert_eq!(calls[1]["raw_input"], json!({"command": "ls -la"}));
    let asks = of("approval_request");
    let ids: Vec<&Value> = asks.iter().map(|a| &a["id"]).collect();
    assert_eq!(ids, [&json!("101"), &json!("102")]);
    let call = &asks[1]["call"];
    assert_eq!(call["toolCallId"], "call-b");
    assert_eq!(call["rawInput"], json!({"command": "ls -la"}));
    assert_eq!(
        call["locations"],
        json!([{"path": "/work/call-b.rs", "line": 3}])
    );
    assert_eq!(call.get("content"), None, "{call}");
    ok(&dir, &["approve", "w", "reject-once", "--request", "102"]);
    ok(&dir, &["approve", "w", "allow-once", "--request", "101"]);
    ok(&dir, &["wait", "w", "--until", "done", "--timeout", "10"]);
    assert_eq!(
        ok(&dir, &["read", "w"]),
        "> pair\n[tool] Edit a.rs (completed)\n[tool] Run ls (failed)\ncall-a allow-once, call-b reject-once\n"
    );
}

#[test]
fn an_agent_that_fails_the_handshake_reports_error() {
    let dir = temp_dir("acp-error");
    let _server = Server::start(&dir);
    let args = [
        "run", "--name", "e", "--acp", "nope", "--", "sh", "-c", "read x",
    ];
    ok(&dir, &args);
    let out = shed(&dir, &["wait", "e", "--until", "error", "--timeout", "10"]);
    let info: Value = serde_json::from_str(&ok(&dir, &["get", "e"])).expect("JSON");
    assert_eq!(
        info["reported"],
        json!({"state": "error", "note": "acp: the agent closed its stdout", "from": "reported"}),
        "{out:?}"
    );
    let missing = shed(&dir, &["run", "--acp", "nope"]);
    let err = String::from_utf8_lossy(&missing.stderr);
    assert!(err.contains("ACP agent nope is not installed"), "{err}");
}

#[test]
fn the_watchdog_ends_a_silent_turn_but_not_a_busy_one() {
    let dir = temp_dir("acp-idle");
    let env = [("SHED_TURN_IDLE_TIMEOUT", "1")];
    let _server = Server::with_env(&dir, "3600", &env);
    start_fake(&dir, "w", "fake");
    let until = [
        "wait",
        "w",
        "--until",
        "done",
        "waiting-approval",
        "error",
        "--timeout",
        "10",
    ];
    ok(&dir, &["send", "w", "busy"]);
    assert_eq!(ok(&dir, &until), "w done\n");
    assert_eq!(
        reported(&dir, "w"),
        json!({"state": "done", "note": "end_turn", "from": "reported"})
    );
    ok(&dir, &["send", "w", "silent"]);
    assert_eq!(ok(&dir, &until), "w error\n");
    assert_eq!(
        reported(&dir, "w"),
        json!({"state": "error", "note": "no response", "from": "reported"})
    );
    assert_eq!(
        ok(&dir, &["read", "w"]),
        "> busy\n[tool] Build (completed)\nbusy done\n> silent\n"
    );
}

#[test]
fn a_v2_only_agent_is_refused_with_a_clear_message() {
    let dir = temp_dir("acp-v2");
    let _server = Server::start(&dir);
    let agent = fake_agent();
    let run = ["run", "--name", "v", "--acp", "fake", "--", "python3"];
    ok(
        &dir,
        &[&run[..], &[agent.to_str().expect("utf-8"), "--v2"]].concat(),
    );
    ok(&dir, &["wait", "v", "--until", "error", "--timeout", "10"]);
    let note = reported(&dir, "v")["note"].clone();
    let note = note.as_str().unwrap_or_default();
    assert!(
        note.starts_with("acp: ")
            && note.contains("required ACP protocol version 1 but peer negotiated 2"),
        "{note}"
    );
}

#[test]
fn an_unknown_update_kind_is_skipped() {
    let dir = temp_dir("acp-unknown");
    let _server = Server::start(&dir);
    start_fake(&dir, "u", "fake");
    ok(&dir, &["send", "u", "unknown"]);
    ok(&dir, &["wait", "u", "--until", "done", "--timeout", "10"]);
    let logged = [
        "wait",
        "u",
        "--output",
        "skipped a session/update",
        "--timeout",
        "1",
    ];
    assert_eq!(ok(&dir, &logged), "u output\n");
    assert_eq!(ok(&dir, &["read", "u"]), "> unknown\nstill here\n");
    ok(&dir, &["send", "u", "hi"]);
    ok(&dir, &["wait", "u", "--until", "done", "--timeout", "10"]);
    assert_eq!(
        reported(&dir, "u"),
        json!({"state": "done", "note": "end_turn", "from": "reported"})
    );
}

#[test]
fn each_change_of_the_slash_commands_is_one_session_event() {
    let dir = temp_dir("acp-commands");
    let _server = Server::start(&dir);
    start_fake(&dir, "k", "fake");
    ok(&dir, &["send", "k", "commands"]);
    ok(&dir, &["wait", "k", "--until", "done", "--timeout", "10"]);
    let commands: Vec<Value> = events_until(&dir, "k", is_state("done"))
        .iter()
        .map(|l| serde_json::from_str::<Value>(l).expect("JSON"))
        .filter(|e| e["event"] == "session")
        .map(|e| e["commands"].clone())
        .collect();
    let plan = json!(["plan", "Make a plan", ""]);
    let both = json!([plan, ["review", "Review the diff", "files"]]);
    assert_eq!(commands, [Value::Null, json!([plan]), both.clone()]);
    let info: Value = serde_json::from_str(&ok(&dir, &["get", "k"])).expect("JSON");
    assert_eq!(info["session"]["commands"], both);
    assert_eq!(ok(&dir, &["read", "k"]), "> commands\ncommands sent\n");
}
