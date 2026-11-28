//! The lock an agent killed mid-write leaves in its config folder, as its record names it.

use std::fs;
use std::path::Path;
use std::time::UNIX_EPOCH;

use crate::proto::{EventKind, Provider};

/// The record's lock in `folder` when it is stale at `now`: empty and unchanged for longer than its window.
#[must_use]
pub fn found(record: &Provider, folder: &Path, now: u64) -> Option<EventKind> {
    let lock = record.stale_lock.as_ref()?;
    let path = folder.join(&lock.file);
    let meta = fs::symlink_metadata(&path).ok()?;
    let at = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    let age_secs = now.saturating_sub(at.as_secs());
    let stale = meta.is_file() && meta.len() == 0 && age_secs > lock.secs;
    stale.then(|| EventKind::LockCleared {
        path,
        size: meta.len(),
        age_secs,
    })
}

/// Removes the record's lock from `folder` when it is stale now, and says what it removed.
#[must_use]
pub fn clear(record: &Provider, folder: &Path) -> Option<EventKind> {
    let found = found(record, folder, crate::clock::unix_now())?;
    let EventKind::LockCleared { path, .. } = &found else {
        return None;
    };
    fs::remove_file(path).ok().map(|()| found)
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_an_empty_lock_past_the_records_window_is_removed() {
        let lock = crate::proto::StaleLock {
            file: ".refresh.lock".into(),
            secs: 60,
        };
        let record = &crate::proto::Provider {
            stale_lock: Some(lock.clone()),
            ..Default::default()
        };
        let dir = crate::temp_dir("stale-lock");
        let path = dir.join(&lock.file);
        let made = |text: &str, ago: u64| {
            let file = std::fs::File::create(&path).expect("make the lock");
            std::io::Write::write_all(&mut &file, text.as_bytes()).expect("write it");
            let at = crate::clock::unix_now() - ago;
            let at = std::time::UNIX_EPOCH + std::time::Duration::from_secs(at);
            file.set_modified(at).expect("date it");
        };
        assert_eq!(super::clear(record, &dir), None);
        made("", lock.secs - 10);
        assert_eq!(super::clear(record, &dir), None);
        assert!(path.exists(), "a fresh empty lock stays");
        made("{\"pid\":1}", lock.secs * 100);
        assert_eq!(super::clear(record, &dir), None);
        assert!(path.exists(), "a lock with content stays");
        made("", lock.secs + 60);
        let Some(crate::proto::EventKind::LockCleared {
            path: gone,
            size,
            age_secs,
        }) = super::clear(record, &dir)
        else {
            panic!("an old empty lock is cleared");
        };
        assert_eq!((gone, size), (path.clone(), 0));
        assert!((lock.secs + 60..lock.secs + 70).contains(&age_secs));
        assert!(!path.exists(), "and it is gone");
    }
}
