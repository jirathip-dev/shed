use crate::call::{Call, Reply, json};
use crate::proto::{Hello, RpcError};

pub(super) fn call(cx: &mut Call<'_>) -> Reply {
    let p: Hello = cx.params()?;
    if !p.compatible() {
        return Err(RpcError::version(&p, &Hello::ours()));
    }
    let answer = Hello::answer(&p);
    cx.session.protocol = Some(answer.protocol);
    json(&answer)
}
