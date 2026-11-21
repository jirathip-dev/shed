use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::net::UnixStream;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use shed_server::Paths;

use crate::common::{Server, eventually, temp_dir};
use crate::{SHED, ok, run_sh, shed_in};

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
        "params": shed_server::proto::Hello::ours()});
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
