use crate::call::{Call, Reply, blocking, json};
use crate::install;
use crate::proto::Dest;

pub(super) async fn call(cx: &mut Call<'_>) -> Reply {
    let p: Dest = cx.params()?;
    install::own(cx.session.tool.as_ref(), None, Some(&p.dest))?;
    json(&blocking(move || install::back(&p.dest)).await??)
}
