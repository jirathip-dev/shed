use shed_core::proctree::{self, ProcessTable as _};

use crate::call::{Call, Reply, blocking, io_error, json};
use crate::proto::Target;
use crate::registry::procs::with_commands;

pub(super) async fn call(cx: &mut Call<'_>) -> Reply {
    let Target { name } = cx.params()?;
    let table = blocking(|| proctree::system().list()).await?;
    let rows = cx.reg()?.procs(&name, &table.map_err(io_error)?)?;
    json(&blocking(move || with_commands(rows)).await?)
}
