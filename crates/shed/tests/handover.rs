//! `handover`: serve replaces itself with its own build in place, keeping a shell and an ACP session.
#![cfg(test)]

mod common;

use std::io::{BufRead as _, Read as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::Duration;

use common::{Server, temp_dir};
use serde_json::{Value, json};
use shed_client::Conn;
use shed_server::Paths;
use shed_server::proto::{EventsParams, Handover, Method, Target, TerminalInfo};

const SHED: &str = env!("CARGO_BIN_EXE_shed");

fn shed(dir: &Path, args: &[&str]) -> Output {
    Command::new(SHED)
        .arg("--dir")
        .arg(dir)
        .args(args)
        .output()
        .expect("shed runs")
}

fn ok(dir: &Path, args: &[&str]) -> String {
    let out = shed(dir, args);
    assert!(out.status.success(), "shed {args:?} failed: {out:?}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn client(dir: &Path) -> Conn {
    Conn::connect(&Paths::in_dir(dir).socket).expect("connect")
}

fn get(dir: &Path, name: &str) -> TerminalInfo {
    let target = Target {
        name: name.to_owned(),
    };
    client(dir).call(Method::Get, &target).expect("get")
}

fn hand_over(dir: &Path, build: &str) -> Result<Handover, shed_client::Error> {
    client(dir).call(Method::Handover, &json!({ "build": build }))
}

fn attach(dir: &Path, name: &str) -> std::io::BufReader<std::os::unix::net::UnixStream> {
    let mut attached = client(dir);
    let target = Target {
        name: name.to_owned(),
    };
    let _: TerminalInfo = attached.call(Method::Attach, &target).expect("attach");
    let (reader, _writer) = attached.split();
    reader
        .get_ref()
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("timeout");
    reader
}

fn parent(pid: i32) -> String {
    let out = Command::new("ps")
        .args(["-o", "ppid=", "-p", &pid.to_string()])
        .output()
        .expect("ps");
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

fn fake_agent() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake_acp.py")
}

#[test]
fn handover_keeps_the_terminals_their_screens_and_children_twice() {
    let dir = temp_dir("handover");
    let mut server = Server::start(&dir);
    let serve = server.0.id().to_string();
    ok(&dir, &["run", "--name", "sh", "--", "sh"]);
    ok(&dir, &["send", "sh", "printf 'mark-%s\\n' one"]);
    ok(
        &dir,
        &["wait", "sh", "--output", "mark-one", "--timeout", "10"],
    );
    let agent = fake_agent();
    let acp = ["run", "--name", "a", "--acp", "fake", "--", "python3"];
    ok(
        &dir,
        &[&acp[..], &[agent.to_str().expect("utf-8")]].concat(),
    );
    let ready = [
        "wait",
        "a",
        "--output",
        "ACP session ready",
        "--timeout",
        "10",
    ];
    ok(&dir, &ready);
    let before = (get(&dir, "sh").pid, get(&dir, "a").pid);
    let build = client(&dir).server().build.clone();

    let refused = hand_over(&dir, "not-a-build").expect_err("another build is refused");
    assert!(
        refused.to_string().starts_with("HANDOVER_REFUSED"),
        "{refused}"
    );
    let check = json!({"build": build, "candidate": "/usr/bin/false"});
    let checked: Result<Handover, _> = client(&dir).call(Method::HandoverCheck, &check);
    assert!(checked.is_err(), "a binary that does not start is refused");
    assert_eq!((get(&dir, "sh").pid, get(&dir, "a").pid), before);

    let mut attached = attach(&dir, "sh");
    let handed = hand_over(&dir, &build).expect("handover");
    assert_eq!((handed.terminals, &handed.to), (2, &build));
    let mut rest = Vec::new();
    attached
        .read_to_end(&mut rest)
        .expect("the attach is closed");

    for round in ["two", "three"] {
        assert_eq!(server.0.try_wait().expect("serve"), None, "serve runs on");
        assert_eq!((get(&dir, "sh").pid, get(&dir, "a").pid), before);
        let sh = before.0.expect("a shell pid");
        assert_eq!(parent(sh), serve, "the shell is still serve's child");
        assert!(ok(&dir, &["read", "sh"]).contains("mark-one"));
        ok(
            &dir,
            &["send", "sh", &format!("printf 'mark-%s\\n' {round}")],
        );
        let mark = format!("mark-{round}");
        ok(&dir, &["wait", "sh", "--output", &mark, "--timeout", "10"]);
        let mut again = attach(&dir, "sh");
        let mut seen = String::new();
        while !seen.contains("mark-one") {
            let got = again.fill_buf().expect("the redraw");
            assert!(!got.is_empty(), "attached again: {seen:?}");
            seen.push_str(&String::from_utf8_lossy(got));
            let n = got.len();
            again.consume(n);
        }
        let mut events = client(&dir);
        let since = EventsParams {
            since: Some(1),
            ..EventsParams::default()
        };
        let _: Value = events.call(Method::Events, &since).expect("events");
        let (mut reader, _writer) = events.split();
        let mut first = String::new();
        reader.read_line(&mut first).expect("an event line");
        let first: Value = serde_json::from_str(&first).expect("JSON");
        assert_eq!(first["method"], "reset", "{first}");
        if round == "two" {
            hand_over(&dir, &build).expect("a second handover");
            again.read_to_end(&mut rest).expect("the attach is closed");
        }
    }
    ok(&dir, &["send", "a", "hi"]);
    ok(&dir, &["wait", "a", "--until", "done", "--timeout", "10"]);
    ok(&dir, &["send", "sh", "exit 3"]);
    shed(&dir, &["wait", "sh", "--timeout", "10"]);
    assert_eq!(get(&dir, "sh").exit, Some(3), "the real exit status");
}
