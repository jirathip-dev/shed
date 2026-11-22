use serde_json::Value;

use crate::call::Call;

pub(super) fn call(_: &mut Call<'_>) -> Value {
    crate::proto::openrpc()
}
