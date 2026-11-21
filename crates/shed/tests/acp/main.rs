//! ACP client mode end to end: `tests/fake_acp.py`, a scripted ACP agent, installed from a fixture
//! registry and driven through the CLI against a real server.
//!
//! The turn with an approval is checked against `tests/golden/acp.txt` (commands, output and the event
//! log). Regenerate with `just generate` (the tests in `golden.rs`).
#![cfg(test)]

#[path = "../common/mod.rs"]
mod common;
mod golden;
mod hibernate;
mod install;
mod login;
mod mcp;
mod memory;
mod model;
mod restart;
mod resume;
mod turn;
mod usage;
mod warm;

use std::io::BufRead as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use common::Server;
use serde_json::{Value, json};
use shed_client::Conn;
use shed_server::Paths;
use shed_server::proto::{EventsParams, Method};

const SHED: &str = env!("CARGO_BIN_EXE_shed");

fn fake_agent() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake_acp.py")
}

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

fn sha256(path: &Path) -> String {
    let out = Command::new("shasum")
        .args(["-a", "256"])
        .arg(path)
        .output()
        .or_else(|_| Command::new("sha256sum").arg(path).output())
        .expect("a sha256 tool");
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.split_whitespace().next().expect("a sum").to_owned()
}

fn platform() -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        os => os,
    };
    format!("{os}-{}", std::env::consts::ARCH)
}

fn npm_package(dir: &Path, name: &str, manifest: &Value) -> String {
    let pkg = dir.join("npm").join(name).join("package");
    std::fs::create_dir_all(&pkg).expect("npm package dir");
    std::fs::write(pkg.join("package.json"), manifest.to_string()).expect("package.json");
    std::fs::write(pkg.join("bin.js"), "console.log('npx-agent ran')\n").expect("bin.js");
    let tgz = dir.join(format!("{name}.tgz"));
    let tar = Command::new("tar")
        .arg("-czf")
        .arg(&tgz)
        .arg("-C")
        .arg(pkg.parent().expect("parent"))
        .arg("package")
        .status()
        .expect("tar");
    assert!(tar.success());
    tgz.to_str().expect("utf-8").to_owned()
}

fn registry(dir: &Path) -> PathBuf {
    let pkg = dir.join("pkg");
    std::fs::create_dir_all(&pkg).expect("pkg dir");
    std::fs::copy(fake_agent(), pkg.join("fake-acp")).expect("copy the agent");
    let mode = std::fs::Permissions::from_mode(0o755);
    std::fs::set_permissions(pkg.join("fake-acp"), mode).expect("chmod");
    let prompts = Command::new("cp")
        .arg("-R")
        .arg(fake_agent().with_file_name("fake_acp"))
        .arg(&pkg)
        .status()
        .expect("cp");
    assert!(prompts.success());
    let archive = dir.join("fake-acp.tar.gz");
    let tar = Command::new("tar")
        .arg("-czf")
        .arg(&archive)
        .arg("-C")
        .arg(&pkg)
        .arg("fake-acp")
        .arg("fake_acp")
        .status()
        .expect("tar");
    assert!(tar.success());
    let url = format!("file://{}", archive.display());
    let binary =
        |sum: &str| json!({platform(): {"archive": url, "cmd": "./fake-acp", "sha256": sum}});
    let sum = sha256(&archive);
    let lister = json!({platform(): {"archive": url, "cmd": "./fake-acp", "sha256": sum,
        "args": ["--sessions"]}});
    let npx = npm_package(
        dir,
        "npx-agent",
        &json!({"name": "npx-agent", "version": "1.2.3", "bin": {"npx-agent": "./bin.js"}}),
    );
    let no_bin = npm_package(
        dir,
        "no-bin",
        &json!({"name": "no-bin", "version": "1.0.0"}),
    );
    let missing = dir.join("missing.tgz");
    let registry = json!({"version": "1.0.0", "agents": [
        {"id": "fake", "version": "0.1.0", "distribution": {"binary": binary(&sum)}},
        {"id": "lister", "version": "0.1.0", "distribution": {"binary": lister}},
        {"id": "npx-agent", "version": "1.2.3", "distribution": {"npx": {
            "package": npx, "args": ["--acp"], "env": {"NO_UPDATE": "1"}}}},
        {"id": "no-bin", "version": "1.0.0", "distribution": {"npx": {"package": no_bin}}},
        {"id": "npx-missing", "version": "1.0.0", "distribution": {"npx": {"package": missing}}},
        {"id": "uvx-agent", "version": "0.4.0", "distribution": {"uvx": {"package": "uvx-agent==0.4.0"}}},
        {"id": "bad-sum", "version": "0.1.0", "distribution": {"binary": binary("00")}},
    ]});
    let path = dir.join("registry.json");
    std::fs::write(&path, registry.to_string()).expect("write the registry");
    path
}

