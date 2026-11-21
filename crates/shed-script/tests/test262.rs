//! test262's files for the constructs the machine runs: `run.txt` passes, `out.txt` is refused by name.
#![cfg(test)]

mod common;

use std::path::{Path, PathBuf};

use shed_script::vm::{Limits, Run};
use shed_script::{Program, ScriptError};

use common::{Fixture, drive};

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/test262")
}

fn read(name: &str) -> String {
    std::fs::read_to_string(dir().join(name)).unwrap()
}

fn bare_asserts(code: &str) -> String {
    let mut out = String::with_capacity(code.len());
    let mut rest = code;
    while let Some(at) = rest.find("assert(") {
        let (before, after) = rest.split_at(at);
        out.push_str(before);
        let joined = before
            .chars()
            .last()
            .is_some_and(|c| c.is_alphanumeric() || "_$.".contains(c));
        out.push_str(if joined { "assert(" } else { "assert.ok(" });
        rest = &after["assert(".len()..];
    }
    out.push_str(rest);
    out
}

const AWAIT_DONE: &str = "\nfor (let $i = 0; $i < 1000 && !$async.done; $i++) await null\nif (!$async.done) throw fail(\"$DONE was not called\")\nif ($async.error !== undefined) throw $async.error\n";

fn is_async(code: &str) -> bool {
    let meta = code.split("---*/").next().unwrap_or_default();
    meta.lines()
        .any(|line| line.trim_start().starts_with("flags:") && line.contains("async"))
}

fn negative(code: &str) -> Option<(String, String)> {
    let meta = code.split("/*---").nth(1)?.split("---*/").next()?;
    let block = meta.split("negative:").nth(1)?;
    let field = |key: &str| {
        Some(
            block
                .split(key)
                .nth(1)?
                .split_whitespace()
                .next()?
                .to_owned(),
        )
    };
    Some((field("phase:")?, field("type:")?))
}

fn outcome(code: &str) -> Result<(), ScriptError> {
    let program = Program::parse(code)?;
    let host = Fixture::default();
    let mut run = Run::new(&program, Limits::default(), &host);
    drive(&mut run, &host).map(|_| ())
}

#[test]
fn test262_subset() {
    let harness = read("harness.js");
    let mut failed = Vec::new();
    let run = read("run.txt");
    for rel in run.lines() {
        let code = read(&format!("files/{rel}"));
        let tail = if is_async(&code) { AWAIT_DONE } else { "" };
        let source = format!("{harness}\n{}{tail}", bare_asserts(&code));
        let got = outcome(&source);
        let ok = match (negative(&code), &got) {
            (None, Ok(())) => true,
            (Some((_, kind)), Err(error)) => error.name == kind,
            _ => false,
        };
        if !ok {
            failed.push(format!("{rel}: {got:?}"));
        }
    }
    let out = read("out.txt");
    for line in out.lines() {
        let (words, rel) = line.split_once('\t').unwrap();
        let code = read(&format!("files/{rel}"));
        let got = outcome(&format!("{harness}\n{}", bare_asserts(&code)));
        let named = got.as_ref().is_err_and(|e| {
            e.name == "SyntaxError" && words.split('|').any(|word| e.message.contains(word))
        });
        if !named {
            failed.push(format!("{rel}: not refused by name ({words}): {got:?}"));
        }
    }
    let skipped = read("skipped.txt");
    for tag in skipped.lines().filter(|line| line.starts_with("# ")) {
        println!("skipped {}", &tag[2..]);
    }
    println!("run {}, out {}", run.lines().count(), out.lines().count());
    assert!(
        failed.is_empty(),
        "{} of test262 failed:\n{}",
        failed.len(),
        failed.join("\n")
    );
}
