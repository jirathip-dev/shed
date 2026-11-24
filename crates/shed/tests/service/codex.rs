use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

use serde_json::{Value, json};

use crate::common::{Server, temp_dir};
use crate::{shed, text};

const FAKE: &str = "#!/bin/sh\n\
case \"$*\" in 'login status') echo 'Logged in using ChatGPT'; exit 0 ;; app-server) ;; *) exit 2 ;; esac\n\
while read -r line; do case $line in *'\"id\":2,'*) cat \"$CODEX_HOME/rate-limits.json\" ;; esac; done\n";

fn run(home: &Path, args: &[&str]) -> String {
    let d = home.to_str().expect("utf-8");
    let out = shed(home, &[&["--dir", d], args].concat());
    assert!(out.status.success(), "{args:?}: {out:?}");
    text(&out)
}

fn windows(home: &Path) -> Value {
    let out = run(home, &["--json", "account", "status", "codex", "work"]);
    let status: Value = serde_json::from_str(&out).expect("JSON");
    status["usage"]["windows"].clone()
}

#[test]
fn codex_reads_its_windows_from_the_app_server_in_the_account_s_folder() {
    let home = temp_dir("codex-usage");
    let bin = home.join("codex-bin");
    std::fs::create_dir_all(&bin).expect("bin");
    let codex = bin.join("codex");
    std::fs::write(&codex, FAKE).expect("fake codex");
    std::fs::set_permissions(&codex, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let h = home.to_str().expect("utf-8");
    let _server = Server::with_env(&home, "3600", &[("HOME", h), ("PATH", &path)]);
    run(&home, &["account", "add", "codex", "work"]);

    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/codex-rate-limits.json");
    let recorded = std::fs::read_to_string(fixture).expect("fixture");
    let answer = home.join("accounts/codex/work/rate-limits.json");
    std::fs::write(&answer, &recorded).expect("answer");
    assert_eq!(
        windows(&home),
        json!([{"id": "30d", "used_pct": 1, "resets_at": 1_798_015_615_u64}])
    );

    let plus = recorded
        .replacen(
            r#""usedPercent":1,"windowDurationMins":43200"#,
            r#""usedPercent":83,"windowDurationMins":300"#,
            1,
        )
        .replacen(
            r#""secondary":null"#,
            r#""secondary":{"usedPercent":4,"windowDurationMins":10080,"resetsAt":1798100000}"#,
            1,
        );
    std::fs::write(&answer, plus).expect("answer");
    assert_eq!(
        windows(&home),
        json!([
            {"id": "5h", "used_pct": 83, "resets_at": 1_798_015_615_u64},
            {"id": "7d", "used_pct": 4, "resets_at": 1_798_100_000_u64}
        ])
    );
    let ls = run(&home, &["account", "ls"]);
    assert!(ls.contains("codex/work  ready  -  5h 83%,7d 4%"), "{ls}");
}
