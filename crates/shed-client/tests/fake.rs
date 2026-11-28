//! The fake server through the real client: answers, errors, what it saw, streams, and its temp folder.
#![cfg(test)]

use serde_json::{Value, json};
use shed_client::fake::Server;
use shed_client::proto::{
    ErrorCode, Event, EventKind, EventsParams, ExecParams, ExecResult, FileParams, Hello, LsParams,
    Method, RpcError, RunParams, State, TerminalInfo,
};
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

fn events(shed: &Shed, since: Option<u64>) -> shed_client::Stream<Event> {
    let params = EventsParams {
        since,
        ..EventsParams::default()
    };
    shed.events(&params).unwrap()
}

fn next(stream: &mut shed_client::Stream<Event>) -> Event {
    match stream.next() {
        Some(Streamed::Item(e)) => e,
        other => panic!("{other:?}"),
    }
}

fn run(shed: &Shed, name: &str, labels: &[(&str, &str)]) -> TerminalInfo {
    let labels = labels.iter().map(|(k, v)| ((*k).into(), (*v).into()));
    shed.run(&RunParams {
        name: Some(name.into()),
        labels: labels.collect(),
        cmd: vec!["agent".into()],
        ..RunParams::default()
    })
    .unwrap()
}

#[test]
fn a_handler_answers_from_the_shared_state_over_the_built_in_answer() {
    let fake = Server::start().unwrap();
    let shed = Shed::at(fake.socket());
    run(&shed, "t1", &[]);
    fake.handle(Method::Agents, |world, _| {
        let names: Vec<_> = world.terminals.keys().map(|n| json!({"id": n})).collect();
        Ok(json!(names))
    });
    fake.handle(Method::Get, |world, params| {
        let name = params["name"].as_str().unwrap_or_default();
        let info = world.terminal(name)?;
        info.labels.insert("seen".into(), "1".into());
        Ok(json!(info))
    });
    assert_eq!(shed.agents().unwrap()[0].id, "t1");
    assert_eq!(shed.get("t1").unwrap().labels["seen"], "1");
    assert_eq!(
        shed.ls(&LsParams::default()).unwrap()[0].labels["seen"],
        "1"
    );
    let gone = shed.get("t2").unwrap_err();
    assert_eq!(gone.code(), Some(ErrorCode::NoSuchTerminal));
}

#[test]
fn the_run_lifecycle_is_shed_s_run_state_kill_rm_with_its_events() {
    let fake = Server::start().unwrap();
    let shed = Shed::at(fake.socket());
    let mut stream = events(&shed, None);
    let info = run(&shed, "t1", &[("fleet", "a")]);
    assert_eq!((info.state, info.pid), (State::Running, Some(1)));
    let taken = shed.run(&RunParams {
        name: Some("t1".into()),
        ..RunParams::default()
    });
    assert_eq!(taken.unwrap_err().code(), Some(ErrorCode::NameTaken));
    let started = next(&mut stream);
    assert_eq!((started.seq, started.kind), (1, EventKind::Started));
    assert_eq!(started.labels["fleet"], "a");
    let mut only = LsParams::default();
    only.labels.insert("fleet".into(), "b".into());
    assert_eq!(shed.ls(&only).unwrap(), []);
    let still = shed.rm("t1").unwrap_err();
    assert_eq!(still.code(), Some(ErrorCode::StillRunning));
    let killed = shed.kill("t1").unwrap();
    assert_eq!((killed.state, killed.pid), (State::Exited, None));
    assert_eq!(next(&mut stream).kind, EventKind::Killed);
    shed.close("t1").unwrap();
    assert_eq!(shed.ls(&LsParams::default()).unwrap(), []);
}

