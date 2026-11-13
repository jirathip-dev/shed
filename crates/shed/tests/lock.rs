//! #72: one slow terminal never holds up the server for the others.
#![cfg(test)]

mod common;

use std::path::{Path, PathBuf};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use common::{Server, eventually, temp_dir};
use serde_json::{Value, json};
use shed_server::proto::Method;
use shed_server::{Client, Paths};

fn client(dir: &Path) -> Client {
    Client::connect(&Paths::in_dir(dir).socket).expect("connect")
}

fn call(c: &mut Client, method: Method, params: &Value) -> Value {
    c.call(method, params).expect("call")
}

fn run(c: &mut Client, dir: &Path, name: &str, script: &str) {
    let params = json!({"name": name, "cwd": dir, "cmd": ["sh", "-c", script]});
    call(c, Method::Run, &params);
    eventually("ready", || {
        call(c, Method::Read, &json!({"name": name}))["text"]
            .as_str()
            .is_some_and(|t| t.contains("ready"))
    });
}

fn timed(c: &mut Client, method: Method, params: &Value) -> Duration {
    let start = Instant::now();
    call(c, method, params);
    start.elapsed()
}

fn in_thread<T: Send + 'static>(
    dir: &Path,
    f: impl FnOnce(&mut Client) -> T + Send + 'static,
) -> JoinHandle<T> {
    let dir: PathBuf = dir.to_owned();
    std::thread::spawn(move || f(&mut client(&dir)))
}

#[test]
fn a_terminal_that_never_reads_its_input_holds_up_no_other_and_is_reported_stuck() {
    let dir = temp_dir("lock-stuck");
    let _server = Server::start(&dir);
    let mut c = client(&dir);
    run(
        &mut c,
        &dir,
        "deaf",
        "stty raw -echo; echo ready; exec sleep 100",
    );
    run(&mut c, &dir, "other", "echo ready; exec cat >/dev/null");
    let flood = in_thread(&dir, |c| {
        let text = "x".repeat(1 << 20);
        let sent = c.call::<_, Value>(Method::Send, &json!({"name": "deaf", "text": text}));
        sent.expect_err("the flood times out").to_string()
    });
    let mut rounds = 0;
    while !flood.is_finished() {
        let ls = timed(&mut c, Method::Ls, &json!({}));
        let send = timed(
            &mut c,
            Method::Send,
            &json!({"name": "other", "text": "hi\r"}),
        );
        assert!(ls < Duration::from_millis(100), "ls took {ls:?}");
        assert!(send < Duration::from_millis(100), "send took {send:?}");
        rounds += 1;
    }
    assert!(rounds > 5, "only {rounds} rounds while the flood was stuck");
    let why = flood.join().expect("flood");
    assert!(why.contains("stuck"), "{why}");
    let deaf = call(&mut c, Method::Get, &json!({"name": "deaf"}));
    assert_eq!(deaf["reported"]["state"], "stuck", "{deaf}");
}

#[test]
fn killing_a_program_that_ignores_sigterm_holds_up_no_list() {
    let dir = temp_dir("lock-kill");
    let _server = Server::start(&dir);
    let mut c = client(&dir);
    run(
        &mut c,
        &dir,
        "stubborn",
        "trap '' TERM; echo ready; exec sleep 100",
    );
    let kill = in_thread(&dir, |c| {
        timed(c, Method::Kill, &json!({"name": "stubborn"}))
    });
    let mut rounds = 0;
    while !kill.is_finished() {
        let ls = timed(&mut c, Method::Ls, &json!({}));
        assert!(ls < Duration::from_millis(100), "ls took {ls:?}");
        rounds += 1;
    }
    let took = kill.join().expect("kill");
    assert!(
        took >= Duration::from_secs(2),
        "SIGTERM was not ignored: {took:?}"
    );
    assert!(rounds > 5, "only {rounds} rounds during the kill");
    let stubborn = call(&mut c, Method::Get, &json!({"name": "stubborn"}));
    assert_eq!(stubborn["state"], "exited", "{stubborn}");
}

#[test]
fn a_thousand_emits_from_one_lane_keep_list_p95_under_20_ms() {
    let dir = temp_dir("lock-emit");
    let _server = Server::start(&dir);
    let mut c = client(&dir);
    run(&mut c, &dir, "lane", "echo ready; exec sleep 100");
    run(&mut c, &dir, "quiet", "echo ready; exec sleep 100");
    let emits = in_thread(&dir, |c| {
        for n in 0..1_000 {
            let text = format!("chunk {n}");
            call(
                c,
                Method::Emit,
                &json!({"name": "lane", "event": "text", "text": text}),
            );
        }
    });
    let mut took = Vec::new();
    while !emits.is_finished() {
        took.push(timed(&mut c, Method::Ls, &json!({})));
    }
    emits.join().expect("emits");
    took.sort();
    let p95 = took.get(took.len() * 95 / 100).copied().unwrap_or_default();
    assert!(took.len() > 20, "only {} lists", took.len());
    assert!(p95 < Duration::from_millis(20), "list p95 {p95:?}");
}
