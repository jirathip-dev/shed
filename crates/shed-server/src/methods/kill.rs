use crate::call::{Call, Reply, json};
use crate::proto::Target;

pub(super) async fn call(cx: &mut Call<'_>) -> Reply {
    let Target { name } = cx.params()?;
    json(&crate::serve::kill(cx.registry, &name).await?)
}
