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
    /// Address and length of the `:has()` argument list.
    list: usize,
    len: usize,
    scope: Option<StyleNodeId>,
    allow_detached: bool,
    quirks_mode: StyleQuirksMode,
}

struct HasEntry {
    /// The argument list's selectors, retained so that a list allocated
    /// later at the same address can be told apart: its selectors cannot
    /// share these allocations unless they are the same selectors.
    selectors: Box<[Selector<RaikiriSelectorImpl>]>,
    matched: bool,
}

impl HasEntry {
    fn describes(&self, relative_selectors: &[RelativeSelector<RaikiriSelectorImpl>]) -> bool {
        self.selectors.len() == relative_selectors.len()
            && self
                .selectors
                .iter()
                .zip(relative_selectors)
                .all(|(kept, live)| kept.thin_arc_heap_ptr() == live.selector.thin_arc_heap_ptr())
    }
}

pub(super) struct HasCache {
    entries: FxHashMap<HasKey, HasEntry>,
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
            entries: FxHashMap::default(),
            max_entries,
        }
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
    let key = HasKey {
        anchor: anchor_id,
        list: relative_selectors.as_ptr() as usize,
        len: relative_selectors.len(),
        scope: ctx.scope,
        allow_detached: ctx.allow_detached,
        quirks_mode: ctx.quirks_mode,
    };
    if let Some(entry) = ctx.caches.has.borrow().entries.get(&key)
        && entry.describes(relative_selectors)
    {
        return entry.matched;
    }
    // No cache borrow is held across the search, which re-enters the matcher.
    let matched =
        has_relative_selector_matches(ctx, relative_selectors, anchor_id, anchor_ancestors);
    let mut cache = ctx.caches.has.borrow_mut();
    if cache.entries.len() < cache.max_entries || cache.entries.contains_key(&key) {
        cache.entries.insert(
            key,
            HasEntry {
                selectors: relative_selectors
                    .iter()
                    .map(|relative| relative.selector.clone())
                    .collect(),
                matched,
            },
        );
    }
    matched
}

#[cfg(test)]
impl MatchCaches {
    pub(crate) fn has_entries(&self) -> usize {
        self.has.borrow().entries.len()
    }

    pub(crate) fn with_has_limit(max_entries: usize) -> Self {
        Self {
            has: RefCell::new(HasCache::with_limit(max_entries)),
            ..Self::default()
        }
    }
}
