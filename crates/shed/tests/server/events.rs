use serde_json::{Value, json};

use crate::common::{Server, temp_dir};
use crate::{Conn, hello, ours};

#[test]
fn events_stream_in_seq_order_filter_by_label_and_replay_since() {
    let dir = temp_dir("events");
    let _server = Server::start(&dir);
    let mut c = Conn::open(&dir);
    c.result("hello", &ours());
    let mut live = Conn::events(&dir, &json!({"labels": {"test": "order"}}));

    let script = "printf '\\033]7501;state=working\\007'; read x; exit 3";
    let run = |name: &str, labels: Value, cmd: Value| json!({"name": name, "labels": labels, "cwd": dir, "cmd": cmd});
    c.result("run", &run("other", json!({}), json!(["sleep", "100"])));
    c.result(
        "run",
        &run("a", json!({"test": "order"}), json!(["sh", "-c", script])),
    );
    let started = live.event();
    assert_eq!(
        (&started["event"], &started["name"]),
        (&json!("started"), &json!("a"))
    );
    assert_eq!(started["labels"], json!({"test": "order"}));
    let state = live.event();
    assert_eq!(
        (&state["event"], &state["state"]),
        (&json!("state"), &json!("working"))
    );
    c.result("send", &json!({"name": "a", "text": "\n"}));
    let exited = live.event();
    assert_eq!(
        (&exited["event"], &exited["code"]),
        (&json!("exited"), &json!(3))
    );

    c.result(
        "run",
        &run("b", json!({"test": "order"}), json!(["sleep", "100"])),
    );
    assert_eq!(live.event()["event"], "started");
    c.result("kill", &json!({"name": "b"}));
    let killed = live.event();
    assert_eq!(
        (&killed["event"], &killed["name"]),
        (&json!("killed"), &json!("b"))
    );

    assert_eq!(started["seq"], 2);
    assert_eq!(state["seq"], 3);
    assert_eq!(exited["seq"], 4);
    assert_eq!(killed["seq"], 6);

    let mut all = Conn::events(&dir, &json!({"since": 0}));
    let seqs: Vec<Value> = (0..6).map(|_| all.event()["seq"].clone()).collect();
    assert_eq!(seqs, [1, 2, 3, 4, 5, 6].map(|n| json!(n)));
    let mut since = Conn::events(&dir, &json!({"since": 3, "labels": {"test": "order"}}));
    let replay: Vec<Value> = (0..3).map(|_| since.event()["seq"].clone()).collect();
    assert_eq!(replay, [4, 5, 6].map(|n| json!(n)));

    c.result(
        "run",
        &run("c", json!({"test": "order"}), json!(["sleep", "100"])),
    );
    for stream in [&mut since, &mut live, &mut all] {
        assert_eq!(
            stream.event()["seq"],
            7,
            "after the replay, the live stream"
        );
    }
}

#[test]
fn jan_osc_9999_states_are_reported_with_their_source_and_garbled_ones_ignored() {
    let dir = temp_dir("jan");
    let _server = Server::start(&dir);
    let mut c = Conn::open(&dir);
    c.result("hello", &ours());
    let mut live = Conn::events(&dir, &json!({}));
    let jan = |state: &str| format!(r#"\033]9999;{{"agentType":"jan","state":"{state}"}}\007"#);
    let script = format!(
        "printf 'a{}b'; read x; printf '{}{}{}{}c'; read x",
        jan("working"),
        r#"\033]9999;{"agentType":"jan","sta\007"#,
        r"\033]9999;not json\007",
        jan("sleeping"),
        jan("done"),
    );
    let params = json!({"name": "jan", "cwd": dir, "cmd": ["sh", "-c", script]});
    c.result("run", &params);
    assert_eq!(live.event()["event"], "started");
    let state = |e: Value| {
        (
            e["state"].clone(),
            e["state_source"].clone(),
            e["event"].clone(),
        )
    };
    let working = (json!("working"), json!("jan-osc9999"), json!("state"));
    assert_eq!(state(live.event()), working);
    c.result("send", &json!({"name": "jan", "text": ""}));
    let done = (json!("done"), json!("jan-osc9999"), json!("state"));
    assert_eq!(state(live.event()), done);
    let info = c.result("get", &json!({"name": "jan"}));
    assert_eq!(
        info["reported"],
        json!({"state": "done", "state_source": "jan-osc9999", "from": "reported"})
    );
    let read = c.result("read", &json!({"name": "jan"}));
    assert_eq!(read["text"], "ab\nc", "no escape reaches the screen");
}

#[test]
fn osc_7501_state_events_carry_the_keys_and_each_id() {
    let dir = temp_dir("osc7501-events");
    let _server = Server::start(&dir);
    let mut c = hello(&dir);
    let mut live = Conn::events(&dir, &json!({}));
    let script = "printf '\\033]7501;state=blocked:kind=auth:progress=7:app=gh:title=TG9naW4=\\007\
                  \\033]7501;state=working:id=sub/a\\007'; read x";
    c.result(
        "run",
        &json!({"name": "e", "cwd": dir, "cmd": ["sh", "-c", script]}),
    );
    assert_eq!(live.event()["event"], "started");
    let keys = |e: Value| {
        let mut e = e.as_object().cloned().expect("an object");
        e.retain(|k, _| !["seq", "name", "labels", "time"].contains(&k.as_str()));
        Value::Object(e)
    };
    let root = json!({"event": "state", "state": "blocked", "kind": "auth", "progress": 7, "app": "gh", "title": "Login", "from": "reported"});
    let sub = json!({"event": "state", "state": "working", "id": "sub/a"});
    let got = [keys(live.event()), keys(live.event())];
    assert_eq!(got, [root, sub]);
}
