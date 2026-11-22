use crate::call::{Call, Reply, json};
use crate::proto::Target;

pub(super) fn call(cx: &mut Call<'_>) -> Reply {
    let reg = cx.reg()?;
    json(&reg.get(&cx.params::<Target>()?.name)?)
}
