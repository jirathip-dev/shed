use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use shed_accounts::{Store, Token, check, choice, store};
use shed_core::{Wake, clock};

use crate::call::{Call, blocking};
use crate::proto::{
    Account, AccountLogin, ErrorCode, Login, Provider, RpcError, RunParams, State, TerminalInfo,
};
use crate::registry::Registry;
use crate::{Shared, lock, providers};

const LOGIN: Duration = Duration::from_mins(10);
const BEAT: Duration = Duration::from_mins(1);

static PROBING: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

pub(crate) async fn open(cx: &Call<'_>, provider: &str) -> Result<(Store, Provider), RpcError> {
    let all = records(cx).await?;
    let found = all.into_iter().find(|p| p.agent == provider);
    let why = format!("no provider {provider} with a config folder variable");
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
    let held = held.filter(|p| p.config_dir_var.is_some());
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
        ..account
    }
}

pub(crate) async fn login(cx: &Call<'_>, p: AccountLogin) -> Result<Login, RpcError> {
    let (store, provider) = open(cx, &p.provider).await?;
    let folder = store.folder(&provider, &p.name)?;
    let token = provider.token.as_ref().filter(|_| p.token);
    let line = token.map(|t| &t.command).or(provider.login.as_ref());
    let Some(line) = line.filter(|_| token.is_some() || !p.token) else {
        let why = format!("{} has no login command for that", provider.agent);
        return Err(RpcError::new(ErrorCode::InvalidParams, why));
    };
    let prefix = token.map(|t| t.prefix.clone());
    let mut reg = cx.reg()?;
    if let Some(terminal) = login_running(&reg, &p.provider, &p.name) {
        return Ok(Login { terminal });
    }
    let terminal = store::login_terminal(&p.provider, &p.name);
    if reg.get(&terminal).is_ok() {
        let (_, files) = reg.rm(&terminal)?;
        files();
    }
    let none = std::collections::BTreeMap::new();
    let cmd = check::command(&provider, folder.as_deref(), line, &none, &[]);
    let cwd = folder.unwrap_or_else(|| PathBuf::from("/"));
    let run = RunParams {
        name: Some(terminal.clone()),
        cmd,
        cwd,
        cols: prefix.is_some().then_some(1000),
        ..RunParams::default()
    };
    reg.run(run)?;
    reg.mark_redacted(&terminal);
    let term = reg.fresh(&terminal);
    drop(reg);
    if let Some(term) = term {
        crate::serve::watch(cx.registry, terminal.clone(), Arc::clone(&term));
        let (registry, name) = (Arc::clone(cx.registry), terminal.clone());
        let account = (store, provider, p.name);
        tokio::spawn(checked(registry, name, term, account, prefix));
    }
    Ok(Login { terminal })
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
