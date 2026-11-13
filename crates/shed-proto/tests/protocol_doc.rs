//! `docs/protocol.md` is generated from the types; this test fails when it drifts.
//! Regenerate with `SHED_BLESS=1 cargo test -p shed-proto`.
#![cfg(test)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json, to_value};
use shed_proto::{
    AcpSession, AcpUsage, ApprovalOption, Color, EmitParams, ErrorCode, Event, EventKind,
    EventsParams, ExecParams, ExecResult, FileParams, Hello, KeysParams, Line, Link, LinkState,
    LsParams, MachineStats, Method, ModelParams, ReadParams, ReadResult, Reported, Request,
    ResizeParams, Resources, Response, RpcError, Run, RunParams, Sample, ScreenUpdate, SendParams,
    State, StateParams, StatsParams, Target, TerminalInfo, TerminalParams, TerminalResizeParams,
    VERSION, WaitOutcome, WaitParams, WaitResult, WatchParams, openrpc,
};

fn info(state: State) -> TerminalInfo {
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
        idle_after: None,
        resources: (state == State::Running).then_some(RESOURCES),
        acp: None,
        session: None,
    }
}

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

const RESOURCES: Resources = Resources {
    rss_mb: 52,
    cpu_pct: 3,
};

fn events() -> Vec<Event> {
    [
        EventKind::Started,
        EventKind::State(done()),
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
        kind,
        replay: false,
    })
    .collect()
}

fn screen_updates() -> [ScreenUpdate; 2] {
    let plain = |text: &str| Run {
        text: text.into(),
        fg: None,
        bg: None,
        bold: false,
        italic: false,
        underline: false,
        inverse: false,
    };
    let ok = Run {
        fg: Some(Color::Index(2)),
        bold: true,
        ..plain("ok")
    };
    let first = ScreenUpdate {
        seq: 41,
        full: true,
        rows: 2,
        cols: 40,
        cursor: [1, 2],
        scrollback: 120,
        lines: vec![
            Line {
                row: 0,
                runs: vec![ok, plain(" 12 tests")],
            },
            Line {
                row: 1,
                runs: vec![plain("$")],
            },
        ],
    };
    let typed = Run {
        bg: Some(Color::Rgb([40, 40, 40])),
        ..plain("make")
    };
    let next = ScreenUpdate {
        seq: 44,
        full: false,
        cursor: [1, 6],
        lines: vec![Line {
            row: 1,
            runs: vec![plain("$ "), typed],
        }],
        ..first.clone()
    };
    [first, next]
}

fn done() -> Reported {
    Reported {
        state: "done".into(),
        note: Some("tests pass".into()),
        state_source: None,
    }
}

fn target() -> Value {
    to_value(Target {
        name: "build".into(),
    })
    .unwrap()
}

fn events_params() -> Value {
    to_value(EventsParams {
        labels: BTreeMap::from([("issue".into(), "7".into())]),
        since: Some(120),
    })
    .unwrap()
}

fn run_params() -> Value {
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
        rows: None,
        cols: None,
    })
    .unwrap()
}

fn emit_params() -> Value {
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

fn example(method: Method) -> (Value, Value) {
    let running = || to_value(info(State::Running)).unwrap();
    let exited = || to_value(info(State::Exited)).unwrap();
    match method {
        Method::Hello => {
            let hello = to_value(Hello::ours()).unwrap();
            (hello.clone(), hello)
        }
        Method::Run => (run_params(), running()),
        Method::Ls => (
            to_value(LsParams {
                labels: BTreeMap::from([("issue".into(), "7".into())]),
            })
            .unwrap(),
            json!([running()]),
        ),
        Method::Get | Method::Attach => (target(), running()),
        Method::Read => read_example(),
        Method::Send => (
            to_value(SendParams {
                name: "build".into(),
                text: "make lint\n".into(),
            })
            .unwrap(),
            json!({}),
        ),
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
                until: vec!["done".into(), "blocked".into()],
                output: Some("^ok \\d+ tests$".into()),
                timeout: Some(600.0),
            })
            .unwrap(),
            to_value(WaitResult {
                outcome: WaitOutcome::Matched,
                terminal: TerminalInfo {
                    reported: Some(done()),
                    ..info(State::Running)
                },
            })
            .unwrap(),
        ),
        Method::Events => (events_params(), json!({})),
        Method::Emit => (emit_params(), json!({})),
        Method::ScreenWatch
        | Method::Input
        | Method::TerminalKeys
        | Method::TerminalResize
        | Method::ReadOnly => (terminal_params(method), json!({})),
        Method::MachineStats => stats_example(),
        Method::Model => (
            to_value(ModelParams {
                name: "agent".into(),
                model: "haiku".into(),
            })
            .unwrap(),
            json!({}),
        ),
        Method::MachineLinks => links_example(),
        Method::Exec | Method::FileRead | Method::FileWrite => exec_example(method),
        Method::Discover => {
            let doc = openrpc();
            (
                json!({}),
                json!({"openrpc": doc["openrpc"], "info": doc["info"]}),
            )
        }
    }
}

