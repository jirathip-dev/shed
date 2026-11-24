use std::process::Command;

use serde_json::{Value, json};

use shed_client::Conn;
use shed_server::Paths;
use shed_server::proto::{Handover, Method};

use crate::common::{self, Server, temp_dir};
use crate::{first_turn, ok, sleepy_server, start_fake, start_with_history, stop, until};

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

#[test]
fn lanes_saved_without_their_protocol_do_not_block_a_handover() {
    let dir = temp_dir("acp-unsaved-speaks");
    let mut server = sleepy_server(&dir);
    start_with_history(&dir, "h", &dir.join("history.jsonl"), &[]);
    start_fake(&dir, "l", "fake");
    first_turn(&dir, "h");
    until(&dir, "h", "hibernated");
    stop(&mut server);
    let state = Paths::in_dir(&dir).state;
    let mut file: Value =
        serde_json::from_slice(&std::fs::read(&state).expect("state")).expect("JSON");
    for t in file["terminals"].as_array_mut().expect("terminals") {
        assert!(
            t.as_object_mut()
                .expect("a terminal")
                .remove("speaks")
                .is_some()
        );
    }
    std::fs::write(&state, file.to_string()).expect("state written");
    let _server = Server::start(&dir);
    let mut conn = Conn::connect(&Paths::in_dir(&dir).socket).expect("connect");
    let build = conn.server().build.clone();
    let handed: Handover = conn
        .call(Method::Handover, &json!({ "build": build }))
        .expect("an old lane does not block the handover");
    assert_eq!(handed.terminals, 2);
    ok(&dir, &["send", "h", "again"]);
    until(&dir, "h", "done");
    assert!(ok(&dir, &["read", "h"]).ends_with("> again\nyou said: again\n"));
    ok(&dir, &["send", "l", "hi"]);
    until(&dir, "l", "done");
    ok(&dir, &["kill", "h"]);
    ok(&dir, &["kill", "l"]);
}

#[test]
fn a_lane_that_speaks_an_older_protocol_still_blocks_a_handover() {
    let dir = temp_dir("acp-old-speaks");
    let mut server = Server::start(&dir);
    start_fake(&dir, "old", "fake");
    stop(&mut server);
    let state = Paths::in_dir(&dir).state;
    let mut file: Value =
        serde_json::from_slice(&std::fs::read(&state).expect("state")).expect("JSON");
    file["terminals"][0]["speaks"] = json!(0);
    std::fs::write(&state, file.to_string()).expect("state written");
    let _server = Server::start(&dir);
    let mut conn = Conn::connect(&Paths::in_dir(&dir).socket).expect("connect");
    let build = conn.server().build.clone();
    let refused = conn
        .call::<_, Handover>(Method::Handover, &json!({ "build": build }))
        .expect_err("an older bridge is refused");
    assert!(
        refused
            .to_string()
            .contains("ACP terminal old speaks protocol 0, the binary"),
        "{refused}"
    );
    ok(&dir, &["kill", "old"]);
}
