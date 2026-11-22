//! The token proxy, moved from drover's: a live key's call goes upstream signed with the account's token and
//! none of the caller's auth headers; a path outside the record's `proxy.paths` is refused before the token
//! file opens; a revoked, ended or unknown key never reaches upstream; an upstream 401 marks the token
//! `rejected` once, and the proxy answers 401 itself until a new token is saved.
#![cfg(test)]

use std::collections::BTreeMap;
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use shed_accounts::Store;
use shed_accounts::keys::Keys;
use shed_accounts::proxy::Proxy;
use shed_proto::{AccountAdd, Provider, ProxyRecord};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{UnixListener, UnixStream};

const TOKEN: &str = "sk-fake-oat01-0123456789abcdef";

struct Temp(PathBuf);

impl Drop for Temp {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

fn temp(name: &str) -> Temp {
    let dir = std::env::temp_dir().join(format!("shed-proxy-{name}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).expect("temp dir");
    Temp(dir)
}

type Seen = Arc<Mutex<Vec<String>>>;

fn upstream(status: &'static str) -> (u16, Seen) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bound");
    let port = listener.local_addr().expect("addr").port();
    let seen = Seen::default();
    let kept = Arc::clone(&seen);
    std::thread::spawn(move || {
        for mut conn in listener.incoming().map_while(Result::ok) {
            let mut reader = BufReader::new(conn.try_clone().expect("clone"));
            let (mut head, mut length) = (String::new(), 0);
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).expect("a line");
                if let Some(n) = line.to_lowercase().strip_prefix("content-length:") {
                    length = n.trim().parse().expect("a length");
                }
                if line.trim().is_empty() {
                    break;
                }
                head += &line;
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).expect("the body");
            kept.lock()
                .expect("seen")
                .push(head + &String::from_utf8_lossy(&body));
            let reply = "event: message_stop\n";
            let answer = format!(
                "HTTP/1.1 {status}\r\ncontent-length: {}\r\nx-up: yes\r\n\r\n{reply}",
                reply.len()
            );
            conn.write_all(answer.as_bytes()).expect("answered");
        }
    });
    (port, seen)
}

fn record(port: u16) -> Provider {
    Provider {
        agent: "fake".into(),
        config_dir_var: Some("FAKE_HOME".into()),
        proxy: Some(ProxyRecord {
            upstream: format!("http://127.0.0.1:{port}"),
            paths: vec!["/v1/".into()],
            base_url_var: "FAKE_BASE_URL".into(),
            key_var: "FAKE_KEY".into(),
            auth: "authorization: Bearer {token}".into(),
            append: BTreeMap::from([("anthropic-beta".into(), "oauth-2025-04-20".into())]),
        }),
        ..Provider::default()
    }
}

fn served(dir: &Path, provider: &Provider) -> (PathBuf, PathBuf) {
    let state = dir.join("state.json");
    let file = dir.join("work.token");
    std::fs::write(&file, TOKEN).expect("write");
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).expect("chmod");
    let add = AccountAdd {
        provider: "fake".into(),
        name: "work".into(),
        token_file: Some(file),
        ..AccountAdd::default()
    };
    Store::beside(&state).add(provider, &add).expect("added");
    let socket = dir.join("proxy.sock");
    let listener = UnixListener::bind(&socket).expect("bound");
    let record = provider.clone();
    let records = Box::new(move |agent: &str| (agent == "fake").then(|| record.clone()));
    tokio::spawn(Arc::new(Proxy::beside(&state, records)).serve(listener));
    (state, socket)
}

async fn call(socket: &Path, key: &str, path: &str) -> (String, String) {
    let body = r#"{"model":"opus","messages":[]}"#;
    let request = format!(
        "POST {path} HTTP/1.1\r\nhost: proxy\r\nx-api-key: {key}\r\nauthorization: Bearer junk-auth\r\n\
         anthropic-beta: claude-code-20250219\r\ncontent-length: {}\r\n\r\n{body}",
        body.len()
    );
    let mut stream = UnixStream::connect(socket).await.expect("connected");
    stream.write_all(request.as_bytes()).await.expect("sent");
    let mut answer = String::new();
    stream.read_to_string(&mut answer).await.expect("an answer");
    let (status, rest) = answer.split_once("\r\n").unwrap_or_default();
    (status.to_owned(), rest.to_owned())
}

