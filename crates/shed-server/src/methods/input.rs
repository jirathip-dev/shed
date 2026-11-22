use crate::call::{Call, Reply, empty};

pub(super) async fn call(cx: &mut Call<'_>) -> Reply {
    let input = cx.reg()?.typed(cx.params()?)?;
    input.write().await?;
    Ok(empty())
}
