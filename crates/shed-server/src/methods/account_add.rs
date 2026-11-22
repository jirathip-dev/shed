use crate::call::{Call, Reply, blocking, json};
use crate::proto::AccountAdd;

pub(super) async fn call(cx: &mut Call<'_>) -> Reply {
    let p: AccountAdd = cx.params()?;
    let (store, provider) = crate::accounts::open(cx, &p.provider).await?;
    let id = provider.id.clone();
    let account = blocking(move || store.add(&provider, &p)).await??;
    json(&crate::accounts::shown(&*cx.reg()?, &id, account))
}
