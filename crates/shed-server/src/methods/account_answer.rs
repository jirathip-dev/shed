use crate::call::{Call, Reply, empty};

pub(super) async fn call(cx: &mut Call<'_>) -> Reply {
    let p = cx.params()?;
    crate::accounts::answer(cx, p).await?;
    Ok(empty())
}
