use std::fmt::Write as _;
use std::path::Path;

use serde_json::{Value, json};

use crate::common::{Server, temp_dir};
use crate::{events_until, fake_agent, is_state, ok, reported, shed, start_with_history, stop};

#[test]
fn resume_replays_the_history_once_marked_replay() {
    let dir = temp_dir("acp-resume");
    let history = dir.join("history.jsonl");
    let mut server = Server::start(&dir);
    start_with_history(&dir, "r1", &history, &[]);
    ok(
        &dir,
        &[
            "wait",
            "r1",
            "--output",
            "ACP session ready",
            "--timeout",
            "10",
        ],
    );
    for prompt in ["hi", "again"] {
        ok(&dir, &["send", "r1", prompt]);
        ok(&dir, &["wait", "r1", "--until", "done", "--timeout", "10"]);
    }
    stop(&mut server);
    let _server = Server::start(&dir);
    let info: Value = serde_json::from_str(&ok(&dir, &["get", "r1"])).expect("JSON");
    assert_eq!(
        (&info["state"], &info["session"]["id"]),
        (&json!("running"), &json!("sess-1"))
    );
    ok(&dir, &["kill", "r1"]);
    let agent_only = json!({"sessionUpdate": "user_message_chunk",
        "content": {"type": "text", "text": "only the agent kept this"}});
    let mut kept = std::fs::read_to_string(&history).expect("the agent's history");
    writeln!(kept, "{agent_only}").expect("append");
    std::fs::write(&history, kept).expect("write the agent's history");

    start_with_history(&dir, "r2", &history, &["--resume", "sess-1"]);
    ok(
        &dir,
        &[
            "wait",
            "r2",
            "--output",
            "ACP session ready",
            "--timeout",
            "10",
        ],
    );
    ok(&dir, &["send", "r2", "more"]);
    ok(&dir, &["wait", "r2", "--until", "done", "--timeout", "10"]);
    let events: Vec<Value> = events_until(&dir, "r2", is_state("done"))
        .iter()
        .map(|l| serde_json::from_str(l).expect("JSON"))
        .collect();
    let acp: Vec<Value> = events
        .iter()
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
            json!(["turn", "hi", true]),
            json!(["reply", "you said: hi", true]),
            json!(["turn", "again", true]),
            json!(["reply", "you said: again", true]),
            json!(["turn", "more", null]),
            json!(["reply", "you said: more", null]),
        ]
    );
    assert_eq!(
        ok(&dir, &["read", "r2"]),
        "> hi\nyou said: hi\n> again\nyou said: again\n> more\nyou said: more\n"
    );
}

#[test]
fn resume_is_refused_without_load_session() {
    let dir = temp_dir("acp-noload");
    let _server = Server::start(&dir);
    let agent = fake_agent();
    let agent = agent.to_str().expect("utf-8");
    let run = [
        "run", "--name", "n", "--acp", "fake", "--resume", "sess-1", "--",
    ];
    ok(&dir, &[&run[..], &["python3", agent]].concat());
    ok(&dir, &["wait", "n", "--until", "error", "--timeout", "10"]);
    assert_eq!(
        reported(&dir, "n"),
        json!({"state": "error", "note": "acp: the agent cannot resume a session (no loadSession)", "from": "reported"})
    );
}

#[test]
fn agents_sessions_lists_shed_s_own_sessions_and_one_shed_never_ran_resumes_without_history() {
    let dir = temp_dir("acp-sessions");
    let _server = Server::start(&dir);
    assert_eq!(
        ok(&dir, &["agents", "sessions", "fake"]),
        "",
        "no agent installed or started"
    );
    let agent = fake_agent();
    let fake = ["python3", agent.to_str().expect("utf-8"), "--sessions"];
    let run = [
        "run", "--name", "s2", "--acp", "fake", "--resume", "sess-2", "--",
    ];
    ok(&dir, &[&run[..], &fake].concat());
    let ready = [
        "wait",
        "s2",
        "--output",
        "ACP session ready",
        "--timeout",
        "10",
    ];
    ok(&dir, &ready);
    assert_eq!(ok(&dir, &["read", "s2"]), "", "no earlier history");
    let listed = |dir: &Path| -> Value {
        serde_json::from_str(&ok(dir, &["--json", "agents", "sessions", "fake"])).expect("JSON")
    };
    assert_eq!(
        (&listed(&dir)[0]["id"], &listed(&dir)[0]["turns"]),
        (&json!("sess-2"), &json!(0)),
        "listed from its start"
    );
    ok(&dir, &["send", "s2", "hi"]);
    ok(&dir, &["wait", "s2", "--until", "done", "--timeout", "10"]);
    let info: Value = serde_json::from_str(&ok(&dir, &["get", "s2"])).expect("JSON");
    assert_eq!(info["session"]["id"], "sess-2");
    assert_eq!(ok(&dir, &["read", "s2"]), "> hi\nyou said: hi\n");
    ok(&dir, &["kill", "s2"]);
    let listed = listed(&dir);
    let record = &listed[0];
    let transcript = dir.join("sessions/sess-2.jsonl");
    assert_eq!(
        (
            &record["agent"],
            &record["title"],
            &record["turns"],
            &record["cwd"]
        ),
        (
            &json!("fake"),
            &json!("hi"),
            &json!(1),
            &json!(std::env::current_dir().expect("cwd"))
        )
    );
    assert_eq!(record["transcript"], json!(transcript));
    let updated = record["updated"].as_u64().expect("updated");
    let cwd = std::env::current_dir().expect("cwd");
    assert_eq!(
        ok(&dir, &["agents", "sessions", "fake"]),
        format!("sess-2  {updated}  {}  hi\n", cwd.display())
    );
    assert_eq!(ok(&dir, &["agents", "sessions", "other"]), "");
}

#[test]
fn rm_closes_the_session_once_and_only_when_the_agent_can() {
    let dir = temp_dir("acp-close");
    let _server = Server::start(&dir);
    let agent = fake_agent();
    let agent = agent.to_str().expect("utf-8");
    for (name, caps, want) in [
        ("c", &["--sessions"][..], Some("sess-1\n")),
        ("n", &[], None),
    ] {
        let closed = dir.join(format!("{name}.closed"));
        let fake = [
            "python3",
            agent,
            "--closed",
            closed.to_str().expect("utf-8"),
        ];
        let run = ["run", "--name", name, "--acp", "fake", "--"];
        ok(&dir, &[&run[..], &fake, caps].concat());
        ok(
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
        ok(&dir, &["send", name, "hi"]);
        ok(&dir, &["wait", name, "--until", "done", "--timeout", "10"]);
        ok(&dir, &["kill", name]);
        assert!(!closed.exists(), "{name}: closed before rm");
        let files = ["sessions/sess-1.json", "sessions/sess-1.jsonl"].map(|f| dir.join(f));
        assert!(
            files.iter().all(|f| f.exists()),
            "{name}: the session's files"
        );
        assert_eq!(ok(&dir, &["rm", name]), format!("{name} removed\n"));
        assert!(!files.iter().any(|f| f.exists()), "{name}: rm deletes them");
        assert_eq!(
            std::fs::read_to_string(&closed).ok().as_deref(),
            want,
            "{name}"
        );
        assert!(!shed(&dir, &["rm", name]).status.success());
        assert_eq!(
            std::fs::read_to_string(&closed).ok().as_deref(),
            want,
            "{name}"
        );
    }
}