fn exec_example(method: Method) -> (Value, Value) {
    match method {
        Method::Exec => (
            to_value(ExecParams {
                cmd: vec!["git".into(), "status".into(), "--short".into()],
                cwd: Some(PathBuf::from("/home/user/project")),
                timeout: Some(30.0),
                stdin: Vec::new(),
                machine: Some("box".into()),
            })
            .unwrap(),
            to_value(ExecResult {
                code: 0,
                stdout: b" M src/lib.rs\n".to_vec(),
                stderr: Vec::new(),
            })
            .unwrap(),
        ),
        Method::FileRead => (
            to_value(FileParams {
                path: PathBuf::from("/home/user/project/VERSION"),
                data: None,
                machine: None,
            })
            .unwrap(),
            json!(b"1.2.0\n"),
        ),
        Method::FileWrite => (
            to_value(FileParams {
                path: PathBuf::from("/home/user/project/VERSION"),
                data: Some(b"1.3.0\n".to_vec()),
                machine: None,
            })
            .unwrap(),
            json!({}),
        ),
        _ => unreachable!("{method:?}"),
    }
}

fn links_example() -> (Value, Value) {
    let link = |state, error: Option<&str>| Link {
        state,
        since: 1_762_416_000,
        error: error.map(str::to_owned),
    };
    let refused = "SHED_VERSION: shed 0.1.0 here, 0.2.0 on old; install the same version on both";
    let links = BTreeMap::from([
        ("box", link(LinkState::Up, None)),
        ("old", link(LinkState::Down, Some(refused))),
    ]);
    (json!({}), to_value(links).unwrap())
}

fn stats_example() -> (Value, Value) {
    let since = Some(1_762_416_000);
    let stats = MachineStats {
        now: sample(1_762_416_090),
        history: vec![sample(1_762_416_060)],
    };
    (
        to_value(StatsParams { since }).unwrap(),
        to_value(stats).unwrap(),
    )
}

fn sample(at: u64) -> Sample {
    Sample {
        at,
        load_per_cpu: 0.25,
        mem_pressure_pct: 42,
        swap_used_mb: 512,
        disk_free_mb: 120_000,
        cpus: 8,
        uptime_s: 86_400,
        terminals: BTreeMap::from([("exited".into(), 1), ("working".into(), 2)]),
    }
}

fn terminal_params(method: Method) -> Value {
    match method {
        Method::ScreenWatch => to_value(WatchParams {
            terminal: "build".into(),
            since: Some(41),
        }),
        Method::Input => Ok(json!({"terminal": "build", "text": "make lint"})),
        Method::TerminalKeys => {
            Ok(json!({"terminal": "build", "keys": ["esc", "ctrl-c", "up", "enter"]}))
        }
        Method::TerminalResize => to_value(TerminalResizeParams {
            terminal: None,
            cols: 60,
            rows: 30,
        }),
        _ => to_value(TerminalParams {
            terminal: "agent".into(),
        }),
    }
    .unwrap()
}

fn line(v: &impl serde::Serialize) -> String {
    serde_json::to_string(v).unwrap()
}

fn sorted(v: Value) -> Value {
    match v {
        Value::Object(map) => {
            let mut entries: Vec<_> = map.into_iter().map(|(k, v)| (k, sorted(v))).collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Object(entries.into_iter().collect())
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sorted).collect()),
        v => v,
    }
}

