//! The ACP driver: the agent's ACP adapter on stdio, spoken through the `agent-client-protocol` SDK
//! (v1). It turns the agent's updates and permission requests into what a lane hears.

use std::collections::{BTreeSet, VecDeque};
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::v1::schema::{
    AgentCapabilities, AvailableCommandInput, CancelNotification, CloseSessionRequest,
    ContentBlock, ContentChunk, Implementation, InitializeRequest, LoadSessionRequest, McpServer,
    McpServerHttp, McpServerSse, Meta, NewSessionRequest, PromptRequest, PromptResponse,
    RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse,
    SelectedPermissionOutcome, SessionConfigKind as Kind, SessionConfigOption,
    SessionConfigOptionCategory as Category, SessionConfigOptionValue, SessionConfigSelectOption,
    SessionConfigSelectOptions as Choices, SessionId, SessionNotification, SessionUpdate,
    SetSessionConfigOptionRequest, TextContent,
};
use agent_client_protocol::v1::{ConnectionTo, Responder};
use agent_client_protocol::{Agent, ErrorCode, JsonRpcMessage as _, Lines, UntypedMessage};
use futures::{sink, stream};
use serde::Serialize;
use serde_json::Value;
use shed_proto::{ApprovalOption, TerminalInfo};
use tokio::io::{AsyncBufReadExt as _, AsyncRead, AsyncWrite, AsyncWriteExt as _, BufReader};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::Error;
use crate::lane::{Ask, Driver, Heard, Opened, Start, Stop, Tool};

type AcpResult<T> = Result<T, agent_client_protocol::Error>;
type Asked = Responder<RequestPermissionResponse>;

/// `run`'s `mcp_servers` as the SDK's entries; an error names the first unknown or malformed one.
///
/// # Errors
/// The entry's index and name, never its values.
pub fn mcp_servers(entries: &[Value]) -> Result<Vec<McpServer>, String> {
    let parse = |(i, e): (usize, &Value)| {
        let known = (e.get("type")).is_none_or(|t| ["stdio", "http", "sse"].iter().any(|k| t == k));
        let server = serde_json::from_value(e.clone()).ok().filter(|_| known);
        let name = e.get("name").unwrap_or(&Value::Null);
        server
            .ok_or_else(|| format!("mcp_servers[{i}] {name}: not an ACP stdio, http or sse server"))
    };
    entries.iter().enumerate().map(parse).collect()
}

pub(crate) fn redacted(servers: &[McpServer]) -> Vec<Value> {
    let mut shown = servers.to_vec();
    for server in &mut shown {
        let values: Vec<&mut String> = match server {
            McpServer::Stdio(s) => s.env.iter_mut().map(|e| &mut e.value).collect(),
            McpServer::Http(McpServerHttp { headers, .. })
            | McpServer::Sse(McpServerSse { headers, .. }) => {
                headers.iter_mut().map(|h| &mut h.value).collect()
            }
            _ => Vec::new(),
        };
        for value in values {
            "***".clone_into(value);
        }
    }
    let shown = serde_json::to_value(shown).unwrap_or_default();
    serde_json::from_value(shown).unwrap_or_default()
}

enum FromAgent {
    Update(Result<Box<SessionUpdate>, String>),
    Permission(Box<RequestPermissionRequest>, Asked),
    Prompt(u64, AcpResult<PromptResponse>),
    Config(AcpResult<Vec<SessionConfigOption>>),
}

fn update(params: &Value) -> Result<Box<SessionUpdate>, String> {
    fn at<'a>(v: Option<&'a Value>, key: &str) -> Option<&'a str> {
        v.and_then(|v| v.get(key)).and_then(Value::as_str)
    }
    let u = params.get("update");
    let content = u.and_then(|u| u.get("content"));
    let chunk = at(u, "sessionUpdate") == Some("agent_message_chunk")
        && at(content, "type") == Some("text");
    if let Some(text) = at(content, "text").filter(|_| chunk) {
        let chunk = ContentChunk::new(text.into());
        return Ok(Box::new(SessionUpdate::AgentMessageChunk(chunk)));
    }
    let n = SessionNotification::parse_message("session/update", params);
    n.map(|n| Box::new(n.update)).map_err(|e| e.to_string())
}

fn wire<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value)
        .unwrap_or_default()
        .trim_matches('"')
        .to_owned()
}

fn acp_error(why: impl Into<String>) -> agent_client_protocol::Error {
    agent_client_protocol::Error::new(ErrorCode::INTERNAL_ERROR, why)
}

