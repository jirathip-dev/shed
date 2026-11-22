use crate::call::{Call, Reply};

pub(super) async fn call(cx: &mut Call<'_>) -> Reply {
    crate::files::answer(cx.params()?).await
}
