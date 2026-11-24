//! A program's own secrets, kept at the [`Place`] it names: Keychain items on macOS, opened only by the signed
//! program that saved them, with prompts off and every call limited to [`WAIT`]; elsewhere 0600 files, a wider
//! one never read. A value is a [`Secret`], which nothing prints, sends or copies.
#![forbid(unsafe_code)]

mod files;
#[cfg(target_os = "macos")]
mod keychain;

use std::path::PathBuf;
use std::time::Duration;

/// How long a Keychain call may take: past it the call is [`Error::Locked`].
pub const WAIT: Duration = Duration::from_secs(5);

const MAX: usize = 8192;

/// A secret's value, 1 to 8,192 bytes, which has no `Debug`, `Display`, `Serialize` or `Clone`:
///
/// ```compile_fail,E0277
/// fn shown(s: &shed_secrets::Secret) -> String { format!("{s:?}") }
/// ```
///
/// ```compile_fail,E0277
/// fn shown(s: &shed_secrets::Secret) -> String { format!("{s}") }
/// ```
///
/// ```compile_fail,E0277
/// fn copied<T: Clone>() {}
/// copied::<shed_secrets::Secret>();
/// ```
#[derive(PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    /// The value as a secret.
    ///
    /// # Errors
    /// [`Error::Size`] when it is empty or over 8,192 bytes.
    pub fn new(value: String) -> Result<Self, Error> {
        if value.is_empty() || value.len() > MAX {
            return Err(Error::Size);
        }
        Ok(Self(value))
    }

    /// The value, for the one place that uses it.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

/// Where a program's secrets are kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    /// macOS: the Keychain service of every item.
    pub service: String,
    /// macOS: what each item's account name starts with, before the secret's name.
    pub prefix: String,
    /// Elsewhere: the folder of 0600 files, one per secret.
    pub files: PathBuf,
}

/// A saved secret's name and when it was saved (unix seconds), never its value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Saved {
    /// The secret's name.
    pub name: String,
    /// When it was saved, when the store says.
    pub at: Option<u64>,
}

/// Why the store refused.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The file is open to others, so it was not read.
    #[error("{} is wider than 0600: not read; fix: chmod 600 {}", .0.display(), .0.display())]
    Open(PathBuf),
    /// The Keychain did not answer in [`WAIT`].
    #[error("the Keychain did not answer in {} s: unlock it", WAIT.as_secs())]
    Locked,
    /// This program has no Apple-issued signature, so its next build could not read what it saved.
    #[error(
        "this program has no Apple-issued signature, so its next build could not read a secret: nothing saved; install a signed build"
    )]
    Unsigned,
    /// The Keychain refused the call.
    #[error("the Keychain refused: {0}")]
    Refused(String),
    /// Not a secret's name: `[A-Za-z0-9._-]`, 1 to 128, not starting with a dot.
    #[error(
        "{0:?} is not a secret's name: A-Z, a-z, 0-9, '.', '_' and '-', 1 to 128, not starting with '.'"
    )]
    Name(String),
    /// A value that is empty or over 8,192 bytes.
    #[error("a secret is 1 to 8,192 bytes")]
    Size,
    /// The file or folder could not be read or written.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// A program's secrets at one [`Place`].
pub struct Store {
    place: Place,
}

impl Store {
    /// The store at `place`; nothing is opened until a call.
    #[must_use]
    pub fn open(place: Place) -> Self {
        Self { place }
    }

    /// Secret `name`, or none when it is not saved.
    ///
    /// # Errors
    /// A bad name, a wide file, a locked Keychain, or one that refuses this program.
    pub fn get(&self, name: &str) -> Result<Option<Secret>, Error> {
        let file = self.file(name)?;
        #[cfg(target_os = "macos")]
        if let Some(v) = keychain::get(&self.place, name)? {
            return Ok(Some(v));
        }
        files::get(&file)
    }

    /// Saves `value` as secret `name`, over any value saved before.
    ///
    /// # Errors
    /// A bad name, an unsigned program on macOS, a locked Keychain, or a folder that cannot be written.
    pub fn set(&self, name: &str, value: &Secret) -> Result<(), Error> {
        let file = self.file(name)?;
        if cfg!(target_os = "macos") {
            #[cfg(target_os = "macos")]
            return keychain::set(&self.place, name, value);
        }
        files::set(&file, value)
    }

    /// Removes secret `name`; one already gone is no error.
    ///
    /// # Errors
    /// A bad name, or a Keychain or file that refuses.
    pub fn remove(&self, name: &str) -> Result<(), Error> {
        let file = self.file(name)?;
        #[cfg(target_os = "macos")]
        keychain::remove(&self.place, name)?;
        files::remove(&file)
    }

