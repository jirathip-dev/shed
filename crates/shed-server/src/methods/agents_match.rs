use crate::call::{Call, Reply, blocking, json};
use crate::{lock, proto, providers};

pub(super) async fn call(cx: &mut Call<'_>) -> Reply {
    let dir = lock(cx.registry).providers.clone();
    let proto::MatchParams { id, text } = cx.params()?;
    let catalog = blocking(move || providers::Catalog::load(&dir).0).await?;
    let rules = catalog.by_id(&id).cloned().unwrap_or_default();
    let limit = rules.limit_hit(&text);
    json(&proto::Matched {
        limit: limit.as_ref().map(|l| l.0.to_owned()),
        resets: limit.and_then(|l| l.1),
        auth: rules.auth_failure(&text).map(str::to_owned),
    })
}
