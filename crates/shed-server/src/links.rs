use std::collections::BTreeMap;
use std::io::{BufRead as _, Write as _};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::Value;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::call::blocking;
use crate::machines::{self, Machine};
use crate::proto::{
    ErrorCode, Event, EventKind, EventsParams, Link, LinkState, Method, Request, Route, RpcError,
};
use crate::{Error, Shared, lock};
use shed_client::Conn;

pub(crate) struct Links {
    pub(crate) config: PathBuf,
    control: PathBuf,
    held: Mutex<BTreeMap<String, Arc<Held>>>,
    watched: UnboundedSender<Event>,
}

pub(crate) struct Held {
    machine: Machine,
    watched: UnboundedSender<Event>,
    pub(crate) calls: Mutex<Option<Conn>>,
    pub(crate) link: Mutex<Link>,
    lines: Mutex<Vec<UnixStream>>,
}

impl Held {
    fn set(&self, state: LinkState, error: Option<String>) {
        let mut link = lock(&self.link);
        let now = shed_core::clock::unix_now();
        if link.state != state {
            link.since = now;
        }
        let up = state == LinkState::Up;
        if up || link.state == LinkState::Up {
            link.down_since = (!up).then_some(now);
        }
        (link.state, link.error) = (state, error);
    }

    fn cut(&self) {
        for line in lock(&self.lines).drain(..) {
            line.shutdown(Shutdown::Both).ok();
        }
    }

    fn learn(&self, calls: &mut Conn) {
        let [os, arch, home] = machines::facts(calls);
        let mut link = lock(&self.link);
        (link.hello, link.os, link.arch) = (Some(calls.server().clone()), os, arch);
        link.home = home.map(PathBuf::from);
    }
}

pub(crate) fn remote(method: Method, request: &Request) -> Option<(String, Value)> {
    let field = match method.route()? {
        Route::Machine => "machine",
        Route::Name => "name",
    };
    let value = request.params.get(field)?.as_str()?;
    let (machine, name) = match field {
        "machine" => (value, None),
        _ => value.split_once('/').map(|(m, n)| (m, Some(n)))?,
    };
    let mut params = request.params.clone();
    match name {
        Some(name) => _ = params.as_object_mut()?.insert("name".into(), name.into()),
        None => _ = params.as_object_mut()?.remove(field),
    }
    Some((machine.to_owned(), params))
}

fn prefix(value: &mut Value, machine: &str) {
    match value {
        Value::Array(list) => list.iter_mut().for_each(|v| prefix(v, machine)),
        Value::Object(o) => {
            if let Some(Value::String(name)) = o.get_mut("name") {
                *name = format!("{machine}/{name}");
            }
            if let Some(terminal) = o.get_mut("terminal") {
                prefix(terminal, machine);
            }
            for key in ["top_cpu", "top_mem"] {
                let procs = o.get_mut(key).and_then(Value::as_array_mut);
                for p in procs.into_iter().flatten() {
                    if let Some(Value::String(name)) = p.get_mut("terminal") {
                        *name = format!("{machine}/{name}");
                    }
                }
            }
        }
        _ => {}
    }
}

impl Default for Links {
    fn default() -> Self {
        Self::new(PathBuf::new(), PathBuf::new()).0
    }
}

impl Links {
    pub(crate) fn new(config: PathBuf, control: PathBuf) -> (Self, UnboundedReceiver<Event>) {
        let (watched, rx) = unbounded_channel();
        let held = Mutex::default();
        let links = Self {
            config,
            control,
            held,
            watched,
        };
        (links, rx)
    }

    pub(crate) fn sync(&self, registry: &Shared) {
        let Ok(machines) = machines::load(&self.config) else {
            return;
        };
        let mut held = lock(&self.held);
        held.retain(|_, h| machines.contains(&h.machine));
        for machine in machines {
            if held.contains_key(&machine.name) {
                continue;
            }
            let link = Arc::new(Held {
                machine: machine.clone(),
                watched: self.watched.clone(),
                calls: Mutex::new(None),
                lines: Mutex::default(),
                link: Mutex::new(Link {
                    since: shed_core::clock::unix_now(),
                    down_since: Some(shed_core::clock::unix_now()),
                    ..Link::default()
                }),
            });
            held.insert(machine.name.clone(), Arc::clone(&link));
            let (registry, control) = (Arc::clone(registry), self.control.clone());
            std::thread::spawn(move || hold(&registry, &machine, &control, &link));
        }
    }

    pub(crate) fn report(&self) -> BTreeMap<String, Link> {
        let held = lock(&self.held);
        let link = |h: &Held| lock(&h.link).clone();
        held.iter().map(|(m, h)| (m.clone(), link(h))).collect()
    }

    pub(crate) fn up(&self) -> Vec<String> {
        let report = self.report().into_iter();
        let up = report.filter(|(_, l)| l.state == LinkState::Up);
        up.map(|(m, _)| m).collect()
    }

