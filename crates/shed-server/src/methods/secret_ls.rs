use crate::call::{Call, Reply, blocking, json};
use crate::proto::SavedSecret;
use crate::secrets::{refused, store};

pub(super) async fn call(cx: &mut Call<'_>) -> Reply {
    let store = store(cx);
    let saved = blocking(move || store.saved()).await?.map_err(refused)?;
    let listed = saved.into_iter().map(|s| SavedSecret {
        name: s.name,
        saved_at: s.at,
    });
    json(&listed.collect::<Vec<_>>())
}
