//! What a run on an account gets and loses: the account's folder variable set (its token too while the folder's
//! own login is signed out, or its key alone when it has no folder), and none of the record's `scrub` and `cache.api_vars` inherited.

use std::collections::BTreeMap;

use shed_proto::{AccountState, ErrorCode, Provider, RpcError};

use crate::store::Store;
use crate::token::Token;

/// The variables a run on an account gets over the server's own, and the server's own it does not inherit.
#[derive(Default)]
pub struct Injected {
    /// Set for the run; the token among them stays in memory, like `run`'s `env`.
    pub set: BTreeMap<String, String>,
    /// Dropped from what the run inherits.
    pub unset: Vec<String>,
}

/// The record's variables a run on an account never inherits: its `scrub` and `cache.api_vars`.
pub fn dropped(provider: &Provider) -> impl Iterator<Item = &String> {
    let api = provider.cache.iter().flat_map(|c| &c.api_vars);
    provider.scrub.iter().chain(api)
}

fn owned(provider: &Provider) -> impl Iterator<Item = &String> {
    let proxy = (provider.proxy.iter()).flat_map(|p| [&p.base_url_var, &p.key_var]);
    let login = provider.config_dir_var.iter().chain(&provider.token_var);
    dropped(provider).chain(login).chain(proxy)
}

impl Injected {
    /// What a run on account `name` gets, `env` being the caller's own variables for it.
    ///
    /// # Errors
    /// `INVALID_PARAMS` when `env` sets one of the account's variables, `NO_ACCOUNT`, `ACCOUNT_SETUP` when the
    /// folder holds a setup, `ACCOUNT_LIMITED`, `ACCOUNT_SIGNED_OUT` for a refused key, and the token file's own
    /// errors.
    pub fn for_run(
        store: &Store,
        provider: &Provider,
        name: &str,
        env: &BTreeMap<String, String>,
    ) -> Result<Self, RpcError> {
        if let Some(var) = owned(provider).find(|v| env.contains_key(*v)) {
            let why = format!("env sets {var}, which a run on an account gets from the account");
            return Err(RpcError::new(ErrorCode::InvalidParams, why));
        }
        if provider.config_dir_var.is_none() && provider.token_var.is_none() {
            let why = format!("{} has no config folder or key variable", provider.agent);
            return Err(RpcError::new(ErrorCode::NoAccount, why));
        }
        let account = store.account(provider, name)?;
        if let Some(file) = account.setup.first() {
            let why = format!(
                "{}/{name} holds {file}: a setup, not a login",
                provider.agent
            );
            return Err(RpcError::new(ErrorCode::AccountSetup, why));
        }
        if let Some(at) = account.limited_until {
            let why = format!(
                "{}/{name} is limited until {at} (unix seconds)",
                provider.agent
            );
            return Err(RpcError::new(ErrorCode::AccountLimited, why));
        }
        let key_only = provider.config_dir_var.is_none();
        if key_only && account.token.as_ref().is_some_and(|t| t.rejected) {
            let p = &provider.agent;
            let why = format!(
                "the upstream refused {p}/{name}'s key: `shed account add {p} {name} --token-file <file>`"
            );
            return Err(RpcError::new(ErrorCode::AccountSignedOut, why));
        }
        let mut run = Self {
            set: provider.env.clone(),
            unset: dropped(provider).cloned().collect(),
        };
        match (&provider.config_dir_var, account.folder) {
            (Some(var), Some(folder)) => {
                run.set.insert(var.clone(), folder.display().to_string());
            }
            (Some(var), None) => run.unset.push(var.clone()),
            (None, _) => {}
        }
        let signed_out = key_only || store.reading(provider, name).state == AccountState::SignedOut;
        let token = account.token.filter(|t| !t.rejected);
        if let (Some(var), Some(_), true) = (&provider.token_var, token, signed_out) {
            let token = Token::read(&store.token_file(provider, name))?;
            run.set.insert(var.clone(), token.expose().to_owned());
        }
        run.unset.retain(|v| !run.set.contains_key(v));
        Ok(run)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::os::unix::fs::PermissionsExt as _;

    use shed_proto::{AccountAdd, AccountState, Cache, ErrorCode, Provider};

    use super::Injected;
    use crate::store::{Reading, Store, tests::temp};

    fn record() -> Provider {
        Provider {
            agent: "fake".into(),
            config_dir_var: Some("FAKE_HOME".into()),
            token_var: Some("FAKE_TOKEN".into()),
            scrub: vec!["FAKE_TOKEN".into(), "FAKE_SCRUB".into()],
            cache: Some(Cache {
                plan: 1,
                api: 1,
                api_vars: vec!["FAKE_API_KEY".into()],
            }),
            setup_files: vec!["settings.json".into()],
            env: BTreeMap::from([("FAKE_STORE".into(), "file".into())]),
            ..Provider::default()
        }
    }

    #[test]
    fn a_run_gets_the_folder_and_a_signed_out_account_s_token_and_loses_the_scrubbed_variables() {
        let dir = temp("env-run");
        let store = Store::beside(&dir.path().join("state.json"));
        let provider = record();
        let file = dir.path().join("t.token");
        fs::write(&file, "sk-test-0123456789abcdef").expect("write");
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).expect("chmod");
        let add = AccountAdd {
            provider: "fake".into(),
            name: "work".into(),
            token_file: Some(file),
            ..AccountAdd::default()
        };
        let folder = store.add(&provider, &add).expect("added").folder;
        let none = BTreeMap::new();
        let run = Injected::for_run(&store, &provider, "work", &none).expect("injected");
        let home = folder.map(|f| f.display().to_string());
        assert_eq!(run.set.get("FAKE_HOME"), home.as_ref());
        assert_eq!(
            run.set.get("FAKE_TOKEN").map(String::as_str),
            Some("sk-test-0123456789abcdef")
        );
        assert_eq!(run.unset, ["FAKE_SCRUB", "FAKE_API_KEY"]);
        let ready = Reading {
            state: AccountState::Ready,
            ..Reading::default()
        };
        store.keep(&provider, "work", ready).expect("kept");
        let run = Injected::for_run(&store, &provider, "work", &none).expect("injected");
        assert_eq!(
            run.set.keys().collect::<Vec<_>>(),
            ["FAKE_HOME", "FAKE_STORE"]
        );
        assert_eq!(run.unset, ["FAKE_TOKEN", "FAKE_SCRUB", "FAKE_API_KEY"]);
        let default = Injected::for_run(&store, &provider, "default", &none).expect("default");
        assert_eq!(default.set.keys().collect::<Vec<_>>(), ["FAKE_STORE"]);
        assert_eq!(default.unset.last().map(String::as_str), Some("FAKE_HOME"));
    }

