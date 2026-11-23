use crate::call::{Call, Reply, json};

pub(super) fn call(cx: &mut Call<'_>) -> Reply {
    let p: crate::proto::StateParams = cx.params()?;
    let mut reg = cx.reg()?;
    let was = reg.get(&p.name).ok().and_then(|t| t.reported).map(|r| r.state);
    let info = reg.state(p)?;
    crate::accounts::limited(&reg, &info, was.as_deref());
    json(&info)
}
