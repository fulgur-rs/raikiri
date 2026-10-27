//! Short-lived class-token indexes for explicitly opted-in elements.

use std::hash::{Hash, Hasher};

use super::{CascadeError, FxHashSet, SelectorWork, StyleElement};

pub(super) const CLASS_INDEX_MIN_BYTES: usize = 64;
pub(super) const CLASS_INDEX_MAX_BYTES: usize = 65_536;
pub(super) const CLASS_INDEX_MAX_TOKENS: usize = 512;
const CLASS_INDEX_WARMUP: u8 = 3;

#[cfg(test)]
thread_local! {
    static STATS: std::cell::Cell<(usize, usize, usize)> = const {std::cell::Cell::new((0, 0, 0))};
}

#[cfg(test)]
pub(super) fn reset_stats() {
    STATS.set((0, 0, 0));
}
#[cfg(test)]
pub(super) fn stats() -> (usize, usize, usize) {
    STATS.get()
}

#[derive(Clone, Copy)]
struct Token<'a> {
    value: &'a str,
    folded: bool,
}

impl PartialEq for Token<'_> {
    fn eq(&self, other: &Self) -> bool {
        #[cfg(test)]
        STATS.with(|stats| {
            let (tokens, bytes, equalities) = stats.get();
            stats.set((tokens, bytes, equalities + 1));
        });
        self.folded == other.folded
            && if self.folded {
                self.value.eq_ignore_ascii_case(other.value)
            } else {
                self.value == other.value
            }
    }
}
impl Eq for Token<'_> {}

impl Hash for Token<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        #[cfg(test)]
        STATS.with(|stats| {
            let (tokens, bytes, equalities) = stats.get();
            stats.set((tokens, bytes + self.value.len(), equalities));
        });
        self.folded.hash(state);
        if self.folded {
            for byte in self.value.bytes() {
                byte.to_ascii_lowercase().hash(state);
            }
            // Keep the encoding prefix-free, as str hashing does.
            0xffu8.hash(state);
        } else {
            self.value.hash(state);
        }
    }
}

struct ClassIndex<'a> {
    tokens: FxHashSet<Token<'a>>,
    folded: bool,
}

impl<'a> ClassIndex<'a> {
    fn new(value: &'a str, folded: bool) -> Option<Self> {
        let mut tokens = FxHashSet::default();
        for (count, token) in value
            .split([' ', '\t', '\n', '\r', '\x0c'])
            .filter(|token| !token.is_empty())
            .enumerate()
        {
            #[cfg(test)]
            STATS.with(|stats| {
                let (tokens, bytes, equalities) = stats.get();
                stats.set((tokens + 1, bytes, equalities));
            });
            if count == CLASS_INDEX_MAX_TOKENS {
                return None;
            }
            tokens.insert(Token {
                value: token,
                folded,
            });
        }
        Some(Self { tokens, folded })
    }

    fn contains(&self, class: &str) -> bool {
        !class.is_empty()
            && self.tokens.contains(&Token {
                value: class,
                folded: self.folded,
            })
    }
}

/// Owned by one compound call; no state crosses a DOM borrow or mutation.
#[derive(Default)]
pub(super) struct ClassLookup<'a> {
    checks: u8,
    index: Option<ClassIndex<'a>>,
}

impl<'a> ClassLookup<'a> {
    pub(super) fn matches(
        &mut self,
        elem: &'a impl StyleElement,
        class: &str,
        folded: bool,
        work: &SelectorWork,
    ) -> Result<bool, CascadeError> {
        if self.checks < CLASS_INDEX_WARMUP {
            self.checks += 1;
        } else if self.checks == CLASS_INDEX_WARMUP {
            self.checks += 1;
            if let Some(value) = elem.class_attribute_for_matching()
                && (CLASS_INDEX_MIN_BYTES..=CLASS_INDEX_MAX_BYTES).contains(&value.len())
            {
                // Reserve one logical unit per source byte before scanning or
                // allocating. Ordinary class string comparisons retain their
                // existing component-only accounting.
                work.charge(value.len() as u64)?;
                self.index = ClassIndex::new(value, folded);
            }
        }
        if let Some(index) = &self.index {
            return Ok(index.contains(class));
        }
        Ok(if folded {
            elem.has_class_ascii_case_insensitive(class)
        } else {
            elem.has_class(class)
        })
    }
}

#[cfg(test)]
mod tests;
