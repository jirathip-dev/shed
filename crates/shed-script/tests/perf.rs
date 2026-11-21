//! The speed report of `just script-bench`: targets 3, 4 and 5, each the median and range of 5 runs.
#![cfg(test)]

use std::path::Path;
use std::time::{Duration, Instant};

use shed_script::heap::{Full, Heap};
use shed_script::value::{Cell, Handle, Value};

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
            shed_script::parse(code).unwrap();
            ms(start.elapsed())
        })
        .collect();
    times.sort_by(f64::total_cmp);
    times[500]
}

#[derive(Default)]
struct Pauses {
    slice: Duration,
    collection: Duration,
}

fn collect(heap: &mut Heap, roots: &[Handle], pauses: &mut Pauses) {
    let started = Instant::now();
    loop {
        let slice = Instant::now();
        heap.collect(roots, SLICE);
        pauses.slice = pauses.slice.max(slice.elapsed());
        if !heap.collecting() {
            break;
        }
    }
    pauses.collection = pauses.collection.max(started.elapsed());
}

fn alloc(heap: &mut Heap, roots: &[Handle], cell: &Cell, pauses: &mut Pauses) -> Handle {
    if heap.wants_collection() {
        collect(heap, roots, pauses);
    }
    loop {
        match heap.alloc(cell.clone()) {
            Ok(handle) => return handle,
            Err(Full::Collect) => collect(heap, roots, pauses),
            Err(Full::Memory) => panic!("hold's heap is over its limit"),
        }
    }
}

fn pair(heap: &mut Heap, roots: &[Handle], at: u32, pauses: &mut Pauses) -> Handle {
    let n = Value::Number(at.into());
    let array = alloc(heap, roots, &Cell::Array(vec![n, n]), pauses);
    let mut held = roots.to_vec();
    held.push(array);
    let object = Cell::Object(vec![("n".into(), n), ("pair".into(), Value::Ref(array))]);
    alloc(heap, &held, &object, pauses)
}

fn hold_from_rust() -> Pauses {
    let mut pauses = Pauses::default();
    let mut heap = Heap::new(64 << 20);
    let kept = heap.alloc(Cell::Array(Vec::new())).unwrap();
    for at in 0..380_000 {
        let item = pair(&mut heap, &[kept], at, &mut pauses);
        loop {
            match heap.push(kept, Value::Ref(item)) {
                Ok(_) => break,
                Err(Full::Collect) => collect(&mut heap, &[kept, item], &mut pauses),
                Err(Full::Memory) => panic!("hold's heap is over its limit"),
            }
        }
    }
    for at in 0..1_400_000 {
        pair(&mut heap, &[kept], at, &mut pauses);
    }
    pauses
}

#[test]
#[ignore = "timed: run by `just script-bench`, in release"]
fn speed_report() {
    println!("script engine, {RUNS} runs each; times in ms");
    let ten_kb = fixed("ten-kb.js");
    report(
        "target 4: ten-kb.js parsed (median of 1,000)",
        "at most 2 ms",
        (0..RUNS).map(|_| parse_ms(&ten_kb)).collect(),
    );
    let holds: Vec<Pauses> = (0..RUNS).map(|_| hold_from_rust()).collect();
    report(
        "target 5: hold, from Rust: the longest slice",
        "at most 1 ms",
        holds.iter().map(|p| ms(p.slice)).collect(),
    );
    report(
        "target 5: hold, from Rust: the longest collection",
        "at most 150 ms",
        holds.iter().map(|p| ms(p.collection)).collect(),
    );
    println!("{:<52} new: the base runs no script", "target 3: spin.js");
}
