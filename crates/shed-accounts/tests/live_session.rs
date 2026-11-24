//! Live: a Claude Code session made in one config folder loads in another once its `session_files` are copied
//! there, which is what an account switch on a limit relies on (docs/architecture/accounts/adr.md). It needs
//! `claude` on `PATH` and two signed-in config folders, e.g. two accounts' folders from `shed account ls`:
//!
//! ```sh
//! SHED_LIVE_FROM=<folder A> SHED_LIVE_TO=<folder B> \
//!   cargo test -p shed-accounts --test live_session -- --ignored --nocapture
//! ```
#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use shed_accounts::session;
use shed_proto::Provider;

struct Gone(PathBuf);

impl Drop for Gone {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).ok();
    }
}

fn claude(folder: &Path, cwd: &Path, args: &[&str]) -> String {
    let out = Command::new("claude")
        .env("CLAUDE_CONFIG_DIR", folder)
        .env_remove("CLAUDE_CODE_OAUTH_TOKEN")
        .env_remove("ANTHROPIC_API_KEY")
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("claude runs");
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(out.status.success(), "claude {args:?}: {out:?}");
    text
}

#[test]
#[ignore = "live: needs claude and two signed-in config folders in SHED_LIVE_FROM and SHED_LIVE_TO"]
fn a_session_copied_between_two_account_folders_loads() {
    let folder = |var| PathBuf::from(std::env::var(var).expect("SHED_LIVE_FROM and SHED_LIVE_TO"));
    let (from, to) = (folder("SHED_LIVE_FROM"), folder("SHED_LIVE_TO"));
    let secs = shed_core::clock::unix_now();
    let session = format!(
        "{:08x}-0000-4000-8000-{:012x}",
        std::process::id(),
        secs % (1 << 48)
    );
    let cwd = std::env::temp_dir().join(format!("shed-live-session-{}", std::process::id()));
    fs::create_dir_all(&cwd).expect("cwd");
    let cwd = Gone(cwd);
    let cwd = cwd.0.as_path();
    let first = "The code word is kumquat. Reply with just: ok";
    claude(&from, cwd, &["-p", "--session-id", &session, first]);
    let record = Provider {
        session_files: vec!["projects/*/{session}.jsonl".into()],
        ..Provider::default()
    };
    assert_eq!(
        session::copy(&record, &session, &from, &to).expect("copied"),
        1,
        "one session file in {}",
        from.display()
    );
    let asked = "What is the code word? Reply with just the word.";
    let reply = claude(&to, cwd, &["-p", "--resume", &session, asked]);
    assert!(reply.to_lowercase().contains("kumquat"), "{reply}");
}
