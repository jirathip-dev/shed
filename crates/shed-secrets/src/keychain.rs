use std::sync::OnceLock;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use security_framework::base::Error as KeychainError;
use security_framework::item::{ItemClass, ItemSearchOptions, Limit, SearchResult};
use security_framework::os::macos::code_signing::{Flags, SecCode, SecRequirement};
use security_framework::os::macos::keychain::SecKeychain;
use security_framework::os::macos::passwords::find_generic_password;
use security_framework::passwords::delete_generic_password;

use crate::{Error, Place, Saved, Secret, WAIT};

const NOT_FOUND: i32 = -25300;
const SIGNED: &str = "anchor apple generic and certificate leaf[subject.OU] exists";

fn refused(e: KeychainError) -> Error {
    Error::Refused(e.to_string())
}

fn quiet() {
    static OFF: OnceLock<()> = OnceLock::new();
    OFF.get_or_init(|| std::mem::forget(SecKeychain::disable_user_interaction()));
}

fn signed() -> bool {
    let req = SIGNED.parse::<SecRequirement>();
    req.and_then(|r| SecCode::for_self(Flags::NONE)?.check_validity(Flags::NONE, &r))
        .is_ok()
}

pub(crate) fn get(place: &Place, name: &str) -> Result<Option<Secret>, Error> {
    let (service, account) = (place.service.clone(), format!("{}{name}", place.prefix));
    limited(WAIT, move || {
        quiet();
        match find_generic_password(None, &service, &account) {
            Ok((v, _)) => Secret::new(String::from_utf8_lossy(&v).into_owned()).map(Some),
            Err(e) if e.code() == NOT_FOUND => Ok(None),
            Err(e) => Err(refused(e)),
        }
    })
}

pub(crate) fn set(place: &Place, name: &str, value: &Secret) -> Result<(), Error> {
    if !signed() {
        return Err(Error::Unsigned);
    }
    let (service, account) = (place.service.clone(), format!("{}{name}", place.prefix));
    let value = value.expose().as_bytes().to_vec();
    limited(WAIT, move || {
        quiet();
        let k = SecKeychain::default().map_err(refused)?;
        k.set_generic_password(&service, &account, &value)
            .map_err(refused)
    })
}

pub(crate) fn remove(place: &Place, name: &str) -> Result<(), Error> {
    let (service, account) = (place.service.clone(), format!("{}{name}", place.prefix));
    limited(WAIT, move || {
        quiet();
        match delete_generic_password(&service, &account) {
            Err(e) if e.code() != NOT_FOUND => Err(refused(e)),
            _ => Ok(()),
        }
    })
}

pub(crate) fn saved(place: &Place) -> Result<Vec<Saved>, Error> {
    let (service, prefix) = (place.service.clone(), place.prefix.clone());
    limited(WAIT, move || {
        quiet();
        let found = ItemSearchOptions::new()
            .class(ItemClass::generic_password())
            .service(&service)
            .load_attributes(true)
            .limit(Limit::All)
            .search();
        let found = match found {
            Err(e) if e.code() == NOT_FOUND => return Ok(Vec::new()),
            f => f.map_err(refused)?,
        };
        let mut all: Vec<Saved> = (found.iter().filter_map(SearchResult::simplify_dict))
            .filter_map(|mut d| {
                let name = d.remove("acct")?.strip_prefix(&prefix)?.to_owned();
                let at = d.get("mdat").and_then(|t| unix(t));
                Some(Saved { name, at })
            })
            .collect();
        all.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(all)
    })
}

fn limited<T: Send + 'static>(
    wait: Duration,
    f: impl FnOnce() -> Result<T, Error> + Send + 'static,
) -> Result<T, Error> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || tx.send(f()));
    match rx.recv_timeout(wait) {
        Ok(r) => r,
        Err(RecvTimeoutError::Timeout) => Err(Error::Locked),
        Err(RecvTimeoutError::Disconnected) => {
            Err(Error::Refused("the call ended without an answer".into()))
        }
    }
}

pub(crate) fn problems(place: &Place) -> Vec<Error> {
    let mut found: Vec<Error> = saved(place).err().into_iter().collect();
    if !signed() {
        found.push(Error::Unsigned);
    }
    found
}

fn unix(date: &str) -> Option<u64> {
    let mut parts = date.split([' ', '-', ':', '+']).map(str::parse::<i64>);
    let mut next = || parts.next()?.ok();
    let (y, m, d) = (next()?, next()?, next()?);
    let secs = next()? * 3600 + next()? * 60 + next()?;
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    u64::try_from((era * 146_097 + doe - 719_468) * 86_400 + secs).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_keychain_call_that_hangs_is_locked() {
        let (_hold, gate) = mpsc::channel::<()>();
        let r = limited(Duration::from_millis(20), move || {
            gate.recv().ok();
            Ok(())
        });
        let e = r.unwrap_err();
        assert!(matches!(e, Error::Locked), "{e}");
        assert!(e.to_string().contains("did not answer"), "{e}");
    }

    #[test]
    fn a_keychain_date_reads_as_unix_seconds() {
        assert_eq!(unix("1970-01-01 00:00:00 +0000"), Some(0));
        assert_eq!(unix("2026-11-24 10:30:05 +0000"), Some(1_795_516_205));
        assert_eq!(unix("2000-03-01 00:00:00 +0000"), Some(951_868_800));
        assert_eq!(unix("unknown"), None);
    }
}
