use crate::call::{Call, Reply, json};

pub(super) fn call(cx: &mut Call<'_>) -> Reply {
    let mut reg = cx.reg()?;
    json(&reg.state(cx.params()?)?)
}
