use std::path::PathBuf;

use serde_json::{Value, to_value};
use shed_proto::{
    FileMatches, GrepLine, GrepParams, GrepResult, Group, Kind, Lang, Method, OutlineParams,
    OutlineResult, Symbol,
};

fn symbol(kind: Kind, name: &str, start: usize, end: usize, depth: usize) -> Symbol {
    Symbol {
        kind,
        name: name.into(),
        start,
        end,
        depth,
    }
}

fn grep_example() -> (Value, Value) {
    let params = GrepParams {
        root: PathBuf::from("/home/user/project"),
        pattern: "retries".into(),
        word: true,
        glob: vec!["*.rs".into()],
        ..GrepParams::default()
    };
    let line = |n, text: &str| GrepLine {
        n,
        text: text.into(),
        clipped: false,
        length: None,
    };
    let result = GrepResult {
        files: vec![FileMatches {
            path: "src/job.rs".into(),
            lang: Lang::Rust,
            count: 2,
            groups: vec![
                Group {
                    scope: vec![symbol(Kind::Struct, "Job", 9, 14, 0)],
                    lines: vec![line(13, "    pub retries: usize,")],
                },
                Group {
                    scope: vec![
                        symbol(Kind::Impl, "Job", 37, 49, 0),
                        symbol(Kind::Fn, "run", 38, 44, 1),
                    ],
                    lines: vec![line(42, "        let retries = self.retries.min(budget);")],
                },
            ],
        }],
        matches: 2,
        searched: 12,
    };
    (to_value(params).unwrap(), to_value(result).unwrap())
}

pub(crate) fn example(method: Method) -> Option<(Value, Value)> {
    if method == Method::CodeGrep {
        return Some(grep_example());
    }
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