    pub(crate) async fn call(
        &self,
        machine: &str,
        method: Method,
        params: Value,
    ) -> Result<Value, RpcError> {
        let held = lock(&self.held).get(machine).cloned();
        let name = machine.to_owned();
        let no_link = move |why: &str| RpcError::new(ErrorCode::NoLink, format!("{name}: {why}"));
        let Some(held) = held else {
            return Err(no_link("no such machine; see `shed machine ls`"));
        };
        let (own, _ends) = machines::alone(&held.machine, &self.control);
        let alone = matches!(method, Method::Wait | Method::Script);
        let (bound, cut) = (machines::bound(method, &params), Arc::clone(&held));
        let answer = blocking(move || {
            if alone && lock(&held.link).state == LinkState::Up {
                return match own(method, &params) {
                    Err(Error::Client(shed_client::Error::Rpc(e))) => Err(e),
                    Err(e) => Err(no_link(&e.to_string())),
                    Ok(value) => Ok(value),
                };
            }
            let mut calls = lock(&held.calls);
            let Some(client) = calls.as_mut() else {
                let link = lock(&held.link).clone();
                let state = format!("link {:?}", link.state).to_lowercase();
                return Err(match link.error {
                    Some(message) if message.starts_with(ErrorCode::Version.name()) => RpcError {
                        code: ErrorCode::Version.code(),
                        message,
                    },
                    error => no_link(&error.unwrap_or(state)),
                });
            };
            match client.call(method, &params) {
                Err(shed_client::Error::Rpc(e)) => Err(e),
                Err(e) => {
                    *calls = None;
                    Err(no_link(&e.to_string()))
                }
                Ok(value) => Ok(value),
            }
        });
        let Some(bound) = bound else {
            let mut value = answer.await??;
            prefix(&mut value, machine);
            return Ok(value);
        };
        let Ok(answer) = tokio::time::timeout(bound, answer).await else {
            cut.cut();
            let why = format!(
                "{machine}: no answer in {} s; the link is cut",
                bound.as_secs()
            );
            return Err(RpcError::new(ErrorCode::NoLink, why));
        };
        let mut value = answer??;
        prefix(&mut value, machine);
        Ok(value)
    }
}

fn hold(registry: &Shared, machine: &Machine, control: &Path, held: &Arc<Held>) {
    let (mut since, mut failures) = (None, 0);
    while Arc::strong_count(held) > 1 {
        let pair = machines::line(machine, control, false)
            .and_then(|calls| Ok((calls, machines::line(machine, control, false)?)));
        match pair {
            Ok(((mut calls, calls_line), (mut events, events_line))) => {
                *lock(&held.lines) = vec![calls_line, events_line];
                let params = EventsParams {
                    labels: BTreeMap::new(),
                    since,
                };
                let beating = events.call::<_, Value>(Method::Heartbeat, &serde_json::json!({}));
                if beating.is_ok() && events.call::<_, Value>(Method::Events, &params).is_ok() {
                    failures = 0;
                    held.learn(&mut calls);
                    crate::proxy::sweep(&lock(registry).path.clone(), &machine.name, &mut calls);
                    let relay = calls.call(Method::ProxyRelay, &serde_json::json!({})).ok();
                    *lock(&held.calls) = Some(calls);
                    held.set(LinkState::Up, None);
                    let (to, at) = (Arc::clone(held), control.to_owned());
                    let there = machine.clone();
                    std::thread::spawn(move || crate::proxy::forward(&to, &there, &at, relay));
                    let (reader, writer) = events.split();
                    reader
                        .get_ref()
                        .set_read_timeout(Some(crate::WRITE_TIMEOUT))
                        .ok();
                    std::thread::spawn(move || beat(&writer));
                    for line in reader.lines().map_while(Result::ok) {
                        if Arc::strong_count(held) == 1 {
                            break;
                        }
                        let Ok(e) = serde_json::from_str::<Event>(&line) else {
                            continue;
                        };
                        since = Some(e.seq);
                        let name = format!("{}/{}", machine.name, e.name);
                        let parent = e.parent.map(|p| format!("{}/{p}", machine.name));
                        let e = Event { name, parent, ..e };
                        if matches!(
                            e.kind,
                            EventKind::Exited { .. } | EventKind::Killed | EventKind::State(_)
                        ) {
                            held.watched.send(e.clone()).ok();
                        }
                        lock(registry).events.push(e);
                    }
                    held.cut();
                    *lock(&held.calls) = None;
                    held.set(LinkState::Reconnecting, None);
                }
                held.cut();
            }
            Err(e) => {
                let reconnecting = lock(&held.link).state == LinkState::Reconnecting;
                let key = matches!(e, Error::KeyChanged { .. });
                if key || matches!(e, Error::MachineVersion { .. }) || !reconnecting {
                    held.set(LinkState::Down, Some(e.to_string()));
                }
                if key {
                    return;
                }
            }
        }
        shed_core::clock::sleep(machines::backoff(failures));
        failures += 1;
    }
}

fn beat(mut writer: &UnixStream) {
    let line = shed_client::notification(Method::Heartbeat, &serde_json::json!({}));
    let line = line.unwrap_or_default();
    while writer.write_all(&line).is_ok() {
        shed_core::clock::sleep(crate::HEARTBEAT);
    }
}
