//! `account login --token`: shed runs the record's token command on a redacted screen, so its readers see
//! `[redacted]`, and saves the token it prints, read off the screen itself, to the account's 0600 file.
#![cfg(test)]

mod common;

use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::process::Command;

use common::{Server, temp_dir};
use shed_server::Paths;

const SHED: &str = env!("CARGO_BIN_EXE_shed");
const TOKEN: &str = "sk-fake-oat01-Zq7xK2mP9vL4nR8tW3yB6cD1fG5hJ0kA";

fn ok(dir: &Path, args: &[&str]) -> String {
    let out = Command::new(SHED).arg("--dir").arg(dir).args(args).output();
    let out = out.expect("shed runs");
    assert!(out.status.success(), "shed {args:?} failed: {out:?}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn the_setup_token_is_saved_from_the_screen_and_its_readers_see_it_redacted() {
    let dir = temp_dir("account-token");
    let providers = Paths::in_dir(&dir).providers();
    std::fs::create_dir_all(&providers).expect("providers dir");
    let record = format!(
        "id = \"fake\"\nagent = \"fake\"\nconfig_dir_var = \"FAKE_HOME\"\nstatus = \"echo in\"\n\
         [token]\ncommand = \"printf 'Your token:\\\\n\\\\n  {TOKEN}\\\\n\\\\nStore it safely.\\\\n'\"\n\
         prefix = \"sk-fake-oat\"\ndays = 365\n"
    );
    std::fs::write(providers.join("fake.toml"), record).expect("write the record");
    let _server = Server::start(&dir);
    ok(&dir, &["account", "add", "fake", "work"]);
    let shown = ok(&dir, &["account", "login", "fake", "work", "--token"]);
    assert!(!shown.contains(TOKEN), "{shown}");
    common::eventually("the saved token", || {
        dir.join("accounts/fake/work.token").exists()
    });
    let file = dir.join("accounts/fake/work.token");
    assert_eq!(
        std::fs::read_to_string(&file).expect("the token file"),
        TOKEN
    );
    let mode = std::fs::metadata(&file).expect("meta").permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    let read = ok(&dir, &["read", "login-fake-work"]);
    assert!(
        read.contains("[redacted]") && !read.contains(TOKEN),
        "{read}"
    );
    let listed = ok(&dir, &["--json", "account", "ls", "fake"]);
    assert!(
        listed.contains("saved_at") && !listed.contains(TOKEN),
        "{listed}"
    );
}
