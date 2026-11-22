use crate::call::{Call, Reply, empty};

pub(super) fn call(cx: &mut Call<'_>) -> Reply {
    let reg = cx.reg()?;
    reg.resize(&cx.params()?).map(|()| empty())
}
