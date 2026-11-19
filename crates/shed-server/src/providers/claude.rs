use serde_json::{Map, Value, json};
use shed_proto::{AcpUsage, Stream};

use crate::drivers::stream::Wire;
use crate::lane::{Heard, Tool};

pub(crate) fn wire() -> Box<dyn Wire> {
    Box::new(Claude::default())
}

#[derive(Default)]
struct Claude {
    model: Option<String>,
    mode: Option<String>,
    used: u64,
}

impl Wire for Claude {
    fn prompt(&self, text: &str) -> String {
        let content = [json!({"type": "text", "text": text})];
        json!({"type": "user", "message": {"role": "user", "content": content}}).to_string()
    }

    fn heard(&mut self, line: &Value, record: &Stream) -> Vec<Heard> {
        let main = line["parent_tool_use_id"].is_null();
        match (line["type"].as_str(), line["subtype"].as_str()) {
            (Some("system"), Some("init")) => self.init(line, record),
            (Some("user"), _) if line["isReplay"] == true => vec![Heard::Prompted],
            (Some("user"), _) if main => results(line),
            (Some("stream_event"), _) if main => started(&line["event"], record),
            (Some("assistant"), _) if main => self.calls(&line["message"], record),
            (Some("rate_limit_event"), _) => limit(&line["rate_limit_info"]),
            (Some("result"), _) => self.end(line),
            _ => Vec::new(),
        }
    }

    fn mcp(&self, servers: &[Value]) -> String {
        let pairs = |list: &Value| -> Map<String, Value> {
            let pair = |p: &Value| Some((p["name"].as_str()?.to_owned(), p["value"].clone()));
            list.as_array().into_iter().flatten().filter_map(pair).collect()
        };
        let server = |s: &Value| {
            let name = s["name"].as_str().unwrap_or_default().to_owned();
            let server = match s["type"].as_str().unwrap_or("stdio") {
                "stdio" => {
                    let args = s.get("args").cloned().unwrap_or_else(|| json!([]));
                    json!({"type": "stdio", "command": s["command"], "args": args, "env": pairs(&s["env"])})
                }
                kind => json!({"type": kind, "url": s["url"], "headers": pairs(&s["headers"])}),
            };
            (name, server)
        };
        let servers: Map<String, Value> = servers.iter().map(server).collect();
        json!({ "mcpServers": servers }).to_string()
    }
}

impl Claude {
    fn init(&mut self, line: &Value, record: &Stream) -> Vec<Heard> {
        let mut heard = Vec::new();
        let model = line["model"].as_str().map(str::to_owned);
        if model.is_some() && model != self.model {
            self.model.clone_from(&model);
            let models = Some(record.models.clone());
            let commands = None;
            heard.push(Heard::Session { model, models, commands });
        }
        let name = |c: &Value| c.as_str().map(|c| [c.to_owned(), String::new(), String::new()]);
        let commands = line["slash_commands"].as_array().into_iter().flatten();
        let commands = Some(commands.filter_map(name).collect());
        heard.push(Heard::Session { model: None, models: None, commands });
        let mode = line["permissionMode"].as_str().map(str::to_owned);
        if mode.is_some() && mode != self.mode {
            self.mode.clone_from(&mode);
            heard.extend(mode.map(Heard::Mode));
        }
        heard
    }

    fn calls(&mut self, message: &Value, record: &Stream) -> Vec<Heard> {
        let usage = &message["usage"];
        let tokens = ["input_tokens", "cache_read_input_tokens", "cache_creation_input_tokens"];
        self.used = tokens.iter().filter_map(|k| usage[k].as_u64()).sum();
        let mut heard = Vec::new();
        let calls = message["content"].as_array().into_iter().flatten();
        for call in calls.filter(|c| c["type"] == "tool_use") {
            let (name, input) = (call["name"].as_str().unwrap_or("tool"), &call["input"]);
            let [kind, key] = record.tools.get(name).cloned().unwrap_or_default();
            let title = input[&key].as_str().map_or_else(|| name.to_owned(), str::to_owned);
            let locations = (key.ends_with("_path")).then(|| title.clone().into());
            heard.push(Heard::Tool(Tool {
                id: call["id"].as_str().unwrap_or_default().to_owned(),
                title: Some(title),
                kind: Some(kind).filter(|k| !k.is_empty()),
                status: Some("in_progress".to_owned()),
                locations: locations.into_iter().collect(),
                raw_input: Some(input.clone()),
            }));
            if let Some([plan, key]) = &record.plan
                && plan == name
            {
                heard.extend(input[key].as_array().cloned().map(Heard::Plan));
            }
        }
        heard
    }

