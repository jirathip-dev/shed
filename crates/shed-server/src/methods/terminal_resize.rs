use crate::call::{Call, Reply, empty};
use crate::proto::{ErrorCode, ResizeParams, RpcError, TerminalResizeParams};

pub(super) fn call(cx: &mut Call<'_>) -> Reply {
    let reg = cx.reg()?;
    let p: TerminalResizeParams = cx.params()?;
    let Some(name) = p.terminal.or_else(|| cx.session.watching.clone()) else {
        let e = "give terminal, or watch one first";
        return Err(RpcError::new(ErrorCode::InvalidParams, e));
    };
    let (rows, cols) = (p.rows, p.cols);
    reg.resize(&ResizeParams { name, rows, cols })
        .map(|()| empty())
}
