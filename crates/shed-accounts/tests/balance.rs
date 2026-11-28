//! The balance probe against a fake provider: an account that is only a key is read with its key in the auth
//! header, the reading lands on the account, the key appears in no output, and a 401 marks the key rejected
//! and stops the probe until a new key is saved; the month's cost is the balance's drops while the lanes report
//! no tokens.
#![cfg(test)]

use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use shed_accounts::{Store, check};
use shed_proto::{AccountAdd, AccountState, BalanceProbe, ErrorCode, Money, Provider, Tokens};

const KEY: &str = "sk-fake-balance-0123456789abcdef";

struct Temp(PathBuf);

impl Drop for Temp {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

fn temp(name: &str) -> Temp {
    let dir = std::env::temp_dir().join(format!("shed-balance-{name}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).expect("temp dir");
    Temp(dir)
}

type Seen = Arc<Mutex<Vec<String>>>;

type Answer = Arc<Mutex<&'static str>>;

fn provider_api(status: Answer, total: Answer) -> (u16, Seen) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bound");
    let port = listener.local_addr().expect("addr").port();
    let seen = Seen::default();
    let kept = Arc::clone(&seen);
    std::thread::spawn(move || {
        for mut conn in listener.incoming().map_while(Result::ok) {
            let mut reader = BufReader::new(conn.try_clone().expect("clone"));
            let mut head = String::new();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).expect("a line");
                if line.trim().is_empty() {
                    break;
                }
                head += &line;
            }
            kept.lock().expect("seen").push(head);
            let total = *total.lock().expect("total");
            let body = format!(
                r#"{{"is_available":true,"balance_infos":[{{"currency":"CNY","total_balance":"{total}"}}]}}"#
            );
            let status = *status.lock().expect("status");
            let answer = format!(
                "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            conn.write_all(answer.as_bytes()).expect("answered");
        }
    });
    (port, seen)
}

fn record(port: u16) -> Provider {
    Provider {
        agent: "fake".into(),
        token_var: Some("FAKE_API_KEY".into()),
        balance: Some(Box::new(BalanceProbe {
            url: format!("http://127.0.0.1:{port}/user/balance"),
            auth: "Authorization: Bearer {token}".into(),
            currency: vec!["balance_infos".into(), 0.into(), "currency".into()],
            total: vec!["balance_infos".into(), 0.into(), "total_balance".into()],
            fresh: 600,
        })),
        ..Provider::default()
    }
}

fn add(name: &str, token_file: Option<PathBuf>, folder: Option<PathBuf>) -> AccountAdd {
    AccountAdd {
        provider: "fake".into(),
        name: name.into(),
        folder,
        token_file,
    }
}

#[tokio::test]
async fn a_key_only_account_reads_its_balance_and_shows_no_key() {
    let dir = temp("read");
    let status = Arc::new(Mutex::new("200 OK"));
    let (port, seen) = provider_api(Arc::clone(&status), Arc::new(Mutex::new("42.50")));
    let provider = record(port);
    let store = Store::beside(&dir.0.join("state.json"));
    let file = dir.0.join("work.token");
    std::fs::write(&file, KEY).expect("write");
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).expect("chmod");

    for refused in [
        add("work", None, None),
        add("work", Some(file.clone()), Some(dir.0.clone())),
    ] {
        let err = store.add(&provider, &refused).expect_err("only a key");
        assert_eq!(err.code, ErrorCode::InvalidParams.code(), "{}", err.message);
    }
    let added = store
        .add(&provider, &add("work", Some(file), None))
        .expect("added");
    assert_eq!(
        (added.folder.clone(), added.state),
        (None, AccountState::Ready)
    );
    assert!(!dir.0.join("accounts/fake/work").exists());
    assert!(check::stale(&provider, &added, 0));

