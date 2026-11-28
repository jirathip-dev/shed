//! The JSON-RPC envelope: requests, responses, errors and the handshake.

#[cfg(feature = "schema")]
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{BUILD, ErrorCode, JSONRPC, Method, OLDEST_PROTOCOL, PROTOCOL, VERSION};

/// A JSON-RPC request. Without an `id` it is a notification and gets no response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Request {
    /// Always `"2.0"`.
    pub jsonrpc: String,
    /// The request id, echoed in the response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    /// The method name; see [`Method`].
    pub method: String,
    /// The method's parameters.
    #[serde(default)]
    pub params: Value,
}

impl Request {
    /// A request for `method` with `params` and a numeric `id`.
    #[must_use]
    pub fn new(id: u64, method: Method, params: Value) -> Self {
        Self {
            jsonrpc: JSONRPC.to_owned(),
            id: Some(id.into()),
            method: method.name().to_owned(),
            params,
        }
    }
}

/// A JSON-RPC response: exactly one of `result` and `error` is set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Response {
    /// Always `"2.0"`.
    pub jsonrpc: String,
    /// The id of the request this answers (`null` when the request could not be read).
    pub id: Value,
    /// The result on success.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// The error on failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl Response {
    /// A success response.
    #[must_use]
    pub fn ok(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: JSONRPC.to_owned(),
            id,
            result: Some(result),
            error: None,
        }
    }

    /// An error response.
    #[must_use]
    pub fn err(id: Value, error: RpcError) -> Self {
        Self {
            jsonrpc: JSONRPC.to_owned(),
            id,
            result: None,
            error: Some(error),
        }
    }
}

/// A JSON-RPC error object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct RpcError {
    /// The numeric code; see [`ErrorCode`].
    pub code: i64,
    /// `NAME: detail`, where `NAME` is the [`ErrorCode::name`].
    pub message: String,
}

impl RpcError {
    /// An error with `code` and a message `NAME: detail`.
    #[must_use]
    pub fn new(code: ErrorCode, detail: impl std::fmt::Display) -> Self {
        Self {
            code: code.code(),
            message: format!("{}: {detail}", code.name()),
        }
    }

    /// The protocol refusal: `SHED_VERSION: client <sha> protocol <a>-<b> / server <sha> protocol <c>-<d>; update the <side>`.
    #[must_use]
    pub fn version(client: &Hello, server: &Hello) -> Self {
        let side = if client.protocol < server.oldest() {
            "the client"
        } else {
            "the server and restart shed serve"
        };
        Self::new(
            ErrorCode::Version,
            format!(
                "client {} protocol {}-{} / server {} protocol {}-{}; update {side}",
                client.label(),
                client.oldest(),
                client.protocol,
                server.label(),
                server.oldest(),
                server.protocol
            ),
        )
    }
}

impl std::error::Error for RpcError {}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// `hello` params and result: the sender's product, version, build and protocol range.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Hello {
    /// The product, `shed`; a peer that sends none is refused like another protocol.
    #[serde(default)]
    pub product: String,
    /// The shed version.
    pub version: String,
    /// The git commit the sender was built from.
    #[serde(default)]
    pub build: String,
    /// The sender's [`PROTOCOL`]; a peer that sends none counts as protocol 0. A server answers a client on an older protocol it still serves with that protocol.
    #[serde(default)]
    pub protocol: u32,
    /// The oldest protocol the sender still speaks: a server's oldest served, a client's oldest accepted server; none means `protocol` alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oldest: Option<u32>,
    /// A client's own tool, which the `install` methods may move besides shed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<crate::Tool>,
    /// A server's binary, the one it was started from and hands over to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exe: Option<std::path::PathBuf>,
}

impl Hello {
    /// This build's hello: `shed`, [`VERSION`], [`BUILD`], [`PROTOCOL`] and [`OLDEST_PROTOCOL`].
    #[must_use]
    pub fn ours() -> Self {
        Self {
            product: "shed".to_owned(),
            version: VERSION.to_owned(),
            build: BUILD.to_owned(),
            protocol: PROTOCOL,
            oldest: Some(OLDEST_PROTOCOL),
            tool: None,
            exe: None,
        }
    }

    /// The server's answer to a compatible `client`: ours, in the client's shapes when it speaks an older protocol.
    #[must_use]
    pub fn answer(client: &Self) -> Self {
        if client.protocol < PROTOCOL {
            Self {
                protocol: client.protocol,
                oldest: None,
                ..Self::ours()
            }
        } else {
            Self::ours()
        }
    }

    /// Whether a peer that sent this hello is `shed` with a protocol range that overlaps ours.
    #[must_use]
    pub fn compatible(&self) -> bool {
        self.product == "shed" && self.oldest() <= PROTOCOL && OLDEST_PROTOCOL <= self.protocol
    }

    fn oldest(&self) -> u32 {
        self.oldest.unwrap_or(self.protocol)
    }

    /// The build's sha (`unknown` when it sent none), after the product unless that is `shed`.
    #[must_use]
    pub fn label(&self) -> String {
        let build = if self.build.is_empty() {
            "unknown"
        } else {
            &self.build
        };
        match self.product.as_str() {
            "shed" => build.to_owned(),
            "" => format!("unnamed {build}"),
            product => format!("{product} {build}"),
        }
    }
}