fn markdown() -> String {
    let mut md = format!(
        "# shed protocol\n\n\
         <!-- Generated from crates/shed-proto by `SHED_BLESS=1 cargo test -p shed-proto`; do not edit. -->\n\n\
         Version: `{VERSION}`. Client and server must run the same version.\n\n\
         Newline-delimited [JSON-RPC 2.0](https://www.jsonrpc.org/specification) over a Unix socket, mode 0600:\n\
         `$XDG_RUNTIME_DIR/shed.sock` (macOS: `~/Library/Caches/shed/shed.sock`). One request per line, one\n\
         response per line; a request without an `id` is a notification and gets no response.\n\n\
         Every connection starts with `hello`. Any other method first is refused with `NO_HELLO`; another\n\
         product or version is refused with `SHED_VERSION: client shed x, server shed y; restart the server`\n\
         (a peer that sends no `product` shows as `unnamed`).\n\n\
         A terminal, as `run`, `ls`, `get`, `kill` and `rm` return it: `state` is `running` or `exited`; `exit` is\n\
         the exit status (128 + N for signal N), `null` while running or when the program was lost in a server\n\
         restart; `pid` is set while running; `reported` is the state the program last reported (absent until\n\
         then), or `stuck` while the program has left input unread for over a second (`send`, `keys` and\n\
         `acp.model` then fail, their input queued); `idle_after` is set when the `idle` fallback is on;\n\
         `resources` is the process tree's memory (MiB) and CPU (percent of one core) at the last sample, while\n\
         running; an ACP terminal's `session` is its agent session (`id`, for `run`'s `resume`; the current `model`\n\
         and the `models`, `[id, name]`; `run`'s `mcp_servers` with each env and header value as `***`; the agent's\n\
         slash `commands`, `[name, description, hint]`; its last `usage`: `used` and `size` of the context window in\n\
         tokens, and the `cost` when sent).\n\n## Methods\n"
    );
    for method in Method::ALL {
        let (params, result) = example(method);
        let request = Request::new(1, method, sorted(params));
        let response = Response::ok(1.into(), sorted(result));
        write!(
            md,
            "\n### `{}`\n\n{}\n\n```json\n{}\n{}\n```\n",
            method.name(),
            method.summary(),
            line(&request),
            line(&response)
        )
        .unwrap();
    }
    md.push_str(
        "\n## Events\n\nAn event names its terminal, carries its labels and counts up in `seq`; `event` is \
         `started`, `state` (the reported state changed), `exited` (by itself, with its `code`), `killed` (by \
         `kill`) or `resources` (memory or CPU moved by more than 10 % since the last one).\n\n\
         An ACP terminal (`shed run --acp`) adds its agent's events, emitted by its bridge: `turn` (a prompt), \
         `text` (a reply chunk), `tool_call` (started or changed), `plan`, `mode`, `approval_request` (answer \
         with `shed approve`), `approved`, `reply` (the turn's whole reply and its `stop_reason`), `session` (the \
         session is ready, or its model or slash commands changed) and `usage` (the agent's `usage_update`: tokens \
         `used` of the context window's `size`, and its `cost`). A resumed session's history (`run` with `resume`) comes first, \
         its events marked `\"replay\": true` so a client does not count them as new. An idle agent hibernates \
         (state `hibernated`, its processes stopped); the next send wakes it, its history replayed the same way.\n\n```json\n",
    );
    for event in events() {
        writeln!(md, "{}", line(&event)).unwrap();
    }
    md.push_str("```\n");
    md.push_str(
        "\n## Screen updates\n\n`screen.watch` writes one update per line. `seq` is the screen's change count; \
         `rows`, `cols`, `cursor` (`[row, col]`, from 0) and `scrollback` (lines) are always there; `lines` holds \
         the rows that changed, each as runs of cells with one pen: `fg` and `bg` (a palette index 0–255, or \
         `[r, g, b]`; `null` is the default), `bold`, `italic`, `underline` and `inverse`. Trailing blank \
         default cells are dropped, so a blank row has no runs. `full` marks an update that carries every row: the \
         first, and the one after a resize.\n\n```json\n",
    );
    for update in screen_updates() {
        writeln!(md, "{}", line(&update)).unwrap();
    }
    md.push_str("```\n");
    md.push_str("\n## Errors\n\nAn error's `message` starts with its name: `NAME: detail`.\n\n");
    md.push_str("| Code | Name |\n|---|---|\n");
    for code in ErrorCode::ALL {
        writeln!(md, "| {} | `{}` |", code.code(), code.name()).unwrap();
    }
    let refusal = Response::err(1.into(), RpcError::version("0.1.0", VERSION));
    write!(md, "\n```json\n{}\n```\n", line(&refusal)).unwrap();
    md
}

