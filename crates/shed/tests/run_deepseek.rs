//! The built-in `opencode` and `pi-acp` records run on a `deepseek` key account with no file under
//! `<config>/providers/`: the key reaches the agent, OpenCode gets the model through its config, and pi-acp,
//! which takes no model, refuses one.
#![cfg(test)]

mod common;

use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::process::{Command, Output};

use common::{Server, temp_dir};
use shed_server::Paths;

const KEY: &str = "sk-fake-deepseek-0123456789";
const MODEL: &str = "deepseek/deepseek-chat";
const CONFIG: &str = r#"{"model":"{env:OPENCODE_MODEL}"}"#;

fn shed(dir: &Path, args: &[&str]) -> Output {
    let out = Command::new(env!("CARGO_BIN_EXE_shed"))
        .arg("--dir")
        .arg(dir)
        .args(args)
        .output();
    out.expect("shed runs")
}

fn ok(dir: &Path, args: &[&str]) -> String {
    let out = shed(dir, args);
    assert!(out.status.success(), "shed {args:?} failed: {out:?}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn start(dir: &Path, id: &str, model: &[&str]) -> Output {
    let agent = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake_acp.py");
    let run = ["run", "--name", id, "--acp", id, "--account", "work"];
    let cmd = ["--", "python3", agent.to_str().expect("utf-8"), "--unique"];
    let out = shed(dir, &[&run[..], model, &cmd].concat());
    if out.status.success() {
        let ready = [
            "wait",
            id,
            "--output",
            "ACP session ready",
            "--timeout",
            "10",
        ];
        ok(dir, &ready);
    }
    out
}

fn says(dir: &Path, name: &str, var: &str) -> String {
    ok(dir, &["send", name, &format!("env {var}")]);
    ok(dir, &["wait", name, "--until", "done", "--timeout", "10"]);
    let read = ok(dir, &["read", name]);
    read.lines().last().unwrap_or_default().to_owned()
}

#[test]
fn opencode_and_pi_run_on_a_deepseek_key_with_no_provider_file() {
    let dir = temp_dir("run-deepseek");
    let _server = Server::start(&dir);
    let file = dir.join("work.key");
    std::fs::write(&file, KEY).expect("write the key");
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).expect("chmod");
    let file = file.to_str().expect("utf-8");
    let add = ["account", "add", "deepseek", "work", "--token-file", file];
    ok(&dir, &add);
    assert!(!Paths::in_dir(&dir).providers().exists());

    let out = start(&dir, "opencode", &["--model", MODEL]);
    assert!(out.status.success(), "{out:?}");
    let has = |var: &str, value: &str| format!("{var} has {} chars", value.len());
    let key = "DEEPSEEK_API_KEY";
    assert_eq!(says(&dir, "opencode", key), has(key, KEY));
    let model = "OPENCODE_MODEL";
    assert_eq!(says(&dir, "opencode", model), has(model, MODEL));
    let config = "OPENCODE_CONFIG_CONTENT";
    assert_eq!(says(&dir, "opencode", config), has(config, CONFIG));

    let refused = start(&dir, "pi-acp", &["--model", MODEL]);
    let why = String::from_utf8_lossy(&refused.stderr);
    assert!(
        !refused.status.success() && why.contains("pi-acp takes no model"),
        "{refused:?}"
    );
    let out = start(&dir, "pi-acp", &[]);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(says(&dir, "pi-acp", key), has(key, KEY));
    assert_eq!(says(&dir, "pi-acp", model), format!("no {model}"));
}
