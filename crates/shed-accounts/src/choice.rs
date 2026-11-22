//! What a run's `account` asks for, and the account `auto` picks: a provider's signed-in named account that is
//! not limited, the preferred one first, then the most headroom, then fewer runs, then by name. It does no I/O.

use std::cmp::Reverse;

use shed_proto::{Account, AccountState, ErrorCode, RpcError, Usage};

use crate::store::DEFAULT;

/// The name that asks for [`Choice::Auto`].
pub const AUTO: &str = "auto";

/// What a run's `account` asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    /// That account.
    Named(String),
    /// The account with the most headroom, never `default`.
    Auto,
}

impl From<&str> for Choice {
    fn from(name: &str) -> Self {
        if name == AUTO {
            Self::Auto
        } else {
            Self::Named(name.to_owned())
        }
    }
}

/// The used percentage and reset of the fullest window; a window past its reset reads as 0 % used.
#[must_use]
pub fn fullest(usage: &Usage, now: u64) -> Option<(u32, Option<u64>)> {
    let used = |w: &shed_proto::Window| match w.resets_at {
        Some(t) if t <= now => 0,
        _ => w.used_pct,
    };
    let windows = usage.windows.iter().map(|w| (used(w), w.resets_at));
    windows.max_by_key(|w| w.0)
}

fn headroom(a: &Account, now: u64) -> u32 {
    let used = a.usage.as_ref().and_then(|u| fullest(u, now));
    100_u32.saturating_sub(used.map_or(0, |w| w.0))
}

fn candidate(a: &Account) -> bool {
    a.name != DEFAULT && a.state == AccountState::Ready && a.setup.is_empty()
}

fn key(a: &Account, now: u64) -> (bool, bool, bool, Reverse<u32>, u32, &str) {
    let first = (a.name != DEFAULT, !candidate(a), !a.preferred);
    (
        first.0,
        first.1,
        first.2,
        Reverse(headroom(a, now)),
        a.runs,
        &a.name,
    )
}

/// Whether `account.use` may make `a` current: a named account that is ready and holds no setup.
///
/// # Errors
/// `INVALID_PARAMS` for `default`, `ACCOUNT_LIMITED`, `ACCOUNT_SIGNED_OUT` or `ACCOUNT_SETUP`.
pub fn usable(a: &Account) -> Result<(), RpcError> {
    let named = format!("{}/{}", a.provider, a.name);
    let (code, why) = match a.state {
        _ if a.name == DEFAULT => (ErrorCode::InvalidParams, "default: auto never picks it"),
        AccountState::Limited => (ErrorCode::AccountLimited, "is limited"),
        AccountState::SignedOut => (ErrorCode::AccountSignedOut, "is signed out"),
        AccountState::Drifted => (ErrorCode::AccountSignedOut, "has drifted"),
        AccountState::Ready if !a.setup.is_empty() => (ErrorCode::AccountSetup, "holds a setup"),
        AccountState::Ready => return Ok(()),
    };
    Err(RpcError::new(code, format!("{named} {why}")))
}

/// Sorts one provider's accounts as `account.ls` shows them: `default` first, then the order `auto` picks.
pub fn rank(accounts: &mut [Account], now: u64) {
    accounts.sort_by(|a, b| key(a, now).cmp(&key(b, now)));
}

/// The account `auto` picks among one provider's `accounts`, never `except`.
///
/// # Errors
/// `NO_ACCOUNT` without a named account, `ACCOUNT_LIMITED` with the first reset when one is limited, else
/// `ACCOUNT_SIGNED_OUT`.
pub fn pick(accounts: &[Account], except: Option<&str>, now: u64) -> Result<String, RpcError> {
    let named =
        || (accounts.iter()).filter(|a| a.name != DEFAULT && Some(a.name.as_str()) != except);
    let best = named().filter(|a| candidate(a)).min_by_key(|a| key(a, now));
    if let Some(a) = best {
        return Ok(a.name.clone());
    }
    let first = named().filter_map(|a| a.limited_until).min();
    Err(match (named().next(), first) {
        (None, _) => RpcError::new(
            ErrorCode::NoAccount,
            "auto has no named account to pick: `shed account add`",
        ),
        (_, Some(at)) => RpcError::new(
            ErrorCode::AccountLimited,
            format!("every account is limited; the first resets at {at} (unix seconds)"),
        ),
        (Some(a), None) => RpcError::new(
            ErrorCode::AccountSignedOut,
            format!(
                "every account is signed out: `shed account login {} <name>`",
                a.provider
            ),
        ),
    })
}

#[cfg(test)]
mod tests {
    use shed_proto::{Account, AccountState, ErrorCode, Usage, Window};

    use super::{Choice, pick, rank, usable};