async fn handshake(cx: &ConnectionTo<Agent>) -> AcpResult<AgentCapabilities> {
    let init = InitializeRequest::new(ProtocolVersion::V1)
        .client_info(Implementation::new("shed", shed_proto::VERSION));
    Ok(cx.send_request(init).block_task().await?.agent_capabilities)
}

fn answer(asked: Asked, id: Option<&str>) -> Result<(), String> {
    let outcome = match id {
        Some(id) => {
            RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(id.to_owned()))
        }
        None => RequestPermissionOutcome::Cancelled,
    };
    let answered = asked.respond(RequestPermissionResponse::new(outcome));
    answered.map_err(|e| e.to_string())
}

#[derive(Default)]
pub(crate) struct Launch {
    mcp: Vec<McpServer>,
    meta: Option<Meta>,
    model: Option<String>,
}

impl Launch {
    pub(crate) fn from_env() -> Result<Self, Error> {
        let mcp = crate::config::var("SHED_ACP_MCP").unwrap_or_default();
        let mcp = serde_json::from_str::<Vec<Value>>(&mcp).unwrap_or_default();
        let meta = crate::config::var("SHED_ACP_META").and_then(|m| serde_json::from_str(&m).ok());
        let mcp = mcp_servers(&mcp).map_err(Error::Acp)?;
        Ok(Self {
            mcp,
            meta,
            model: None,
        })
    }
}

pub(crate) struct Acp {
    cx: ConnectionTo<Agent>,
    back: UnboundedSender<FromAgent>,
    from: UnboundedReceiver<FromAgent>,
    stirred: Arc<AtomicBool>,
    pid: Option<i32>,
    id: SessionId,
    launch: Launch,
    prompts: BTreeSet<u64>,
    next: u64,
    asks: VecDeque<(String, Asked)>,
    notes: VecDeque<Heard>,
}

impl Acp {
    fn hear(&mut self, m: FromAgent) -> Option<Heard> {
        Some(match m {
            FromAgent::Update(Ok(u)) => return self.update(*u),
            FromAgent::Update(Err(why)) => Heard::Skipped(why),
            FromAgent::Permission(request, asked) => self.permission(*request, asked),
            FromAgent::Prompt(n, r) if self.prompts.remove(&n) => match r {
                Err(e) => Heard::TurnEnd {
                    stop: String::new(),
                    error: Some(e.message),
                    meta: None,
                },
                Ok(r) => Heard::TurnEnd {
                    stop: wire(&r.stop_reason),
                    error: None,
                    meta: r.meta,
                },
            },
            FromAgent::Prompt(..) => return None,
            FromAgent::Config(Ok(options)) => self.models(&options),
            FromAgent::Config(Err(e)) => Heard::Note(format!("shed: model: {e}")),
        })
    }

    fn update(&mut self, u: SessionUpdate) -> Option<Heard> {
        let text = |c: ContentBlock| match c {
            ContentBlock::Text(t) => Some(t.text),
            _ => None,
        };
        Some(match u {
            SessionUpdate::AgentMessageChunk(c) => Heard::Text(text(c.content)?),
            SessionUpdate::ToolCall(call) => {
                return self.update(SessionUpdate::ToolCallUpdate(call.into()));
            }
            SessionUpdate::ToolCallUpdate(call) => Heard::Tool(Tool {
                id: call.tool_call_id.to_string(),
                title: call.fields.title,
                kind: (call.fields.kind.map(|k| wire(&k))).filter(|k| k != "other"),
                status: call.fields.status.map(|s| wire(&s)),
                locations: (call.fields.locations.unwrap_or_default().into_iter())
                    .map(|l| l.path)
                    .collect(),
                raw_input: call.fields.raw_input,
            }),
            SessionUpdate::Plan(plan) => {
                let entries = plan.entries.iter().map(serde_json::to_value);
                Heard::Plan(entries.collect::<Result<_, _>>().unwrap_or_default())
            }
            SessionUpdate::CurrentModeUpdate(m) => Heard::Mode(m.current_mode_id.to_string()),
            SessionUpdate::ConfigOptionUpdate(c) => self.models(&c.config_options),
            SessionUpdate::UsageUpdate(u) => Heard::Usage(
                serde_json::to_value(u)
                    .and_then(serde_json::from_value)
                    .ok(),
            ),
            SessionUpdate::AvailableCommandsUpdate(u) => {
                let commands: Vec<_> = (u.available_commands.into_iter())
                    .map(|c| match c.input {
                        Some(AvailableCommandInput::Unstructured(i)) => {
                            [c.name, c.description, i.hint]
                        }
                        _ => [c.name, c.description, String::new()],
                    })
                    .collect();
                Heard::Session {
                    model: None,
                    models: None,
                    commands: Some(commands),
                }
            }
            _ => return None,
        })
    }

