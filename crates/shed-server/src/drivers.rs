//! The drivers a lane runs an agent through: one awake agent process and its protocol.

pub mod acp;
pub(crate) mod stream;

pub(crate) use stream::choose;
