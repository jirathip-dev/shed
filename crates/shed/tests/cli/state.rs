use std::process::Command;

use crate::common::{Server, eventually, temp_dir};
use crate::{SHED, shed_in, stdout};
use shed_server::clock;

#[test]
fn state_gives_up_on_a_server_that_never_answers_within_its_deadline() {
    let dir = temp_dir("state-deadline");
    let listener = std::os::unix::net::UnixListener::bind(dir.join("shed.sock")).expect("bind");
    let (done, held) = std::sync::mpsc::channel::<()>();
    let server = std::thread::spawn(move || {
        let (_stream, _) = listener.accept().expect("accept");
        held.recv_timeout(clock::PATIENCE).is_ok()
    });
    let out = Command::new(SHED)
        .arg("--dir")
        .arg(&dir)
        .args(["state", "t1", "working"])
        .env("SHED_STATE_TIMEOUT", "0.5")
        .output()
        .expect("shed runs");
    done.send(()).ok();
    let held_on = server.join().expect("fake server");
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "shed: no answer from the server within 500ms ($SHED_STATE_TIMEOUT)\n"
    );
    assert!(held_on, "the client waited for the server to hang up");
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
