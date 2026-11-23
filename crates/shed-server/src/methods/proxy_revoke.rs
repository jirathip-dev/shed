use shed_accounts::keys::Keys;

use crate::call::{Call, Reply, empty};
use crate::proto::KeyId;

pub(super) fn call(cx: &mut Call<'_>) -> Reply {
    let KeyId { id } = cx.params()?;
    Keys::beside(&cx.reg()?.path).revoke(&id)?;
    Ok(empty())
}
