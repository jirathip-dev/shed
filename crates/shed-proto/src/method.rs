//! The methods and error codes, collected from `methods/*.md` and `errors/*.md`, and the `OpenRPC` document.

#[cfg(feature = "schema")]
use std::collections::BTreeMap;

#[cfg(feature = "schema")]
use schemars::generate::SchemaSettings;
#[cfg(feature = "schema")]
use schemars::{JsonSchema, Schema, SchemaGenerator};
#[cfg(feature = "schema")]
use serde_json::{Value, json};

#[cfg(feature = "schema")]
use crate::{PROTOCOL, SessionRecord, VERSION};

include!(concat!(env!("OUT_DIR"), "/methods.rs"));
include!(concat!(env!("OUT_DIR"), "/errors.rs"));

/// The field that names the machine a held link carries a request to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// `machine`: the machine itself.
    Machine,
    /// `name`: the `<machine>/` before a terminal's name.
    Name,
}

impl Method {
    /// The method with this wire name.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == name)
    }
}

#[cfg(feature = "schema")]
fn pair<P: JsonSchema, R: JsonSchema>(g: &mut SchemaGenerator) -> [Schema; 2] {
    [P::json_schema(g), g.subschema_for::<R>()]
}

#[cfg(feature = "schema")]
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
struct Empty {}

#[cfg(feature = "schema")]
fn terminal_for_name([mut params, result]: [Schema; 2]) -> [Schema; 2] {
    for (key, value) in params.as_object_mut().into_iter().flatten() {
        match (key.as_str(), value) {
            ("properties", Value::Object(props)) => {
                if let Some(name) = props.remove("name") {
                    props.insert("terminal".to_owned(), name);
                }
            }
            ("required", Value::Array(names)) => {
                for name in names.iter_mut().filter(|n| *n == "name") {
                    *name = "terminal".into();
                }
            }
            _ => {}
        }
    }
    [params, result]
}

#[cfg(feature = "schema")]
/// The whole API as one `OpenRPC` document (`shed schema`, `rpc.discover`): every method's params and result,
/// the streamed [`Event`](crate::Event)s and [`ScreenUpdate`](crate::ScreenUpdate)s (`x-stream`) and the error codes, as JSON Schema.
#[must_use]
pub fn openrpc() -> Value {
    let mut g = SchemaSettings::draft2020_12()
        .with(|s| s.definitions_path = "/components/schemas".into())
        .into_generator();
    let methods: Vec<Value> = Method::ALL.map(|m| describe(m, &mut g)).into();
    let record = g.subschema_for::<SessionRecord>();
    let errors: serde_json::Map<String, Value> = ErrorCode::ALL
        .into_iter()
        .map(|c| {
            (
                c.name().to_owned(),
                json!({"code": c.code(), "message": c.name()}),
            )
        })
        .collect();
    json!({"openrpc": "1.3.2", "info": {"title": "shed", "version": VERSION, "x-protocol": PROTOCOL}, "methods": methods,
        "x-files": {"sessions/<id>.json": record}, "components": {"schemas": g.take_definitions(true), "errors": errors}})
}

#[cfg(feature = "schema")]
fn describe(method: Method, g: &mut SchemaGenerator) -> Value {
    let [params, result] = method.schemas(g);
    let required = params.get("required").and_then(Value::as_array);
    let descriptors: Vec<Value> = params
        .get("properties")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .map(|(name, schema)| {
            let required = required.is_some_and(|r| r.contains(&name.as_str().into()));
            json!({"name": name, "required": required, "schema": schema})
        })
        .collect();
    let extras = [
        (
            "x-params",
            params.get("oneOf").map(|_| Value::from(params.clone())),
        ),
        ("x-stream", method.stream(g).map(Value::from)),
    ];
    let mut doc = json!({
        "name": method.name(),
        "description": method.summary(),
        "paramStructure": "by-name",
        "params": descriptors,
        "result": {"name": "result", "schema": result},
    });
    if let Value::Object(map) = &mut doc {
        map.extend(
            extras
                .into_iter()
                .filter_map(|(k, v)| Some((k.to_owned(), v?))),
        );
    }
    doc
}
