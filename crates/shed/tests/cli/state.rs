use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use crate::common::{Server, eventually, temp_dir};
use crate::{SHED, info, shed_in, shed_ok, start, stdout};

#[test]
fn state_gives_up_on_a_server_that_never_answers_within_its_deadline() {
    let dir = temp_dir("state-deadline");
    let listener = std::os::unix::net::UnixListener::bind(dir.join("shed.sock")).expect("bind");
    let (done, held) = std::sync::mpsc::channel::<()>();
    let server = std::thread::spawn(move || {
        let (_stream, _) = listener.accept().expect("accept");
        held.recv_timeout(Duration::from_secs(10)).ok();
    });
    let started = Instant::now();
    let out = Command::new(SHED)
        .arg("--dir")
        .arg(&dir)
        .args(["state", "t1", "working"])
        .env("SHED_STATE_TIMEOUT", "0.5")
        .output()
        .expect("shed runs");
    let took = started.elapsed();
    done.send(()).ok();
    server.join().expect("fake server");
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "shed: no answer from the server within 500ms ($SHED_STATE_TIMEOUT)\n"
    );
    assert!(took < Duration::from_secs(2), "took {took:?}");
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
fn a_trust_prompt_on_screen_outranks_a_reported_working_until_it_leaves() {
    let dir = temp_dir("rules");
    let _server = Server::start(&dir);
    let golden = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../shed-server/tests/screens/claude-trust.txt")
        .canonicalize()
        .expect("the golden trust screen");
    let agent = "printf '\\033]7501;state=working\\007'; cat \"$0\"; read x; printf '\\033[2J\\033[Hgo on\\n'; read y";
    let golden = golden.to_str().expect("utf-8 path");
    start(
        &dir,
        "lane",
        &["--label", "agent=claude"],
        &["sh", "-c", agent, golden],
    );
    let until = |state: &str| {
        shed_in(&dir, &["wait", "lane", "--until", state, "--timeout", "10"])
            .status
            .success()
    };
    assert!(until("blocked"), "{}", info(&dir, "lane"));
    let reported = &info(&dir, "lane")["reported"];
    assert_eq!(reported["state"], "blocked", "{reported}");
    assert_eq!(reported["kind"], "permission", "{reported}");
    assert_eq!(reported["from"], "rule:trust", "{reported}");
    shed_ok(&dir, &["keys", "lane", "enter"]);
    assert!(until("working"), "{}", info(&dir, "lane"));
    let reported = &info(&dir, "lane")["reported"];
    assert_eq!(reported["from"], "reported", "{reported}");
}
