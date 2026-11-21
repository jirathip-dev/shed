//! The heap counts what is live, stops growing at its limit, and collects in slices of bounded work.
#![cfg(test)]

use shed_script::heap::{Full, Heap};
use shed_script::value::{Cell, Handle, Value};

const LIMIT: usize = 64 << 20;
const SLICE: u64 = 10_000;

fn finish(heap: &mut Heap, roots: &[Handle]) -> Vec<u64> {
    let mut slices = vec![heap.collect(roots, SLICE)];
    while heap.collecting() {
        slices.push(heap.collect(roots, SLICE));
    }
    slices
}

fn retry<T>(
    heap: &mut Heap,
    roots: &[Handle],
    mut grow: impl FnMut(&mut Heap) -> Result<T, Full>,
) -> Result<T, Full> {
    if heap.wants_collection() {
        finish(heap, roots);
    }
    loop {
        match grow(heap) {
            Err(Full::Collect) => {
                finish(heap, roots);
            }
            other => return other,
        }
    }
}

#[test]
fn growing_values_stop_on_memory() {
    let mut heap = Heap::new(LIMIT);
    let list = heap.alloc(Cell::Array(Vec::new())).unwrap();
    let roots = [list];
    let text: Box<str> = "x".repeat(1000).into();
    let stopped = loop {
        let item = retry(&mut heap, &roots, |heap| {
            heap.alloc(Cell::Str(text.clone()))
        });
        let pushed = item.and_then(|item| {
            retry(&mut heap, &[list, item], |heap| {
                heap.push(list, Value::Ref(item))
            })
        });
        match pushed {
            Ok(is_array) => assert!(is_array),
            Err(full) => break full,
        }
    };
    assert_eq!(stopped, Full::Memory);
    let each = 32 + 1000 + 16;
    let Some(Cell::Array(items)) = heap.get(list) else {
        panic!("the list was collected")
    };
    assert_eq!(items.len(), (LIMIT - 32) / each);
    assert_eq!(heap.bytes(), 32 + items.len() * each);
    assert!(heap.counts().collections > 0);
}

#[test]
fn garbage_is_not_counted() {
    let mut heap = Heap::new(LIMIT);
    let keep = heap.alloc(Cell::Object(Vec::new())).unwrap();
    let roots = [keep];
    let text: Box<str> = "y".repeat(64 << 10).into();
    for _ in 0..(1 << 14) {
        let item = retry(&mut heap, &roots, |heap| {
            heap.alloc(Cell::Str(text.clone()))
        })
        .unwrap();
        let set = retry(&mut heap, &[keep, item], |heap| {
            heap.set_field(keep, "last", Value::Ref(item))
        });
        assert_eq!(set, Ok(true));
    }
    finish(&mut heap, &roots);
    assert_eq!(heap.bytes(), (32 + 4 + 16) + (32 + (64 << 10)));
    assert!(heap.counts().collections > 100);
    assert!(heap.counts().peak_bytes < 4 << 20);
}

#[test]
fn a_collection_never_holds_a_step_over_its_slice() {
    let mut heap = Heap::new(LIMIT);
    let mut cells = Vec::new();
    for at in 0..200_000_u32 {
        let cell = Cell::Object(vec![("n".into(), Value::Number(at.into()))]);
        cells.push(Value::Ref(heap.alloc(cell).unwrap()));
    }
    let big = heap.alloc(Cell::Array(cells)).unwrap();
    let outer = Cell::Scope {
        parent: None,
        slots: vec![Value::Ref(big)],
    };
    let outer = heap.alloc(outer).unwrap();
    let inner = Cell::Scope {
        parent: Some(outer),
        slots: Vec::new(),
    };
    let inner = heap.alloc(inner).unwrap();
    for _ in 0..50_000 {
        heap.alloc(Cell::Str("garbage".into())).unwrap();
    }
    let roots = [inner];

    let mut slices = vec![heap.collect(&roots, SLICE)];
    assert!(heap.collecting());
    assert_eq!(heap.alloc(Cell::Array(Vec::new())), Err(Full::Collect));
    assert_eq!(heap.push(big, Value::Null), Err(Full::Collect));
    slices.extend(finish(&mut heap, &roots));

    assert!(slices.iter().all(|&used| used <= SLICE), "{slices:?}");
    let work: u64 = slices.iter().sum();
    assert_eq!(heap.counts().largest_collection, work);
    assert!(work > 600_000, "{work}");
    let Some(Cell::Array(items)) = heap.get(big) else {
        panic!("the array was collected")
    };
    let Some(&Value::Ref(last)) = items.last() else {
        panic!("the array lost its items")
    };
    assert!(heap.get(last).is_some());
    let live = (32 + 200_000 * 16) + 200_000 * (32 + 1 + 16) + 2 * 32 + 16;
    assert_eq!(heap.bytes(), live);
}
