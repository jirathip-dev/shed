use std::collections::{BTreeMap, BTreeSet};
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use shed_accounts::{Store, Token, check, choice, store};
use shed_core::{Wake, clock};

use crate::call::{Call, blocking};
use crate::proto::{
    Account, AccountAnswer, AccountLogin, AcpUsage, ErrorCode, Login, LoginAnswer, Money, Provider,
    RemoteLogin, RpcError, RunParams, SendParams, State, TerminalInfo, Tokens,
};
use crate::registry::Registry;
use crate::{Shared, lock, providers};

const LOGIN: Duration = Duration::from_mins(10);
const LINK: Duration = Duration::from_secs(30);
const CALLBACK: Duration = Duration::from_secs(15);
const BEAT: Duration = Duration::from_mins(1);

static PROBING: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

fn money(cost: Option<&Value>) -> Option<Money> {
    let cost = cost?;
    let currency = cost["currency"].as_str()?.to_owned();
    Some(Money {
        amount: cost["amount"].as_f64()?,
        currency,
    })
}

fn tokens_since(was: Tokens, now: Tokens) -> Tokens {
    let less = |w: u64, n: u64| n < w;
    let reset = less(was.input, now.input)
        || less(was.output, now.output)
        || less(was.cache_read, now.cache_read)
        || less(was.cache_write, now.cache_write);
    if reset {
        return now;
    }
    Tokens {
        input: now.input - was.input,
        cache_read: now.cache_read - was.cache_read,
        cache_write: now.cache_write - was.cache_write,
        output: now.output - was.output,
    }
}

fn cost_since(was: Option<Money>, now: Money) -> Money {
    match was {
        Some(w) if w.currency == now.currency && w.amount <= now.amount => Money {
            amount: now.amount - w.amount,
            ..now
        },
        _ => now,
    }
}

pub(crate) fn spent(
    reg: &Registry,
    (acp, name): (&str, &str),
    was: Option<&AcpUsage>,
    now: &AcpUsage,
    month: &str,
) -> Result<(), RpcError> {
    let catalog = providers::Catalog::load(&reg.providers).0;
    let Some(provider) = catalog.by_id(acp).filter(|p| p.config_dir_var.is_none()) else {
        return Ok(());
    };
    let total = |u: Option<&AcpUsage>| u.and_then(|u| u.total).unwrap_or_default();
    let tokens = tokens_since(total(was), total(Some(now)));
    let paid =
        money(now.cost.as_ref()).map(|c| cost_since(money(was.and_then(|w| w.cost.as_ref())), c));
    let cost = paid.or_else(|| provider.price.as_ref().map(|p| p.of(tokens)));
    if tokens == Tokens::default() && cost.as_ref().is_none_or(|c| c.amount <= 0.0) {
        return Ok(());
    }
    Store::beside(&reg.path).spent(provider, name, month, tokens, cost)
}

pub(crate) async fn open(cx: &Call<'_>, provider: &str) -> Result<(Store, Provider), RpcError> {
    let all = records(cx).await?;
    let found = all.into_iter().find(|p| p.agent == provider);
    let why = format!("no provider {provider} with a config folder variable or a balance probe");
    let provider = found.ok_or_else(|| RpcError::new(ErrorCode::NoAccount, why))?;
    Ok((store(cx), provider))
}

pub(crate) fn home() -> PathBuf {
    crate::config::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
}

pub(crate) fn store(cx: &Call<'_>) -> Store {
    Store::beside(&lock(cx.registry).path)
}

pub(crate) async fn records(cx: &Call<'_>) -> Result<Vec<Provider>, RpcError> {
    let dir = lock(cx.registry).providers.clone();
    blocking(move || records_in(&dir)).await
}

pub(crate) fn records_in(dir: &Path) -> Vec<Provider> {
    let catalog = providers::Catalog::load(dir).0;
    let held = catalog.iter().map(|e| &e.provider);
    let held = held.filter(|p| p.config_dir_var.is_some() || p.balance.is_some());
    held.cloned().collect()
}

pub(crate) fn listed(reg: &Registry, provider: &Provider, accounts: Vec<Account>) -> Vec<Account> {
    let mut shown: Vec<_> = (accounts.into_iter())
        .map(|a| self::shown(reg, &provider.id, a))
        .collect();
    let now = clock::unix_now();
    choice::rank(&mut shown, now);
    for a in shown.iter().filter(|a| check::stale(provider, a, now)) {
        let key = format!("{}/{}", a.provider, a.name);
        if !lock(&PROBING).insert(key.clone()) {
            continue;
        }
        let (store, provider, name) = (Store::beside(&reg.path), provider.clone(), a.name.clone());
        tokio::spawn(async move {
            check::status(&store, &provider, &name, &home()).await.ok();
            lock(&PROBING).remove(&key);
        });
    }
    shown
}

