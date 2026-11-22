use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, OnceLock};

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
        let (response, changed) =
            registry::answering(respond(&registry, &mut session, &line)).await;
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
        return session.links.call(&machine, method, params).await;
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
    use std::sync::Mutex;
    use std::time::Duration;

    use tokio::io::AsyncWriteExt as _;

    use super::*;
    use crate::call::empty;
    use crate::proto::Hello;
    use crate::registry::Registry;
    use crate::{HEARTBEAT, clock, temp_dir};
    use tokio::net::unix::OwnedWriteHalf;

    async fn peer(name: &str, beat: bool) -> (tokio::task::JoinHandle<()>, OwnedWriteHalf) {
        let dir = temp_dir(name);
        let reg = Registry::load(&dir.join("state.json"), &dir.join("shed.sock"), Vec::new());
        let registry = Arc::new(Mutex::new(reg.expect("a registry")));
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
        (conn, write)
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
