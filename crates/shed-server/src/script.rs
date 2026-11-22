mod guard;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde_json::{Value, json};
use shed_script::vm::{Limit, Limits, Run, Step};
use shed_script::{Call, Host, HostError, Program};
use tokio::sync::Semaphore;

use crate::call::{Session, json};
use crate::proto::{
    self, Access, Acting, ActingError, Caller, ErrorCode, Method, Request, RpcError, ScriptParams,
    ScriptResult,
};
use crate::{Shared, clock, links, lock};
use guard::Guard;

const CODE_LIMIT: usize = 256 << 10;
const TIMEOUT: f64 = 60.0;
const MOST_TIMEOUT: f64 = 600.0;

static SLOTS: Semaphore = Semaphore::const_new(8);

fn stepping() -> &'static Semaphore {
    static STEPPING: OnceLock<Semaphore> = OnceLock::new();
    STEPPING.get_or_init(|| {
        let workers = tokio::runtime::Handle::current().metrics().num_workers();
        Semaphore::new(workers.saturating_sub(1).max(1))
    })
}

pub(crate) async fn answer(
    registry: &Shared,
    outer: &Session,
    p: ScriptParams,
) -> Result<Value, RpcError> {
    let started = clock::tokio_now();
    let bad = |e: String| RpcError::new(ErrorCode::InvalidParams, e);
    let timeout = p.timeout.unwrap_or(TIMEOUT);
    if !(timeout > 0.0 && timeout <= MOST_TIMEOUT) {
        return Err(bad(format!(
            "timeout {timeout}: give 0 to {MOST_TIMEOUT} seconds"
        )));
    }
    let deadline = started + Duration::from_secs_f64(timeout);
    if p.code.len() > CODE_LIMIT {
        let len = p.code.len();
        return Err(bad(format!(
            "code is {len} bytes, over the limit of {CODE_LIMIT}"
        )));
    }
    let program = match Program::parse(&p.code) {
        Ok(program) => program,
        Err(error) => return reply(Err(error), None, Vec::new(), started),
    };
    if p.check {
        return reply(Ok(Value::Null), None, Vec::new(), started);
    }
    let host = ScriptHost {
        registry,
        protocol: outer.protocol,
        readonly: outer.readonly.clone(),
        links: Arc::clone(&outer.links),
        guard: Guard::for_run(&outer.links.config),
        caller: p.caller,
        acting: Mutex::new(Vec::new()),
    };
    let mut run = Run::new(&program, Limits::default(), &host);
    let outcome = tokio::time::timeout_at(deadline, drive(&mut run)).await;
    let outcome = outcome.unwrap_or_else(|_| Err(run.stopped(Limit::Deadline)));
    let acting = std::mem::take(&mut *lock(&host.acting));
    reply(outcome, Some(&mut run), acting, started)
}

async fn drive(run: &mut Run<'_, ScriptHost<'_>>) -> Result<Value, shed_script::ScriptError> {
    let _slot = SLOTS.acquire().await;
    loop {
        let step = {
            let _stepping = stepping().acquire().await;
            run.step()
        };
        if let Step::Done(outcome) = step {
            return outcome;
        }
        run.settle().await;
        tokio::task::yield_now().await;
    }
}

fn reply(
    outcome: Result<Value, shed_script::ScriptError>,
    run: Option<&mut Run<'_, ScriptHost<'_>>>,
    acting: Vec<Acting>,
    started: tokio::time::Instant,
) -> Result<Value, RpcError> {
    let (logs, calls) = run
        .as_ref()
        .map_or((Vec::new(), 0), |r| (r.logs().to_vec(), r.counts().calls));
    let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let mut answer = ScriptResult {
        logs,
        calls,
        acting,
        ms,
        ..ScriptResult::default()
    };
    match outcome {
        Ok(result) => answer.result = Some(result),
        Err(error) => answer.error = Some(wire(error)),
    }
    let len = size(&answer);
    let Some(run) = run.filter(|_| len > REPLY_LIMIT) else {
        return json(&answer);
    };
    let mut error = wire(run.stopped(Limit::Reply));
    error.message =
        format!("the reply is {len} bytes, over the limit of {REPLY_LIMIT}: return less");
    answer.result = None;
    answer.error = Some(error);
    if size(&answer) > REPLY_LIMIT {
        for entry in &mut answer.acting {
            entry.params = None;
            entry.result = entry.result.take().map(|_| Value::Bool(true));
        }
        if let Some(error) = &mut answer.error {
            error
                .message
                .push_str("; acting keeps each call's method and outcome only");
        }
    }
    json(&answer)
}

const REPLY_LIMIT: usize = 8 << 20;

fn size(answer: &ScriptResult) -> usize {
    serde_json::to_vec(answer).map_or(usize::MAX, |b| b.len().saturating_add(64))
}

