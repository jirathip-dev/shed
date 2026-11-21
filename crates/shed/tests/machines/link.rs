use std::io::BufRead as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use shed_server::proto::{BUILD, EventsParams, Method};

use crate::common::{self, temp_dir};
use crate::{
    OLD, SHED, VERSION, box_ssh, fake_hosts, link, linked, local_events, ok, script, shed_on,
};

#[test]
fn over_a_held_link_remote_terminals_are_listed_and_controlled_and_another_version_is_refused() {
    let root = temp_dir("link");
    let (_remote, _local) = linked(&root);
    let remote = root.join("remote");
    let remote_arg = remote.to_str().expect("utf-8 dir");
    let refused = link(&root, "old");
    assert_eq!(refused["state"], "down", "{refused}");
    let error = refused["error"].as_str().expect("why it is down");
    assert!(
        error.contains(&format!("shed {BUILD} here, {OLD} on old")),
        "{error}"
    );
    let ssh = box_ssh(&root);

    let cmd = "echo hello from box; exec cat";
    let ran = ok(
        &root,
        &[
            "--on", "box", "run", "--name", "t1", "--cwd", remote_arg, "--", "sh", "-c", cmd,
        ],
    );
    assert_eq!(ran, "box/t1\n");
    ok(&root, &["run", "--name", "here", "--", "sleep", "100"]);
    let names = |args: &[&str]| -> Vec<String> {
        let list: Value = serde_json::from_str(&ok(&root, args)).expect("JSON");
        let list = list.as_array().expect("a list").iter();
        list.map(|t| t["name"].as_str().expect("name").to_owned())
            .collect()
    };
    assert_eq!(names(&["--json", "ls"]), ["here", "box/t1"]);
    assert_eq!(names(&["--json", "--on", "box", "ls"]), ["box/t1"]);
    common::eventually("t1's output", || {
        ok(&root, &["read", "box/t1"]).contains("hello from box")
    });
    ok(&root, &["send", "box/t1", "typed over the link"]);
    ok(&root, &["--on", "box", "keys", "t1", "enter"]);
    common::eventually("the typed line", || {
        ok(&root, &["--on", "box", "read", "t1"])
            .matches("typed over the link")
            .count()
            == 2
    });
    let got: Value = serde_json::from_str(&ok(&root, &["get", "box/t1"])).expect("JSON");
    assert_eq!(
        (&got["name"], &got["state"]),
        (&json!("box/t1"), &json!("running"))
    );

    let socket = shed_server::Paths::in_dir(&root.join("local")).socket;
    let mut client = shed_client::Conn::connect(&socket).expect("connect");
    let read = json!({"name": "box/t1"});
    let mut took: Vec<Duration> = (0..50)
        .map(|_| {
            let started = Instant::now();
            let _: Value = client.call(Method::Read, &read).expect("read");
            started.elapsed()
        })
        .collect();
    took.sort();
    eprintln!(
        "remote read over the held link: p50 {:?}, p90 {:?}",
        took[25], took[45]
    );

    assert_eq!(ok(&root, &["kill", "box/t1"]), "box/t1 exited 143\n");
    assert_eq!(box_ssh(&root), ssh, "no ssh per call while the link is up");

    let out = shed_on(&root, &["get", "old/t1"])
        .output()
        .expect("shed runs");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success());
    assert!(
        stderr.contains(&format!("SHED_VERSION: shed {BUILD} here, {OLD} on old")),
        "{stderr}"
    );
    let out = shed_on(&root, &["get", "nope/t1"])
        .output()
        .expect("shed runs");
    assert!(String::from_utf8_lossy(&out.stderr).contains("NO_LINK: nope"));
}

