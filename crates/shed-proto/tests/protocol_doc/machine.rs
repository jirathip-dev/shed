use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::{Value, json, to_value};
use shed_proto::{
    ExecParams, ExecResult, FileParams, Link, LinkState, MachineStats, Method, Sample, StatsParams,
};

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

pub(crate) fn example(method: Method) -> Option<(Value, Value)> {
    Some(match method {
        Method::MachineStats => stats_example(),
        Method::MachineLinks => links_example(),
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
        _ => return None,
    })
}
