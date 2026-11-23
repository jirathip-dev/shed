use crate::call::{Call, Reply, json};
use crate::proto::AccountName;

pub(super) async fn call(cx: &mut Call<'_>) -> Reply {
    let AccountName { provider, name } = cx.params()?;
    let (store, provider) = crate::accounts::open(cx, &provider).await?;
    let account = shed_accounts::check::status(&store, &provider, &name, &crate::accounts::home()).await?;
    json(&crate::accounts::shown(&*cx.reg()?, &provider.id, account))
}
