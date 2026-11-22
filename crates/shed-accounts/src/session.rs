//! Copying a session's files, the record's `session_files`, from one account's folder to another's.

use std::fs::{self, DirBuilder};
use std::io;
use std::os::unix::fs::DirBuilderExt as _;
use std::path::{Path, PathBuf};

use shed_proto::Provider;

use crate::store::{DEFAULT, Store};

/// Where account `name`'s sessions live: its folder, or for `default` the record's `default_folder`, `~/`
/// standing for `home`.
#[must_use]
pub fn folder(store: &Store, provider: &Provider, name: &str, home: &Path) -> Option<PathBuf> {
    if name != DEFAULT {
        return store.folder(provider, name).ok().flatten();
    }
    let dir = provider.default_folder.as_deref()?;
    Some(
        dir.strip_prefix("~/")
            .map_or_else(|| dir.into(), |rest| home.join(rest)),
    )
}

fn matches(dir: &Path, rel: &Path, parts: &[&str]) -> Vec<PathBuf> {
    let Some((part, rest)) = parts.split_first() else {
        return vec![rel.to_owned()];
    };
    let names: Vec<_> = if *part == "*" {
        let entries = fs::read_dir(dir.join(rel)).into_iter().flatten().flatten();
        entries.map(|e| e.file_name()).collect()
    } else {
        vec![part.into()]
    };
    let held = names.into_iter().map(|n| rel.join(n));
    let held = held.filter(|r| fs::symlink_metadata(dir.join(r)).is_ok());
    held.flat_map(|r| matches(dir, &r, rest)).collect()
}

/// Copies session `session`'s files from folder `from` to `to`, at the same paths; how many it copied.
///
/// # Errors
/// When a file cannot be copied.
pub fn copy(provider: &Provider, session: &str, from: &Path, to: &Path) -> io::Result<usize> {
    let mut copied = 0;
    for pattern in &provider.session_files {
        let pattern = pattern.replace("{session}", session);
        let parts: Vec<_> = pattern.split('/').collect();
        for rel in matches(from, Path::new(""), &parts) {
            let target = to.join(&rel);
            if let Some(dir) = target.parent() {
                DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
            }
            fs::copy(from.join(&rel), target)?;
            copied += 1;
        }
    }
    Ok(copied)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::copy;
    use crate::store::tests::{provider, temp};

    #[test]
    fn a_session_s_files_are_copied_to_the_same_paths_and_no_other_file() {
        let dir = temp("session-copy");
        let (from, to) = (dir.path().join("a"), dir.path().join("b"));
        fs::create_dir_all(from.join("projects/-w")).expect("mkdir");
        fs::create_dir_all(from.join("projects/-x")).expect("mkdir");
        fs::write(from.join("projects/-w/s1.jsonl"), "one").expect("write");
        fs::write(from.join("projects/-x/s2.jsonl"), "two").expect("write");
        fs::write(from.join(".credentials.json"), "{}").expect("write");
        let mut p = provider();
        p.session_files = vec!["projects/*/{session}.jsonl".into()];
        assert_eq!(copy(&p, "s1", &from, &to).expect("copied"), 1);
        let read = fs::read_to_string(to.join("projects/-w/s1.jsonl")).expect("read");
        assert_eq!(read, "one");
        assert!(!to.join("projects/-x").exists());
        assert!(!to.join(".credentials.json").exists());
        assert_eq!(copy(&p, "gone", &from, &to).expect("none"), 0);
    }
}
