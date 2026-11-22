use crate::call::{Call, Reply, empty};
use crate::lock;

pub(super) fn call(cx: &mut Call<'_>) -> Reply {
    lock(cx.registry).emit(cx.params()?).map(|()| empty())
}
