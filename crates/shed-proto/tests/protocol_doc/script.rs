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
