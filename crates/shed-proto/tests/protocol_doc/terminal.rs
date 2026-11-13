use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::{Value, json, to_value};
use shed_proto::{
    KeysParams, LsParams, Method, Parent, ReadParams, ReadResult, Reported, ResizeParams,
    Resources, RunParams, SendParams, State, StateParams, Target, TerminalInfo, WaitOutcome,
    WaitParams, WaitResult,
};

pub(crate) fn info(state: State) -> TerminalInfo {
    TerminalInfo {
        name: "build".into(),
        labels: BTreeMap::from([("issue".into(), "7".into()), ("role".into(), "impl".into())]),
        cwd: PathBuf::from("/home/user/project"),
        cmd: vec!["make".into(), "test".into()],
        keep: true,
        state,
        exit: (state == State::Exited).then_some(0),
        pid: (state == State::Running).then_some(4242),
        reported: None,
        records: Vec::new(),
        idle_after: None,
        resources: (state == State::Running).then_some(RESOURCES),
        acp: None,
        session: None,
        parent: Some(lead()),
    }
}

fn lead() -> Parent {
    Parent {
        name: "lead".into(),
        outlive: false,
        notify: true,
    }
}

pub(crate) const RESOURCES: Resources = Resources {
    rss_mb: 52,
    cpu_pct: 3,
};

pub(crate) fn done() -> Reported {
    Reported {
        state: "done".into(),
        note: Some("tests pass".into()),
        ..Reported::default()
    }
}

fn asking() -> TerminalInfo {
    TerminalInfo {
        reported: Some(Reported {
            state: "blocked".into(),
            kind: Some("question".into()),
            progress: Some(40),
            app: Some("make".into()),
            title: Some("Docs".into()),
            msg: Some("Overwrite config.toml?".into()),
            ..Reported::default()
        }),
        records: vec![Reported {
            state: "working".into(),
            id: Some("build/tests".into()),
            ..Reported::default()
        }],
        ..info(State::Running)
    }
}

fn target() -> Value {
    to_value(Target {
        name: "build".into(),
    })
    .unwrap()
}

pub(crate) fn run_params() -> Value {
    to_value(RunParams {
        name: Some("build".into()),
        labels: info(State::Running).labels,
        cwd: PathBuf::from("/home/user/project"),
        keep: true,
        idle_after: None,
        cmd: vec!["make".into(), "test".into()],
        acp: None,
        policy: None,
        resume: None,
        mcp_servers: Vec::new(),
        meta: None,
        no_hibernate: false,
        machine: None,
        parent: Some(lead()),
        rows: None,
        cols: None,
    })
    .unwrap()
}

fn read_example() -> (Value, Value) {
    (
        to_value(ReadParams {
            name: "build".into(),
            lines: Some(2),
            history: false,
            events: false,
            before: None,
            n: None,
        })
        .unwrap(),
        to_value(ReadResult {
            text: "ok 12 tests\n$".into(),
            events: Vec::new(),
        })
        .unwrap(),
    )
}

fn send_example() -> (Value, Value) {
    let params = SendParams {
        name: "build".into(),
        text: "make lint".into(),
        ..SendParams::default()
    };
    (to_value(params).unwrap(), json!({}))
}

pub(crate) fn example(method: Method) -> Option<(Value, Value)> {
    let running = || to_value(info(State::Running)).unwrap();
    let exited = || to_value(info(State::Exited)).unwrap();
    Some(match method {
        Method::Run => (run_params(), running()),
        Method::Ls => (
            to_value(LsParams {
                labels: BTreeMap::from([("issue".into(), "7".into())]),
                parent: Some("lead".into()),
            })
            .unwrap(),
            json!([running()]),
        ),
        Method::Get | Method::Attach => (target(), running()),
        Method::Read => read_example(),
        Method::Send => send_example(),
        Method::Keys => (
            to_value(KeysParams {
                name: "build".into(),
                keys: vec!["ctrl+c".into(), "enter".into()],
            })
            .unwrap(),
            json!({}),
        ),
        Method::Resize => (
            to_value(ResizeParams {
                name: "build".into(),
                rows: 50,
                cols: 120,
            })
            .unwrap(),
            json!({}),
        ),
        Method::Kill | Method::Rm => (target(), exited()),
        Method::State => (
            to_value(StateParams {
                name: "build".into(),
                report: done(),
            })
            .unwrap(),
            to_value(TerminalInfo {
                reported: Some(done()),
                ..info(State::Running)
            })
            .unwrap(),
        ),
        Method::Wait => (
            to_value(WaitParams {
                name: "build".into(),
                children: None,
                until: vec!["done".into(), "blocked".into()],
                kind: Some("question".into()),
                output: Some("^ok \\d+ tests$".into()),
                timeout: Some(600.0),
            })
            .unwrap(),
            to_value(WaitResult {
                outcome: WaitOutcome::Matched,
                terminal: asking(),
            })
            .unwrap(),
        ),
        _ => return None,
    })
}
