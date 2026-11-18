use std::collections::BTreeMap;

use serde_json::{Value, json, to_value};
use shed_proto::{
    AcpSession, AcpUsage, Agent, AgentInfo, ApprovalOption, Cache, EmitParams, Event, EventKind,
    EventsParams, Hooks, MatchParams, Matched, Medians, Method, ModelParams, Provider, Quota, Rule,
    State, Tokens,
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
        turn_ended: Some(1_763_337_600),
        cache_lifetime: Some(3600),
        cache_likely: Some("warm".into()),
        ready_ms: Some(2400),
        first_token_ms: Some(1800),
    }
}

fn usage() -> AcpUsage {
    AcpUsage {
        used: 53_000,
        size: 200_000,
        cost: Some(json!({"amount": 0.045, "currency": "USD"})),
        turn: Some(Tokens {
            input: 100,
            cache_read: 900,
            cache_write: 1000,
            output: 50,
        }),
        total: Some(Tokens {
            input: 200,
            cache_read: 1800,
            cache_write: 2000,
            output: 100,
        }),
    }
}

pub(crate) fn events() -> Vec<Event> {
    [
        EventKind::Started,
        EventKind::State(Box::new(done())),
        EventKind::Resources(RESOURCES),
        EventKind::MemLimit {
            limit_mb: 4096,
            process: "cc1plus".into(),
            rss_mb: 4301,
            ended_by: "shed".into(),
        },
        EventKind::Exited { code: Some(0) },
        EventKind::Killed,
        EventKind::Turn {
            prompt: "Fix the failing test".into(),
            agent: Vec::new(),
        },
        EventKind::Text {
            text: "Looking at the test".into(),
        },
        EventKind::ToolCall {
            id: "call_1".into(),
            title: "Edit src/lib.rs".into(),
            status: "pending".into(),
            kind: Some("edit".into()),
            locations: vec!["/work/src/lib.rs".into()],
            raw_input: Some(json!({"file_path": "/work/src/lib.rs"})),
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
            id: "7".into(),
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
            call: Some(json!({"toolCallId": "call_1", "kind": "edit",
                "locations": [{"path": "/work/src/lib.rs"}]})),
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
        Method::Agents => (json!({}), to_value(vec![agent_info()]).unwrap()),
        Method::AgentsMatch => (
            to_value(MatchParams {
                id: "fake-acp".into(),
                text: "working…\nfake: limit hit, back at 3pm.".into(),
            })
            .unwrap(),
            to_value(Matched {
                limit: Some("fake: limit hit, back at 3pm.".into()),
                resets: Some("3pm".into()),
                auth: None,
            })
            .unwrap(),
        ),
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

fn agent_info() -> AgentInfo {
    let strings = |s: &[&str]| s.iter().map(|&s| s.to_owned()).collect();
    let provider = Provider {
        id: "fake-acp".into(),
        agent: "fake".into(),
        auth_failures: strings(&["fake: login gone"]),
        limit: strings(&["fake: limit hit"]),
        resets: strings(&["back at "]),
        hooks: Some(Hooks {
            file: ".fake/settings.json".into(),
            events: vec![["Stop".into(), "done".into()]],
        }),
        login: Some("fake login".into()),
        status: Some("fake login status".into()),
        logout: Some("fake logout".into()),
        config_dir_var: Some("FAKE_HOME".into()),
        settings_var: None,
        effort_args: strings(&["--effort", "{effort}"]),
        efforts: strings(&["low", "high"]),
        rules: vec![Rule {
            id: "trust".into(),
            state: "blocked".into(),
            kind: Some("permission".into()),
            priority: 10,
            region: 20,
            any: strings(&["Trust this folder?"]),
            version: "0.1.0".into(),
        }],
        quota: Some(Quota {
            path: strings(&["quota", "token_count"]),
            input: "inputTokens".into(),
            cache_read: "cachedInputTokens".into(),
            cache_write: "cachedWriteTokens".into(),
            output: "outputTokens".into(),
        }),
        cache: Some(Cache {
            plan: 3600,
            api: 300,
            api_vars: strings(&["FAKE_API_KEY"]),
        }),
    };
    AgentInfo {
        id: "fake-acp".into(),
        source: Some("/home/me/.config/shed/providers/fake.toml".into()),
        provider: Some(provider),
        installed: Some(Agent {
            id: "fake-acp".into(),
            version: "0.1.0".into(),
            distribution: "npx".into(),
            cmd: strings(&["npx", "-y", "fake-acp@0.1.0"]),
        }),
        medians: Some(Medians {
            sessions: 20,
            ready_ms: 2500,
            first_token_ms: 1900,
        }),
    }
}
