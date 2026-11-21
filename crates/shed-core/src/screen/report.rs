//! The states a program reports, read from its OSC 7501 escape or another agent's.

/// The states a program can report (`shed state`, or the OSC 7501 escape).
pub const STATES: [&str; 4] = ["working", "blocked", "done", "error"];

type Mapping = &'static [(&'static str, &'static str)];

pub(super) const FOREIGN: [(&[u8], &str, Mapping); 1] = [(
    b"9999",
    "jan-osc9999",
    &[
        ("working", "working"),
        ("blocked", "blocked"),
        ("waiting", "blocked"),
        ("done", "done"),
    ],
)];

/// A state a program reported, with an optional note.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// One of [`STATES`], `idle`, or `clear` (drops the record and those under it) from OSC 7501.
    pub state: String,
    /// A free-text note.
    pub note: Option<String>,
    /// The foreign escape it came from (e.g. `jan-osc9999`); `None` for shed's own.
    pub source: Option<String>,
    /// The record's path (`build/tests`); `None` for the terminal's own.
    pub id: Option<String>,
    /// Why it is blocked: `permission`, `question` or `auth`.
    pub kind: Option<String>,
    /// Percent done, 0–100, while working or blocked.
    pub progress: Option<u8>,
    /// The program's stable name, e.g. `make`.
    pub app: Option<String>,
    /// A short label.
    pub title: Option<String>,
    /// A one-line description.
    pub msg: Option<String>,
    /// Where the state came from: `reported`, `rule:<id>` or `silence`.
    pub from: Option<String>,
}

pub(super) const PROBE_REPLY: &[u8] =
    b"\x1b]7501;?:states=idle,working,done,blocked,error,clear:kinds=permission,question,auth";

fn segment(s: &str) -> bool {
    (1..=32).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.+-".contains(&b))
}

fn text64(v: Option<&str>, max: usize) -> Result<Option<String>, ()> {
    let Some(v) = v else { return Ok(None) };
    let mut bits = 0u32;
    let mut out = Vec::new();
    for (i, c) in v.trim_end_matches('=').bytes().enumerate() {
        let six = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return Err(()),
        };
        bits = bits << 6 | u32::from(six);
        if i % 4 != 0 {
            out.extend(u8::try_from(bits >> (6 - 2 * (i % 4)) & 0xff).ok());
        }
    }
    let text = String::from_utf8(out).map_err(|_| ())?;
    let ok = v.len() <= max.div_ceil(3) * 4
        && v.trim_end_matches('=').len() % 4 != 1
        && !text.chars().any(char::is_control);
    ok.then_some(Some(text)).ok_or(())
}

pub(super) fn program_status(text: &str) -> Option<Report> {
    let mut keys = std::collections::HashMap::new();
    for (k, v) in text.split(':').filter_map(|p| p.split_once('=')) {
        let (k, v) = (k.trim(), v.trim());
        let value = |b: u8| b.is_ascii_alphanumeric() || b"_.,+/=-".contains(&b);
        if !k.is_empty() && k.len() <= 16 && v.bytes().all(value) {
            keys.insert(k, v);
        }
    }
    let state = *keys.get("state")?;
    let id = keys.get("id").map(|id| (*id).to_owned());
    let id_ok =
        |id: &String| id.len() <= 128 && id.split('/').count() <= 8 && id.split('/').all(segment);
    if !["idle", "working", "done", "blocked", "error", "clear"].contains(&state)
        || !id.as_ref().is_none_or(id_ok)
    {
        return None;
    }
    let busy = ["working", "blocked"].contains(&state);
    let kinds = ["permission", "question", "auth"];
    Some(Report {
        state: state.to_owned(),
        id,
        kind: keys
            .get("kind")
            .filter(|k| state == "blocked" && kinds.contains(k))
            .map(|k| (*k).to_owned()),
        progress: keys
            .get("progress")
            .and_then(|p| p.parse().ok())
            .filter(|p| busy && *p <= 100),
        app: keys
            .get("app")
            .filter(|a| segment(a))
            .map(|a| (*a).to_owned()),
        title: text64(keys.get("title").copied(), 192).ok()?,
        msg: text64(keys.get("msg").copied(), 2048).ok()?,
        ..Report::default()
    })
}
