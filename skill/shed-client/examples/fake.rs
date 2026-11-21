#![doc = "A test against `shed_client::fake::Server` (the `fake` feature): give it answers, check what it saw."]

use std::io;

use serde_json::json;
use shed_client::fake::Server;
use shed_client::proto::{ErrorCode, Method, RpcError, State};
use shed_client::{Calls as _, Error, Shed};

fn running(shed: &Shed, name: &str) -> Result<bool, Error> {
    Ok(shed.get(name)?.state == State::Running)
}

fn main() -> io::Result<()> {
    let fake = Server::start()?;
    let terminal = json!({"name": "t1", "labels": {}, "cwd": "/", "cmd": ["sh"], "keep": false,
        "state": "running", "exit": null, "pid": 1});
    fake.answer(Method::Get, terminal);
    let shed = Shed::at(fake.socket());
    assert_eq!(running(&shed, "t1").ok(), Some(true));
    assert_eq!(fake.seen(), [(Method::Get, json!({"name": "t1"}))]);
    fake.fail(Method::Kill, RpcError::new(ErrorCode::NoSuchTerminal, "t1"));
    assert_eq!(shed.close("t1").ok(), Some(()));
    Ok(())
}
