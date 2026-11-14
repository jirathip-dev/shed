use std::fmt::Write as _;
use std::path::Path;

use serde_json::Value;
use shed_proto::{ErrorCode, Method, PROTOCOL, Request, Response, RpcError, VERSION, openrpc};

use crate::acp::events;
use crate::screen::screen_updates;
use crate::{example, peer};

fn line(v: &impl serde::Serialize) -> String {
    serde_json::to_string(v).unwrap()
}

fn sorted(v: Value) -> Value {
    match v {
        Value::Object(map) => {
            let mut entries: Vec<_> = map.into_iter().map(|(k, v)| (k, sorted(v))).collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Object(entries.into_iter().collect())
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sorted).collect()),
        v => v,
    }
}

fn prose<'a>(doc: &'a Value, schema: &str) -> &'a str {
    doc["components"]["schemas"][schema]["description"]
        .as_str()
        .unwrap()
}

fn markdown() -> String {
    let doc = openrpc();
    let mut md = format!(
        "# shed protocol\n\n\
         <!-- Generated from crates/shed-proto by `SHED_BLESS=1 cargo test -p shed-proto --features schema`; do not edit. -->\n\n\
         Version: `{VERSION}`, protocol `{PROTOCOL}`. Client and server must speak the same protocol.\n\n\
         Newline-delimited [JSON-RPC 2.0](https://www.jsonrpc.org/specification) over a Unix socket, mode 0600:\n\
         `$XDG_RUNTIME_DIR/shed.sock` (macOS: `~/Library/Caches/shed/shed.sock`). One request per line, one\n\
         response per line; a request without an `id` is a notification and gets no response.\n\n\
         Every connection starts with `hello`. Any other method first is refused with `NO_HELLO`; another\n\
         product or protocol is refused with `SHED_VERSION: client <sha> / server <sha>: protocol <a> vs <b>;\n\
         restart shed serve` (a peer that sends no `product` shows as `unnamed`, no `protocol` as 0).\n\n\
         {}\n\n## Methods\n",
        prose(&doc, "TerminalInfo")
    );
    for (method, described) in Method::ALL
        .into_iter()
        .zip(doc["methods"].as_array().unwrap())
    {
        let (params, result) = example(method);
        let request = Request::new(1, method, sorted(params));
        let response = Response::ok(1.into(), sorted(result));
        write!(
            md,
            "\n### `{}`\n\n{}\n\n```json\n{}\n{}\n```\n",
            described["name"].as_str().unwrap(),
            described["description"].as_str().unwrap(),
            line(&request),
            line(&response)
        )
        .unwrap();
    }
    write!(md, "\n## Events\n\n{}\n\n```json\n", prose(&doc, "Event")).unwrap();
    for event in events() {
        writeln!(md, "{}", line(&event)).unwrap();
    }
    md.push_str("```\n");
    let screen = prose(&doc, "ScreenUpdate");
    write!(md, "\n## Screen updates\n\n{screen}\n\n```json\n").unwrap();
    for update in screen_updates() {
        writeln!(md, "{}", line(&update)).unwrap();
    }
    md.push_str("```\n");
    md.push_str("\n## Errors\n\nAn error's `message` starts with its name: `NAME: detail`.\n\n");
    md.push_str("| Code | Name |\n|---|---|\n");
    for code in ErrorCode::ALL {
        writeln!(md, "| {} | `{}` |", code.code(), code.name()).unwrap();
    }
    let refusal = Response::err(
        1.into(),
        RpcError::version(&peer("02fcf97", PROTOCOL + 1), &peer("56de739", PROTOCOL)),
    );
    write!(md, "\n```json\n{}\n```\n", line(&refusal)).unwrap();
    md
}

#[test]
fn protocol_md_matches_the_types() {
    let want = markdown();
    let have = bless("protocol.md", &want);
    assert!(
        have == want,
        "docs/protocol.md is stale; run `just generate`"
    );
}

fn bless(file: &str, want: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs")
        .join(file);
    if std::env::var_os("SHED_BLESS").is_some() {
        std::fs::write(&path, want).unwrap();
    }
    std::fs::read_to_string(&path).unwrap_or_default()
}

#[test]
fn openrpc_json_matches_the_types() {
    let have = bless("shed.openrpc.json", &format!("{:#}\n", openrpc()));
    assert!(
        serde_json::from_str::<Value>(&have).ok() == Some(openrpc()),
        "docs/shed.openrpc.json is stale; run `just generate`"
    );
}