    /// The saved secrets by name: the Keychain's on macOS, else the folder's.
    ///
    /// # Errors
    /// A locked Keychain, or a folder that cannot be listed.
    pub fn saved(&self) -> Result<Vec<Saved>, Error> {
        #[cfg(target_os = "macos")]
        return keychain::saved(&self.place);
        #[cfg(not(target_os = "macos"))]
        files::saved(&self.place.files)
    }

    /// macOS: moves each file into the Keychain, read back before the file is shredded; the names moved.
    /// Elsewhere the files stay and none is moved.
    ///
    /// # Errors
    /// A wide file, an unsigned program, or a value the Keychain did not keep; the files not yet moved stay.
    pub fn adopt_files(&self) -> Result<Vec<String>, Error> {
        if !cfg!(target_os = "macos") {
            return Ok(Vec::new());
        }
        let names: Vec<String> = files::saved(&self.place.files)?
            .into_iter()
            .map(|s| s.name)
            .collect();
        for name in &names {
            let file = self.file(name)?;
            let Some(value) = files::get(&file)? else {
                continue;
            };
            self.set(name, &value)?;
            if self.get(name)?.as_ref() != Some(&value) {
                return Err(Error::Refused(format!(
                    "{name}: the Keychain did not keep it"
                )));
            }
            files::shred(&file)?;
        }
        Ok(names)
    }

    /// What stops this store: each file wider than 0600, a Keychain that does not answer, an unsigned program.
    #[must_use]
    pub fn problems(&self) -> Vec<Error> {
        let found = files::wide(&self.place.files);
        #[cfg(target_os = "macos")]
        let found: Vec<Error> = found
            .into_iter()
            .chain(keychain::problems(&self.place))
            .collect();
        found
    }

    fn file(&self, name: &str) -> Result<PathBuf, Error> {
        let ok = |b: u8| b.is_ascii_alphanumeric() || b".-_".contains(&b);
        let fits = (1..=128).contains(&name.len()) && !name.starts_with('.');
        if !fits || !name.bytes().all(ok) {
            return Err(Error::Name(name.to_owned()));
        }
        Ok(self.place.files.join(name))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::hash::{BuildHasher as _, RandomState};
    use std::path::Path;

    use super::*;

    pub(crate) struct Temp(pub(crate) PathBuf);

    impl Drop for Temp {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    pub(crate) fn temp(name: &str) -> Temp {
        let salt = RandomState::new().hash_one(name);
        let dir = std::env::temp_dir().join(format!("shed-secrets-{name}-{salt:x}"));
        std::fs::create_dir_all(dir.join("secrets")).unwrap();
        Temp(dir)
    }

    pub(crate) fn store(dir: &Path) -> Store {
        Store::open(Place {
            service: "shed-secrets-test".into(),
            prefix: format!("{}-", dir.display()),
            files: dir.join("secrets"),
        })
    }

    #[test]
    fn remove_removes_and_twice_is_once() {
        let d = temp("remove");
        let s = store(&d.0);
        std::fs::write(d.0.join("secrets/TOKEN"), "s3cret\n").unwrap();
        std::fs::write(d.0.join("secrets/KEEP"), "k\n").unwrap();
        for f in ["TOKEN", "KEEP"] {
            set_mode(&d.0.join("secrets").join(f), 0o600);
        }
        assert!(s.get("TOKEN").unwrap() == Some(secret("s3cret")));
        s.remove("TOKEN").unwrap();
        assert!(s.get("TOKEN").unwrap().is_none());
        assert!(!d.0.join("secrets/TOKEN").exists());
        s.remove("TOKEN").unwrap();
        assert!(s.get("KEEP").unwrap() == Some(secret("k")));
    }

    #[test]
    fn a_name_that_could_leave_the_folder_is_refused() {
        let d = temp("names");
        let s = store(&d.0);
        let v = Secret::new("v".into()).unwrap();
        for bad in ["", "..", ".hidden", "a/b", "a b", &"x".repeat(129)] {
            assert!(matches!(s.set(bad, &v), Err(Error::Name(_))), "{bad:?}");
            assert!(matches!(s.get(bad), Err(Error::Name(_))), "{bad:?}");
            assert!(matches!(s.remove(bad), Err(Error::Name(_))), "{bad:?}");
        }
        assert!(matches!(Secret::new(String::new()), Err(Error::Size)));
        assert!(matches!(Secret::new("x".repeat(8193)), Err(Error::Size)));
    }

    pub(crate) fn secret(v: &str) -> Secret {
        Secret::new(v.into()).unwrap()
    }

    pub(crate) fn set_mode(path: &Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }
}
