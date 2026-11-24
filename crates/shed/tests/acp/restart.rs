use std::process::Command;

use serde_json::{Value, json};

use crate::common::{self, Server, temp_dir};
use crate::{ok, start_fake, stop, until};

#[test]
fn a_lane_runs_on_across_a_serve_restart_and_its_client_reattaches() {
    let dir = temp_dir("acp-restart");
    let mut server = Server::start(&dir);
    start_fake(&dir, "r", "fake");
    let before: Value = serde_json::from_str(&ok(&dir, &["get", "r"])).expect("JSON");
    ok(&dir, &["send", "r", "busy"]);
    ok(
        &dir,
        &["wait", "r", "--until", "working", "done", "--timeout", "10"],
    );
    stop(&mut server);
    shed_server::clock::sleep(std::time::Duration::from_secs(3));
    let _server = Server::start(&dir);
    let after: Value = serde_json::from_str(&ok(&dir, &["get", "r"])).expect("JSON");
    assert_eq!(
        (&after["state"], &after["pid"]),
        (&json!("running"), &before["pid"])
    );
    until(&dir, "r", "done");
    assert_eq!(
        ok(&dir, &["read", "r"]),
        "> busy\n[tool] Build (completed)\nbusy done\n"
    );
    ok(&dir, &["send", "r", "hi"]);
    until(&dir, "r", "done");
    assert!(ok(&dir, &["read", "r"]).ends_with("> hi\nyou said: hi\n"));
    ok(&dir, &["kill", "r"]);
}

#[test]
fn a_lane_whose_bridge_died_while_serve_was_down_comes_back_exited() {
    let dir = temp_dir("acp-gone");
    let mut server = Server::start(&dir);
    start_fake(&dir, "gone", "fake");
    start_fake(&dir, "live", "fake");
    let info: Value = serde_json::from_str(&ok(&dir, &["get", "gone"])).expect("JSON");
    let pid = info["pid"].to_string();
    stop(&mut server);
    let killed = Command::new("kill").args(["-KILL", &pid]).status();
    assert!(killed.expect("kill runs").success());
    common::eventually("the bridge to end", || {
        !Command::new("kill")
            .args(["-0", &pid])
            .output()
            .expect("kill runs")
            .status
            .success()
    });
    let _server = Server::start(&dir);
    let state = |name| {
        let info: Value = serde_json::from_str(&ok(&dir, &["get", name])).expect("JSON");
        info["state"].clone()
    };
    assert_eq!(
        (state("gone"), state("live")),
        (json!("exited"), json!("running"))
    );
    ok(&dir, &["kill", "live"]);
}
