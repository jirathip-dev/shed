//! The CLI end to end, against a real server.
//!
//! The human and JSON output is checked against `tests/golden/cli.txt` (the temp dir and pids
//! normalised). Regenerate with `SHED_BLESS=1 cargo test -p shed`.
#![cfg(test)]

mod common;

use std::fmt::Write as _;
use std::io::{BufRead as _, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use common::{Server, eventually, temp_dir};
use serde_json::{Value, json};
use shed_core::{Size, Terminal};

const SHED: &str = env!("CARGO_BIN_EXE_shed");

fn shed(args: &[&str]) -> String {
    let out = Command::new(SHED).args(args).output().expect("shed runs");
    assert!(out.status.success(), "shed {args:?} failed: {out:?}");
    String::from_utf8(out.stdout).expect("utf-8 stdout")
}

fn shed_in(dir: &Path, args: &[&str]) -> Output {
    Command::new(SHED)
        .arg("--dir")
        .arg(dir)
        .args(args)
        .output()
        .expect("shed runs")
}

fn stdout(out: &Output) -> String {
    assert!(out.status.success(), "shed failed: {out:?}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn zero_pids(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(at) = rest.find("\"pid\":") {
        let (head, tail) = rest.split_at(at + 6);
        out.push_str(head);
        let spaces = tail.len() - tail.trim_start_matches(' ').len();
        let (gap, tail) = tail.split_at(spaces);
        out.push_str(gap);
        let digits = tail.len() - tail.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        out.push_str(if digits > 0 { "0" } else { "" });
        rest = &tail[digits..];
    }
    out + rest
}

struct Transcript {
    dir: PathBuf,
    text: String,
    golden: &'static str,
}

impl Transcript {
    fn run(&mut self, args: &[&str]) -> Output {
        let out = shed_in(&self.dir, args);
        let dir = self.dir.to_str().expect("utf-8 dir");
        let mut text = format!(
            "$ shed {}\n{}",
            args.join(" "),
            String::from_utf8_lossy(&out.stdout)
        );
        if !out.status.success() {
            let code = out.status.code().unwrap_or(-1);
            let err = String::from_utf8_lossy(&out.stderr);
            let err = if err.is_empty() { "\n".into() } else { err };
            write!(text, "[exit {code}] {err}").expect("write to a String");
        }
        let text = text.replace(SHED, "shed").replace(dir, "$DIR");
        self.text.push_str(&zero_pids(&text));
        out
    }

    fn check(&self) {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/golden")
            .join(self.golden);
        if std::env::var_os("SHED_BLESS").is_some() {
            std::fs::write(&path, &self.text).expect("write golden");
        }
        let have = std::fs::read_to_string(&path).unwrap_or_default();
        assert_eq!(
            have,
            self.text,
            "{} differs; SHED_BLESS=1 to update",
            path.display()
        );
    }
}

#[test]
fn hello_prints_the_version() {
    assert_eq!(
        shed(&["hello"]),
        format!("shed {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn schema_prints_the_committed_openrpc_document() {
    let committed = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/shed.openrpc.json");
    let committed = std::fs::read_to_string(committed).expect("docs/shed.openrpc.json");
    assert_eq!(shed(&["schema"]), committed);
}

#[test]
fn version_flag_prints_the_version() {
    assert_eq!(
        shed(&["--version"]),
        format!("shed {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn help_lists_every_command() {
    let help = shed(&["--help"]);
    for cmd in [
        "hello", "schema", "serve", "run", "ls", "get", "read", "send", "keys", "kill", "rm",
        "attach", "state", "wait", "events", "top", "approve", "cancel", "agents", "connect",
        "machine",
    ] {
        assert!(
            help.contains(&format!("\n  {cmd} ")),
            "{cmd} missing from {help}"
        );
    }
}

#[test]
fn run_read_send_exit_ls_and_labels_match_the_golden() {
    let dir = temp_dir("cli");
    let d = dir.to_str().expect("utf-8 dir");
    let mut t = Transcript {
        dir: dir.clone(),
        text: String::new(),
        golden: "cli.txt",
    };
    t.run(&["ls"]);
    t.run(&["run", "--cwd", d, "--", "true"]);
    let _server = Server::start(&dir);

    let run = [
        "run",
        "--name",
        "shell",
        "--label",
        "role=impl",
        "--label",
        "issue=7",
    ];
    t.run(&[&run[..], &["--cwd", d, "--", "sh"]].concat());
    eventually("the prompt", || {
        stdout(&shed_in(&dir, &["read", "shell"])) == "$\n"
    });
    t.run(&["read", "shell"]);
    t.run(&["--json", "read", "shell", "--lines", "1"]);
    t.run(&["send", "shell", "exit"]);
    t.run(&["keys", "shell", "enter"]);
    eventually("the shell to exit", || {
        stdout(&shed_in(&dir, &["get", "shell"])).contains("\"exited\"")
    });
    t.run(&["ls"]);

    let rev = ["run", "--name", "rev", "--label", "role=rev", "--cwd", d];
    t.run(&[&["--json"], &rev[..], &["--", "sleep", "100"]].concat());
    t.run(&["kill", "rev"]);
    t.run(&["ls", "--label", "role=impl"]);
    t.run(&["ls", "--label", "role=impl", "--label", "issue=8"]);
    t.run(&["--json", "ls", "--label", "role=rev"]);
    t.run(&["get", "shell"]);
    t.run(&["--json", "get", "shell"]);
    t.run(&["top"]);
    t.run(&["rm", "shell"]);
    t.run(&["--json", "rm", "rev"]);
    t.run(&["ls"]);
    t.run(&["keys", "rev", "enter"]);
    t.run(&["run", "--label", "nolabel", "--", "true"]);
    t.check();
}

#[test]
fn a_fake_agent_reports_by_escape_and_shed_state_and_wait_returns_on_each() {
    let dir = temp_dir("state");
    let d = dir.to_str().expect("utf-8 dir");
    let _server = Server::start(&dir);
    let mut t = Transcript {
        dir: dir.clone(),
        text: String::new(),
        golden: "state.txt",
    };
    let agent = format!(
        "printf 'start\\033]7501;state=working;note=step 1\\007end\\n'; read x; \
         '{SHED}' --dir '{d}' state done --note 'all green'; read y"
    );
    t.run(&[
        "run", "--name", "agent", "--cwd", d, "--", "sh", "-c", &agent,
    ]);
    t.run(&[
        "wait",
        "agent",
        "--until",
        "done",
        "working",
        "--timeout",
        "10",
    ]);
    t.run(&["read", "agent"]);
    t.run(&["read", "agent", "--history"]);
    t.run(&["send", "agent", "\n"]);
    t.run(&[
        "--json",
        "wait",
        "agent",
        "--until",
        "done",
        "--timeout",
        "10",
    ]);
    t.run(&["wait", "agent", "--until", "blocked", "--timeout", "0.2"]);
    t.run(&["state", "agent", "blocked", "--note", "needs review"]);
    t.run(&["--json", "get", "agent"]);
    t.run(&["state", "agent", "sleeping"]);
    t.run(&["send", "agent", "\n"]);
    t.run(&["wait", "agent", "--until", "working"]);
    t.check();
}

#[test]
fn wait_matches_output_by_pattern_and_falls_back_to_idle() {
    let dir = temp_dir("wait-output");
    let d = dir.to_str().expect("utf-8 dir");
    let _server = Server::start(&dir);
    let script = "sleep 0.3; echo 'ok 12 tests'; exec sleep 100";
    stdout(&shed_in(
        &dir,
        &[
            "run", "--name", "test", "--cwd", d, "--", "sh", "-c", script,
        ],
    ));
    let started = Instant::now();
    let out = shed_in(
        &dir,
        &[
            "wait",
            "test",
            "--output",
            "^ok \\d+ tests$",
            "--timeout",
            "10",
        ],
    );
    assert_eq!(stdout(&out), "test output\n");
    assert!(started.elapsed() < Duration::from_secs(5));

    let idle = [
        "run",
        "--name",
        "quiet",
        "--idle-after",
        "1",
        "--cwd",
        d,
        "--",
        "sleep",
        "100",
    ];
    stdout(&shed_in(&dir, &idle));
    let out = shed_in(
        &dir,
        &["wait", "quiet", "--until", "idle", "--timeout", "10"],
    );
    assert_eq!(stdout(&out), "quiet idle\n");
    let out = shed_in(
        &dir,
        &["wait", "test", "--until", "idle", "--timeout", "1.2"],
    );
    assert_eq!(
        out.status.code(),
        Some(124),
        "no fallback unless asked: {out:?}"
    );
}

#[test]
fn attach_types_resizes_and_detaches_in_a_pty() {
    let dir = temp_dir("attach");
    let d = dir.to_str().expect("utf-8 dir");
    let _server = Server::start(&dir);
    stdout(&shed_in(
        &dir,
        &["run", "--name", "shell", "--cwd", d, "--", "sh"],
    ));
    eventually("the prompt", || {
        stdout(&shed_in(&dir, &["read", "shell"])) == "$\n"
    });

    let size = Size { rows: 24, cols: 80 };
    let cmd = [SHED, "--dir", d, "attach", "shell"];
    let outer = Terminal::spawn(&cmd, &dir, [("TERM", "xterm")], size).expect("attach in a pty");
    eventually("the redraw", || outer.screen().starts_with('$'));
    outer.write(b"echo hi-$((1+2))\r").expect("type");
    eventually("typed input to run", || outer.screen().contains("hi-3"));

    outer
        .resize(Size {
            rows: 30,
            cols: 100,
        })
        .expect("resize");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !outer.screen().contains("30 100") {
        assert!(
            Instant::now() < deadline,
            "resize not forwarded: {}",
            outer.screen()
        );
        outer.write(b"stty size\r").expect("type");
        std::thread::sleep(Duration::from_millis(300));
    }

    outer.write(&[0x1c]).expect("Ctrl+\\");
    eventually("detach", || matches!(outer.try_wait(), Ok(Some(_))));
    let status = outer.try_wait().expect("status").expect("exited");
    assert!(status.success(), "attach exits 0 on detach: {status:?}");
    let ls = stdout(&shed_in(&dir, &["ls"]));
    assert!(
        ls.contains("shell  running"),
        "still running after detach: {ls}"
    );
    let read = stdout(&shed_in(&dir, &["read", "shell", "--history"]));
    assert!(read.contains("hi-3"), "{read}");
}

#[test]
fn a_query_the_screen_answers_reaches_the_program_once_with_a_client_attached() {
    let dir = temp_dir("attach-da1");
    let d = dir.to_str().expect("utf-8 dir");
    let _server = Server::start(&dir);
    let script = r#"stty -echo; printf ready; read go; printf '\033[casked'; read line
printf '\nreplies=%d\n' "$(printf %s "$line" | tr -cd c | wc -c)"; exec sleep 100"#;
    stdout(&shed_in(
        &dir,
        &["run", "--name", "q", "--cwd", d, "--", "sh", "-c", script],
    ));
    let size = Size { rows: 24, cols: 80 };
    let cmd = [SHED, "--dir", d, "attach", "q"];
    let outer = Terminal::spawn(&cmd, &dir, [("TERM", "xterm")], size).expect("attach in a pty");
    eventually("the program", || outer.screen().contains("ready"));
    outer.write(b"go\r").expect("type");
    eventually("the query", || outer.screen().contains("asked"));
    outer.write(b"\r").expect("type");
    eventually("the count", || {
        stdout(&shed_in(&dir, &["read", "q"])).contains("replies=")
    });
    let read = stdout(&shed_in(&dir, &["read", "q"]));
    assert!(read.contains("replies=1\n"), "{read}");
}

#[test]
fn run_size_sets_the_program_size_and_the_default_is_wide() {
    let dir = temp_dir("run-size");
    let d = dir.to_str().expect("utf-8 dir");
    let _server = Server::start(&dir);
    let script = "stty size; exec sleep 100";
    for (name, size, want) in [
        ("wide", None, "50 200"),
        ("given", Some("50x200"), "50 200"),
        ("small", Some("7x33"), "7 33"),
    ] {
        let mut args = vec!["run", "--name", name, "--cwd", d];
        args.extend(size.map(|s| ["--size", s]).into_iter().flatten());
        args.extend(["--", "sh", "-c", script]);
        stdout(&shed_in(&dir, &args));
        eventually(want, || {
            stdout(&shed_in(&dir, &["read", name])) == format!("{want}\n")
        });
    }
    let bad = shed_in(&dir, &["run", "--size", "0x10", "--", "true"]);
    assert!(!bad.status.success(), "{bad:?}");
}

#[test]
fn attach_ends_when_the_program_exits() {
    let dir = temp_dir("attach-exit");
    let d = dir.to_str().expect("utf-8 dir");
    let _server = Server::start(&dir);
    stdout(&shed_in(
        &dir,
        &["run", "--name", "cat", "--cwd", d, "--", "cat"],
    ));
    let size = Size { rows: 24, cols: 80 };
    let cmd = [SHED, "--dir", d, "attach", "cat"];
    let outer = Terminal::spawn(&cmd, &dir, [("TERM", "xterm")], size).expect("attach in a pty");
    outer.write(b"ping\r").expect("type");
    eventually("the echo", || outer.screen().contains("ping"));
    stdout(&shed_in(&dir, &["kill", "cat"]));
    eventually("attach to end", || matches!(outer.try_wait(), Ok(Some(_))));
}

#[test]
fn run_autostart_starts_a_detached_server() {
    let dir = temp_dir("autostart");
    let d = dir.to_str().expect("utf-8 dir");
    let out = shed_in(
        &dir,
        &[
            "run",
            "--autostart",
            "--cwd",
            d,
            "--",
            "sh",
            "-c",
            "echo $PPID; exec sleep 100",
        ],
    );
    assert_eq!(stdout(&out), "t1\n");
    let socket = dir.join("shed.sock");
    let started = format!(
        "server {} on {} started",
        env!("CARGO_PKG_VERSION"),
        socket.display()
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains(&started),
        "{out:?}"
    );
    let mut server = String::new();
    eventually("the server's pid", || {
        server = stdout(&shed_in(&dir, &["read", "t1"])).trim().to_owned();
        !server.is_empty()
    });
    let again = shed_in(&dir, &["serve", "--detach"]);
    assert!(!again.status.success(), "a second server is refused");
    let killed = Command::new("kill").arg(&server).status().expect("kill");
    assert!(
        killed.success(),
        "the server ({server}) is the program's parent"
    );
    eventually("the server to exit", || {
        !Command::new("kill")
            .args(["-0", &server])
            .stderr(Stdio::null())
            .status()
            .expect("kill -0")
            .success()
    });
}

#[test]
fn serve_prints_one_ready_line_once_its_socket_accepts() {
    let dir = temp_dir("ready");
    let mut serve = Command::new(SHED)
        .arg("--dir")
        .arg(&dir)
        .arg("serve")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("shed serve");
    let mut lines = BufReader::new(serve.stdout.take().expect("stdout")).lines();
    let line = lines.next().expect("a line").expect("utf-8");
    let socket = dir.join("shed.sock");
    let version = env!("CARGO_PKG_VERSION");
    assert_eq!(line, format!("shed ready {version} {}", socket.display()));
    assert!(std::os::unix::net::UnixStream::connect(&socket).is_ok());
    serve.kill().expect("kill");
    assert!(lines.next().is_none(), "exactly one line");
    serve.wait().ok();
}

#[test]
fn a_server_that_exits_before_it_is_ready_is_reported_as_an_exit() {
    let dir = temp_dir("early-exit");
    std::fs::write(dir.join("state.json"), "not json").expect("state");
    let start = Instant::now();
    let out = shed_in(&dir, &["serve", "--detach"]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("shed serve exited (exit status: 1) before it was ready"),
        "{err}"
    );
    assert!(start.elapsed() < Duration::from_secs(10), "not the timeout");
}

#[test]
fn a_child_allocating_50_mb_fires_a_resources_event_and_tops_the_list() {
    let dir = temp_dir("resources");
    let d = dir.to_str().expect("utf-8 dir");
    let _server = Server::sampling(&dir, "0.2");
    let mut events = Command::new(SHED)
        .args(["--dir", d, "events", "--label", "test=mem"])
        .stdout(Stdio::piped())
        .spawn()
        .expect("shed events");
    let mut lines = BufReader::new(events.stdout.take().expect("stdout")).lines();
    let mut next = || -> Value {
        let line = lines.next().expect("an event").expect("a line");
        serde_json::from_str(&line).expect("event is JSON")
    };
    stdout(&shed_in(
        &dir,
        &["run", "--name", "small", "--cwd", d, "--", "sleep", "100"],
    ));
    let script = r#"sh -c 'x=$(head -c 52428800 /dev/zero | tr "\0" x); sleep 100'"#;
    let run = ["run", "--name", "mem", "--label", "test=mem", "--cwd", d];
    stdout(&shed_in(
        &dir,
        &[&run[..], &["--", "sh", "-c", script]].concat(),
    ));
    assert_eq!(next()["event"], "started");
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        assert!(Instant::now() < deadline, "no 50 MB resources event");
        let event = next();
        assert_eq!(
            (&event["name"], &event["event"]),
            (&json!("mem"), &json!("resources"))
        );
        if event["rss_mb"].as_u64().expect("rss_mb") >= 50 {
            break;
        }
    }
    events.kill().ok();
    events.wait().ok();

    let get: Value = serde_json::from_str(&stdout(&shed_in(&dir, &["get", "mem"]))).expect("json");
    assert!(get["resources"]["rss_mb"].as_u64() >= Some(50), "{get}");
    let top = stdout(&shed_in(&dir, &["top"]));
    let names: Vec<&str> = top
        .lines()
        .skip(1)
        .filter_map(|l| l.split(' ').next())
        .collect();
    assert_eq!(names, ["mem", "small"], "{top}");
}

#[test]
fn timing_report() {
    const CALLS: usize = 30;
    let dir = temp_dir("timing");
    let d = dir.to_str().expect("utf-8 dir");
    let _server = Server::start(&dir);
    stdout(&shed_in(
        &dir,
        &["run", "--name", "t", "--cwd", d, "--", "sleep", "100"],
    ));
    for args in [
        &["ls"][..],
        &["get", "t"],
        &["read", "t"],
        &["exec", "--", "true"],
    ] {
        let mut ms: Vec<f64> = (0..CALLS)
            .map(|_| {
                let start = Instant::now();
                stdout(&shed_in(&dir, args));
                start.elapsed().as_secs_f64() * 1000.0
            })
            .collect();
        ms.sort_by(f64::total_cmp);
        let median = ms[CALLS / 2];
        let note = if median < 20.0 { "ok" } else { "SLOW" };
        eprintln!(
            "timing: shed {:<6} median {median:5.1} ms, p90 {:5.1} ms, max {:5.1} ms over {CALLS} calls (target < 20 ms, soft): {note}",
            args.join(" "),
            ms[CALLS * 9 / 10],
            ms[CALLS - 1],
        );
    }
}

#[test]
fn a_terminal_has_shed_set() {
    let dir = temp_dir("env");
    let _server = Server::start(&dir);
    let d = dir.to_str().expect("utf-8 dir");
    let script = "echo \"shed=$SHED\"; read x";
    stdout(&shed_in(
        &dir,
        &[
            "run", "--name", "envs", "--cwd", d, "--", "sh", "-c", script,
        ],
    ));
    eventually("the name", || {
        stdout(&shed_in(&dir, &["read", "envs"])).contains("shed=envs")
    });
}

#[test]
fn a_test_temp_dir_is_removed_when_its_test_ends_pass_or_fail() {
    let passed = std::thread::spawn(|| temp_dir("ends-pass"))
        .join()
        .expect("passes");
    let (tx, rx) = std::sync::mpsc::channel();
    let failed = std::thread::spawn(move || {
        tx.send(temp_dir("ends-fail")).expect("send");
        panic!("a failing test");
    });
    assert!(failed.join().is_err());
    let failed = rx.recv().expect("the failed test's dir");
    assert!(!passed.exists(), "{}", passed.display());
    assert!(!failed.exists(), "{}", failed.display());
}
