use std::collections::BTreeSet;
use std::sync::{Arc, MutexGuard};

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::proto::{ErrorCode, Method, Request, RpcError};
use crate::registry::{Registry, Stream};
use crate::{Shared, handover, links, lock};

pub(crate) type Reply = Result<Value, RpcError>;

pub(crate) struct Call<'a> {
    pub(crate) method: Method,
    pub(crate) request: &'a Request,
    pub(crate) registry: &'a Shared,
    pub(crate) session: &'a mut Session,
}

impl<'a> Call<'a> {
    pub(crate) fn params<T: DeserializeOwned>(&self) -> Result<T, RpcError> {
        params(self.request)
    }

    pub(crate) fn reg(&self) -> Result<MutexGuard<'a, Registry>, RpcError> {
        let mut reg = lock(self.registry);
        reg.refresh()?;
        Ok(reg)
    }
}

#[derive(Default)]
pub(crate) struct Session {
    pub(crate) protocol: Option<u32>,
    pub(crate) beats: bool,
    pub(crate) stream: Option<Stream>,
    pub(crate) watching: Option<String>,
    pub(crate) readonly: BTreeSet<String>,
    pub(crate) links: Arc<links::Links>,
    pub(crate) busy: Option<handover::Busy>,
    pub(crate) tool: Option<crate::proto::Tool>,
    pub(crate) inside: bool,
}

pub(crate) async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> T + Send + 'static,
) -> Result<T, RpcError> {
    let joined = tokio::task::spawn_blocking(f).await;
    joined.map_err(|e| RpcError::new(ErrorCode::Internal, e))
}

pub(crate) fn params<T: DeserializeOwned>(request: &Request) -> Result<T, RpcError> {
    let empty = Value::Object(serde_json::Map::new());
    let p = Some(&request.params).filter(|p| !p.is_null());
    T::deserialize(p.unwrap_or(&empty)).map_err(|e| RpcError::new(ErrorCode::InvalidParams, e))
}

pub(crate) fn json<T: Serialize>(value: &T) -> Result<Value, RpcError> {
    serde_json::to_value(value).map_err(|e| RpcError::new(ErrorCode::Internal, e))
}

pub(crate) fn io_error(e: impl std::fmt::Display) -> RpcError {
    RpcError::new(ErrorCode::Io, e)
}

pub(crate) fn empty() -> Value {
    Value::Object(serde_json::Map::new())
}
