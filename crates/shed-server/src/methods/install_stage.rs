use crate::call::{Call, Reply, blocking, json};
use crate::install;
use crate::proto::Stage;

pub(super) async fn call(cx: &mut Call<'_>) -> Reply {
    let p: Stage = cx.params()?;
    install::own(cx.session.tool.as_ref(), Some(&p.source.tool), Some(&p.dest))?;
    json(&blocking(move || install::stage(&p)).await??)
}
