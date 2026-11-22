//! `grep` and `find` on real trees: the goldens, the walk's rules, every limit and a clipped line.

#![cfg(test)]

use std::cell::Cell;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use serde_json::Value;
use shed_proto::{FindParams, GrepParams, GrepResult};
use shed_search::{Error, LIMITS, Limit, Limits, Over, find, grep, text};

struct Temp(PathBuf);

impl Temp {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("shed-grep-{name}-{}", std::process::id()));
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn put(&self, rel: &str, bytes: &[u8]) -> &Self {
        let path = self.0.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
        self
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).ok();
    }
}

fn params(root: &Path, pattern: &str) -> GrepParams {
    GrepParams {
        root: root.to_path_buf(),
        pattern: pattern.into(),
        ..GrepParams::default()
    }
}

fn run(p: &GrepParams) -> GrepResult {
    grep(p, &LIMITS, &|| false).unwrap()
}

fn paths(r: &GrepResult) -> Vec<&str> {
    r.files.iter().map(|f| f.path.as_str()).collect()
}

fn samples(name: &str) -> Temp {
    let tmp = Temp::new(name);
    let golden = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    for entry in fs::read_dir(&golden).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_str().unwrap();
        if let Some(name) = name.strip_suffix(".in") {
            tmp.put(name, &fs::read(&path).unwrap());
        }
    }
    tmp
}

fn golden(name: &str) -> String {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    fs::read_to_string(dir.join(name)).unwrap()
}

#[test]
fn the_grep_goldens_hold() {
    let tmp = samples("samples");
    let p = GrepParams {
        word: true,
        ..params(&tmp.0, "retries")
    };
    assert_eq!(text::grep(&run(&p)), golden("grep-retries.txt"));
    let one = GrepParams {
        root: tmp.0.join("sample.rs"),
        ..p
    };
    let want: Value = serde_json::from_str(&golden("grep-retries-rs.json")).unwrap();
    assert_eq!(serde_json::to_value(run(&one)).unwrap(), want);
}

#[test]
fn the_find_golden_holds() {
    let tmp = samples("find-samples");
    let p = FindParams {
        root: tmp.0.clone(),
        query: "Queue retries queue".into(),
        top: Some(3),
        ..FindParams::default()
    };
    let r = find(&p, &LIMITS, &|| false).unwrap();
    assert_eq!(text::find(&r), golden("find-queue-retries.txt"));
    assert!(r.files.windows(2).all(|w| w[0].score >= w[1].score));
}

#[test]
fn find_ranks_names_paths_and_tests_and_says_why() {
    let tmp = Temp::new("find");
    tmp.put("lane/cap.rs", b"fn cap() {}\n")
        .put("lane/tests/cap_test.rs", b"fn cap() {}\n")
        .put("src/caplimit.rs", b"x\n")
        .put("src/other.rs", b"nothing here\n");
    let p = |query: &str, top| FindParams {
        root: tmp.0.clone(),
        query: query.into(),
        top,
        ..FindParams::default()
    };
    let r = find(&p("cap lane", None), &LIMITS, &|| false).unwrap();
    let why: Vec<(&str, u32, Vec<&str>)> = r
        .files
        .iter()
        .map(|f| {
            (
                f.path.as_str(),
                f.score,
                f.why.iter().map(String::as_str).collect(),
            )
        })
        .collect();
    assert_eq!(
        why,
        [
            (
                "lane/cap.rs",
                138,
                vec!["name is cap", "path has lane", "lines: cap 1"]
            ),
            (
                "lane/tests/cap_test.rs",
                53,
                vec!["name has cap", "path has lane", "lines: cap 1", "a test"]
            ),
            ("src/caplimit.rs", 24, vec!["name has cap"]),
        ]
    );
    assert_eq!((r.total, r.searched), (3, 4));
    assert_eq!(r.files[0].symbols[0].name, "cap");
    let one = find(&p("cap", Some(1)), &LIMITS, &|| false).unwrap();
    assert_eq!((one.files.len(), one.total), (1, 3));
    let all = find(&p("cap", Some(500)), &LIMITS, &|| false).unwrap();
    assert_eq!(all.files.len(), 3);
    let empty = find(&p("  ", None), &LIMITS, &|| false);
    assert!(matches!(empty, Err(Error::Query(_))), "{empty:?}");
}

