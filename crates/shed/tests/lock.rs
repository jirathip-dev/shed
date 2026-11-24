//! #72: one slow terminal never holds up the server for the others.
#![cfg(test)]

mod common;

use std::path::{Path, PathBuf};
use std::thread::JoinHandle;
use std::time::Duration;

use common::{Server, eventually, temp_dir};
use serde_json::{Value, json};
use shed_client::Conn;
use shed_server::Paths;
use shed_server::clock;
use shed_server::proto::Method;

fn client(dir: &Path) -> Conn {
    Conn::connect(&Paths::in_dir(dir).socket).expect("connect")
}

fn call(c: &mut Conn, method: Method, params: &Value) -> Value {
    c.call(method, params).expect("call")
}

fn run(c: &mut Conn, dir: &Path, name: &str, script: &str) {
    let params = json!({"name": name, "cwd": dir, "cmd": ["sh", "-c", script]});
    call(c, Method::Run, &params);
    eventually("ready", || {
        call(c, Method::Read, &json!({"name": name}))["text"]
            .as_str()
            .is_some_and(|t| t.contains("ready"))
    });
}

fn timed(c: &mut Conn, method: Method, params: &Value) -> Duration {
    let start = clock::now();
    call(c, method, params);
    start.elapsed()
}

fn in_thread<T: Send + 'static>(
    dir: &Path,
    f: impl FnOnce(&mut Conn) -> T + Send + 'static,
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
        call(&mut c, Method::Ls, &json!({}));
        call(
            &mut c,
            Method::Send,
            &json!({"name": "other", "text": "hi\r"}),
        );
        rounds += 1;
    }
    assert!(rounds > 5, "only {rounds} rounds while the flood was stuck");
    let why = flood.join().expect("flood");
    assert!(why.contains("stuck"), "{why}");
    let deaf = call(&mut c, Method::Get, &json!({"name": "deaf"}));
    assert_eq!(deaf["reported"]["state"], "stuck", "{deaf}");
}

#[test]
fn a_state_save_that_never_finishes_holds_up_no_other_request() {
    let dir = temp_dir("lock-save");
    let _server = Server::start(&dir);
    let mut c = client(&dir);
    run(&mut c, &dir, "other", "echo ready; exec cat >/dev/null");
    let tmp = Paths::in_dir(&dir).state.with_extension("json.tmp");
    let made = std::process::Command::new("mkfifo").arg(&tmp).status();
    assert!(made.expect("mkfifo").success());
    let cwd = dir.clone();
    let saving = in_thread(&dir, move |c| {
        let params = json!({"name": "late", "cwd": cwd, "cmd": ["sleep", "100"]});
        call(c, Method::Run, &params)
    });
    let others = in_thread(&dir, |c| {
        eventually("late listed", || {
            let list = call(c, Method::Ls, &json!({}));
            list.as_array()
                .is_some_and(|l| l.iter().any(|t| t["name"] == "late"))
        });
        call(c, Method::Send, &json!({"name": "other", "text": "hi\r"}));
    });
    let answered = clock::poll(clock::PATIENCE, || others.is_finished());
    let mut saved = String::new();
    std::io::Read::read_to_string(&mut std::fs::File::open(&tmp).expect("fifo"), &mut saved)
        .expect("drain the save");
    others.join().expect("others");
    assert_eq!(saving.join().expect("run")["name"], "late");
    assert!(answered, "ls and send waited for another request's save");
    assert!(saved.contains("late"), "{saved}");
}

#[test]
fn killing_a_program_that_ignores_sigterm_and_sighup_holds_up_no_list() {
    let dir = temp_dir("lock-kill");
    let _server = Server::start(&dir);
    let mut c = client(&dir);
    run(
        &mut c,
        &dir,
        "stubborn",
        "trap '' TERM HUP; echo ready; exec sleep 100",
    );
    let kill = in_thread(&dir, |c| {
        timed(c, Method::Kill, &json!({"name": "stubborn"}))
    });
    let mut rounds = 0;
    while !kill.is_finished() {
        call(&mut c, Method::Ls, &json!({}));
        rounds += 1;
    }
    let took = kill.join().expect("kill");
    assert!(
        took >= Duration::from_secs(2),
        "SIGTERM and SIGHUP were not ignored: {took:?}"
    );
    assert!(rounds > 5, "only {rounds} rounds during the kill");
    let stubborn = call(&mut c, Method::Get, &json!({"name": "stubborn"}));
    assert_eq!(stubborn["state"], "exited", "{stubborn}");
}

#[test]
fn a_thousand_emits_from_one_lane_hold_up_no_list() {
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
    let mut lists = 0;
    while !emits.is_finished() {
        call(&mut c, Method::Ls, &json!({}));
        lists += 1;
    }
    emits.join().expect("emits");
    assert!(lists > 20, "only {lists} lists");
}
