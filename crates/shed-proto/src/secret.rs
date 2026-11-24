//! shed's own secrets on the wire: set by name, listed by name, removed; a value is never returned.

#[cfg(feature = "schema")]
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// `secret.set` params, whose `Debug` leaves the value out.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct SecretSet {
    /// `[A-Za-z0-9._-]`, 1 to 128, not starting with a dot.
    pub name: String,
    /// 1 to 8,192 bytes; never logged, saved in the state file or echoed in an error.
    pub value: String,
}

impl std::fmt::Debug for SecretSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretSet")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

/// `secret.rm` params.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct SecretName {
    /// The secret's name.
    pub name: String,
}

/// One of shed's own secrets in `secret.ls`: its name, never its value.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct SavedSecret {
    /// The secret's name.
    pub name: String,
    /// When it was saved, when the store says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saved_at: Option<u64>,
}
