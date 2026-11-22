use crate::call::{Call, Reply};

pub(super) async fn call(cx: &mut Call<'_>) -> Reply {
    crate::code::grep(cx.params()?).await
}
