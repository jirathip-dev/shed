use shed_proto::Kind;

use super::{ident, strip_any};

pub(super) fn head(t: &str) -> Option<(Kind, String)> {
    let s = strip_any(t, &["async "]);
    if let Some(rest) = s.strip_prefix("def ") {
        return Some((Kind::Fn, ident(rest)?));
    }
    Some((Kind::Class, ident(s.strip_prefix("class ")?)?))
}