    fn models(&mut self, options: &[SessionConfigOption]) -> Heard {
        let model = options.iter().find_map(|o| match (&o.category, &o.kind) {
            (Some(Category::Model), Kind::Select(s)) => Some((o, s)),
            _ => None,
        });
        let (model, models) = model
            .map(|(option, select)| {
                let choices: Vec<_> = match &select.options {
                    Choices::Ungrouped(o) => o.iter().collect(),
                    Choices::Grouped(g) => g.iter().flat_map(|g| &g.options).collect(),
                    _ => Vec::new(),
                };
                self.launch.model = Some(option.id.to_string());
                let pair = |c: &&SessionConfigSelectOption| [c.value.to_string(), c.name.clone()];
                let models = choices.iter().map(pair).collect();
                (select.current_value.to_string(), models)
            })
            .unzip();
        Heard::Session {
            model,
            models,
            commands: None,
        }
    }

    fn permission(&mut self, p: RequestPermissionRequest, asked: Asked) -> Heard {
        let id = asked.raw_id().to_string();
        let mut call = serde_json::to_value(&p.tool_call).ok();
        if let Some(Value::Object(fields)) = &mut call {
            fields.remove("content");
        }
        let options = (p.options.into_iter())
            .map(|o| ApprovalOption {
                id: o.option_id.to_string(),
                name: o.name,
                kind: wire(&o.kind),
            })
            .collect();
        self.asks.push_back((id.clone(), asked));
        Heard::Ask(Ask {
            id,
            title: p.tool_call.fields.title,
            tool_call: p.tool_call.tool_call_id.to_string(),
            options,
            call,
        })
    }
}

impl Driver for Acp {
    const NAME: &'static str = "acp";

    async fn open(&mut self, cwd: &Path, start: &Start) -> Result<Opened, String> {
        let cx = self.cx.clone();
        let refused = |e| {
            if cx.is_incoming_closed() {
                "the agent closed its stdout".to_owned()
            } else {
                format!("agent refused: {e}")
            }
        };
        let agent = handshake(&cx).await.map_err(refused)?;
        let can = &agent.mcp_capabilities;
        for server in &self.launch.mcp {
            let (kind, name) = match server {
                McpServer::Http(h) if !can.http => ("http", &h.name),
                McpServer::Sse(h) if !can.sse => ("sse", &h.name),
                _ => continue,
            };
            return Err(format!(
                "MCP server {name}: the agent takes no {kind} servers (mcpCapabilities)"
            ));
        }
        let (mcp, meta) = (self.launch.mcp.clone(), self.launch.meta.clone());
        let options = match start {
            Start { resumed: true, .. } if !agent.load_session => {
                return Err("the agent cannot resume a session (no loadSession)".to_owned());
            }
            Start {
                resumed: true,
                session,
                ..
            } => {
                self.id = SessionId::new(session.clone());
                let load = LoadSessionRequest::new(self.id.clone(), cwd)
                    .mcp_servers(mcp)
                    .meta(meta);
                let loaded = cx.send_request(load).block_task().await.map_err(refused)?;
                while let Ok(m) = self.from.try_recv() {
                    let held = self.hear(m);
                    let history =
                        |h: &Heard| matches!(h, Heard::Text(_) | Heard::Tool(_) | Heard::Plan(_));
                    self.notes.extend(held.filter(|h| !history(h)));
                }
                loaded.config_options
            }
            Start { .. } => {
                let new = NewSessionRequest::new(cwd).mcp_servers(mcp).meta(meta);
                let new = cx.send_request(new).block_task().await.map_err(refused)?;
                self.id = new.session_id;
                new.config_options
            }
        };
        Ok(Opened {
            id: self.id.to_string(),
            resumable: agent.load_session,
            mcp: redacted(&self.launch.mcp),
            session: self.models(&options.unwrap_or_default()),
            pid: self.pid,
            stirred: Arc::clone(&self.stirred),
        })
    }

