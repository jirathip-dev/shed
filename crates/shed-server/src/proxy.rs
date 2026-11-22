use std::collections::BTreeMap;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use shed_accounts::choice::{self, Choice};
use shed_accounts::keys::Keys;
use shed_accounts::proxy::Proxy;
use shed_accounts::relay::Relay;
use shed_accounts::{Store, check, store::DEFAULT};
use shed_client::Conn;
use tokio::sync::OnceCell;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::call::{io_error, json};
use crate::links::{Held, Links};
use crate::machines::Machine;
use crate::proto::{
    Account, ErrorCode, Event, EventKind, Input, KeyState, LinkState, Method, Provider, RpcError,
    RunParams, State, TerminalInfo,
};
use crate::{Shared, accounts, lock, providers};

static RELAY: OnceCell<Relay> = OnceCell::const_new();

pub(crate) fn socket(control: &Path) -> PathBuf {
    control.join("proxy.sock")
}

fn state(registry: &Shared) -> PathBuf {
    lock(registry).path.clone()
}

fn record(registry: &Shared, agent: &str) -> Option<Provider> {
    let dir = lock(registry).providers.clone();
    accounts::records_in(&dir)
        .into_iter()
        .find(|p| p.agent == agent)
}

pub(crate) fn start(
    registry: &Shared,
    links: &Arc<Links>,
    watched: UnboundedReceiver<Event>,
    listener: tokio::net::UnixListener,
) {
    let held = Arc::clone(registry);
    let proxy = Proxy::beside(&state(registry), Box::new(move |a| record(&held, a)));
    tokio::spawn(Arc::new(proxy).serve(listener));
    tokio::spawn(watch(Arc::clone(registry), Arc::clone(links), watched));
}

pub(crate) async fn relay(registry: &Shared) -> Result<crate::proto::Relay, RpcError> {
    let state = state(registry);
    let dir = state.parent().unwrap_or(Path::new("/")).to_owned();
    let relay = RELAY.get_or_try_init(|| Relay::start(&state, &dir)).await;
    let relay = relay.map_err(io_error)?;
    let (port, socket) = (relay.addr().port(), relay.next()?);
    Ok(crate::proto::Relay { port, socket })
}

fn local_only(provider: &Provider, name: &str) -> RpcError {
    let why = format!(
        "{}/{name} has no live setup token or the record no [proxy]: it runs on the main host only; \
         `shed account login {} {name} --token`",
        provider.agent, provider.agent
    );
    RpcError::new(ErrorCode::AccountLocalOnly, why)
}

fn pick(store: &Store, provider: &Provider, except: Option<&str>) -> Result<String, RpcError> {
    let signs = |a: &Account| a.name != DEFAULT && a.token.is_some_and(|t| !t.rejected);
    let all: Vec<Account> = store.ls(provider).into_iter().filter(signs).collect();
    if all.is_empty() {
        return Err(local_only(provider, "auto"));
    }
    choice::pick(&all, except, shed_core::clock::unix_now())
}

fn named(store: &Store, provider: &Provider, name: &str) -> Result<(), RpcError> {
    let account = store.account(provider, name)?;
    if account.token.is_none_or(|t| t.rejected) || provider.proxy.is_none() {
        return Err(local_only(provider, name));
    }
    let limited = |at| {
        format!(
            "{}/{name} is limited until {at} (unix seconds)",
            provider.agent
        )
    };
    let limited = account.limited_until.map(limited);
    limited.map_or(Ok(()), |why| {
        Err(RpcError::new(ErrorCode::AccountLimited, why))
    })
}

pub(crate) async fn run_on(
    registry: &Shared,
    links: &Links,
    machine: &str,
    params: Value,
) -> Result<Value, RpcError> {
    let bad = |why: String| RpcError::new(ErrorCode::InvalidParams, why);
    let mut p: RunParams = serde_json::from_value(params).map_err(|e| bad(e.to_string()))?;
    let asked = p.account.take().unwrap_or_default();
    let id = (p.acp.clone()).ok_or_else(|| bad("account needs acp".to_owned()))?;
    let catalog = providers::Catalog::load(&lock(registry).providers).0;
    let provider = catalog.by_id(&id).filter(|p| p.config_dir_var.is_some());
    let why = format!("{id} has no provider record with a config folder variable");
    let provider = provider
        .cloned()
        .ok_or_else(|| RpcError::new(ErrorCode::NoAccount, why))?;
    let Some(proxy) = provider.proxy.clone() else {
        return Err(local_only(&provider, &asked));
    };
    let store = Store::beside(&state(registry));
    let (name, auto) = match Choice::from(asked.as_str()) {
        Choice::Named(name) => (name, false),
        Choice::Auto => (pick(&store, &provider, None)?, true),
    };
    named(&store, &provider, &name)?;
    if let Some(var) = [&proxy.base_url_var, &proxy.key_var]
        .into_iter()
        .find(|v| p.env.contains_key(*v))
    {
        return Err(bad(format!(
            "env sets {var}, which a run on an account gets from shed"
        )));
    }
    let link = links.report().remove(machine);
    let Some(port) = link.and_then(|l| l.proxy) else {
        let why = format!(
            "{machine} has no forward to the token proxy: its link is down or its shed under protocol 5"
        );
        return Err(RpcError::new(ErrorCode::NoProxy, why));
    };
    let keys = Keys::beside(&state(registry));
    let (secret, key) = keys.mint(&format!("{machine}/"), (&provider.agent, &name), auto)?;
    p.env
        .insert(proxy.base_url_var, format!("http://127.0.0.1:{port}"));
    p.env.insert(proxy.key_var, secret);
    match links.call(machine, Method::Run, json(&p)?).await {
        Ok(info) => {
            let terminal = info.get("name").and_then(Value::as_str).unwrap_or_default();
            keys.name(&key.id, terminal);
            Ok(info)
        }
        Err(e) => {
            keys.revoke(&key.id).ok();
            Err(e)
        }
    }
}