    fn account(name: &str, state: AccountState, used: Option<u32>, runs: u32) -> Account {
        let usage = used.map(|used_pct| Usage {
            windows: vec![Window {
                id: "5h".into(),
                used_pct,
                resets_at: Some(2_000),
            }],
            read_at: 900,
        });
        Account {
            provider: "fake".into(),
            name: name.into(),
            folder: None,
            adopted: false,
            state,
            who: None,
            line: String::new(),
            drift: None,
            checked_at: None,
            limited_until: (state == AccountState::Limited).then_some(1_500),
            usage,
            token: None,
            setup: Vec::new(),
            runs,
            login: None,
            preferred: false,
        }
    }

    fn preferred(mut a: Account) -> Account {
        a.preferred = true;
        a
    }

    #[test]
    fn auto_skips_a_drifted_account_like_a_signed_out_one() {
        let drifted = account("a", AccountState::Drifted, Some(0), 0);
        let accounts = [
            drifted.clone(),
            account("b", AccountState::Ready, Some(90), 5),
        ];
        assert_eq!(pick(&accounts, None, 1_000).ok().as_deref(), Some("b"));
        let alone = pick(std::slice::from_ref(&drifted), None, 1_000).map_err(|e| e.code);
        assert_eq!(alone, Err(ErrorCode::AccountSignedOut.code()));
        assert!(usable(&drifted).is_err());
    }

    #[test]
    fn auto_picks_the_most_headroom_then_fewer_runs_then_by_name_and_never_default() {
        use AccountState::{Limited, Ready, SignedOut};
        let table: [(&[Account], Result<&str, i64>); 10] = [
            (
                &[
                    account("a", Ready, Some(10), 0),
                    preferred(account("b", Ready, Some(90), 5)),
                ],
                Ok("b"),
            ),
            (
                &[
                    account("a", Ready, Some(10), 0),
                    preferred(account("b", Limited, Some(100), 0)),
                ],
                Ok("a"),
            ),
            (
                &[
                    account("a", Ready, Some(10), 0),
                    account("b", Ready, Some(90), 5),
                ],
                Ok("a"),
            ),
            (
                &[
                    account("default", Ready, None, 0),
                    account("a", Ready, Some(80), 0),
                    account("b", Ready, Some(20), 3),
                ],
                Ok("b"),
            ),
            (
                &[
                    account("a", Ready, Some(50), 2),
                    account("b", Ready, Some(50), 1),
                ],
                Ok("b"),
            ),
            (
                &[
                    account("b", Ready, Some(50), 1),
                    account("a", Ready, Some(50), 1),
                ],
                Ok("a"),
            ),
            (
                &[
                    account("a", Ready, None, 0),
                    account("b", Ready, Some(1), 0),
                ],
                Ok("a"),
            ),
            (
                &[account("default", Ready, None, 0)],
                Err(ErrorCode::NoAccount.code()),
            ),
            (
                &[
                    account("a", Limited, Some(100), 0),
                    account("b", SignedOut, None, 0),
                ],
                Err(ErrorCode::AccountLimited.code()),
            ),
            (
                &[account("a", SignedOut, None, 0)],
                Err(ErrorCode::AccountSignedOut.code()),
            ),
        ];
        for (accounts, want) in table {
            let got = pick(accounts, None, 1_000).map_err(|e| e.code);
            assert_eq!(got.as_deref(), want.as_ref().map(|s| *s), "{accounts:?}");
        }
        let mut both = [
            account("b", Ready, Some(90), 0),
            account("a", Ready, Some(10), 0),
        ];
        for w in both
            .iter_mut()
            .flat_map(|a| &mut a.usage)
            .flat_map(|u| &mut u.windows)
        {
            w.resets_at = Some(if w.used_pct == 90 { 1_500 } else { 3_000 });
        }
        assert_eq!(pick(&both, Some("a"), 1_000).expect("b"), "b");
        assert_eq!(pick(&both, None, 1_000).expect("a"), "a");
        rank(&mut both, 1_600);
        assert_eq!(both.map(|a| a.name), ["b", "a"], "b's window reset");
        let mut listed = [
            account("a", Ready, Some(10), 0),
            preferred(account("b", Ready, Some(90), 0)),
            account("default", Ready, None, 0),
        ];
        rank(&mut listed, 1_000);
        assert_eq!(listed.map(|a| a.name), ["default", "b", "a"]);
        assert_eq!(Choice::from("auto"), Choice::Auto);
        assert_eq!(Choice::from("work"), Choice::Named("work".into()));
    }

    #[test]
    fn only_a_ready_named_account_without_a_setup_is_usable() {
        use AccountState::{Limited, Ready, SignedOut};
        let mut setup = account("s", Ready, None, 0);
        setup.setup = vec!["skills".into()];
        let refused = [
            (account("default", Ready, None, 0), ErrorCode::InvalidParams),
            (account("l", Limited, None, 0), ErrorCode::AccountLimited),
            (
                account("o", SignedOut, None, 0),
                ErrorCode::AccountSignedOut,
            ),
            (setup, ErrorCode::AccountSetup),
        ];
        for (a, code) in refused {
            assert_eq!(
                usable(&a).map_err(|e| e.code),
                Err(code.code()),
                "{}",
                a.name
            );
        }
        assert!(usable(&account("b", Ready, None, 0)).is_ok());
    }
}
