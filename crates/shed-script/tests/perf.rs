//! The fixed scripts' gate (exact counts, and ceilings) and `just script-bench`'s report of targets 3, 4 and 5, and big.js's hold.
#![cfg(test)]

use std::path::Path;
use std::time::{Duration, Instant};

use shed_script::Program;
use shed_script::vm::{Counts, Limits, Run, Step};

const RUNS: usize = 5;
const SLICE: u64 = 10_000;

fn fixed(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/perf")
        .join(name);
    std::fs::read_to_string(&path).unwrap()
}

fn report(what: &str, target: &str, mut runs: Vec<f64>) {
    runs.sort_by(f64::total_cmp);
    println!(
        "{what:<52} {:>8.3}   target {target:<15} range {:.3} to {:.3}",
        runs[RUNS / 2],
        runs[0],
        runs[RUNS - 1]
    );
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn parse_ms(code: &str) -> f64 {
    let mut times: Vec<f64> = (0..1000)
        .map(|_| {
            let start = Instant::now();
            Program::parse(code).unwrap();
            ms(start.elapsed())
        })
        .collect();
    times.sort_by(f64::total_cmp);
    times[500]
}

fn scripts() -> Vec<(String, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/perf");
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| Path::new(name).extension().is_some_and(|e| e == "js"))
        .collect();
    names.sort();
    names.into_iter().map(|name| (fixed(&name), name)).collect()
}

fn ceiling(code: &str) -> Option<(u64, usize)> {
    let line = code.lines().next()?.strip_prefix("// ceiling: steps ")?;
    let (steps, bytes) = line.split_once(", peak_bytes ")?;
    Some((steps.parse().unwrap(), bytes.parse().unwrap()))
}

fn counts(code: &str) -> Counts {
    let program = Program::parse(code).unwrap();
    let mut run = Run::new(&program, Limits::default());
    loop {
        if let Step::Done(outcome) = run.step() {
            outcome.unwrap();
            return run.counts();
        }
    }
}

fn table() -> String {
    let rows: Vec<String> = scripts()
        .into_iter()
        .filter(|(code, _)| ceiling(code).is_some())
        .map(|(code, name)| {
            let c = counts(&code);
            format!(
                "  \"{name}\": {{\"steps\": {}, \"peak_bytes\": {}, \"collections\": {}, \"largest_collection\": {}}}",
                c.steps, c.peak_bytes, c.collections, c.largest_collection
            )
        })
        .collect();
    format!("{{\n{}\n}}\n", rows.join(",\n"))
}

#[test]
fn counts_match_the_baseline() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/perf/counts.json");
    let first = table();
    assert_eq!(first, table(), "the counts differ between two runs");
    if std::env::var_os("SHED_BLESS").is_some() {
        std::fs::write(&path, &first).unwrap();
    }
    let committed = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        committed, first,
        "counts.json is stale: run `just generate`"
    );
}

#[test]
fn counts_stay_under_their_ceilings() {
    let mut checked = 0;
    for (code, name) in scripts() {
        let Some((steps, peak_bytes)) = ceiling(&code) else {
            continue;
        };
        let c = counts(&code);
        assert!(
            c.steps <= steps,
            "{name}: {} steps, over its ceiling of {steps}",
            c.steps
        );
        assert!(
            c.peak_bytes <= peak_bytes,
            "{name}: {} peak bytes, over its ceiling of {peak_bytes}",
            c.peak_bytes
        );
        checked += 1;
    }
    assert!(checked >= 2, "only {checked} fixed scripts have a ceiling");
}

struct Timed {
    steps_per_second: f64,
    slice: Duration,
    collection: Duration,
}

fn timed(code: &str) -> Timed {
    let program = Program::parse(code).unwrap();
    let mut run = Run::new(&program, Limits::default());
    let (started, mut slice, mut collection, mut longest) = (
        Instant::now(),
        Duration::ZERO,
        Duration::ZERO,
        Duration::ZERO,
    );
    loop {
        let (at, before) = (Instant::now(), run.counts().steps);
        let step = run.step();
        let took = at.elapsed();
        slice = slice.max(took);
        if run.counts().steps - before < SLICE {
            collection += took;
            longest = longest.max(collection);
        } else {
            collection = Duration::ZERO;
        }
        if let Step::Done(outcome) = step {
            outcome.unwrap();
            break;
        }
    }
    let steps = f64::from(u32::try_from(run.counts().steps).unwrap());
    Timed {
        steps_per_second: steps / started.elapsed().as_secs_f64(),
        slice,
        collection: longest,
    }
}

#[test]
#[ignore = "timed: run by `just script-bench`, in release"]
fn speed_report() {
    println!("script engine, {RUNS} runs each; times in ms");
    let ten_kb = fixed("ten-kb.js");
    report(
        "target 4: ten-kb.js parsed and compiled (median of 1,000)",
        "at most 2 ms",
        (0..RUNS).map(|_| parse_ms(&ten_kb)).collect(),
    );
    let spins: Vec<Timed> = (0..RUNS).map(|_| timed(&fixed("spin.js"))).collect();
    report(
        "target 3: spin.js, million steps a second",
        "at least 25",
        spins.iter().map(|t| t.steps_per_second / 1e6).collect(),
    );
    let shapes: Vec<Timed> = (0..RUNS).map(|_| timed(&fixed("shapes.js"))).collect();
    report(
        "target 3: shapes.js, million steps a second",
        "at least 25",
        shapes.iter().map(|t| t.steps_per_second / 1e6).collect(),
    );
    let bigs: Vec<Timed> = (0..RUNS).map(|_| timed(&fixed("big.js"))).collect();
    report(
        "big.js, the longest slice (one built-in's hold)",
        "at most 30 ms",
        bigs.iter().map(|t| ms(t.slice)).collect(),
    );
    let holds: Vec<Timed> = (0..RUNS).map(|_| timed(&fixed("hold.js"))).collect();
    report(
        "target 5: hold.js, the longest slice",
        "at most 1 ms",
        holds.iter().map(|t| ms(t.slice)).collect(),
    );
    report(
        "target 5: hold.js, the longest collection",
        "at most 150 ms",
        holds.iter().map(|t| ms(t.collection)).collect(),
    );
}
