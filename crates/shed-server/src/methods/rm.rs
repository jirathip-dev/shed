use crate::call::{Call, Reply, blocking, json};
use crate::proto::Target;

pub(super) async fn call(cx: &mut Call<'_>) -> Reply {
    let Target { name } = cx.params()?;
    let (info, remove) = cx.reg()?.rm(&name)?;
    blocking(remove).await?;
    let closing = info.clone();
    blocking(move || crate::drivers::acp::close(&closing))
        .await?
        .ok();
    json(&info)
}
