use serde_json::{Value, json, to_value};
use shed_proto::{
    Account, AccountAdd, AccountLogin, AccountMoves, AccountName, AccountState, AccountUse,
    AccountsParams, KeyId, KeyState, Login, Method, ProxyKey, Relay, Stayed, TokenInfo, Usage,
    Window,
};

fn work() -> Account {
    Account {
        provider: "claude".into(),
        name: "work".into(),
        folder: Some("/home/user/.local/state/shed/accounts/claude/work".into()),
        adopted: false,
        state: AccountState::Ready,
        who: Some("dev@example.com".into()),
        line: "signed in · claude.ai (dev@example.com)".into(),
        drift: None,
        checked_at: Some(1_762_416_000),
        limited_until: None,
        usage: Some(Usage {
            windows: vec![Window {
                id: "5h".into(),
                used_pct: 12,
                resets_at: Some(1_762_430_400),
            }],
            read_at: 1_762_416_000,
        }),
        token: Some(TokenInfo {
            saved_at: 1_762_000_000,
            expires_at: Some(1_793_536_000),
            rejected: false,
        }),
        setup: Vec::new(),
        runs: 1,
        login: None,
        preferred: true,
    }
}

fn named() -> Value {
    to_value(AccountName {
        provider: "claude".into(),
        name: "work".into(),
    })
    .unwrap()
}

pub(crate) fn example(method: Method) -> Option<(Value, Value)> {
    Some(match method {
        Method::AccountAdd => (
            to_value(AccountAdd {
                provider: "claude".into(),
                name: "work".into(),
                folder: None,
                token_file: Some("/home/user/claude-work.token".into()),
            })
            .unwrap(),
            to_value(work()).unwrap(),
        ),
        Method::AccountLs => (
            to_value(AccountsParams {
                provider: Some("claude".into()),
            })
            .unwrap(),
            to_value(vec![work()]).unwrap(),
        ),
        Method::AccountLogin => (
            to_value(AccountLogin {
                provider: "claude".into(),
                name: "work".into(),
                token: true,
            })
            .unwrap(),
            to_value(Login {
                terminal: "login-claude-work".into(),
            })
            .unwrap(),
        ),
        Method::AccountStatus => (named(), to_value(work()).unwrap()),
        Method::AccountRm => (named(), json!({})),
        Method::ProxyKeys => (
            json!({}),
            to_value(vec![ProxyKey {
                id: "3f9a1c2e".into(),
                terminal: "box/lane-256-impl".into(),
                provider: "claude".into(),
                account: "work".into(),
                created: 1_762_416_000,
                state: KeyState::Live,
            }])
            .unwrap(),
        ),
        Method::ProxyRevoke => (
            to_value(KeyId {
                id: "3f9a1c2e".into(),
            })
            .unwrap(),
            json!({}),
        ),
        Method::ProxyRelay => (
            json!({}),
            to_value(Relay {
                port: 41_234,
                socket: "/run/user/1000/up-5e0c9a1f.sock".into(),
            })
            .unwrap(),
        ),
        Method::AccountUse => (
            to_value(AccountUse {
                provider: "claude".into(),
                name: Some("work".into()),
            })
            .unwrap(),
            to_value(AccountMoves {
                moved: vec!["lane-1".into()],
                stayed: vec![Stayed {
                    terminal: "lane-2".into(),
                    why: "on named account home".into(),
                }],
            })
            .unwrap(),
        ),
        _ => return None,
    })
}