    async fn heard(&mut self) -> Option<Heard> {
        loop {
            if let Some(held) = self.notes.pop_front() {
                return Some(held);
            }
            let m = self.from.recv().await?;
            if let Some(heard) = self.hear(m) {
                return Some(heard);
            }
        }
    }

    fn gone(&self) -> impl Future<Output = ()> + 'static {
        let cx = self.cx.clone();
        async move { cx.incoming_closed().await }
    }

    fn prompt(&mut self, text: &str) -> Result<(), String> {
        let blocks = vec![ContentBlock::Text(TextContent::new(text))];
        let (n, back) = (self.next, self.back.clone());
        self.next += 1;
        self.cx
            .send_request(PromptRequest::new(self.id.clone(), blocks))
            .on_receiving_result(async move |r| {
                back.send(FromAgent::Prompt(n, r)).ok();
                Ok(())
            })
            .map_err(|e| e.to_string())?;
        self.prompts.insert(n);
        Ok(())
    }

    fn cancel(&mut self) -> Result<Stop, String> {
        if !self.prompts.is_empty() {
            let cancel = CancelNotification::new(self.id.clone());
            self.cx
                .send_notification(cancel)
                .map_err(|e| e.to_string())?;
        }
        while let Some((_, asked)) = self.asks.pop_front() {
            answer(asked, None)?;
        }
        Ok(Stop::No)
    }

    fn model(&mut self, model: &str) -> Result<Stop, String> {
        let Some(option) = self.launch.model.clone() else {
            let none = "shed: the agent offers no models".to_owned();
            self.notes.push_back(Heard::Note(none));
            return Ok(Stop::No);
        };
        let model = SessionConfigOptionValue::value_id(model.to_owned());
        let set = SetSessionConfigOptionRequest::new(self.id.clone(), option, model);
        let back = self.back.clone();
        self.cx
            .send_request(set)
            .on_receiving_result(async move |r| {
                back.send(FromAgent::Config(r.map(|r| r.config_options)))
                    .ok();
                Ok(())
            })
            .map(|()| Stop::No)
            .map_err(|e| e.to_string())
    }

    fn approve(&mut self, request: &str, option: Option<&str>) -> Result<(), String> {
        let at = self.asks.iter().position(|a| a.0 == request);
        match at.and_then(|at| self.asks.remove(at)) {
            Some((_, asked)) => answer(asked, option),
            None => Ok(()),
        }
    }

    fn forget(&mut self) {
        self.prompts.clear();
    }
}

pub(crate) async fn talk<T>(
    stdin: impl AsyncWrite + Send + Unpin + 'static,
    stdout: impl AsyncRead + Send + Unpin + 'static,
    stderr: impl AsyncRead + Send + Unpin + 'static,
    pid: Option<i32>,
    launch: &mut Launch,
    main: impl AsyncFnOnce(&mut Acp) -> Result<T, String>,
) -> Result<T, Error> {
    connect(stdin, stdout, stderr, async |cx, back, from, stirred| {
        let mut acp = Acp {
            cx,
            back,
            from,
            stirred,
            pid,
            id: SessionId::new(""),
            launch: std::mem::take(launch),
            prompts: BTreeSet::new(),
            next: 0,
            asks: VecDeque::new(),
            notes: VecDeque::new(),
        };
        let done = main(&mut acp).await;
        *launch = acp.launch;
        done.map_err(acp_error)
    })
    .await
}

pub(crate) async fn run<T>(
    cmd: &[String],
    cwd: &Path,
    launch: &mut Launch,
    main: impl AsyncFnOnce(&mut Acp) -> Result<T, String>,
) -> Result<(Child, T), Error> {
    let (child, stdin, stdout, stderr) = start(cmd, cwd)?;
    let pid = child.id().and_then(|p| i32::try_from(p).ok());
    let done = Box::pin(talk(stdin, stdout, stderr, pid, launch, main)).await?;
    Ok((child, done))
}

