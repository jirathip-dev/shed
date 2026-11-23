use serde_json::{Value, json, to_value};
use shed_proto::{Acting, ActingError, Caller, Method, ScriptError, ScriptParams, ScriptResult};

pub(crate) fn example(method: Method) -> Option<(Value, Value)> {
    let Method::Script = method else {
        return None;
    };
    let params = ScriptParams {
        code: "await shed.kill({ name: \"impl-190\" });\nreturn \"killed\";".into(),
        timeout: Some(120.0),
        caller: Some(Caller {
            terminal: Some("orchestrator-7".into()),
            role: Some("orchestrator".into()),
            ..Caller::default()
        }),
        ..ScriptParams::default()
    };
    let message = "kill refused by the guard: lanes are stopped with drover kill";
    let result = ScriptResult {
        error: Some(ScriptError {
            name: "GuardError".into(),
            message: message.into(),
            line: 1,
            column: 12,
            code: Some("GUARD_REFUSED".into()),
            method: Some("kill".into()),
        }),
        calls: 1,
        acting: vec![Acting {
            method: "kill".into(),
            params: Some(json!({"name": "impl-190"})),
            result: None,
            error: Some(ActingError {
                code: "GUARD_REFUSED".into(),
                message: message.into(),
            }),
        }],
        ms: 21,
        ..ScriptResult::default()
    };
    Some((to_value(params).unwrap(), to_value(result).unwrap()))
}

#[test]
fn skill_methods_md_matches_the_methods() {
    use std::fmt::Write as _;
    let mut want = String::from(
        "# shed's methods in a script\n\n\
         <!-- Generated from crates/shed-proto by `just generate`; do not edit. -->\n\n\
         Call one as `shed.<method>(params)`; `shed.describe(\"<method>\")` gives its params and result.\n\
         A `read` is called as it is, a `guarded` one goes through the server's guard first (refused with \
         none), and a `refused` one is a `ShedError` `NOT_IN_SCRIPT`.\n\n\
         | Method | In a script | What it does |\n|---|---|---|\n",
    );
    for m in Method::ALL {
        let summary = m.summary().replace('|', "\\|");
        writeln!(
            want,
            "| `{}` | {} | {summary} |",
            m.name(),
            m.in_script().name()
        )
        .unwrap();
    }
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skill/script/methods.md");
    if std::env::var_os("SHED_BLESS").is_some() {
        std::fs::write(&path, &want).unwrap();
    }
    let have = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        have == want,
        "skill/script/methods.md is stale; run `just generate`"
    );
}
