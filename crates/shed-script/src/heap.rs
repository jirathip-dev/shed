//! The arena every cell lives in, its live-byte count, and a mark and sweep taken in slices.

use std::mem;

use crate::value::{Cell, Handle, SLOT, Value};

const MIN_TRIGGER: usize = 1 << 20;

/// Why the heap would not grow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Full {
    /// A collection runs, or one could free room: finish one with [`Heap::collect`], then try again.
    Collect,
    /// Over the limit with nothing to collect since the last sweep: the run ends with `MEMORY`.
    Memory,
}

/// What the collector has done over the run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HeapCounts {
    /// The most bytes that were live after a sweep.
    pub peak_bytes: usize,
    /// Collections finished.
    pub collections: u64,
    /// The units of work (cells, slots and fields visited) of the largest collection.
    pub largest_collection: u64,
}

#[derive(Debug)]
struct Slot {
    cell: Cell,
    bytes: usize,
    black: bool,
}

#[derive(Debug, Default)]
enum Phase {
    #[default]
    Idle,
    Mark(Vec<(Handle, usize)>),
    Sweep(usize, usize),
}

/// The arena: cells by handle, every byte counted, collected only through [`Heap::collect`].
#[derive(Debug)]
pub struct Heap {
    slots: Vec<Option<Slot>>,
    free: Vec<usize>,
    limit: usize,
    live: usize,
    since: usize,
    swept: bool,
    black: bool,
    phase: Phase,
    work: u64,
    counts: HeapCounts,
}

