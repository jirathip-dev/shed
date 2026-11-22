use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

use serde_json::{Value, json};
use shed_client::Conn as Client;
use shed_server::Paths;
use shed_server::proto::{
    BUILD, Method, OLDEST_PROTOCOL, PROTOCOL, ReadParams, ReadResult, SendParams, State,
    TerminalInfo,
};

use crate::common::{Server, eventually, temp_dir};
use crate::{Conn, VERSION, ours};

#[test]
fn a_kept_terminal_comes_back_as_a_shell_in_its_cwd() {
    let dir = temp_dir("restart");
    let work = dir.join("work");
    std::fs::create_dir(&work).expect("work dir");
    let paths = Paths::in_dir(&dir);
    let server = Server::start(&dir);
    let mut client = Client::connect(&paths.socket).expect("connect");
    let run = |client: &mut Client, name: &str, keep: bool| -> TerminalInfo {
        let params = json!({
            "name": name, "labels": {"issue": "7"}, "cwd": work, "keep": keep,
            "cmd": ["sh", "-c", format!("echo out-{name}; echo {name} >> ran; exec sleep 100")],
        });
        client.call(Method::Run, &params).expect("run")
    };
    let kept = run(&mut client, "kept", true);
    run(&mut client, "lost", false);
    let ran = || std::fs::read_to_string(work.join("ran")).unwrap_or_default();
    eventually("both tasks ran", || ran().lines().count() == 2);
    run(&mut client, "killed", true);
    eventually("the third task ran", || ran().lines().count() == 3);
    let killed: TerminalInfo = client
        .call(Method::Kill, &json!({"name": "killed"}))
        .expect("kill");
    assert_eq!(killed.state, State::Exited);
    let state_mode = std::fs::metadata(&paths.state)
        .expect("state")
        .permissions()
        .mode();
    assert_eq!(state_mode & 0o777, 0o600);
    let read = |client: &mut Client, name: &str| -> String {
        let read = ReadParams {
            name: name.into(),
            lines: None,
            history: true,
            ..read_params()
        };
        client
            .call::<_, ReadResult>(Method::Read, &read)
            .expect("read")
            .text
    };
    for name in ["kept", "lost"] {
        eventually("the output", || {
            read(&mut client, name) == format!("out-{name}")
        });
    }
    drop(client);
    drop(server);

    let _server = Server::start(&dir);
    let mut client = Client::connect(&paths.socket).expect("reconnect");
    let back: TerminalInfo = client
        .call(Method::Get, &json!({"name": "kept"}))
        .expect("get");
    assert_eq!(back.state, State::Running);
    assert_eq!(back.cmd, ["/bin/sh"]);
    assert_eq!((back.cwd, back.labels), (work.clone(), kept.labels));
    assert_ne!(back.pid, kept.pid);
    eventually("the kept history", || {
        read(&mut client, "kept").starts_with("out-kept\n")
    });
    assert_eq!(
        read(&mut client, "lost"),
        "out-lost",
        "a lost terminal reads its log"
    );
    let lost: TerminalInfo = client
        .call(Method::Get, &json!({"name": "lost"}))
        .expect("get");
    assert_eq!(
        (lost.state, lost.exit, lost.pid),
        (State::Exited, None, None)
    );

    let still: TerminalInfo = client
        .call(Method::Get, &json!({"name": "killed"}))
        .expect("get");
    assert_eq!(
        still, killed,
        "a kept terminal that had exited stays exited"
    );

    let send = SendParams {
        name: "kept".into(),
        text: "echo \"in:$PWD\"".into(),
        ..SendParams::default()
    };
    client.call::<_, Value>(Method::Send, &send).expect("send");
    let want = format!("in:{}", work.display());
    eventually("the shell's pwd", || {
        let read = ReadParams {
            name: "kept".into(),
            lines: None,
            history: false,
            ..read_params()
        };
        let screen: ReadResult = client.call(Method::Read, &read).expect("read");
        screen.text.replace('\n', "").contains(&want)
    });
    assert_eq!(
        ran(),
        "kept\nlost\nkilled\n",
        "a kept task must never re-run"
    );
}