#[test]
fn a_dropped_link_reconnects_and_the_merged_stream_loses_no_events() {
    let root = temp_dir("link-drop");
    let (_remote, _local) = linked(&root);
    let remote = root.join("remote");
    let on_remote = |args: &[&str]| {
        let out = Command::new(SHED)
            .arg("--dir")
            .arg(&remote)
            .args(args)
            .output();
        assert!(out.expect("shed runs").status.success(), "{args:?}");
    };
    let mut events = local_events(&root, 0);
    ok(&root, &["run", "--name", "l1", "--", "sleep", "100"]);
    on_remote(&["run", "--name", "r1", "--", "sleep", "100"]);
    let mut seen = Vec::new();
    let mut until = |name: &str, event: &str, seen: &mut Vec<Value>| loop {
        let e = events.next().expect("an event");
        seen.push(e.clone());
        if e["name"] == name && e["event"] == event {
            return e;
        }
    };
    until("l1", "started", &mut seen);
    let before = until("box/r1", "started", &mut seen)["seq"].clone();

    let pids = std::fs::read_to_string(root.join("bin/ssh.pids")).expect("pids");
    for pid in pids.lines() {
        Command::new("kill")
            .arg(pid)
            .stderr(Stdio::null())
            .status()
            .ok();
    }
    common::eventually("reconnecting", || {
        link(&root, "box")["state"] == "reconnecting"
    });
    let dropped = Instant::now();
    on_remote(&["run", "--name", "r2", "--", "sleep", "100"]);
    on_remote(&["state", "r1", "working"]);
    on_remote(&["kill", "r2"]);
    until("box/r2", "killed", &mut seen);
    let waited = dropped.elapsed();
    assert!(waited < Duration::from_secs(10), "{waited:?}");
    assert_eq!(link(&root, "box")["state"], "up");

    let seqs: Vec<u64> = seen.iter().map(|e| e["seq"].as_u64().unwrap()).collect();
    assert_eq!(
        seqs,
        (1..=u64::try_from(seqs.len()).unwrap()).collect::<Vec<_>>()
    );
    let ours = |e: &Value| {
        let name = e["name"].as_str().unwrap().strip_prefix("box/")?;
        Some((name.to_owned(), e["event"].clone(), e["state"].clone()))
    };
    let merged: Vec<_> = seen.iter().filter_map(ours).collect();
    let there: Vec<_> = {
        let socket = shed_server::Paths::in_dir(&remote).socket;
        let mut client = shed_client::Conn::connect(&socket).expect("connect");
        let params = EventsParams {
            since: Some(0),
            ..EventsParams::default()
        };
        let _: Value = client.call(Method::Events, &params).expect("events");
        let (reader, _writer) = client.split();
        let lines = reader.lines().take(merged.len());
        lines
            .map(|l| {
                let e: Value = serde_json::from_str(&l.unwrap()).unwrap();
                (
                    e["name"].as_str().unwrap().to_owned(),
                    e["event"].clone(),
                    e["state"].clone(),
                )
            })
            .collect()
    };
    assert_eq!(merged, there);
    let kinds: Vec<_> = merged
        .iter()
        .map(|(n, e, _)| format!("{n} {}", e.as_str().unwrap()))
        .collect();
    assert_eq!(kinds, ["r1 started", "r2 started", "r1 state", "r2 killed"]);

    let mut resumed = local_events(&root, before.as_u64().unwrap());
    let next = resumed.next().expect("an event");
    assert_eq!(
        (&next["name"], &next["event"]),
        (&json!("box/r2"), &json!("started"))
    );
}

#[test]
fn events_on_a_machine_pass_reset_and_lagged_notices_through() {
    let root = temp_dir("notices");
    fake_hosts(&root);
    let hello = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/hello.jsonl"),
    )
    .expect("hello golden");
    let hello = hello
        .lines()
        .nth(1)
        .expect("reply")
        .replace("$VERSION", VERSION);
    let reset = r#"{"jsonrpc":"2.0","method":"reset","params":{"seq":7}}"#;
    let event = r#"{"seq":8,"name":"t1","labels":{},"event":"started"}"#;
    let lagged = r#"{"jsonrpc":"2.0","method":"lagged","params":{"seq":8}}"#;
    script(
        &root.join("lag.example/shed"),
        &format!(
            "read -r line\nprintf '%s\\n' '{hello}'\nread -r line\n\
             id=$(printf '%s' \"$line\" | sed 's/.*\"id\":\\([0-9]*\\).*/\\1/')\n\
             printf '{{\"jsonrpc\":\"2.0\",\"id\":%s,\"result\":{{}}}}\\n' \"$id\"\n\
             printf '%s\\n%s\\n\\n%s\\n' '{reset}' '{event}' '{lagged}'\n"
        ),
    );
    ok(&root, &["machine", "add", "lag", "lag.example"]);
    let out = ok(&root, &["--on", "lag", "events", "--since", "1"]);
    let renamed = event.replace("\"t1\"", "\"lag/t1\"");
    assert_eq!(out, format!("{reset}\n{renamed}\n\n{lagged}\n"));
}
