use std::process::Command;

use serde_json::{Value, json};

use crate::common::temp_dir;
use crate::{ok, registry, shed};

#[test]
fn registry_installs_pinned_agents_from_a_fixture() {
    let dir = temp_dir("acp-registry");
    let reg = registry(&dir);
    let reg = reg.to_str().expect("utf-8");
    let installed = ok(&dir, &["agents", "add", "fake", "--registry", reg]);
    assert_eq!(installed, "installed fake 0.1.0 (binary)\n");
    let agent = dir.join("agents/fake/0.1.0/fake-acp");
    assert!(agent.is_file(), "{} unpacked", agent.display());
    ok(&dir, &["agents", "add", "npx-agent", "--registry", reg]);
    ok(&dir, &["agents", "add", "uvx-agent", "--registry", reg]);
    let npx_bin = dir.join("agents/npx-agent/1.2.3/node_modules/npx-agent/bin.js");
    let ran = Command::new("node")
        .arg(&npx_bin)
        .output()
        .expect("node runs");
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "npx-agent ran\n");
    let list: Value = serde_json::from_str(&ok(&dir, &["--json", "agents"])).expect("JSON");
    let cmds: Vec<(&str, &Value)> = list
        .as_array()
        .expect("a list")
        .iter()
        .filter(|a| a["installed"].is_object())
        .map(|a| (a["id"].as_str().expect("id"), &a["installed"]["cmd"]))
        .collect();
    assert_eq!(
        cmds,
        [
            ("fake", &json!([agent])),
            (
                "npx-agent",
                &json!(["env", "NO_UPDATE=1", "node", npx_bin, "--acp"])
            ),
            ("uvx-agent", &json!(["uvx", "uvx-agent==0.4.0"])),
        ]
    );

    let again = shed(&dir, &["agents", "add", "fake", "--registry", reg]);
    assert!(!again.status.success());
    let err = String::from_utf8_lossy(&again.stderr);
    assert!(
        err.contains("fake is installed at 0.1.0; rm it first"),
        "{err}"
    );
    let bad = shed(&dir, &["agents", "add", "bad-sum", "--registry", reg]);
    assert!(String::from_utf8_lossy(&bad.stderr).contains("sha256 mismatch"));
    let missing = shed(&dir, &["agents", "add", "nope", "--registry", reg]);
    assert!(String::from_utf8_lossy(&missing.stderr).contains("no agent \"nope\""));
    for (id, says) in [
        ("no-bin", "no-bin.tgz: no bin to run"),
        ("npx-missing", "missing.tgz\"] failed"),
    ] {
        let failed = shed(&dir, &["agents", "add", id, "--registry", reg]);
        assert!(!failed.status.success());
        let err = String::from_utf8_lossy(&failed.stderr);
        assert!(err.contains("agents: ") && err.contains(says), "{err}");
        assert!(!ok(&dir, &["agents"]).contains(id));
    }

    assert_eq!(
        ok(&dir, &["agents", "rm", "fake"]),
        "removed fake 0.1.0 (binary)\n"
    );
    assert!(!dir.join("agents/fake").exists());
    assert!(ok(&dir, &["agents"]).starts_with("npx-agent 1.2.3 (npx)  env NO_UPDATE=1 node"));
    ok(&dir, &["agents", "rm", "npx-agent"]);
    assert!(!dir.join("agents/npx-agent").exists());
}
