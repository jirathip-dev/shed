use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{ErrorKind, Write as _};
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::Path;
use std::time::UNIX_EPOCH;

use crate::{Error, Saved, Secret};

pub(crate) fn get(file: &Path) -> Result<Option<Secret>, Error> {
    let mode = match fs::metadata(file) {
        Ok(m) => m.permissions().mode(),
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    if mode & 0o077 != 0 {
        return Err(Error::Open(file.to_path_buf()));
    }
    let mut text = fs::read_to_string(file)?;
    if text.ends_with('\n') {
        text.pop();
    }
    if text.is_empty() {
        return Ok(None);
    }
    Secret::new(text).map(Some)
}

pub(crate) fn set(file: &Path, value: &Secret) -> Result<(), Error> {
    let dir = file.parent().unwrap_or(Path::new("."));
    DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    let name = file.file_name().unwrap_or_default().to_string_lossy();
    let temp = dir.join(format!(".{name}.new"));
    let mut f = (OpenOptions::new().write(true).create(true).truncate(true))
        .mode(0o600)
        .open(&temp)?;
    f.write_all(value.expose().as_bytes())?;
    f.sync_all()?;
    Ok(fs::rename(temp, file)?)
}

pub(crate) fn remove(file: &Path) -> Result<(), Error> {
    match fs::remove_file(file) {
        Err(e) if e.kind() != ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

pub(crate) fn saved(dir: &Path) -> Result<Vec<Saved>, Error> {
    let mut all = Vec::new();
    for (name, meta) in entries(dir)? {
        let at = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok());
        all.push(Saved {
            name,
            at: at.map(|d| d.as_secs()),
        });
    }
    all.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(all)
}

pub(crate) fn wide(dir: &Path) -> Vec<Error> {
    let all = entries(dir).unwrap_or_default().into_iter();
    let wide = all.filter(|(_, m)| m.permissions().mode() & 0o077 != 0);
    wide.map(|(name, _)| Error::Open(dir.join(name))).collect()
}

pub(crate) fn shred(file: &Path) -> Result<(), Error> {
    let len = fs::metadata(file)?.len();
    let mut f = OpenOptions::new().write(true).open(file)?;
    f.write_all(&vec![0; usize::try_from(len).unwrap_or(0)])?;
    f.sync_all()?;
    Ok(fs::remove_file(file)?)
}

fn entries(dir: &Path) -> Result<Vec<(String, fs::Metadata)>, Error> {
    let read = match fs::read_dir(dir) {
        Ok(d) => d,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let mut all = Vec::new();
    for e in read {
        let e = e?;
        let name = e.file_name().to_string_lossy().into_owned();
        let meta = e.metadata()?;
        if !name.starts_with('.') && meta.is_file() {
            all.push((name, meta));
        }
    }
    Ok(all)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::tests::{set_mode, store, temp};

    const VALUE: &str = "s3cret-value";

    fn path(dir: &Path, name: &str) -> PathBuf {
        dir.join("secrets").join(name)
    }

    #[test]
    fn a_file_wider_than_0600_is_not_read_and_its_error_has_the_fix_not_the_value() {
        let d = temp("wide");
        let f = path(&d.0, "TOKEN");
        fs::write(&f, VALUE).unwrap();
        set_mode(&f, 0o644);
        let e = get(&f).err().unwrap();
        assert!(matches!(e, Error::Open(_)), "{e}");
        let text = e.to_string();
        assert!(
            text.contains(&format!("chmod 600 {}", f.display())),
            "{text}"
        );
        assert!(!text.contains(VALUE), "{text}");
        let wide = wide(&d.0.join("secrets"));
        assert_eq!(wide.len(), 1);
        assert!(!format!("{wide:?}").contains(VALUE));
        set_mode(&f, 0o600);
        assert_eq!(get(&f).unwrap().unwrap().expose(), VALUE);
        assert!(super::wide(&d.0.join("secrets")).is_empty());
    }

    #[test]
    fn a_missing_or_empty_file_is_no_secret_and_one_newline_is_dropped() {
        let d = temp("empty");
        let f = path(&d.0, "TOKEN");
        assert!(get(&f).unwrap().is_none());
        fs::write(&f, "\n").unwrap();
        set_mode(&f, 0o600);
        assert!(get(&f).unwrap().is_none());
        fs::write(&f, "v \n").unwrap();
        assert_eq!(get(&f).unwrap().unwrap().expose(), "v ");
    }

    #[test]
    fn a_file_is_written_0600_in_a_0700_folder_and_listed_by_name() {
        let d = temp("files");
        let dir = d.0.join("new/secrets");
        let f = dir.join("TOKEN");
        set(&f, &Secret::new(VALUE.into()).unwrap()).unwrap();
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!((mode(&f), mode(&dir)), (0o600, 0o700));
        assert_eq!(fs::read_to_string(&f).unwrap(), VALUE);
        fs::write(dir.join(".run-x"), "handoff").unwrap();
        fs::create_dir(dir.join("sub")).unwrap();
        let listed = saved(&dir).unwrap();
        assert_eq!(
            listed.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            ["TOKEN"]
        );
        assert!(listed[0].at.is_some_and(|at| at > 1_700_000_000));
        shred(&f).unwrap();
        assert!(!f.exists());
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn linux_keeps_secrets_in_their_0600_file_and_moves_none() {
        let d = temp("linux");
        let s = store(&d.0);
        s.set("TOKEN", &Secret::new(VALUE.into()).unwrap()).unwrap();
        let f = path(&d.0, "TOKEN");
        assert_eq!(fs::read_to_string(&f).unwrap(), VALUE);
        assert_eq!(s.get("TOKEN").unwrap().unwrap().expose(), VALUE);
        let names: Vec<_> = s.saved().unwrap().into_iter().map(|s| s.name).collect();
        assert_eq!(names, ["TOKEN"]);
        assert!(s.adopt_files().unwrap().is_empty());
        assert!(f.exists());
        assert!(s.problems().is_empty());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn an_unsigned_program_saves_nothing_leaves_the_file_and_says_why() {
        let d = temp("unsigned");
        let s = store(&d.0);
        let f = path(&d.0, "TOKEN");
        fs::write(&f, format!("{VALUE}\n")).unwrap();
        set_mode(&f, 0o600);
        let e = s.adopt_files().unwrap_err();
        assert!(matches!(e, Error::Unsigned), "{e}");
        assert!(!e.to_string().contains(VALUE));
        assert_eq!(fs::read_to_string(&f).unwrap(), format!("{VALUE}\n"));
        assert_eq!(s.get("TOKEN").unwrap().unwrap().expose(), VALUE);
        let e = s
            .set("OTHER", &Secret::new(VALUE.into()).unwrap())
            .unwrap_err();
        assert!(matches!(e, Error::Unsigned), "{e}");
        assert!(!path(&d.0, "OTHER").exists());
        assert!(s.problems().iter().any(|e| matches!(e, Error::Unsigned)));
    }
}
