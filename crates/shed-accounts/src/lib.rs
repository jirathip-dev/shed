//! shed's accounts: one config folder per account on the main host, its setup token, and what shed last read
//! of its sign-in and usage. It names no provider: every provider fact comes in as a [`Provider`](shed_proto::Provider).
#![forbid(unsafe_code)]

pub mod check;
pub mod store;
pub mod token;

pub use store::Store;
pub use token::Token;
