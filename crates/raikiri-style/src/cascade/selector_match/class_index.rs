//! Short-lived class-token indexes for explicitly opted-in elements.

use std::hash::{Hash, Hasher};

use super::{CascadeError, FxHashSet, SelectorWork, StyleElement};

pub(super) const CLASS_INDEX_MIN_BYTES: usize = 64;
pub(super) const CLASS_INDEX_MAX_BYTES: usize = 65_536;
pub(super) const CLASS_INDEX_MAX_TOKENS: usize = 512;
const CLASS_INDEX_WARMUP: u8 = 3;
const CLASS_INDEX_SCAN_MULTIPLIER: usize = 4;

#[cfg(test)]
thread_local! {
    static STATS: std::cell::Cell<(usize, usize, usize)> = const {std::cell::Cell::new((0, 0, 0))};
    static RAW_STATS: std::cell::Cell<(usize, usize)> = const {std::cell::Cell::new((0, 0))};
}

#[cfg(test)]
pub(super) fn reset_stats() {
    STATS.set((0, 0, 0));
    RAW_STATS.set((0, 0));
}
#[cfg(test)]
pub(super) fn stats() -> (usize, usize, usize) {
    STATS.get()
}

#[cfg(test)]
pub(super) fn raw_stats() -> (usize, usize) {
    RAW_STATS.get()
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

enum LookupState<'a> {
    Warmup(u8),
    Legacy,
    Borrowed { source: &'a str, scanned: usize },
    Indexed(ClassIndex<'a>),
    ScanOnly(&'a str),
}

/// Owned by one compound call; no state crosses a DOM borrow or mutation.
pub(super) struct ClassLookup<'a> {
    state: LookupState<'a>,
}

impl Default for ClassLookup<'_> {
    fn default() -> Self {
        Self {
            state: LookupState::Warmup(0),
        }
    }
}

fn legacy_matches(elem: &impl StyleElement, class: &str, folded: bool) -> bool {
    if folded {
        elem.has_class_ascii_case_insensitive(class)
    } else {
        elem.has_class(class)
    }
}

fn scan_matches(source: &str, class: &str, folded: bool) -> (bool, usize) {
    if class.is_empty() {
        return (false, 0);
    }
    for token in source
        .split([' ', '\t', '\n', '\r', '\x0c'])
        .filter(|token| !token.is_empty())
    {
        #[cfg(test)]
        RAW_STATS.with(|stats| {
            let (tokens, bytes) = stats.get();
            stats.set((tokens + 1, bytes));
        });
        if if folded {
            token.eq_ignore_ascii_case(class)
        } else {
            token == class
        } {
            // Both addresses belong to source; the offset includes preceding separators.
            let scanned = token.as_ptr().addr() - source.as_ptr().addr() + token.len();
            #[cfg(test)]
            RAW_STATS.with(|stats| {
                let (tokens, bytes) = stats.get();
                stats.set((tokens, bytes + scanned));
            });
            return (true, scanned);
        }
    }
    #[cfg(test)]
    RAW_STATS.with(|stats| {
        let (tokens, bytes) = stats.get();
        stats.set((tokens, bytes + source.len()));
    });
    (false, source.len())
}

impl<'a> ClassLookup<'a> {
    pub(super) fn matches(
        &mut self,
        elem: &'a impl StyleElement,
        class: &str,
        folded: bool,
        work: &SelectorWork,
    ) -> Result<bool, CascadeError> {
        if let LookupState::Warmup(checks) = &mut self.state {
            if *checks < CLASS_INDEX_WARMUP {
                *checks += 1;
                return Ok(legacy_matches(elem, class, folded));
            }
            self.state = match elem.class_attribute_for_matching() {
                None => LookupState::Legacy,
                Some(source)
                    if (CLASS_INDEX_MIN_BYTES..=CLASS_INDEX_MAX_BYTES).contains(&source.len()) =>
                {
                    LookupState::Borrowed { source, scanned: 0 }
                }
                Some(source) => LookupState::ScanOnly(source),
            };
        }
        if let LookupState::Borrowed { source, scanned } = &self.state
            && *scanned > source.len().saturating_mul(CLASS_INDEX_SCAN_MULTIPLIER)
        {
            let source = *source;
            // Ordinary raw comparisons keep component-only accounting. Reserve
            // all source bytes before the one permitted index scan/allocation.
            work.charge(source.len() as u64)?;
            self.state = match ClassIndex::new(source, folded) {
                Some(index) => LookupState::Indexed(index),
                None => LookupState::ScanOnly(source),
            };
        }
        Ok(match &mut self.state {
            LookupState::Warmup(_) | LookupState::Legacy => legacy_matches(elem, class, folded),
            LookupState::Borrowed { source, scanned } => {
                let (matched, bytes) = scan_matches(source, class, folded);
                *scanned = scanned.saturating_add(bytes);
                matched
            }
            LookupState::Indexed(index) => index.contains(class),
            LookupState::ScanOnly(source) => scan_matches(source, class, folded).0,
        })
    }
}

#[cfg(test)]
mod tests;
