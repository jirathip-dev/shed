//! Machines over SSH end to end: `shed connect` over a socketpair (simulated SSH), and `--on` through a
//! fake `ssh` that runs `shed connect` against a second server in a temp dir.
//!
//! The CLI output is checked against `tests/golden/machines.txt` (the temp dir and version
//! normalised). Regenerate with `SHED_BLESS=1 cargo test -p shed`.

mod common;

use std::fmt::Write as _;
use std::io::{BufRead as _, BufReader, Write as _};
use std::os::fd::OwnedFd;
use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use common::{Server, temp_dir};
use serde_json::{Value, json};
use shed_server::proto::{EventsParams, Method, Response, RpcError};

const SHED: &str = env!("CARGO_BIN_EXE_shed");
const VERSION: &str = env!("CARGO_PKG_VERSION");

#[test]
fn connect_carries_the_protocol_over_a_socketpair() {
    let dir = temp_dir("connect");
    let _server = Server::start(&dir);
    let (ours, theirs) = UnixStream::pair().expect("socketpair");
    let mut child = Command::new(SHED)
        .arg("--dir")
        .arg(&dir)
        .arg("connect")
        .stdin(Stdio::from(OwnedFd::from(
            theirs.try_clone().expect("clone"),
        )))
        .stdout(Stdio::from(OwnedFd::from(theirs)))
        .spawn()
        .expect("shed connect starts");
    let mut reader = BufReader::new(ours.try_clone().expect("clone"));
    let golden = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/hello.jsonl");
    let golden = std::fs::read_to_string(golden).expect("hello golden");
    let exchange: Vec<String> = golden
        .lines()
        .map(|l| l.replace("$VERSION", VERSION))
        .collect();
    let mut call = |request: &str| {
        (&ours)
            .write_all(format!("{request}\n").as_bytes())
            .expect("write");
        let mut line = String::new();
        reader.read_line(&mut line).expect("read");
        line.trim_end().to_owned()
    };
    assert_eq!(call(&exchange[0]), exchange[1]);
    assert_eq!(
        call(r#"{"jsonrpc":"2.0","id":2,"method":"ls","params":{}}"#),
        r#"{"jsonrpc":"2.0","id":2,"result":[]}"#
    );
    ours.shutdown(std::net::Shutdown::Write).expect("shutdown");
    assert!(child.wait().expect("shed connect ends").success());
}

fn script(path: &Path, body: &str) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
    std::fs::write(path, format!("#!/bin/sh\n{body}")).expect("write script");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
}

fn fake_hosts(root: &Path) -> PathBuf {
    let bin = root.join("bin");
    script(
        &bin.join("ssh"),
        "# A fake ssh: logs its arguments, skips the options, runs the command on the target's PATH.\n\
         echo \"$*\" >> \"$(dirname \"$0\")/ssh.log\"\n\
         echo $$ >> \"$(dirname \"$0\")/ssh.pids\"\n\
         while [ \"$1\" = -o ]; do shift 2; done\n\
         [ \"$1\" = -- ] && shift\n\
         target=$1\nshift\n\
         PATH=\"$(dirname \"$0\")/../$target:$PATH\" exec \"$@\"\n",
    );
    let remote = root.join("remote");
    script(
        &root.join("user@box.example/shed"),
        &format!("exec '{SHED}' --dir '{}' \"$@\"\n", remote.display()),
    );
    let refusal = Response::err(1.into(), RpcError::version(VERSION, "9.9.9"));
    let refusal = serde_json::to_string(&refusal).expect("json");
    script(
        &root.join("old.example/shed"),
        &format!("read -r line\nprintf '%s\\n' '{refusal}'\n"),
    );
    bin
}

fn shed_on(root: &Path, args: &[&str]) -> Command {
    let path = std::env::var("PATH").unwrap_or_default();
    let mut cmd = Command::new(SHED);
    cmd.arg("--dir")
        .arg(root.join("local"))
        .args(args)
        .env("PATH", format!("{}:{path}", root.join("bin").display()));
    cmd
}