pub(crate) fn removed(registry: &Shared, name: Option<&Value>) {
    if let Some(name) = name.and_then(Value::as_str) {
        Keys::beside(&state(registry)).end(name);
    }
}

pub(crate) fn sweep(state: &Path, machine: &str, calls: &mut Conn) {
    let Ok(there) = calls.call::<_, Vec<TerminalInfo>>(Method::Ls, &serde_json::json!({})) else {
        return;
    };
    let keys = Keys::beside(state);
    for k in keys.ls().into_iter().filter(|k| k.state == KeyState::Live) {
        let name = k
            .terminal
            .strip_prefix(machine)
            .and_then(|t| t.strip_prefix('/'));
        let running = |t: &TerminalInfo| Some(t.name.as_str()) == name && t.state == State::Running;
        if name.is_some_and(|n| !n.is_empty()) && !there.iter().any(running) {
            keys.end(&k.terminal);
        }
    }
}

async fn watch(registry: Shared, links: Arc<Links>, mut watched: UnboundedReceiver<Event>) {
    let mut last = BTreeMap::new();
    while let Some(e) = watched.recv().await {
        match &e.kind {
            EventKind::Exited { .. } | EventKind::Killed => {
                Keys::beside(&state(&registry)).end(&e.name);
            }
            EventKind::State(r) => {
                let was = last.insert(e.name.clone(), r.state.clone());
                if r.state == "rate-limited"
                    && was.as_deref() != Some("rate-limited")
                    && switch(&registry, &e.name, r.note.as_deref()).await
                {
                    go_on(&links, &e.name).await;
                }
            }
            _ => {}
        }
    }
}

async fn switch(registry: &Shared, terminal: &str, note: Option<&str>) -> bool {
    let keys = Keys::beside(&state(registry));
    let Some((key, true)) = keys.of(terminal) else {
        return false;
    };
    let Some(provider) = record(registry, &key.provider) else {
        return false;
    };
    let store = Store::beside(&state(registry));
    let resets = note.and_then(|n| crate::proto::when(n, shed_core::clock::unix_now()));
    check::limit(&store, &provider, &key.account, resets)
        .await
        .ok();
    let Ok(to) = pick(&store, &provider, Some(&key.account)) else {
        return false;
    };
    keys.repoint(terminal, &to);
    let kind = EventKind::Account {
        account: to,
        was: Some(key.account),
        why: "limit".to_owned(),
    };
    lock(registry).events.push(Event {
        seq: 0,
        name: terminal.to_owned(),
        labels: BTreeMap::new(),
        parent: None,
        kind,
        replay: false,
    });
    true
}

async fn go_on(links: &Links, terminal: &str) {
    let Some((machine, name)) = terminal.split_once('/') else {
        return;
    };
    let text = crate::proto::frame(&Input::Prompt(crate::lane::GO_ON.to_owned()));
    let working = serde_json::json!({"name": name, "state": "working"});
    for (method, params) in [
        (Method::State, working),
        (
            Method::Send,
            serde_json::json!({"name": name, "text": text}),
        ),
    ] {
        if links.call(machine, method, params).await.is_err() {
            return;
        }
    }
}

pub(crate) fn forward(
    held: &Held,
    machine: &Machine,
    local: &Path,
    mut relay: Option<crate::proto::Relay>,
) {
    let up = || lock(&held.link).state == LinkState::Up;
    while let Some(r) = relay.take().filter(|_| up()) {
        let spec = format!("{}:{}", r.socket.display(), local.display());
        let mut args = vec![];
        for option in [
            "BatchMode=yes",
            "ControlPath=none",
            "ExitOnForwardFailure=yes",
            "ServerAliveInterval=15",
            "StreamLocalBindMask=0177",
        ] {
            args.extend(["-o", option]);
        }
        args.extend(["-R", &spec, "--", &machine.target, "cat"]);
        let Ok((ours, theirs)) = UnixStream::pair() else {
            return;
        };
        let Ok(mut ssh) = shed_core::runner::over("ssh", args, OwnedFd::from(theirs)) else {
            return;
        };
        lock(&held.link).proxy = Some(r.port);
        while up() && matches!(ssh.try_wait(), Ok(None)) {
            shed_core::clock::sleep(Duration::from_secs(1));
        }
        ssh.kill().ok();
        ssh.wait().ok();
        drop(ours);
        lock(&held.link).proxy = None;
        shed_core::clock::sleep(crate::machines::backoff(0));
        let mut calls = lock(&held.calls);
        relay =
            (calls.as_mut()).and_then(|c| c.call(Method::ProxyRelay, &serde_json::json!({})).ok());
    }
}
