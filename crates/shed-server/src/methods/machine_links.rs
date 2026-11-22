use crate::call::{Call, Reply, json};

pub(super) fn call(cx: &mut Call<'_>) -> Reply {
    cx.session.links.sync(cx.registry);
    json(&cx.session.links.report())
}