#[tokio::test]
async fn a_live_key_is_signed_with_the_token_and_a_bad_path_or_dead_key_never_reaches_upstream() {
    let dir = temp("signed");
    let (port, seen) = upstream("200 OK");
    let (state, socket) = served(&dir.0, &record(port));
    let keys = Keys::beside(&state);
    let (live, _) = keys.mint("box/a", ("fake", "work"), false).expect("minted");
    let (gone, gone_key) = keys.mint("box/b", ("fake", "work"), false).expect("minted");
    let (ended, _) = keys.mint("box/c", ("fake", "work"), false).expect("minted");

    let (status, body) = call(&socket, &live, "/v1/messages").await;
    assert!(status.contains("200"), "{status} {body}");
    assert!(
        body.contains("x-up: yes") && body.contains("message_stop"),
        "{body}"
    );
    let up = seen.lock().expect("seen").clone();
    assert_eq!(up.len(), 1);
    let head = up[0].to_lowercase();
    assert!(
        head.contains(&format!("authorization: bearer {}", TOKEN.to_lowercase())),
        "{head}"
    );
    assert!(
        head.contains("anthropic-beta: claude-code-20250219,oauth-2025-04-20"),
        "{head}"
    );
    assert!(
        !head.contains("x-api-key") && !head.contains("junk-auth") && !head.contains(&live),
        "{head}"
    );
    assert!(
        up[0].ends_with(r#"{"model":"opus","messages":[]}"#),
        "{}",
        up[0]
    );

    for path in [
        "/v2/messages",
        "@127.0.0.1:9/v1/",
        "/v1/@x",
        "/v1/#@x",
        "v1/messages",
        "/v1\\@x",
        "/v1/../admin",
        "/v1/./../admin",
        "/v1/%2e%2E/admin",
        "/v1/.%2e?x=1",
    ] {
        let (status, body) = call(&socket, &live, path).await;
        assert!(
            status.contains("400") && body.contains("proxy.paths"),
            "{path}: {status} {body}"
        );
    }
    keys.revoke(&gone_key.id).expect("revoked");
    keys.end("box/c");
    for key in [gone.as_str(), ended.as_str(), "shk-nope", ""] {
        let (status, body) = call(&socket, key, "/v1/messages").await;
        assert!(
            status.contains("401") && body.contains("unknown, revoked or ended key"),
            "{status} {body}"
        );
    }
    assert_eq!(
        seen.lock().expect("seen").len(),
        1,
        "refused calls never reach upstream"
    );
}

#[tokio::test]
async fn the_path_is_checked_before_the_token_file_opens() {
    let dir = temp("path-first");
    let (port, seen) = upstream("200 OK");
    let (state, socket) = served(&dir.0, &record(port));
    let (key, _) = Keys::beside(&state)
        .mint("box/a", ("fake", "work"), false)
        .expect("minted");
    let file = dir.0.join("accounts/fake/work.token");
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).expect("chmod");
    let (status, body) = call(&socket, &key, "/v2/messages").await;
    assert!(
        status.contains("400") && !body.contains("chmod"),
        "{status} {body}"
    );
    let (status, body) = call(&socket, &key, "/v1/messages").await;
    assert!(
        status.contains("502") && body.contains("chmod 600"),
        "the token file opened: {status} {body}"
    );
    assert!(seen.lock().expect("seen").is_empty());
}

#[tokio::test]
async fn an_upstream_401_marks_the_token_rejected_once_not_per_lane() {
    let dir = temp("rejected");
    let (port, seen) = upstream("401 Unauthorized");
    let provider = record(port);
    let (state, socket) = served(&dir.0, &provider);
    let keys = Keys::beside(&state);
    let lanes =
        ["box/a", "box/b"].map(|t| keys.mint(t, ("fake", "work"), false).expect("minted").0);
    for key in lanes.iter().chain(&lanes) {
        let (status, _) = call(&socket, key, "/v1/messages").await;
        assert!(status.contains("401"), "{status}");
    }
    assert_eq!(
        seen.lock().expect("seen").len(),
        1,
        "one call went upstream, then the proxy refused"
    );
    let store = Store::beside(&state);
    let token = store
        .account(&provider, "work")
        .expect("account")
        .token
        .expect("a token");
    assert!(token.rejected, "{token:?}");
    let (_, body) = call(&socket, &lanes[0], "/v1/messages").await;
    assert!(
        body.contains("shed account login fake work --token"),
        "{body}"
    );
    let file = std::fs::File::options()
        .write(true)
        .open(dir.0.join("accounts/fake/work.token"))
        .expect("the token file");
    let later =
        std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(token.saved_at + 60);
    file.set_modified(later).expect("a new token's date");
    call(&socket, &lanes[0], "/v1/messages").await;
    assert_eq!(
        seen.lock().expect("seen").len(),
        2,
        "a new token goes upstream again"
    );
}
