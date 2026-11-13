//! `docs/protocol.md` and `docs/shed.openrpc.json` are generated from the types; these tests fail when
//! they drift. Regenerate with `just generate`. One golden example file per domain.
#![cfg(test)]

mod acp;
mod machine;
mod render;
mod screen;
mod terminal;

use serde_json::{Map, Value, json, to_value};
use shed_proto::{ErrorCode, Hello, Method, PROTOCOL, RpcError, openrpc};

use acp::{emit_params, events};
use screen::screen_updates;
use terminal::run_params;

fn peer(build: &str, protocol: u32) -> Hello {
    Hello {
        build: build.into(),
        protocol,
        ..Hello::ours()
    }
}

fn example(method: Method) -> (Value, Value) {
    let hello = || to_value(peer("4254d78", PROTOCOL)).unwrap();
    match method {
        Method::Hello => (hello(), hello()),
        Method::Heartbeat => (json!({}), json!({})),
        Method::Discover => {
            let doc = openrpc();
            (
                json!({}),
                json!({"openrpc": doc["openrpc"], "info": doc["info"]}),
            )
        }
        _ => terminal::example(method)
            .or_else(|| acp::example(method))
            .or_else(|| screen::example(method))
            .or_else(|| machine::example(method))
            .unwrap_or_else(|| panic!("no golden example for {method:?}")),
    }
}

fn check(doc: &Value, schema: &Value, v: &Value) -> Result<(), String> {
    let Some(s) = schema.as_object() else {
        return if *schema == json!(false) {
            Err(format!("nothing allowed: {v}"))
        } else {
            Ok(())
        };
    };
    let known = [
        "$ref",
        "type",
        "const",
        "enum",
        "minimum",
        "maximum",
        "required",
        "properties",
        "additionalProperties",
        "items",
        "minItems",
        "maxItems",
        "oneOf",
        "anyOf",
        "description",
        "format",
        "default",
        "title",
    ];
    if let Some(k) = s.keys().find(|k| !known.contains(&k.as_str())) {
        return Err(format!("the checker does not know {k}"));
    }
    let fail = |why: String| Err(format!("{why}: {v}"));
    if let Some(r) = s.get("$ref").and_then(Value::as_str) {
        check(doc, doc.pointer(r.trim_start_matches('#')).ok_or(r)?, v)?;
    }
    let types: Vec<&str> = match s.get("type") {
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).collect(),
        Some(t) => vec![t.as_str().unwrap()],
        None => vec!["object", "array", "string", "boolean", "null", "number"],
    };
    let is = |t: &&str| match *t {
        "object" => v.is_object(),
        "array" => v.is_array(),
        "string" => v.is_string(),
        "boolean" => v.is_boolean(),
        "null" => v.is_null(),
        "integer" => v.is_i64() || v.is_u64(),
        "number" => v.is_number(),
        t => panic!("unknown type {t}"),
    };
    if !types.iter().any(is) {
        return fail(format!("not {types:?}"));
    }
    if s.get("const").is_some_and(|c| c != v) {
        return fail(format!("not {}", s["const"]));
    }
    if s.get("enum")
        .and_then(Value::as_array)
        .is_some_and(|e| !e.contains(v))
    {
        return fail("not in the enum".into());
    }
    let (n, min, max) = (v.as_f64(), s.get("minimum"), s.get("maximum"));
    if n.zip(min.and_then(Value::as_f64))
        .is_some_and(|(n, m)| n < m)
        || n.zip(max.and_then(Value::as_f64))
            .is_some_and(|(n, m)| n > m)
    {
        return fail("out of range".into());
    }
    match v {
        Value::Object(obj) => check_object(doc, s, obj)?,
        Value::Array(items) => check_array(doc, s, items)?,
        _ => {}
    }
    let ok = |key: &str| {
        let subs = s.get(key).and_then(Value::as_array);
        subs.map(|subs| subs.iter().filter(|sub| check(doc, sub, v).is_ok()).count())
    };
    match (ok("oneOf"), ok("anyOf")) {
        (Some(n), _) if n != 1 => fail(format!("{n} of oneOf match")),
        (_, Some(0)) => fail("no anyOf matches".into()),
        _ => Ok(()),
    }
}

fn check_object(
    doc: &Value,
    s: &Map<String, Value>,
    obj: &Map<String, Value>,
) -> Result<(), String> {
    let props = s.get("properties").and_then(Value::as_object);
    for key in s
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if !obj.contains_key(key.as_str().unwrap()) {
            return Err(format!("no {key}"));
        }
    }
    for (key, x) in obj {
        let sub = props.and_then(|p| p.get(key));
        let sub = sub
            .or(s.get("additionalProperties"))
            .unwrap_or(&Value::Bool(true));
        check(doc, sub, x).map_err(|e| format!("{key}: {e}"))?;
    }
    Ok(())
}

