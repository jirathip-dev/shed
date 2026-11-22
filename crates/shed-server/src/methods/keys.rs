use crate::call::{Call, Reply, empty};

pub(super) async fn call(cx: &mut Call<'_>) -> Reply {
    let input = cx.reg()?.keys(&cx.params()?, false)?;
    input.write().await?;
    Ok(empty())
}
