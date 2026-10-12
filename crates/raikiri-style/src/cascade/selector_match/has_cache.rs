//! Memoized `:has()` results.
//!
//! A `:has()` in a compound other than the subject's is evaluated again for
//! the same anchor by every element whose selector reaches it, for example
//! `.card:has(img) p` once per `p` inside each `.card`. Each evaluation
//! searches the anchor's subtree or following siblings, so without a record
//! the cost grows with the number of such elements times the region size.
//! The result depends only on the anchor (its ancestor chain is fixed, see
//! [`MatchCaches`]), the relative selectors, and the matching parameters.

use super::*;

/// Caps the recorded results per matcher. Past it, new results are computed
/// without being recorded.
const MAX_ENTRIES: usize = 65_536;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct HasKey {
    anchor: StyleNodeId,
    list: ListKey,
    scope: Option<StyleNodeId>,
    allow_detached: bool,
    quirks_mode: StyleQuirksMode,
}

/// Address and length of a `:has()` argument list.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct ListKey {
    address: usize,
    len: usize,
}

/// One argument list, shared by every anchor's entry for it.
struct ListRecord {
    /// The list's selectors, retained so that a list allocated later at the
    /// same address can be told apart: its selectors cannot share these
    /// allocations unless they are the same selectors.
    selectors: Box<[Selector<RaikiriSelectorImpl>]>,
    /// Entries are valid only while they carry this generation; replacing
    /// the record for a different list at the same address bumps it.
    generation: u64,
}

impl ListRecord {
    fn describes(&self, relative_selectors: &[RelativeSelector<RaikiriSelectorImpl>]) -> bool {
        self.selectors.len() == relative_selectors.len()
            && self
                .selectors
                .iter()
                .zip(relative_selectors)
                .all(|(kept, live)| kept.thin_arc_heap_ptr() == live.selector.thin_arc_heap_ptr())
    }
}

struct HasEntry {
    generation: u64,
    matched: bool,
}

pub(super) struct HasCache {
    lists: FxHashMap<ListKey, ListRecord>,
    entries: FxHashMap<HasKey, HasEntry>,
    next_generation: u64,
    max_entries: usize,
}

impl Default for HasCache {
    fn default() -> Self {
        Self::with_limit(MAX_ENTRIES)
    }
}

impl HasCache {
    pub(super) fn with_limit(max_entries: usize) -> Self {
        Self {
            lists: FxHashMap::default(),
            entries: FxHashMap::default(),
            next_generation: 0,
            max_entries,
        }
    }

    /// The generation of the record for `list`, if it describes the live
    /// `relative_selectors`.
    fn current_generation(
        &self,
        list: ListKey,
        relative_selectors: &[RelativeSelector<RaikiriSelectorImpl>],
    ) -> Option<u64> {
        self.lists
            .get(&list)
            .filter(|record| record.describes(relative_selectors))
            .map(|record| record.generation)
    }
}

/// Whether any of `relative_selectors` matches with `anchor_id` as the
/// `:has()` anchor, answered from the record when one exists.
pub(super) fn matches<D: StyleDom>(
    ctx: MatchContext<'_, D>,
    relative_selectors: &[RelativeSelector<RaikiriSelectorImpl>],
    anchor_id: StyleNodeId,
    anchor_ancestors: &[StyleNodeId],
) -> bool {
    let list = ListKey {
        address: relative_selectors.as_ptr() as usize,
        len: relative_selectors.len(),
    };
    let key = HasKey {
        anchor: anchor_id,
        list,
        scope: ctx.scope,
        allow_detached: ctx.allow_detached,
        quirks_mode: ctx.quirks_mode,
    };
    {
        let cache = ctx.caches.has.borrow();
        if let Some(generation) = cache.current_generation(list, relative_selectors)
            && let Some(entry) = cache.entries.get(&key)
            && entry.generation == generation
        {
            return entry.matched;
        }
    }
    // No cache borrow is held across the search, which re-enters the matcher.
    let matched =
        has_relative_selector_matches(ctx, relative_selectors, anchor_id, anchor_ancestors);
    let mut cache = ctx.caches.has.borrow_mut();
    if cache.entries.len() >= cache.max_entries && !cache.entries.contains_key(&key) {
        return matched;
    }
    let generation = match cache.current_generation(list, relative_selectors) {
        Some(generation) => generation,
        None => {
            let generation = cache.next_generation;
            cache.next_generation = cache.next_generation.wrapping_add(1);
            cache.lists.insert(
                list,
                ListRecord {
                    selectors: relative_selectors
                        .iter()
                        .map(|relative| relative.selector.clone())
                        .collect(),
                    generation,
                },
            );
            generation
        }
    };
    cache.entries.insert(
        key,
        HasEntry {
            generation,
            matched,
        },
    );
    matched
}

#[cfg(test)]
impl MatchCaches {
    pub(crate) fn has_entries(&self) -> usize {
        self.has.borrow().entries.len()
    }

    pub(crate) fn has_lists(&self) -> usize {
        self.has.borrow().lists.len()
    }

    pub(crate) fn with_has_limit(max_entries: usize) -> Self {
        Self {
            has: RefCell::new(HasCache::with_limit(max_entries)),
            ..Self::default()
        }
    }
}
