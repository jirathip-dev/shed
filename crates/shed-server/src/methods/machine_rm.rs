use crate::call::{Call, Reply, empty};
use crate::proto::Target;
use crate::{Error, machines};

pub(super) fn call(cx: &mut Call<'_>) -> Reply {
    let Target { name } = cx.params()?;
    let links = &cx.session.links;
    match machines::remove(&links.config, &name) {
        Ok(_) | Err(Error::NoMachine(_)) => {}
        Err(e) => return Err(machines::refusal(e)),
    }
    links.sync(cx.registry);
    Ok(empty())
}