impl Heap {
    /// An empty heap that holds at most `limit` bytes of live cells.
    #[must_use]
    pub fn new(limit: usize) -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
            limit,
            live: 0,
            since: 0,
            swept: true,
            black: false,
            phase: Phase::Idle,
            work: 0,
            counts: HeapCounts::default(),
        }
    }

    /// The bytes held: exact after a sweep, the live ones plus all allocated since otherwise.
    #[must_use]
    pub fn bytes(&self) -> usize {
        self.live.saturating_add(self.since)
    }

    /// What the collector has done.
    #[must_use]
    pub fn counts(&self) -> HeapCounts {
        self.counts
    }

    /// A collection runs: the script's code waits until [`Heap::collect`] finishes it.
    #[must_use]
    pub fn collecting(&self) -> bool {
        !matches!(self.phase, Phase::Idle)
    }

    /// The bytes allocated since the last sweep passed what it left live (at least 1 MiB): start one.
    #[must_use]
    pub fn wants_collection(&self) -> bool {
        self.since >= self.live.max(MIN_TRIGGER)
    }

    /// The cell behind a handle.
    #[must_use]
    pub fn get(&self, handle: Handle) -> Option<&Cell> {
        self.slot(handle).map(|slot| &slot.cell)
    }

    /// Puts a cell in the heap.
    ///
    /// # Errors
    ///
    /// [`Full`] while a collection runs, or when the cell would pass the limit.
    pub fn alloc(&mut self, cell: Cell) -> Result<Handle, Full> {
        let bytes = cell.bytes();
        self.reserve(bytes)?;
        let slot = Some(Slot {
            cell,
            bytes,
            black: self.black,
        });
        Ok(Handle(if let Some(at) = self.free.pop() {
            if let Some(free) = self.slots.get_mut(at) {
                *free = slot;
            }
            at
        } else {
            self.slots.push(slot);
            self.slots.len().saturating_sub(1)
        }))
    }

    /// Appends to an array; `false` when the handle is not one.
    ///
    /// # Errors
    ///
    /// [`Full`] while a collection runs, or when the slot would pass the limit.
    pub fn push(&mut self, array: Handle, value: Value) -> Result<bool, Full> {
        if !matches!(self.get(array), Some(Cell::Array(_))) {
            return Ok(false);
        }
        self.reserve(SLOT)?;
        if let Some(Slot {
            cell: Cell::Array(items),
            bytes,
            ..
        }) = self.slot_mut(array)
        {
            items.push(value);
            *bytes = bytes.saturating_add(SLOT);
        }
        Ok(true)
    }

    /// Sets an object's field, adding it last when it is new; `false` when the handle is not an object.
    ///
    /// # Errors
    ///
    /// [`Full`] while a collection runs, or when a new field would pass the limit.
    pub fn set_field(&mut self, object: Handle, key: &str, value: Value) -> Result<bool, Full> {
        let Some(Cell::Object(fields)) = self.get(object) else {
            return Ok(false);
        };
        let grow = if fields.iter().any(|(name, _)| &**name == key) {
            0
        } else {
            key.len().saturating_add(SLOT)
        };
        self.reserve(grow)?;
        if let Some(Slot {
            cell: Cell::Object(fields),
            bytes,
            ..
        }) = self.slot_mut(object)
        {
            match fields.iter_mut().find(|(name, _)| &**name == key) {
                Some((_, old)) => *old = value,
                None => fields.push((key.into(), value)),
            }
            *bytes = bytes.saturating_add(grow);
        }
        Ok(true)
    }

    /// Works on the collection for at most `units` cells, slots and fields, starting one from `roots`
    /// when none runs; returns the units used. The collection is over when [`Heap::collecting`] is false.
    pub fn collect(&mut self, roots: &[Handle], units: u64) -> u64 {
        if let Phase::Idle = self.phase {
            self.black = !self.black;
            self.work = 0;
            self.phase = Phase::Mark(roots.iter().map(|&root| (root, 0)).collect());
        }
        let mut used = 0;
        while used < units {
            match mem::take(&mut self.phase) {
                Phase::Idle => break,
                Phase::Mark(mut stack) => {
                    self.phase = if self.mark(&mut stack, units, &mut used) {
                        Phase::Sweep(0, 0)
                    } else {
                        Phase::Mark(stack)
                    };
                }
                Phase::Sweep(at, live) => self.sweep(at, live, units, &mut used),
            }
        }
        self.work = self.work.saturating_add(used);
        used
    }

    fn mark(&mut self, stack: &mut Vec<(Handle, usize)>, units: u64, used: &mut u64) -> bool {
        while *used < units {
            let Some((handle, mut at)) = stack.pop() else {
                return true;
            };
            *used += 1;
            let black = self.black;
            let Some(slot) = self.slot_mut(handle) else {
                continue;
            };
            if at == 0 {
                if slot.black == black {
                    continue;
                }
                slot.black = black;
            }
            while *used < units {
                let Some(child) = self.get(handle).and_then(|cell| cell.child(at)) else {
                    break;
                };
                *used += 1;
                at += 1;
                if let Value::Ref(child) = child
                    && self.slot(child).is_some_and(|slot| slot.black != black)
                {
                    stack.push((child, 0));
                }
            }
            if self.get(handle).and_then(|cell| cell.child(at)).is_some() {
                stack.push((handle, at));
            }
        }
        stack.is_empty()
    }

    fn sweep(&mut self, mut at: usize, mut live: usize, units: u64, used: &mut u64) {
        while *used < units {
            let Some(entry) = self.slots.get_mut(at) else {
                self.live = live;
                self.since = 0;
                self.swept = true;
                self.counts.peak_bytes = self.counts.peak_bytes.max(live);
                self.counts.collections += 1;
                self.counts.largest_collection = self
                    .counts
                    .largest_collection
                    .max(self.work.saturating_add(*used));
                self.phase = Phase::Idle;
                return;
            };
            match entry {
                Some(slot) if slot.black == self.black => live = live.saturating_add(slot.bytes),
                Some(_) => {
                    *entry = None;
                    self.free.push(at);
                }
                None => {}
            }
            *used += 1;
            at += 1;
        }
        self.phase = Phase::Sweep(at, live);
    }

    fn reserve(&mut self, bytes: usize) -> Result<(), Full> {
        if self.collecting() {
            return Err(Full::Collect);
        }
        if self.bytes().saturating_add(bytes) > self.limit {
            return Err(if self.swept {
                Full::Memory
            } else {
                Full::Collect
            });
        }
        self.since = self.since.saturating_add(bytes);
        self.swept = false;
        Ok(())
    }

    fn slot(&self, handle: Handle) -> Option<&Slot> {
        self.slots.get(handle.0).and_then(Option::as_ref)
    }

    fn slot_mut(&mut self, handle: Handle) -> Option<&mut Slot> {
        self.slots.get_mut(handle.0).and_then(Option::as_mut)
    }
}
