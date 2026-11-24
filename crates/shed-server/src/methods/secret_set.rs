use shed_secrets::Secret;

use crate::call::{Call, Reply, blocking, empty};
use crate::proto::SecretSet;
use crate::secrets::{refused, store};

pub(super) async fn call(cx: &mut Call<'_>) -> Reply {
    let SecretSet { name, value } = cx.params()?;
    let value = Secret::new(value).map_err(refused)?;
    let store = store(cx);
    blocking(move || store.set(&name, &value)).await?.map_err(refused)?;
    Ok(empty())
}
