use std::os::unix::fs::PermissionsExt as _;

use shed_client::Calls as _;
use shed_server::proto::{Account, AccountState};

use crate::service::{Checks, Row};

const DAY: u64 = 86_400;

fn token(cx: &Checks<'_>, a: &Account, now: u64) -> Option<Row> {
    let t = a.token?;
    let accounts = cx.paths.state.with_file_name("accounts");
    let file = accounts.join(&a.provider).join(format!("{}.token", a.name));
    let mode = std::fs::metadata(&file).map(|m| m.permissions().mode() & 0o777);
    let age = now.saturating_sub(t.saved_at) / DAY;
    let text = match (mode, t.expires_at) {
        (Ok(mode), _) if mode & 0o077 != 0 => Err(format!(
            "{} is mode {mode:o}: not read; fix: chmod 600 {}",
            file.display(),
            file.display()
        )),
        (_, Some(at)) if at <= now => Err(format!("{} expired; save a new one", file.display())),
        (_, Some(at)) => Ok(format!("{} saved {age} days ago, {} days left", file.display(), (at - now) / DAY)),
        (_, None) => Ok(format!("{} saved {age} days ago", file.display())),
    };
    Some(Row::of("token", text))
}

pub(super) fn rows(cx: &Checks<'_>) -> Vec<Row> {
    let Ok(server) = &cx.server else {
        return Vec::new();
    };
    let Ok(accounts) = server.accounts(None) else {
        return Vec::new();
    };
    let now = shed_server::clock::unix_now();
    let mut rows = Vec::new();
    for a in accounts.iter().filter(|a| a.folder.is_some()) {
        let (p, n) = (&a.provider, &a.name);
        if let Some(file) = a.setup.first() {
            let why = format!("{p}/{n} holds {file}, a setup: no run on it; move it out");
            rows.push(Row::of("account", Err(why)));
        } else if a.state == AccountState::SignedOut {
            let why = format!("{p}/{n} signed out; fix: shed account login {p} {n}");
            rows.push(Row::of("account", Err(why)));
        }
        rows.extend(token(cx, a, now));
    }
    rows
}
