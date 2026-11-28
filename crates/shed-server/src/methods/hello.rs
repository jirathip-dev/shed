use std::ffi::OsStr;

use crate::call::{Call, Reply, json};
use crate::proto::{ErrorCode, Hello, RpcError};

pub(super) fn call(cx: &mut Call<'_>) -> Reply {
    let p: Hello = cx.params()?;
    if !p.compatible() {
        return Err(RpcError::version(&p, &Hello::ours()));
    }
    if let Some(t) = p.tool.as_ref()
        && (!t.exe.is_absolute() || t.exe.file_name() != Some(OsStr::new(&t.name)))
    {
        let why = "tool: exe must be an absolute path whose last part is its name";
        return Err(RpcError::new(ErrorCode::InvalidParams, why));
    }
    let mut answer = Hello::answer(&p);
    answer.exe = (crate::lock(cx.registry).control.as_ref()).map(|c| c.again.exe.clone());
    cx.session.protocol = Some(answer.protocol);
    cx.session.tool = p.tool;
    json(&answer)
}
