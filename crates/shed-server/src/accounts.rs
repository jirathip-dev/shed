//! What the account methods share: the store beside the state file, the provider's record and the login terminal.

use std::path::PathBuf;
use std::sync::Arc;

use shed_accounts::{Store, check, store};
use shed_core::{Wake, clock};

use crate::call::{Call, blocking};
use crate::proto::{Account, AccountName, ErrorCode, Login, Provider, RpcError, RunParams, State};
use crate::registry::Registry;
use crate::{Shared, lock, providers};

const LOGIN: std::time::Duration = std::time::Duration::from_mins(10);

pub(crate) async fn open(cx: &Call<'_>, provider: &str) -> Result<(Store, Provider), RpcError> {
    let all = records(cx).await?;
    let found = all.into_iter().find(|p| p.agent == provider);
    let why = format!("no provider {provider} with a config folder variable");
    let provider = found.ok_or_else(|| RpcError::new(ErrorCode::NoAccount, why))?;
    Ok((store(cx), provider))
}

pub(crate) fn store(cx: &Call<'_>) -> Store {
    Store::beside(&lock(cx.registry).path)
}

pub(crate) async fn records(cx: &Call<'_>) -> Result<Vec<Provider>, RpcError> {
    let dir = lock(cx.registry).providers.clone();
    let catalog = blocking(move || providers::Catalog::load(&dir).0).await?;
    let held = catalog.iter().map(|e| &e.provider);
    Ok(held
        .filter(|p| p.config_dir_var.is_some())
        .cloned()
        .collect())
}

pub(crate) fn login_running(reg: &Registry, provider: &str, name: &str) -> Option<String> {
    let terminal = store::login_terminal(provider, name);
    let running = reg.get(&terminal).is_ok_and(|t| t.state == State::Running);
    running.then_some(terminal)
}

pub(crate) fn shown(reg: &Registry, account: Account) -> Account {
    Account {
        login: login_running(reg, &account.provider, &account.name),
        ..account
    }
}

pub(crate) async fn login(cx: &Call<'_>, p: AccountName) -> Result<Login, RpcError> {
    let (store, provider) = open(cx, &p.provider).await?;
    let folder = store.folder(&provider, &p.name)?;
    let Some(line) = &provider.login else {
        let why = format!("{} has no login command", provider.agent);
        return Err(RpcError::new(ErrorCode::InvalidParams, why));
    };
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
        ..RunParams::default()
    };
    reg.run(run)?;
    reg.mark_redacted(&terminal);
    let term = reg.fresh(&terminal);
    drop(reg);
    if let Some(term) = term {
        crate::serve::watch(cx.registry, terminal.clone(), Arc::clone(&term));
        let (registry, name) = (Arc::clone(cx.registry), terminal.clone());
        tokio::spawn(checked(registry, name, term, store, provider, p.name));
    }
    Ok(Login { terminal })
}

async fn checked(
    registry: Shared,
    terminal: String,
    term: Arc<shed_core::Terminal>,
    store: Store,
    provider: Provider,
    name: String,
) {
    let ended = term.wait_until(|_| false, Some(clock::now() + LOGIN)).await;
    if !matches!(ended, Ok(Wake::Exited)) {
        crate::serve::kill(&registry, &terminal).await.ok();
    }
    check::status(&store, &provider, &name).await.ok();
}
