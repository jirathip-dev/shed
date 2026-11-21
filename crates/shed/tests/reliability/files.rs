use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};

use serde_json::{Value, json};
use shed_client::Conn;
use shed_server::Paths;

use crate::common::{eventually, temp_dir};
use crate::{connect, spawn};

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

fn list(client: &mut Conn) -> Vec<Value> {
    client
        .call(shed_server::proto::Method::Ls, &json!({}))
        .expect("ls")
}

fn running_count(client: &mut Conn) -> usize {
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
