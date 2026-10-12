use super::*;

/// Limits filter entries plus rejected parents, and the aggregate child slots
/// charged to them. Charging all node kinds is conservative: rank maps store
/// only matching elements. An oversized filter uses the direct scan without
/// evicting useful entries or repeatedly rebuilding an evicted filter.
const MAX_ENTRIES: usize = 256;
const MAX_CHILDREN: usize = 65_536;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct FilterKey {
    parent: StyleNodeId,
    filter: usize,
    scope: Option<StyleNodeId>,
    allow_detached: bool,
    quirks_mode: StyleQuirksMode,
}

struct FilterRanks {
    // Keep the ThinArc allocation alive: its selector-slice address is the
    // filter identity, and must not be recycled during this cache's lifetime.
    _filter: NthOfSelectorData<RaikiriSelectorImpl>,
    from_start: HashMap<StyleNodeId, i32>,
    total: i32,
}

impl FilterRanks {
    fn position(&self, elem: StyleNodeId) -> (i32, i32, i32) {
        let from_start = self.from_start.get(&elem).copied().unwrap_or(0);
        (from_start, self.total - from_start + 1, self.total)
    }
}

/// Filter-specific facts for one immutable cascade or query matcher.
/// Ancestor chains obey the same subject-chain invariant as [`MatchCaches`].
/// Each key retains its filter allocation and every matching condition.
pub(super) struct FilteredNthCache {
    entries: HashMap<FilterKey, FilterRanks>,
    // The remaining budget only decreases. A parent rejected on its raw
    // child count remains oversized for any future uncached filter.
    over_budget_parents: HashSet<StyleNodeId>,
    charged_children: usize,
    max_entries: usize,
    max_children: usize,
    // One allocation-free negative memo avoids repeating the admission scan
    // for every filter/element of a narrow parent, without retaining a map
    // entry for each parent of a deep, narrow tree.
    last_narrow_parent: Option<StyleNodeId>,
}

impl Default for FilteredNthCache {
    fn default() -> Self {
        Self::with_limits(MAX_ENTRIES, MAX_CHILDREN)
    }
}

impl FilteredNthCache {
    pub(super) fn with_limits(max_entries: usize, max_children: usize) -> Self {
        Self {
            entries: HashMap::new(),
            over_budget_parents: HashSet::new(),
            charged_children: 0,
            max_entries,
            max_children,
            last_narrow_parent: None,
        }
    }
}

/// Return shared ranks, or `None` when the direct scan should be used.
pub(super) fn position<D: StyleDom>(
    ctx: MatchContext<'_, D>,
    parent: StyleNodeId,
    elem: StyleNodeId,
    data: &NthOfSelectorData<RaikiriSelectorImpl>,
    ancestors: &[StyleNodeId],
) -> Option<(i32, i32, i32)> {
    let key = FilterKey {
        parent,
        filter: data.selectors().as_ptr() as usize,
        scope: ctx.scope,
        allow_detached: ctx.allow_detached,
        quirks_mode: ctx.quirks_mode,
    };
    let remaining = {
        let cache = ctx.caches.filtered_nth.borrow();
        if cache.last_narrow_parent == Some(parent) {
            return None;
        }
        if let Some(ranks) = cache.entries.get(&key) {
            return Some(ranks.position(elem));
        }
        if cache.over_budget_parents.contains(&parent)
            || cache.entries.len() + cache.over_budget_parents.len() >= cache.max_entries
        {
            return None;
        }
        cache.max_children - cache.charged_children
    };
    // Stop counting at the budget boundary, before allocating or matching.
    let children = ctx
        .dom
        .child_ids(parent)
        .take(remaining.saturating_add(1))
        .count();
    if children > remaining {
        ctx.caches
            .filtered_nth
            .borrow_mut()
            .over_budget_parents
            .insert(parent);
        return None;
    }
    if children < SIBLING_CACHE_MIN_CHILDREN {
        ctx.caches.filtered_nth.borrow_mut().last_narrow_parent = Some(parent);
        return None;
    }

    let mut ranks = FilterRanks {
        _filter: data.clone(),
        from_start: HashMap::new(),
        total: 0,
    };
    for child in ctx.dom.child_ids(parent) {
        if !is_candidate_element(ctx.dom, child, ctx.allow_detached) {
            continue;
        }
        if let Some(node) = ctx.dom.node(child)
            && let Some(element) = node.as_element()
            && selector_slice_matches(data.selectors(), ctx, &element, child, ancestors)
        {
            ranks.total += 1;
            ranks.from_start.insert(child, ranks.total);
        }
    }
    let result = ranks.position(elem);
    // Recursive filters may consume budget while building. Recheck without
    // holding any cache borrow across the recursive matcher above.
    let mut cache = ctx.caches.filtered_nth.borrow_mut();
    if cache.entries.len() + cache.over_budget_parents.len() < cache.max_entries
        && children <= cache.max_children - cache.charged_children
    {
        cache.charged_children += children;
        cache.entries.insert(key, ranks);
    }
    Some(result)
}

#[cfg(test)]
mod tests;