fn wire(e: shed_script::ScriptError) -> proto::ScriptError {
    proto::ScriptError {
        name: e.name,
        message: e.message,
        line: e.line,
        column: e.column,
        code: e.code,
        method: e.method,
    }
}

struct ScriptHost<'r> {
    registry: &'r Shared,
    protocol: Option<u32>,
    readonly: BTreeSet<String>,
    links: Arc<links::Links>,
    guard: Guard,
    caller: Option<Caller>,
    acting: Mutex<Vec<Acting>>,
}

impl Host for ScriptHost<'_> {
    fn call(&self, method: &str, params: Value) -> Call<'_> {
        let answer = match method {
            "methods" => Some(Ok(methods())),
            "describe" => Some(describe(&params)),
            _ => None,
        };
        let Some(m) = Method::parse(method).filter(|_| answer.is_none()) else {
            let answer =
                answer.unwrap_or_else(|| Err(shed(method, ErrorCode::MethodNotFound, method)));
            return Box::pin(std::future::ready(answer));
        };
        match m.in_script() {
            Access::Refused => {
                let why = "may not be called in a script";
                Box::pin(std::future::ready(Err(HostError {
                    name: "ShedError".into(),
                    code: "NOT_IN_SCRIPT".into(),
                    message: format!("{method}: NOT_IN_SCRIPT: {why}"),
                })))
            }
            Access::Read => Box::pin(self.dispatch(m, params)),
            Access::Guarded => {
                let at = {
                    let mut acting = lock(&self.acting);
                    acting.push(Acting {
                        method: method.to_owned(),
                        params: Some(params.clone()),
                        result: None,
                        error: Some(ActingError {
                            code: "CANCELLED".into(),
                            message: "still in flight when the run stopped".into(),
                        }),
                    });
                    acting.len() - 1
                };
                Box::pin(async move {
                    let asked = self.guard.ask(m, &params, self.caller.as_ref()).await;
                    let answer = match asked {
                        Ok(()) => self.dispatch(m, params).await,
                        Err(refused) => Err(refused),
                    };
                    if let Some(entry) = lock(&self.acting).get_mut(at) {
                        (entry.result, entry.error) = match &answer {
                            Ok(result) => (Some(result.clone()), None),
                            Err(e) => (
                                None,
                                Some(ActingError {
                                    code: e.code.clone(),
                                    message: e.message.clone(),
                                }),
                            ),
                        };
                    }
                    answer
                })
            }
        }
    }
}

impl ScriptHost<'_> {
    async fn dispatch(&self, m: Method, params: Value) -> Result<Value, HostError> {
        let mut session = Session {
            protocol: self.protocol,
            readonly: self.readonly.clone(),
            links: Arc::clone(&self.links),
            ..Session::default()
        };
        let request = Request::new(0, m, params);
        let answer = Box::pin(crate::dispatch::dispatch(
            self.registry,
            &mut session,
            &request,
        ))
        .await;
        answer.map_err(|e| {
            let (code, _) = e.message.split_once(": ").unwrap_or((&e.message, ""));
            HostError {
                name: "ShedError".into(),
                code: code.to_owned(),
                message: format!("{}: {}", m.name(), e.message),
            }
        })
    }
}

fn shed(method: &str, code: ErrorCode, detail: &str) -> HostError {
    HostError {
        name: "ShedError".into(),
        code: code.name().into(),
        message: format!("{method}: {}", RpcError::new(code, detail).message),
    }
}

fn methods() -> Value {
    let list = Method::ALL
        .map(|m| json!({"name": m.name(), "summary": m.summary(), "script": m.in_script().name()}));
    Value::Array(list.into())
}

fn describe(params: &Value) -> Result<Value, HostError> {
    let name = params.as_str().unwrap_or_default();
    let doc = proto::openrpc();
    let methods = doc.get("methods").and_then(Value::as_array);
    let found = methods
        .into_iter()
        .flatten()
        .find(|m| m.get("name").and_then(Value::as_str) == Some(name));
    let Some(found) = found else {
        return Err(shed("describe", ErrorCode::MethodNotFound, name));
    };
    let mut schemas = BTreeMap::new();
    let mut todo = vec![found];
    while let Some(at) = todo.pop() {
        match at {
            Value::Object(map) => {
                let target = map.get("$ref").and_then(Value::as_str);
                let name = target.and_then(|r| r.strip_prefix("#/components/schemas/"));
                if let Some(name) = name
                    && !schemas.contains_key(name)
                    && let Some(schema) = doc.pointer(&format!("/components/schemas/{name}"))
                {
                    schemas.insert(name.to_owned(), schema.clone());
                    todo.push(schema);
                }
                todo.extend(map.values());
            }
            Value::Array(list) => todo.extend(list),
            _ => {}
        }
    }
    Ok(json!({"method": found, "schemas": schemas}))
}
