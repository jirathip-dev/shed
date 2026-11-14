use std::collections::BTreeMap;

use serde_json::{Value, json, to_value};
use shed_proto::{
    AcpSession, AcpUsage, ApprovalOption, EmitParams, Event, EventKind, EventsParams, Method,
    ModelParams, State,
};

use crate::terminal::{RESOURCES, done, info};

fn session() -> AcpSession {
    AcpSession {
        id: "sess_1".into(),
        model: Some("opus".into()),
        models: vec![
            ["opus".into(), "Opus".into()],
            ["haiku".into(), "Haiku".into()],
        ],
        mcp_servers: vec![
            serde_json::json!({"headers": [{"name": "Authorization", "value": "***"}],
            "name": "web", "type": "http", "url": "https://mcp.example/"}),
        ],
        commands: vec![["review".into(), "Review the diff".into(), "files".into()]],
        usage: Some(usage()),
    }
}

fn usage() -> AcpUsage {
    AcpUsage {
        used: 53_000,
        size: 200_000,
        cost: Some(json!({"amount": 0.045, "currency": "USD"})),
    }
}

pub(crate) fn events() -> Vec<Event> {
    [
        EventKind::Started,
        EventKind::State(Box::new(done())),
        EventKind::Resources(RESOURCES),
        EventKind::Exited { code: Some(0) },
        EventKind::Killed,
        EventKind::Turn {
            prompt: "Fix the failing test".into(),
        },
        EventKind::Text {
            text: "Looking at the test".into(),
        },
        EventKind::ToolCall {
            id: "call_1".into(),
            title: "Edit src/lib.rs".into(),
            status: "pending".into(),
            kind: Some("edit".into()),
        },
        EventKind::Plan {
            entries: vec![
                json!({"content": "Fix the test", "priority": "high", "status": "in_progress"}),
            ],
        },
        EventKind::Mode {
            mode: "default".into(),
        },
        EventKind::ApprovalRequest {
            tool: "Edit src/lib.rs".into(),
            options: vec![
                ApprovalOption {
                    id: "allow".into(),
                    name: "Allow".into(),
                    kind: "allow_once".into(),
                },
                ApprovalOption {
                    id: "reject".into(),
                    name: "Reject".into(),
                    kind: "reject_once".into(),
                },
            ],
        },
        EventKind::Approved {
            option: "allow".into(),
        },
        EventKind::Reply {
            text: "Fixed; all 12 tests pass.".into(),
            stop_reason: "end_turn".into(),
        },
        EventKind::Session(Box::new(session())),
        EventKind::Usage(usage()),
    ]
    .into_iter()
    .zip(1..)
    .map(|(kind, seq)| Event {
        seq,
        name: "build".into(),
        labels: info(State::Running).labels,
        parent: Some("lead".into()),
        kind,
        replay: false,
    })
    .collect()
}

fn events_params() -> Value {
    to_value(EventsParams {
        labels: BTreeMap::from([("issue".into(), "7".into())]),
        since: Some(120),
    })
    .unwrap()
}

pub(crate) fn emit_params() -> Value {
    to_value(EmitParams {
        name: "agent".into(),
        kind: EventKind::Reply {
            text: "All 12 tests pass.".into(),
            stop_reason: "end_turn".into(),
        },
        replay: false,
    })
    .unwrap()
}

#[test]
fn emit_params_round_trip() {
    let p: EmitParams = serde_json::from_value(emit_params()).unwrap();
    assert_eq!(p.name, "agent");
    assert!(p.kind.is_acp());
}

pub(crate) fn example(method: Method) -> Option<(Value, Value)> {
    Some(match method {
        Method::Events => (events_params(), json!({})),
        Method::Emit => (emit_params(), json!({})),
        Method::Model => (
            to_value(ModelParams {
                name: "agent".into(),
                model: "haiku".into(),
            })
            .unwrap(),
            json!({}),
        ),
        _ => return None,
    })
}
