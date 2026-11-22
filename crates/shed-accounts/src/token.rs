//! A setup token: read only from a file of mode 0600, saved only at that mode, and never shown.

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::time::UNIX_EPOCH;

use shed_proto::{ErrorCode, RpcError, TokenInfo};

use crate::store::write_private;

const DAY: u64 = 86_400;

/// A setup token, which has no `Debug`, `Display`, `Serialize` or `Clone`, so nothing prints, sends or copies it:
///
/// ```compile_fail,E0277
/// fn shown(t: &shed_accounts::Token) -> String { format!("{t:?}") }
/// ```
///
/// ```compile_fail,E0277
/// fn shown(t: &shed_accounts::Token) -> String { format!("{t}") }
/// ```
///
/// ```compile_fail,E0277
/// fn sent<T: serde::Serialize>(_: &T) {}
/// fn token(t: &shed_accounts::Token) { sent(t) }
/// ```
///
/// ```compile_fail,E0277
/// fn copied<T: Clone>() {}
/// copied::<shed_accounts::Token>();
/// ```
pub struct Token(String);

impl Token {
    /// The token in `path`, refused when the file is open to anyone but its owner.
    ///
    /// # Errors
    /// `INVALID_PARAMS` when the file is wider than 0600 or empty, `IO` when it cannot be read.
    pub fn read(path: &Path) -> Result<Self, RpcError> {
        let io =
            |e: std::io::Error| RpcError::new(ErrorCode::Io, format!("{}: {e}", path.display()));
        let mode = fs::metadata(path).map_err(io)?.permissions().mode();
        if mode & 0o077 != 0 {
            let why = format!(
                "{} is mode {:o}, wider than 0600: not read; chmod 600 it",
                path.display(),
                mode & 0o777
            );
            return Err(RpcError::new(ErrorCode::InvalidParams, why));
        }
        let text = fs::read_to_string(path).map_err(io)?;
        let token = text.trim();
        if token.is_empty() {
            let why = format!("{} holds no token", path.display());
            return Err(RpcError::new(ErrorCode::InvalidParams, why));
        }
        Ok(Self(token.to_owned()))
    }

    pub(crate) fn expose(&self) -> &str {
        &self.0
    }

    pub(crate) fn save(&self, path: &Path) -> std::io::Result<()> {
        write_private(path, self.0.as_bytes())
    }
}

/// The dates of the token file at `path`, which lasts `days`; none without one.
#[must_use]
pub fn info(path: &Path, days: Option<u64>) -> Option<TokenInfo> {
    let modified = fs::metadata(path).and_then(|m| m.modified()).ok()?;
    let saved_at = modified.duration_since(UNIX_EPOCH).ok()?.as_secs();
    Some(TokenInfo {
        saved_at,
        expires_at: days.map(|d| saved_at + d * DAY),
        rejected: false,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt as _;

    use super::Token;
    use crate::store::tests::temp;

    #[test]
    fn a_token_file_wider_than_0600_is_not_read() {
        let dir = temp("token-mode");
        let path = dir.path().join("t.token");
        fs::write(&path, "sk-test-0123456789abcdef\n").expect("write");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("chmod");
        let refused = Token::read(&path).map(|_| ()).expect_err("refused");
        assert!(refused.message.contains("chmod 600"), "{}", refused.message);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("chmod");
        let token = Token::read(&path).expect("read at 0600");
        let copy = dir.path().join("copy.token");
        token.save(&copy).expect("saved");
        let mode = fs::metadata(&copy).expect("meta").permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(
            fs::read_to_string(&copy).expect("read"),
            "sk-test-0123456789abcdef"
        );
    }
}