#[test]
fn protocol_md_matches_the_types() {
    let want = markdown();
    let have = bless("protocol.md", &want);
    assert!(
        have == want,
        "docs/protocol.md is stale; run `SHED_BLESS=1 cargo test -p shed-proto`"
    );
}

fn bless(file: &str, want: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs")
        .join(file);
    if std::env::var_os("SHED_BLESS").is_some() {
        std::fs::write(&path, want).unwrap();
    }
    std::fs::read_to_string(&path).unwrap_or_default()
}

#[test]
fn openrpc_json_matches_the_types() {
    let have = bless("shed.openrpc.json", &format!("{:#}\n", openrpc()));
    assert!(
        serde_json::from_str::<Value>(&have).ok() == Some(openrpc()),
        "docs/shed.openrpc.json is stale; run `SHED_BLESS=1 cargo test -p shed-proto`"
    );
}

fn check(doc: &Value, schema: &Value, v: &Value) -> Result<(), String> {
    let Some(s) = schema.as_object() else {
        return if *schema == json!(false) {
            Err(format!("nothing allowed: {v}"))
        } else {
            Ok(())
        };
    };
    let known = [
        "$ref",
        "type",
        "const",
        "enum",
        "minimum",
        "maximum",
        "required",
        "properties",
        "additionalProperties",
        "items",
        "minItems",
        "maxItems",
        "oneOf",
        "anyOf",
        "description",
        "format",
        "default",
        "title",
    ];
    if let Some(k) = s.keys().find(|k| !known.contains(&k.as_str())) {
        return Err(format!("the checker does not know {k}"));
    }
    let fail = |why: String| Err(format!("{why}: {v}"));
    if let Some(r) = s.get("$ref").and_then(Value::as_str) {
        check(doc, doc.pointer(r.trim_start_matches('#')).ok_or(r)?, v)?;
    }
    let types: Vec<&str> = match s.get("type") {
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).collect(),
        Some(t) => vec![t.as_str().unwrap()],
        None => vec!["object", "array", "string", "boolean", "null", "number"],
    };
    let is = |t: &&str| match *t {
        "object" => v.is_object(),
        "array" => v.is_array(),
        "string" => v.is_string(),
        "boolean" => v.is_boolean(),
        "null" => v.is_null(),
        "integer" => v.is_i64() || v.is_u64(),
        "number" => v.is_number(),
        t => panic!("unknown type {t}"),
    };
    if !types.iter().any(is) {
        return fail(format!("not {types:?}"));
    }
    if s.get("const").is_some_and(|c| c != v) {
        return fail(format!("not {}", s["const"]));
    }
    if s.get("enum")
        .and_then(Value::as_array)
        .is_some_and(|e| !e.contains(v))
    {
        return fail("not in the enum".into());
    }
    let (n, min, max) = (v.as_f64(), s.get("minimum"), s.get("maximum"));
    if n.zip(min.and_then(Value::as_f64))
        .is_some_and(|(n, m)| n < m)
        || n.zip(max.and_then(Value::as_f64))
            .is_some_and(|(n, m)| n > m)
    {
        return fail("out of range".into());
    }
    match v {
        Value::Object(obj) => check_object(doc, s, obj)?,
        Value::Array(items) => check_array(doc, s, items)?,
        _ => {}
    }
    let ok = |key: &str| {
        let subs = s.get(key).and_then(Value::as_array);
        subs.map(|subs| subs.iter().filter(|sub| check(doc, sub, v).is_ok()).count())
    };
    match (ok("oneOf"), ok("anyOf")) {
        (Some(n), _) if n != 1 => fail(format!("{n} of oneOf match")),
        (_, Some(0)) => fail("no anyOf matches".into()),
        _ => Ok(()),
    }
}

fn check_object(
    doc: &Value,
    s: &Map<String, Value>,
    obj: &Map<String, Value>,
) -> Result<(), String> {
    let props = s.get("properties").and_then(Value::as_object);
    for key in s
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if !obj.contains_key(key.as_str().unwrap()) {
            return Err(format!("no {key}"));
        }
    }
    for (key, x) in obj {
        let sub = props.and_then(|p| p.get(key));
        let sub = sub
            .or(s.get("additionalProperties"))
            .unwrap_or(&Value::Bool(true));
        check(doc, sub, x).map_err(|e| format!("{key}: {e}"))?;
    }
    Ok(())
}

