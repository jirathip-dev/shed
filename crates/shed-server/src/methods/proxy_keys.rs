use shed_accounts::keys::Keys;

use crate::call::{Call, Reply, json};

pub(super) fn call(cx: &mut Call<'_>) -> Reply {
    json(&Keys::beside(&cx.reg()?.path).ls())
}
