use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use serde_json::Value;
use tokio::io::AsyncBufReadExt as _;

use crate::call::{Call, Reply, Session};
use crate::proto::{self, ErrorCode, Method, Request, Response, RpcError};
use crate::{Shared, WRITE_TIMEOUT, handover, links, lock, registry, send};

fn readonly<'a>(method: Method, request: &'a Request, session: &'a Session) -> Option<&'a str> {
    let mut named = ["terminal", "name"].map(|k| request.params.get(k).and_then(Value::as_str));
    if named == [None, None] {
        named[0] = session.watching.as_deref();
    }
    (named.into_iter().flatten()).find(|n| method.input() && session.readonly.contains(*n))
}

pub(crate) async fn connection(
    registry: Shared,
    held: Arc<links::Links>,
    stream: tokio::net::UnixStream,
) {
    let mut session = Session {
        links: held,
        ..Session::default()
    };
    let (read, out) = stream.into_split();
    let (mut out, mut streaming) = (Some(out), tokio::task::JoinSet::new());
    let mut lines = tokio::io::BufReader::new(read).lines();
    loop {
        let read = async {
            if session.beats {
                tokio::time::timeout(WRITE_TIMEOUT, lines.next_line()).await
            } else {
                Ok(lines.next_line().await)
            }
        };
        let next = tokio::select! {
            next = read => next,
            Some(_) = streaming.join_next() => return,
        };
        let Ok(Ok(Some(line))) = next else {
            return;
        };
        if line.trim().is_empty() {
            continue;
        }
        let answering = registry::answering(respond(&registry, &mut session, &line));
        let (response, changed) = if scripted(&line) {
            tokio::select! {
                answered = answering => answered,
                () = gone(out.as_ref()) => return,
            }
        } else {
            answering.await
        };
        let flushed = lock(&registry).saver.flushed_to(changed);
        flushed.await;
        if let (Some(response), Some(out)) = (response, &mut out) {
            let mut bytes = serde_json::to_vec(&response).unwrap_or_default();
            bytes.push(b'\n');
            if !send(out, &bytes).await {
                return;
            }
        }
        session.busy = None;
        if let Some(stream) = session.stream.take()
            && let Some(out) = out.take()
        {
            streaming.spawn(stream.run(out, session.beats));
        }
    }
}

fn scripted(line: &str) -> bool {
    line.contains("\"script\"")
        && serde_json::from_str::<Request>(line).is_ok_and(|r| r.method == Method::Script.name())
}