async fn connect<T>(
    stdin: impl AsyncWrite + Send + Unpin + 'static,
    stdout: impl AsyncRead + Send + Unpin + 'static,
    stderr: impl AsyncRead + Send + Unpin + 'static,
    main: impl AsyncFnOnce(
        ConnectionTo<Agent>,
        UnboundedSender<FromAgent>,
        UnboundedReceiver<FromAgent>,
        Arc<AtomicBool>,
    ) -> AcpResult<T>,
) -> Result<T, Error> {
    let stirred = Arc::new(AtomicBool::new(false));
    let lines = (BufReader::new(stdout).lines(), stirred.clone());
    let incoming = stream::unfold(lines, async |(mut lines, stirred)| {
        let line = lines.next_line().await.transpose()?;
        stirred.store(true, Ordering::Relaxed);
        Some((line, (lines, stirred)))
    });
    crate::lane::echo(stderr, stirred.clone());
    let outgoing = sink::unfold(stdin, async |mut stdin, line: String| {
        stdin.write_all(format!("{line}\n").as_bytes()).await?;
        stdin.flush().await?;
        Ok::<_, io::Error>(stdin)
    });
    let (back, from_agent) = unbounded_channel();
    let (updates, asks) = (back.clone(), back.clone());
    agent_client_protocol::Client
        .v1()
        .name("shed")
        .on_receive_notification(
            async move |n: UntypedMessage, _| {
                if SessionNotification::matches_method(n.method()) {
                    updates.send(FromAgent::Update(update(n.params()))).ok();
                }
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async move |r: RequestPermissionRequest, asked, _| {
                asks.send(FromAgent::Permission(Box::new(r), asked)).ok();
                Ok(())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(
            Lines::new(Box::pin(outgoing), Box::pin(incoming)),
            async move |cx| main(cx, back, from_agent, stirred).await,
        )
        .await
        .map_err(|e| Error::Acp(e.to_string()))
}

fn start(
    cmd: &[String],
    cwd: &Path,
) -> Result<(Child, ChildStdin, ChildStdout, ChildStderr), Error> {
    let (program, args) = cmd
        .split_first()
        .ok_or_else(|| Error::Acp("empty agent command".to_owned()))?;
    let mut child = shed_core::runner::piped(program, args, cwd, &super::launch::LANE_ONLY)
        .map_err(|e| Error::Acp(format!("{program}: {e}")))?;
    let (Some(stdin), Some(stdout), Some(stderr)) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        return Err(Error::Acp("the agent's stdio is not piped".to_owned()));
    };
    Ok((child, stdin, stdout, stderr))
}

fn oneshot<T>(
    cmd: &[String],
    cwd: &Path,
    act: impl AsyncFnOnce(ConnectionTo<Agent>, AgentCapabilities) -> AcpResult<T>,
) -> Result<T, Error> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let (child, stdin, stdout, stderr) = start(cmd, cwd)?;
        let talk = connect(stdin, stdout, stderr, async |cx, _, _, _| {
            let can = handshake(&cx).await?;
            act(cx, can).await
        });
        let done = tokio::time::timeout(Duration::from_secs(30), talk).await;
        crate::lane::stop(child);
        done.map_err(|_| Error::Acp("the agent did not answer within 30 s".to_owned()))?
    })
}