fn check_array(doc: &Value, s: &Map<String, Value>, items: &[Value]) -> Result<(), String> {
    let len = u64::try_from(items.len()).ok();
    if s.get("minItems").and_then(Value::as_u64) > len
        || s.get("maxItems")
            .and_then(Value::as_u64)
            .is_some_and(|m| Some(m) < len)
    {
        return Err(format!("{} items", items.len()));
    }
    items
        .iter()
        .try_for_each(|x| check(doc, s.get("items").unwrap_or(&Value::Bool(true)), x))
}

fn method_doc<'a>(doc: &'a Value, name: &str) -> &'a Value {
    let methods = doc["methods"].as_array().unwrap();
    methods.iter().find(|m| m["name"] == name).unwrap()
}

fn check_params(doc: &Value, method: &Value, params: &Value) -> Result<(), String> {
    let descriptors = method["params"].as_array().unwrap();
    let schema = json!({
        "type": "object",
        "properties": descriptors.iter().map(|d| (d["name"].as_str().unwrap().to_owned(), d["schema"].clone())).collect::<Map<_, _>>(),
        "required": descriptors.iter().filter(|d| d["required"] == true).map(|d| d["name"].clone()).collect::<Vec<_>>(),
    });
    check(doc, &schema, params)?;
    method
        .get("x-params")
        .map_or(Ok(()), |s| check(doc, s, params))
}

#[test]
fn every_method_is_in_the_openrpc_document() {
    let doc = openrpc();
    let names: Vec<&str> = doc["methods"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, Method::ALL.map(Method::name));
    assert!(names.contains(&"rpc.discover"));
}

#[test]
fn every_golden_message_validates_against_its_schema() {
    let doc = openrpc();
    for method in Method::ALL {
        let (params, result) = example(method);
        let m = method_doc(&doc, method.name());
        check_params(&doc, m, &params).unwrap_or_else(|e| panic!("{} params: {e}", method.name()));
        check(&doc, &m["result"]["schema"], &result)
            .unwrap_or_else(|e| panic!("{} result: {e}", method.name()));
    }
    let stream = &method_doc(&doc, "events")["x-stream"];
    for event in events() {
        check(&doc, stream, &to_value(&event).unwrap()).unwrap_or_else(|e| panic!("{e}"));
    }
    let updates = &method_doc(&doc, "screen.watch")["x-stream"];
    for update in screen_updates() {
        check(&doc, updates, &to_value(&update).unwrap()).unwrap_or_else(|e| panic!("{e}"));
    }
    let refusal = RpcError::version("0.1.0", VERSION);
    let error = &doc["components"]["errors"]["SHED_VERSION"];
    assert_eq!(error["code"], refusal.code);
    assert!(
        refusal
            .message
            .starts_with(error["message"].as_str().unwrap())
    );
    assert_eq!(
        doc["components"]["errors"].as_object().unwrap().len(),
        ErrorCode::ALL.len()
    );
}

#[test]
fn the_schemas_refuse_broken_messages() {
    let doc = openrpc();
    let mut run = run_params();
    run.as_object_mut().unwrap().remove("cmd");
    assert!(check_params(&doc, method_doc(&doc, "run"), &run).is_err());
    let mut emit = emit_params();
    emit["event"] = json!("nope");
    assert!(check_params(&doc, method_doc(&doc, "emit"), &emit).is_err());
    let stream = &method_doc(&doc, "events")["x-stream"];
    let mut event = to_value(&events()[0]).unwrap();
    event["event"] = json!("nope");
    assert!(check(&doc, stream, &event).is_err());
    let updates = &method_doc(&doc, "screen.watch")["x-stream"];
    let mut update = to_value(&screen_updates()[0]).unwrap();
    update["cursor"] = json!([1, 2, 3]);
    assert!(check(&doc, updates, &update).is_err());
    update["cursor"] = json!([1, -2]);
    assert!(check(&doc, updates, &update).is_err());
    let input = json!({"name": "build", "text": "make lint"});
    assert!(check_params(&doc, method_doc(&doc, "terminal.input"), &input).is_err());
}

#[test]
fn method_names_round_trip() {
    for method in Method::ALL {
        assert_eq!(Method::parse(method.name()), Some(method));
    }
    assert_eq!(Method::parse("nope"), None);
}

#[test]
fn version_refusal_reads_as_specified() {
    assert_eq!(
        RpcError::version("0.1.0", "0.2.0").message,
        "SHED_VERSION: client 0.1.0, server 0.2.0; restart the server"
    );
}
