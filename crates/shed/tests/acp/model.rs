use serde_json::{Value, json};
use shed_server::Paths;

use crate::common::{self, Server, temp_dir};
use crate::{first_turn, ok, shed, start_fake, start_with_history, until};

#[test]
fn a_model_switch_is_reported_and_the_next_turn_uses_it() {
    let dir = temp_dir("acp-model");
    let _server = Server::start(&dir);
    start_fake(&dir, "m", "fake");
    let session = || {
        let info: Value = serde_json::from_str(&ok(&dir, &["get", "m"])).expect("JSON");
        info["session"].clone()
    };
    let models = json!([["fast", "Fast"], ["smart", "Smart"]]);
    let first = json!({"id": "sess-1", "driver": "acp", "model": "fast", "models": models});
    common::eventually("the session", || {
        let mut s = session();
        let ready = s.as_object_mut().and_then(|s| s.remove("ready_ms"));
        ready.is_some_and(|r| r.is_u64()) && s == first
    });
    let before_a_turn = shed(&dir, &["model", "m", "smart"]);
    assert!(before_a_turn.status.success(), "{before_a_turn:?}");
    assert_eq!(String::from_utf8_lossy(&before_a_turn.stderr), "");
    common::eventually("the switch", || session()["model"] == "smart");
    ok(&dir, &["send", "m", "hi"]);
    ok(&dir, &["wait", "m", "--until", "done", "--timeout", "10"]);
    assert_eq!(
        ok(&dir, &["read", "m", "--lines", "1"]),
        "[smart] you said: hi\n"
    );
    assert!(session()["turn_ended"].is_u64());
    assert_eq!(session().get("cache_likely"), None, "no provider data");
}

#[test]
fn an_api_login_sleeps_once_its_cache_is_likely_cold_and_a_switch_after_a_turn_says_it_drops_it() {
    let dir = temp_dir("acp-cache");
    let _server = Server::with_env(&dir, "3600", &[("SHED_TEST_API_KEY", "set")]);
    let providers = Paths::in_dir(&dir).providers();
    std::fs::create_dir_all(&providers).expect("providers dir");
    let record =
        "id = \"fake\"\n[cache]\nplan = 3600\napi = 3\napi_vars = [\"SHED_TEST_API_KEY\"]\n";
    std::fs::write(providers.join("fake.toml"), record).expect("write the record");
    start_with_history(&dir, "w", &dir.join("w.jsonl"), &[]);
    let session = || {
        let info: Value = serde_json::from_str(&ok(&dir, &["get", "w"])).expect("JSON");
        info["session"].clone()
    };
    let likely = || {
        let top = ok(&dir, &["top"]);
        let cell = |l: &str| l.split_whitespace().nth(9).map(str::to_owned);
        let head = top.lines().next().and_then(cell);
        (
            head,
            top.lines().find(|l| l.starts_with("w ")).and_then(cell),
        )
    };
    first_turn(&dir, "w");
    assert_eq!(session()["cache_lifetime"], 3);
    assert_eq!(session()["cache_likely"], "warm");
    assert_eq!(likely(), (Some("CACHE-LIKELY".into()), Some("warm".into())));
    let switch = shed(&dir, &["model", "w", "smart"]);
    assert!(switch.status.success(), "{switch:?}");
    assert_eq!(
        String::from_utf8_lossy(&switch.stderr),
        "shed: the model switch drops the prompt cache; the next turn reads the conversation uncached\n"
    );
    until(&dir, "w", "hibernated");
    common::eventually("a cold cache", || session()["cache_likely"] == "cold");
    assert_eq!(likely(), (Some("CACHE-LIKELY".into()), Some("cold".into())));
}
