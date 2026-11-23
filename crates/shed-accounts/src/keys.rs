//! The token proxy's per-run keys: `shk-` and 48 hex digits, made by `run` for one terminal on another
//! machine; `proxy-keys.json` (0600) holds each one's sha256, never the key.

use std::fmt::Write as _;
use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use shed_proto::{ErrorCode, KeyState, ProxyKey, RpcError};

use crate::store::write_private;

static KEEPING: Mutex<()> = Mutex::new(());

#[derive(Serialize, Deserialize)]
struct Kept {
    sha: String,
    #[serde(default)]
    auto: bool,
    #[serde(flatten)]
    key: ProxyKey,
}

/// The keys file beside the state file.
pub struct Keys {
    file: PathBuf,
}

/// `bytes` random bytes from `/dev/urandom`, as hex.
///
/// # Errors
/// `IO` when `/dev/urandom` cannot be read.
pub fn random_hex(bytes: usize) -> Result<String, RpcError> {
    let mut buf = vec![0; bytes];
    let read = fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut buf));
    read.map_err(|e| RpcError::new(ErrorCode::Io, format!("/dev/urandom: {e}")))?;
    Ok(hex(&buf))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut s, b| {
        write!(s, "{b:02x}").ok();
        s
    })
}

fn sha(key: &str) -> String {
    hex(&Sha256::digest(key.as_bytes()))
}

impl Keys {
    /// The keys beside the state file `state`.
    #[must_use]
    pub fn beside(state: &Path) -> Self {
        Self {
            file: state.with_file_name("proxy-keys.json"),
        }
    }

    fn all(&self) -> Vec<Kept> {
        let text = fs::read_to_string(&self.file).unwrap_or_default();
        serde_json::from_str(&text).unwrap_or_default()
    }

    fn update<T>(&self, f: impl FnOnce(&mut Vec<Kept>) -> T) -> Result<T, RpcError> {
        let _held = KEEPING.lock().unwrap_or_else(PoisonError::into_inner);
        let mut all = self.all();
        let out = f(&mut all);
        let json = serde_json::to_vec(&all).map_err(|e| RpcError::new(ErrorCode::Internal, e))?;
        let io = |e: std::io::Error| RpcError::new(ErrorCode::Io, e);
        write_private(&self.file, &json).map_err(io)?;
        Ok(out)
    }

    /// A new live key for `terminal` on `provider`'s `account`, `auto` when the run may switch: the key and
    /// what is kept of it.
    ///
    /// # Errors
    /// `IO` when no random bytes can be read or the file cannot be written.
    pub fn mint(
        &self,
        terminal: &str,
        (provider, account): (&str, &str),
        auto: bool,
    ) -> Result<(String, ProxyKey), RpcError> {
        let secret = format!("shk-{}", random_hex(24)?);
        let sha = sha(&secret);
        let key = ProxyKey {
            id: sha.chars().take(8).collect(),
            terminal: terminal.to_owned(),
            provider: provider.to_owned(),
            account: account.to_owned(),
            created: shed_core::clock::unix_now(),
            state: KeyState::Live,
        };
        let kept = Kept {
            sha,
            auto,
            key: key.clone(),
        };
        self.update(|all| all.push(kept))?;
        Ok((secret, key))
    }

    /// Every key, newest first.
    #[must_use]
    pub fn ls(&self) -> Vec<ProxyKey> {
        self.all().into_iter().rev().map(|k| k.key).collect()
    }

    /// The live key `secret` is, compared by its sha256.
    #[must_use]
    pub fn live(&self, secret: &str) -> Option<ProxyKey> {
        let sha = sha(secret);
        let found = self.all().into_iter().find(|k| k.sha == sha)?.key;
        (found.state == KeyState::Live).then_some(found)
    }

    /// The live key of `terminal`, and whether its run is on `auto`.
    #[must_use]
    pub fn of(&self, terminal: &str) -> Option<(ProxyKey, bool)> {
        let live = |k: &Kept| k.key.terminal == terminal && k.key.state == KeyState::Live;
        self.all().into_iter().find(live).map(|k| (k.key, k.auto))
    }

    fn change(&self, pick: impl Fn(&ProxyKey) -> bool, f: impl Fn(&mut ProxyKey)) -> usize {
        let changed = self.update(|all| {
            let live = all.iter_mut().map(|k| &mut k.key);
            let live = live.filter(|k| k.state == KeyState::Live && pick(k));
            live.map(f).count()
        });
        changed.unwrap_or_default()
    }

    /// Stops key `id` signing at once.
    ///
    /// # Errors
    /// `INVALID_PARAMS` when no live key has that id.
    pub fn revoke(&self, id: &str) -> Result<(), RpcError> {
        match self.change(|k| k.id == id, |k| k.state = KeyState::Revoked) {
            0 => Err(RpcError::new(
                ErrorCode::InvalidParams,
                format!("no live key {id}"),
            )),
            _ => Ok(()),
        }
    }

    /// Ends the keys of `terminal`, which exited or was removed.
    pub fn end(&self, terminal: &str) {
        self.change(|k| k.terminal == terminal, |k| k.state = KeyState::Ended);
    }

    /// Key `id` was made for the terminal the run then named `terminal`.
    pub fn name(&self, id: &str, terminal: &str) {
        self.change(|k| k.id == id, |k| terminal.clone_into(&mut k.terminal));
    }

    /// `terminal`'s key now signs with `account`'s token.
    pub fn repoint(&self, terminal: &str, account: &str) {
        self.change(
            |k| k.terminal == terminal,
            |k| account.clone_into(&mut k.account),
        );
    }
}

#[cfg(test)]
mod tests {
    use shed_proto::KeyState;

    use super::Keys;
    use crate::store::tests::temp;

    #[test]
    fn only_a_key_s_sha256_is_kept_and_it_signs_until_revoked_or_its_terminal_ends() {
        let dir = temp("keys");
        let keys = Keys::beside(&dir.path().join("state.json"));
        let (a, kept) = keys.mint("box/a", ("fake", "work"), true).expect("minted");
        let (b, _) = keys.mint("box/b", ("fake", "work"), false).expect("minted");
        assert!(a.starts_with("shk-") && a.len() == 52, "{a}");
        let file = std::fs::read_to_string(dir.path().join("proxy-keys.json")).expect("file");
        assert!(!file.contains(&a) && !file.contains(&b), "{file}");
        assert!(file.contains(&kept.id));
        assert_eq!(keys.live(&a), Some(kept.clone()));
        keys.repoint("box/a", "spare");
        let (now, auto) = keys.of("box/a").expect("a's key");
        assert_eq!((now.account.as_str(), auto), ("spare", true));
        keys.end("box/a");
        assert_eq!(keys.live(&a), None);
        assert!(keys.live(&b).is_some());
        keys.revoke(&keys.ls()[0].id).expect("revoked");
        assert_eq!(keys.live(&b), None);
        let states: Vec<_> = keys.ls().into_iter().map(|k| k.state).collect();
        assert_eq!(states, [KeyState::Revoked, KeyState::Ended]);
        assert!(keys.revoke(&kept.id).is_err(), "an ended key is not live");
        assert_eq!(keys.live("shk-nope"), None);
    }
}
