use serde_json::Value;
use shed_server::lane::Policy;
use shed_server::proto::{Children, Span};

pub fn policy(s: &str) -> Result<Policy, String> {
    serde_json::from_value(Value::String(s.to_owned()))
        .map_err(|_| format!("{s:?}: allow, deny, ask or judge"))
}

pub fn children(s: &str) -> Result<Children, String> {
    serde_json::from_value(Value::String(s.to_owned())).map_err(|_| format!("{s:?}: all or any"))
}

pub fn size(s: &str) -> Result<(u16, u16), String> {
    shed_server::config::size(s).ok_or_else(|| format!("{s:?} is not RxC, e.g. 50x200"))
}

pub fn mem(s: &str) -> Result<u64, String> {
    shed_server::config::mem_mb(s).ok_or_else(|| format!("{s:?} is not a size, e.g. 200M or 4G"))
}

pub fn label(s: &str) -> Result<(String, String), String> {
    s.split_once('=')
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .ok_or_else(|| format!("{s:?} is not k=v"))
}

pub fn span(s: &str) -> Result<Span, String> {
    let (start, end) = s.split_once('-').unwrap_or((s, s));
    let line = |n: &str| n.parse::<usize>().ok().filter(|&n| n > 0);
    match (line(start), line(end)) {
        (Some(start), Some(end)) if start <= end => Ok(Span { start, end }),
        _ => Err(format!("{s:?} is not N or N-M, 1-based, N <= M")),
    }
}
