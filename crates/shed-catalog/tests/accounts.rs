//! The records' account keys through the status and usage readers.

use shed_catalog::Catalog;
use shed_proto::{Provider, Status, Usage, Window, when};

fn record(agent: &str) -> Provider {
    Catalog::built_in()
        .by_agent(agent)
        .cloned()
        .unwrap_or_default()
}

fn status(signed_in: bool, line: &str, who: Option<&str>) -> Status {
    Status {
        signed_in,
        line: line.to_owned(),
        who: who.map(str::to_owned),
    }
}

#[test]
fn claude_status_from_its_json() {
    let c = record("claude");
    let read = |ok, stdout: &str| c.read_status(ok, stdout, "");
    assert_eq!(
        read(
            true,
            r#"{"loggedIn": true, "authMethod": "claude.ai", "email": "o@example.com"}"#
        ),
        status(
            true,
            "signed in · claude.ai (o@example.com)",
            Some("o@example.com")
        )
    );
    assert_eq!(
        read(true, r#"{"loggedIn": true, "orgName": "Acme"}"#),
        status(true, "signed in", Some("Acme"))
    );
    assert_eq!(
        read(false, r#"{"loggedIn": false, "authMethod": "none"}"#),
        status(false, "signed out", None)
    );
    assert_eq!(
        read(true, r#"{"a": 1}"#),
        status(true, "status unknown", None)
    );
    assert_eq!(
        read(true, "Logged in\n as owner"),
        status(true, "Logged in as owner", None)
    );
}

#[test]
fn a_record_without_status_read_is_its_exit_and_output() {
    let c = record("codex");
    assert_eq!(
        c.read_status(true, "  Logged in\n  as owner", ""),
        status(true, "Logged in as owner", None)
    );
    assert_eq!(
        c.read_status(false, "", "not signed in\n"),
        status(false, "not signed in", None)
    );
    assert_eq!(c.read_usage("Current session: 4% used", 0), None);
}

#[test]
fn claude_usage_from_its_usage_lines() {
    let c = record("claude");
    let now = when("Nov 10 at 9am", 1_762_000_000).unwrap_or_default();
    let out = "Settings\n Current session: 83% used · resets 3:30pm (UTC)\n\
               Current week (Fable): 0% used · resets Nov 16 at 2am (UTC)\n\
               Current week (all models): 4% used · resets Nov 16 at 2am (UTC)\n";
    let window = |id: &str, used_pct, at: &str| Window {
        id: id.to_owned(),
        used_pct,
        resets_at: when(at, now),
    };
    assert_eq!(
        c.read_usage(out, now),
        Some(Usage {
            windows: vec![window("5h", 83, "3:30pm"), window("7d", 4, "Nov 16 at 2am")],
            read_at: now,
        })
    );
    let elsewhere = c.read_usage(
        "Current session: 4% used · resets Nov 16 at 2am (Asia/Bangkok)",
        now,
    );
    let windows = elsewhere.map(|u| u.windows).unwrap_or_default();
    assert_eq!(
        windows,
        [Window {
            resets_at: None,
            ..window("5h", 4, "")
        }]
    );
    assert_eq!(c.read_usage("Current week (Fable): 0% used", now), None);
    assert_eq!(c.read_usage("", now), None);
}

#[test]
fn the_records_carry_the_account_keys() {
    let c = record("claude");
    assert!(c.setup_files.iter().any(|f| f == "settings.json"));
    assert_eq!(c.scrub, ["CLAUDE_CODE_OAUTH_TOKEN"]);
    assert_eq!(c.token.map(|t| t.days), Some(365));
    assert_eq!(c.proxy.map(|p| p.paths), Some(vec!["/v1/".to_owned()]));
    let gemini = record("gemini");
    assert_eq!(gemini.config_dir_var.as_deref(), Some("GEMINI_CLI_HOME"));
    assert_eq!(gemini.login.as_deref(), Some("gemini"));
    let codex = record("codex");
    assert!(codex.usage.is_none() && codex.proxy.is_none() && codex.status_read.is_none());
    let agy = record("antigravity-acp");
    assert_eq!(agy.id, "antigravity-acp");
    assert_eq!(agy.config_dir_var.as_deref(), Some("GEMINI_HOME"));
    assert_eq!(agy.model_env.as_deref(), Some("AGY_ACP_DEFAULT_MODEL"));
    assert_eq!(
        agy.env
            .get("AGY_ACP_FORCE_FILE_STORAGE")
            .map(String::as_str),
        Some("1")
    );
    assert!(agy.logout.is_none() && agy.token_var.is_none());
    let line = "Usage Limit Reached\n\nYou have reached your current quota for this period. Your limit will reset in 4 hours.";
    let hit = agy.limit_hit(line).map(|h| h.1);
    assert_eq!(hit, Some(Some("4 hours".to_owned())));
}
