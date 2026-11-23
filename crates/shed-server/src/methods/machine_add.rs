use crate::call::{Call, Reply, empty};
use crate::machines;
use crate::proto::MachineParams;

pub(super) fn call(cx: &mut Call<'_>) -> Reply {
    let p: MachineParams = cx.params()?;
    let links = &cx.session.links;
    let added = machines::add(&links.config, &p);
    added.map_err(machines::refusal)?;
    links.sync(cx.registry);
    Ok(empty())
}
