use shed_proto::Kind;

use super::{ident, strip_any};

const ITEMS: [(&str, Kind); 10] = [
    ("fn ", Kind::Fn),
    ("struct ", Kind::Struct),
    ("union ", Kind::Struct),
    ("enum ", Kind::Enum),
    ("trait ", Kind::Trait),
    ("type ", Kind::Type),
    ("const ", Kind::Const),
    ("static mut ", Kind::Const),
    ("static ", Kind::Const),
    ("macro_rules! ", Kind::Macro),
];

pub(super) fn head(t: &str) -> Option<(Kind, String)> {
    let mut s = t;
    loop {
        let before = s;
        if s.starts_with("pub(") {
            s = s.split_once(')')?.1.trim_start();
        }
        s = strip_any(s, &["pub ", "async ", "unsafe ", "default "]);
        if let Some(rest) = s.strip_prefix("extern \"") {
            s = rest.split_once('"')?.1.trim_start();
        }
        if let Some(rest) = s.strip_prefix("const ")
            && ["fn ", "unsafe ", "async ", "extern "]
                .iter()
                .any(|k| rest.starts_with(k))
        {
            s = rest;
        }
        if s == before {
            break;
        }
    }
    if let Some((rest, kind)) = ITEMS
        .iter()
        .find_map(|(w, k)| Some((s.strip_prefix(w)?, *k)))
    {
        return Some((kind, ident(rest)?));
    }
    if let Some(rest) = s.strip_prefix("mod ") {
        return (!t.ends_with(';')).then_some((Kind::Mod, ident(rest)?));
    }
    let rest = s.strip_prefix("impl")?;
    if !rest.starts_with([' ', '<']) {
        return None;
    }
    let mut rest = rest.trim_start();
    if rest.starts_with('<') {
        let mut depth = 0_i32;
        let (close, _) = rest.char_indices().find(|&(_, c)| {
            depth += i32::from(c == '<') - i32::from(c == '>');
            depth == 0
        })?;
        rest = rest.get(close.saturating_add(1)..)?.trim_start();
    }
    let name = rest.split(" where").next()?.trim_end_matches('{').trim();
    (!name.is_empty()).then(|| (Kind::Impl, name.to_owned()))
}
