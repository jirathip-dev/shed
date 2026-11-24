use std::os::unix::net::UnixStream;
use std::process::{Command, Stdio};

use shed_server::Paths;

use crate::common::{Server, eventually, temp_dir};
use crate::{SHED, alive, ok};
use shed_server::clock;

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
    let mut status = None;
    let exited = || {
        status = second.try_wait().expect("try_wait");
        status.is_some()
    };
    if !clock::poll(clock::PATIENCE, exited) {
        second.kill().ok();
        second.wait().ok();
        panic!("a second server came up beside the first");
    }
    let status = status.expect("exited");
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
