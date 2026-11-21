//! The stream driver end to end: `tests/fake_stream.py` speaks the `plain` wire, its record turns the
//! driver on, and lanes are driven through the CLI against a real server.
#![cfg(test)]

mod common;

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use common::{Server, eventually, temp_dir};
use serde_json::{Value, json};
use shed_core::proctree::{self, ProcessTable as _};
use shed_server::Paths;

const SHED: &str = env!("CARGO_BIN_EXE_shed");

fn tests_file(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join(name);
    path.to_str().expect("utf-8").to_owned()
}

fn shed(dir: &Path, args: &[&str]) -> Output {
    let out = Command::new(SHED).arg("--dir").arg(dir).args(args).output();
    out.expect("shed runs")
}

fn ok(dir: &Path, args: &[&str]) -> String {
    let out = shed(dir, args);
    assert!(out.status.success(), "shed {args:?} failed: {out:?}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn get(dir: &Path, name: &str) -> Value {
    serde_json::from_str(&ok(dir, &["get", name])).expect("JSON")
}

struct Lab {
    dir: PathBuf,
    log: PathBuf,
    _server: Server,
}

impl Lab {
    fn new(name: &str, env: &[(&str, &str)]) -> Self {
        let dir = temp_dir(name);
        let (log, store) = (dir.join("starts.jsonl"), dir.join("store"));
        std::fs::create_dir_all(&store).expect("the store");
        let providers = Paths::in_dir(&dir).providers();
        std::fs::create_dir_all(&providers).expect("providers dir");
        let command = json!([
            "python3",
            tests_file("fake_stream.py"),
            "--log",
            log,
            "--store",
            store
        ]);
        let record = format!(
            "id = \"fake\"\ndriver = \"stream\"\n[stream]\nwire = \"plain\"\ncommand = {command}\n\
             adapter = \"fake_acp\"\nlaunchers = [\"python3\"]\n\
             new = [\"--session-id\", \"{{session}}\"]\nresume = [\"--resume\", \"{{session}}\"]\n\
             model = [\"--model\", \"{{model}}\"]\nmcp = [\"--mcp-config\", \"{{file}}\"]\n\
             [stream.policy]\nallow = [\"--allow\"]\ndeny = [\"--deny\"]\n\
             [stream.meta]\n\"opts.strict\" = \"--strict\"\n"
        );
        std::fs::write(providers.join("fake.toml"), record).expect("write the record");
        let server = if env.is_empty() {
            Server::start(&dir)
        } else {
            Server::with_env(&dir, "3600", env)
        };
        Self {
            dir,
            log,
            _server: server,
        }
    }

    fn run(&self, name: &str, args: &[&str]) {
        let run = ["run", "--name", name, "--acp", "fake"];
        ok(&self.dir, &[&run[..], args].concat());
        let ready = [
            "wait",
            name,
            "--output",
            "ACP session ready",
            "--timeout",
            "10",
        ];
        ok(&self.dir, &ready);
    }

    fn turn(&self, name: &str, prompt: &str) {
        ok(&self.dir, &["send", name, prompt]);
        let until = ["wait", name, "--until", "done", "error", "--timeout", "10"];
        assert_eq!(ok(&self.dir, &until), format!("{name} done\n"));
    }

    fn starts(&self) -> Vec<Vec<String>> {
        let log = std::fs::read_to_string(&self.log).unwrap_or_default();
        let args = |l: &str| serde_json::from_str::<Vec<String>>(l).expect("a start");
        let tail = |a: Vec<String>| a.into_iter().skip(4).collect();
        log.lines().map(args).map(tail).collect()
    }

    fn session(&self, name: &str) -> (String, String) {
        let session = &get(&self.dir, name)["session"];
        let field = |k: &str| session[k].as_str().unwrap_or_default().to_owned();
        (field("driver"), field("id"))
    }

    fn processes(&self, name: &str) -> usize {
        let pid = get(&self.dir, name)["pid"].as_i64().expect("a pid");
        let pid = i32::try_from(pid).expect("a pid");
        let procs = proctree::system().list().expect("the process table");
        proctree::tree(&procs, pid, &std::collections::BTreeSet::new()).len()
    }
}

fn strings(words: &[&str]) -> Vec<String> {
    words.iter().map(|&w| w.to_owned()).collect()
}

#[test]
fn a_turn_and_a_tool_call_run_on_the_agent_s_own_lines_under_an_id_shed_made() {
    let lab = Lab::new("stream-turn", &[]);
    lab.run("s", &["--policy", "allow"]);
    let (driver, id) = lab.session("s");
    assert_eq!(driver, "stream");
    let groups: Vec<usize> = id.split('-').map(str::len).collect();
    assert_eq!(
        (groups, id.chars().nth(14)),
        (vec![8, 4, 4, 4, 12], Some('4'))
    );
    lab.turn("s", "hi");
    lab.turn("s", "tool");
    assert_eq!(
        ok(&lab.dir, &["read", "s"]),
        "> hi\n[default] you said: hi\n> tool\n[tool] Build (completed)\nbuilt\n"
    );
    assert_eq!(lab.starts(), [strings(&["--allow", "--session-id", &id])]);
    assert_eq!(
        lab.processes("s"),
        2,
        "the bridge and the agent, no adapter"
    );
    ok(&lab.dir, &["kill", "s"]);
    eventually("the kill", || get(&lab.dir, "s")["state"] == "exited");
    ok(&lab.dir, &["rm", "s"]);
    assert_eq!(
        lab.starts().len(),
        1,
        "rm sends no ACP close through the agent's command"
    );
}

#[test]
fn cancel_stops_the_agent_and_the_next_send_resumes_the_same_session() {
    let lab = Lab::new("stream-cancel", &[]);
    lab.run("c", &["--policy", "deny"]);
    let (_, id) = lab.session("c");
    ok(&lab.dir, &["send", "c", "slow"]);
    let events = || ok(&lab.dir, &["read", "c", "--events", "--json"]);
    eventually("the first text", || events().contains("thinking"));
    ok(&lab.dir, &["cancel", "c"]);
    ok(
        &lab.dir,
        &["wait", "c", "--until", "done", "--timeout", "10"],
    );
    let reported = json!({"state": "done", "note": "cancelled", "from": "reported"});
    assert_eq!(get(&lab.dir, "c")["reported"], reported);
    eventually("the agent stopped", || lab.processes("c") == 1);
    lab.turn("c", "more");
    assert_eq!(lab.session("c"), ("stream".to_owned(), id.clone()));
    assert_eq!(
        lab.starts(),
        [
            strings(&["--deny", "--session-id", &id]),
            strings(&["--deny", "--resume", &id]),
        ]
    );
    let reported = json!({"state": "done", "note": "end_turn", "from": "reported"});
    assert_eq!(
        get(&lab.dir, "c")["reported"],
        reported,
        "the stray turn end ended nothing"
    );
    assert_eq!(
        ok(&lab.dir, &["read", "c"]),
        "> slow\n[tool] Sleep (failed)\nthinking\n> more\n[default] you said: more\n"
    );
}

#[test]
fn a_model_switch_stops_the_agent_and_the_next_send_starts_it_with_the_model() {
    let lab = Lab::new("stream-model", &[]);
    lab.run("m", &["--policy", "allow"]);
    let (_, id) = lab.session("m");
    eventually("the first start", || lab.starts().len() == 1);
    ok(&lab.dir, &["model", "m", "smart"]);
    eventually("the switch", || {
        get(&lab.dir, "m")["session"]["model"] == "smart" && lab.processes("m") == 1
    });
    lab.turn("m", "hi");
    assert_eq!(ok(&lab.dir, &["read", "m"]), "> hi\n[smart] you said: hi\n");
    assert_eq!(
        lab.starts(),
        [
            strings(&["--allow", "--session-id", &id]),
            strings(&["--allow", "--resume", &id, "--model", "smart"]),
            strings(&["--allow", "--session-id", &id, "--model", "smart"]),
        ],
        "a session with no turn yet is unknown to the agent, so the resume is tried once with new"
    );
}

#[test]
fn an_idle_lane_hibernates_and_a_send_wakes_it_on_its_session() {
    let lab = Lab::new("stream-hibernate", &[("SHED_ACP_HIBERNATE_AFTER", "2")]);
    lab.run("h", &["--policy", "allow"]);
    let (_, id) = lab.session("h");
    lab.turn("h", "hi");
    let until = ["wait", "h", "--until", "hibernated", "--timeout", "10"];
    assert_eq!(ok(&lab.dir, &until), "h hibernated\n");
    assert_eq!(lab.processes("h"), 1, "the agent is gone");
    lab.turn("h", "more");
    assert_eq!(lab.processes("h"), 2, "awake again");
    let last = lab.starts().pop();
    assert_eq!(last, Some(strings(&["--allow", "--resume", &id])));
    assert_eq!(
        ok(&lab.dir, &["read", "h"]),
        "> hi\n[default] you said: hi\n> more\n[default] you said: more\n"
    );
}

#[test]
fn policy_ask_and_an_unmapped_meta_option_stay_on_acp_and_say_why() {
    let lab = Lab::new("stream-acp", &[]);
    let file = lab.dir.join("meta.json");
    std::fs::write(&file, r#"{"other": 1}"#).expect("write the meta");
    let (acp, file) = (tests_file("fake_acp.py"), file.to_str().expect("utf-8"));
    let with_meta = format!("exec python3 \"$0\" --meta {file}");
    let cases = [
        (
            "a",
            vec!["--policy", "ask", "--", "python3", &acp],
            "policy ask has no flags",
        ),
        (
            "b",
            vec![
                "--policy",
                "allow",
                "--meta",
                r#"{"other": 1}"#,
                "--",
                "sh",
                "-c",
                &with_meta,
                &acp,
            ],
            "--meta option other has no flag",
        ),
    ];
    for (name, args, why) in cases {
        lab.run(name, &args);
        let why = format!("shed: on ACP: {why} on the stream driver");
        ok(
            &lab.dir,
            &["wait", name, "--output", &why, "--timeout", "10"],
        );
        assert_eq!(lab.session(name).0, "acp");
        lab.turn(name, "hi");
        assert_eq!(ok(&lab.dir, &["read", name]), "> hi\nyou said: hi\n");
    }
    assert!(lab.starts().is_empty(), "the stream agent never started");
    lab.run(
        "c",
        &[
            "--policy",
            "allow",
            "--meta",
            r#"{"opts": {"strict": true}}"#,
        ],
    );
    assert_eq!(lab.session("c").0, "stream");
    eventually("the start", || lab.starts().len() == 1);
    let flags = lab.starts().pop().map(|a| a[..2].to_vec());
    assert_eq!(flags, Some(strings(&["--allow", "--strict"])));
}

#[test]
fn a_caller_s_wrapper_is_kept_and_only_its_adapter_is_swapped() {
    let lab = Lab::new("stream-wrapper", &[]);
    let wrapper = ["--", "sh", "-c", "WRAPPED=yes exec \"$@\"", "sh", "python3"];
    lab.run(
        "w",
        &[
            &["--policy", "allow"][..],
            &wrapper,
            &[&tests_file("fake_acp.py")],
        ]
        .concat(),
    );
    assert_eq!(lab.session("w").0, "stream");
    lab.turn("w", "env");
    assert_eq!(
        ok(&lab.dir, &["read", "w", "--lines", "1"]),
        "WRAPPED=yes\n"
    );
    let cmd = get(&lab.dir, "w")["cmd"].clone();
    let cmd: Vec<String> = serde_json::from_value(cmd).expect("a command");
    let after = cmd
        .iter()
        .skip_while(|a| *a != "--")
        .skip(1)
        .take(6)
        .cloned();
    let fake = tests_file("fake_stream.py");
    let want = [
        "sh",
        "-c",
        "WRAPPED=yes exec \"$@\"",
        "sh",
        "python3",
        &fake,
    ];
    assert_eq!(after.collect::<Vec<_>>(), strings(&want));
}

#[test]
fn mcp_servers_are_in_a_private_file_until_the_agent_s_first_line() {
    let lab = Lab::new("stream-mcp", &[]);
    let servers = json!([{"name": "notes", "command": "/bin/echo", "args": [],
        "env": [{"name": "TOKEN", "value": "s3cret"}]}]);
    let file = lab.dir.join("mcp.json");
    std::fs::write(&file, servers.to_string()).expect("write the servers");
    let file = file.to_str().expect("utf-8");
    lab.run("p", &["--policy", "allow", "--mcp", file]);
    let (_, id) = lab.session("p");
    let state = Paths::in_dir(&lab.dir).state;
    let private = state
        .with_file_name("sessions")
        .join(format!("{id}.mcp.json"));
    let mode = std::fs::metadata(&private)
        .expect("the file")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
    lab.turn("p", "mcp");
    assert_eq!(
        ok(&lab.dir, &["read", "p", "--lines", "1"]),
        format!("0o600 {servers}\n")
    );
    assert!(!private.exists(), "removed at the agent's first line");
    let shown = &get(&lab.dir, "p")["session"]["mcp_servers"][0]["env"][0]["value"];
    assert_eq!(shown, "***");
}

#[test]
fn run_s_model_takes_the_stream_record_s_words() {
    let lab = Lab::new("stream-run-model", &[]);
    lab.run("m", &["--policy", "allow", "--model", "smart"]);
    let (_, id) = lab.session("m");
    lab.turn("m", "hi");
    let want = ["--allow", "--model", "smart", "--session-id", &id];
    assert_eq!(lab.starts(), [strings(&want)]);
}
