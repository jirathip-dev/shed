//! shed's session index: a record and a transcript per session, in `sessions/` beside the state file.

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{self, Write as _};
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};

use shed_proto::SessionRecord;

fn file(state: &Path, id: &str, ext: &str) -> PathBuf {
    let name: String = (id.bytes())
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-' => char::from(b).into(),
            b => format!("%{b:02X}"),
        })
        .collect();
    state
        .with_file_name("sessions")
        .join(format!("{name}.{ext}"))
}

/// The transcript of session `id` of the server whose state file is `state`.
#[must_use]
pub fn transcript(state: &Path, id: &str) -> PathBuf {
    file(state, id, "jsonl")
}

fn folder(path: &Path) -> io::Result<()> {
    let dir = path.parent().unwrap_or(path);
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

pub(crate) fn mcp(state: &Path, id: &str) -> PathBuf {
    file(state, id, "mcp.json")
}

pub(crate) fn create(path: &Path, append: bool) -> io::Result<File> {
    folder(path)?;
    let mut open = fs::OpenOptions::new();
    open.create(true).mode(0o600);
    if append {
        open.append(true).open(path)
    } else {
        open.write(true).truncate(true).open(path)
    }
}

pub(crate) fn read(state: &Path, id: &str) -> Option<SessionRecord> {
    serde_json::from_slice(&fs::read(file(state, id, "json")).ok()?).ok()
}

pub(crate) fn write(state: &Path, record: &SessionRecord) -> io::Result<()> {
    let path = file(state, &record.id, "json");
    let tmp = path.with_extension("json.tmp");
    let mut f = create(&tmp, false)?;
    serde_json::to_writer(&mut f, record)?;
    f.write_all(b"\n")?;
    fs::rename(&tmp, path)
}

pub(super) fn title(prompt: &str) -> String {
    prompt
        .lines()
        .next()
        .unwrap_or("")
        .chars()
        .take(80)
        .collect()
}

/// The sessions shed ran, newest first, each with its transcript's path.
///
/// # Errors
/// When the folder exists but cannot be read.
pub fn list(state: &Path) -> io::Result<Vec<SessionRecord>> {
    let dir = state.with_file_name("sessions");
    let entries = match fs::read_dir(dir) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        entries => entries?,
    };
    let paths = entries.flatten().map(|e| e.path());
    let records = paths.filter(|p| p.extension().is_some_and(|e| e == "json"));
    let mut all: Vec<SessionRecord> = records
        .filter_map(|p| serde_json::from_slice(&fs::read(p).ok()?).ok())
        .collect();
    for record in &mut all {
        record.transcript = Some(transcript(state, &record.id));
    }
    all.sort_by_key(|r| std::cmp::Reverse(r.updated));
    Ok(all)
}

pub(crate) fn remove(state: &Path, id: &str) {
    for ext in ["json", "jsonl"] {
        fs::remove_file(file(state, id, ext)).ok();
    }
}

pub(crate) fn prune(state: &Path, now: u64, keep: u64, live: &BTreeSet<String>) {
    let old = |r: &SessionRecord| now.saturating_sub(r.updated) >= keep && !live.contains(&r.id);
    for record in list(state).unwrap_or_default().iter().filter(|r| old(r)) {
        remove(state, &record.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_id_with_a_slash_or_dots_stays_in_the_folder() {
        let state = Path::new("/srv/shed/state.json");
        let folder = Path::new("/srv/shed/sessions");
        for (id, name) in [
            ("../../etc/passwd", "..%2F..%2Fetc%2Fpasswd.jsonl"),
            ("..", "...jsonl"),
            ("a/b", "a%2Fb.jsonl"),
            ("50%", "50%25.jsonl"),
            ("sess_1.x-2", "sess_1.x-2.jsonl"),
        ] {
            let path = transcript(state, id);
            assert_eq!(path.parent(), Some(folder), "{id}");
            assert_eq!(
                path.file_name().and_then(|n| n.to_str()),
                Some(name),
                "{id}"
            );
        }
    }
}
