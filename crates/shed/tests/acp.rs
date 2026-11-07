//! ACP client mode end to end: `tests/fake_acp.py`, a scripted ACP agent, installed from a fixture
//! registry and driven through the CLI against a real server.
//!
//! The turn with an approval is checked against `tests/golden/acp.txt` (commands, output and the event
//! log). Regenerate with `SHED_BLESS=1 cargo test -p shed --test acp`.

mod common;

use std::fmt::Write as _;
use std::io::BufRead as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use common::{Server, temp_dir};
use serde_json::{Value, json};
use shed_core::proctree::{self, ProcessTable as _};
use shed_server::proto::{EventsParams, Method};
use shed_server::{Client, Paths};

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

/// A registry file in `dir`: `fake` as this platform's binary (a tar.gz of the fake agent, built here),
/// `npx-agent` with variables and `uvx-agent`; `bad-sum` has a wrong checksum.
fn registry(dir: &Path) -> PathBuf {
    let pkg = dir.join("pkg");
    std::fs::create_dir_all(&pkg).expect("pkg dir");
    std::fs::copy(fake_agent(), pkg.join("fake-acp")).expect("copy the agent");
    let mode = std::fs::Permissions::from_mode(0o755);
    std::fs::set_permissions(pkg.join("fake-acp"), mode).expect("chmod");
    let archive = dir.join("fake-acp.tar.gz");
    let tar = Command::new("tar")
        .arg("-czf")
        .arg(&archive)
        .arg("-C")
        .arg(&pkg)
        .arg("fake-acp")
        .status()
        .expect("tar");
    assert!(tar.success());
    let url = format!("file://{}", archive.display());
    let binary =
        |sum: &str| json!({platform(): {"archive": url, "cmd": "./fake-acp", "sha256": sum}});
    let registry = json!({"version": "1.0.0", "agents": [
        {"id": "fake", "version": "0.1.0", "distribution": {"binary": binary(&sha256(&archive))}},
        {"id": "npx-agent", "version": "1.2.3", "distribution": {"npx": {
            "package": "npx-agent@1.2.3", "args": ["--acp"], "env": {"NO_UPDATE": "1"}}}},
        {"id": "uvx-agent", "version": "0.4.0", "distribution": {"uvx": {"package": "uvx-agent==0.4.0"}}},
        {"id": "bad-sum", "version": "0.1.0", "distribution": {"binary": binary("00")}},
    ]});
    let path = dir.join("registry.json");
    std::fs::write(&path, registry.to_string()).expect("write the registry");
    path
}

