//! The catalog's layers, its built-in records and `pick`.

use std::path::{Path, PathBuf};

use shed_catalog::Catalog;
use shed_proto::{AgentInfo, DriverKind, Picked};

fn host() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/host")
}

fn owned(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|&(k, v)| (k.to_owned(), v.to_owned()))
        .collect()
}

fn words(text: &[&str]) -> Vec<String> {
    text.iter().map(|&w| w.to_owned()).collect()
}

#[test]
fn each_built_in_record_is_one_file_under_its_id_and_gemini_has_one_id() {
    let catalog = Catalog::built_in();
    let ids: Vec<_> = catalog
        .iter()
        .map(|e| (e.provider.id.as_str(), e.source.as_str()))
        .collect();
    assert_eq!(
        ids,
        [
            ("antigravity-acp", "built-in"),
            ("claude-acp", "built-in"),
            ("codex-acp", "built-in"),
            ("deepseek", "built-in"),
            ("gemini", "built-in"),
            ("opencode", "built-in"),
            ("pi-acp", "built-in")
        ]
    );
    let gemini = catalog.by_agent("gemini").cloned().unwrap_or_default();
    assert_eq!(gemini.id, "gemini");
    assert_eq!(gemini.token_var.as_deref(), Some("GEMINI_API_KEY"));
    let claude = catalog.by_id("claude-acp").cloned().unwrap_or_default();
    assert_eq!(claude.models.len(), 4);
    assert_eq!(claude.login.as_deref(), Some("claude auth login"));
    assert!(catalog.by_id("gemini-acp").is_none());
}

#[test]
fn a_host_file_sets_its_keys_over_the_built_in_record_and_a_bad_one_is_skipped() {
    let (catalog, bad) = Catalog::load(&host());
    let claude = catalog.by_id("claude-acp").cloned().unwrap_or_default();
    assert_eq!(
        claude.limit_hit("you are out of tokens until 5pm."),
        Some(("you are out of tokens until 5pm.", Some("5pm".to_owned())))
    );
    assert_eq!(
        claude.auth_failure("OAuth session expired"),
        Some("OAuth session expired")
    );
    let codex = catalog.by_id("codex-acp").map(|p| p.limit.clone());
    assert_eq!(codex, Some(words(&["hit your usage limit"])));
    let file = host().join("fake.toml").display().to_string();
    let fake = catalog
        .iter()
        .find(|e| e.provider.id == "fake-acp")
        .map(|e| e.source.as_str());
    assert_eq!(fake, Some(file.as_str()));
    assert!(catalog.by_id("new-acp").is_none());
    let keyed = catalog.by_id("keyed").map(|p| p.token_var.as_deref());
    assert_eq!(keyed, Some(Some("KEYED_API_KEY")));
    let named: Vec<_> = bad
        .iter()
        .filter_map(|b| b.split(".toml: ").next())
        .collect();
    let want = ["codex", "new"].map(|n| host().join(n).display().to_string());
    assert_eq!(named, want, "{bad:?}");
}

#[test]
fn an_apps_row_sets_its_keys_over_the_served_record_and_an_unknown_key_names_the_row() {
    let served: Vec<AgentInfo> = (Catalog::load(&host()).0.iter())
        .map(|e| AgentInfo {
            id: e.provider.id.clone(),
            source: Some(e.source.clone()),
            provider: Some(e.provider.clone()),
            ..AgentInfo::default()
        })
        .collect();
    let catalog = Catalog::served(&served);
    let row = r#"login = "codex login --device-auth""#;
    let catalog = catalog.over("codex", row).unwrap_or_default();
    let codex = catalog.by_agent("codex").cloned().unwrap_or_default();
    assert_eq!(codex.login.as_deref(), Some("codex login --device-auth"));
    assert_eq!(codex.config_dir_var.as_deref(), Some("CODEX_HOME"));
    let source = catalog
        .iter()
        .find(|e| e.provider.agent == "codex")
        .map(|e| e.source.as_str());
    assert_eq!(source, Some("app"));
    assert!(catalog.by_id("fake-acp").is_some());
    let shared: Vec<_> = ["opencode", "pi-acp"]
        .iter()
        .filter_map(|id| catalog.by_id(id).map(|p| p.agent.as_str()))
        .collect();
    assert_eq!(shared, ["deepseek", "deepseek"]);
    assert!(
        catalog
            .by_id("deepseek")
            .is_some_and(|p| p.balance.is_some())
    );
    let unknown = catalog
        .clone()
        .over("claude", "logins = \"x\"")
        .err()
        .unwrap_or_default();
    assert!(
        unknown.starts_with("claude: ") && unknown.contains("logins"),
        "{unknown}"
    );
    let added = catalog
        .over("solo", "id = \"solo-acp\"\nagent = \"solo\"")
        .unwrap_or_default();
    assert_eq!(
        added.by_agent("solo").map(|p| p.id.as_str()),
        Some("solo-acp")
    );
}

#[test]
fn pick_places_a_model_and_an_effort_for_the_driver_that_runs() {
    let catalog = Catalog::built_in();
    let claude = catalog.by_id("claude-acp").cloned().unwrap_or_default();
    let acp = claude.pick(DriverKind::Acp, Some("opus"), Some("max"));
    let want = Picked {
        env: owned(&[("ANTHROPIC_MODEL", "opus")]),
        args: words(&["--effort", "max"]),
    };
    assert_eq!(acp, Ok(want));
    let stream = claude.pick(DriverKind::Stream, Some("opus"), Some("low"));
    let want = Picked {
        env: Vec::new(),
        args: words(&["--model", "opus", "--effort", "low"]),
    };
    assert_eq!(stream, Ok(want));
    assert_eq!(
        claude.pick(DriverKind::Acp, None, None),
        Ok(Picked::default())
    );
    let wrong = claude.pick(DriverKind::Acp, None, Some("huge"));
    assert_eq!(
        wrong,
        Err("claude-acp takes no effort huge on this driver".to_owned())
    );
    let gemini = catalog.by_id("gemini").cloned().unwrap_or_default();
    let model = gemini
        .pick(DriverKind::Acp, Some("flash"), None)
        .map(|p| p.env);
    assert_eq!(model, Ok(owned(&[("GEMINI_MODEL", "flash")])));
    assert!(gemini.pick(DriverKind::Acp, None, Some("low")).is_err());
    let codex = catalog.by_id("codex-acp").cloned().unwrap_or_default();
    assert!(codex.pick(DriverKind::Acp, Some("any"), None).is_err());
}
