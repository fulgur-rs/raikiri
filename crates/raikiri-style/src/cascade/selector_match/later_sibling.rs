//! Shared answers for the `~` (subsequent-sibling) combinator.
//!
//! For `A ~ B`, whether some earlier sibling of a `B` candidate satisfies `A`
//! (and everything left of it) depends on the candidate only through its
//! position: the siblings share one parent and one ancestor chain, so "does
//! sibling `s` satisfy the left side" is the same fact for every later
//! sibling that asks. Recording, per parent and left-side compound, how far
//! the siblings have been tested and the first one that satisfied the left
//! side lets each later candidate answer from that record instead of
//! rescanning from the first sibling. A run of `n` siblings then costs O(n)
//! left-side tests per compound instead of O(n²).

use super::*;

/// Caps the recorded (parent, compound) pairs. Past it, new pairs use the
/// direct scan; recorded pairs keep answering.
const MAX_ENTRIES: usize = 65_536;

/// Everything the left-side result of one `~` frame depends on besides the
/// sibling itself.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct LaterSiblingKey {
    parent: StyleNodeId,
    /// Address of the first component of the compound left of the `~`. The
    /// entry retains its selector, so the address cannot be reused by
    /// another selector while the entry exists.
    compound: usize,
    relative_anchor: Option<StyleNodeId>,
    scope: Option<StyleNodeId>,
    allow_detached: bool,
    quirks_mode: StyleQuirksMode,
}

struct LaterSiblingState {
    // Keeps the selector allocation (and so `LaterSiblingKey::compound`)
    // alive for the lifetime of the entry.
    _selector: Selector<RaikiriSelectorImpl>,
    /// Every candidate sibling before this index in [`SiblingInfo::elements`]
    /// is known not to satisfy the left side, unless `first_match` says
    /// otherwise.
    scanned: usize,
    /// The first candidate sibling known to satisfy the left side.
    first_match: Option<usize>,
}

/// Per-matcher record of `~` left-side results. Ancestor chains obey the
/// same subject-chain invariant as [`MatchCaches`].
pub(super) struct LaterSiblingCache {
    entries: FxHashMap<LaterSiblingKey, LaterSiblingState>,
    max_entries: usize,
}

impl Default for LaterSiblingCache {
    fn default() -> Self {
        Self::with_limit(MAX_ENTRIES)
    }
}

impl LaterSiblingCache {
    pub(super) fn with_limit(max_entries: usize) -> Self {
        Self {
            entries: FxHashMap::default(),
            max_entries,
        }
    }
}

/// How a `~` frame should look for a matching earlier sibling.
pub(super) enum LaterSiblingPlan {
    /// The record already decides the frame.
    Known(bool),
    /// Test `info.elements[start..end]` in order, reporting progress under
    /// `key`.
    Scan {
        info: Rc<SiblingInfo>,
        key: LaterSiblingKey,
        start: usize,
        end: usize,
    },
}

/// Plans the `~` frame for `current_id` under `parent_id`, whose left side
/// starts at `iter`. `None` means the caller should use the direct scan: the
/// parent is too narrow for a [`SiblingInfo`], `current_id` is not one of its
/// candidates, or the cache is full.
pub(super) fn plan<D: StyleDom>(
    ctx: MatchContext<'_, D>,
    parent_id: StyleNodeId,
    current_id: StyleNodeId,
    selector: &Selector<RaikiriSelectorImpl>,
    iter: &SelectorIter<'_, RaikiriSelectorImpl>,
    relative_anchor: Option<StyleNodeId>,
) -> Option<LaterSiblingPlan> {
    let compound = iter.clone().next()?;
    let info = ctx
        .caches
        .sibling_info(ctx.dom, parent_id, ctx.allow_detached)?;
    let end = *info.index_of.get(&current_id)?;
    let key = LaterSiblingKey {
        parent: parent_id,
        compound: std::ptr::from_ref(compound) as usize,
        relative_anchor,
        scope: ctx.scope,
        allow_detached: ctx.allow_detached,
        quirks_mode: ctx.quirks_mode,
    };
    let mut cache = ctx.caches.later_siblings.borrow_mut();
    let start = match cache.entries.get(&key) {
        Some(state) => match state.first_match {
            Some(first) => return Some(LaterSiblingPlan::Known(first < end)),
            None => state.scanned,
        },
        None if cache.entries.len() < cache.max_entries => {
            cache.entries.insert(
                key,
                LaterSiblingState {
                    _selector: selector.clone(),
                    scanned: 0,
                    first_match: None,
                },
            );
            0
        }
        None => return None,
    };
    if start >= end {
        return Some(LaterSiblingPlan::Known(false));
    }
    Some(LaterSiblingPlan::Scan {
        info,
        key,
        start,
        end,
    })
}

/// Records that every candidate sibling before `index` fails the left side.
pub(super) fn record_scanned(caches: &MatchCaches, key: &LaterSiblingKey, index: usize) {
    if let Some(state) = caches.later_siblings.borrow_mut().entries.get_mut(key) {
        state.scanned = state.scanned.max(index);
    }
}

/// Records that the candidate sibling at `index` satisfies the left side.
/// The scan tests siblings in document order and records each failure
/// first, so `index` is the first match.
pub(super) fn record_match(caches: &MatchCaches, key: &LaterSiblingKey, index: usize) {
    if let Some(state) = caches.later_siblings.borrow_mut().entries.get_mut(key) {
        state.first_match = Some(state.first_match.map_or(index, |first| first.min(index)));
    }
}

#[cfg(test)]
impl MatchCaches {
    pub(crate) fn later_sibling_entries(&self) -> usize {
        self.later_siblings.borrow().entries.len()
    }

    pub(crate) fn with_later_sibling_limit(max_entries: usize) -> Self {
        Self {
            later_siblings: RefCell::new(LaterSiblingCache::with_limit(max_entries)),
            ..Self::default()
        }
    }
}