pub(crate) async fn beat(registry: Shared) {
    loop {
        tokio::time::sleep(BEAT).await;
        let (dir, path) = {
            let reg = lock(&registry);
            (reg.providers.clone(), reg.path.clone())
        };
        let store = Store::beside(&path);
        let read = move || {
            let store = Store::beside(&path);
            let all = records_in(&dir).into_iter().map(|r| (store.ls(&r), r));
            all.collect::<Vec<_>>()
        };
        let Ok(all) = blocking(read).await else {
            continue;
        };
        let home = home();
        for (accounts, record) in &all {
            for a in accounts {
                check::drift(&store, record, &a.name, &home).await.ok();
            }
        }
        let reg = lock(&registry);
        for (accounts, record) in all {
            let run = |a: &Account| reg.runs(&record.id, &a.name) > 0;
            listed(&reg, &record, accounts.into_iter().filter(run).collect());
        }
    }
}

pub(crate) fn limited(reg: &Registry, info: &TerminalInfo, was: Option<&str>) {
    let report = info.reported.as_ref();
    let Some(report) = report.filter(|r| r.state == "rate-limited" && was != Some("rate-limited"))
    else {
        return;
    };
    let (Some(name), Some(id)) = (info.account.clone(), info.acp.as_deref()) else {
        return;
    };
    let catalog = providers::Catalog::load(&reg.providers).0;
    let Some(provider) = catalog
        .by_id(id)
        .filter(|p| p.config_dir_var.is_some())
        .cloned()
    else {
        return;
    };
    let now = clock::unix_now();
    let resets = (report.note.as_deref()).and_then(|n| crate::proto::when(n, now));
    let store = Store::beside(&reg.path);
    tokio::spawn(async move { check::limit(&store, &provider, &name, resets).await.ok() });
}

pub(crate) fn login_running(reg: &Registry, provider: &str, name: &str) -> Option<String> {
    let terminal = store::login_terminal(provider, name);
    let running = reg.get(&terminal).is_ok_and(|t| t.state == State::Running);
    running.then_some(terminal)
}

pub(crate) fn shown(reg: &Registry, id: &str, account: Account) -> Account {
    Account {
        login: login_running(reg, &account.provider, &account.name),
        runs: reg.runs(id, &account.name),
        spend: account.spend.filter(|s| s.month == check::month()),
        ..account
    }
}

pub(crate) async fn login(cx: &Call<'_>, p: AccountLogin) -> Result<Login, RpcError> {
    let (store, provider) = open(cx, &p.provider).await?;
    let folder = store.folder(&provider, &p.name)?;
    let token = provider.token.as_ref().filter(|_| p.token);
    let remote = provider.remote_login.clone().filter(|_| p.remote);
    let line = match (&remote, token) {
        (Some(r), None) => r.command.as_ref().or(provider.login.as_ref()),
        (None, Some(t)) if !p.remote => Some(&t.command),
        (None, None) if !p.remote && !p.token => provider.login.as_ref(),
        _ => None,
    };
    let Some(line) = line else {
        let why = format!("{} has no login command for that", provider.agent);
        return Err(RpcError::new(ErrorCode::InvalidParams, why));
    };
    let prefix = token.map(|t| t.prefix.clone());
    let registry = lock(cx.registry).path.clone();
    let env = match remote {
        Some(_) => no_browser(&registry).await?,
        None => BTreeMap::new(),
    };
    let (terminal, term, fresh) = {
        let mut reg = cx.reg()?;
        if let Some(terminal) = login_running(&reg, &p.provider, &p.name) {
            let term = reg.live(&terminal).ok().map(Arc::clone);
            (terminal, term, false)
        } else {
            let terminal = store::login_terminal(&p.provider, &p.name);
            if reg.get(&terminal).is_ok() {
                let (_, files) = reg.rm(&terminal)?;
                files();
            }
            let mut cmd = check::command(&provider, folder.as_deref(), line, &env, &[]);
            let installed = crate::agents::list(&reg.path).unwrap_or_default();
            if let Some(agent) = installed.into_iter().find(|a| a.id == provider.id) {
                cmd.push("sh".to_owned());
                cmd.extend(agent.cmd);
            }
            let cwd = folder.unwrap_or_else(|| PathBuf::from("/"));
            let run = RunParams {
                name: Some(terminal.clone()),
                cmd,
                cwd,
                cols: (prefix.is_some() || remote.is_some()).then_some(1000),
                ..RunParams::default()
            };
            reg.run(run)?;
            reg.mark_redacted(&terminal);
            let term = reg.fresh(&terminal);
            (terminal, term, true)
        }
    };
    if let Some(term) = term.as_ref().filter(|_| fresh) {
        crate::serve::watch(cx.registry, terminal.clone(), Arc::clone(term));
        let (registry, name) = (Arc::clone(cx.registry), terminal.clone());
        let account = (store, provider, p.name);
        tokio::spawn(checked(registry, name, Arc::clone(term), account, prefix));
    }
    Ok(linked(terminal, term, remote.as_ref()).await)
}

