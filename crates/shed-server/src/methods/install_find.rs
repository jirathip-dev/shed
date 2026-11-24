use crate::call::{Call, Reply, blocking, json};
use crate::install;
use crate::proto::Source;

pub(super) async fn call(cx: &mut Call<'_>) -> Reply {
    let p: Source = cx.params()?;
    install::own(cx.session.tool.as_ref(), Some(&p.tool), None)?;
    json(&blocking(move || install::find(&p)).await??)
}
