//! The token proxy on the main host: a 0600 Unix socket a machine's relay forwards to. A request's key is
//! checked by its sha256, its path against the record's `proxy.paths` before any token file opens, and the call
//! goes upstream through `curl`, whose config, the account's token in it, goes over stdin.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io;
use std::os::unix::fs::DirBuilderExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use shed_core::runner;
use shed_proto::{Provider, ProxyKey, ProxyRecord};
use tokio::io::{
    AsyncBufReadExt as _, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _, BufReader,
};
use tokio::net::{UnixListener, UnixStream};

use crate::keys::{Keys, random_hex};
use crate::store::{Store, write_private};
use crate::token::Token;

const DROP: &str = "host connection content-length transfer-encoding keep-alive x-api-key authorization accept-encoding expect";

fn dropped(name: &str) -> bool {
    DROP.split(' ').any(|d| d.eq_ignore_ascii_case(name.trim()))
}

const MAX_BODY: usize = 64 << 20;

/// The provider records the proxy signs for, by their `agent` name.
pub type Records = Box<dyn Fn(&str) -> Option<Provider> + Send + Sync>;

/// The proxy's keys, accounts and records.
pub struct Proxy {
    keys: Keys,
    store: Store,
    scratch: PathBuf,
    records: Records,
}

struct Request {
    method: String,
    path: String,
    heads: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    fn head(&self, name: &str) -> Option<&str> {
        self.heads
            .iter()
            .find(|h| h.0 == name)
            .map(|h| h.1.as_str())
    }
}

type Refusal = (u16, String);

fn quoted(v: &str) -> String {
    v.replace('\\', "\\\\").replace('"', "\\\"")
}

fn safe(path: &str, proxy: &ProxyRecord) -> bool {
    let hostile = path.chars().any(|c| "@\\#\"".contains(c) || c.is_control());
    let dots = (path.split(['?', '/']))
        .map(|seg| seg.to_ascii_lowercase().replace("%2e", "."))
        .any(|seg| seg == "." || seg == "..");
    !hostile && !dots && proxy.paths.iter().any(|p| path.starts_with(p.as_str()))
}

async fn read(stream: &mut BufReader<UnixStream>) -> io::Result<Request> {
    let mut line = String::new();
    stream.read_line(&mut line).await?;
    let mut words = line.split_whitespace().map(str::to_owned);
    let (method, path) = (
        words.next().unwrap_or_default(),
        words.next().unwrap_or_default(),
    );
    let mut heads = Vec::new();
    loop {
        let mut h = String::new();
        if stream.read_line(&mut h).await? == 0 || h.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            heads.push((k.trim().to_ascii_lowercase(), v.trim().to_owned()));
        }
    }
    let length = heads.iter().find(|h| h.0 == "content-length");
    let length = length.and_then(|h| h.1.parse().ok()).unwrap_or(0_usize);
    let mut body = vec![0; length.min(MAX_BODY)];
    stream.read_exact(&mut body).await?;
    Ok(Request {
        method,
        path,
        heads,
        body,
    })
}

