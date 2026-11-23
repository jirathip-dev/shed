use std::process::Stdio;

use serde_json::{Value, json};

use crate::common::{self, Server, temp_dir};
use crate::{fake_hosts, script, shed_on};

#[test]
fn machine_stats_here_and_on_a_machine_and_an_offline_one_answers_without_waiting_for_ssh() {
    let root = temp_dir("machine-stats");
    let bin = fake_hosts(&root);
    let ssh = std::fs::read_to_string(bin.join("ssh")).expect("ssh");
    let hang = "[ -e \"$(dirname \"$0\")/down\" ] && exec sleep 600\n";
    script(&bin.join("ssh"), &ssh.replacen("#!/bin/sh\n", hang, 1));
    let _local = Server::start(&root.join("local"));
    let _remote = Server::start(&root.join("remote"));
    let shed = |args: &[&str]| {
        let out = shed_on(&root, args)
            .stderr(Stdio::null())
            .output()
            .expect("shed runs");
        let json = serde_json::from_slice(&out.stdout).unwrap_or(Value::Null);
        (out.status.code(), json)
    };
    assert_eq!(
        shed(&["run", "--name", "a", "--", "sleep", "30"]).0,
        Some(0)
    );
    assert_eq!(shed(&["state", "a", "working"]).0, Some(0));
    assert_eq!(shed(&["run", "--name", "b", "--", "true"]).0, Some(0));
    common::eventually("b exits", || {
        shed(&["--json", "get", "b"]).1["state"] == "exited"
    });
    let check = |s: &Value| {
        assert!(s["at"].as_u64().expect("at") > 1_700_000_000, "{s}");
        assert!(s["load_per_cpu"].as_f64().expect("load") >= 0.0, "{s}");
        assert!(
            s["mem_pressure_pct"].as_u64().expect("memory") <= 100,
            "{s}"
        );
        assert!(s["swap_used_mb"].is_u64(), "{s}");
        assert!(s["disk_free_mb"].as_u64().expect("disk") > 0, "{s}");
        assert!(s["cpus"].as_u64().expect("cpus") >= 1, "{s}");
        assert!(s["uptime_s"].as_u64().expect("uptime") > 0, "{s}");
    };
    let (code, here) = shed(&["--json", "machine", "stats", "--since", "0"]);
    assert_eq!(code, Some(0));
    check(&here);
    assert_eq!(here["terminals"], json!({"working": 1, "exited": 1}));
    let history = here["history"].as_array().expect("history");
    assert_eq!(history.len(), 1, "{here}");
    check(&history[0]);
    assert_eq!(history[0]["terminals"], json!({}));
    let (code, without) = shed(&["--json", "machine", "stats"]);
    assert_eq!(code, Some(0));
    assert!(without.get("history").is_none(), "{without}");

    assert_eq!(
        shed(&["machine", "add", "box", "user@box.example"]).0,
        Some(0)
    );
    let (code, there) = shed(&["--json", "--on", "box", "machine", "stats"]);
    assert_eq!(code, Some(0));
    check(&there);
    assert_eq!(there["terminals"], json!({}));
    let seen = std::fs::read_to_string(root.join("local/seen/box")).expect("seen");

    std::fs::write(bin.join("down"), "").expect("down");
    let started = shed_server::clock::now();
    let (code, offline) = shed(&["--json", "--on", "box", "machine", "stats"]);
    let took = started.elapsed();
    assert_eq!(code, Some(1));
    let last_seen: u64 = seen.parse().expect("unix seconds");
    assert_eq!(
        offline,
        json!({"machine": "box", "online": false, "last_seen": last_seen})
    );
    assert!(
        took < shed_server::clock::PATIENCE,
        "answered after {took:?}"
    );
    let text = shed_on(&root, &["--on", "box", "machine", "stats"])
        .stderr(Stdio::null())
        .output()
        .expect("shed runs");
    assert_eq!(
        String::from_utf8_lossy(&text.stdout),
        format!("box offline, last seen {last_seen}\n")
    );
}
