//! herdsman's fixed bug classes that apply to shed (herdsman#553, "Reliability"), one test per row.
#![cfg(test)]

mod common;

use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use common::{Server, eventually, temp_dir};
use serde_json::{Value, json};
use shed_server::{Client, Error, Paths};

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
    eventually("the server to exit", || !alive(&servers[0]));
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

struct Limited(Child);

impl Limited {
    fn serve(dir: &Path, ulimit: &str) -> Self {
        let script = format!("ulimit {ulimit} && exec \"$0\" serve --sample 3600 --dir \"$1\"");
        let child = Command::new("/bin/sh")
            .args(["-c", &script, env!("CARGO_BIN_EXE_shed")])
            .arg(dir)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("shed serve starts");
        let server = Self(child);
        let socket = Paths::in_dir(dir).socket;
        eventually("the socket", || UnixStream::connect(&socket).is_ok());
        server
    }
}

impl Drop for Limited {
    fn drop(&mut self) {
        self.0.kill().ok();
        self.0.wait().ok();
    }
}

fn connect(dir: &Path) -> Client {
    Client::connect(&Paths::in_dir(dir).socket).expect("connect")
}

fn spawn(client: &mut Client, dir: &Path, name: &str, cmd: &[&str]) -> Result<Value, Error> {
    client.call(
        shed_server::proto::Method::Run,
        &json!({"name": name, "cwd": dir, "cmd": cmd}),
    )
}

fn list(client: &mut Client) -> Vec<Value> {
    client
        .call(shed_server::proto::Method::Ls, &json!({}))
        .expect("ls")
}

fn running_count(client: &mut Client) -> usize {
    list(client)
        .iter()
        .filter(|t| t["state"] == "running")
        .count()
}

#[test]
fn serve_raises_a_lowered_soft_limit_so_many_sessions_spawn() {
    let dir = temp_dir("nofile-raise");
    let _server = Limited::serve(&dir, "-S -n 64");
    let mut c = connect(&dir);
    for i in 0..100 {
        let name = format!("s{i}");
        if let Err(e) = spawn(&mut c, &dir, &name, &["cat"]) {
            panic!("spawn {name} under a soft limit of 64: {e}");
        }
    }
    assert_eq!(running_count(&mut c), 100);
}

#[test]
fn emfile_fails_the_spawn_and_the_daemon_and_other_sessions_live_on() {
    let dir = temp_dir("nofile-emfile");
    let _server = Limited::serve(&dir, "-n 64");
    let mut c = connect(&dir);
    let mut spawned = 0;
    let failed = loop {
        match spawn(&mut c, &dir, &format!("s{spawned}"), &["cat"]) {
            Ok(_) => spawned += 1,
            Err(e) => break e.to_string(),
        }
        assert!(spawned < 64, "64 sessions spawned under a limit of 64 fds");
    };
    assert!(
        failed.contains("SPAWN_FAILED") || failed.contains("Too many open files"),
        "{failed}"
    );
    let socket = Paths::in_dir(&dir).socket;
    let held: Vec<UnixStream> = (0..8)
        .filter_map(|_| UnixStream::connect(&socket).ok())
        .collect();
    assert_eq!(
        running_count(&mut c),
        spawned,
        "the daemon and every session live"
    );
    drop(held);
    for i in 0..4 {
        let target = json!({"name": format!("s{i}")});
        let _: Value = c
            .call(shed_server::proto::Method::Kill, &target)
            .expect("kill");
    }
    eventually("the killed sessions exited", || {
        running_count(&mut c) == spawned - 4
    });
    assert_eq!(running_count(&mut connect(&dir)), spawned - 4);
}

#[test]
fn exited_sessions_release_their_fds() {
    let dir = temp_dir("nofile-leak");
    let server = Limited::serve(&dir, "-n 64");
    let fds = Path::new("/proc")
        .join(server.0.id().to_string())
        .join("fd");
    let count = || std::fs::read_dir(&fds).map(Iterator::count).ok();
    let mut c = connect(&dir);
    let baseline = count();
    for i in 0..200 {
        let name = format!("s{i}");
        spawn(&mut c, &dir, &name, &["true"]).unwrap_or_else(|e| panic!("cycle {i}: {e}"));
        eventually("the session exited", || {
            let list = list(&mut c);
            list.iter()
                .any(|t| t["name"] == name && t["state"] == "exited")
        });
    }
    eventually("the fd count back to its baseline", || count() <= baseline);
}

