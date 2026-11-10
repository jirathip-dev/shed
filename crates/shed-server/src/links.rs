//! The held links: one per configured machine, two `ssh <target> shed connect` streams (calls, and
//! events), kept up with [`machines::backoff`]. Their events join this server's stream as
//! `<machine>/<name>`; requests naming `<machine>/<name>` go over the calls stream.

use std::collections::BTreeMap;
use std::io::BufRead as _;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::machines::{self, Machine};
use crate::proto::{ErrorCode, Event, EventsParams, Link, LinkState, Method, Request, RpcError};
use crate::{Client, Error, Shared, blocking, lock};

/// Every machine's link, by name; the config file and the ssh control dir they come from.
#[derive(Default)]
pub(crate) struct Links {
    config: PathBuf,
    control: PathBuf,
    held: Mutex<BTreeMap<String, Arc<Held>>>,
}

/// One link: its machine, its calls stream while up, and its state.
struct Held {
    machine: Machine,
    calls: Mutex<Option<Client>>,
    link: Mutex<Link>,
}

impl Held {
    /// Moves to `state` (keeping `since` when it stays), with `error`.
    fn set(&self, state: LinkState, error: Option<String>) {
        let mut link = lock(&self.link);
        if link.state != state {
            link.since = crate::stats::unix_now();
        }
        (link.state, link.error) = (state, error);
    }
}

/// The machine and the request for it when `request` names `<machine>/<name>` (or `run` names a
/// `machine`), its params rewritten for the server there.
pub(crate) fn remote(method: Method, request: &Request) -> Option<(String, Value)> {
    // Checked before any clone: every request passes here, ACP's `emit` once per update.
    let field = match method {
        Method::Run => "machine",
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
    let (machine, name) = match method {
        Method::Run => (value, None),
        _ => value.split_once('/').map(|(m, n)| (m, Some(n)))?,
    };
    let mut params = request.params.clone();
    match name {
        Some(name) => params["name"] = name.into(),
        None => _ = params.as_object_mut()?.remove(field),
    }
    Some((machine.to_owned(), params))
}

/// `name` (of a result, its `terminal`, or each in a list) as `<machine>/<name>`.
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
    /// Links for the machines in `config`, through ssh's shared connections in `control`.
    pub(crate) fn new(config: PathBuf, control: PathBuf) -> Self {
        let held = Mutex::default();
        Self {
            config,
            control,
            held,
        }
    }

    /// Starts a link for each machine in the config file that has none (best effort: a bad file
    /// links nothing new).
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
                    since: crate::stats::unix_now(),
                    error: None,
                }),
            });
            held.insert(machine.name.clone(), Arc::clone(&link));
            let (registry, control) = (Arc::clone(registry), self.control.clone());
            std::thread::spawn(move || hold(&registry, &machine, &control, &link));
        }
    }

    /// `machine.links`.
    pub(crate) fn report(&self) -> BTreeMap<String, Link> {
        let held = lock(&self.held);
        let link = |h: &Held| lock(&h.link).clone();
        held.iter().map(|(m, h)| (m.clone(), link(h))).collect()
    }

    /// The machines whose links are up.
    pub(crate) fn up(&self) -> Vec<String> {
        let report = self.report().into_iter();
        let up = report.filter(|(_, l)| l.state == LinkState::Up);
        up.map(|(m, _)| m).collect()
    }

    /// `method` with `params` on `machine`'s server, its names prefixed `<machine>/`. A `wait` holds a
    /// stream of its own over the link's shared ssh connection, leaving the calls stream free.
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
                    // Already `SHED_VERSION: …`, naming both versions.
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
                    // A broken stream: the events stream sees it too, and reconnects.
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

/// Keeps `machine`'s link up for good: connects both streams, streams its events into `registry`
/// (resuming after the last `seq` seen), and after a drop or failure retries on the backoff schedule.
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
                if events.call::<_, Value>(Method::Events, &params).is_ok() {
                    failures = 0;
                    *lock(&held.calls) = Some(calls);
                    held.set(LinkState::Up, None);
                    for line in events.split().0.lines().map_while(Result::ok) {
                        let Ok(e) = serde_json::from_str::<Event>(&line) else {
                            continue;
                        };
                        since = Some(e.seq);
                        let name = format!("{}/{}", machine.name, e.name);
                        let mut reg = lock(registry);
                        reg.events.emit_as(&name, &e.labels, e.kind, e.replay);
                    }
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
        std::thread::sleep(machines::backoff(failures));
        failures += 1;
    }
}
