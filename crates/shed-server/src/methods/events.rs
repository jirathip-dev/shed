use crate::call::{Call, Reply, empty};

pub(super) fn call(cx: &mut Call<'_>) -> Reply {
    let mut reg = cx.reg()?;
    cx.session.stream = Some(reg.subscribe(cx.params()?));
    Ok(empty())
}
