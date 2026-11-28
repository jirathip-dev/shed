//! The accounts on disk: `accounts/<provider>/<name>` folders (or links to adopted ones), their `.token` files,
//! and `accounts.json`, what shed last read of each; every folder 0700, every file 0600.

use std::collections::BTreeMap;
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::Write as _;
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use serde::{Deserialize, Serialize};
use shed_proto::{
    Account, AccountAdd, AccountState, Balance, Drift, ErrorCode, Money, Provider, RpcError, Spend,
    SpendSource, TokenInfo, Tokens, Usage,
};

use crate::token::{self, Token};

/// The name of the CLI's own default folder.
pub const DEFAULT: &str = "default";

static KEEPING: Mutex<()> = Mutex::new(());

/// What shed last read of one account.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Reading {
    /// Signed in or out.
    pub state: AccountState,
    /// Who it is signed in as.
    pub who: Option<String>,
    /// The status line, redacted.
    pub line: String,
    /// When the status command ran.
    pub checked_at: Option<u64>,
    /// The last usage reading.
    pub usage: Option<Usage>,
    /// The last balance reading.
    #[serde(default)]
    pub balance: Option<Balance>,
    /// When its last limit ends.
    #[serde(default)]
    pub limited_until: Option<u64>,
    /// The `saved_at` of the token the proxy's upstream refused.
    #[serde(default)]
    pub rejected: Option<u64>,
    /// What the last drift check found.
    #[serde(default)]
    pub drift: Option<Drift>,
    /// `auto` picks it first while it is ready.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub preferred: bool,
    /// Its tokens and cost this month; only `spent` writes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spend: Option<Spend>,
    /// Its balance's drops this month; only `fell` writes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fell: Option<Spend>,
}

/// The accounts folder beside the state file.
pub struct Store {
    root: PathBuf,
}

fn bad(why: impl std::fmt::Display) -> RpcError {
    RpcError::new(ErrorCode::InvalidParams, why)
}

fn io(path: &Path) -> impl Fn(std::io::Error) -> RpcError {
    move |e| RpcError::new(ErrorCode::Io, format!("{}: {e}", path.display()))
}

