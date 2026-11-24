use serde_json::{Value, json};

use crate::common::{Server, temp_dir};
use crate::{events_until, ok, reported, shed, start_fake};

#[test]
fn an_expired_login_ends_the_turn_as_an_auth_error() {
    let dir = temp_dir("acp-auth");
    let _server = Server::start(&dir);
    start_fake(&dir, "x", "claude-acp");
    let line = "Failed to authenticate: OAuth session expired and could not be refreshed";
    ok(&dir, &["send", "x", "auth"]);
    let until = ["wait", "x", "--until", "done", "waiting-approval", "error"];
    assert_eq!(
        ok(&dir, &[&until[..], &["--timeout", "10"]].concat()),
        "x error\n"
    );
    assert_eq!(
        reported(&dir, "x"),
        json!({"state": "error", "note": format!("auth: {line}"), "from": "reported"})
    );
    let events = events_until(&dir, "x", |e| e["event"] == "reply");
    let reply: Value = serde_json::from_str(events.last().expect("a reply")).expect("JSON");
    assert_eq!(reply["text"], line);
    assert_eq!(reply["stop_reason"], "auth_error");
    start_fake(&dir, "y", "fake");
    ok(&dir, &["send", "y", "auth"]);
    let out = shed(
        &dir,
        &["wait", "y", "--until", "done", "error", "--timeout", "1"],
    );
    assert_eq!(out.status.code(), Some(124), "{out:?}");
}
