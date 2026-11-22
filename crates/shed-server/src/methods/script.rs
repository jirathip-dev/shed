use crate::call::{Call, Reply};

pub(super) async fn call(cx: &mut Call<'_>) -> Reply {
    let p = cx.params()?;
    crate::script::answer(cx.registry, cx.session, p).await
}
