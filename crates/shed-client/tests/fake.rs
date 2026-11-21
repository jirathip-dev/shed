//! The fake server through the real client: answers, errors, what it saw, streams, and its temp folder.
#![cfg(test)]

use serde_json::{Value, json};
use shed_client::fake::Server;
use shed_client::proto::{ErrorCode, EventsParams, Hello, Method, RpcError, State};
use shed_client::{Calls as _, Error, Shed, Streamed};

fn terminal(name: &str) -> Value {
    json!({"name": name, "labels": {}, "cwd": "/", "cmd": ["sh"], "keep": false,
        "state": "running", "exit": null, "pid": 7})
}

#[test]
fn an_answer_comes_back_typed_and_the_call_is_seen_with_its_params() {
    let fake = Server::start().unwrap();
    fake.answer(Method::Get, terminal("t1"));
    let shed = Shed::at(fake.socket());
    let info = shed.get("t1").unwrap();
    assert_eq!(
        (info.name.as_str(), info.state, info.pid),
        ("t1", State::Running, Some(7))
    );
    assert_eq!(shed.hello().unwrap(), Hello::ours());
    shed.keys("t1", &["Enter"]).ok();
    assert_eq!(
        fake.seen(),
        [
            (Method::Get, json!({"name": "t1"})),
            (Method::Keys, json!({"name": "t1", "keys": ["Enter"]}))
        ]
    );
}

#[test]
fn a_failure_is_the_servers_own_error_and_a_method_not_given_is_a_version_error_naming_it() {
    let fake = Server::start().unwrap();
    fake.fail(Method::Get, RpcError::new(ErrorCode::NoSuchTerminal, "t1"));
    let shed = Shed::at(fake.socket());
    let gone = shed.get("t1").unwrap_err();
    assert_eq!(gone.code(), Some(ErrorCode::NoSuchTerminal));
    let unknown = shed.agents().unwrap_err();
    assert!(
        matches!(&unknown, Error::Version { needs: Some(n), .. } if n == "agents"),
        "{unknown:?}"
    );
    fake.answer(Method::Get, terminal("t1"));
    assert_eq!(shed.get("t1").unwrap().name, "t1");
}

#[test]
fn a_hello_answer_replaces_shed_s_so_a_test_can_play_an_old_server() {
    let fake = Server::start().unwrap();
    fake.answer(
        Method::Hello,
        json!({"product": "shed", "version": "0.0.1", "build": "abc1234", "protocol": 2}),
    );
    let refused = Shed::at(fake.socket()).hello().unwrap_err();
    assert!(matches!(refused, Error::Version { .. }), "{refused:?}");
    assert_eq!(fake.seen(), []);
}

#[test]
fn a_streamed_answer_is_its_items_then_the_end() {
    let fake = Server::start().unwrap();
    let event = json!({"seq": 4, "name": "t1", "labels": {}, "event": "started"});
    fake.answer(
        Method::Events,
        json!([event, {"jsonrpc": "2.0", "method": "reset", "params": {"seq": 9}}]),
    );
    let stream = Shed::at(fake.socket())
        .events(&EventsParams::default())
        .unwrap();
    let items: Vec<_> = stream.collect();
    assert!(
        matches!(&items[..], [Streamed::Item(e), Streamed::Reset(9)] if e.seq == 4),
        "{items:?}"
    );
}

#[test]
fn dropping_it_removes_its_folder_and_two_at_once_get_their_own() {
    let (one, two) = (Server::start().unwrap(), Server::start().unwrap());
    let dir = one.socket().parent().unwrap().to_owned();
    assert_ne!(dir, two.socket().parent().unwrap());
    assert!(one.socket().exists());
    drop(one);
    assert!(!dir.exists());
    assert!(Shed::at(two.socket()).hello().is_ok());
}
