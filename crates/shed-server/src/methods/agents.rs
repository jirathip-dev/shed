use crate::call::{Call, Reply, blocking, io_error, json};
use crate::{lock, providers};

pub(super) async fn call(cx: &mut Call<'_>) -> Reply {
    let (state, dir) = {
        let reg = lock(cx.registry);
        (reg.path.clone(), reg.providers.clone())
    };
    let known = blocking(move || providers::known(&state, &dir)).await?;
    json(&known.map_err(io_error)?)
}
