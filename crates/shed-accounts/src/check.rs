//! The record's status, usage, login and logout commands for one account, through the runner, with the
//! account's folder set and none of the record's `scrub` or `api_vars` inherited.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use shed_core::{clock, redact, runner};
use shed_proto::{Account, AccountState, Provider, RpcError};

use crate::store::{Reading, Store};

const TIMEOUT: Duration = Duration::from_mins(2);
const LIMIT: u64 = 1 << 16;
const LINE: usize = 80;

/// The argv that runs `line` for an account in `folder` (none: the CLI's default) with `env` set and `unset` unset.
#[must_use]
pub fn command(
    provider: &Provider,
    folder: Option<&Path>,
    line: &str,
    env: &BTreeMap<String, String>,
    unset: &[String],
) -> Vec<String> {
    let api = provider.cache.iter().flat_map(|c| &c.api_vars);
    let var = provider.config_dir_var.iter().filter(|_| folder.is_none());
    let gone = (provider.scrub.iter()).chain(api).chain(unset).chain(var);
    let mut cmd = vec!["env".to_owned()];
    cmd.extend(gone.flat_map(|v| ["-u".to_owned(), v.clone()]));
    if let (Some(var), Some(folder)) = (&provider.config_dir_var, folder) {
        cmd.push(format!("{var}={}", folder.display()));
    }
    cmd.extend(env.iter().map(|(k, v)| format!("{k}={v}")));
    cmd.extend(["sh".to_owned(), "-c".to_owned(), line.to_owned()]);
    cmd
}

async fn run(cmd: &[String]) -> (bool, String, String) {
    match runner::exec(cmd, Path::new("/"), Vec::new(), TIMEOUT, LIMIT).await {
        Ok((status, out, err)) => (
            status.success(),
            String::from_utf8_lossy(&out).into_owned(),
            String::from_utf8_lossy(&err).into_owned(),
        ),
        Err(e) => (false, String::new(), e.to_string()),
    }
}

/// Runs the record's status command and usage probe for the account now, and keeps what they say.
///
/// # Errors
/// `NO_ACCOUNT`, or `IO` when the reading cannot be kept.
pub async fn status(store: &Store, provider: &Provider, name: &str) -> Result<Account, RpcError> {
    let folder = store.folder(provider, name)?;
    let mut reading = store.reading(provider, name);
    let none = BTreeMap::new();
    if let Some(line) = &provider.status {
        let (ok, out, err) = run(&command(provider, folder.as_deref(), line, &none, &[])).await;
        let status = provider.read_status(ok, &out, &err);
        reading = Reading {
            state: if status.signed_in {
                AccountState::Ready
            } else {
                AccountState::SignedOut
            },
            who: status.who.as_deref().map(redact),
            line: redact(&status.line).chars().take(LINE).collect(),
            checked_at: Some(clock::unix_now()),
            ..reading
        };
    }
    if let Some(probe) = provider
        .usage
        .as_ref()
        .filter(|_| reading.state != AccountState::SignedOut)
    {
        let cmd = command(
            provider,
            folder.as_deref(),
            &probe.command,
            &probe.env,
            &probe.unset,
        );
        let (_, out, _) = run(&cmd).await;
        let read = provider.read_usage(&out, clock::unix_now());
        reading.usage = read.or(reading.usage);
    }
    store.keep(provider, name, reading)?;
    store.account(provider, name)
}

/// Runs the record's logout for the account, whatever it answers.
///
/// # Errors
/// `NO_ACCOUNT`.
pub async fn logout(store: &Store, provider: &Provider, name: &str) -> Result<(), RpcError> {
    let folder = store.folder(provider, name)?;
    if let Some(line) = &provider.logout {
        run(&command(
            provider,
            folder.as_deref(),
            line,
            &BTreeMap::new(),
            &[],
        ))
        .await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;

    use shed_proto::{AccountAdd, AccountState, Cache};

    use super::{command, logout, status};
    use crate::store::Store;
    use crate::store::tests::{provider, temp};

    #[test]
    fn a_command_sets_the_folder_and_inherits_no_scrubbed_variable() {
        let mut p = provider();
        p.scrub = vec!["FAKE_TOKEN".into()];
        p.cache = Some(Cache {
            api_vars: vec!["FAKE_API_KEY".into()],
            plan: 0,
            api: 0,
        });
        let none = BTreeMap::new();
        let named = command(&p, Some("/a/work".as_ref()), "fake auth", &none, &[]);
        let want = "env -u FAKE_TOKEN -u FAKE_API_KEY FAKE_HOME=/a/work sh -c fake auth";
        assert_eq!(named.join(" "), want);
        let default = command(&p, None, "fake auth", &none, &[]);
        let want = "env -u FAKE_TOKEN -u FAKE_API_KEY -u FAKE_HOME sh -c fake auth";
        assert_eq!(default.join(" "), want);
    }

    #[tokio::test]
    async fn status_runs_in_the_folder_and_keeps_the_redacted_line() {
        let dir = temp("check-status");
        let store = Store::beside(&dir.path().join("state.json"));
        let mut p = provider();
        p.status =
            Some("test -f \"$FAKE_HOME/in\" && echo in as a@b.c sk0123456789abcdefghijkl".into());
        p.logout = Some("rm \"$FAKE_HOME/in\"".into());
        let add = AccountAdd {
            provider: "fake".into(),
            name: "work".into(),
            ..AccountAdd::default()
        };
        let folder = store.add(&p, &add).expect("added").folder.expect("folder");
        let out = status(&store, &p, "work").await.expect("status");
        assert_eq!(out.state, AccountState::SignedOut);
        fs::write(folder.join("in"), "").expect("write");
        let read = status(&store, &p, "work").await.expect("status");
        assert_eq!(read.state, AccountState::Ready);
        assert_eq!(read.line, "in as a@b.c [redacted]");
        assert!(read.checked_at.is_some());
        assert_eq!(store.account(&p, "work").expect("kept"), read);
        logout(&store, &p, "work").await.expect("logout");
        assert!(!folder.join("in").exists());
    }
}
