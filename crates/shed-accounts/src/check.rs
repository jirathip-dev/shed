//! The record's status, usage, login and logout commands for one account, through the runner, with the
//! account's folder set and none of the record's `scrub` or `api_vars` inherited.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use shed_core::{clock, redact, runner};
use shed_proto::{Account, AccountState, Provider, RpcError, Usage};

use crate::choice::fullest;
use crate::store::{Reading, Store};

const HOLD: u64 = 3600;
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
    let var = provider.config_dir_var.iter().filter(|_| folder.is_none());
    let gone = crate::env::dropped(provider).chain(unset).chain(var);
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
    let env = BTreeMap::new();
    match runner::exec(cmd, Path::new("/"), &env, Vec::new(), TIMEOUT, LIMIT).await {
        Ok((status, out, err)) => (
            status.success(),
            String::from_utf8_lossy(&out).into_owned(),
            String::from_utf8_lossy(&err).into_owned(),
        ),
        Err(e) => (false, String::new(), e.to_string()),
    }
}

async fn usage(provider: &Provider, folder: Option<&Path>, now: u64) -> Option<Usage> {
    let probe = provider.usage.as_ref()?;
    let (_, out, _) = run(&command(
        provider,
        folder,
        &probe.command,
        &probe.env,
        &probe.unset,
    ))
    .await;
    provider.read_usage(&out, now)
}

/// Runs the record's status command and usage probe for the account now, and keeps what they say.
///
/// # Errors
/// `NO_ACCOUNT`, or `IO` when the reading cannot be kept.
pub async fn status(store: &Store, provider: &Provider, name: &str) -> Result<Account, RpcError> {
    let folder = store.folder(provider, name)?;
    let mut reading = store.reading(provider, name);
    let (none, now) = (BTreeMap::new(), clock::unix_now());
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
            checked_at: Some(now),
            ..reading
        };
    }
    if provider.usage.is_some() && reading.state != AccountState::SignedOut {
        let read = usage(provider, folder.as_deref(), now).await;
        if (read.as_ref())
            .and_then(|u| fullest(u, now))
            .is_some_and(|w| w.0 < 100)
        {
            reading.limited_until = None;
        }
        (reading.usage, reading.checked_at) = (read.or(reading.usage), Some(now));
    }
    store.keep(provider, name, reading)?;
    store.account(provider, name)
}

/// Marks the account limited until `resets`, else the fullest window's reset in a reading taken now, else the
/// record's `usage.hold` from now.
///
/// # Errors
/// `NO_ACCOUNT`, or `IO` when the mark cannot be kept.
pub async fn limit(
    store: &Store,
    provider: &Provider,
    name: &str,
    resets: Option<u64>,
) -> Result<Account, RpcError> {
    let (folder, now) = (store.folder(provider, name)?, clock::unix_now());
    let read = match resets {
        Some(_) => None,
        None => usage(provider, folder.as_deref(), now).await,
    };
    let reset = (read.as_ref()).and_then(|u| fullest(u, now)?.1);
    let hold = provider.usage.as_ref().map_or(HOLD, |u| u.hold);
    let until = resets.or(reset.filter(|t| *t > now)).unwrap_or(now + hold);
    let old = store.reading(provider, name);
    let reading = Reading {
        limited_until: Some(until),
        usage: read.or(old.usage),
        ..old
    };
    store.keep(provider, name, reading)?;
    store.account(provider, name)
}

/// Whether the account's reading is older than the record's `usage.fresh`; never for a provider without a
/// probe or an account signed out.
#[must_use]
pub fn stale(provider: &Provider, account: &Account, now: u64) -> bool {
    let read = |fresh| account.checked_at.is_none_or(|t| t + fresh <= now);
    let signed_in = account.state != AccountState::SignedOut;
    signed_in && provider.usage.as_ref().is_some_and(|u| read(u.fresh))
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

    use shed_proto::{AccountAdd, AccountState, Cache, UsageProbe, WindowRead};

    use super::{command, limit, logout, stale, status};
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

    #[tokio::test]
    async fn a_limit_holds_until_its_reset_and_a_reading_under_full_ends_it() {
        let dir = temp("check-limit");
        let store = Store::beside(&dir.path().join("state.json"));
        let mut p = provider();
        p.status = Some("echo in".into());
        p.usage = Some(UsageProbe {
            command: "cat \"$FAKE_HOME/usage\"".into(),
            env: BTreeMap::new(),
            unset: Vec::new(),
            fresh: 300,
            hold: 3600,
            windows: vec![WindowRead {
                id: "5h".into(),
                line: "Session:".into(),
                used: "% used".into(),
                resets: "resets ".into(),
            }],
        });
        let add = AccountAdd {
            provider: "fake".into(),
            name: "work".into(),
            ..AccountAdd::default()
        };
        let folder = store.add(&p, &add).expect("added").folder.expect("folder");
        let usage = folder.join("usage");
        fs::write(&usage, "Session: 100% used, resets in 2h").expect("write");
        let now = shed_core::clock::unix_now();
        let read = limit(&store, &p, "work", None).await.expect("limit");
        assert_eq!(read.state, AccountState::Limited);
        let until = read.limited_until.expect("until");
        assert!((now + 7200..now + 7210).contains(&until), "{until}");
        let read = limit(&store, &p, "work", Some(now + 60))
            .await
            .expect("limit");
        assert_eq!(read.limited_until, Some(now + 60));
        fs::write(&usage, "Session: 40% used, resets in 2h").expect("write");
        let read = status(&store, &p, "work").await.expect("status");
        assert_eq!(
            (read.state, read.limited_until),
            (AccountState::Ready, None)
        );
        let at = read.checked_at.expect("checked");
        assert!(!stale(&p, &read, at) && stale(&p, &read, at + 300));
        fs::remove_file(&usage).expect("rm");
        let read = limit(&store, &p, "work", None).await.expect("limit");
        let until = read.limited_until.expect("held");
        assert!((now + 3600..now + 3610).contains(&until), "{until}");
    }
}
