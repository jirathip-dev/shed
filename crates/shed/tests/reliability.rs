//! herdsman's fixed bug classes that apply to shed (herdsman#553, "Reliability"), one test per row.
#![cfg(test)]

mod common;

use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use common::{Server, eventually, temp_dir};
use serde_json::{Value, json};
use shed_server::Paths;

const SHED: &str = env!("CARGO_BIN_EXE_shed");

fn shed_in(dir: &Path, args: &[&str]) -> Output {
    Command::new(SHED)
        .arg("--dir")
        .arg(dir)
        .args(args)
        .output()
        .expect("shed runs")
}

fn ok(dir: &Path, args: &[&str]) -> String {
    let out = shed_in(dir, args);
    assert!(out.status.success(), "shed {args:?} failed: {out:?}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn run_sh(dir: &Path, name: &str, script: &str) {
    let d = dir.to_str().expect("utf-8 dir");
    ok(
        dir,
        &["run", "--name", name, "--cwd", d, "--", "sh", "-c", script],
    );
}

fn alive(pid: &str) -> bool {
    Command::new("kill")
        .args(["-0", pid])
        .stderr(Stdio::null())
        .status()
        .expect("kill -0")
        .success()
}

fn pids(dir: &Path, name: &str) -> (String, String) {
    let mut child = String::new();
    eventually("the child's pid", || {
        let screen = ok(dir, &["read", name]);
        screen
            .lines()
            .find_map(|l| l.strip_prefix("child="))
            .unwrap_or_default()
            .clone_into(&mut child);
        !child.is_empty()
    });
    let info: Value = serde_json::from_str(&ok(dir, &["--json", "get", name])).expect("json");
    (info["pid"].to_string(), child)
}

#[test]
fn kill_ends_only_its_own_tree_and_the_other_terminals_survive() {
    let dir = temp_dir("rel-kill");
    let _server = Server::start(&dir);
    for name in ["a", "b", "c"] {
        run_sh(&dir, name, "sleep 100 & echo child=$!; wait");
    }
    let [a, b, c] = ["a", "b", "c"].map(|n| pids(&dir, n));
    ok(&dir, &["kill", "b"]);
    eventually("b's tree gone", || !alive(&b.0) && !alive(&b.1));
    for (name, (pid, child)) in [("a", &a), ("c", &c)] {
        assert!(alive(pid) && alive(child), "{name} survives");
        let info: Value = serde_json::from_str(&ok(&dir, &["--json", "get", name])).expect("json");
        assert_eq!(info["state"], "running", "{name}");
    }
}

#[test]
fn a_terminal_replaced_under_the_same_name_gets_the_messages() {
    let dir = temp_dir("rel-replace");
    let _server = Server::start(&dir);
    run_sh(&dir, "lane", "while read l; do echo \"old:$l\"; done");
    ok(&dir, &["kill", "lane"]);
    ok(&dir, &["rm", "lane"]);
    run_sh(&dir, "lane", "while read l; do echo \"new:$l\"; done");
    ok(&dir, &["send", "lane", "hi\n"]);
    eventually("the new program's answer", || {
        ok(&dir, &["read", "lane"]).contains("new:hi")
    });
    assert!(!ok(&dir, &["read", "lane"]).contains("old:"));

    let socket = Paths::in_dir(&dir).socket;
    let mut events = UnixStream::connect(&socket).expect("connect");
    let hello = json!({"jsonrpc": "2.0", "id": 1, "method": "hello",
        "params": {"product": "shed", "version": env!("CARGO_PKG_VERSION")}});
    let sub = json!({"jsonrpc": "2.0", "id": 2, "method": "events", "params": {"since": 0}});
    writeln!(events, "{hello}\n{sub}").expect("write");
    events
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("timeout");
    let kinds: Vec<String> = BufReader::new(events)
        .lines()
        .skip(2)
        .take(3)
        .map(|l| {
            let v: Value = serde_json::from_str(&l.expect("an event")).expect("json");
            format!("{} {}", v["event"].as_str().unwrap_or_default(), v["name"])
        })
        .collect();
    assert_eq!(
        kinds,
        ["started \"lane\"", "killed \"lane\"", "started \"lane\""]
    );
}

#[test]
fn a_second_server_is_refused_and_a_dead_ones_lock_is_taken_over() {
    let dir = temp_dir("rel-lock");
    let paths = Paths::in_dir(&dir);
    let first = Server::start(&dir);
    std::fs::remove_file(&paths.socket).expect("remove the socket");
    let mut second = Command::new(SHED)
        .arg("--dir")
        .arg(&dir)
        .args(["serve", "--sample", "3600"])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("shed serve");
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = second.try_wait().expect("try_wait") {
            break status;
        }
        if Instant::now() > deadline {
            second.kill().ok();
            second.wait().ok();
            panic!("a second server came up beside the first");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(!status.success());
    let out = second.wait_with_output().expect("stderr");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("already runs"),
        "{out:?}"
    );

    drop(first);
    assert!(dir.join("state.lock").exists(), "the dead server's lock");
    let _third = Server::start(&dir);
    assert!(UnixStream::connect(&paths.socket).is_ok());
}

#[test]
fn two_autostarts_at_once_share_one_server() {
    let dir = temp_dir("rel-autostart");
    let d = dir.to_str().expect("utf-8 dir");
    let start = |name: &str| {
        Command::new(SHED)
            .arg("--dir")
            .arg(&dir)
            .args(["run", "--autostart", "--name", name, "--cwd", d, "--"])
            .args(["sh", "-c", "echo $PPID; exec sleep 100"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("shed run")
    };
    let runs = [start("x"), start("y")].map(|c| c.wait_with_output().expect("run"));
    for out in &runs {
        assert!(out.status.success(), "{out:?}");
    }
    let mut servers = Vec::new();
    for name in ["x", "y"] {
        let mut server = String::new();
        eventually("the server's pid", || {
            server = ok(&dir, &["read", name]).trim().to_owned();
            !server.is_empty()
        });
        servers.push(server);
    }
    let killed = Command::new("kill").arg(&servers[0]).status();
    assert_eq!(servers[0], servers[1], "one server");
    assert!(killed.expect("kill").success());
}

#[test]
fn a_first_start_menu_is_seen_read_and_answered() {
    let dir = temp_dir("rel-trust");
    let _server = Server::start(&dir);
    let menu = "printf 'Do you trust the files in this folder?\\n> 1. Yes, proceed\\n  2. No, exit\\n'; \
                read a; echo trusted; exec sleep 100";
    run_sh(&dir, "agent", menu);
    let waited = shed_in(
        &dir,
        &[
            "wait",
            "agent",
            "--output",
            "trust the files",
            "--timeout",
            "10",
        ],
    );
    assert!(waited.status.success(), "{waited:?}");
    let screen = ok(&dir, &["read", "agent"]);
    assert!(screen.contains("> 1. Yes, proceed"), "{screen}");
    ok(&dir, &["keys", "agent", "enter"]);
    eventually("the answer", || {
        ok(&dir, &["read", "agent"]).contains("trusted")
    });
}

#[test]
fn malformed_requests_are_denied_with_a_reason_and_never_pass_the_handshake() {
    let dir = temp_dir("rel-malformed");
    let _server = Server::start(&dir);
    let stream = UnixStream::connect(Paths::in_dir(&dir).socket).expect("connect");
    let mut reader = BufReader::new(stream.try_clone().expect("clone"));
    let mut writer = stream;
    let mut ask = |line: &str| -> Value {
        writeln!(writer, "{line}").expect("write");
        let mut reply = String::new();
        reader.read_line(&mut reply).expect("read");
        serde_json::from_str(&reply).expect("json")
    };
    for (line, code) in [
        ("{not json", -32700),
        (r#"{"jsonrpc":"1.0","id":1,"method":"hello"}"#, -32600),
        (r#"{"jsonrpc":"2.0","id":1}"#, -32600),
        (
            r#"{"jsonrpc":"2.0","id":1,"method":"hello","params":7}"#,
            -32602,
        ),
        (
            r#"{"jsonrpc":"2.0","id":1,"method":"hello","params":{"product":"shed"}}"#,
            -32602,
        ),
        (r#"{"jsonrpc":"2.0","id":1,"method":"nope"}"#, -32601),
        (r#"{"jsonrpc":"2.0","id":1,"method":"ls"}"#, -32001),
    ] {
        let reply = ask(line);
        assert_eq!(reply["error"]["code"], code, "{line}: {reply}");
        assert!(
            !reply["error"]["message"]
                .as_str()
                .unwrap_or_default()
                .is_empty(),
            "{line}: a reason"
        );
    }
    let state =
        r#"{"jsonrpc":"2.0","id":2,"method":"state","params":{"name":"t1","state":"done"}}"#;
    assert_eq!(ask(state)["error"]["code"], -32001, "still no handshake");
}

#[test]
fn a_quiet_terminal_is_surfaced_as_idle_on_the_events_stream() {
    let dir = temp_dir("rel-idle");
    let d = dir.to_str().expect("utf-8 dir");
    let _server = Server::start(&dir);
    let mut events = Command::new(SHED)
        .arg("--dir")
        .arg(&dir)
        .args(["events", "--label", "rel=idle"])
        .stdout(Stdio::piped())
        .spawn()
        .expect("shed events");
    let quiet = [
        "run",
        "--name",
        "stuck",
        "--label",
        "rel=idle",
        "--idle-after",
        "1",
        "--cwd",
        d,
        "--",
        "sleep",
        "100",
    ];
    ok(&dir, &quiet);
    let mut lines = BufReader::new(events.stdout.take().expect("stdout")).lines();
    let started = Instant::now();
    let idle = lines.find_map(|l| {
        let v: Value = serde_json::from_str(&l.ok()?).ok()?;
        (v["event"] == "state").then_some(v)
    });
    events.kill().ok();
    events.wait().ok();
    let idle = idle.expect("a state event");
    assert_eq!(
        (&idle["name"], &idle["state"]),
        (&json!("stuck"), &json!("idle"))
    );
    assert!(started.elapsed() < Duration::from_secs(10));
}
