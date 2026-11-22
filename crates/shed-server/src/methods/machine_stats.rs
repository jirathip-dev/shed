use crate::call::{Call, Reply, blocking, json};
use crate::proto::StatsParams;
use crate::{lock, stats};

pub(super) async fn call(cx: &mut Call<'_>) -> Reply {
    let p: StatsParams = cx.params()?;
    let dir = lock(cx.registry).dir();
    let now = blocking(move || stats::system(&dir)).await?;
    let reg = cx.reg()?;
    json(&reg.machine_stats(now, p.since))
}
