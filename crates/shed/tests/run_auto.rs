//! `run --account auto`: it starts on the named account `auto` picks, never `default`; when the lane hits a
//! limit shed marks that account, copies the session's files to the next account's folder, starts the agent
//! there on the same session and sends one prompt to go on; `account.ls` shows the mark and the runs.
#![cfg(test)]

mod common;

use std::io::BufRead as _;
use std::path::Path;
use std::process::{Command, Output};

use common::{Server, temp_dir};
use serde_json::{Value, json};
use shed_client::Conn;
use shed_server::Paths;
use shed_server::proto::{EventsParams, Method};

const SHED: &str = env!("CARGO_BIN_EXE_shed");
const RECORD: &str = "id = \"fake\"\nagent = \"fake\"\nconfig_dir_var = \"FAKE_HOME\"\n\
                      status = \"echo in\"\nlimit = [\"hit your limit\"]\nresets = [\"resets \"]\n\
                      session_files = [\"projects/*/{session}.jsonl\"]\n";

fn shed(dir: &Path, args: &[&str]) -> Output {
    let out = Command::new(SHED).arg("--dir").arg(dir).args(args).output();
    out.expect("shed runs")
}

fn ok(dir: &Path, args: &[&str]) -> String {
    let out = shed(dir, args);
    assert!(out.status.success(), "shed {args:?} failed: {out:?}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn json_of(dir: &Path, args: &[&str]) -> Value {
    serde_json::from_str(&ok(dir, args)).expect("JSON")
}

#[test]
fn a_lane_on_auto_moves_to_the_next_account_on_a_limit_and_goes_on() {
    let dir = temp_dir("account-auto");
    let providers = Paths::in_dir(&dir).providers();
    std::fs::create_dir_all(&providers).expect("providers dir");
    std::fs::write(providers.join("fake.toml"), RECORD).expect("write the record");
    let _server = Server::start(&dir);
    let refused = shed(
        &dir,
        &["run", "--acp", "fake", "--account", "auto", "--", "true"],
    );
    let err = String::from_utf8_lossy(&refused.stderr);
    assert!(err.contains("NO_ACCOUNT"), "default is never picked: {err}");
    for name in ["b", "a"] {
        ok(&dir, &["account", "add", "fake", name]);
        ok(&dir, &["account", "status", "fake", name]);
    }
    let mut client = Conn::connect(&Paths::in_dir(&dir).socket).expect("connect");
    let params = EventsParams {
        labels: [("run".to_owned(), "auto".to_owned())].into(),
        since: Some(0),
    };
    let _: Value = client.call(Method::Events, &params).expect("events");
    let agent = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake_acp.py");
    let history = dir.join("history.jsonl");
    let run = ["run", "--name", "l", "--label", "run=auto", "--acp", "fake"];
    let cmd = [
        "--account",
        "auto",
        "--",
        "python3",
        agent.to_str().expect("utf-8"),
    ];
    let fake = ["--unique", "--history", history.to_str().expect("utf-8")];
    ok(&dir, &[&run[..], &cmd, &fake].concat());
    ok(
        &dir,
        &[
            "wait",
            "l",
            "--output",
            "ACP session ready",
            "--timeout",
            "10",
        ],
    );
    assert_eq!(json_of(&dir, &["get", "l"])["account"], json!("a"));
    ok(&dir, &["send", "l", "account limit"]);
    let (reader, _writer) = client.split();
    let timeout = Some(std::time::Duration::from_secs(10));
    reader.get_ref().set_read_timeout(timeout).expect("timeout");
    let (mut switched, mut prompts) = (None, Vec::new());
    for line in reader.lines() {
        let event: Value = serde_json::from_str(&line.expect("an event line")).expect("JSON");
        match event["event"].as_str() {
            Some("account") => switched = Some((event["was"].clone(), event["account"].clone())),
            Some("turn") => prompts.push(event["prompt"].as_str().unwrap_or_default().to_owned()),
            Some("reply") if prompts.len() == 2 => break,
            _ => {}
        }
    }
    assert_eq!(switched, Some((json!("a"), json!("b"))));
    assert_eq!(prompts.first().map(String::as_str), Some("account limit"));
    assert!(
        prompts[1].contains("moved this session to another account"),
        "{prompts:?}"
    );
    ok(&dir, &["wait", "l", "--until", "done", "--timeout", "10"]);
    ok(&dir, &["send", "l", "account"]);
    ok(&dir, &["wait", "l", "--output", "on b,", "--timeout", "10"]);
    let read = ok(&dir, &["read", "l"]);
    assert!(read.contains("on b, session file: yes"), "{read}");
    assert_eq!(json_of(&dir, &["get", "l"])["account"], json!("b"));
    let listed = json_of(&dir, &["account", "ls", "fake", "--json"]);
    let shown: Vec<_> = (listed.as_array().expect("a list").iter())
        .map(|a| (a["name"].clone(), a["state"].clone(), a["runs"].clone()))
        .collect();
    let want = [
        (json!("default"), json!("signed-out"), json!(0)),
        (json!("b"), json!("ready"), json!(1)),
        (json!("a"), json!("limited"), json!(0)),
    ];
    assert_eq!(shown, want);
    assert!(listed[2]["limited_until"].is_u64(), "{listed}");
    ok(&dir, &["send", "l", "account limit"]);
    ok(
        &dir,
        &["wait", "l", "--until", "rate-limited", "--timeout", "10"],
    );
    let get = |dir: &Path| json_of(dir, &["get", "l"]);
    common::eventually("the first reset in the note", || {
        let note = get(&dir)["reported"]["note"].as_str().map(str::to_owned);
        note.is_some_and(|n| n.contains("every account is limited; the first resets at"))
    });
    assert_eq!(get(&dir)["account"], json!("b"));
}
