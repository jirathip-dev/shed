//! `run --account`: the account's folder and token reach the agent and the scrubbed variables do not, the
//! session's `_meta` and the command are those of a run without it, a caller `env` that sets an account
//! variable is refused, and the token is in no `get`, event, log, state file or hibernated session, before
//! and after a wake and a server restart.
#![cfg(test)]

mod common;

use std::io::BufRead as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use common::{Server, temp_dir};
use serde_json::{Value, json};
use shed_client::Conn;
use shed_server::Paths;
use shed_server::proto::{EventsParams, Method, RunParams};

const SHED: &str = env!("CARGO_BIN_EXE_shed");
const TOKEN: &str = "sk-fake-oat01-0123456789abcdef";
const API_KEY: &str = "sk-fake-api-key";
const RECORD: &str = "id = \"fake\"\nagent = \"fake\"\nconfig_dir_var = \"FAKE_HOME\"\n\
                      token_var = \"FAKE_TOKEN\"\nscrub = [\"FAKE_SCRUB\"]\n\
                      [cache]\nplan = 3600\napi = 300\napi_vars = [\"FAKE_API_KEY\"]\n";
const META: &str = r#"{"guard":{"settingSources":["project"]}}"#;

fn shed(dir: &Path, args: &[&str]) -> Output {
    let out = Command::new(SHED).arg("--dir").arg(dir).args(args).output();
    out.expect("shed runs")
}

fn ok(dir: &Path, args: &[&str]) -> String {
    let out = shed(dir, args);
    assert!(out.status.success(), "shed {args:?} failed: {out:?}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn lab(name: &str, hibernate: &str) -> (PathBuf, Server) {
    let dir = temp_dir(name);
    let providers = Paths::in_dir(&dir).providers();
    std::fs::create_dir_all(&providers).expect("providers dir");
    std::fs::write(providers.join("fake.toml"), RECORD).expect("write the record");
    std::fs::write(dir.join("meta.json"), META).expect("write the meta");
    let env = [
        ("FAKE_API_KEY", API_KEY),
        ("FAKE_SCRUB", "inherited"),
        ("SHED_ACP_HIBERNATE_AFTER", hibernate),
    ];
    let server = Server::with_env(&dir, "3600", &env);
    let file = dir.join("work.token");
    std::fs::write(&file, TOKEN).expect("write the token");
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).expect("chmod");
    let file = file.to_str().expect("utf-8");
    ok(
        &dir,
        &["account", "add", "fake", "work", "--token-file", file],
    );
    std::fs::remove_file(dir.join("work.token")).expect("rm the token");
    (dir, server)
}

fn start(dir: &Path, name: &str, account: &[&str], history: bool) -> Output {
    let agent = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake_acp.py");
    let meta = dir.join("meta.json");
    let file = dir.join(format!("{name}.jsonl"));
    let run = [
        "run",
        "--name",
        name,
        "--label",
        "run=account",
        "--acp",
        "fake",
    ];
    let mut cmd = vec!["--", "python3", agent.to_str().expect("utf-8"), "--unique"];
    cmd.extend(["--meta", meta.to_str().expect("utf-8")]);
    if history {
        cmd.extend(["--history", file.to_str().expect("utf-8")]);
    }
    let out = shed(dir, &[&run[..], account, &["--meta", META], &cmd].concat());
    if out.status.success() {
        let ready = [
            "wait",
            name,
            "--output",
            "ACP session ready",
            "--timeout",
            "10",
        ];
        ok(dir, &ready);
    }
    out
}

fn turn(dir: &Path, name: &str, prompt: &str) -> String {
    ok(dir, &["send", name, prompt]);
    ok(dir, &["wait", name, "--until", "done", "--timeout", "10"]);
    let read = ok(dir, &["read", name]);
    read.lines().last().unwrap_or_default().to_owned()
}

fn folder(dir: &Path) -> PathBuf {
    let state = Paths::in_dir(dir).state;
    state.with_file_name("accounts").join("fake/work")
}

fn on_the_account(dir: &Path, name: &str) {
    let home = folder(dir).display().to_string().len();
    let says = |var: &str| turn(dir, name, &format!("env {var}"));
    assert_eq!(says("FAKE_HOME"), format!("FAKE_HOME has {home} chars"));
    let token = TOKEN.len();
    assert_eq!(says("FAKE_TOKEN"), format!("FAKE_TOKEN has {token} chars"));
    assert_eq!(says("FAKE_API_KEY"), "no FAKE_API_KEY");
    assert_eq!(says("FAKE_SCRUB"), "no FAKE_SCRUB");
}

fn files_without_the_token(root: &Path) -> usize {
    let mut dirs = vec![root.to_owned()];
    let mut read = 0;
    while let Some(d) = dirs.pop() {
        for entry in std::fs::read_dir(&d).expect("read the state dir") {
            let path = entry.expect("an entry").path();
            if path.is_dir() {
                dirs.push(path);
            } else if let Some(bytes) = (path.extension() != Some("token".as_ref()))
                .then(|| std::fs::read(&path).ok())
                .flatten()
            {
                let text = String::from_utf8_lossy(&bytes);
                assert!(!text.contains(TOKEN), "{} has the token", path.display());
                read += 1;
            }
        }
    }
    read
}

