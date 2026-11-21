use std::path::Path;

use serde_json::Value;
use shed_core::proctree::{self, ProcessTable as _};

use crate::common::{self, Server, temp_dir};
use crate::{events_until, fake_agent, ok, row, start_fake};

#[test]
fn top_splits_the_memory_of_a_child_the_agent_started_into_work_and_lists_it_by_name() {
    let dir = temp_dir("acp-top-split");
    let _server = Server::sampling(&dir, "0.2");
    start_fake(&dir, "g", "fake");
    ok(&dir, &["send", "g", "hi"]);
    ok(&dir, &["wait", "g", "--until", "done", "--timeout", "10"]);
    for args in [&["top"][..], &["ls"]] {
        assert!(
            row(&dir, args, "g").contains(&"STATE=done".to_owned()),
            "{args:?}"
        );
    }
    ok(&dir, &["send", "g", "hog"]);
    let mb = |info: &Value, k: &str| info["resources"][k].as_u64().unwrap_or(0);
    common::eventually("the child's memory under work", || {
        let info: Value = serde_json::from_str(&ok(&dir, &["get", "g"])).expect("JSON");
        mb(&info, "work_mb") >= 200
    });
    let info: Value = serde_json::from_str(&ok(&dir, &["get", "g"])).expect("JSON");
    assert!(mb(&info, "agent_mb") < 100, "{info}");
    let top = row(&dir, &["top"], "g");
    let cell = |k: &str| {
        top.iter()
            .find_map(|c| c.strip_prefix(k))
            .unwrap_or_default()
    };
    let agent: u64 = cell("AGENT=").trim_end_matches('M').parse().expect("AGENT");
    let work: u64 = cell("WORK=").trim_end_matches('M').parse().expect("WORK");
    assert!(agent < 100 && work >= 200, "{top:?}");
    let totals = ok(&dir, &["top"]);
    let totals = totals.lines().last().unwrap_or_default();
    assert!(
        totals.starts_with("1 awake, 0 hibernated; agent "),
        "{totals}"
    );

    let short = ok(&dir, &["top", "g"]);
    assert!(short.starts_with("PID"), "{short}");
    let name = short
        .lines()
        .nth(1)
        .and_then(|l| l.split_whitespace().last());
    assert!(
        name.is_some_and(|n| n.to_lowercase().starts_with("python")),
        "{short}"
    );
    assert!(
        !short.contains("example.invalid") && !short.contains("-c"),
        "{short}"
    );
    assert!(ok(&dir, &["top", "g", "--full"]).contains("https://user:secret@example.invalid/"));
    ok(&dir, &["kill", "g"]);
}

