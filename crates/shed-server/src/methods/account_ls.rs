use crate::call::{Call, Reply, blocking, json};
use crate::proto::AccountsParams;

pub(super) async fn call(cx: &mut Call<'_>) -> Reply {
    let AccountsParams { provider } = cx.params()?;
    let mut records = crate::accounts::records(cx).await?;
    records.retain(|r| provider.as_ref().is_none_or(|p| *p == r.agent));
    records.sort_by(|a, b| a.agent.cmp(&b.agent));
    let store = crate::accounts::store(cx);
    let read = move || records.into_iter().map(|r| (store.ls(&r), r)).collect::<Vec<_>>();
    let all = blocking(read).await?;
    let reg = cx.reg()?;
    let listed = |(accounts, record)| crate::accounts::listed(&reg, &record, accounts);
    json(&all.into_iter().flat_map(listed).collect::<Vec<_>>())
}
