//! `run --model --effort --env`: the record's words reach the agent, and the variables' values reach the
//! program but never `get`, `agents`, events, logs or a hibernated session.
#![cfg(test)]

mod common;

use std::io::BufRead as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use common::{Server, temp_dir};
use serde_json::Value;
use shed_client::Conn;
use shed_server::Paths;
use shed_server::proto::{EventsParams, Method};

const SHED: &str = env!("CARGO_BIN_EXE_shed");
const SECRET: &str = "s3cret-env-value";

fn shed(dir: &Path, args: &[&str]) -> Output {
    let out = Command::new(SHED).arg("--dir").arg(dir).args(args).output();
    out.expect("shed runs")
}

fn ok(dir: &Path, args: &[&str]) -> String {
    let out = shed(dir, args);
    assert!(out.status.success(), "shed {args:?} failed: {out:?}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn lab(name: &str, env: &[(&str, &str)]) -> (PathBuf, Server) {
    let dir = temp_dir(name);
    let providers = Paths::in_dir(&dir).providers();
    std::fs::create_dir_all(&providers).expect("providers dir");
    let record = "id = \"fake\"\nmodel_env = \"FAKE_MODEL\"\n\
                  effort_args = [\"--effort\", \"{effort}\"]\nefforts = [\"low\", \"high\"]\n";
    std::fs::write(providers.join("fake.toml"), record).expect("write the record");
    let server = if env.is_empty() {
        Server::start(&dir)
    } else {
        Server::with_env(&dir, "3600", env)
    };
    (dir, server)
}

fn start(dir: &Path, name: &str, effort: &str) -> Output {
    let agent = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake_acp.py");
    let history = dir.join(format!("{name}.jsonl"));
    let secret = format!("SHED_T_SECRET={SECRET}");
    let run = ["run", "--name", name, "--label", "run=env", "--acp", "fake"];
    let picks = ["--model", "smart", "--effort", effort, "--env", &secret];
    let cmd = [
        "--",
        "python3",
        agent.to_str().expect("utf-8"),
        "--history",
        history.to_str().expect("utf-8"),
    ];
    shed(dir, &[&run[..], &picks, &cmd].concat())
}

fn turn(dir: &Path, name: &str, prompt: &str) -> String {
    ok(dir, &["send", name, prompt]);
    ok(dir, &["wait", name, "--until", "done", "--timeout", "10"]);
    let read = ok(dir, &["read", name]);
    read.lines().last().unwrap_or_default().to_owned()
}

fn started(dir: &Path, name: &str) {
    assert!(start(dir, name, "high").status.success());
    let ready = ["wait", name, "--output", "ACP session ready"];
    ok(dir, &[&ready[..], &["--timeout", "10"]].concat());
    assert_eq!(
        turn(dir, name, "env SHED_T_SECRET"),
        "SHED_T_SECRET has 16 chars"
    );
    assert_eq!(turn(dir, name, "env FAKE_MODEL"), "FAKE_MODEL has 5 chars");
    assert!(turn(dir, name, "args").ends_with(" --effort high"));
}

fn files_without_the_secret(root: &Path) -> usize {
    let mut dirs = vec![root.to_owned()];
    let mut read = 0;
    while let Some(d) = dirs.pop() {
        for entry in std::fs::read_dir(&d).expect("read the state dir") {
            let path = entry.expect("an entry").path();
            if path.is_dir() {
                dirs.push(path);
            } else if let Ok(bytes) = std::fs::read(&path) {
                let text = String::from_utf8_lossy(&bytes);
                assert!(!text.contains(SECRET), "{} has the secret", path.display());
                read += 1;
            }
        }
    }
    read
}

#[test]
fn env_reaches_the_program_but_not_get() {
    let (dir, _server) = lab("env-get", &[]);
    started(&dir, "g");
    let shown = ok(&dir, &["get", "g"]) + &ok(&dir, &["ls", "--json"]);
    assert!(
        shown.contains("--effort") && !shown.contains(SECRET),
        "{shown}"
    );
}

#[test]
fn env_is_not_in_agents() {
    let (dir, _server) = lab("env-agents", &[]);
    started(&dir, "a");
    let shown = ok(&dir, &["--json", "agents"]);
    assert!(shown.contains("fake") && !shown.contains(SECRET), "{shown}");
}

#[test]
fn env_is_not_in_events() {
    let (dir, _server) = lab("env-events", &[]);
    started(&dir, "e");
    let mut client = Conn::connect(&Paths::in_dir(&dir).socket).expect("connect");
    let params = EventsParams {
        labels: [("run".to_owned(), "env".to_owned())].into(),
        since: Some(0),
    };
    let _: Value = client.call(Method::Events, &params).expect("events");
    let (reader, _writer) = client.split();
    let timeout = Some(std::time::Duration::from_secs(10));
    reader.get_ref().set_read_timeout(timeout).expect("timeout");
    let mut seen = String::new();
    for line in reader.lines() {
        let line = line.expect("an event line");
        assert!(!line.contains(SECRET), "{line}");
        seen.push_str(&line);
        if line.contains("has 5 chars") {
            break;
        }
    }
    assert!(seen.contains("SHED_T_SECRET has 16 chars"), "{seen}");
}

#[test]
fn env_is_not_in_logs() {
    let (dir, _server) = lab("env-logs", &[]);
    started(&dir, "l");
    let history = Paths::in_dir(&dir).state.with_file_name("history");
    assert!(files_without_the_secret(&history) > 0);
}

#[test]
fn env_is_not_in_a_hibernated_session_and_comes_back_with_the_agent() {
    let (dir, _server) = lab("env-hibernate", &[("SHED_ACP_HIBERNATE_AFTER", "1")]);
    started(&dir, "h");
    let wait = ["wait", "h", "--until", "hibernated", "--timeout", "10"];
    assert_eq!(ok(&dir, &wait), "h hibernated\n");
    assert!(files_without_the_secret(&dir) > 0);
    assert_eq!(
        turn(&dir, "h", "env SHED_T_SECRET"),
        "SHED_T_SECRET has 16 chars"
    );
}

#[test]
fn the_lanes_own_variables_do_not_reach_the_agent() {
    let (dir, _server) = lab("env-lane-only", &[("SHED_ACP_HIBERNATE_AFTER", "3600")]);
    started(&dir, "o");
    assert_eq!(
        turn(&dir, "o", "env SHED_ACP_HIBERNATE_AFTER"),
        "no SHED_ACP_HIBERNATE_AFTER"
    );
}

#[test]
fn an_effort_the_record_does_not_take_is_refused() {
    let (dir, _server) = lab("env-bad-effort", &[]);
    let out = start(&dir, "b", "max");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success() && err.contains("takes no effort max"),
        "{err}"
    );
}

#[test]
fn caller_on_names_shed_to_the_adapter_unless_the_record_s_env_names_another() {
    let (dir, _server) = lab("env-caller", &[]);
    let file = Paths::in_dir(&dir).providers().join("fake.toml");
    let record = std::fs::read_to_string(&file).expect("read the record");
    let said = |name: &str, keys: &str| {
        std::fs::write(&file, format!("{keys}\n{record}")).expect("write the record");
        assert!(start(&dir, name, "high").status.success());
        let ready = [
            "wait",
            name,
            "--output",
            "ACP session ready",
            "--timeout",
            "10",
        ];
        ok(&dir, &ready);
        turn(&dir, name, "env FAKE_CALLER")
    };
    let on = "caller_var = \"FAKE_CALLER\"\ncaller = true";
    assert_eq!(
        said("off", "caller_var = \"FAKE_CALLER\""),
        "no FAKE_CALLER"
    );
    assert_eq!(said("on", on), "FAKE_CALLER has 4 chars");
    let own = format!("{on}\nenv = {{ FAKE_CALLER = \"own-caller\" }}");
    assert_eq!(said("own", &own), "FAKE_CALLER has 10 chars");
}
