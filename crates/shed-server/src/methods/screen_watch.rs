use std::sync::Arc;

use crate::call::{Call, Reply, empty};
use crate::proto::WatchParams;
use crate::registry::Stream;

pub(super) fn call(cx: &mut Call<'_>) -> Reply {
    let reg = cx.reg()?;
    let p: WatchParams = cx.params()?;
    let term = Arc::clone(reg.live(&p.terminal)?);
    cx.session.stream = Some(Stream::Screen(term, p.since));
    cx.session.watching = Some(p.terminal);
    Ok(empty())
}
