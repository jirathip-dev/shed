use serde_json::Value;

use crate::call::{Call, Reply, json};
use crate::lock;
use crate::proto::LsParams;

pub(super) async fn call(cx: &mut Call<'_>) -> Reply {
    let list = cx.params::<LsParams>().and_then(|p| {
        let mut reg = lock(cx.registry);
        reg.refresh()?;
        json(&reg.ls(&p))
    });
    let Ok(Value::Array(mut list)) = list else {
        return list;
    };
    let links = &cx.session.links;
    for machine in links.up() {
        if let Ok(Value::Array(more)) = links.call(&machine, cx.method, cx.request.params.clone()).await {
            list.extend(more);
        }
    }
    Ok(Value::Array(list))
}