#[test]
fn the_walk_keeps_to_the_ignore_rules_in_name_order() {
    let tmp = Temp::new("walk");
    tmp.put("store/info/exclude", b"excluded.txt\n")
        .put("store/worktrees/wt/commondir", b"../..\n")
        .put("wt/.git", b"gitdir: ../store/worktrees/wt\n")
        .put("wt/.gitignore", b"*.log\n!keep.log\nbuilt/\n")
        .put("wt/z.txt", b"needle\n")
        .put("wt/a.txt", b"needle\n")
        .put("wt/b.log", b"needle\n")
        .put("wt/keep.log", b"needle\n")
        .put("wt/excluded.txt", b"needle\n")
        .put("wt/built/x.txt", b"needle\n")
        .put("wt/built/.gitignore", b"!x.txt\n")
        .put("wt/sub/.gitignore", b"deep.txt\n")
        .put("wt/sub/deep.txt", b"needle\n")
        .put("wt/sub/c.log", b"needle\n")
        .put("wt/sub/m.txt", b"needle\r\nno\n")
        .put("wt/.hidden", b"needle\n")
        .put("wt/bin.dat", b"needle\0\n");
    let wt = tmp.0.join("wt");
    symlink(wt.join("a.txt"), wt.join("link.txt")).unwrap();
    symlink(&wt, wt.join("loop")).unwrap();

    let all = run(&params(&wt, "needle"));
    assert_eq!(paths(&all), ["a.txt", "keep.log", "sub/m.txt", "z.txt"]);
    assert_eq!((all.searched, all.matches), (5, 4));
    assert_eq!(all.files[2].groups[0].lines[0].text, "needle");
    let hidden = run(&GrepParams {
        hidden: true,
        ..params(&wt, "needle")
    });
    assert_eq!(paths(&hidden)[0], ".hidden");
    assert_eq!(paths(&run(&params(&wt.join("sub"), "needle"))), ["m.txt"]);
    let globbed = run(&GrepParams {
        glob: vec!["*.txt".into(), "!sub/".into()],
        ..params(&wt, "needle")
    });
    assert_eq!(paths(&globbed), ["a.txt", "z.txt"]);
    let missing = grep(&params(&tmp.0.join("gone"), "x"), &LIMITS, &|| false);
    assert!(matches!(missing, Err(Error::Io(why)) if why.contains("gone")));
    let bad = grep(&params(&wt, "a(?=b)"), &LIMITS, &|| false);
    assert!(matches!(bad, Err(Error::Query(why)) if why.contains("lookahead")));
    assert!(matches!(
        grep(&params(&wt, ""), &LIMITS, &|| false),
        Err(Error::Query(_))
    ));
}