    fn end(&self, line: &Value) -> Vec<Heard> {
        let usage = &line["usage"];
        let count = |key: &str| usage[key].as_u64().unwrap_or_default();
        let counts = json!({
            "inputTokens": count("input_tokens"),
            "cachedInputTokens": count("cache_read_input_tokens"),
            "cachedWriteTokens": count("cache_creation_input_tokens"),
            "outputTokens": count("output_tokens"),
        });
        let meta = json!({"quota": {"token_count": counts}});
        let mut size = line["modelUsage"].as_object().into_iter().flatten();
        let size = size.find_map(|(_, m)| m["contextWindow"].as_u64());
        let cost = (line["total_cost_usd"].as_f64()).map(|c| json!({"amount": c, "currency": "USD"}));
        let usage = size.map(|size| AcpUsage {
            used: self.used,
            size,
            cost,
            turn: None,
            total: None,
        });
        let error = (line["is_error"] == true).then(|| {
            let errors = line["errors"].as_array().into_iter().flatten();
            let errors: Vec<_> = errors.filter_map(Value::as_str).collect();
            (line["result"].as_str()).map_or_else(|| errors.join("; "), str::to_owned)
        });
        let stop = line["stop_reason"].as_str().unwrap_or("end_turn").to_owned();
        let meta = meta.as_object().cloned();
        let end = Heard::TurnEnd { stop, error, meta };
        usage.map(|u| Heard::Usage(Some(u))).into_iter().chain([end]).collect()
    }
}

fn tool(id: &Value, status: &str) -> Tool {
    Tool {
        id: id.as_str().unwrap_or_default().to_owned(),
        title: None,
        kind: None,
        status: Some(status.to_owned()),
        locations: Vec::new(),
        raw_input: None,
    }
}

fn results(line: &Value) -> Vec<Heard> {
    let content = line.pointer("/message/content").and_then(Value::as_array).into_iter().flatten();
    let results = content.filter(|c| c["type"] == "tool_result");
    let status = |c: &Value| if c["is_error"] == true { "failed" } else { "completed" };
    results.map(|c| Heard::Tool(tool(&c["tool_use_id"], status(c)))).collect()
}

fn started(event: &Value, record: &Stream) -> Vec<Heard> {
    let block = &event["content_block"];
    match (event["type"].as_str(), block["type"].as_str()) {
        (Some("content_block_delta"), _) => {
            let delta = event.get("delta").filter(|d| d["type"] == "text_delta");
            let text = delta.and_then(|d| d["text"].as_str());
            text.map(|t| Heard::Text(t.to_owned())).into_iter().collect()
        }
        (Some("content_block_start"), Some("tool_use")) => {
            let name = block["name"].as_str().unwrap_or("tool");
            let kind = record.tools.get(name).map(|[kind, _]| kind.clone());
            let mut call = tool(&block["id"], "pending");
            (call.title, call.kind) = (Some(name.to_owned()), kind);
            vec![Heard::Tool(call)]
        }
        _ => Vec::new(),
    }
}

