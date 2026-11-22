use crate::call::{Call, Reply, json};
use crate::proto::RunParams;

pub(super) fn call(cx: &mut Call<'_>) -> Reply {
    let mut reg = cx.reg()?;
    let p: RunParams = cx.params()?;
    let spare = p.warm.then(|| p.clone());
    let info = reg.run(p)?;
    if let Some(wizard) = reg.wizard(&info.name) {
        tokio::spawn(wizard);
    }
    let spare = spare.and_then(|p| reg.warm_up(p));
    for name in [Some(info.name.clone()), spare].into_iter().flatten() {
        if let Some(term) = reg.fresh(&name) {
            crate::serve::watch(cx.registry, name, term);
        }
    }
    json(&info)
}
