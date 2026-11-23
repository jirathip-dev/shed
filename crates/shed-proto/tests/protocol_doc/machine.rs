use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::{Value, json, to_value};
use shed_proto::flow::{Class, Differs, FlowName, FlowReport, Mode};
use shed_proto::{
    DrainParams, Draining, ExecParams, ExecResult, FileParams, Handover, HandoverCheckParams,
    HandoverParams, Hello, Link, LinkState, MachineParams, MachineStats, Method, Proc, Report,
    ReportParams, Sample, Span, StatsParams,
};

fn links_example() -> (Value, Value) {
    let link = |state, error: Option<&str>| Link {
        state,
        since: 1_762_416_000,
        error: error.map(str::to_owned),
        down_since: error.map(|_| 1_762_415_000),
        hello: None,
        os: None,
        arch: None,
        home: None,
        proxy: None,
    };
    let up = Link {
        hello: Some(Hello {
            build: "4254d78".into(),
            ..Hello::ours()
        }),
        os: Some("linux".into()),
        arch: Some("x86_64".into()),
        home: Some(PathBuf::from("/home/user")),
        proxy: Some(41_234),
        ..link(LinkState::Up, None)
    };
    let refused = "SHED_VERSION: shed 0.1.0 here, 0.2.0 on old; install the same version on both";
    let links = BTreeMap::from([("box", up), ("old", link(LinkState::Down, Some(refused)))]);
    (json!({}), to_value(links).unwrap())
}

fn stats_example() -> (Value, Value) {
    let since = Some(1_762_416_000);
    let stats = MachineStats {
        now: sample(1_762_416_090),
        history: vec![sample(1_762_416_060)],
        top_cpu: vec![proc(4242, Some("box/lane-1"))],
        top_mem: vec![proc(4242, Some("box/lane-1"))],
        draining: Some(Draining {
            since: 1_762_416_000,
            why: Some("idle 20 min".into()),
        }),
    };
    let params = StatsParams {
        since,
        top: Some(1),
        machine: None,
    };
    (to_value(params).unwrap(), to_value(stats).unwrap())
}

fn proc(pid: i32, terminal: Option<&str>) -> Proc {
    Proc {
        pid,
        ppid: 4200,
        depth: 1,
        cpu_pct: 85,
        rss_mb: 1_200,
        elapsed_s: 600,
        command: "cargo test --workspace".into(),
        terminal: terminal.map(str::to_owned),
        own: false,
    }
}

fn sample(at: u64) -> Sample {
    Sample {
        at,
        load_per_cpu: 0.25,
        mem_pressure_pct: 42,
        mem_free_mb: 9_500,
        mem_total_mb: 16_384,
        swap_used_mb: 512,
        swap_out_pages: 12_000,
        disk_free_mb: 120_000,
        disk_total_mb: 460_000,
        cpus: 8,
        uptime_s: 86_400,
        terminals: BTreeMap::from([("exited".into(), 1), ("working".into(), 2)]),
    }
}

fn report_example() -> (Value, Value) {
    let terminal = FlowReport {
        flow: FlowName::Terminal,
        mode: Mode::Shadow,
        subjects: 4,
        fired: BTreeMap::from([("T1".into(), 4), ("T4".into(), 4), ("T9".into(), 1)]),
        never_seen: vec!["replace".into(), "expired".into()],
        unexpected: BTreeMap::new(),
        stuck: BTreeMap::from([("tree-left".into(), 1)]),
        disagreements: vec![Class {
            kind: Differs::Action,
            row: "T9".into(),
            table: "end-tree".into(),
            code: "exited".into(),
            count: 1,
            first: 1_762_416_000,
            last: 1_762_416_000,
            sample: "build".into(),
        }],
    };
    let params = ReportParams {
        since: Some(1_762_000_000),
        machine: None,
    };
    let report = Report {
        build: "4254d78".into(),
        since: 1_762_000_000,
        flows: vec![terminal],
    };
    (to_value(params).unwrap(), to_value(report).unwrap())
}

pub(crate) fn example(method: Method) -> Option<(Value, Value)> {
    Some(match method {
        Method::FlowReport => report_example(),
        Method::MachineStats => stats_example(),
        Method::MachineDrain => (
            to_value(DrainParams {
                on: true,
                why: Some("idle 20 min".into()),
                machine: Some("dev-16".into()),
            })
            .unwrap(),
            json!({}),
        ),
        Method::MachineLinks => links_example(),
        Method::Exec => (
            to_value(ExecParams {
                cmd: vec!["git".into(), "status".into(), "--short".into()],
                cwd: Some(PathBuf::from("/home/user/project")),
                timeout: Some(30.0),
                stdin: Vec::new(),
                env: BTreeMap::from([("GIT_PAGER".into(), "cat".into())]),
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
                path: PathBuf::from("/home/user/project/CHANGELOG.md"),
                data: None,
                lines: Some(Span { start: 3, end: 3 }),
                mode: None,
                machine: None,
            })
            .unwrap(),
            json!(b"## 1.2.0\n"),
        ),
        Method::FileWrite => (
            to_value(FileParams {
                path: PathBuf::from("/home/user/project/VERSION"),
                data: Some(b"1.3.0\n".to_vec()),
                lines: None,
                mode: Some(0o600),
                machine: None,
            })
            .unwrap(),
            json!({}),
        ),
        Method::MachineAdd => (
            to_value(MachineParams {
                name: "box".into(),
                target: "user@box.example".into(),
            })
            .unwrap(),
            json!({}),
        ),
        Method::MachineRm => (json!({"name": "box"}), json!({})),
        Method::TerminalProcs => {
            let agent = Proc {
                depth: 0,
                ppid: 1,
                pid: 4200,
                cpu_pct: 3,
                rss_mb: 180,
                command: "node agent.js".into(),
                own: true,
                ..proc(4200, None)
            };
            let procs = vec![agent, proc(4242, None)];
            (json!({"name": "lane-1"}), to_value(procs).unwrap())
        }
        Method::HandoverCheck => (
            to_value(HandoverCheckParams {
                build: "1a2b3c4".into(),
                candidate: PathBuf::from("/home/user/.cargo/bin/shed.new"),
            })
            .unwrap(),
            handover(),
        ),
        Method::Handover => (
            to_value(HandoverParams {
                build: "1a2b3c4".into(),
            })
            .unwrap(),
            handover(),
        ),
        _ => return None,
    })
}

fn handover() -> Value {
    to_value(Handover {
        from: "4254d78".into(),
        to: "1a2b3c4".into(),
        terminals: 12,
    })
    .unwrap()
}
