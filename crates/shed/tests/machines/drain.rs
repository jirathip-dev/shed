use std::process::Command;

use serde_json::Value;

use crate::common::{self, Server, temp_dir};
use crate::{SHED, fake_hosts, ok, shed_on};

fn fired(root: &std::path::Path, row: &str) -> bool {
    let report: Value =
        serde_json::from_str(&ok(root, &["--json", "flow", "report"])).expect("JSON");
    let flows = report["flows"].as_array().expect("flows").iter();
    let machine = flows.into_iter().find(|f| f["flow"] == "machine");
    machine.is_some_and(|m| m["fired"][row].is_u64())
}

#[test]
fn a_machine_drained_over_the_fake_link_refuses_run_shows_it_and_the_machine_flow_reads_it() {
    let root = temp_dir("machine-drain");
    let remote = root.join("remote");
    fake_hosts(&root);
    let _remote = Server::start(&remote);
    let drained = Command::new(SHED)
        .arg("--dir")
        .arg(&remote)
        .args(["machine", "drain", "--why", "reboot"])
        .output()
        .expect("drain box itself");
    assert!(drained.status.success(), "{drained:?}");
    ok(&root, &["machine", "add", "box", "user@box.example"]);
    let path = format!(
        "{}:{}",
        root.join("bin").display(),
        std::env::var("PATH").unwrap()
    );
    let _local = Server::with_env(&root.join("local"), "3600", &[("PATH", &path)]);
    common::eventually("M13: up to draining, read from box's stats", || {
        fired(&root, "M13")
    });
    let stats: Value =
        serde_json::from_str(&ok(&root, &["--json", "--on", "box", "machine", "stats"])).unwrap();
    assert_eq!(stats["draining"]["why"], "reboot", "{stats}");
    let run = [
        "--on", "box", "run", "--name", "x", "--cwd", "/", "--", "true",
    ];
    let refused = shed_on(&root, &run).output().expect("shed runs");
    let said = String::from_utf8_lossy(&refused.stderr);
    assert!(
        !refused.status.success() && said.contains("DRAINING: "),
        "{said}"
    );
    let lifted = ok(&root, &["machine", "drain", "--on", "box", "--off"]);
    assert_eq!(lifted, "box no longer draining\n");
    ok(&root, &run);
    assert_eq!(
        ok(&root, &["machine", "drain", "--on", "box", "--why", "idle"]),
        "box draining\n"
    );
    let stats: Value =
        serde_json::from_str(&ok(&root, &["--json", "--on", "box", "machine", "stats"])).unwrap();
    assert_eq!(stats["draining"]["why"], "idle", "{stats}");
}
