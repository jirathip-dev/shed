use std::path::Path;

use serde_json::{Value, json};

use crate::common::{Server, temp_dir};
use crate::{shed, text};

const P: &str = "antigravity-acp";
const LINK: &str = "https://accounts.google.com/o/oauth2/v2/auth?redirect_uri=http%3A%2F%2F127.0.0.1%3A57865%2Fauth%2Faicode&access_type=offline&scope=https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fcloud-platform%20https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fuserinfo.email%20https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fuserinfo.profile&code_challenge=Xk3vQ9mB2rT7wLpZ8nY4cF6hJ1sD5gA0eU2iO3kP7qW&code_challenge_method=S256&state=9f2c4e7a1b3d5f6e8a0c2b4d6f8e1a3c&response_type=code&client_id=123456789012-a1b2c3d4e5f6g7h8i9j0k1l2m3n4o5p6.apps.googleusercontent.com";

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
        login
            .replace('\n', "")
            .contains(&format!("authenticate the ACP server: {LINK}"))
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