/// The event lines of terminals labelled `run=<run>` from seq 0 until one matches `last`.
fn events_until(dir: &Path, run: &str, last: impl Fn(&Value) -> bool) -> Vec<String> {
    let mut client = Client::connect(&Paths::in_dir(dir).socket).expect("connect");
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

#[test]
fn registry_installs_pinned_agents_from_a_fixture() {
    let dir = temp_dir("acp-registry");
    let reg = registry(&dir);
    let reg = reg.to_str().expect("utf-8");
    let installed = ok(&dir, &["agents", "add", "fake", "--registry", reg]);
    assert_eq!(installed, "installed fake 0.1.0 (binary)\n");
    let agent = dir.join("agents/fake/0.1.0/fake-acp");
    assert!(agent.is_file(), "{} unpacked", agent.display());
    ok(&dir, &["agents", "add", "npx-agent", "--registry", reg]);
    ok(&dir, &["agents", "add", "uvx-agent", "--registry", reg]);
    let list: Value = serde_json::from_str(&ok(&dir, &["--json", "agents"])).expect("JSON");
    let cmds: Vec<(&str, &Value)> = list
        .as_array()
        .expect("a list")
        .iter()
        .map(|a| (a["id"].as_str().expect("id"), &a["cmd"]))
        .collect();
    assert_eq!(
        cmds,
        [
            ("fake", &json!([agent])),
            (
                "npx-agent",
                &json!([
                    "env",
                    "NO_UPDATE=1",
                    "npx",
                    "-y",
                    "npx-agent@1.2.3",
                    "--acp"
                ])
            ),
            ("uvx-agent", &json!(["uvx", "uvx-agent==0.4.0"])),
        ]
    );

    let again = shed(&dir, &["agents", "add", "fake", "--registry", reg]);
    assert!(!again.status.success());
    let err = String::from_utf8_lossy(&again.stderr);
    assert!(
        err.contains("fake is installed at 0.1.0; rm it first"),
        "{err}"
    );
    let bad = shed(&dir, &["agents", "add", "bad-sum", "--registry", reg]);
    assert!(String::from_utf8_lossy(&bad.stderr).contains("sha256 mismatch"));
    let missing = shed(&dir, &["agents", "add", "nope", "--registry", reg]);
    assert!(String::from_utf8_lossy(&missing.stderr).contains("no agent \"nope\""));

    assert_eq!(
        ok(&dir, &["agents", "rm", "fake"]),
        "removed fake 0.1.0 (binary)\n"
    );
    assert!(!dir.join("agents/fake").exists());
    assert!(ok(&dir, &["agents"]).starts_with("npx-agent 1.2.3 (npx)  env NO_UPDATE=1 npx"));
}

#[test]
fn a_turn_with_an_approval_matches_the_golden_event_log() {
    let dir = temp_dir("acp-golden");
    let _server = Server::start(&dir);
    let reg = registry(&dir);
    ok(
        &dir,
        &[
            "agents",
            "add",
            "fake",
            "--registry",
            reg.to_str().expect("utf-8"),
        ],
    );
    let mut log = String::new();
    let mut step = |args: &[&str]| {
        let out = ok(&dir, args);
        write!(log, "$ shed {}\n{out}", args.join(" ")).expect("write to a String");
    };
    step(&[
        "run",
        "--name",
        "a",
        "--label",
        "run=golden",
        "--acp",
        "fake",
    ]);
    step(&[
        "wait",
        "a",
        "--output",
        "ACP session ready",
        "--timeout",
        "10",
    ]);
    step(&["send", "a", "tool"]);
    step(&["wait", "a", "--until", "blocked", "--timeout", "10"]);
    step(&["approve", "a", "allow-once"]);
    step(&["wait", "a", "--until", "done", "--timeout", "10"]);
    step(&["read", "a"]);
    step(&["send", "a", "hi"]);
    step(&["wait", "a", "--until", "done", "--timeout", "10"]);
    step(&["read", "a", "--lines", "2"]);
    let dones = std::cell::Cell::new(0);
    let events = events_until(&dir, "golden", |e| {
        dones.set(dones.get() + usize::from(is_state("done")(e)));
        dones.get() == 2
    });
    let events = events.join("\n");
    writeln!(log, "$ shed events --label run=golden --since 0\n{events}").expect("write");
    let transcript = std::fs::read_to_string(dir.join("acp/a.jsonl")).expect("the transcript");
    assert_eq!(transcript.lines().count(), 14, "{transcript}");

    let golden = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/acp.txt");
    if std::env::var_os("SHED_BLESS").is_some() {
        std::fs::write(&golden, &log).expect("write golden");
    }
    let have = std::fs::read_to_string(&golden).unwrap_or_default();
    assert_eq!(
        have,
        log,
        "{} differs; SHED_BLESS=1 to update",
        golden.display()
    );
}

#[test]
fn cancel_ends_a_turn_mid_way() {
    let dir = temp_dir("acp-cancel");
    let _server = Server::start(&dir);
    let agent = fake_agent();
    let agent = agent.to_str().expect("utf-8");
    let run = [
        "run",
        "--name",
        "c",
        "--label",
        "run=cancel",
        "--acp",
        "fake",
        "--",
    ];
    ok(&dir, &[&run[..], &["python3", agent]].concat());
    ok(
        &dir,
        &[
            "wait",
            "c",
            "--output",
            "ACP session ready",
            "--timeout",
            "10",
        ],
    );
    for cancel in [&["cancel", "c"][..], &["keys", "c", "esc"][..]] {
        ok(&dir, &["send", "c", "slow"]);
        let seen = std::cell::Cell::new(0);
        events_until(&dir, "cancel", |e| {
            if e["event"] == "text" {
                seen.set(seen.get() + 1);
            }
            e["event"] == "text" && seen.get() == if cancel[0] == "cancel" { 1 } else { 2 }
        });
        ok(&dir, cancel);
        ok(&dir, &["wait", "c", "--until", "done", "--timeout", "10"]);
        let info: Value = serde_json::from_str(&ok(&dir, &["get", "c"])).expect("JSON");
        assert_eq!(
            info["reported"],
            json!({"state": "done", "note": "cancelled"})
        );
    }
    assert_eq!(
        ok(&dir, &["read", "c"]),
        "> slow\nthinking\n> slow\nthinking\n"
    );
}

#[test]
fn the_deny_policy_rejects_permission_requests() {
    let dir = temp_dir("acp-deny");
    let _server = Server::start(&dir);
    let agent = fake_agent();
    let agent = agent.to_str().expect("utf-8");
    let args = [
        "run", "--name", "d", "--label", "run=deny", "--acp", "fake", "--policy", "deny", "--",
    ];
    ok(&dir, &[&args[..], &["python3", agent]].concat());
    ok(
        &dir,
        &[
            "wait",
            "d",
            "--output",
            "ACP session ready",
            "--timeout",
            "10",
        ],
    );
    ok(&dir, &["send", "d", "tool"]);
    ok(&dir, &["wait", "d", "--until", "done", "--timeout", "10"]);
    let events = events_until(&dir, "deny", is_state("done"));
    let kinds: Vec<Value> = events
        .iter()
        .map(|l| serde_json::from_str::<Value>(l).expect("JSON"))
        .filter(|e| e["event"] == "approved" || e["event"] == "state")
        .map(|e| json!([e["event"], e.get("option").or(e.get("state"))]))
        .collect();
    assert_eq!(
        kinds,
        [
            json!(["state", "working"]),
            json!(["approved", "reject-once"]),
            json!(["state", "done"])
        ]
    );
    assert_eq!(
        ok(&dir, &["read", "d"]),
        "> tool\n[tool] Write notes.txt (failed)\ndenied\n"
    );
}

#[test]
fn an_agent_that_fails_the_handshake_reports_error() {
    let dir = temp_dir("acp-error");
    let _server = Server::start(&dir);
    let args = [
        "run", "--name", "e", "--acp", "nope", "--", "sh", "-c", "read x",
    ];
    ok(&dir, &args);
    let out = shed(&dir, &["wait", "e", "--until", "error", "--timeout", "10"]);
    let info: Value = serde_json::from_str(&ok(&dir, &["get", "e"])).expect("JSON");
    assert_eq!(
        info["reported"],
        json!({"state": "error", "note": "acp: the agent closed its stdout"}),
        "{out:?}"
    );
    let missing = shed(&dir, &["run", "--acp", "nope"]);
    let err = String::from_utf8_lossy(&missing.stderr);
    assert!(err.contains("ACP agent nope is not installed"), "{err}");
}

/// Starts the fake agent as terminal `name` (labelled `run=<name>`) under registry id `id`, ready for a
/// prompt.
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

#[test]
fn an_expired_login_ends_the_turn_as_an_auth_error() {
    let dir = temp_dir("acp-auth");
    let _server = Server::start(&dir);
    start_fake(&dir, "x", "claude-acp");
    let line = "Failed to authenticate: OAuth session expired and could not be refreshed";
    let sent = std::time::Instant::now();
    ok(&dir, &["send", "x", "auth"]);
    let until = ["wait", "x", "--until", "done", "blocked", "error"];
    assert_eq!(
        ok(&dir, &[&until[..], &["--timeout", "10"]].concat()),
        "x error\n"
    );
    let took = sent.elapsed();
    assert!(took < std::time::Duration::from_secs(1), "took {took:?}");
    assert_eq!(
        reported(&dir, "x"),
        json!({"state": "error", "note": format!("auth: {line}")})
    );
    let events = events_until(&dir, "x", |e| e["event"] == "reply");
    let reply: Value = serde_json::from_str(events.last().expect("a reply")).expect("JSON");
    assert_eq!(reply["text"], line);
    assert_eq!(reply["stop_reason"], "auth_error");
    // The same text from an agent with no auth patterns is an ordinary chunk; the turn goes on.
    start_fake(&dir, "y", "fake");
    ok(&dir, &["send", "y", "auth"]);
    let out = shed(
        &dir,
        &["wait", "y", "--until", "done", "error", "--timeout", "1"],
    );
    assert_eq!(out.status.code(), Some(124), "{out:?}");
}

#[test]
fn the_watchdog_ends_a_silent_turn_but_not_a_busy_one() {
    let dir = temp_dir("acp-idle");
    let env = [("SHED_TURN_IDLE_TIMEOUT", "1")];
    let _server = Server::with_env(&dir, "3600", &env);
    start_fake(&dir, "w", "fake");
    let until = [
        "wait",
        "w",
        "--until",
        "done",
        "blocked",
        "error",
        "--timeout",
        "10",
    ];
    ok(&dir, &["send", "w", "busy"]);
    assert_eq!(ok(&dir, &until), "w done\n");
    assert_eq!(
        reported(&dir, "w"),
        json!({"state": "done", "note": "end_turn"})
    );
    ok(&dir, &["send", "w", "silent"]);
    assert_eq!(ok(&dir, &until), "w error\n");
    assert_eq!(
        reported(&dir, "w"),
        json!({"state": "error", "note": "no response"})
    );
    assert_eq!(
        ok(&dir, &["read", "w"]),
        "> busy\n[tool] Build (completed)\nbusy done\n> silent\n"
    );
}

/// Starts the fake agent as terminal `name` keeping its history in `history`, `extra` passed to `run`.
fn start_with_history(dir: &Path, name: &str, history: &Path, extra: &[&str]) {
    let agent = fake_agent();
    let label = format!("run={name}");
    let run = ["run", "--name", name, "--label", &label, "--acp", "fake"];
    let cmd = ["--", "python3", agent.to_str().expect("utf-8"), "--history"];
    let history = history.to_str().expect("utf-8");
    ok(dir, &[&run[..], extra, &cmd, &[history]].concat());
}

#[test]
fn resume_replays_the_history_once_marked_replay() {
    let dir = temp_dir("acp-resume");
    let history = dir.join("history.jsonl");
    let server = Server::start(&dir);
    start_with_history(&dir, "r1", &history, &[]);
    ok(
        &dir,
        &[
            "wait",
            "r1",
            "--output",
            "ACP session ready",
            "--timeout",
            "10",
        ],
    );
    for prompt in ["hi", "again"] {
        ok(&dir, &["send", "r1", prompt]);
        ok(&dir, &["wait", "r1", "--until", "done", "--timeout", "10"]);
    }
    // A server restart loses the terminal but keeps its session id.
    drop(server);
    let _server = Server::start(&dir);
    let info: Value = serde_json::from_str(&ok(&dir, &["get", "r1"])).expect("JSON");
    assert_eq!(
        (&info["state"], &info["session"]["id"]),
        (&json!("exited"), &json!("sess-1"))
    );

    start_with_history(&dir, "r2", &history, &["--resume", "sess-1"]);
    ok(
        &dir,
        &[
            "wait",
            "r2",
            "--output",
            "ACP session ready",
            "--timeout",
            "10",
        ],
    );
    ok(&dir, &["send", "r2", "more"]);
    ok(&dir, &["wait", "r2", "--until", "done", "--timeout", "10"]);
    let events: Vec<Value> = events_until(&dir, "r2", is_state("done"))
        .iter()
        .map(|l| serde_json::from_str(l).expect("JSON"))
        .collect();
    let acp: Vec<Value> = events
        .iter()
        .filter(|e| ["turn", "reply"].contains(&e["event"].as_str().unwrap_or_default()))
        .map(|e| {
            json!([
                e["event"],
                e.get("prompt").or(e.get("text")),
                e.get("replay")
            ])
        })
        .collect();
    assert_eq!(
        acp,
        [
            json!(["turn", "hi", true]),
            json!(["reply", "you said: hi", true]),
            json!(["turn", "again", true]),
            json!(["reply", "you said: again", true]),
            json!(["turn", "more", null]),
            json!(["reply", "you said: more", null]),
        ]
    );
    assert_eq!(
        ok(&dir, &["read", "r2"]),
        "> hi\nyou said: hi\n> again\nyou said: again\n> more\nyou said: more\n"
    );
}

#[test]
fn resume_is_refused_without_load_session() {
    let dir = temp_dir("acp-noload");
    let _server = Server::start(&dir);
    let agent = fake_agent();
    let agent = agent.to_str().expect("utf-8");
    let run = [
        "run", "--name", "n", "--acp", "fake", "--resume", "sess-1", "--",
    ];
    ok(&dir, &[&run[..], &["python3", agent]].concat());
    ok(&dir, &["wait", "n", "--until", "error", "--timeout", "10"]);
    assert_eq!(
        reported(&dir, "n"),
        json!({"state": "error", "note": "acp: the agent cannot resume a session (no loadSession)"})
    );
}

#[test]
fn a_model_switch_is_reported_and_the_next_turn_uses_it() {
    let dir = temp_dir("acp-model");
    let _server = Server::start(&dir);
    start_fake(&dir, "m", "fake");
    let session = || {
        let info: Value = serde_json::from_str(&ok(&dir, &["get", "m"])).expect("JSON");
        info["session"].clone()
    };
    let models = json!([["fast", "Fast"], ["smart", "Smart"]]);
    let first = json!({"id": "sess-1", "model": "fast", "models": models});
    common::eventually("the session", || session() == first);
    ok(&dir, &["model", "m", "smart"]);
    common::eventually("the switch", || session()["model"] == "smart");
    ok(&dir, &["send", "m", "hi"]);
    ok(&dir, &["wait", "m", "--until", "done", "--timeout", "10"]);
    assert_eq!(
        ok(&dir, &["read", "m", "--lines", "1"]),
        "[smart] you said: hi\n"
    );
}

#[test]
fn an_unknown_update_kind_is_skipped() {
    let dir = temp_dir("acp-unknown");
    let _server = Server::start(&dir);
    start_fake(&dir, "u", "fake");
    ok(&dir, &["send", "u", "unknown"]);
    ok(&dir, &["wait", "u", "--until", "done", "--timeout", "10"]);
    let logged = [
        "wait",
        "u",
        "--output",
        "skipped a session/update",
        "--timeout",
        "1",
    ];
    assert_eq!(ok(&dir, &logged), "u output\n");
    assert_eq!(ok(&dir, &["read", "u"]), "> unknown\nstill here\n");
    ok(&dir, &["send", "u", "hi"]);
    ok(&dir, &["wait", "u", "--until", "done", "--timeout", "10"]);
    assert_eq!(
        reported(&dir, "u"),
        json!({"state": "done", "note": "end_turn"})
    );
}

/// The `--mcp` file for `servers`, in a dir of its own (not shed's).
fn mcp_file(name: &str, servers: &Value) -> PathBuf {
    let path = temp_dir(name).join("mcp.json");
    std::fs::write(&path, servers.to_string()).expect("write the MCP servers");
    path
}

/// Starts the fake agent as terminal `name` (labelled `run=mcp`) with `run`'s `--mcp file` and
/// `extra`, the fake checking it gets exactly `file`'s servers.
fn start_with_mcp(dir: &Path, name: &str, file: &Path, extra: &[&str], fake: &[&str]) {
    let (agent, file) = (fake_agent(), file.to_str().expect("utf-8"));
    let run = ["run", "--name", name, "--label", "run=mcp", "--acp", "fake"];
    let cmd = [
        "--",
        "python3",
        agent.to_str().expect("utf-8"),
        "--mcp",
        file,
    ];
    ok(
        dir,
        &[&run[..], &["--mcp", file], extra, &cmd, fake].concat(),
    );
}

const SECRET: &str = "s3cret-mcp-value";

#[test]
fn mcp_servers_pass_through_on_new_and_resume_with_their_values_hidden() {
    let dir = temp_dir("acp-mcp");
    let history = dir.join("history.jsonl");
    let history = history.to_str().expect("utf-8");
    let servers = |value: &str| {
        json!([
            {"name": "files", "command": "/bin/echo", "args": ["hi"],
             "env": [{"name": "TOKEN", "value": value}]},
            {"type": "http", "name": "web", "url": "https://mcp.example/",
             "headers": [{"name": "Authorization", "value": value}]},
        ])
    };
    let file = mcp_file("acp-mcp-in", &servers(SECRET));
    let _server = Server::start(&dir);
    let fake = ["--history", history, "--mcp-caps", "http"];
    let ready = |name| {
        [
            "wait",
            name,
            "--output",
            "ACP session ready",
            "--timeout",
            "10",
        ]
    };
    let turn = |name, prompt| {
        ok(&dir, &["send", name, prompt]);
        ok(&dir, &["wait", name, "--until", "done", "--timeout", "10"]);
    };
    start_with_mcp(&dir, "m1", &file, &[], &fake);
    ok(&dir, &ready("m1"));
    turn("m1", "hi");
    start_with_mcp(&dir, "m2", &file, &["--resume", "sess-1"], &fake);
    ok(&dir, &ready("m2"));
    turn("m2", "more");
    let info: Value = serde_json::from_str(&ok(&dir, &["get", "m2"])).expect("JSON");
    assert_eq!(info["session"]["mcp_servers"], servers("***"));

    let events = events_until(&dir, "mcp", |e| e["name"] == "m2" && is_state("done")(e));
    let mut shown = events.join("\n");
    for name in ["m1", "m2"] {
        shown.push_str(&ok(&dir, &["get", name]));
    }
    assert_hidden(&dir, shown);
}

/// `SECRET` is in neither `shown` (which has the redacted servers) nor `ls`, nor any file under `dir`.
fn assert_hidden(dir: &Path, mut shown: String) {
    shown.push_str(&ok(dir, &["ls", "--json"]));
    assert!(shown.contains("TOKEN") && shown.contains("***"), "{shown}");
    assert!(!shown.contains(SECRET), "{shown}");
    let mut dirs = vec![dir.to_owned()];
    while let Some(d) = dirs.pop() {
        for entry in std::fs::read_dir(&d).expect("read the state dir") {
            let path = entry.expect("an entry").path();
            if path.is_dir() {
                dirs.push(path);
            } else if let Ok(bytes) = std::fs::read(&path) {
                let text = String::from_utf8_lossy(&bytes);
                assert!(!text.contains(SECRET), "{} has the secret", path.display());
            }
        }
    }
}

#[test]
fn sse_servers_pass_only_when_the_agent_advertises_them() {
    let dir = temp_dir("acp-mcp-sse");
    let _server = Server::start(&dir);
    let servers = json!([{"type": "sse", "name": "events", "url": "https://mcp.example/sse",
                          "headers": []}]);
    let file = mcp_file("acp-mcp-sse-in", &servers);
    start_with_mcp(&dir, "s1", &file, &[], &["--mcp-caps", "sse"]);
    ok(
        &dir,
        &[
            "wait",
            "s1",
            "--output",
            "ACP session ready",
            "--timeout",
            "10",
        ],
    );
    start_with_mcp(&dir, "s2", &file, &[], &["--mcp-caps", "http"]);
    ok(&dir, &["wait", "s2", "--until", "error", "--timeout", "10"]);
    let why = "acp: MCP server events: the agent takes no sse servers (mcpCapabilities)";
    assert_eq!(reported(&dir, "s2"), json!({"state": "error", "note": why}));
}

#[test]
fn unknown_or_malformed_mcp_servers_are_refused() {
    let dir = temp_dir("acp-mcp-bad");
    let _server = Server::start(&dir);
    let agent = fake_agent();
    let bad = [
        (
            json!([{"type": "ws", "name": "x", "command": "c"}]),
            r#"mcp_servers[0] "x""#,
        ),
        (
            json!([{"name": "f", "command": "c", "args": [], "env": []}, {"name": "y"}]),
            r#"mcp_servers[1] "y""#,
        ),
        (json!({"name": "z"}), "not a JSON list"),
    ];
    for (servers, why) in bad {
        let file = mcp_file("acp-mcp-bad-in", &servers);
        let file = file.to_str().expect("utf-8");
        let run = ["run", "--acp", "fake", "--mcp", file, "--", "python3"];
        let out = shed(
            &dir,
            &[&run[..], &[agent.to_str().expect("utf-8")]].concat(),
        );
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(
            !out.status.success() && err.contains(why),
            "{servers}: {err}"
        );
    }
    let ls: Value = serde_json::from_str(&ok(&dir, &["ls", "--json"])).expect("JSON");
    assert_eq!(ls, json!([]));
}

#[test]
fn each_change_of_the_slash_commands_is_one_session_event() {
    let dir = temp_dir("acp-commands");
    let _server = Server::start(&dir);
    start_fake(&dir, "k", "fake");
    ok(&dir, &["send", "k", "commands"]);
    ok(&dir, &["wait", "k", "--until", "done", "--timeout", "10"]);
    let commands: Vec<Value> = events_until(&dir, "k", is_state("done"))
        .iter()
        .map(|l| serde_json::from_str::<Value>(l).expect("JSON"))
        .filter(|e| e["event"] == "session")
        .map(|e| e["commands"].clone())
        .collect();
    let plan = json!(["plan", "Make a plan", ""]);
    let both = json!([plan, ["review", "Review the diff", "files"]]);
    assert_eq!(commands, [Value::Null, json!([plan]), both.clone()]);
    let info: Value = serde_json::from_str(&ok(&dir, &["get", "k"])).expect("JSON");
    assert_eq!(info["session"]["commands"], both);
    assert_eq!(ok(&dir, &["read", "k"]), "> commands\ncommands sent\n");
}

/// A server whose idle ACP agents hibernate after a second.
fn sleepy_server(dir: &Path) -> Server {
    Server::with_env(dir, "3600", &[("SHED_ACP_HIBERNATE_AFTER", "1")])
}

/// The live processes of terminal `name`: its bridge, plus the agent's while it is awake.
fn processes(dir: &Path, name: &str) -> usize {
    let info: Value = serde_json::from_str(&ok(dir, &["get", name])).expect("JSON");
    let pid = i32::try_from(info["pid"].as_i64().expect("a pid")).expect("a pid");
    let procs = proctree::system().list().expect("the process table");
    proctree::tree(&procs, pid, &std::collections::BTreeSet::new()).len()
}

/// Waits for `state` of terminal `name`.
fn until(dir: &Path, name: &str, state: &str) {
    let wait = ["wait", name, "--until", state, "--timeout", "10"];
    assert_eq!(ok(dir, &wait), format!("{name} {state}\n"));
}

/// Twice the hibernation delay, to show an agent that stays awake.
fn past_the_delay() {
    std::thread::sleep(std::time::Duration::from_millis(2500));
}

#[test]
fn an_idle_agent_hibernates_but_not_mid_turn_or_while_a_request_waits() {
    let dir = temp_dir("acp-hibernate");
    let history = dir.join("history.jsonl");
    let _server = sleepy_server(&dir);
    start_with_history(&dir, "h", &history, &[]);
    until(&dir, "h", "hibernated");
    assert_eq!(processes(&dir, "h"), 1, "the agent is gone");
    assert_eq!(reported(&dir, "h"), json!({"state": "hibernated"}));
    let info: Value = serde_json::from_str(&ok(&dir, &["get", "h"])).expect("JSON");
    assert_eq!(
        (&info["state"], &info["session"]["id"]),
        (&json!("running"), &json!("sess-1"))
    );
    // A turn that hears nothing for longer than the delay, and a permission request, keep it awake.
    ok(&dir, &["send", "h", "slow"]);
    past_the_delay();
    assert_eq!(reported(&dir, "h")["state"], "working");
    assert_eq!(processes(&dir, "h"), 2);
    ok(&dir, &["cancel", "h"]);
    until(&dir, "h", "done");
    ok(&dir, &["send", "h", "tool"]);
    until(&dir, "h", "blocked");
    past_the_delay();
    assert_eq!(reported(&dir, "h")["state"], "blocked");
    assert_eq!(processes(&dir, "h"), 2);
    ok(&dir, &["approve", "h", "allow-once"]);
    until(&dir, "h", "done");
    until(&dir, "h", "hibernated");
    assert_eq!(processes(&dir, "h"), 1);
}

#[test]
fn a_prompt_wakes_the_agent_and_its_history_is_not_repeated() {
    let dir = temp_dir("acp-wake");
    let history = dir.join("history.jsonl");
    let _server = sleepy_server(&dir);
    start_with_history(&dir, "h", &history, &[]);
    ok(&dir, &["send", "h", "hi"]);
    until(&dir, "h", "done");
    until(&dir, "h", "hibernated");
    ok(&dir, &["send", "h", "more"]);
    until(&dir, "h", "done");
    assert_eq!(processes(&dir, "h"), 2, "awake again");
    assert_eq!(
        ok(&dir, &["read", "h"]),
        "> hi\nyou said: hi\n> more\nyou said: more\n"
    );
    let events = events_until(&dir, "h", |e| {
        e["event"] == "reply" && e["text"] == "you said: more"
    });
    let acp: Vec<Value> = events
        .iter()
        .map(|l| serde_json::from_str::<Value>(l).expect("JSON"))
        .filter(|e| ["turn", "reply"].contains(&e["event"].as_str().unwrap_or_default()))
        .map(|e| {
            json!([
                e["event"],
                e.get("prompt").or(e.get("text")),
                e.get("replay")
            ])
        })
        .collect();
    assert_eq!(
        acp,
        [
            json!(["turn", "hi", null]),
            json!(["reply", "you said: hi", null]),
            json!(["turn", "hi", true]),
            json!(["reply", "you said: hi", true]),
            json!(["turn", "more", null]),
            json!(["reply", "you said: more", null]),
        ]
    );
}

#[test]
fn sends_during_a_wake_are_delivered_in_order() {
    let dir = temp_dir("acp-wake-order");
    let history = dir.join("history.jsonl");
    let _server = sleepy_server(&dir);
    start_with_history(&dir, "h", &history, &[]);
    until(&dir, "h", "hibernated");
    // The model switch wakes the agent; both prompts queue behind it.
    ok(&dir, &["model", "h", "smart"]);
    ok(&dir, &["send", "h", "one"]);
    ok(&dir, &["send", "h", "two"]);
    events_until(&dir, "h", |e| {
        e["event"] == "reply" && e["text"] == "[smart] you said: two"
    });
    assert_eq!(
        ok(&dir, &["read", "h"]),
        "> one\n> two\n[smart] you said: one\n[smart] you said: two\n"
    );
}

#[test]
fn no_hibernation_without_load_session_or_with_no_hibernate() {
    let dir = temp_dir("acp-awake");
    let history = dir.join("history.jsonl");
    let _server = sleepy_server(&dir);
    start_fake(&dir, "a", "fake");
    start_with_history(&dir, "b", &history, &["--no-hibernate"]);
    for name in ["a", "b"] {
        ok(&dir, &["send", name, "hi"]);
        until(&dir, name, "done");
    }
    past_the_delay();
    for name in ["a", "b"] {
        assert_eq!(reported(&dir, name)["state"], "done", "{name}");
        assert_eq!(processes(&dir, name), 2, "{name}");
    }
}

#[test]
fn mcp_secrets_stay_in_memory_through_a_hibernation() {
    let dir = temp_dir("acp-hibernate-mcp");
    let history = dir.join("history.jsonl");
    let history = history.to_str().expect("utf-8");
    let servers = json!([{"name": "files", "command": "/bin/echo", "args": [],
                          "env": [{"name": "TOKEN", "value": SECRET}]}]);
    let file = mcp_file("acp-hibernate-mcp-in", &servers);
    let _server = sleepy_server(&dir);
    start_with_mcp(&dir, "m", &file, &[], &["--history", history]);
    until(&dir, "m", "hibernated");
    // The fake agent exits unless session/load brings the same servers, secret and all.
    ok(&dir, &["send", "m", "hi"]);
    until(&dir, "m", "done");
    assert_eq!(ok(&dir, &["read", "m"]), "> hi\nyou said: hi\n");
    let events = events_until(&dir, "mcp", |e| e["event"] == "reply");
    assert_hidden(&dir, events.join("\n") + &ok(&dir, &["get", "m"]));
}