async fn refuse(out: &mut (impl AsyncWrite + Unpin), (code, why): Refusal) -> io::Result<()> {
    let error = serde_json::json!({"type": "authentication_error", "message": why});
    let body = serde_json::json!({"type": "error", "error": error}).to_string();
    let head = format!(
        "HTTP/1.1 {code} Refused\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    out.write_all((head + &body).as_bytes()).await
}

fn headers(req: &Request, proxy: &ProxyRecord, token: &Token) -> String {
    let mut conf = format!(
        "header = \"{}\"\nheader = \"Expect:\"\n",
        quoted(&proxy.auth.replace("{token}", token.expose()))
    );
    let mut kept: Vec<(String, String)> = (req.heads.iter())
        .filter(|h| !dropped(&h.0))
        .cloned()
        .collect();
    for (name, value) in &proxy.append {
        match kept.iter_mut().find(|h| h.0 == name.to_ascii_lowercase()) {
            Some(h) if h.1.split(',').any(|v| v.trim() == value) => {}
            Some(h) => h.1 = format!("{},{value}", h.1),
            None => kept.push((name.to_ascii_lowercase(), value.clone())),
        }
    }
    for (k, v) in kept {
        writeln!(conf, "header = \"{k}: {}\"", quoted(&v)).ok();
    }
    conf
}

async fn relay(
    up: &mut BufReader<impl tokio::io::AsyncRead + Unpin>,
    out: &mut UnixStream,
) -> io::Result<String> {
    let (mut status, mut heads) = (String::new(), Vec::new());
    loop {
        let mut l = String::new();
        if up.read_line(&mut l).await? == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        let l = l.trim_end().to_owned();
        if l.starts_with("HTTP/") {
            status = l.split_once(' ').map_or("502".into(), |x| x.1.to_owned());
            heads.clear();
        } else if l.is_empty() && !status.starts_with('1') {
            break;
        } else if !l.is_empty() {
            heads.push(l);
        }
    }
    let dropped = |h: &&String| h.split_once(':').is_some_and(|(k, _)| dropped(k));
    let mut head = format!("HTTP/1.1 {status}\r\n");
    for h in heads.iter().filter(|h| !dropped(h)) {
        write!(head, "{h}\r\n").ok();
    }
    out.write_all((head + "connection: close\r\n\r\n").as_bytes())
        .await?;
    tokio::io::copy(up, out).await?;
    Ok(status)
}

impl Proxy {
    /// The proxy over the keys and accounts beside the state file `state`, signing for `records`.
    #[must_use]
    pub fn beside(state: &Path, records: Records) -> Self {
        Self {
            keys: Keys::beside(state),
            store: Store::beside(state),
            scratch: state.with_file_name("proxy-bodies"),
            records,
        }
    }

    /// Answers every connection `listener` accepts, until it closes.
    pub async fn serve(self: Arc<Self>, listener: UnixListener) {
        while let Ok((stream, _)) = listener.accept().await {
            let proxy = Arc::clone(&self);
            tokio::spawn(async move { proxy.answer(stream).await.ok() });
        }
    }

    fn check(&self, req: &Request) -> Result<(ProxyKey, Provider, Token), Refusal> {
        let bearer = req
            .head("authorization")
            .and_then(|a| a.strip_prefix("Bearer "));
        let secret = req.head("x-api-key").or(bearer).unwrap_or_default();
        let refused = || (401, "unknown, revoked or ended key".to_owned());
        let key = self.keys.live(secret).ok_or_else(refused)?;
        let provider = (self.records)(&key.provider).ok_or_else(refused)?;
        let proxy = provider.proxy.as_ref().ok_or_else(refused)?;
        if !safe(&req.path, proxy) {
            return Err((
                400,
                format!("path {:?} is outside the record's proxy.paths", req.path),
            ));
        }
        let account = self.store.account(&provider, &key.account);
        let account = account.map_err(|e| (502, e.message))?;
        if account.token.is_some_and(|t| t.rejected) {
            let why = format!(
                "the upstream refused {}/{}'s token: `shed account login {} {} --token`",
                provider.agent, key.account, provider.agent, key.account
            );
            return Err((401, why));
        }
        let token = Token::read(&self.store.token_file(&provider, &key.account));
        Ok((key, provider, token.map_err(|e| (502, e.message))?))
    }

    async fn answer(&self, stream: UnixStream) -> io::Result<()> {
        let mut stream = BufReader::new(stream);
        let req = read(&mut stream).await?;
        let mut out = stream.into_inner();
        let (key, provider, token) = match self.check(&req) {
            Ok(checked) => checked,
            Err(refusal) => return refuse(&mut out, refusal).await,
        };
        let Some(proxy) = &provider.proxy else {
            return Ok(());
        };
        let mut conf = format!(
            "url = \"{}\"\n",
            quoted(&format!("{}{}", proxy.upstream, req.path))
        );
        conf += &headers(&req, proxy, &token);
        drop(token);
        let body = self.scratch.join(format!(
            "{}.body",
            random_hex(8).map_err(|e| io::Error::other(e.message))?
        ));
        if !req.body.is_empty() {
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&self.scratch)?;
            write_private(&body, &req.body)?;
            writeln!(
                conf,
                "data-binary = \"@{}\"",
                quoted(&body.display().to_string())
            )
            .ok();
        }
        let verb = match req.method.as_str() {
            "HEAD" => "head".to_owned(),
            method => format!("request = \"{}\"\ndump-header = \"-\"", quoted(method)),
        };
        writeln!(conf, "{verb}").ok();
        let args = ["-sS", "-N", "-g", "--http1.1", "-K", "-"];
        let none = BTreeMap::new();
        let mut curl = runner::piped("curl", args, Path::new("/"), (&[], &none))?;
        if let Some(mut stdin) = curl.stdin.take() {
            stdin.write_all(conf.as_bytes()).await?;
        }
        drop(conf);
        let status = match curl.stdout.take() {
            Some(up) => relay(&mut BufReader::new(up), &mut out).await,
            None => Err(io::ErrorKind::BrokenPipe.into()),
        };
        curl.wait().await.ok();
        std::fs::remove_file(&body).ok();
        match status {
            Ok(s) if s.starts_with("401") => {
                self.store.reject(&provider, &key.account).ok();
                Ok(())
            }
            Ok(_) => Ok(()),
            Err(_) => refuse(&mut out, (502, "upstream did not answer".to_owned())).await,
        }
    }
}
