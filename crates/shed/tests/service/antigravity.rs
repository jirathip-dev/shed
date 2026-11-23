use std::path::Path;

use serde_json::{Value, json};

use crate::common::{Server, temp_dir};
use crate::{shed, text};

const P: &str = "antigravity-acp";

fn run(home: &Path, args: &[&str]) -> String {
    let d = home.to_str().expect("utf-8");
    let out = shed(home, &[&["--dir", d], args].concat());
    assert!(out.status.success(), "{args:?}: {out:?}");
    text(&out)
}

fn status(home: &Path) -> Value {
    let out = run(home, &["--json", "account", "status", P, "work"]);
    serde_json::from_str(&out).expect("JSON")
}

#[test]
fn antigravity_signs_in_over_acp_into_the_account_s_folder_and_doctor_names_the_login() {
    let home = temp_dir("antigravity");
    let h = home.to_str().expect("utf-8");
    let _server = Server::with_env(&home, "3600", &[("HOME", h)]);
    run(&home, &["account", "add", P, "work"]);
    let token = home.join("accounts/antigravity-acp/work/antigravity-acp/acp_token.json");

    assert_eq!(status(&home)["state"], "signed-out");
    let doctor = text(&shed(&home, &["--dir", h, "doctor"]));
    let row = format!("FAIL  account  {P}/work signed out; fix: shed account login {P} work\n");
    assert!(doctor.contains(&row), "{doctor}");

    let missing = run(&home, &["account", "login", P, "work"]);
    assert!(
        missing.contains("fix: shed agents add antigravity-acp"),
        "{missing}"
    );

    let fake = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_agy.py");
    let agents = json!({"agents": [{"id": P, "version": "1.5.2", "distribution": "binary",
        "cmd": ["python3", fake.to_str().expect("utf-8")]}]});
    std::fs::write(home.join("agents.json"), agents.to_string()).expect("agents.json");
    let login = run(&home, &["account", "login", P, "work"]);
    assert!(
        login.contains("authenticate the ACP server: https://example.test/auth")
            && login.contains("signed in with Google"),
        "{login}"
    );
    assert!(token.is_file(), "{login}");
    let ready = status(&home);
    assert_eq!(
        (&ready["state"], &ready["line"]),
        (&json!("ready"), &json!("signed in with Google")),
        "{ready}"
    );
    let ls = run(&home, &["account", "ls"]);
    assert!(ls.contains("antigravity-acp/work  ready  "), "{ls}");

    std::fs::remove_file(&token).expect("rm token");
    assert_eq!(status(&home)["state"], "signed-out");
}