#[test]
fn a_cued_agent_replies_to_each_prompt_in_turn_repeating_its_last() {
    let fake = Server::start().unwrap();
    let shed = Shed::at(fake.socket());
    run(&shed, "t1", &[]);
    fake.with(|world| world.cue("t1", &["one", "two"]));
    let mut stream = events(&shed, Some(1));
    let mut turn = |prompt: &str| {
        shed.prompt("t1", prompt).unwrap();
        let kinds: Vec<_> = (0..5).map(|_| next(&mut stream).kind).collect();
        kinds
    };
    let reply = |text: &str| EventKind::Reply {
        text: text.into(),
        stop_reason: "end_turn".into(),
    };
    for (prompt, text) in [("hi", "one"), ("and", "two"), ("again", "two")] {
        let kinds = turn(prompt);
        assert!(
            matches!(&kinds[..], [EventKind::State(working), EventKind::Turn { prompt: said, .. },
                EventKind::Text { text: chunk }, whole, EventKind::State(done)]
                if working.state == "working" && said == prompt && chunk == text
                    && *whole == reply(text) && done.state == "done"),
            "{kinds:?}"
        );
    }
    assert_eq!(shed.get("t1").unwrap().reported.unwrap().state, "done");
}

#[test]
fn a_live_stream_gets_what_the_test_pushes_until_the_run_ends_and_the_test_closes_it() {
    let fake = Server::start().unwrap();
    let shed = Shed::at(fake.socket());
    run(&shed, "t1", &[("fleet", "a")]);
    run(&shed, "t2", &[]);
    let wanted = EventsParams {
        labels: [("fleet".into(), "a".into())].into(),
        since: Some(0),
    };
    let mut stream = shed.events(&wanted).unwrap();
    assert_eq!(next(&mut stream).name, "t1");
    fake.with(|world| world.emit("t2", EventKind::Killed));
    let text = EventKind::Text { text: "hi".into() };
    let seq = fake.with(|world| world.emit("t1", text.clone()));
    let pushed = next(&mut stream);
    assert_eq!((pushed.seq, pushed.kind), (seq, text));
    fake.with(|world| world.exit("t1", 0)).unwrap();
    assert_eq!(next(&mut stream).kind, EventKind::Exited { code: Some(0) });
    assert_eq!(shed.get("t1").unwrap().exit, Some(0));
    fake.with(shed_client::fake::World::close_streams);
    assert_eq!(stream.next(), None);
}

#[test]
fn a_restart_ends_streams_but_keeps_terminals_and_a_forgotten_ring_resets_old_resumes() {
    let fake = Server::start().unwrap();
    let shed = Shed::at(fake.socket());
    run(&shed, "t1", &[]);
    let mut stream = events(&shed, None);
    fake.restart();
    assert_eq!(stream.next(), None);
    assert_eq!(shed.get("t1").unwrap().state, State::Running);
    let floor = fake.with(|world| world.forget(10));
    assert_eq!(floor, 11);
    let mut resumed = events(&shed, Some(1));
    assert_eq!(resumed.next(), Some(Streamed::Reset(11)));
    fake.with(|world| world.emit("t1", EventKind::Killed));
    assert_eq!(next(&mut resumed).seq, 12);
    assert!(
        fake.seen()
            .contains(&(Method::Events, json!({"labels": {}, "since": 1})))
    );
}

#[test]
fn exec_answers_the_program_it_was_given_and_files_are_written_then_read() {
    let fake = Server::start().unwrap();
    let shed = Shed::at(fake.socket());
    let done = ExecResult {
        code: 0,
        stdout: b"main\n".to_vec(),
        stderr: Vec::new(),
    };
    fake.with(|world| world.on_exec(&["git", "branch"], done.clone()));
    let exec = |cmd: &[&str]| {
        shed.exec(&ExecParams {
            cmd: cmd.iter().map(|c| (*c).into()).collect(),
            ..ExecParams::default()
        })
    };
    assert_eq!(exec(&["git", "branch"]).unwrap(), done);
    let failed = exec(&["git", "log"]).unwrap_err();
    assert_eq!(failed.code(), Some(ErrorCode::SpawnFailed));
    let file = |data: Option<&[u8]>| FileParams {
        path: "/w/a.txt".into(),
        data: data.map(<[u8]>::to_vec),
        ..FileParams::default()
    };
    let missing = shed.file_read(&file(None)).unwrap_err();
    assert_eq!(missing.code(), Some(ErrorCode::Io));
    shed.file_write(&file(Some(b"hi"))).unwrap();
    assert_eq!(shed.file_read(&file(None)).unwrap(), b"hi");
    assert_eq!(fake.with(|world| world.files.len()), 1);
}