struct Serve(Child);

impl Serve {
    fn start(dir: &Path, prelude: &str, stderr: Stdio) -> Self {
        let script = format!("{prelude} exec \"$0\" serve --sample 3600 --dir \"$1\"");
        let child = Command::new("sh")
            .args(["-c", &script, SHED])
            .arg(dir)
            .env("SHELL", "/bin/sh")
            .stdout(Stdio::null())
            .stderr(stderr)
            .spawn()
            .expect("shed serve");
        let server = Self(child);
        let socket = Paths::in_dir(dir).socket;
        eventually("the socket", || UnixStream::connect(&socket).is_ok());
        server
    }

    fn signal(&self, sig: &str) {
        let pid = self.0.id().to_string();
        let sent = Command::new("kill").args([sig, &pid]).status();
        assert!(sent.expect("kill").success());
    }
}

impl Drop for Serve {
    fn drop(&mut self) {
        self.0.kill().ok();
        self.0.wait().ok();
    }
}

fn survivors(trees: &[(String, String)], within: Duration) -> Vec<String> {
    let deadline = Instant::now() + within;
    loop {
        let live: Vec<String> = trees
            .iter()
            .flat_map(|(pid, child)| [pid, child])
            .filter(|p| alive(p))
            .cloned()
            .collect();
        if live.is_empty() || Instant::now() > deadline {
            for pid in &live {
                Command::new("kill").args(["-KILL", pid]).status().ok();
            }
            return live;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn sigterm_to_serve_ends_every_tree_even_one_that_ignores_sighup() {
    let dir = temp_dir("rel-sigterm");
    let mut server = Serve::start(&dir, "", Stdio::null());
    run_sh(&dir, "a", "sleep 100 & echo child=$!; wait");
    run_sh(&dir, "b", "sleep 100 & echo child=$!; wait");
    run_sh(&dir, "deaf", "trap '' HUP; sleep 100 & echo child=$!; wait");
    let trees = ["a", "b", "deaf"].map(|n| pids(&dir, n));
    server.signal("-TERM");
    let left = survivors(&trees, Duration::from_secs(3));
    assert!(left.is_empty(), "alive 3 s after SIGTERM: {left:?}");
    server.0.wait().expect("serve exits");
}

#[test]
fn a_program_gets_default_signals_from_a_serve_that_ignores_sighup() {
    let dir = temp_dir("rel-sigdfl");
    let _server = Serve::start(&dir, "trap '' HUP USR1;", Stdio::null());
    let d = dir.to_str().expect("utf-8 dir");
    let script = "import signal as s, time\n\
        print('hup', s.getsignal(s.SIGHUP).name, 'usr1', s.getsignal(s.SIGUSR1).name, \
        'blocked', len(s.pthread_sigmask(s.SIG_BLOCK, [])), flush=True)\n\
        time.sleep(60)";
    ok(
        &dir,
        &[
            "run", "--name", "p", "--cwd", d, "--", "python3", "-c", script,
        ],
    );
    let mut line = String::new();
    eventually("the program's dispositions", || {
        let screen = ok(&dir, &["read", "p"]);
        let found = screen.lines().find(|l| l.starts_with("hup "));
        found.unwrap_or_default().clone_into(&mut line);
        !line.is_empty()
    });
    assert_eq!(line.trim_end(), "hup SIG_DFL usr1 SIG_DFL blocked 0");
}

#[test]
fn the_next_start_ends_a_tree_a_killed_serve_left_behind() {
    let dir = temp_dir("rel-left");
    let first = Serve::start(&dir, "", Stdio::null());
    run_sh(&dir, "deaf", "trap '' HUP; sleep 100 & echo child=$!; wait");
    let tree = pids(&dir, "deaf");
    drop(first);
    assert!(alive(&tree.0) && alive(&tree.1), "left behind: {tree:?}");
    let log = dir.join("serve.log");
    let err = std::fs::File::create(&log).expect("log file");
    let _second = Serve::start(&dir, "", Stdio::from(err));
    let left = survivors(std::slice::from_ref(&tree), Duration::from_secs(3));
    assert!(left.is_empty(), "alive after the next start: {left:?}");
    let text = std::fs::read_to_string(&log).expect("log");
    let named: Vec<&str> = text.lines().filter(|l| l.contains("deaf")).collect();
    assert_eq!(named.len(), 1, "{text}");
    assert!(named.iter().all(|l| l.contains(&tree.0)), "{text}");
}