async fn gone(out: Option<&tokio::net::unix::OwnedWriteHalf>) {
    let Some(out) = out else {
        return std::future::pending().await;
    };
    loop {
        let ready = out.ready(tokio::io::Interest::WRITABLE).await;
        if ready.map_or(true, tokio::io::Ready::is_write_closed) {
            return;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

async fn respond(registry: &Shared, session: &mut Session, line: &str) -> Option<Response> {
    let request = match serde_json::from_str::<Request>(line).or_else(|_| invalid(line)) {
        Ok(r) if r.jsonrpc == proto::JSONRPC => r,
        Ok(r) => {
            let e = RpcError::new(ErrorCode::InvalidRequest, "jsonrpc must be \"2.0\"");
            return Some(Response::err(r.id.unwrap_or_default(), e));
        }
        Err((id, e)) => return Some(Response::err(id, e)),
    };
    session.busy = handover::enter(&request.method).await;
    let result = dispatch(registry, session, &request).await;
    let id = request.id?;
    Some(match result {
        Ok(v) => Response::ok(id, v),
        Err(e) => Response::err(id, e),
    })
}

fn invalid(line: &str) -> Result<Request, (Value, RpcError)> {
    let parse = |e| (Value::Null, RpcError::new(ErrorCode::Parse, e));
    let value: Value = serde_json::from_str(line).map_err(parse)?;
    let id = value.get("id").cloned().unwrap_or(Value::Null);
    serde_json::from_value(value).map_err(|e| (id, RpcError::new(ErrorCode::InvalidRequest, e)))
}

const STRICT_PROTOCOL: u32 = 3;

fn undocumented(method: Method, params: &Value) -> Result<(), RpcError> {
    static FIELDS: OnceLock<BTreeMap<String, BTreeSet<String>>> = OnceLock::new();
    let fields = FIELDS.get_or_init(|| {
        let doc = proto::openrpc();
        let methods = doc.get("methods").and_then(Value::as_array);
        let fields = |m: &Value| {
            let mut names = BTreeSet::new();
            let params = m
                .get("params")
                .and_then(Value::as_array)
                .into_iter()
                .flatten();
            names.extend(params.filter_map(|p| p.get("name")?.as_str().map(str::to_owned)));
            m.get("x-params")
                .inspect(|s| documented(s, &doc, &mut names));
            let name = m.get("name").and_then(Value::as_str).unwrap_or_default();
            (name.to_owned(), names)
        };
        methods.into_iter().flatten().map(fields).collect()
    });
    let Some(known) = fields.get(method.name()) else {
        return Ok(());
    };
    let mut given = params.as_object().into_iter().flatten();
    let Some((field, _)) = given.find(|(k, _)| !known.contains(*k)) else {
        return Ok(());
    };
    let field = format!("unknown field {}.{field}", method.name());
    Err(RpcError::new(ErrorCode::InvalidParams, field))
}

fn documented(schema: &Value, doc: &Value, names: &mut BTreeSet<String>) {
    let at = schema.get("$ref").and_then(Value::as_str);
    if let Some(target) = at.and_then(|r| doc.pointer(r.trim_start_matches('#'))) {
        documented(target, doc, names);
    }
    let properties = schema.get("properties").and_then(Value::as_object);
    names.extend(properties.into_iter().flatten().map(|(k, _)| k.clone()));
    for branches in ["oneOf", "anyOf", "allOf"].map(|k| schema.get(k).and_then(Value::as_array)) {
        for branch in branches.into_iter().flatten() {
            documented(branch, doc, names);
        }
    }
}

pub(crate) async fn dispatch(registry: &Shared, session: &mut Session, request: &Request) -> Reply {
    let method = Method::parse(&request.method)
        .ok_or_else(|| RpcError::new(ErrorCode::MethodNotFound, &request.method))?;
    if session.protocol.is_none() && method.needs_hello() {
        return Err(RpcError::new(
            ErrorCode::NoHello,
            "send hello with your version first",
        ));
    }
    if let Some(name) = readonly(method, request, session) {
        return Err(RpcError::new(ErrorCode::ReadOnly, name));
    }
    if session.protocol >= Some(STRICT_PROTOCOL) {
        undocumented(method, &request.params)?;
    }
    if let Some((machine, params)) = links::remote(method, request) {
        if method == Method::Run && params.get("account").is_some() {
            return crate::proxy::run_on(registry, &session.links, &machine, params).await;
        }
        let value = session.links.call(&machine, method, params).await?;
        if method == Method::Rm {
            crate::proxy::removed(registry, request.params.get("name"));
        }
        return Ok(value);
    }
    let mut cx = Call {
        method,
        request,
        registry,
        session,
    };
    methods::call(&mut cx).await
}

include!(concat!(env!("OUT_DIR"), "/methods.rs"));

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Mutex;
    use std::time::Duration;

    use tokio::io::AsyncWriteExt as _;

    use super::*;
    use crate::call::empty;
    use crate::proto::Hello;
    use crate::registry::Registry;
    use crate::{HEARTBEAT, clock, temp_dir};
    use tokio::net::unix::OwnedWriteHalf;

    type Lines = tokio::io::Lines<tokio::io::BufReader<tokio::net::unix::OwnedReadHalf>>;

    fn registry(name: &str) -> Registry {
        let dir = temp_dir(name);
        let reg = Registry::load(&dir.join("state.json"), &dir.join("shed.sock"), Vec::new());
        reg.expect("a registry")
    }

    async fn peer(name: &str, beat: bool) -> (tokio::task::JoinHandle<()>, OwnedWriteHalf) {
        let (conn, _, write) = served(Arc::new(Mutex::new(registry(name))), beat).await;
        (conn, write)
    }

    async fn served(
        registry: Shared,
        beat: bool,
    ) -> (tokio::task::JoinHandle<()>, Lines, OwnedWriteHalf) {
        let (server, client) = tokio::net::UnixStream::pair().expect("pair");
        let conn = tokio::spawn(connection(registry, Arc::default(), server));
        let (read, mut write) = client.into_split();
        let hello = Request::new(
            1,
            Method::Hello,
            serde_json::to_value(Hello::ours()).expect("hello"),
        );
        let mut line = serde_json::to_vec(&hello).expect("json");
        line.push(b'\n');
        write.write_all(&line).await.expect("hello sent");
        let mut lines = tokio::io::BufReader::new(read).lines();
        let answer = lines
            .next_line()
            .await
            .expect("read")
            .expect("a hello back");
        assert!(answer.contains(r#""result""#), "{answer}");
        if beat {
            let heartbeat = shed_client::notification(Method::Heartbeat, &empty()).expect("json");
            write.write_all(&heartbeat).await.expect("heartbeat sent");
        }
        (conn, lines, write)
    }

    async fn call(peer: &mut (Lines, OwnedWriteHalf), method: Method, params: Value) -> Value {
        let mut line = serde_json::to_vec(&Request::new(2, method, params)).expect("json");
        line.push(b'\n');
        peer.1.write_all(&line).await.expect("sent");
        next(&mut peer.0).await
    }

    async fn next(lines: &mut Lines) -> Value {
        let line = lines.next_line().await.expect("read").expect("a line");
        assert!(!line.contains(TOKEN), "{line}");
        serde_json::from_str(&line).expect("json")
    }

    const TOKEN: &str = "sk-ant-oat01-abc123def456ghi789";

    #[tokio::test]
    async fn a_redacted_terminal_shows_no_token_on_any_read_path() {
        let mut reg = registry("redacted");
        let (head, tail) = TOKEN.split_at(12);
        let cmd = [
            "sh",
            "-c",
            "printf 'paste %s%s here\\n' \"$0\" \"$1\"; exec sleep 60",
            head,
            tail,
        ];
        let run = serde_json::json!({"name": "login", "cwd": "/", "cmd": cmd});
        reg.run(serde_json::from_value(run).expect("run"))
            .expect("run");
        reg.mark_redacted("login");
        let reg = Arc::new(Mutex::new(reg));
        let open = || async {
            let (_, lines, write) = served(Arc::clone(&reg), false).await;
            (lines, write)
        };
        let mut peer = open().await;
        let wait = |output: &str, timeout: u64| serde_json::json!({"name": "login", "output": output, "timeout": timeout});
        let shown = call(&mut peer, Method::Wait, wait("paste .redacted. here", 10)).await;
        assert_eq!(shown["result"]["outcome"], "matched", "{shown}");
        let raw = call(&mut peer, Method::Wait, wait("abc123", 0)).await;
        assert_eq!(raw["result"]["outcome"], "timeout", "{raw}");
        let read = call(
            &mut peer,
            Method::Read,
            serde_json::json!({"name": "login"}),
        )
        .await;
        let text = read["result"]["text"].as_str().unwrap_or_default();
        assert!(text.contains("paste [redacted] here"), "{read}");
        let attach = call(
            &mut peer,
            Method::Attach,
            serde_json::json!({"name": "login"}),
        )
        .await;
        assert_eq!(attach["error"]["code"], -32006, "{attach}");
        let mut watch = open().await;
        let at = serde_json::json!({"terminal": "login"});
        call(&mut watch, Method::ScreenWatch, at).await;
        let screen = next(&mut watch.0).await.to_string();
        assert!(screen.contains("paste [redacted] here"), "{screen}");
        let mut events = open().await;
        call(&mut events, Method::Events, serde_json::json!({})).await;
        let note = serde_json::json!({"name": "login", "state": "blocked", "note": TOKEN});
        call(&mut peer, Method::State, note).await;
        let event = next(&mut events.0).await;
        assert_eq!(event["note"], "[redacted]", "{event}");
        lock(&reg)
            .live("login")
            .map(|t| t.kill(Duration::ZERO))
            .ok();
    }

    const FAKE: &str = r#"id = "fake-acp"
agent = "fake"
login = "printf 'paste %s%s here\\n' sk-ant-oat01-abc123 def456ghi789; read x; touch \"$FAKE_HOME/in\""
status = "test -f \"$FAKE_HOME/in\" && echo signed in"
logout = "rm -f \"$FAKE_HOME/in\""
config_dir_var = "FAKE_HOME"
setup_files = ["settings.json"]
"#;

    #[tokio::test]
    async fn the_account_lifecycle_never_shows_its_token() {
        let reg = registry("accounts");
        let dir = reg.path.parent().map(Path::to_path_buf).unwrap_or_default();
        std::fs::create_dir_all(dir.join("providers")).expect("mkdir");
        std::fs::write(dir.join("providers/fake.toml"), FAKE).expect("record");
        let given = dir.join("given.token");
        std::fs::write(&given, TOKEN).expect("token");
        std::fs::set_permissions(&given, std::os::unix::fs::PermissionsExt::from_mode(0o600))
            .expect("chmod");
        let setup = dir.join("setup");
        std::fs::create_dir_all(&setup).expect("mkdir");
        std::fs::write(setup.join("settings.json"), "{}").expect("settings");
        let reg = Arc::new(Mutex::new(reg));
        tokio::spawn(registry::keep_saved(Arc::clone(&reg)));
        let (_, lines, write) = served(reg, false).await;
        let mut peer = (lines, write);
        let work = serde_json::json!({"provider": "fake", "name": "work"});
        let add = serde_json::json!({"provider": "fake", "name": "work", "token_file": given});
        let added = call(&mut peer, Method::AccountAdd, add).await;
        assert_eq!(added["result"]["state"], "ready", "{added}");
        assert!(added["result"]["token"]["saved_at"].is_u64(), "{added}");
        let adopt = serde_json::json!({"provider": "fake", "name": "mine", "folder": setup});
        let refused = call(&mut peer, Method::AccountAdd, adopt).await;
        assert_eq!(refused["error"]["code"], -32013, "{refused}");
        let login = call(&mut peer, Method::AccountLogin, work.clone()).await;
        assert_eq!(login["result"]["terminal"], "login-fake-work", "{login}");
        let again = call(&mut peer, Method::AccountLogin, work.clone()).await;
        assert_eq!(again, login);
        let wait = serde_json::json!({"name": "login-fake-work", "output": "paste .redacted. here", "timeout": 10});
        let shown = call(&mut peer, Method::Wait, wait).await;
        assert_eq!(shown["result"]["outcome"], "matched", "{shown}");
        let attach = serde_json::json!({"name": "login-fake-work"});
        let attach = call(&mut peer, Method::Attach, attach).await;
        assert_eq!(attach["error"]["code"], -32006, "{attach}");
        let busy = call(&mut peer, Method::AccountRm, work.clone()).await;
        assert_eq!(busy["error"]["code"], -32004, "{busy}");
        let ls = serde_json::json!({"provider": "fake"});
        let listed = call(&mut peer, Method::AccountLs, ls.clone()).await;
        assert_eq!(listed["result"][1]["login"], "login-fake-work", "{listed}");
        let enter = serde_json::json!({"name": "login-fake-work", "text": ""});
        call(&mut peer, Method::Send, enter).await;
        let mut checked = Value::Null;
        for _ in 0..500 {
            checked = call(&mut peer, Method::AccountLs, ls.clone()).await;
            if checked["result"][1]["line"] == "signed in" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(checked["result"][0]["name"], "default", "{checked}");
        assert_eq!(checked["result"][1]["line"], "signed in", "{checked}");
        let status = call(&mut peer, Method::AccountStatus, work.clone()).await;
        assert_eq!(status["result"]["state"], "ready", "{status}");
        let gone = call(&mut peer, Method::AccountRm, work.clone()).await;
        assert!(gone["result"].is_object(), "{gone}");
        assert!(!dir.join("accounts/fake/work").exists());
        assert!(!dir.join("accounts/fake/work.token").exists());
        let none = call(&mut peer, Method::AccountStatus, work).await;
        assert_eq!(none["error"]["code"], -32012, "{none}");
    }

    #[tokio::test(start_paused = true)]
    async fn a_silent_peer_is_dropped_after_two_missed_heartbeats() {
        let (served, _write) = peer("silent", true).await;
        let start = clock::tokio_now();
        tokio::time::sleep((HEARTBEAT * 2).saturating_sub(Duration::from_millis(1))).await;
        assert!(!served.is_finished(), "kept through the second beat");
        served.await.expect("the connection ends");
        assert_eq!(start.elapsed(), HEARTBEAT * 2);
    }

    #[tokio::test(start_paused = true)]
    async fn a_beating_peer_is_kept_and_one_that_never_beat_is_never_timed_out() {
        let (beating, mut write) = peer("beating", true).await;
        let (quiet, _write) = peer("quiet", false).await;
        let heartbeat = shed_client::notification(Method::Heartbeat, &empty()).expect("json");
        for _ in 0..8 {
            tokio::time::sleep(HEARTBEAT).await;
            write.write_all(&heartbeat).await.expect("heartbeat sent");
        }
        assert!(!beating.is_finished() && !quiet.is_finished());
    }
}
