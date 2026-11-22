use crate::call::{Call, Reply, json};
use crate::proto::Target;

pub(super) fn call(cx: &mut Call<'_>) -> Reply {
    let reg = cx.reg()?;
    let (info, attached) = reg.attach(&cx.params::<Target>()?.name)?;
    cx.session.stream = Some(attached);
    json(&info)
}
