//! shed's own secrets (a VM provider's key): one store on the main host, the `shed-secret` Keychain service on
//! macOS, else `secrets/` beside the state file.

use std::path::Path;

use shed_secrets::{Error, Place, Store};

use crate::call::Call;
use crate::lock;
use crate::proto::{ErrorCode, RpcError};

fn at(state: &Path) -> Store {
    Store::open(Place {
        service: "shed-secret".into(),
        prefix: String::new(),
        files: state.with_file_name("secrets"),
    })
}

pub(crate) fn store(cx: &Call<'_>) -> Store {
    at(&lock(cx.registry).path)
}

pub(crate) fn refused(e: Error) -> RpcError {
    let code = match e {
        Error::Name(_) | Error::Size => ErrorCode::InvalidParams,
        _ => ErrorCode::SecretStore,
    };
    RpcError::new(code, e)
}

/// What stops the store beside state file `state`, each with whether it fails a read or save of every secret.
#[must_use]
pub fn problems(state: &Path) -> Vec<(bool, String)> {
    let all = at(state).problems().into_iter();
    all.map(|e| (!matches!(e, Error::Unsigned), e.to_string()))
        .collect()
}
