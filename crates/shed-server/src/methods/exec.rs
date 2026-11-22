use std::os::unix::process::ExitStatusExt as _;
use std::path::PathBuf;
use std::time::Duration;

use crate::call::{Call, Reply, io_error, json};
use crate::proto::{self, ErrorCode, ExecParams, ExecResult, RpcError};
use crate::runner;

pub(super) async fn call(cx: &mut Call<'_>) -> Reply {
    let p: ExecParams = cx.params()?;
    let timeout = p
        .timeout
        .map_or(Ok(Duration::MAX), Duration::try_from_secs_f64);
    let timeout = timeout.map_err(|e| RpcError::new(ErrorCode::InvalidParams, e))?;
    let cwd = p.cwd.unwrap_or_else(|| PathBuf::from("."));
    let ran = runner::exec(&p.cmd, &cwd, p.stdin, timeout, proto::REPLY_LIMIT).await;
    let (status, stdout, stderr) = ran.map_err(io_error)?;
    crate::files::within_limit(stdout.len() + stderr.len()).map_err(io_error)?;
    let signal = status.signal().map(|s| s.saturating_add(128));
    let code = status.code().or(signal).unwrap_or(1);
    json(&ExecResult {
        code,
        stdout,
        stderr,
    })
}
