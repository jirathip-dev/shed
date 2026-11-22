//! The CLI end to end, against a real server.
//!
//! The human and JSON output is checked against `tests/golden/cli.txt` (the temp dir and pids
//! normalised). Regenerate with `just generate` (the tests in `golden.rs`).
#![cfg(test)]

mod attach;
#[path = "../common/mod.rs"]
mod common;
mod family;
mod golden;
mod hello;
mod resources;
mod serve;
mod skill;
mod state;
mod wait;

use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;

const SHED: &str = env!("CARGO_BIN_EXE_shed");

fn shed(args: &[&str]) -> String {
    let out = Command::new(SHED).args(args).output().expect("shed runs");
    assert!(out.status.success(), "shed {args:?} failed: {out:?}");
    String::from_utf8(out.stdout).expect("utf-8 stdout")
}

fn shed_in(dir: &Path, args: &[&str]) -> Output {
    Command::new(SHED)
        .arg("--dir")
        .arg(dir)
        .args(args)
        .output()
        .expect("shed runs")
}

fn stdout(out: &Output) -> String {
    assert!(out.status.success(), "shed failed: {out:?}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn shed_ok(dir: &Path, args: &[&str]) -> String {
    stdout(&shed_in(dir, args))
}

fn info(dir: &Path, name: &str) -> Value {
    serde_json::from_str(&shed_ok(dir, &["--json", "get", name])).expect("terminal JSON")
}

fn start(dir: &Path, name: &str, extra: &[&str], cmd: &[&str]) {
    let d = dir.to_str().expect("utf-8 dir");
    let args = [&["run", "--name", name, "--cwd", d], extra, &["--"], cmd].concat();
    shed_ok(dir, &args);
}