pub(crate) fn close(info: &TerminalInfo) -> Result<(), Error> {
    let id = (info.session.as_ref())
        .filter(|s| s.driver != "stream")
        .map(|s| s.id.clone());
    let Some(id) = id.filter(|id| !id.is_empty() && info.acp.is_some()) else {
        return Ok(());
    };
    let cmd = info.cmd.iter().skip_while(|a| *a != "--").skip(1);
    let cmd: Vec<String> = cmd.cloned().collect();
    oneshot(&cmd, &info.cwd, async |cx, can| {
        if can.session_capabilities.close.is_some() {
            let close = CloseSessionRequest::new(id);
            cx.send_request(close).block_task().await?;
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::time::Duration;

    use serde_json::{Value, json};
    use shed_core::proctree::{self, ProcessTable as _};
    use shed_proto::{Policy, Provider, Reported};

    use super::*;
    use crate::lane::writer::Out;
    use crate::lane::{Lane, turn_idle_timeout};
    use crate::providers::tests::fake;
    use shed_proto::Input;

    #[test]
    fn a_text_chunk_skips_the_sdk_parse_and_matches_it() {
        let params = |update: Value| json!({"sessionId": "s", "update": update});
        let text = params(json!({"sessionUpdate": "agent_message_chunk",
            "content": {"type": "text", "text": "hi"}}));
        let typed = SessionNotification::parse_message("session/update", &text).unwrap();
        assert_eq!(update(&text), Ok(Box::new(typed.update)));
        let image = params(json!({"sessionUpdate": "agent_message_chunk",
            "content": {"type": "image", "data": "", "mimeType": "image/png"}}));
        let Ok(image) = update(&image).map(|u| *u) else {
            panic!("an image chunk parses");
        };
        assert!(matches!(image, SessionUpdate::AgentMessageChunk(c)
            if matches!(c.content, ContentBlock::Image(_))));
        assert!(update(&params(json!({"sessionUpdate": "brand_new_kind"}))).is_err());
    }

    enum Beat {
        Thought,
        Stderr,
    }

    async fn fake_agent(
        stdio: tokio::io::DuplexStream,
        mut stderr: tokio::io::DuplexStream,
        beats: (Beat, u32),
        gap: Duration,
    ) {
        let (read, mut write) = tokio::io::split(stdio);
        let mut lines = BufReader::new(read).lines();
        let thought = json!({"jsonrpc": "2.0", "method": "session/update", "params": {
            "sessionId": "s", "update": {"sessionUpdate": "agent_thought_chunk",
            "content": {"type": "text", "text": "hm"}}}});
        while let Ok(Some(line)) = lines.next_line().await {
            let msg: Value = serde_json::from_str(&line).unwrap();
            let result = match msg["method"].as_str() {
                Some("initialize") => {
                    json!({"protocolVersion": 1, "agentCapabilities": {}, "authMethods": []})
                }
                Some("session/new") => json!({"sessionId": "s"}),
                Some("session/prompt") => {
                    for _ in 0..beats.1 {
                        tokio::time::sleep(gap).await;
                        match beats.0 {
                            Beat::Thought => {
                                write.write_all(format!("{thought}\n").as_bytes()).await
                            }
                            Beat::Stderr => stderr.write_all(b"still thinking\n").await,
                        }
                        .unwrap();
                    }
                    tokio::time::sleep(gap).await;
                    json!({"stopReason": "end_turn"})
                }
                _ => continue,
            };
            let reply = json!({"jsonrpc": "2.0", "id": msg["id"], "result": result});
            write
                .write_all(format!("{reply}\n").as_bytes())
                .await
                .unwrap();
        }
    }

    async fn turn(idle: Duration, beats: (Beat, u32), gap: Duration) -> (Reported, Duration) {
        let (bridge, agent) = tokio::io::duplex(1 << 16);
        let (err_agent, err_ours) = tokio::io::duplex(1 << 16);
        tokio::spawn(fake_agent(agent, err_agent, beats, gap));
        let (read, write) = tokio::io::split(bridge);
        let (out, mut outs) = unbounded_channel();
        let (typed, mut inputs) = unbounded_channel();
        typed.send(Input::Prompt("think hard".to_owned())).unwrap();
        let started = shed_core::clock::tokio_now();
        let mut launch = Launch::default();
        let session = talk(write, read, err_ours, None, &mut launch, async |acp| {
            let mut lane = Lane::new(out, Policy::Ask, Provider::default(), idle);
            lane.drive(acp, &mut inputs, Path::new("/"), Start::default())
                .await
        });
        let ended = async {
            loop {
                match outs.recv().await {
                    Some(Out::State(r)) if r.state == "done" || r.state == "error" => return r,
                    Some(_) => {}
                    None => panic!("the session ended without a state"),
                }
            }
        };
        tokio::select! {
            r = ended => (r, started.elapsed()),
            e = session => panic!("the session ended first: {:?}", e.err()),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn the_default_watchdog_lets_a_turn_think_silently_past_two_minutes() {
        let (r, took) = turn(
            turn_idle_timeout(),
            (Beat::Thought, 0),
            Duration::from_secs(121),
        )
        .await;
        assert_eq!(
            (r.state.as_str(), r.note.as_deref()),
            ("done", Some("end_turn"))
        );
        assert!(took >= Duration::from_secs(121), "took {took:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn thought_chunks_or_stderr_lines_keep_a_turn_alive_past_the_timeout() {
        for beat in [Beat::Thought, Beat::Stderr] {
            let idle = Duration::from_secs(5);
            let (r, took) = turn(idle, (beat, 30), Duration::from_secs(1)).await;
            assert_eq!(
                (r.state.as_str(), r.note.as_deref()),
                ("done", Some("end_turn"))
            );
            assert!(took >= Duration::from_secs(31), "took {took:?}");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_silent_turn_ends_as_no_response_after_the_timeout() {
        let idle = Duration::from_secs(5);
        let (r, took) = turn(idle, (Beat::Thought, 0), Duration::from_secs(30)).await;
        assert_eq!(
            (r.state.as_str(), r.note.as_deref()),
            ("error", Some("no response"))
        );
        assert!(
            took >= idle && took < idle + Duration::from_secs(1),
            "took {took:?}"
        );
    }

    enum Step {
        Wait(u64),
        Update(Value),
        Ask(&'static str),
        EndTurn(&'static str),
        Spawn(tokio::process::ChildStdin),
        Kill,
    }

    fn below(pid: i32) -> Vec<i32> {
        let procs = proctree::system().list().unwrap();
        let tree = proctree::tree(&procs, pid, &BTreeSet::new());
        tree.iter().map(|p| p.pid).filter(|&p| p != pid).collect()
    }

    async fn scripted(stdio: tokio::io::DuplexStream, mut steps: Vec<Step>, pid: i32) {
        let (read, mut write) = tokio::io::split(stdio);
        let mut lines = BufReader::new(read).lines();
        let update = |u: Value| {
            json!({"method": "session/update",
            "params": {"sessionId": "s", "update": u}})
        };
        while let Ok(Some(line)) = lines.next_line().await {
            let msg: Value = serde_json::from_str(&line).unwrap();
            let mut sent = Vec::new();
            match msg["method"].as_str() {
                Some("initialize") => sent.push(json!({"id": msg["id"], "result": {
                    "protocolVersion": 1, "agentCapabilities": {"loadSession": true},
                    "authMethods": []}})),
                Some("session/new") => {
                    sent.push(json!({"id": msg["id"], "result": {"sessionId": "s"}}));
                }
                Some("session/prompt") => {}
                _ => continue,
            }
            let prompt = msg["method"] == "session/prompt";
            let script = if prompt {
                std::mem::take(&mut steps)
            } else {
                Vec::new()
            };
            for step in script {
                match step {
                    Step::Wait(secs) => tokio::time::sleep(Duration::from_secs(secs)).await,
                    Step::Update(u) => sent.push(update(u)),
                    Step::Ask(title) => sent.push(json!({"id": 99,
                        "method": "session/request_permission", "params": {"sessionId": "s",
                        "toolCall": {"toolCallId": "t", "title": title}, "options": [
                        {"optionId": "ok", "name": "Allow", "kind": "allow_once"}]}})),
                    Step::EndTurn(text) => {
                        sent.push(update(json!({"sessionUpdate": "agent_message_chunk",
                            "content": {"type": "text", "text": text}})));
                        sent.push(json!({"id": msg["id"], "result": {"stopReason": "end_turn"}}));
                    }
                    Step::Spawn(mut input) => {
                        input.write_all(b"go\n").await.unwrap();
                        while below(pid).is_empty() {
                            tokio::time::sleep(Duration::from_millis(10)).await;
                        }
                    }
                    Step::Kill => {
                        for p in below(pid)
                            .into_iter()
                            .filter_map(rustix::process::Pid::from_raw)
                        {
                            rustix::process::kill_process(p, rustix::process::Signal::KILL)
                                .unwrap();
                        }
                    }
                }
                for mut m in sent.drain(..) {
                    m["jsonrpc"] = json!("2.0");
                    write.write_all(format!("{m}\n").as_bytes()).await.unwrap();
                }
            }
            for mut m in sent {
                m["jsonrpc"] = json!("2.0");
                write.write_all(format!("{m}\n").as_bytes()).await.unwrap();
            }
        }
    }

    type States = Vec<(String, Option<String>)>;

    async fn lane(
        steps: Vec<Step>,
        pid: Option<i32>,
        rules: Provider,
    ) -> (States, Option<Duration>) {
        let (bridge, fake) = tokio::io::duplex(1 << 16);
        let (_, err_ours) = tokio::io::duplex(1 << 16);
        tokio::spawn(scripted(fake, steps, pid.unwrap_or(0)));
        let (read, write) = tokio::io::split(bridge);
        let (out, mut outs) = unbounded_channel();
        let (typed, mut inputs) = unbounded_channel();
        typed.send(Input::Prompt("go".to_owned())).unwrap();
        let started = shed_core::clock::tokio_now();
        let mut launch = Launch::default();
        let lane = talk(write, read, err_ours, pid, &mut launch, async |acp| {
            let mut lane = Lane::new(out, Policy::Ask, rules, Duration::from_secs(5));
            lane.drive(acp, &mut inputs, Path::new("/"), Start::default())
                .await
        });
        let slept = tokio::time::timeout(Duration::from_hours(2), lane).await;
        let slept = matches!(slept, Ok(Ok(true))).then(|| started.elapsed());
        let mut states = Vec::new();
        while let Ok(o) = outs.try_recv() {
            if let Out::State(r) = o {
                states.push((r.state, r.note));
            }
        }
        (states, slept)
    }

    fn st(state: &str, note: Option<&str>) -> (String, Option<String>) {
        (state.to_owned(), note.map(str::to_owned))
    }

    #[tokio::test(start_paused = true)]
    async fn a_tool_call_in_progress_runs_past_both_timeouts_then_the_lane_is_done_and_sleeps() {
        let call = |status: &str| {
            json!({"sessionUpdate": "tool_call_update",
            "toolCallId": "t", "title": "cargo test", "status": status})
        };
        let steps = vec![
            Step::Update(call("in_progress")),
            Step::Wait(40),
            Step::EndTurn("still testing"),
            Step::Wait(700),
            Step::Update(call("completed")),
        ];
        let (states, slept) = lane(steps, None, fake("fake-acp")).await;
        let running = st("running", Some("cargo test"));
        let done = st("done", Some("end_turn"));
        assert_eq!(states, [st("working", None), running, done]);
        let slept = slept.expect("it hibernates once done");
        assert!(
            slept >= Duration::from_secs(740 + 600),
            "slept at {slept:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_child_left_running_after_the_turn_keeps_the_lane_running_until_it_exits() {
        let script = ["-c", "read _; sleep 900; :"];
        let mut agent = shed_core::runner::piped("sh", script, Path::new("/"), &[]).unwrap();
        let pid = agent.id().and_then(|p| i32::try_from(p).ok());
        let stdin = agent.stdin.take().unwrap();
        let steps = vec![
            Step::Spawn(stdin),
            Step::EndTurn("started it"),
            Step::Wait(700),
            Step::Kill,
        ];
        let (states, slept) = lane(steps, pid, fake("fake-acp")).await;
        agent.wait().await.unwrap();
        let [working, running, done] = states.as_slice() else {
            panic!("states {states:?}");
        };
        assert_eq!(
            (working, done),
            (&st("working", None), &st("done", Some("end_turn")))
        );
        assert_eq!(running.0, "running");
        let note = running.1.as_deref().unwrap_or("");
        assert!(note.starts_with("running: sleep 900 ("), "note {note:?}");
        let slept = slept.expect("it hibernates once the child is gone");
        assert!(
            slept >= Duration::from_secs(700 + 600),
            "slept at {slept:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_limit_line_is_rate_limited_only_for_an_agent_with_a_pattern() {
        let line = "fake: limit hit, back at 3pm.";
        let (states, slept) = lane(vec![Step::EndTurn(line)], None, fake("fake-acp")).await;
        assert_eq!(
            states,
            [st("working", None), st("rate-limited", Some("3pm"))]
        );
        assert_eq!(slept, None, "a rate-limited lane never hibernates");
        let (states, slept) = lane(vec![Step::EndTurn(line)], None, fake("unknown-acp")).await;
        assert_eq!(states, [st("working", None), st("done", Some("end_turn"))]);
        assert!(slept.is_some());
    }

    #[tokio::test(start_paused = true)]
    async fn a_user_file_changes_the_limit_line_a_lane_reads_without_a_rebuild() {
        let dir = crate::temp_dir("acp-provider");
        let line = "Out of tokens, back at 6pm.";
        let rules = || {
            let catalog = crate::providers::Catalog::load(&dir).0;
            catalog.by_id("claude-acp").cloned().unwrap_or_default()
        };
        let (states, _) = lane(vec![Step::EndTurn(line)], None, rules()).await;
        assert_eq!(states, [st("working", None), st("done", Some("end_turn"))]);
        let file = "limit = [\"Out of tokens\"]\nresets = [\"back at \"]\n";
        std::fs::write(dir.join("claude.toml"), file).unwrap();
        let (states, _) = lane(vec![Step::EndTurn(line)], None, rules()).await;
        assert_eq!(
            states,
            [st("working", None), st("rate-limited", Some("6pm"))]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_permission_request_waits_for_approval_and_never_times_out() {
        let call = json!({"sessionUpdate": "tool_call", "toolCallId": "t", "title": "rm notes",
            "status": "pending"});
        let (states, slept) = lane(
            vec![Step::Update(call), Step::Ask("rm notes")],
            None,
            fake("x"),
        )
        .await;
        let waiting = st("waiting-approval", Some("rm notes"));
        assert_eq!(
            states,
            [
                st("working", None),
                st("running", Some("rm notes")),
                waiting
            ]
        );
        assert_eq!(slept, None);
    }
}
