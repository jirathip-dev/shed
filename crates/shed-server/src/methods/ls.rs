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
    let params = &cx.request.params;
    let up = links.up();
    let calls = up.iter().map(|m| links.call(m, cx.method, params.clone()));
    for more in futures::future::join_all(calls).await {
        if let Ok(Value::Array(more)) = more {
            list.extend(more);
        }
    }
    Ok(Value::Array(list))
}
