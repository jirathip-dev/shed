//! The accounts on disk: `accounts/<provider>/<name>` folders (or links to adopted ones), their `.token` files,
//! and `accounts.json`, what shed last read of each; every folder 0700, every file 0600.

use std::collections::BTreeMap;
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::Write as _;
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use serde::{Deserialize, Serialize};
use shed_proto::{Account, AccountAdd, AccountState, ErrorCode, Provider, RpcError, Usage};

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
    /// When its last limit ends.
    #[serde(default)]
    pub limited_until: Option<u64>,
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
    let held = |f: &&String| fs::symlink_metadata(folder.join(f)).is_ok();
    provider.setup_files.iter().filter(held).cloned().collect()
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

    /// The account's folder: none for `default`.
    ///
    /// # Errors
    /// `NO_ACCOUNT` when the provider has no such account.
    pub fn folder(&self, provider: &Provider, name: &str) -> Result<Option<PathBuf>, RpcError> {
        if name == DEFAULT {
            return Ok(None);
        }
        let entry = self.entry(provider, name);
        if !valid(name) || fs::symlink_metadata(&entry).is_err() {
            let why = format!("{} has no account {name}", provider.agent);
            return Err(RpcError::new(ErrorCode::NoAccount, why));
        }
        Ok(Some(fs::read_link(&entry).unwrap_or(entry)))
    }

    /// Makes the account, or adopts `p.folder` by link; the same call twice is one account.
    ///
    /// # Errors
    /// `INVALID_PARAMS` for a bad name or folder, `ACCOUNT_SETUP` for a folder holding a setup, `IO`.
    pub fn add(&self, provider: &Provider, p: &AccountAdd) -> Result<Account, RpcError> {
        if !valid(&p.name) || p.name == DEFAULT {
            return Err(bad("name must match [a-z0-9-]+ and not be default"));
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
        } else {
            Self::made(&entry)?;
        }
        if let Some(file) = &p.token_file {
            let path = self.token_file(provider, &p.name);
            Token::read(file)?.save(&path).map_err(io(&path))?;
        }
        self.account(provider, &p.name)
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
        let token = token::info(&self.token_file(provider, name), days);
        let limited_until = (reading.limited_until).filter(|t| *t > shed_core::clock::unix_now());
        let state = match reading.state {
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
            line: reading.line,
            checked_at: reading.checked_at,
            limited_until,
            usage: reading.usage,
            token,
            runs: 0,
            login: None,
        })
    }

    /// The provider's accounts, `default` first, then by name.
    #[must_use]
    pub fn ls(&self, provider: &Provider) -> Vec<Account> {
        let dir = self.root.join(&provider.agent);
        let mut names: Vec<String> = (fs::read_dir(dir).into_iter().flatten().flatten())
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| valid(n))
            .collect();
        names.sort();
        let names = std::iter::once(DEFAULT.to_owned()).chain(names);
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
            all.insert(key, reading);
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
        let gone = if fs::read_link(&entry).is_ok() {
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
}