#[test]
fn a_field_the_method_does_not_document_is_refused_on_protocol_3_and_taken_on_2() {
    let dir = temp_dir("fields");
    let _server = Server::start(&dir);
    let get = |protocol: u32| {
        let mut c = Conn::open(&dir);
        let hello = json!({"product": "shed", "version": VERSION, "protocol": protocol});
        c.call(1, "hello", &hello);
        let (_, _, reply) = c.call(2, "get", &json!({"name": "t1", "env": {"K": "v"}}));
        reply["error"]["message"].clone()
    };
    assert_eq!(get(3), "INVALID_PARAMS: unknown field get.env");
    assert_eq!(get(2), "NO_SUCH_TERMINAL: t1");
}

#[test]
fn a_server_serves_its_protocol_and_the_previous_one_and_refuses_the_rest() {
    let dir = temp_dir("version");
    let _server = Server::start(&dir);
    let hello = |protocol: u32, oldest: Option<u32>| {
        let mut c = Conn::open(&dir);
        let mut p = json!({"product": "shed", "version": VERSION, "build": "02fcf97", "protocol": protocol});
        if let Some(oldest) = oldest {
            p["oldest"] = json!(oldest);
        }
        let (_, line, reply) = c.call(1, "hello", &p);
        let (_, _, after) = c.call(2, "ls", &json!({}));
        (line, reply, after)
    };
    let refused = |line: &str, client: &str, side: &str| {
        assert_eq!(
            line,
            format!(
                r#"{{"jsonrpc":"2.0","id":1,"error":{{"code":-32000,"message":"SHED_VERSION: client {client} / server {BUILD} protocol {OLDEST_PROTOCOL}-{PROTOCOL}; update {side}"}}}}"#
            )
        );
    };

    let (_, reply, after) = hello(PROTOCOL, Some(PROTOCOL - 1));
    assert_eq!(reply["result"], ours(), "a client on N gets N's hello");
    assert_eq!(reply["result"]["oldest"], OLDEST_PROTOCOL);
    assert!(after["result"].is_array(), "{after}");

    let (_, reply, after) = hello(PROTOCOL - 1, None);
    let mut previous = ours();
    previous["protocol"] = json!(PROTOCOL - 1);
    previous.as_object_mut().expect("object").remove("oldest");
    assert_eq!(
        reply["result"], previous,
        "a client on N-1 gets N-1's hello"
    );
    assert!(after["result"].is_array(), "{after}");

    let (_, reply, _) = hello(PROTOCOL + 1, Some(PROTOCOL));
    assert_eq!(reply["result"], ours(), "a newer client that accepts N");

    let (line, _, after) = hello(OLDEST_PROTOCOL - 1, None);
    refused(
        &line,
        &format!("02fcf97 protocol {0}-{0}", OLDEST_PROTOCOL - 1),
        "the client",
    );
    assert_eq!(
        after["error"]["code"], -32001,
        "a refused client stays refused"
    );

    let (line, _, _) = hello(PROTOCOL + 2, Some(PROTOCOL + 1));
    refused(
        &line,
        &format!("02fcf97 protocol {}-{}", PROTOCOL + 1, PROTOCOL + 2),
        "the server and restart shed serve",
    );

    let mut old = Conn::open(&dir);
    let (_, line, _) = old.call(1, "hello", &json!({"version": VERSION}));
    refused(&line, "unnamed unknown protocol 0-0", "the client");

    let mut same = Conn::open(&dir);
    let other_build =
        json!({"product": "shed", "version": "9.9.9", "build": "56de739", "protocol": PROTOCOL});
    let (_, _, reply) = same.call(1, "hello", &other_build);
    assert_eq!(
        reply["result"],
        ours(),
        "same protocol, other build: accepted"
    );
}

#[test]
fn rpc_discover_answers_the_openrpc_document_before_and_after_hello() {
    let dir = temp_dir("discover");
    let _server = Server::start(&dir);
    let committed = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/shed.openrpc.json");
    let committed = std::fs::read_to_string(committed).expect("docs/shed.openrpc.json");
    let committed: Value = serde_json::from_str(&committed).expect("the document is JSON");
    let mut c = Conn::open(&dir);
    assert_eq!(c.result("rpc.discover", &json!({})), committed);
    c.result("hello", &ours());
    assert_eq!(c.result("rpc.discover", &json!({})), committed);
}

fn read_params() -> ReadParams {
    serde_json::from_value(serde_json::json!({"name": ""})).expect("read params")
}
