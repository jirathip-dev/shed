use crate::call::{Call, Reply, blocking, empty};
use crate::proto::SecretName;
use crate::secrets::{refused, store};

pub(super) async fn call(cx: &mut Call<'_>) -> Reply {
    let SecretName { name } = cx.params()?;
    let store = store(cx);
    blocking(move || store.remove(&name)).await?.map_err(refused)?;
    Ok(empty())
}
