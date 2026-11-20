use std::path::PathBuf;

use serde_json::{Value, to_value};
use shed_proto::{Kind, Lang, Method, OutlineParams, OutlineResult, Symbol};

fn symbol(kind: Kind, name: &str, start: usize, end: usize, depth: usize) -> Symbol {
    Symbol {
        kind,
        name: name.into(),
        start,
        end,
        depth,
    }
}

pub(crate) fn example(method: Method) -> Option<(Value, Value)> {
    let Method::CodeOutline = method else {
        return None;
    };
    let params = OutlineParams {
        path: PathBuf::from("/home/user/project/src/job.rs"),
        machine: Some("box".into()),
    };
    let result = OutlineResult {
        path: "/home/user/project/src/job.rs".into(),
        lang: Lang::Rust,
        lines: 49,
        symbols: vec![
            symbol(Kind::Struct, "Job", 9, 14, 0),
            symbol(Kind::Impl, "Job", 37, 49, 0),
            symbol(Kind::Fn, "run", 38, 44, 1),
        ],
    };
    Some((to_value(params).unwrap(), to_value(result).unwrap()))
}
