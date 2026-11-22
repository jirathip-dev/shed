use shed_core::proctree::{self, ProcessTable as _};

use crate::call::{Call, Reply, blocking, io_error, json};
use crate::proto::StatsParams;
use crate::registry::procs::with_commands;
use crate::{lock, stats};

pub(super) async fn call(cx: &mut Call<'_>) -> Reply {
    let p: StatsParams = cx.params()?;
    let dir = lock(cx.registry).dir();
    let top = p.top.map(|n| usize::try_from(n).unwrap_or(usize::MAX));
    let table = top.map(|_| proctree::system());
    let (now, table) = blocking(move || (stats::system(&dir), table.map(|t| t.list()))).await?;
    let table = table.transpose().map_err(io_error)?;
    let (mut stats, [cpu, mem]) = {
        let reg = cx.reg()?;
        let top = table.zip(top).map(|(t, n)| reg.top(&t, n));
        (reg.machine_stats(now, p.since), top.unwrap_or_default())
    };
    json(
        &blocking(move || {
            (stats.top_cpu, stats.top_mem) = (with_commands(cpu), with_commands(mem));
            stats
        })
        .await?,
    )
}
