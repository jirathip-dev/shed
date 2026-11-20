use shed_proto::Kind;

use super::{ident, strip_any};

const MEMBER: [&str; 11] = [
    "public ",
    "private ",
    "protected ",
    "static ",
    "readonly ",
    "override ",
    "async ",
    "get ",
    "set ",
    "*",
    "#",
];

const ITEMS: [(&str, Kind); 6] = [
    ("class ", Kind::Class),
    ("interface ", Kind::Interface),
    ("const enum ", Kind::Enum),
    ("enum ", Kind::Enum),
    ("namespace ", Kind::Mod),
    ("module ", Kind::Mod),
];

pub(super) fn head(t: &str, member: bool) -> Option<(Kind, String)> {
    let s = strip_any(t, &["export ", "default ", "declare ", "abstract "]);
    if member {
        let s = strip_any(s, &MEMBER);
        let name = ident(s)?;
        let rest = s.get(name.len()..)?.trim_start();
        let keyword = ["if", "for", "while", "switch", "catch", "return"].contains(&name.as_str());
        let call = rest.starts_with(['(', '<']) && !keyword;
        let arrow = rest
            .strip_prefix('=')
            .is_some_and(|r| r.contains("=>") || r.trim_start().starts_with("function"));
        return (call || arrow).then_some((Kind::Fn, name));
    }
    let s = strip_any(s, &["async "]);
    if let Some(rest) = s.strip_prefix("function") {
        let rest = rest.trim_start_matches('*').trim_start();
        return Some((Kind::Fn, ident(rest).unwrap_or_else(|| "default".into())));
    }
    if let Some((rest, kind)) = ITEMS
        .iter()
        .find_map(|(w, k)| Some((s.strip_prefix(w)?, *k)))
    {
        return Some((kind, ident(rest)?));
    }
    if let Some(rest) = s.strip_prefix("type ") {
        let name = ident(rest)?;
        let alias = rest.get(name.len()..)?.trim_start().starts_with(['=', '<']);
        return alias.then_some((Kind::Type, name));
    }
    let rest = ["const ", "let ", "var "]
        .iter()
        .find_map(|w| s.strip_prefix(w))?;
    let name = ident(rest)?;
    let value = rest.split_once('=')?.1.trim_start();
    let func = value.contains("=>")
        || ["(", "async", "function", "<"]
            .iter()
            .any(|p| value.starts_with(p));
    Some((if func { Kind::Fn } else { Kind::Const }, name))
}
