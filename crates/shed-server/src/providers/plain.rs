use serde_json::{Value, json};

use shed_proto::Stream;

use crate::drivers::stream::Wire;
use crate::lane::Heard;

pub(crate) fn wire() -> Box<dyn Wire> {
    Box::new(Plain)
}

struct Plain;

impl Wire for Plain {
    fn prompt(&self, text: &str) -> String {
        json!({ "prompt": text }).to_string()
    }

    fn heard(&mut self, line: &Value, _: &Stream) -> Vec<Heard> {
        vec![serde_json::from_value(line.clone()).unwrap_or_else(|e| Heard::Skipped(e.to_string()))]
    }

    fn mcp(&self, servers: &[Value]) -> String {
        Value::from(servers).to_string()
    }
}
