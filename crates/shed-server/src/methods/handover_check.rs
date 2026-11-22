use crate::call::{Call, Reply};

pub(super) async fn call(cx: &mut Call<'_>) -> Reply {
    crate::handover::answer(cx.registry, cx.method, cx.request).await
}
