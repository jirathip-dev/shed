//! What a script holds: `Copy` values, and the cells in the heap that everything which can grow lives in.

use std::sync::Arc;

/// A cell's place in the heap; only the heap makes one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Handle(pub(crate) usize);

/// One value. `Copy`: everything that can grow lives in the heap, behind a handle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value {
    /// `undefined`.
    Undefined,
    /// `null`.
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// A number, an f64.
    Number(f64),
    /// A cell in the heap.
    Ref(Handle),
    /// A built-in: a global, or a function of one.
    Native(Builtin),
    /// A built-in method bound to the string or array it was read from.
    Method(Builtin, Handle),
}

/// A built-in by its owner's row and its place in the row; only the built-ins' table makes one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Builtin {
    pub(crate) owner: u8,
    pub(crate) at: u8,
}

impl Value {
    /// The cell it holds on to, if any.
    #[must_use]
    pub fn handle(self) -> Option<Handle> {
        match self {
            Self::Ref(h) | Self::Method(_, h) => Some(h),
            _ => None,
        }
    }
}

/// What a handle points at.
#[derive(Debug, Clone, PartialEq)]
pub enum Cell {
    /// A string.
    Str(Box<str>),
    /// An array.
    Array(Vec<Value>),
    /// An object: string keys in insertion order, no prototype.
    Object(Vec<(Arc<str>, Value)>),
    /// A scope: its variables, and the scope it is inside.
    Scope {
        /// The enclosing scope.
        parent: Option<Handle>,
        /// Its variables, by slot.
        slots: Vec<Value>,
    },
    /// A function with the scope it was made in.
    Closure {
        /// Its code, by index in the program.
        function: usize,
        /// The scope it closes over.
        scope: Option<Handle>,
    },
    /// A promise: how it settled, or what waits for it.
    Promise {
        /// Pending, fulfilled or rejected.
        state: Settled,
        /// What runs when it settles.
        waiters: Vec<Waiter>,
        /// The items a `Promise.all` still waits for.
        left: usize,
    },
    /// A path under `shed` on its way to a call: `shed.file` before `shed.file.read(…)`.
    HostPath(Box<str>),
    /// An async call waiting to go on: its frames, operand stack and handlers.
    Task(Box<Task>),
}

/// A suspended async call; only the machine makes one.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Task {
    pub(crate) frames: Vec<Frame>,
    pub(crate) stack: Vec<Value>,
    pub(crate) handlers: Vec<Handler>,
    pub(crate) promise: Option<Handle>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Frame {
    pub(crate) code: usize,
    pub(crate) pc: usize,
    pub(crate) base: usize,
    pub(crate) env: Option<Handle>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Handler {
    pub(crate) frame: usize,
    pub(crate) pc: usize,
    pub(crate) stack: usize,
    pub(crate) env: Option<Handle>,
}

/// How a promise stands.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Settled {
    /// Not yet settled.
    Pending,
    /// Settled with a value.
    Fulfilled(Value),
    /// Settled with a thrown value.
    Rejected(Value),
}

/// What waits for a promise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Waiter {
    /// A suspended task, resumed with the outcome.
    Task(Handle),
    /// A promise that settles the same way.
    Adopt(Handle),
    /// A `Promise.all`: its promise, its array and the item's place in it.
    All(Handle, Handle, usize),
    /// A `Promise.race`: its promise.
    Race(Handle),
}

impl Waiter {
    fn handles(self) -> impl Iterator<Item = Handle> {
        let (a, b) = match self {
            Self::Task(h) | Self::Adopt(h) | Self::Race(h) => (Some(h), None),
            Self::All(out, items, _) => (Some(out), Some(items)),
        };
        a.into_iter().chain(b)
    }
}

const CELL: usize = 32;
pub(crate) const SLOT: usize = 16;

impl Cell {
    /// What it counts toward the memory limit: 32, plus its text, 16 a slot, key bytes plus 16 a field.
    #[must_use]
    pub fn bytes(&self) -> usize {
        CELL.saturating_add(match self {
            Self::Str(text) | Self::HostPath(text) => text.len(),
            Self::Array(items) | Self::Scope { slots: items, .. } => {
                items.len().saturating_mul(SLOT)
            }
            Self::Object(fields) => fields.iter().fold(0, |sum, (key, _)| {
                sum.saturating_add(key.len()).saturating_add(SLOT)
            }),
            Self::Closure { .. } | Self::Promise { .. } => 0,
            Self::Task(task) => task
                .stack
                .len()
                .saturating_add(task.frames.len())
                .saturating_add(task.handlers.len())
                .saturating_mul(SLOT),
        })
    }

    pub(crate) fn child(&self, at: usize) -> Option<Value> {
        match self {
            Self::Str(_) | Self::HostPath(_) => None,
            Self::Array(items) => items.get(at).copied(),
            Self::Object(fields) => fields.get(at).map(|(_, value)| *value),
            Self::Scope { parent, slots } => match at.checked_sub(1) {
                None => Some(parent.map_or(Value::Undefined, Value::Ref)),
                Some(slot) => slots.get(slot).copied(),
            },
            Self::Closure { scope, .. } => {
                (at == 0).then(|| scope.map_or(Value::Undefined, Value::Ref))
            }
            Self::Promise { state, waiters, .. } => {
                let settled = match state {
                    Settled::Fulfilled(v) | Settled::Rejected(v) => Some(*v),
                    Settled::Pending => None,
                };
                let held = waiters.iter().flat_map(|w| w.handles()).map(Value::Ref);
                settled.into_iter().chain(held).nth(at)
            }
            Self::Task(task) => {
                let envs = task.frames.iter().filter_map(|f| f.env);
                let handlers = task.handlers.iter().filter_map(|h| h.env);
                let held = envs.chain(handlers).chain(task.promise).map(Value::Ref);
                task.stack.iter().copied().chain(held).nth(at)
            }
        }
    }
}
