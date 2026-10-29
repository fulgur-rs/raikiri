//! Pending counter scopes, indexed only when a reset queries for obscuring.

use raikiri_traits::Symbol;
use std::collections::HashSet;

/// Newly instantiated names are unique within a bucket. Increments and sets
/// register only when no level is active; resets remove before registering.
/// Boxing the index keeps the unindexed bucket's inline storage compact.
#[derive(Debug)]
pub(super) enum CounterScopeBucket {
    Unindexed(Vec<Symbol>),
    #[allow(clippy::box_collection)]
    // Keep each pending bucket at 24 bytes on the pinned toolchain.
    Indexed(Box<HashSet<Symbol>>),
}

impl Default for CounterScopeBucket {
    #[inline]
    fn default() -> Self {
        Self::Unindexed(Vec::new())
    }
}

impl CounterScopeBucket {
    #[inline]
    pub(super) fn register(&mut self, name: Symbol) {
        match self {
            Self::Unindexed(names) => names.push(name),
            Self::Indexed(names) => {
                names.insert(name);
            }
        }
    }

    pub(super) fn remove(&mut self, name: &Symbol) -> bool {
        match self {
            Self::Unindexed(names) if names.is_empty() => false,
            Self::Unindexed(names) => {
                let mut index = HashSet::with_capacity(names.len());
                for pending in std::mem::take(names) {
                    #[cfg(test)]
                    super::tests::record_counter_name_work(0);
                    index.insert(pending);
                }
                let removed = index.remove(name);
                *self = Self::Indexed(Box::new(index));
                removed
            }
            Self::Indexed(names) => names.remove(name),
        }
    }
}
