use crate::call::{Call, Reply, blocking, json};
use crate::lock;
use crate::proto::ReportParams;

pub(super) async fn call(cx: &mut Call<'_>) -> Reply {
    let p: ReportParams = cx.params()?;
    let state = lock(cx.registry).path.clone();
    let now = shed_core::clock::unix_now();
    json(&blocking(move || crate::flows::report(&state, p.since, now)).await?)
}
