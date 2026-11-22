use crate::call::{Call, Reply, json};

pub(super) async fn call(cx: &mut Call<'_>) -> Reply {
    json(&crate::proxy::relay(cx.registry).await?)
}
