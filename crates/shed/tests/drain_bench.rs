//! The drain bench (`scripts/bench-drain.sh` runs it at a PR's base and head): each stream `cat` into a
//! terminal of a fresh `shed serve`, timed until `read` (polled every 2 ms) shows its last line, and serve's peak RSS (sampled
//! every 10 ms) while it drains. The raw-read ceiling is shed-core's `tests/drain_raw.rs`.
#![cfg(test)]

#[path = "drain/fixture.rs"]
mod fixture;

use std::os::unix::net::UnixStream;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use shed_server::proto::Method;
use shed_server::{Client, Paths};

fn rss_kb(pid: u32) -> u64 {
    let ps = Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .expect("ps");
    String::from_utf8_lossy(&ps.stdout)
        .trim()
        .parse()
        .unwrap_or(0)
}

fn drain(dir: &std::path::Path, kind: &str) {
    let file = fixture::write(dir, kind);
    let state = dir.join(kind);
    let mut server = Command::new(env!("CARGO_BIN_EXE_shed"))
        .args(["serve", "--sample", "3600", "--dir"])
        .arg(&state)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("shed serve starts");
    let socket = Paths::in_dir(&state).socket;
    while UnixStream::connect(&socket).is_err() {
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut client = Client::connect(&socket).expect("connect");
    let done = AtomicBool::new(false);
    let pid = server.id();
    let (secs, peak) = std::thread::scope(|s| {
        let sampler = s.spawn(|| {
            let mut peak = 0;
            while !done.load(Ordering::Relaxed) {
                peak = peak.max(rss_kb(pid));
                std::thread::sleep(Duration::from_millis(10));
            }
            peak.max(rss_kb(pid))
        });
        let cmd = [
            "sh",
            "-c",
            "cat \"$0\"; exec sleep 600",
            file.to_str().expect("utf-8"),
        ];
        let start = Instant::now();
        let run = json!({"name": "t", "cmd": cmd, "cwd": "/"});
        let _: Value = client.call(Method::Run, &run).expect("run");
        let read = json!({"name": "t", "lines": 1});
        while client.call::<_, Value>(Method::Read, &read).expect("read")["text"] != fixture::END {
            assert!(
                start.elapsed() < Duration::from_mins(2),
                "{kind} never drained"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        let secs = start.elapsed().as_secs_f64();
        done.store(true, Ordering::Relaxed);
        (secs, sampler.join().expect("sampler"))
    });
    let bytes = std::fs::metadata(&file).expect("stream").len();
    fixture::report(kind, bytes, secs);
    println!("drain {kind} peak rss: {} MB", peak >> 10);
    server.kill().ok();
    server.wait().ok();
    std::fs::remove_file(&file).ok();
}

#[test]
#[ignore = "a timing report: scripts/bench-drain.sh"]
fn drain_streams() {
    let dir = std::env::temp_dir().join(format!("shed-drain-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    for kind in fixture::STREAMS {
        drain(&dir, kind);
    }
    std::fs::remove_dir_all(&dir).ok();
}
