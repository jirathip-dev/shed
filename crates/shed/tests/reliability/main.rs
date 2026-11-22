//! herdsman's fixed bug classes that apply to shed (herdsman#553, "Reliability"), one test per row.
#![cfg(test)]

#[path = "../common/mod.rs"]
mod common;
mod files;
mod kill;
mod lock;
mod requests;
mod signals;

use std::path::Path;
use std::process::{Command, Output, Stdio};

use common::eventually;
use serde_json::{Value, json};
use shed_client::{Conn, Error};
use shed_server::Paths;

const SHED: &str = env!("CARGO_BIN_EXE_shed");

fn shed_in(dir: &Path, args: &[&str]) -> Output {
    Command::new(SHED)
        .arg("--dir")
        .arg(dir)
        .args(args)
        .output()
        .expect("shed runs")
}

fn ok(dir: &Path, args: &[&str]) -> String {
    let out = shed_in(dir, args);
    assert!(out.status.success(), "shed {args:?} failed: {out:?}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn run_sh(dir: &Path, name: &str, script: &str) {
    let d = dir.to_str().expect("utf-8 dir");
    ok(
        dir,
        &["run", "--name", name, "--cwd", d, "--", "sh", "-c", script],
    );
}

fn alive(pid: &str) -> bool {
    Command::new("kill")
        .args(["-0", pid])
        .stderr(Stdio::null())
        .status()
        .expect("kill -0")
        .success()
}

fn pids(dir: &Path, name: &str) -> (String, String) {
    let mut child = String::new();
    eventually("the child's pid", || {
        let screen = ok(dir, &["read", name]);
        screen
            .lines()
            .find_map(|l| l.strip_prefix("child="))
            .unwrap_or_default()
            .clone_into(&mut child);
        !child.is_empty()
    });
    let info: Value = serde_json::from_str(&ok(dir, &["--json", "get", name])).expect("json");
    (info["pid"].to_string(), child)
}

fn connect(dir: &Path) -> Conn {
    Conn::connect(&Paths::in_dir(dir).socket).expect("connect")
}

fn spawn(client: &mut Conn, dir: &Path, name: &str, cmd: &[&str]) -> Result<Value, Error> {
    client.call(
        shed_server::proto::Method::Run,
        &json!({"name": name, "cwd": dir, "cmd": cmd}),
    )
}

fn run_shell(dir: &Path, name: &str) {
    let d = dir.to_str().expect("utf-8 dir");
    ok(
        dir,
        &[
            "run",
            "--name",
            name,
            "--cwd",
            d,
            "--",
            "env",
            "PS1=ready> ",
            "sh",
            "-i",
        ],
    );
    eventually("the prompt", || ok(dir, &["read", name]).contains("ready>"));
}
