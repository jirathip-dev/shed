use crate::call::{Call, Reply};

pub(super) async fn call(cx: &mut Call<'_>) -> Reply {
    crate::code::find(cx.params()?).await
}
