//! The checks the build runs on each `methods/*.md` and `errors/*.md` file: a bad file fails the build, naming it.
#![cfg(test)]

#[path = "../../../build/collect.rs"]
mod collect;

use collect::{Record, check, record};

const METHOD: [&str; 2] = ["wire", "params"];
const ERROR: [&str; 2] = ["name", "code"];

fn error(file: &str, text: &str) -> Record {
    record(file, "Variant", text, &ERROR).unwrap()
}

#[test]
fn an_unknown_key_fails_naming_the_file() {
    let text = "wire: run\nwhere: here\n\nStart a terminal.\n";
    let err = record("methods/run.md", "Run", text, &METHOD).err();
    assert_eq!(err.as_deref(), Some("methods/run.md: unknown key `where`"));
}

#[test]
fn a_method_with_no_wire_name_fails_naming_the_file() {
    let run = record(
        "methods/run.md",
        "Run",
        "params: RunParams\n\nStart.\n",
        &METHOD,
    )
    .unwrap();
    let err = check(vec![run], &["wire"]).err();
    assert_eq!(err.as_deref(), Some("methods/run.md: no `wire`"));
}

#[test]
fn a_code_used_twice_fails_naming_both_files() {
    let rows = vec![
        error("errors/io.md", "name: IO\ncode: -32008\n\nIt failed.\n"),
        error(
            "errors/lost.md",
            "name: LOST\ncode: -32008\n\nIt is gone.\n",
        ),
    ];
    let err = check(rows, &ERROR).err();
    assert_eq!(
        err.as_deref(),
        Some("errors/lost.md: `code: -32008` is errors/io.md's too")
    );
}

#[test]
fn a_name_used_twice_fails_naming_both_files() {
    let rows = vec![
        error(
            "errors/disk.md",
            "name: IO\ncode: -32012\n\nThe disk failed.\n",
        ),
        error("errors/io.md", "name: IO\ncode: -32008\n\nIt failed.\n"),
    ];
    let err = check(rows, &ERROR).err();
    assert_eq!(
        err.as_deref(),
        Some("errors/io.md: `name: IO` is errors/disk.md's too")
    );
}
