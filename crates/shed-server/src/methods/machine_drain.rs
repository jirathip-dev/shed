use std::time::Duration;

use crate::call::{Call, Reply, empty, io_error};
use crate::proto::DrainParams;

pub(super) async fn call(cx: &mut Call<'_>) -> Reply {
    let p: DrainParams = cx.params()?;
    let now = shed_core::clock::unix_now();
    let spares = {
        let mut reg = cx.reg()?;
        let end = reg.drain.set(p.on, p.why, now).map_err(io_error)?;
        end.then(|| reg.stale_spares(Duration::ZERO))
    };
    for name in spares.into_iter().flatten() {
        crate::serve::end_spare(cx.registry, &name).await;
    }
    Ok(empty())
}
