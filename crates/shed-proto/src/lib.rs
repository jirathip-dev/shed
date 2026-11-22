//! shed's API types and JSON-RPC method list.
//!
//! The wire format is newline-delimited JSON-RPC 2.0 over a Unix socket: one [`Request`] per line, one
//! [`Response`] per line. Every connection starts with [`Method::Hello`]; a client and server of
//! different products or of protocol ranges that don't overlap are refused with [`ErrorCode::Version`]. `docs/protocol.md` is generated
//! from these types (`SHED_BLESS=1 cargo test -p shed-proto --features schema`); CI fails when it drifts.
#![forbid(unsafe_code)]

pub mod accounts;
pub use accounts::*;
pub mod acp;
pub use acp::*;
pub mod agents;
pub use agents::*;
pub mod code;
pub use code::*;
pub mod envelope;
pub use envelope::*;
pub mod events;
pub use events::*;
pub mod handover;
pub use handover::*;
pub mod input;
pub use input::*;
pub mod machine;
pub use machine::*;
pub mod method;
pub use method::*;
pub mod screen;
pub use screen::*;
pub mod terminal;
pub use terminal::*;

/// The shed version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The git commit this build was made from, or `unknown` outside a git checkout.
pub const BUILD: &str = env!("SHED_BUILD");

/// `<semver> (<sha>)`, as `shed --version`, `shed hello` and the server's ready line print it.
pub const LONG_VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), " (", env!("SHED_BUILD"), ")");

/// The wire protocol version; raise it only to drop or retype a method, field or type (`just protocol`). From 3 a server refuses a request field the method does not document.
pub const PROTOCOL: u32 = 5;

/// The oldest protocol this build still speaks: a server answers a client on it in its shapes, a client accepts a server on it.
pub const OLDEST_PROTOCOL: u32 = 1;

/// The JSON-RPC version string every message carries.
pub const JSONRPC: &str = "2.0";