fn limit(info: &Value) -> Vec<Heard> {
    let rejected = info["status"] == "rejected";
    let resets = info["resetsAt"].as_u64().map(|at| at.to_string());
    rejected.then_some(Heard::Limit(resets)).into_iter().collect()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use serde_json::{Value, json};
    use shed_proto::{Provider, Tokens};

    use super::Claude;
    use crate::drivers::stream::Wire as _;
    use crate::lane::Heard;

    fn record() -> Provider {
        crate::providers::find(Path::new("/no-such-dir"), "claude-acp")
    }

    fn show(heard: &Heard, provider: &Provider) -> String {
        match heard {
            Heard::Session { model, models, commands } => format!(
                "session {model:?} {} {:?}",
                models.as_ref().map_or(0, Vec::len),
                commands.as_ref().and_then(|c| c.iter().find(|c| c[0] == "compact"))
            ),
            Heard::Prompted => "prompted".to_owned(),
            Heard::Text(_) => "text".to_owned(),
            Heard::Tool(t) => format!(
                "tool {} {:?} {:?} {:?} {:?} {}",
                t.id, t.title, t.kind, t.status, t.locations, t.raw_input.is_some()
            ),
            Heard::Plan(p) => format!("plan {p:?}"),
            Heard::Mode(m) => format!("mode {m}"),
            Heard::Usage(u) => format!("usage {}", serde_json::to_string(u).unwrap_or_default()),
            Heard::Limit(at) => format!("limit {at:?}"),
            Heard::TurnEnd { stop, error, meta } => {
                let tokens = meta.as_ref().and_then(|m| provider.tokens(m));
                format!("end {stop} {error:?} {tokens:?}")
            }
            Heard::Ask(_) | Heard::Skipped(_) | Heard::Note(_) => "other".to_owned(),
        }
    }

    #[test]
    fn a_recorded_session_reads_as_shed_hears_it() {
        let provider = record();
        let stream = provider.stream.clone().unwrap_or_default();
        let fixture = include_str!("../../tests/fixtures/claude-stream.jsonl");
        let mut wire = Claude::default();
        let (mut text, mut seen) = (String::new(), Vec::new());
        let mut result = String::new();
        for line in fixture.lines() {
            let line: Value = serde_json::from_str(line).unwrap_or_default();
            if line["type"] == "result" {
                result = line["result"].as_str().unwrap_or_default().to_owned();
            }
            for heard in wire.heard(&line, &stream) {
                match heard {
                    Heard::Text(chunk) => text.push_str(&chunk),
                    heard => seen.push(show(&heard, &provider)),
                }
            }
        }
        let p = "/home/user/project/missing.txt";
        let want = [
            "session Some(\"claude-haiku-5-5\") 4 None".to_owned(),
            "session None 0 Some([\"compact\", \"\", \"\"])".to_owned(),
            "mode bypassPermissions".to_owned(),
            "prompted".to_owned(),
            "tool toolu_011S7X43YDhFun28C7kdu5EV Some(\"Bash\") Some(\"execute\") Some(\"pending\") [] false".to_owned(),
            "tool toolu_011S7X43YDhFun28C7kdu5EV Some(\"echo hi\") Some(\"execute\") Some(\"in_progress\") [] true".to_owned(),
            "tool toolu_01VYNFgj92R5N4E6eHmMYytH Some(\"Read\") Some(\"read\") Some(\"pending\") [] false".to_owned(),
            "tool toolu_011S7X43YDhFun28C7kdu5EV None None Some(\"completed\") [] false".to_owned(),
            format!("tool toolu_01VYNFgj92R5N4E6eHmMYytH Some({p:?}) Some(\"read\") Some(\"in_progress\") [{p:?}] true"),
            "tool toolu_01LeE8tGhGih4S9HBqXyVqPi Some(\"ToolSearch\") None Some(\"pending\") [] false".to_owned(),
            "tool toolu_01VYNFgj92R5N4E6eHmMYytH None None Some(\"failed\") [] false".to_owned(),
            "tool toolu_01LeE8tGhGih4S9HBqXyVqPi Some(\"ToolSearch\") None Some(\"in_progress\") [] true".to_owned(),
            "tool toolu_01LeE8tGhGih4S9HBqXyVqPi None None Some(\"completed\") [] false".to_owned(),
            "usage {\"used\":19165,\"size\":1000000,\"cost\":{\"amount\":0.00217633,\"currency\":\"USD\"}}".to_owned(),
            format!("end end_turn None {:?}", Some(Tokens { input: 4, cache_read: 29983, cache_write: 7768, output: 645 })),
        ];
        assert_eq!(seen, want);
        assert!(!result.is_empty());
        assert_eq!(text, result);
    }

    #[test]
    fn a_rejected_limit_an_error_and_a_plan_are_heard() {
        let stream = record().stream.unwrap_or_default();
        let mut wire = Claude::default();
        let mut hear = |line: Value| -> Vec<String> {
            let provider = record();
            wire.heard(&line, &stream).iter().map(|h| show(h, &provider)).collect()
        };
        let info = json!({"status": "rejected", "resetsAt": 1_795_150_800, "rateLimitType": "five_hour"});
        let limit = json!({"type": "rate_limit_event", "rate_limit_info": info});
        assert_eq!(hear(limit), ["limit Some(\"1795150800\")"]);
        let error = json!({"type": "result", "subtype": "success", "is_error": true,
            "result": "Failed to authenticate. API Error: 401", "stop_reason": null});
        assert_eq!(hear(error), ["end end_turn Some(\"Failed to authenticate. API Error: 401\") Some(Tokens { input: 0, cache_read: 0, cache_write: 0, output: 0 })"]);
        let todos = json!([{"content": "a", "status": "pending"}]);
        let call = json!({"type": "tool_use", "id": "t1", "name": "TodoWrite", "input": {"todos": todos}});
        let plan = json!({"type": "assistant", "parent_tool_use_id": null, "message": {"content": [call]}});
        let heard = hear(plan);
        assert_eq!(heard.get(1).map(String::as_str), Some("plan [Object {\"content\": String(\"a\"), \"status\": String(\"pending\")}]"));
        let sub = json!({"type": "stream_event", "parent_tool_use_id": "t1",
            "event": {"type": "content_block_delta", "delta": {"type": "text_delta", "text": "inner"}}});
        assert!(hear(sub).is_empty());
    }

    #[test]
    fn mcp_servers_become_claudes_mcp_config() {
        let servers = [
            json!({"name": "fs", "command": "fs-mcp", "args": ["--ro"], "env": [{"name": "K", "value": "V"}]}),
            json!({"type": "http", "name": "web", "url": "https://example.com/mcp",
                "headers": [{"name": "Authorization", "value": "Bearer x"}]}),
        ];
        let text = Claude::default().mcp(&servers);
        let got: Value = serde_json::from_str(&text).unwrap_or_default();
        let want = json!({"mcpServers": {
            "fs": {"type": "stdio", "command": "fs-mcp", "args": ["--ro"], "env": {"K": "V"}},
            "web": {"type": "http", "url": "https://example.com/mcp", "headers": {"Authorization": "Bearer x"}},
        }});
        assert_eq!(got, want);
    }
}
