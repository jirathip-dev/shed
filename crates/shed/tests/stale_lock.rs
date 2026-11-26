//! `run --acp`: an empty lock the record names, older than its window, is removed from the agent's config folder
//! before the start and a `lock_cleared` event says so; a start with no lock there logs none.
#![cfg(test)]

mod common;

use std::io::BufRead as _;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, UNIX_EPOCH};

use common::{Server, temp_dir};
use serde_json::Value;
use shed_client::Conn;
use shed_server::Paths;
use shed_server::proto::{EventsParams, Method};

const SHED: &str = env!("CARGO_BIN_EXE_shed");

fn run(dir: &Path, name: &str) {
    let args = ["run", "--name", name, "--acp", "fake", "--", "true"];
    let out = Command::new(SHED).arg("--dir").arg(dir).args(args).output();
    let out = out.expect("shed runs");
    assert!(out.status.success(), "shed run {name} failed: {out:?}");
}

#[test]
fn a_stale_empty_lock_is_removed_before_a_start_and_logged() {
    let dir = temp_dir("stale-lock");
    let home = dir.join("home");
    std::fs::create_dir_all(&home).expect("config folder");
    let record = format!(
        "id = \"fake\"\nagent = \"fake\"\ndefault_folder = {:?}\n\
         [stale_lock]\nfile = \".refresh.lock\"\nsecs = 60\n",
        home.display().to_string()
    );
    let providers = Paths::in_dir(&dir).providers();
    std::fs::create_dir_all(&providers).expect("providers dir");
    std::fs::write(providers.join("fake.toml"), record).expect("write the record");
    let _server = Server::start(&dir);
    run(&dir, "none");
    let lock = home.join(".refresh.lock");
    let file = std::fs::File::create(&lock).expect("make the lock");
    let at = UNIX_EPOCH + Duration::from_secs(shed_server::clock::unix_now() - 120);
    file.set_modified(at).expect("date the lock");
    run(&dir, "stale");
    assert!(!lock.exists(), "the stale lock is gone");
    let mut client = Conn::connect(&Paths::in_dir(&dir).socket).expect("connect");
    let params = EventsParams {
        since: Some(0),
        ..EventsParams::default()
    };
    let _: Value = client.call(Method::Events, &params).expect("events");
    let (reader, _writer) = client.split();
    let timeout = Some(Duration::from_secs(10));
    reader.get_ref().set_read_timeout(timeout).expect("timeout");
    let event = (reader.lines())
        .map(|l| serde_json::from_str::<Value>(&l.expect("an event line")).expect("JSON"))
        .find(|e| e["event"] == "lock_cleared")
        .expect("a lock_cleared event");
    assert_eq!(event["name"], "stale", "{event}");
    assert_eq!(event["path"], lock.display().to_string());
    assert_eq!(event["size"], 0);
    assert!(
        event["age_secs"].as_u64().is_some_and(|a| a >= 120),
        "{event}"
    );
}
