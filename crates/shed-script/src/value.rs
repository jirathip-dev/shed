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
}

const CELL: usize = 32;
pub(crate) const SLOT: usize = 16;

impl Cell {
    /// What it counts toward the memory limit: 32, plus its text, 16 a slot, key bytes plus 16 a field.
    #[must_use]
    pub fn bytes(&self) -> usize {
        CELL.saturating_add(match self {
            Self::Str(text) => text.len(),
            Self::Array(items) | Self::Scope { slots: items, .. } => {
                items.len().saturating_mul(SLOT)
            }
            Self::Object(fields) => fields.iter().fold(0, |sum, (key, _)| {
                sum.saturating_add(key.len()).saturating_add(SLOT)
            }),
            Self::Closure { .. } => 0,
        })
    }

    pub(crate) fn child(&self, at: usize) -> Option<Value> {
        match self {
            Self::Str(_) => None,
            Self::Array(items) => items.get(at).copied(),
            Self::Object(fields) => fields.get(at).map(|(_, value)| *value),
            Self::Scope { parent, slots } => match at.checked_sub(1) {
                None => Some(parent.map_or(Value::Undefined, Value::Ref)),
                Some(slot) => slots.get(slot).copied(),
            },
            Self::Closure { scope, .. } => {
                (at == 0).then(|| scope.map_or(Value::Undefined, Value::Ref))
            }
        }
    }
}
