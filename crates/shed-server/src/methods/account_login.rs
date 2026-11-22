use crate::call::{Call, Reply, json};

pub(super) async fn call(cx: &mut Call<'_>) -> Reply {
    let p = cx.params()?;
    json(&crate::accounts::login(cx, p).await?)
}
