use crate::call::{Call, Reply, empty};
use crate::proto::TerminalParams;

pub(super) fn call(cx: &mut Call<'_>) -> Reply {
    let reg = cx.reg()?;
    let TerminalParams { terminal } = cx.params()?;
    reg.get(&terminal)?;
    cx.session.readonly.insert(terminal);
    Ok(empty())
}