fn record(root: &Path, text: &mut String, args: &[&str]) -> Output {
    let out = shed_on(root, args).output().expect("shed runs");
    let _ = write!(
        text,
        "$ shed {}\n{}",
        args.join(" "),
        String::from_utf8_lossy(&out.stdout)
    );
    if !out.status.success() {
        let code = out.status.code().unwrap_or(-1);
        let _ = write!(
            text,
            "[exit {code}] {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    out
}

#[test]
fn on_a_machine_run_read_kill_and_ls_reach_the_remote_server_and_versions_must_match() {
    let root = temp_dir("machines");
    let bin = fake_hosts(&root);
    let remote = root.join("remote");
    let _server = Server::start(&remote);
    let remote_arg = remote.to_str().expect("utf-8 dir");
    let mut text = String::new();
    for args in [
        &["machine", "ls"][..],
        &["machine", "add", "box", "user@box.example"],
        &["machine", "add", "old", "old.example"],
        &["machine", "add", "box", "other.example"],
        &["machine", "add", "bad", "--", "-oProxyCommand=x"],
        &["--json", "machine", "ls"],
        &["machine", "ls"],
        &["--on", "nope", "ls"],
        &["--on", "box", "serve"],
        &["--on", "box", "run", "--", "true"],
        &[
            "--on",
            "box",
            "run",
            "--name",
            "t1",
            "--cwd",
            remote_arg,
            "--",
            "sh",
            "-c",
            "echo hello from box; exec sleep 100",
        ],
        &["--on", "box", "wait", "t1", "--output", "hello from box"],
        &["--on", "box", "read", "t1"],
        &["--on", "box", "kill", "t1"],
        &["--on", "box", "ls"],
        &["--on", "old", "ls"],
        &["machine", "rm", "old"],
        &["machine", "rm", "old"],
        &["machine", "ls"],
    ] {
        record(&root, &mut text, args);
    }
    let direct = Command::new(SHED)
        .arg("--dir")
        .arg(&remote)
        .arg("ls")
        .output()
        .expect("shed ls");
    assert!(String::from_utf8_lossy(&direct.stdout).contains("t1"));
    let log = std::fs::read_to_string(bin.join("ssh.log")).expect("ssh ran");
    for option in [
        "-o BatchMode=yes",
        "-o ConnectTimeout=10",
        "-o ControlMaster=auto",
        "-- user@box.example shed connect",
    ] {
        assert!(log.contains(option), "{option} missing from {log}");
    }
    let text = text
        .replace(root.to_str().expect("utf-8 dir"), "$DIR")
        .replace(VERSION, "$VERSION");
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/machines.txt");
    if std::env::var_os("SHED_BLESS").is_some() {
        std::fs::write(&path, &text).expect("write golden");
    }
    let have = std::fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(have, text, "machines.txt differs; SHED_BLESS=1 to update");
}

fn linked(root: &Path) -> (Server, Server) {
    fake_hosts(root);
    let remote = Server::start(&root.join("remote"));
    for (name, target) in [("box", "user@box.example"), ("old", "old.example")] {
        let added = shed_on(root, &["machine", "add", name, target]).output();
        assert!(added.expect("machine add").status.success());
    }
    let path = format!(
        "{}:{}",
        root.join("bin").display(),
        std::env::var("PATH").unwrap()
    );
    let local = Server::with_env(&root.join("local"), "3600", &[("PATH", &path)]);
    common::eventually("the link to box", || link(root, "box")["state"] == "up");
    (remote, local)
}

fn link(root: &Path, name: &str) -> Value {
    let out = shed_on(root, &["--json", "machine", "ls"])
        .output()
        .expect("ls");
    let list: Value = serde_json::from_slice(&out.stdout).expect("JSON");
    let list = list.as_array().expect("a list").iter();
    list.into_iter()
        .find(|m| m["name"] == name)
        .map_or(Value::Null, |m| m["link"].clone())
}

fn ok(root: &Path, args: &[&str]) -> String {
    let out = shed_on(root, args).output().expect("shed runs");
    assert!(out.status.success(), "shed {args:?}: {out:?}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn box_ssh(root: &Path) -> usize {
    let log = std::fs::read_to_string(root.join("bin/ssh.log")).unwrap_or_default();
    log.lines()
        .filter(|l| l.contains("user@box.example"))
        .count()
}

fn local_events(root: &Path, since: u64) -> impl Iterator<Item = Value> {
    let socket = shed_server::Paths::in_dir(&root.join("local")).socket;
    let mut client = shed_server::Client::connect(&socket).expect("connect");
    let params = EventsParams {
        since: Some(since),
        ..EventsParams::default()
    };
    let _: Value = client.call(Method::Events, &params).expect("events");
    let (reader, _writer) = client.split();
    let timeout = Some(Duration::from_secs(20));
    reader.get_ref().set_read_timeout(timeout).expect("timeout");
    reader
        .lines()
        .map(|l| serde_json::from_str(&l.expect("an event line")).expect("event JSON"))
}

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
        error.contains(&format!("shed {VERSION} here, 9.9.9 on old")),
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
    let mut client = shed_server::Client::connect(&socket).expect("connect");
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
        stderr.contains(&format!("SHED_VERSION: shed {VERSION} here, 9.9.9 on old")),
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
        let _ = Command::new("kill").arg(pid).stderr(Stdio::null()).status();
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
    assert_eq!(seqs, (1..=seqs.len() as u64).collect::<Vec<_>>());
    let ours = |e: &Value| {
        let name = e["name"].as_str().unwrap().strip_prefix("box/")?;
        Some((name.to_owned(), e["event"].clone(), e["state"].clone()))
    };
    let merged: Vec<_> = seen.iter().filter_map(ours).collect();
    let there: Vec<_> = {
        let socket = shed_server::Paths::in_dir(&remote).socket;
        let mut client = shed_server::Client::connect(&socket).expect("connect");
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
        json!({"state": "done", "note": "cancelled"})
    );
    assert_eq!(
        box_ssh(&root),
        ssh + 3,
        "an ssh per wait only, over the shared connection"
    );
}

#[test]
fn on_a_machine_ssh_cannot_reach_fails_fast_with_the_hint() {
    let root = temp_dir("machine-down");
    script(
        &root.join("bin/ssh"),
        "echo 'ssh: connect to host down.example: Connection refused' >&2\nexit 255\n",
    );
    assert!(
        shed_on(&root, &["machine", "add", "down", "down.example"])
            .status()
            .expect("shed runs")
            .success()
    );
    let mut child = shed_on(&root, &["--on", "down", "ls"])
        .stderr(Stdio::piped())
        .spawn()
        .expect("shed runs");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while child.try_wait().expect("try_wait").is_none() {
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            panic!("shed --on hung after ssh exited");
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let out = child.wait_with_output().expect("output");
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("no answer from shed on down")
            && stderr.contains("`ssh down.example shed hello`"),
        "{stderr}"
    );
}

#[test]
fn machine_stats_here_and_on_a_machine_and_an_offline_one_answers_within_5_s() {
    let root = temp_dir("machine-stats");
    let bin = fake_hosts(&root);
    let ssh = std::fs::read_to_string(bin.join("ssh")).expect("ssh");
    let hang = "[ -e \"$(dirname \"$0\")/down\" ] && exec sleep 30\n";
    script(&bin.join("ssh"), &ssh.replacen("#!/bin/sh\n", hang, 1));
    let _local = Server::start(&root.join("local"));
    let _remote = Server::start(&root.join("remote"));
    let shed = |args: &[&str]| {
        let out = shed_on(&root, args)
            .stderr(Stdio::null())
            .output()
            .expect("shed runs");
        let json = serde_json::from_slice(&out.stdout).unwrap_or(Value::Null);
        (out.status.code(), json)
    };
    assert_eq!(
        shed(&["run", "--name", "a", "--", "sleep", "30"]).0,
        Some(0)
    );
    assert_eq!(shed(&["state", "a", "working"]).0, Some(0));
    assert_eq!(shed(&["run", "--name", "b", "--", "true"]).0, Some(0));
    common::eventually("b exits", || {
        shed(&["--json", "get", "b"]).1["state"] == "exited"
    });
    let check = |s: &Value| {
        assert!(s["at"].as_u64().expect("at") > 1_700_000_000, "{s}");
        assert!(s["load_per_cpu"].as_f64().expect("load") >= 0.0, "{s}");
        assert!(
            s["mem_pressure_pct"].as_u64().expect("memory") <= 100,
            "{s}"
        );
        assert!(s["swap_used_mb"].is_u64(), "{s}");
        assert!(s["disk_free_mb"].as_u64().expect("disk") > 0, "{s}");
        assert!(s["cpus"].as_u64().expect("cpus") >= 1, "{s}");
        assert!(s["uptime_s"].as_u64().expect("uptime") > 0, "{s}");
    };
    let (code, here) = shed(&["--json", "machine", "stats", "--since", "0"]);
    assert_eq!(code, Some(0));
    check(&here);
    assert_eq!(here["terminals"], json!({"working": 1, "exited": 1}));
    let history = here["history"].as_array().expect("history");
    assert_eq!(history.len(), 1, "{here}");
    check(&history[0]);
    assert_eq!(history[0]["terminals"], json!({}));
    let (code, without) = shed(&["--json", "machine", "stats"]);
    assert_eq!(code, Some(0));
    assert!(without.get("history").is_none(), "{without}");

    assert_eq!(
        shed(&["machine", "add", "box", "user@box.example"]).0,
        Some(0)
    );
    let (code, there) = shed(&["--json", "--on", "box", "machine", "stats"]);
    assert_eq!(code, Some(0));
    check(&there);
    assert_eq!(there["terminals"], json!({}));
    let seen = std::fs::read_to_string(root.join("local/seen/box")).expect("seen");

    std::fs::write(bin.join("down"), "").expect("down");
    let started = Instant::now();
    let (code, offline) = shed(&["--json", "--on", "box", "machine", "stats"]);
    let took = started.elapsed();
    assert_eq!(code, Some(1));
    let last_seen: u64 = seen.parse().expect("unix seconds");
    assert_eq!(
        offline,
        json!({"machine": "box", "online": false, "last_seen": last_seen})
    );
    assert!(
        took < Duration::from_millis(5_500),
        "answered after {took:?}"
    );
    let text = shed_on(&root, &["--on", "box", "machine", "stats"])
        .stderr(Stdio::null())
        .output()
        .expect("shed runs");
    assert_eq!(
        String::from_utf8_lossy(&text.stdout),
        format!("box offline, last seen {last_seen}\n")
    );
}