#[test]
fn a_run_on_an_account_gets_its_login_and_the_same_meta_and_command_as_one_without() {
    let (dir, _server) = lab("account-run", "3600");
    assert!(
        start(&dir, "on", &["--account", "work"], false)
            .status
            .success()
    );
    assert!(start(&dir, "off", &[], false).status.success());
    on_the_account(&dir, "on");
    let says = |var: &str| turn(&dir, "off", &format!("env {var}"));
    let key = API_KEY.len();
    assert_eq!(
        says("FAKE_API_KEY"),
        format!("FAKE_API_KEY has {key} chars")
    );
    assert_eq!(says("FAKE_SCRUB"), "FAKE_SCRUB has 9 chars");
    assert_eq!(says("FAKE_HOME"), "no FAKE_HOME");
    assert_eq!(turn(&dir, "on", "args"), turn(&dir, "off", "args"));
    let get = |name| serde_json::from_str::<Value>(&ok(&dir, &["get", name])).expect("JSON");
    let (on, off) = (get("on"), get("off"));
    assert_eq!((&on["cmd"], &on["account"]), (&off["cmd"], &json!("work")));
    assert_eq!(off.get("account"), None);
    let shown = ok(&dir, &["get", "on"]) + &ok(&dir, &["ls", "--json"]);
    assert!(!shown.contains(TOKEN), "{shown}");
}

#[test]
fn the_account_event_says_start_and_no_event_holds_the_token() {
    let (dir, _server) = lab("account-events", "3600");
    let mut client = Conn::connect(&Paths::in_dir(&dir).socket).expect("connect");
    let params = EventsParams {
        labels: [("run".to_owned(), "account".to_owned())].into(),
        since: Some(0),
    };
    let _: Value = client.call(Method::Events, &params).expect("events");
    assert!(
        start(&dir, "e", &["--account", "work"], false)
            .status
            .success()
    );
    on_the_account(&dir, "e");
    let (reader, _writer) = client.split();
    let timeout = Some(std::time::Duration::from_secs(10));
    reader.get_ref().set_read_timeout(timeout).expect("timeout");
    let mut account = None;
    for line in reader.lines() {
        let line = line.expect("an event line");
        assert!(!line.contains(TOKEN), "{line}");
        let event: Value = serde_json::from_str(&line).expect("JSON");
        if event["event"] == "account" {
            account = Some((event["account"].clone(), event["why"].clone()));
        }
        if line.contains("no FAKE_SCRUB") {
            break;
        }
    }
    assert_eq!(account, Some((json!("work"), json!("start"))));
}

#[test]
fn a_wake_and_a_server_restart_keep_the_account_and_leave_the_token_out_of_every_file() {
    let (dir, mut server) = lab("account-wake", "1");
    assert!(
        start(&dir, "h", &["--account", "work"], true)
            .status
            .success()
    );
    on_the_account(&dir, "h");
    let wait = ["wait", "h", "--until", "hibernated", "--timeout", "10"];
    assert_eq!(ok(&dir, &wait), "h hibernated\n");
    assert!(files_without_the_token(&dir) > 0);
    on_the_account(&dir, "h");
    let pid = server.0.id().to_string();
    let sent = Command::new("kill").args(["-TERM", &pid]).status();
    assert!(sent.expect("kill runs").success());
    server.0.wait().expect("shed serve stops");
    let _server = Server::start(&dir);
    let got: Value = serde_json::from_str(&ok(&dir, &["get", "h"])).expect("JSON");
    assert_eq!(got["account"], json!("work"));
    assert_eq!(ok(&dir, &wait), "h hibernated\n");
    on_the_account(&dir, "h");
    assert!(files_without_the_token(&dir) > 0);
}

#[test]
fn a_caller_env_that_sets_an_account_variable_is_refused() {
    let (dir, _server) = lab("account-env", "3600");
    for var in ["FAKE_HOME", "FAKE_TOKEN", "FAKE_API_KEY", "FAKE_SCRUB"] {
        let env = format!("{var}=mine");
        let out = start(&dir, "r", &["--account", "work", "--env", &env], false);
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(
            !out.status.success() && err.contains(&format!("env sets {var}")),
            "{err}"
        );
    }
    let out = start(&dir, "none", &["--account", "none"], false);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success() && err.contains("NO_ACCOUNT"), "{err}");
    let mut client = Conn::connect(&Paths::in_dir(&dir).socket).expect("connect");
    let remote = RunParams {
        acp: Some("fake".into()),
        account: Some("work".into()),
        machine: Some("far".into()),
        cwd: "/".into(),
        ..RunParams::default()
    };
    let refused = client.call::<_, Value>(Method::Run, &remote).map(|_| ());
    let refused = refused.expect_err("refused").to_string();
    assert!(
        refused.contains("no account until the token proxy"),
        "{refused}"
    );
}