fn events_until(dir: &Path, run: &str, last: impl Fn(&Value) -> bool) -> Vec<String> {
    let mut client = Conn::connect(&Paths::in_dir(dir).socket).expect("connect");
    let params = EventsParams {
        labels: [("run".to_owned(), run.to_owned())].into(),
        since: Some(0),
    };
    let _: Value = client.call(Method::Events, &params).expect("events");
    let (reader, _writer) = client.split();
    reader
        .get_ref()
        .set_read_timeout(Some(std::time::Duration::from_secs(10)))
        .expect("timeout");
    let mut lines = Vec::new();
    for line in reader.lines() {
        let line = line.expect("an event line");
        let done = last(&serde_json::from_str(&line).expect("event JSON"));
        lines.push(line);
        if done {
            return lines;
        }
    }
    panic!("the events ended early: {lines:?}");
}

fn is_state(state: &str) -> impl Fn(&Value) -> bool {
    move |e| e["event"] == "state" && e["state"] == state
}

fn start_fake(dir: &Path, name: &str, id: &str) {
    let agent = fake_agent();
    let agent = agent.to_str().expect("utf-8");
    let label = format!("run={name}");
    let run = ["run", "--name", name, "--label", &label, "--acp", id, "--"];
    ok(dir, &[&run[..], &["python3", agent]].concat());
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

fn reported(dir: &Path, name: &str) -> Value {
    let info: Value = serde_json::from_str(&ok(dir, &["get", name])).expect("JSON");
    info["reported"].clone()
}

fn start_with_history(dir: &Path, name: &str, history: &Path, extra: &[&str]) {
    let agent = fake_agent();
    let label = format!("run={name}");
    let run = ["run", "--name", name, "--label", &label, "--acp", "fake"];
    let cmd = ["--", "python3", agent.to_str().expect("utf-8"), "--history"];
    let history = history.to_str().expect("utf-8");
    ok(dir, &[&run[..], extra, &cmd, &[history]].concat());
}

fn row(dir: &Path, args: &[&str], name: &str) -> Vec<String> {
    let out = ok(dir, args);
    let head = out.lines().next().unwrap_or_default().split_whitespace();
    let line = out.lines().find(|l| l.starts_with(&format!("{name} ")));
    let cells = line.unwrap_or_default().split_whitespace();
    head.zip(cells).map(|(h, c)| format!("{h}={c}")).collect()
}

fn sleepy_server(dir: &Path) -> Server {
    Server::with_env(dir, "3600", &[("SHED_ACP_HIBERNATE_AFTER", "2")])
}

fn until(dir: &Path, name: &str, state: &str) {
    let wait = ["wait", name, "--until", state, "--timeout", "10"];
    assert_eq!(ok(dir, &wait), format!("{name} {state}\n"));
}

fn first_turn(dir: &Path, name: &str) {
    ok(dir, &["send", name, "hi"]);
    until(dir, name, "done");
}

fn stop(server: &mut Server) {
    let pid = server.0.id().to_string();
    let sent = Command::new("kill").args(["-TERM", &pid]).status();
    assert!(sent.expect("kill runs").success());
    server.0.wait().expect("shed serve stops");
}
