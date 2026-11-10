//! The idle-connections report (`scripts/bench-fanout.sh` runs it at a PR's base and head, report only):
//! 500 `events` clients on one server, then one state change: the time until every client has it, and
//! the server's thread count.

use std::io::BufRead as _;
use std::os::unix::net::UnixStream;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use shed_server::proto::{EventsParams, Method};
use shed_server::{Client, Paths};

const CLIENTS: usize = 500;

fn threads(pid: u32) -> usize {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap_or_default();
    if let Some(n) = status.lines().find_map(|l| l.strip_prefix("Threads:")) {
        return n.trim().parse().expect("a thread count");
    }
    let ps = Command::new("ps")
        .args(["-M", "-p", &pid.to_string()])
        .output();
    String::from_utf8_lossy(&ps.expect("ps").stdout)
        .lines()
        .count()
        - 1
}

#[test]
#[ignore = "a timing report: scripts/bench-fanout.sh"]
fn idle_connections() {
    let dir = std::env::temp_dir().join(format!("shed-idle-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let mut server = Command::new(env!("CARGO_BIN_EXE_shed"))
        .args(["serve", "--sample", "3600", "--dir"])
        .arg(&dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("shed serve starts");
    let socket = Paths::in_dir(&dir).socket;
    while UnixStream::connect(&socket).is_err() {
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut client = Client::connect(&socket).expect("connect");
    let run = json!({"name": "t", "cmd": ["sleep", "100"], "cwd": "/"});
    let _: Value = client.call(Method::Run, &run).expect("run");
    let mut readers: Vec<_> = (0..CLIENTS)
        .map(|_| {
            let mut c = Client::connect(&socket).expect("connect");
            let _: Value = c
                .call(Method::Events, &EventsParams::default())
                .expect("events");
            c.split().0
        })
        .collect();
    let threads = threads(server.id());
    let start = Instant::now();
    let state = json!({"name": "t", "state": "working"});
    let _: Value = client.call(Method::State, &state).expect("state");
    for r in &mut readers {
        let mut line = String::new();
        r.read_line(&mut line).expect("the state event");
        assert!(line.contains("\"working\""), "{line}");
    }
    let us = start.elapsed().as_micros();
    println!("idle {CLIENTS} clients: one state to all in {us} us, server threads {threads}");
    let _ = server.kill();
    let _ = server.wait();
    let _ = std::fs::remove_dir_all(&dir);
}
