use serde_json::Value;

use crate::call::{Call, empty};

pub(super) fn call(cx: &mut Call<'_>) -> Value {
    cx.session.beats = true;
    empty()
}