    let read = check::status(&store, &provider, "work", "/".as_ref())
        .await
        .expect("status");
    let balance = read.balance.clone().expect("a balance");
    assert_eq!((balance.currency.as_str(), balance.total), ("CNY", 42.5));
    assert_eq!(Some(balance.read_at), read.checked_at);
    assert!(!check::stale(&provider, &read, balance.read_at + 599));
    assert!(check::stale(&provider, &read, balance.read_at + 600));
    let up = seen.lock().expect("seen").clone();
    assert_eq!(up.len(), 1);
    assert!(up[0].starts_with("GET /user/balance"), "{}", up[0]);
    assert!(
        up[0].contains(&format!("Authorization: Bearer {KEY}")),
        "{}",
        up[0]
    );

    let listed = store.ls(&provider);
    assert_eq!(listed, vec![read]);
    let shown = serde_json::to_string(&listed).expect("json");
    let kept = std::fs::read_to_string(dir.0.join("accounts/accounts.json")).expect("kept");
    assert!(
        shown.contains("42.5") && kept.contains("42.5"),
        "{shown} {kept}"
    );
    assert!(
        !shown.contains(KEY) && !kept.contains(KEY),
        "{shown} {kept}"
    );

    *status.lock().expect("status") = "401 Unauthorized";
    let refused = check::status(&store, &provider, "work", "/".as_ref())
        .await
        .expect("status");
    assert!(refused.token.is_some_and(|t| t.rejected), "{refused:?}");
    assert_eq!(refused.state, AccountState::SignedOut);
    assert_eq!(refused.balance.map(|b| b.total), Some(42.5));
    check::status(&store, &provider, "work", "/".as_ref())
        .await
        .expect("status");
    assert_eq!(
        seen.lock().expect("seen").len(),
        2,
        "a rejected key is not sent again"
    );

    store.remove(&provider, "work").expect("removed");
    assert!(store.ls(&provider).is_empty());
}

fn key_account(dir: &Temp, provider: &Provider) -> Store {
    let store = Store::beside(&dir.0.join("state.json"));
    let file = dir.0.join("work.token");
    std::fs::write(&file, KEY).expect("write");
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).expect("chmod");
    store
        .add(provider, &add("work", Some(file), None))
        .expect("added");
    store
}

#[tokio::test]
async fn with_no_tokens_reported_the_months_cost_is_the_balances_drops() {
    let dir = temp("drop");
    let total = Arc::new(Mutex::new("5.57"));
    let (port, _) = provider_api(Arc::new(Mutex::new("200 OK")), Arc::clone(&total));
    let provider = record(port);
    let store = key_account(&dir, &provider);
    let status = || check::status(&store, &provider, "work", "/".as_ref());
    let cost = |a: &shed_proto::Account| {
        let spend = serde_json::to_value(&a.spend).expect("json");
        format!("{:.2}", spend["cost"]["amount"].as_f64().unwrap_or(-1.0))
    };
    let source = |a: &shed_proto::Account| a.spend.as_ref().map(|s| s.source);
    let month = check::month();
    assert_eq!(status().await.expect("status").spend, None);
    let own = Money {
        amount: 0.15,
        currency: "CNY".into(),
    };
    let none = Tokens::default();
    store
        .spent(&provider, "work", &month, none, Some(own))
        .expect("spent");
    for (now, want) in [
        ("5.37", "0.20"),
        ("5.37", "0.20"),
        ("6.00", "0.20"),
        ("5.70", "0.50"),
    ] {
        *total.lock().expect("total") = now;
        let read = status().await.expect("status");
        assert_eq!(cost(&read), want, "at {now}");
        assert_eq!(source(&read), Some(shed_proto::SpendSource::Balance));
    }
    let used = Tokens {
        input: 1000,
        ..Tokens::default()
    };
    store
        .spent(&provider, "work", &month, used, None)
        .expect("spent");
    let read = store.account(&provider, "work").expect("account");
    assert_eq!(read.spend.as_ref().map(|s| s.tokens), Some(used));
    assert_eq!(source(&read), Some(shed_proto::SpendSource::Usage));
}