fn valid(name: &str) -> bool {
    !name.is_empty()
        && (name.bytes()).all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

pub(crate) fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let temp = path.with_extension("new");
    let mut file = (OpenOptions::new().write(true).create(true).truncate(true))
        .mode(0o600)
        .open(&temp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(temp, path)
}

/// The record's `setup_files` that `folder` holds.
#[must_use]
pub fn setup(provider: &Provider, folder: &Path) -> Vec<String> {
    let held = |f: &&String| holds(&provider.setup_ignore, folder, f);
    provider.setup_files.iter().filter(held).cloned().collect()
}

fn holds(ignore: &[String], folder: &Path, rel: &str) -> bool {
    if ignore.iter().any(|i| i == rel) || fs::symlink_metadata(folder.join(rel)).is_err() {
        return false;
    }
    let under = format!("{rel}/");
    if !ignore.iter().any(|i| i.starts_with(&under)) {
        return true;
    }
    fs::read_dir(folder.join(rel)).map_or(true, |entries| {
        entries.flatten().any(|e| {
            let name = e.file_name();
            holds(
                ignore,
                folder,
                &format!("{under}{}", name.to_string_lossy()),
            )
        })
    })
}

fn line(provider: &Provider, name: &str, drift: Option<Drift>, status: String) -> String {
    let file = (provider.credentials.as_ref()).map_or("", |c| c.file.as_str());
    let p = &provider.agent;
    match drift {
        Some(Drift::Drifted) => {
            format!("keychain and {file} disagree: run shed account login {p} {name}")
        }
        Some(Drift::Unknown) => "keychain not readable: drift unknown".to_owned(),
        _ => status,
    }
}

/// The login terminal's name.
#[must_use]
pub fn login_terminal(provider: &str, name: &str) -> String {
    format!("login-{provider}-{name}")
}

impl Store {
    /// The store beside the state file `state`.
    #[must_use]
    pub fn beside(state: &Path) -> Self {
        Self {
            root: state.with_file_name("accounts"),
        }
    }

    fn entry(&self, provider: &Provider, name: &str) -> PathBuf {
        self.root.join(&provider.agent).join(name)
    }

    pub(crate) fn token_file(&self, provider: &Provider, name: &str) -> PathBuf {
        self.root
            .join(&provider.agent)
            .join(format!("{name}.token"))
    }

    fn readings_file(&self) -> PathBuf {
        self.root.join("accounts.json")
    }

    fn readings(&self) -> BTreeMap<String, Reading> {
        let text = fs::read_to_string(self.readings_file()).unwrap_or_default();
        serde_json::from_str(&text).unwrap_or_default()
    }

    fn update(&self, f: impl FnOnce(&mut BTreeMap<String, Reading>)) -> Result<(), RpcError> {
        let _held = KEEPING.lock().unwrap_or_else(PoisonError::into_inner);
        let mut all = self.readings();
        f(&mut all);
        let file = self.readings_file();
        let json = serde_json::to_vec(&all).map_err(|e| RpcError::new(ErrorCode::Internal, e))?;
        Self::made(&self.root)?;
        write_private(&file, &json).map_err(io(&file))
    }

    fn made(dir: &Path) -> Result<(), RpcError> {
        (DirBuilder::new().recursive(true).mode(0o700))
            .create(dir)
            .map_err(io(dir))
    }

    /// The account's folder: none for `default` or an account that is only a key.
    ///
    /// # Errors
    /// `NO_ACCOUNT` when the provider has no such account.
    pub fn folder(&self, provider: &Provider, name: &str) -> Result<Option<PathBuf>, RpcError> {
        let key_only = provider.config_dir_var.is_none();
        if name == DEFAULT && !key_only {
            return Ok(None);
        }
        let entry = if key_only {
            self.token_file(provider, name)
        } else {
            self.entry(provider, name)
        };
        if !valid(name) || fs::symlink_metadata(&entry).is_err() {
            let why = format!("{} has no account {name}", provider.agent);
            return Err(RpcError::new(ErrorCode::NoAccount, why));
        }
        Ok((!key_only).then_some(fs::read_link(&entry).unwrap_or(entry)))
    }

    /// Makes the account, or adopts `p.folder` by link; the same call twice is one account. A provider with
    /// no config folder variable has accounts that are only a key: `p.token_file` and no folder.
    ///
    /// # Errors
    /// `INVALID_PARAMS` for a bad name or folder, `ACCOUNT_SETUP` for a folder holding a setup, `IO`.
    pub fn add(&self, provider: &Provider, p: &AccountAdd) -> Result<Account, RpcError> {
        if !valid(&p.name) || p.name == DEFAULT {
            return Err(bad("name must match [a-z0-9-]+ and not be default"));
        }
        let key_only = provider.config_dir_var.is_none();
        if key_only && (p.folder.is_some() || p.token_file.is_none()) {
            let why = format!(
                "a {} account is only a key: a token file, no folder",
                provider.agent
            );
            return Err(bad(why));
        }
        let entry = self.entry(provider, &p.name);
        Self::made(entry.parent().unwrap_or(&self.root))?;
        if fs::symlink_metadata(&entry).is_ok() {
            if fs::read_link(&entry).ok() != p.folder {
                let why = format!("{}/{} is another folder", provider.agent, p.name);
                return Err(bad(why));
            }
        } else if let Some(folder) = &p.folder {
            if !folder.is_absolute() || !folder.is_dir() {
                return Err(bad(format!("{} is no folder", folder.display())));
            }
            if let Some(file) = setup(provider, folder).first() {
                let why = format!("{} holds {file}: a setup, not a login", folder.display());
                return Err(RpcError::new(ErrorCode::AccountSetup, why));
            }
            std::os::unix::fs::symlink(folder, &entry).map_err(io(&entry))?;
        } else if !key_only {
            Self::made(&entry)?;
        }
        if let Some(file) = &p.token_file {
            let path = self.token_file(provider, &p.name);
            Token::read(file)?.save(&path).map_err(io(&path))?;
        }
        self.account(provider, &p.name)
    }

    /// Saves `token` as the account's setup token (0600).
    ///
    /// # Errors
    /// `NO_ACCOUNT`, or `IO` when the file cannot be written.
    pub fn save_token(
        &self,
        provider: &Provider,
        name: &str,
        token: &Token,
    ) -> Result<(), RpcError> {
        self.folder(provider, name)?;
        let path = self.token_file(provider, name);
        token.save(&path).map_err(io(&path))
    }

    /// The account as shed last read it.
    ///
    /// # Errors
    /// `NO_ACCOUNT` when the provider has no such account.
    pub fn account(&self, provider: &Provider, name: &str) -> Result<Account, RpcError> {
        let folder = self.folder(provider, name)?;
        let key = format!("{}/{name}", provider.agent);
        let reading = self.readings().remove(&key).unwrap_or_default();
        let days = provider.token.as_ref().map(|t| t.days);
        let token = token::info(&self.token_file(provider, name), days).map(|t| TokenInfo {
            rejected: reading.rejected == Some(t.saved_at),
            ..t
        });
        let limited_until = (reading.limited_until).filter(|t| *t > shed_core::clock::unix_now());
        let drifted = reading.drift == Some(Drift::Drifted);
        let state = match reading.state {
            _ if drifted => AccountState::Drifted,
            _ if limited_until.is_some() => AccountState::Limited,
            AccountState::SignedOut if token.is_some_and(|t| !t.rejected) => AccountState::Ready,
            state => state,
        };
        Ok(Account {
            provider: provider.agent.clone(),
            name: name.to_owned(),
            adopted: fs::read_link(self.entry(provider, name)).is_ok(),
            setup: folder
                .as_deref()
                .map(|f| setup(provider, f))
                .unwrap_or_default(),
            folder,
            state,
            who: reading.who,
            line: line(provider, name, reading.drift, reading.line),
            drift: reading.drift,
            checked_at: reading.checked_at,
            limited_until,
            usage: reading.usage,
            balance: reading.balance,
            spend: Spend::of_month(reading.spend, reading.fell),
            token,
            runs: 0,
            login: None,
            preferred: reading.preferred,
        })
    }

    /// The provider's accounts, `default` first, then by name.
    #[must_use]
    pub fn ls(&self, provider: &Provider) -> Vec<Account> {
        let dir = self.root.join(&provider.agent);
        let mut names: Vec<String> = (fs::read_dir(dir).into_iter().flatten().flatten())
            .filter_map(|e| e.file_name().into_string().ok())
            .map(|n| {
                n.strip_suffix(".token")
                    .map_or_else(|| n.clone(), str::to_owned)
            })
            .filter(|n| valid(n) && n != DEFAULT)
            .collect();
        names.sort();
        names.dedup();
        let default = (provider.config_dir_var.is_some()).then(|| DEFAULT.to_owned());
        let names = default.into_iter().chain(names);
        names
            .filter_map(|n| self.account(provider, &n).ok())
            .collect()
    }

    /// The account as shed last read it, kept.
    ///
    /// # Errors
    /// `IO` when `accounts.json` cannot be written.
    pub fn keep(&self, provider: &Provider, name: &str, reading: Reading) -> Result<(), RpcError> {
        let key = format!("{}/{name}", provider.agent);
        self.update(|all| {
            let was = all.get(&key);
            let spend = was.and_then(|r| r.spend.clone());
            let fell = was.and_then(|r| r.fell.clone());
            all.insert(
                key,
                Reading {
                    spend,
                    fell,
                    ..reading
                },
            );
        })
    }

    /// Adds `tokens` costing `cost` to the account's totals for `month`; another month starts them again.
    ///
    /// # Errors
    /// `IO` when `accounts.json` cannot be written.
    pub fn spent(
        &self,
        provider: &Provider,
        name: &str,
        month: &str,
        tokens: Tokens,
        cost: Option<Money>,
    ) -> Result<(), RpcError> {
        self.total(provider, name, month, tokens, cost, SpendSource::Usage)
    }

    /// Adds a drop of the account's balance to its drops for `month`; another month starts them again.
    ///
    /// # Errors
    /// `IO` when `accounts.json` cannot be written.
    pub fn fell(
        &self,
        provider: &Provider,
        name: &str,
        month: &str,
        drop: Money,
    ) -> Result<(), RpcError> {
        let source = SpendSource::Balance;
        self.total(provider, name, month, Tokens::default(), Some(drop), source)
    }

    fn total(
        &self,
        provider: &Provider,
        name: &str,
        month: &str,
        tokens: Tokens,
        cost: Option<Money>,
        source: SpendSource,
    ) -> Result<(), RpcError> {
        self.folder(provider, name)?;
        let key = format!("{}/{name}", provider.agent);
        self.update(|all| {
            let reading = all.entry(key).or_default();
            let slot = match source {
                SpendSource::Usage => &mut reading.spend,
                SpendSource::Balance => &mut reading.fell,
            };
            *slot = Some(match slot.take() {
                Some(s) => s.plus(month, tokens, cost),
                None => Spend::new(month, tokens, cost, source),
            });
        })
    }

    /// Marks the account's token refused by the proxy's upstream, until a new one is saved; whether it was not
    /// marked yet.
    ///
    /// # Errors
    /// `IO` when `accounts.json` cannot be written.
    pub fn reject(&self, provider: &Provider, name: &str) -> Result<bool, RpcError> {
        let Some(saved_at) =
            token::info(&self.token_file(provider, name), None).map(|t| t.saved_at)
        else {
            return Ok(false);
        };
        let key = format!("{}/{name}", provider.agent);
        let mut fresh = false;
        self.update(|all| {
            let reading = all.entry(key).or_default();
            fresh = reading.rejected != Some(saved_at);
            reading.rejected = Some(saved_at);
        })?;
        Ok(fresh)
    }

    /// Makes `name` the provider's one preferred account; none drops the preference.
    ///
    /// # Errors
    /// `IO` when `accounts.json` cannot be written.
    pub fn prefer(&self, provider: &Provider, name: Option<&str>) -> Result<(), RpcError> {
        let ours = format!("{}/", provider.agent);
        self.update(|all| {
            for (key, reading) in all.iter_mut() {
                reading.preferred &= !key.starts_with(&ours);
            }
            if let Some(name) = name {
                all.entry(format!("{ours}{name}")).or_default().preferred = true;
            }
        })
    }

    /// What shed last read of the account.
    #[must_use]
    pub fn reading(&self, provider: &Provider, name: &str) -> Reading {
        let key = format!("{}/{name}", provider.agent);
        self.readings().remove(&key).unwrap_or_default()
    }

    /// Removes the account's folder (an adopted one: its link), its token and its reading.
    ///
    /// # Errors
    /// `INVALID_PARAMS` for `default`, `NO_ACCOUNT`, `IO`.
    pub fn remove(&self, provider: &Provider, name: &str) -> Result<(), RpcError> {
        if name == DEFAULT {
            return Err(bad(
                "default is the CLI's own folder; shed does not remove it",
            ));
        }
        self.folder(provider, name)?;
        let entry = self.entry(provider, name);
        let gone = if provider.config_dir_var.is_none() {
            Ok(())
        } else if fs::read_link(&entry).is_ok() {
            fs::remove_file(&entry)
        } else {
            fs::remove_dir_all(&entry)
        };
        gone.map_err(io(&entry))?;
        fs::remove_file(self.token_file(provider, name)).ok();
        let key = format!("{}/{name}", provider.agent);
        self.update(|all| {
            all.remove(&key);
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt as _;
    use std::path::{Path, PathBuf};

    use shed_proto::{AccountAdd, AccountState, ErrorCode, Provider};

    use super::Store;

    pub(crate) struct Temp(PathBuf);

    impl Temp {
        pub(crate) fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).ok();
        }
    }

    pub(crate) fn temp(name: &str) -> Temp {
        let pid = shed_core::runner::pid();
        let dir = std::env::temp_dir().join(format!("shed-accounts-{name}-{pid}"));
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(&dir).expect("temp dir");
        Temp(dir)
    }

    pub(crate) fn provider() -> Provider {
        Provider {
            agent: "fake".into(),
            config_dir_var: Some("FAKE_HOME".into()),
            setup_files: vec!["settings.json".into(), "skills".into()],
            ..Provider::default()
        }
    }

    fn add(name: &str, folder: Option<&Path>) -> AccountAdd {
        AccountAdd {
            provider: "fake".into(),
            name: name.into(),
            folder: folder.map(Path::to_path_buf),
            token_file: None,
        }
    }

    #[test]
    fn add_makes_an_empty_private_folder_once() {
        let dir = temp("store-add");
        let store = Store::beside(&dir.path().join("state.json"));
        let p = provider();
        let first = store.add(&p, &add("work", None)).expect("added");
        let again = store.add(&p, &add("work", None)).expect("added twice");
        assert_eq!(first, again);
        let folder = first.folder.expect("a folder");
        assert_eq!(folder, dir.path().join("accounts/fake/work"));
        let mode = fs::metadata(&folder).expect("meta").permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
        assert_eq!(first.state, AccountState::SignedOut);
        let names: Vec<_> = store.ls(&p).into_iter().map(|a| a.name).collect();
        assert_eq!(names, ["default", "work"]);
        assert!(store.add(&p, &add("Work", None)).is_err());
        assert!(store.add(&p, &add("default", None)).is_err());
    }

    #[test]
    fn one_account_is_preferred_and_none_clears_it() {
        let dir = temp("store-prefer");
        let store = Store::beside(&dir.path().join("state.json"));
        let p = provider();
        for name in ["a", "b"] {
            store.add(&p, &add(name, None)).expect("added");
        }
        let preferred = |store: &Store| -> Vec<_> {
            let held = store.ls(&p).into_iter().filter(|a| a.preferred);
            held.map(|a| a.name).collect()
        };
        store.prefer(&p, Some("a")).expect("a");
        store.prefer(&p, Some("b")).expect("b");
        assert_eq!(preferred(&store), ["b"]);
        store.prefer(&p, None).expect("cleared");
        assert!(preferred(&store).is_empty());
    }

    #[test]
    fn adopting_a_folder_links_it_and_remove_keeps_it() {
        let dir = temp("store-adopt");
        let store = Store::beside(&dir.path().join("state.json"));
        let p = provider();
        let theirs = dir.path().join("theirs");
        fs::create_dir(&theirs).expect("mkdir");
        fs::write(theirs.join(".credentials.json"), "{}").expect("write");
        let a = store
            .add(&p, &add("cooee", Some(&theirs)))
            .expect("adopted");
        assert!(a.adopted);
        assert_eq!(a.folder.as_deref(), Some(theirs.as_path()));
        store
            .add(&p, &add("cooee", Some(&theirs)))
            .expect("adopted twice");
        assert!(store.add(&p, &add("cooee", None)).is_err());
        store.remove(&p, "cooee").expect("removed");
        assert!(theirs.join(".credentials.json").exists());
        let gone = store.account(&p, "cooee").expect_err("gone");
        assert_eq!(gone.code, ErrorCode::NoAccount.code());
    }

    #[test]
    fn a_folder_holding_a_setup_is_refused_account_setup() {
        let dir = temp("store-setup");
        let store = Store::beside(&dir.path().join("state.json"));
        let p = provider();
        let owners = dir.path().join("owners");
        fs::create_dir_all(owners.join("skills")).expect("mkdir");
        let refused = store
            .add(&p, &add("mine", Some(&owners)))
            .expect_err("refused");
        assert_eq!(refused.code, ErrorCode::AccountSetup.code());
        assert!(refused.message.contains("skills"), "{}", refused.message);
        assert!(store.account(&p, "mine").is_err());
        let made = store.add(&p, &add("made", None)).expect("added");
        let folder = made.folder.expect("folder");
        fs::write(folder.join("settings.json"), "{}").expect("write");
        assert_eq!(
            store.account(&p, "made").expect("read").setup,
            ["settings.json"]
        );
    }

    #[test]
    fn what_the_agent_writes_itself_is_no_setup() {
        let dir = temp("store-ignore");
        let store = Store::beside(&dir.path().join("state.json"));
        let mut p = provider();
        p.setup_files = vec!["skills".into(), "plugins".into()];
        p.setup_ignore = ["skills/synced", "plugins/store", "plugins/synced"]
            .map(String::from)
            .to_vec();
        let folder = store.add(&p, &add("made", None)).expect("added").folder;
        let folder = folder.expect("folder");
        for d in ["skills/synced/org/s", "plugins/store", "plugins/synced/b"] {
            fs::create_dir_all(folder.join(d)).expect("mkdir");
        }
        fs::write(folder.join("plugins/store/p.json"), "{}").expect("write");
        let setup = || store.account(&p, "made").expect("read").setup;
        assert!(setup().is_empty(), "{:?}", setup());
        fs::create_dir_all(folder.join("skills/my-skill")).expect("mkdir");
        fs::write(folder.join("skills/my-skill/SKILL.md"), "x").expect("write");
        assert_eq!(setup(), ["skills"]);
        fs::write(folder.join("plugins/installed_plugins.json"), "{}").expect("write");
        assert_eq!(setup(), ["skills", "plugins"]);
    }
}