    fn keyed(dir: &std::path::Path) -> (Store, Provider) {
        let store = Store::beside(&dir.join("state.json"));
        let provider = Provider {
            agent: "keyed".into(),
            token_var: Some("KEYED_API_KEY".into()),
            scrub: vec!["KEYED_SCRUB".into()],
            ..Provider::default()
        };
        let file = dir.join("k.token");
        fs::write(&file, "sk-key-0123456789abcdef").expect("write");
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).expect("chmod");
        let add = AccountAdd {
            provider: "keyed".into(),
            name: "api".into(),
            token_file: Some(file),
            ..AccountAdd::default()
        };
        store.add(&provider, &add).expect("added");
        (store, provider)
    }

    #[test]
    fn a_run_on_a_key_only_account_gets_its_key_and_no_folder() {
        let dir = temp("env-keyed");
        let (store, provider) = keyed(dir.path());
        let none = BTreeMap::new();
        let run = Injected::for_run(&store, &provider, "api", &none).expect("injected");
        assert_eq!(
            run.set.get("KEYED_API_KEY").map(String::as_str),
            Some("sk-key-0123456789abcdef")
        );
        assert_eq!(run.set.len(), 1);
        assert_eq!(run.unset, ["KEYED_SCRUB"]);
    }

    #[test]
    fn a_key_only_run_is_refused_for_a_caller_key_a_limit_or_a_rejected_key() {
        let dir = temp("env-keyed-refused");
        let (store, provider) = keyed(dir.path());
        let env = BTreeMap::from([("KEYED_API_KEY".to_owned(), "mine".to_owned())]);
        let refused = Injected::for_run(&store, &provider, "api", &env).map(|_| ());
        let refused = refused.expect_err("caller key");
        assert_eq!(refused.code, ErrorCode::InvalidParams.code());
        let none = BTreeMap::new();
        let limited = Reading {
            limited_until: Some(shed_core::clock::unix_now() + 60),
            ..Reading::default()
        };
        store.keep(&provider, "api", limited).expect("kept");
        let refused = Injected::for_run(&store, &provider, "api", &none).map(|_| ());
        assert_eq!(
            refused.expect_err("limited").code,
            ErrorCode::AccountLimited.code()
        );
        store
            .keep(&provider, "api", Reading::default())
            .expect("kept");
        assert!(store.reject(&provider, "api").expect("rejected"));
        let refused = Injected::for_run(&store, &provider, "api", &none).map(|_| ());
        let refused = refused.expect_err("rejected");
        assert_eq!(refused.code, ErrorCode::AccountSignedOut.code());
        assert!(!refused.message.contains("sk-key"), "{}", refused.message);
    }

    #[test]
    fn a_caller_env_that_sets_an_account_variable_or_a_setup_folder_is_refused() {
        let dir = temp("env-refused");
        let store = Store::beside(&dir.path().join("state.json"));
        let provider = record();
        let add = AccountAdd {
            provider: "fake".into(),
            name: "work".into(),
            ..AccountAdd::default()
        };
        let folder = store.add(&provider, &add).expect("added").folder;
        for var in ["FAKE_HOME", "FAKE_TOKEN", "FAKE_SCRUB", "FAKE_API_KEY"] {
            let env = BTreeMap::from([(var.to_owned(), "mine".to_owned())]);
            let refused = Injected::for_run(&store, &provider, "work", &env).map(|_| ());
            let refused = refused.expect_err("refused");
            assert_eq!(refused.code, ErrorCode::InvalidParams.code(), "{var}");
            assert!(refused.message.contains(var), "{}", refused.message);
        }
        let limited = Reading {
            limited_until: Some(shed_core::clock::unix_now() + 60),
            ..Reading::default()
        };
        store.keep(&provider, "work", limited).expect("kept");
        let refused = Injected::for_run(&store, &provider, "work", &BTreeMap::new()).map(|_| ());
        let refused = refused.expect_err("limited");
        assert_eq!(refused.code, ErrorCode::AccountLimited.code());
        store
            .keep(&provider, "work", Reading::default())
            .expect("kept");
        fs::write(folder.expect("a folder").join("settings.json"), "{}").expect("write");
        let refused = Injected::for_run(&store, &provider, "work", &BTreeMap::new()).map(|_| ());
        assert_eq!(
            refused.expect_err("setup").code,
            ErrorCode::AccountSetup.code()
        );
    }
}
