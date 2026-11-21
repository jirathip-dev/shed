use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::common::temp_dir;
use crate::{box_ssh, linked, local_events, ok, shed_on};

fn install_fake(remote: &Path) -> PathBuf {
    let agent = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake_acp.py");
    let installed = json!({"agents": [{"id": "fake", "version": "0.1.0", "distribution": "binary",
        "cmd": ["python3", agent]}]});
    std::fs::write(remote.join("agents.json"), installed.to_string()).expect("agents.json");
    agent
}

#[test]
fn an_acp_agent_not_installed_on_the_machine_is_refused_with_the_machines_agents() {
    let root = temp_dir("link-acp-unknown");
    let (_remote, _local) = linked(&root);
    let remote = root.join("remote");
    install_fake(&remote);
    let cwd = remote.to_str().expect("utf-8 dir");
    let run = ["--on", "box", "run", "--acp", "nope", "--cwd", cwd];
    let unknown = shed_on(&root, &run).output().expect("shed runs");
    let err = String::from_utf8_lossy(&unknown.stderr);
    assert!(!unknown.status.success());
    assert!(
        err.contains("ACP agent nope is not installed; installed: fake"),
        "{err}"
    );
}

#[test]
fn an_acp_session_is_started_waited_on_replied_approved_and_cancelled_on_a_machine_over_the_held_link()
 {
    let root = temp_dir("link-acp");
    let (_remote, _local) = linked(&root);
    let remote = root.join("remote");
    let agent = install_fake(&remote);
    let cwd = remote.to_str().expect("utf-8 dir");
    let ssh = box_ssh(&root);

    let run = [
        "--on", "box", "run", "--name", "a", "--acp", "fake", "--cwd", cwd,
    ];
    assert_eq!(ok(&root, &run), "box/a\n");
    let got: Value = serde_json::from_str(&ok(&root, &["get", "box/a"])).expect("JSON");
    let cmd: Vec<&str> = got["cmd"]
        .as_array()
        .expect("cmd")
        .iter()
        .map(|a| a.as_str().expect("str"))
        .collect();
    let (here, there) = (root.join("local"), remote.display().to_string());
    assert_eq!(
        cmd[1..6],
        [
            "acp-bridge",
            "--socket",
            &format!("{there}/shed.sock"),
            "--state",
            &format!("{there}/state.json")
        ]
    );
    assert!(
        !cmd.iter().any(|a| a.contains(&*here.to_string_lossy())),
        "{cmd:?}"
    );
    assert_eq!(
        &cmd[cmd.len() - 2..],
        ["python3", agent.to_str().expect("utf-8")]
    );

    let wait = |args: &[&str]| {
        shed_on(&root, &[&["wait"], args].concat())
            .output()
            .expect("wait")
    };
    let ready = wait(&["box/a", "--output", "ACP session ready", "--timeout", "10"]);
    assert_eq!(
        String::from_utf8_lossy(&ready.stdout),
        "box/a output\n",
        "{ready:?}"
    );
    assert_eq!(ready.status.code(), Some(0));
    let timeout = wait(&["box/a", "--until", "working", "--timeout", "0.2"]);
    assert_eq!(String::from_utf8_lossy(&timeout.stdout), "box/a timeout\n");
    assert_eq!(timeout.status.code(), Some(124));

    let mut events = local_events(&root, 0);
    let mut until = |what: &dyn Fn(&Value) -> bool| loop {
        let e = events.next().expect("an event");
        if e["name"] == "box/a" && what(&e) {
            return e;
        }
    };
    ok(&root, &["send", "box/a", "hi"]);
    let reply = until(&|e| e["event"] == "reply");
    assert_eq!(reply["text"], "you said: hi", "{reply}");
    let waited = wait(&["--json", "box/a", "--until", "done", "--timeout", "10"]);
    let waited: Value = serde_json::from_slice(&waited.stdout).expect("wait JSON");
    assert_eq!(
        (&waited["outcome"], &waited["terminal"]["name"]),
        (&json!("matched"), &json!("box/a"))
    );

    ok(&root, &["send", "box/a", "tool"]);
    until(&|e| e["event"] == "approval_request");
    ok(&root, &["approve", "box/a", "allow-once"]);
    let approved = until(&|e| e["event"] == "approved");
    assert_eq!(approved["option"], "allow-once", "{approved}");
    until(&|e| e["state"] == "done");

    ok(&root, &["--on", "box", "send", "a", "slow"]);
    until(&|e| e["event"] == "text" && e["text"] == "thinking");
    ok(&root, &["--on", "box", "cancel", "a"]);
    let done = until(&|e| e["state"] == "done");
    assert_eq!(done["note"], "cancelled", "{done}");
    let got: Value = serde_json::from_str(&ok(&root, &["get", "box/a"])).expect("JSON");
    assert_eq!(
        got["reported"],
        json!({"state": "done", "note": "cancelled", "from": "reported"})
    );
    assert_eq!(
        box_ssh(&root),
        ssh + 3,
        "an ssh per wait only, over the shared connection"
    );
}
