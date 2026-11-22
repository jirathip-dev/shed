use crate::call::{Call, Reply, blocking, json};
use crate::proto::ReadParams;

pub(super) async fn call(cx: &mut Call<'_>) -> Reply {
    let p: ReadParams = cx.params()?;
    let read = cx.reg()?.read(p)?;
    json(&blocking(read).await??)
}
