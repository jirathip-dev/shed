use serde_json::Value;

use crate::call::{blocking, json};
use crate::proto::{ErrorCode, GrepParams, OutlineParams, RpcError};
use crate::{clock, search};

pub(crate) async fn outline(p: OutlineParams) -> Result<Value, RpcError> {
    let done = blocking(move || search::outline(&p, &search::LIMITS)).await?;
    json(&done.map_err(refusal)?)
}

pub(crate) async fn grep(p: GrepParams) -> Result<Value, RpcError> {
    let started = clock::now();
    let expired = move || started.elapsed() > search::LIMITS.deadline;
    let done = blocking(move || search::grep(&p, &search::LIMITS, &expired)).await?;
    json(&done.map_err(refusal)?)
}

fn refusal(e: search::Error) -> RpcError {
    let code = match e {
        search::Error::Query(_) => ErrorCode::InvalidParams,
        search::Error::Limit(_) => ErrorCode::Limit,
        search::Error::Io(_) => ErrorCode::Io,
    };
    RpcError::new(code, e)
}
