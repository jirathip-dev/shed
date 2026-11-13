use std::collections::BTreeMap;
use std::io::{BufRead as _, Write as _};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::machines::{self, Machine};
use crate::proto::{ErrorCode, Event, EventsParams, Link, LinkState, Method, Request, RpcError};
use crate::{Client, Error, Shared, blocking, lock};

#[derive(Default)]
pub(crate) struct Links {
    config: PathBuf,
    control: PathBuf,
    held: Mutex<BTreeMap<String, Arc<Held>>>,
}

struct Held {
    machine: Machine,
    calls: Mutex<Option<Client>>,
    link: Mutex<Link>,
}

impl Held {
    fn set(&self, state: LinkState, error: Option<String>) {
        let mut link = lock(&self.link);
        if link.state != state {
            link.since = shed_core::clock::unix_now();
        }
        (link.state, link.error) = (state, error);
    }
}

pub(crate) fn remote(method: Method, request: &Request) -> Option<(String, Value)> {
    let field = match method {
        Method::Run | Method::Exec | Method::FileRead | Method::FileWrite => "machine",
        Method::Get
        | Method::Read
        | Method::Send
        | Method::Keys
        | Method::Kill
        | Method::Rm
        | Method::State
        | Method::Model
        | Method::Wait => "name",
        _ => return None,
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
        }
        _ => {}
    }
}

impl Links {
    pub(crate) fn new(config: PathBuf, control: PathBuf) -> Self {
        let held = Mutex::default();
        Self {
            config,
            control,
            held,
        }
    }

    pub(crate) fn sync(&self, registry: &Shared) {
        let machines = machines::load(&self.config).unwrap_or_default();
        let mut held = lock(&self.held);
        for machine in machines {
            if held.contains_key(&machine.name) {
                continue;
            }
            let link = Arc::new(Held {
                machine: machine.clone(),
                calls: Mutex::new(None),
                link: Mutex::new(Link {
                    state: LinkState::Down,
                    since: shed_core::clock::unix_now(),
                    error: None,
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
        let control = self.control.clone();
        let mut value = blocking(move || {
            if method == Method::Wait && lock(&held.link).state == LinkState::Up {
                let waiting = machines::connect(&held.machine, &control, false);
                return match waiting.and_then(|mut c| c.call(method, &params)) {
                    Err(Error::Rpc(e)) => Err(e),
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
                Err(Error::Rpc(e)) => Err(e),
                Err(e) => {
                    *calls = None;
                    Err(no_link(&e.to_string()))
                }
                Ok(value) => Ok(value),
            }
        })
        .await??;
        prefix(&mut value, machine);
        Ok(value)
    }
}

fn hold(registry: &Shared, machine: &Machine, control: &std::path::Path, held: &Held) {
    let (mut since, mut failures) = (None, 0);
    loop {
        let pair = machines::connect(machine, control, false)
            .and_then(|calls| Ok((calls, machines::connect(machine, control, false)?)));
        match pair {
            Ok((calls, mut events)) => {
                let params = EventsParams {
                    labels: BTreeMap::new(),
                    since,
                };
                let beating = events.call::<_, Value>(Method::Heartbeat, &serde_json::json!({}));
                if beating.is_ok() && events.call::<_, Value>(Method::Events, &params).is_ok() {
                    failures = 0;
                    *lock(&held.calls) = Some(calls);
                    held.set(LinkState::Up, None);
                    let (reader, writer) = events.split();
                    reader
                        .get_ref()
                        .set_read_timeout(Some(crate::WRITE_TIMEOUT))
                        .ok();
                    std::thread::spawn(move || beat(&writer));
                    let socket = reader.get_ref().try_clone();
                    for line in reader.lines().map_while(Result::ok) {
                        let Ok(e) = serde_json::from_str::<Event>(&line) else {
                            continue;
                        };
                        since = Some(e.seq);
                        let name = format!("{}/{}", machine.name, e.name);
                        let parent = e.parent.map(|p| format!("{}/{p}", machine.name));
                        lock(registry).events.push(Event { name, parent, ..e });
                    }
                    socket.and_then(|s| s.shutdown(Shutdown::Both)).ok();
                    *lock(&held.calls) = None;
                    held.set(LinkState::Reconnecting, None);
                }
            }
            Err(e) => {
                let reconnecting = lock(&held.link).state == LinkState::Reconnecting;
                let version = matches!(e, Error::MachineVersion { .. });
                if version || !reconnecting {
                    held.set(LinkState::Down, Some(e.to_string()));
                }
            }
        }
        shed_core::clock::sleep(machines::backoff(failures));
        failures += 1;
    }
}

fn beat(mut writer: &UnixStream) {
    let line = crate::notification(Method::Heartbeat, &serde_json::json!({}));
    let line = line.unwrap_or_default();
    while writer.write_all(&line).is_ok() {
        shed_core::clock::sleep(crate::HEARTBEAT);
    }
}