async fn linked(
    terminal: String,
    term: Option<Arc<shed_core::Terminal>>,
    remote: Option<&RemoteLogin>,
) -> Login {
    let mut login = Login {
        terminal,
        link: None,
        code: None,
        answer: remote.map(|r| r.answer),
    };
    let (Some(term), Some(remote)) = (term, remote) else {
        return login;
    };
    let shown = |t: &shed_core::Terminal| remote.read(&t.screen()).is_some();
    term.wait_until(shown, Some(clock::now() + LINK)).await.ok();
    if let Some((link, code)) = remote.read(&term.screen()) {
        (login.link, login.code) = (Some(link), code);
    }
    login
}

async fn no_browser(registry: &Path) -> Result<BTreeMap<String, String>, RpcError> {
    let dir = registry.with_file_name("browser");
    let path = crate::config::var_os("PATH").unwrap_or_default();
    let env = BTreeMap::from([
        (
            "BROWSER".to_owned(),
            dir.join("xdg-open").display().to_string(),
        ),
        (
            "PATH".to_owned(),
            format!("{}:{}", dir.display(), path.to_string_lossy()),
        ),
    ]);
    blocking(move || {
        std::fs::create_dir_all(&dir)?;
        for name in ["open", "xdg-open"] {
            let file = dir.join(name);
            std::fs::write(&file, "#!/bin/sh\nprintf '%s\\n' \"$*\"\n")?;
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755))?;
        }
        Ok::<_, std::io::Error>(())
    })
    .await?
    .map_err(|e| RpcError::new(ErrorCode::Io, e.to_string()))?;
    Ok(env)
}

pub(crate) async fn answer(cx: &Call<'_>, p: AccountAnswer) -> Result<(), RpcError> {
    let (_, provider) = open(cx, &p.provider).await?;
    let Some(remote) = provider.remote_login else {
        let why = format!("{} has no remote login", provider.agent);
        return Err(RpcError::new(ErrorCode::InvalidParams, why));
    };
    let text = p.answer.trim();
    if text.is_empty() || text.chars().any(char::is_control) {
        let why = "the answer is one line of text";
        return Err(RpcError::new(ErrorCode::InvalidParams, why));
    }
    let Some(terminal) = login_running(&*cx.reg()?, &p.provider, &p.name) else {
        let why = format!("no login runs for {}/{}", p.provider, p.name);
        return Err(RpcError::new(ErrorCode::NoSuchTerminal, why));
    };
    match remote.answer {
        LoginAnswer::Paste => {
            let text = text.to_owned();
            let send = SendParams {
                name: terminal,
                text,
                ..SendParams::default()
            };
            let input = cx.reg()?.send(send)?;
            input.write().await
        }
        LoginAnswer::Callback => callback(text).await,
        LoginAnswer::None => {
            let why = format!(
                "{}'s login takes no answer: finish the link's page",
                provider.agent
            );
            Err(RpcError::new(ErrorCode::InvalidParams, why))
        }
    }
}

async fn callback(address: &str) -> Result<(), RpcError> {
    let local = ["http://localhost:", "http://127.0.0.1:"];
    if !local.iter().any(|l| address.starts_with(l)) {
        let why = "a callback is a http://localhost: or http://127.0.0.1: address";
        return Err(RpcError::new(ErrorCode::InvalidParams, why));
    }
    let cmd = [
        "curl",
        "-sS",
        "-o",
        "/dev/null",
        "--max-time",
        "10",
        "--",
        address,
    ];
    let cmd: Vec<String> = cmd.iter().map(|&w| w.to_owned()).collect();
    let none = BTreeMap::new();
    let ran = runner::exec(&cmd, Path::new("/"), &none, Vec::new(), CALLBACK, 4096).await;
    match ran {
        Ok((status, _, _)) if status.success() => Ok(()),
        Ok((_, _, err)) => {
            let why = format!(
                "the login took no callback: {}",
                String::from_utf8_lossy(&err).trim()
            );
            Err(RpcError::new(ErrorCode::InvalidParams, why))
        }
        Err(e) => Err(RpcError::new(ErrorCode::Io, e.to_string())),
    }
}

async fn checked(
    registry: Shared,
    terminal: String,
    term: Arc<shed_core::Terminal>,
    (store, provider, name): (Store, Provider, String),
    prefix: Option<String>,
) {
    let ended = term.wait_until(|_| false, Some(clock::now() + LOGIN)).await;
    if !matches!(ended, Ok(Wake::Exited)) {
        crate::serve::kill(&registry, &terminal).await.ok();
    }
    let token = prefix.and_then(|p| Token::on_screen(&term.screen(), &p));
    if let Some(token) = token {
        store.save_token(&provider, &name, &token).ok();
    }
    check::status(&store, &provider, &name, &home()).await.ok();
}