#[test]
fn a_child_over_the_memory_limit_is_ended_and_the_agent_carries_on() {
    let dir = temp_dir("acp-mem-max");
    let scoped = shed_server::limit::scoped();
    let _server = Server::sampling(&dir, if scoped { "4" } else { "0.2" });
    let agent = fake_agent();
    let agent = agent.to_str().expect("utf-8");
    let run = [
        "run",
        "--name",
        "g",
        "--label",
        "run=g",
        "--mem-max",
        "200M",
    ];
    ok(
        &dir,
        &[&run[..], &["--acp", "fake", "--", "python3", agent]].concat(),
    );
    ok(&dir, &["wait", "g", "--output", "ACP session ready"]);
    let info: Value = serde_json::from_str(&ok(&dir, &["get", "g"])).expect("JSON");
    assert_eq!(info["mem_max_mb"], 200, "{info}");
    let listed: Value = serde_json::from_str(&ok(&dir, &["ls", "--json"])).expect("JSON");
    assert_eq!(listed[0]["mem_max_mb"], 200, "{listed}");
    assert!(row(&dir, &["top"], "g").contains(&"LIMIT=200M".to_owned()));
    ok(&dir, &["send", "g", "hog 400"]);
    let limited = events_until(&dir, "g", |e| e["event"] == "mem-limit");
    let event: Value = serde_json::from_str(limited.last().expect("an event")).expect("JSON");
    assert_eq!(event["limit_mb"], 200, "{event}");
    let by = if scoped { "kernel" } else { "shed" };
    assert_eq!(event["ended_by"], by, "{event}");
    let process = event["process"].as_str().unwrap_or_default();
    assert!(process.to_lowercase().contains("python"), "{event}");
    let pid = info["pid"].as_i64().and_then(|p| i32::try_from(p).ok());
    let pid = pid.expect("a pid");
    common::eventually("the child gone", || {
        let procs = proctree::system().list().unwrap_or_default();
        let tree = proctree::tree(&procs, pid, &std::collections::BTreeSet::new());
        tree.iter().all(|p| p.rss_bytes < 300 << 20)
    });
    ok(&dir, &["wait", "g", "--until", "done", "--timeout", "10"]);
    ok(&dir, &["send", "g", "hi"]);
    ok(&dir, &["wait", "g", "--until", "done", "--timeout", "10"]);
    assert_eq!(ok(&dir, &["read", "g", "--lines", "1"]), "you said: hi\n");
    let all = events_until(&dir, "g", |e| e["text"] == "you said: hi");
    let limits = all.iter().filter(|l| l.contains(r#""event":"mem-limit""#));
    assert_eq!(limits.count(), 1, "{all:?}");
    ok(&dir, &["kill", "g"]);
}

#[cfg(target_os = "linux")]
#[test]
fn a_limited_terminal_runs_in_its_own_scope_its_children_ranked_first_and_none_outlives_it() {
    if !shed_server::limit::scoped() {
        eprintln!("skipped: no user systemd with the memory controller here");
        return;
    }
    let dir = temp_dir("acp-mem-scope");
    let mut server = Server::sampling(&dir, "0.2");
    let read = |path: &str| std::fs::read_to_string(path).unwrap_or_default();
    let scope = |pid: &Value| {
        let cgroup = read(&format!("/proc/{pid}/cgroup"));
        let path = cgroup.lines().find_map(|l| l.strip_prefix("0::"));
        format!("/sys/fs/cgroup{}", path.unwrap_or_default())
    };
    ok(
        &dir,
        &[
            "run",
            "--name",
            "s",
            "--mem-max",
            "200M",
            "--",
            "sleep",
            "100",
        ],
    );
    let info: Value = serde_json::from_str(&ok(&dir, &["get", "s"])).expect("JSON");
    common::eventually("the scope", || {
        std::path::Path::new(&scope(&info["pid"]))
            .extension()
            .is_some_and(|e| e == "scope")
    });
    let sleeper = scope(&info["pid"]);
    assert_eq!(read(&format!("{sleeper}/memory.max")).trim(), "209715200");

    let agent = fake_agent();
    let agent = agent.to_str().expect("utf-8");
    let run = [
        "run",
        "--name",
        "g",
        "--label",
        "run=g",
        "--mem-max",
        "200M",
    ];
    ok(
        &dir,
        &[&run[..], &["--acp", "fake", "--", "python3", agent]].concat(),
    );
    ok(&dir, &["wait", "g", "--output", "ACP session ready"]);
    ok(&dir, &["send", "g", "hog 50"]);
    let g: Value = serde_json::from_str(&ok(&dir, &["get", "g"])).expect("JSON");
    let bridge = g["pid"]
        .as_i64()
        .and_then(|p| i32::try_from(p).ok())
        .expect("a pid");
    let adj = |pid: i32| {
        read(&format!("/proc/{pid}/oom_score_adj"))
            .trim()
            .to_owned()
    };
    common::eventually("the child ranked first", || {
        let procs = proctree::system().list().unwrap_or_default();
        let tree = proctree::tree(&procs, bridge, &std::collections::BTreeSet::new());
        let (hog, rest): (Vec<_>, Vec<_>) = tree.iter().partition(|p| p.rss_bytes > 40 << 20);
        hog.iter().all(|p| adj(p.pid) == "1000")
            && !hog.is_empty()
            && rest
                .iter()
                .all(|p| adj(p.pid) == adj(std::process::id().cast_signed()))
    });

    server.0.kill().expect("kill shed serve");
    server.0.wait().expect("reap shed serve");
    let _again = Server::sampling(&dir, "0.2");
    common::eventually("the sleeper's scope gone", || !Path::new(&sleeper).exists());
    ok(&dir, &["kill", "g"]);
}
