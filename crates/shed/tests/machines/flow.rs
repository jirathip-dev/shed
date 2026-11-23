use std::path::Path;
use std::process::Command;

use serde_json::{Value, json};
use shed_server::Paths;

use crate::common::{self, Server, temp_dir};
use crate::{SHED, fake_hosts, ok};

fn key(id: &str, terminal: &str) -> Value {
    json!({"sha": id, "id": id, "terminal": terminal, "provider": "fake", "account": "work",
        "created": 0, "state": "live"})
}

fn flow(root: &Path, name: &str) -> Value {
    let report: Value =
        serde_json::from_str(&ok(root, &["--json", "flow", "report"])).expect("JSON");
    let flows = report["flows"].as_array().expect("flows").iter();
    flows
        .into_iter()
        .find(|f| f["flow"] == name)
        .cloned()
        .unwrap_or(Value::Null)
}

fn saw(flow: &Value, fact: &str) -> bool {
    flow["never_seen"]
        .as_array()
        .is_some_and(|n| !n.contains(&json!(fact)))
}

#[test]
fn keys_and_links_read_over_the_fake_link_agree_with_today_s_code_in_the_report() {
    let root = temp_dir("flow-shadow");
    let (local, remote) = (root.join("local"), root.join("remote"));
    fake_hosts(&root);
    let _remote = Server::start(&remote);
    let lane = Command::new(SHED)
        .arg("--dir")
        .arg(&remote)
        .args(["run", "--name", "lane", "--", "sleep", "100"])
        .output()
        .expect("run on box");
    assert!(lane.status.success(), "{lane:?}");
    for (name, target) in [("box", "user@box.example"), ("old", "old.example")] {
        ok(&root, &["machine", "add", name, target]);
    }
    let keys = json!([
        key("aaaa", "box/lane"),
        key("bbbb", "box/none"),
        key("cccc", "old/x")
    ]);
    let file = Paths::in_dir(&local)
        .state
        .with_file_name("proxy-keys.json");
    std::fs::write(file, keys.to_string()).expect("the keys");
    let path = format!(
        "{}:{}",
        root.join("bin").display(),
        std::env::var("PATH").unwrap()
    );
    let _local = Server::with_env(&local, "3600", &[("PATH", &path)]);
    common::eventually("the key facts over the link", || {
        let keys = flow(&root, "key");
        ["alive", "gone", "unknown"].iter().all(|f| saw(&keys, f))
    });
    let keys = flow(&root, "key");
    assert_eq!(keys["subjects"], 3, "{keys}");
    for row in ["K1", "K2", "K3"] {
        assert!(keys["fired"][row].is_u64(), "{row} in {keys}");
    }
    assert_eq!(keys["disagreements"], json!([]), "{keys}");
    let links = flow(&root, "link");
    assert_eq!(links["subjects"], 2, "{links}");
    assert!(
        saw(&links, "dialed") && saw(&links, "dial_failed"),
        "{links}"
    );
    assert!(links["fired"]["L1"].is_u64(), "{links}");
    let local_keys: Value = serde_json::from_str(&ok(&root, &["--json", "proxy", "keys"])).unwrap();
    let ended = |id: &str| {
        let all = local_keys.as_array().expect("keys").iter();
        all.into_iter()
            .any(|k| k["id"] == id && k["state"] == "ended")
    };
    assert!(
        ended("bbbb") && !ended("aaaa") && !ended("cccc"),
        "{local_keys}"
    );
}