#[test]
fn flags_and_counts_shape_the_answer() {
    let tmp = Temp::new("flags");
    tmp.put("a.rs", b"fn wait() {}\nfn waits() {}\nWAIT\nx.y\n");
    let lines = |p: GrepParams| -> Vec<usize> {
        let r = run(&p);
        let groups = r.files.iter().flat_map(|f| &f.groups);
        groups.flat_map(|g| g.lines.iter().map(|l| l.n)).collect()
    };
    let p = params(&tmp.0, "wait");
    assert_eq!(lines(p.clone()), [1, 2]);
    assert_eq!(
        lines(GrepParams {
            word: true,
            ..p.clone()
        }),
        [1]
    );
    let any = GrepParams {
        ignore_case: true,
        ..p.clone()
    };
    assert_eq!(lines(any.clone()), [1, 2, 3]);
    assert_eq!(lines(GrepParams { word: true, ..any }), [1, 3]);
    assert_eq!(lines(params(&tmp.0, "x.y")), [4]);
    assert_eq!(
        lines(GrepParams {
            fixed: true,
            ..params(&tmp.0, "x.")
        }),
        [4]
    );
    assert_eq!(lines(params(&tmp.0, "^fn \\w+s\\(")), [2]);
    let counted = run(&GrepParams { count: true, ..p });
    assert_eq!((counted.matches, counted.files[0].count), (2, 2));
    assert!(counted.files[0].groups.is_empty());
    assert_eq!(
        text::grep(&counted),
        "a.rs: 2\n2 matches in 1 file, 1 file searched\n"
    );
}

#[test]
fn a_long_matched_line_comes_back_clipped_with_its_length() {
    let tmp = Temp::new("clip");
    let long = format!("{}é needle {}", "a".repeat(4095), "b".repeat(900));
    tmp.put("long.txt", format!("short needle\n{long}\n").as_bytes());
    let r = run(&params(&tmp.0, "needle"));
    let lines = &r.files[0].groups[0].lines;
    assert_eq!((lines[0].clipped, lines[0].length), (false, None));
    assert_eq!(
        (lines[1].clipped, lines[1].length),
        (true, Some(long.len()))
    );
    assert_eq!(lines[1].text, "a".repeat(4095));
    let json = serde_json::to_value(&r).unwrap();
    let rows = &json["files"][0]["groups"][0]["lines"];
    assert_eq!(rows[0], serde_json::json!({"n": 1, "text": "short needle"}));
    assert_eq!(
        (rows[1]["clipped"].as_bool(), rows[1]["length"].as_u64()),
        (Some(true), Some(5005))
    );
    let shown = text::grep(&r);
    assert!(
        shown.contains(&format!("    2: {} … (5005 bytes)\n", "a".repeat(4095))),
        "{shown}"
    );
}

#[test]
fn each_limit_stops_the_query_and_names_itself() {
    let tmp = Temp::new("limits");
    tmp.put("a.txt", b"x\nx\n")
        .put("b.txt", b"x\n")
        .put("c.txt", b"x\n");
    let p = params(&tmp.0, "x");
    let small = |f: fn(&mut Limits)| {
        let mut limits = LIMITS;
        f(&mut limits);
        grep(&p, &limits, &|| false)
    };
    let over = |limit, max, at: &str| {
        Err(Error::Limit(Over {
            limit,
            max,
            at: at.into(),
        }))
    };
    assert_eq!(small(|l| l.files = 3).unwrap().matches, 4);
    assert_eq!(small(|l| l.files = 2), over(Limit::Files, 2, "c.txt"));
    assert_eq!(small(|l| l.matches = 4).unwrap().matches, 4);
    assert_eq!(small(|l| l.matches = 2), over(Limit::Matches, 2, "b.txt"));
    assert_eq!(small(|l| l.bytes = 8).unwrap().searched, 3);
    assert_eq!(small(|l| l.bytes = 7), over(Limit::Bytes, 7, "c.txt"));
    let calls = Cell::new(0);
    let third = || {
        calls.set(calls.get() + 1);
        calls.get() >= 3
    };
    assert_eq!(
        grep(&p, &LIMITS, &third),
        over(Limit::Deadline, 10, "b.txt")
    );
    let shown = small(|l| l.matches = 2).unwrap_err().to_string();
    assert_eq!(
        shown,
        "matches over 2 at b.txt; narrow the path, add a glob, or ask for a count"
    );
    let counted = GrepParams {
        count: true,
        ..p.clone()
    };
    let mut limits = LIMITS;
    limits.matches = 3;
    assert_eq!(grep(&counted, &limits, &|| false).unwrap().matches, 4);
}
