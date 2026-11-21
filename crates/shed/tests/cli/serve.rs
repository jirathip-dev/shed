use std::io::{BufRead as _, BufReader};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use shed_server::proto::LONG_VERSION;

use crate::common::{eventually, temp_dir};
use crate::{SHED, shed_in, stdout};

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
    let started = format!("server {LONG_VERSION} on {} started", socket.display());
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
    assert_eq!(
        line,
        format!("shed ready {LONG_VERSION} {}", socket.display())
    );
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
