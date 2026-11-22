//! The perf gate: the ADR's grep and find queries over `tests/golden`, each one's files read, bytes read, rows and
//! output bytes, against `tests/perf-baseline.txt`. `SHED_BLESS=1` rewrites the baseline.

#![cfg(test)]

use std::path::Path;

use shed_proto::{FindParams, GrepParams};
use shed_search::{Error, LIMITS, Limit, Limits, find, grep, text};

const QUERIES: &[(&str, &[&str])] = &[
    ("Pattern::new", &["-F"]),
    ("spawn_blocking", &["-F"]),
    ("wait", &["-F", "-w", "-g", "*.rs*", "-g", "!tests/"]),
    ("handover_refused", &["-F", "-i"]),
    ("fn \\w+_limit", &[]),
    ("TODO|FIXME|HACK", &[]),
    ("Method::(Exec|FileRead|FileWrite)\\b", &[]),
    ("\\d{2,}\\.\\d+\\.\\d+", &["-g", "*.toml*"]),
    ("^\\s*pub (async )?fn [a-z_]+\\(&mut self", &[]),
    ("[0-9a-f]{40}", &[]),
    ("zzzz_not_here", &["-F"]),
    ("retries", &["-w"]),
    ("self", &["-F", "--count"]),
    ("fn|def|class", &["-i"]),
];

const FINDS: &[(&str, Option<usize>)] = &[
    ("handover", None),
    ("line cap", None),
    ("wait output pattern", None),
    ("queue retries", Some(3)),
];

fn params(root: &Path, query: &str, flags: &[&str]) -> GrepParams {
    let mut p = GrepParams {
        root: root.to_path_buf(),
        pattern: query.into(),
        ..GrepParams::default()
    };
    let mut flags = flags.iter();
    while let Some(&flag) = flags.next() {
        match flag {
            "-F" => p.fixed = true,
            "-i" => p.ignore_case = true,
            "-w" => p.word = true,
            "--count" => p.count = true,
            "-g" => p.glob.extend(flags.next().map(|g| (*g).to_owned())),
            other => panic!("unknown flag {other}"),
        }
    }
    p
}

fn bytes_read<T>(run: &dyn Fn(&Limits) -> Result<T, Error>) -> u64 {
    let fits = |bytes| {
        let limits = Limits { bytes, ..LIMITS };
        !matches!(run(&limits), Err(Error::Limit(o)) if o.limit == Limit::Bytes)
    };
    let (mut lo, mut hi) = (0, LIMITS.bytes);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if fits(mid) { hi = mid } else { lo = mid + 1 }
    }
    lo
}

#[test]
fn counts_match_the_committed_baseline() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = dir.join("tests/golden");
    let mut table = String::from("query | flags | files | bytes | rows | text | json\n");
    for (query, flags) in QUERIES {
        let p = params(&root, query, flags);
        let r = grep(&p, &LIMITS, &|| false).unwrap();
        let rows = if p.count { r.files.len() } else { r.matches };
        let text = text::grep(&r).len();
        let json = serde_json::to_string(&r).unwrap().len();
        let bytes = bytes_read(&|l| grep(&p, l, &|| false));
        let flags = flags.join(" ");
        let line = format!(
            "{query} | {flags} | {} | {bytes} | {rows} | {text} | {json}\n",
            r.searched
        );
        table.push_str(&line);
    }
    for (query, top) in FINDS {
        let p = FindParams {
            root: root.clone(),
            query: (*query).into(),
            top: *top,
            ..FindParams::default()
        };
        let r = find(&p, &LIMITS, &|| false).unwrap();
        let text = text::find(&r).len();
        let json = serde_json::to_string(&r).unwrap().len();
        let bytes = bytes_read(&|l| find(&p, l, &|| false));
        let top = top.map_or("find".into(), |n| format!("find --top {n}"));
        let line = format!(
            "{query} | {top} | {} | {bytes} | {} | {text} | {json}\n",
            r.searched, r.total
        );
        table.push_str(&line);
    }
    let path = dir.join("tests/perf-baseline.txt");
    if std::env::var_os("SHED_BLESS").is_some() {
        std::fs::write(&path, &table).unwrap();
    }
    let want = std::fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(
        table, want,
        "a count moved; if that is meant, rerun with SHED_BLESS=1 and commit it"
    );
}
