use serde_json::Value;

use crate::proto::{ErrorCode, OutlineParams, RpcError};
use crate::{blocking, json, search};

pub(crate) async fn outline(p: OutlineParams) -> Result<Value, RpcError> {
    let done = blocking(move || search::outline(&p, &search::LIMITS)).await?;
    json(&done.map_err(refusal)?)
}

fn refusal(e: search::Error) -> RpcError {
    let code = match e {
        search::Error::Limit(_) => ErrorCode::Limit,
        search::Error::Io(_) => ErrorCode::Io,
    };
    RpcError::new(code, e)
}
