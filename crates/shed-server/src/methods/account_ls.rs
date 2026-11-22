use crate::call::{Call, Reply, blocking, json};
use crate::proto::AccountsParams;

pub(super) async fn call(cx: &mut Call<'_>) -> Reply {
    let AccountsParams { provider } = cx.params()?;
    let mut records = crate::accounts::records(cx).await?;
    records.retain(|r| provider.as_ref().is_none_or(|p| *p == r.agent));
    records.sort_by(|a, b| a.agent.cmp(&b.agent));
    let store = crate::accounts::store(cx);
    let all = blocking(move || records.iter().flat_map(|r| store.ls(r)).collect::<Vec<_>>()).await?;
    let reg = cx.reg()?;
    let shown: Vec<_> = all.into_iter().map(|a| crate::accounts::shown(&reg, a)).collect();
    json(&shown)
}