fn check_array(doc: &Value, s: &Map<String, Value>, items: &[Value]) -> Result<(), String> {
    let len = u64::try_from(items.len()).ok();
    if s.get("minItems").and_then(Value::as_u64) > len
        || s.get("maxItems")
            .and_then(Value::as_u64)
            .is_some_and(|m| Some(m) < len)
    {
        return Err(format!("{} items", items.len()));
    }
    items
        .iter()
        .try_for_each(|x| check(doc, s.get("items").unwrap_or(&Value::Bool(true)), x))
}

fn method_doc<'a>(doc: &'a Value, name: &str) -> &'a Value {
    let methods = doc["methods"].as_array().unwrap();
    methods.iter().find(|m| m["name"] == name).unwrap()
}

fn check_params(doc: &Value, method: &Value, params: &Value) -> Result<(), String> {
    let descriptors = method["params"].as_array().unwrap();
    let schema = json!({
        "type": "object",
        "properties": descriptors.iter().map(|d| (d["name"].as_str().unwrap().to_owned(), d["schema"].clone())).collect::<Map<_, _>>(),
        "required": descriptors.iter().filter(|d| d["required"] == true).map(|d| d["name"].clone()).collect::<Vec<_>>(),
    });
    check(doc, &schema, params)?;
    method
        .get("x-params")
        .map_or(Ok(()), |s| check(doc, s, params))
}

#[test]
fn every_method_is_in_the_openrpc_document() {
    let doc = openrpc();
    let names: Vec<&str> = doc["methods"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, Method::ALL.map(Method::name));
    assert!(names.contains(&"rpc.discover"));
}

#[test]
fn every_golden_message_validates_against_its_schema() {
    let doc = openrpc();
    for method in Method::ALL {
        let (params, result) = example(method);
        let m = method_doc(&doc, method.name());
        check_params(&doc, m, &params).unwrap_or_else(|e| panic!("{} params: {e}", method.name()));
        check(&doc, &m["result"]["schema"], &result)
            .unwrap_or_else(|e| panic!("{} result: {e}", method.name()));
    }
    let stream = &method_doc(&doc, "events")["x-stream"];
    for event in events() {
        check(&doc, stream, &to_value(&event).unwrap()).unwrap_or_else(|e| panic!("{e}"));
    }
    let updates = &method_doc(&doc, "screen.watch")["x-stream"];
    for update in screen_updates() {
        check(&doc, updates, &to_value(&update).unwrap()).unwrap_or_else(|e| panic!("{e}"));
    }
    let refusal = RpcError::version(&peer("02fcf97", PROTOCOL + 1), &peer("56de739", PROTOCOL));
    let error = &doc["components"]["errors"]["SHED_VERSION"];
    assert_eq!(error["code"], refusal.code);
    assert!(
        refusal
            .message
            .starts_with(error["message"].as_str().unwrap())
    );
    assert_eq!(
        doc["components"]["errors"].as_object().unwrap().len(),
        ErrorCode::ALL.len()
    );
}

#[test]
fn the_schemas_refuse_broken_messages() {
    let doc = openrpc();
    let mut run = run_params();
    run.as_object_mut().unwrap().remove("cmd");
    assert!(check_params(&doc, method_doc(&doc, "run"), &run).is_err());
    let mut emit = emit_params();
    emit["event"] = json!("nope");
    assert!(check_params(&doc, method_doc(&doc, "emit"), &emit).is_err());
    let stream = &method_doc(&doc, "events")["x-stream"];
    let mut event = to_value(&events()[0]).unwrap();
    event["event"] = json!("nope");
    assert!(check(&doc, stream, &event).is_err());
    let updates = &method_doc(&doc, "screen.watch")["x-stream"];
    let mut update = to_value(&screen_updates()[0]).unwrap();
    update["cursor"] = json!([1, 2, 3]);
    assert!(check(&doc, updates, &update).is_err());
    update["cursor"] = json!([1, -2]);
    assert!(check(&doc, updates, &update).is_err());
    let input = json!({"name": "build", "text": "make lint"});
    assert!(check_params(&doc, method_doc(&doc, "terminal.input"), &input).is_err());
}

#[test]
fn method_names_round_trip() {
    for method in Method::ALL {
        assert_eq!(Method::parse(method.name()), Some(method));
    }
    assert_eq!(Method::parse("nope"), None);
}

#[test]
fn version_refusal_reads_as_specified() {
    assert_eq!(
        RpcError::version(&peer("02fcf97", 2), &peer("56de739", 1)).message,
        "SHED_VERSION: client 02fcf97 / server 56de739: protocol 2 vs 1; restart shed serve"
    );
}
