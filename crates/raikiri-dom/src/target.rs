//! Named-counter snapshots — the document-order counter-scope walk shared
//! by markers, generated content, layout and page projection.
//!
//! # Counter-stack scope model
//!
//! Each snapshot holds every live counter's full **nested stack**
//! (outermost scope first, leaf last), the shape `counters()` reads. CSS
//! Lists Level 3 §4.3 "Nested counters and scope"
//! <https://www.w3.org/TR/css-lists-3/#auto-numbering> states counters are
//! "'self-nesting'; instantiating a new counter on an element which
//! inherited an identically-named counter from its parent creates a new
//! counter of the same name, nested inside the existing counter" — and that
//! a counter's scope "starts at the first element in the document that
//! instantiates that counter and includes the element's descendants and its
//! following siblings with their descendants" (same section). §4.3 also
//! states a second, separate rule: a counter-reset's scope "does not
//! include any elements in the scope of a counter with the same name
//! created by a counter-reset on a later sibling of the element, allowing
//! such explicit counter instantiations to obscure those earlier siblings."
//!
//! [`CounterScopes`] implements both rules for this arena-local walk.
//! `counter-reset` always pushes a new stack level unconditionally
//! (self-nesting); `counter-increment`/`counter-set` push one only when the
//! named counter has no active level yet (CSS Lists 3 §4.4.2
//! <https://www.w3.org/TR/css-lists-3/#instantiating-counters>), otherwise
//! they mutate the innermost existing level in place. Either way, a newly
//! pushed level is popped not at the instantiating element's *own* subtree
//! exit, but at that element's **parent's** subtree exit
//! ([`counter_snapshots_with_budget`]'s `pending_pops` side-stack) — the
//! exit point that keeps the level visible across the instantiating
//! element's own following siblings and their descendants too, matching the
//! following-sibling half of the scope quoted above. A later sibling's own
//! `counter-reset` for the same name then evicts (rather than nests inside)
//! whatever earlier-sibling level is still open, per the obscuring rule
//! quoted above. [`CounterScopes::apply`]'s own doc covers the full
//! mechanism.
//!
//! A `counter-reset` on the walk's own root element has no parent bucket
//! to record into (`pending_pops` starts empty) and is therefore never
//! popped — inert, since a [`CounterScopes`] value never outlives the
//! single walk that owns it.

use std::collections::HashMap;
use std::mem::size_of;

use raikiri_style::CascadeResult;
use raikiri_style::property::DisplayValue;
use raikiri_traits::Symbol;

use crate::document::Document;

/// Maximum estimated memory retained and cloned for counter snapshots.
///
/// The estimate includes the per-node snapshot vector, hash-table capacity,
/// counter-stack capacities, and a conservative allowance for allocation
/// metadata. It is a hard cap independent of `RenderLimits`.
pub const MAX_COUNTER_SNAPSHOT_ESTIMATED_BYTES: u64 = 128 * 1024 * 1024;

/// Counter snapshots would exceed their cumulative estimated memory budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CounterSnapshotLimitExceeded {
    /// Maximum estimated bytes allowed for the operation.
    pub limit: u64,
    /// Estimated cumulative bytes after the rejected snapshot.
    pub actual: u64,
}

impl std::fmt::Display for CounterSnapshotLimitExceeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "counter snapshot memory limit exceeded: {} bytes (limit {})",
            self.actual, self.limit
        )
    }
}

impl std::error::Error for CounterSnapshotLimitExceeded {}

/// Cumulative allowance for counter snapshot work within one render operation.
///
/// Pass the same budget to each page of a multi-page render so the fixed
/// snapshot ceiling applies to the complete document operation. A new default
/// budget starts a new operation with the repository-wide maximum.
#[derive(Debug)]
pub struct CounterSnapshotBudget {
    estimated_bytes: u64,
    limit: u64,
}

impl Default for CounterSnapshotBudget {
    fn default() -> Self {
        Self::new(MAX_COUNTER_SNAPSHOT_ESTIMATED_BYTES)
    }
}

impl CounterSnapshotBudget {
    fn new(limit: u64) -> Self {
        Self {
            estimated_bytes: 0,
            limit,
        }
    }

    fn charge(&mut self, additional: Option<u64>) -> Result<(), CounterSnapshotLimitExceeded> {
        let actual = additional
            .and_then(|bytes| self.estimated_bytes.checked_add(bytes))
            .unwrap_or(u64::MAX);
        if actual > self.limit {
            return Err(CounterSnapshotLimitExceeded {
                limit: self.limit,
                actual,
            });
        }
        self.estimated_bytes = actual;
        Ok(())
    }

    fn charge_base(&mut self, node_count: usize) -> Result<(), CounterSnapshotLimitExceeded> {
        let bytes = u64::try_from(node_count)
            .ok()
            .and_then(|count| count.checked_mul(size_of::<CounterSnapshot>() as u64));
        self.charge(bytes)
    }

    fn charge_implicit_list_item(&mut self) -> Result<(), CounterSnapshotLimitExceeded> {
        // Reserve for a small hash table with capacity slack and the one-value
        // vector allocation added to an otherwise empty snapshot.
        let entry_bytes = (size_of::<(Symbol, Vec<i32>)>() as u64)
            .checked_mul(2)
            .and_then(|bytes| bytes.checked_add(16));
        let bytes = entry_bytes
            .and_then(|bytes| bytes.checked_mul(4))
            .and_then(|bytes| bytes.checked_add(size_of::<i32>() as u64 + 16));
        self.charge(bytes)
    }
}

/// Counter-scope tracker implementing CSS Lists 3 §4.3's nested-scope model
/// (self-nesting, following-sibling inclusion, and sibling obscuring) for
/// one arena-local walk — see module doc "Counter-stack scope model" and
/// [`Self::apply`]'s own doc for the mechanism.
#[derive(Debug, Default)]
struct CounterScopes {
    /// counter name → currently active nested stack (outermost first, leaf
    /// last) — the [`CounterSnapshot`] shape directly, so
    /// [`Self::snapshot`] is a plain clone.
    stacks: HashMap<Symbol, Vec<i32>>,
}

impl CounterScopes {
    /// Apply one element's `counter-reset` / `counter-increment` /
    /// `counter-set` directives, in CSS Lists 3 §4 processing order (reset →
    /// increment → set, following §4.2's note that `counter-set` is applied
    /// after `counter-increment`).
    ///
    /// Every counter name this call newly *instantiates* — a `counter-reset`
    /// unconditionally, or a `counter-increment`/`counter-set` on a name
    /// with no active level yet (CSS Lists 3 §4.4.2) — is recorded into
    /// `parent_bucket`, not tracked against this element itself: CSS Lists 3
    /// §4.3 scopes a counter-reset to "the element's descendants and its
    /// following siblings with their descendants", so the pop must wait for
    /// the *parent's* subtree exit, not this element's own.
    /// [`counter_snapshots_with_budget`]'s `pending_pops` side-stack passes
    /// its own top-of-stack bucket — the one this element's parent pushed —
    /// as `parent_bucket` here, and pops it (via [`Self::pop`]) when that
    /// parent's own `Exit` step runs. `parent_bucket` is `None` only when
    /// this element is the walk's root (no `Enter` has pushed a bucket
    /// yet); a root-level instantiation is then left untracked and never
    /// popped — harmless, since a [`CounterScopes`] value never outlives
    /// the one walk that owns it.
    ///
    /// Also implements CSS Lists 3 §4.3's "obscuring" rule, quoted in full
    /// on the module doc: a `counter-reset` for a name already present in
    /// `parent_bucket` — i.e. instantiated by an *earlier sibling* under the
    /// same parent, not by an ancestor (an ancestor's own level lives in an
    /// outer bucket this element's *parent's* own `Enter` pushed, never in
    /// `parent_bucket` itself, which only ever holds names the parent's
    /// *children* — this element's siblings — instantiated) — evicts that
    /// sibling's level outright (removing it from `parent_bucket` and
    /// popping it from `stacks`) before pushing its own, rather than nesting
    /// a new level inside it. `counter-increment`/`counter-set` never evict;
    /// per CSS Lists 3 §4.4.2 they only ever mutate whichever level is
    /// already in scope, continuing it across siblings.
    ///
    /// A duplicate `<counter-name>` within `cv.counter_reset` itself (e.g.
    /// `counter-reset: foo 1 foo 2`) is deduplicated to the *last*
    /// occurrence's value before pushing — CSS Lists 3 §4.1, see the
    /// dedup step's own comment below.
    fn apply(
        &mut self,
        cv: &raikiri_style::ComputedValues,
        mut parent_bucket: Option<&mut Vec<Symbol>>,
    ) {
        // CSS Lists 3 §4.1 <https://www.w3.org/TR/css-lists-3/#counter-reset>:
        // "If multiple instances of the same <counter-name> occur in the
        // property value, only the last one is honored." Reduce to one
        // (name, value) pair per distinct name — keeping the *last*
        // occurrence's value, via HashMap::insert's overwrite-on-reinsert
        // semantics — before pushing exactly one new scope level per name.
        // `reset_order` preserves first-occurrence order across distinct
        // names only; it has no bearing on correctness (different names
        // push into independent stacks), just deterministic iteration.
        let mut reset_values: HashMap<Symbol, i32> = HashMap::new();
        let mut reset_order: Vec<Symbol> = Vec::new();
        for (name, value) in cv.counter_reset.iter() {
            let sym = Symbol::new(name.clone());
            if reset_values.insert(sym.clone(), *value).is_none() {
                reset_order.push(sym);
            }
        }
        for sym in reset_order {
            let value = reset_values[&sym];
            // Obscuring rule: an earlier sibling's still-open same-name
            // level (found in `parent_bucket`) is evicted, not nested
            // inside, by this element's own counter-reset — see this
            // method's own doc.
            if let Some(bucket) = parent_bucket.as_deref_mut()
                && let Some(pos) = bucket.iter().position(|pending| *pending == sym)
            {
                bucket.remove(pos);
                self.pop_one(&sym);
            }
            // counter-reset always instantiates a *new* nested scope level
            // (CSS Lists 3 §4.3 "self-nesting", module doc), regardless of
            // whether an ancestor scope for `name` already exists.
            self.stacks.entry(sym.clone()).or_default().push(value);
            if let Some(bucket) = parent_bucket.as_deref_mut() {
                bucket.push(sym);
            }
        }
        for (name, delta) in cv.counter_increment.iter() {
            let sym = Symbol::new(name.clone());
            let stack = self.stacks.entry(sym.clone()).or_default();
            match stack.last_mut() {
                // Saturating, not wrapping/panicking, on overflow — the
                // same fix as
                // `raikiri_traits::page::context::CounterStack::increment` —
                // this walker's top-of-stack increment shared the exact same
                // unbounded `+=` bug. `counter-reset: c 2147483647; counter-increment:
                // c 1` is spec-legal CSS and would panic (debug) or
                // silently wrap to `i32::MIN` (release) on plain `+=`.
                Some(top) => *top = top.saturating_add(*delta),
                None => {
                    // No active scope for `name` yet anywhere in scope —
                    // local auto-instantiation (CSS Lists 3 §4.4.2), tracked
                    // into `parent_bucket` exactly like a counter-reset's
                    // own push so it too survives across following
                    // siblings.
                    stack.push(*delta);
                    if let Some(bucket) = parent_bucket.as_deref_mut() {
                        bucket.push(sym);
                    }
                }
            }
        }
        for (name, value) in cv.counter_set.iter() {
            let sym = Symbol::new(name.clone());
            let stack = self.stacks.entry(sym.clone()).or_default();
            match stack.last_mut() {
                Some(top) => *top = *value,
                None => {
                    stack.push(*value);
                    if let Some(bucket) = parent_bucket.as_deref_mut() {
                        bucket.push(sym);
                    }
                }
            }
        }
    }

    /// Pop exactly the scope levels recorded for one bucket of
    /// [`counter_snapshots_with_budget`]'s `pending_pops` side-stack — invoked when
    /// the walker finishes that bucket's owning element's subtree (the
    /// `Exit` step), which per [`Self::apply`]'s doc is the *parent* of
    /// whichever element(s) originally pushed those levels.
    fn pop(&mut self, names: &[Symbol]) {
        for name in names {
            self.pop_one(name);
        }
    }

    /// Pop the innermost scope level for one counter `name`, if any is
    /// active — the single-name primitive [`Self::pop`]'s loop delegates
    /// to, and that [`Self::apply`]'s obscuring-rule eviction (see that
    /// method's doc) also calls directly for the one name it evicts.
    fn pop_one(&mut self, name: &Symbol) {
        if let Some(stack) = self.stacks.get_mut(name) {
            stack.pop();
            if stack.is_empty() {
                self.stacks.remove(name);
            }
        }
    }

    /// Snapshot the current live stacks (outermost first, leaf last).
    fn snapshot(
        &self,
        budget: &mut CounterSnapshotBudget,
    ) -> Result<CounterSnapshot, CounterSnapshotLimitExceeded> {
        budget.charge(self.estimated_snapshot_bytes())?;
        Ok(self.stacks.clone())
    }

    fn estimated_snapshot_bytes(&self) -> Option<u64> {
        // Account for hash-table capacity slack and control bytes, then charge
        // every stack by its capacity because the clone can allocate up to
        // that much storage. The metadata allowance covers each per-counter
        // Vec allocation and allocator bookkeeping.
        let entry_bytes = (size_of::<(Symbol, Vec<i32>)>() as u64)
            .checked_mul(2)?
            .checked_add(16)?;
        let map_bytes = u64::try_from(self.stacks.capacity())
            .ok()?
            .checked_mul(entry_bytes)?;
        let stack_bytes = self.stacks.values().try_fold(0_u64, |total, stack| {
            let capacity = u64::try_from(stack.capacity()).ok()?;
            let allocation = capacity
                .checked_mul(size_of::<i32>() as u64)?
                .checked_add(16)?;
            total.checked_add(allocation)
        })?;
        map_bytes.checked_add(stack_bytes)
    }
}

/// Snapshot of every live named counter stack after applying the directives on
/// the corresponding node.
///
/// The outer-to-inner stack shape matches the `counters()` data consumed by
/// marker and generated-content paint. Nodes that do not generate a box do not
/// apply their own directives; descendants of `display: none` are omitted.
/// This is an owned snapshot so paint can resolve counters without borrowing
/// the DOM walk's transient scope state.
/// List-item nodes also carry their sibling ordinal when no explicit
/// `list-item` counter is in scope.
pub type CounterSnapshot = HashMap<Symbol, Vec<i32>>;

/// Build named-counter snapshots in document order for marker and generated
/// content consumers.
///
/// The traversal applies [`CounterScopes`]'s sibling obscuring,
/// self-nesting and saturating arithmetic, and skips the directives of
/// `display: none` / `display: contents` elements (CSS Lists 3 §4.5).
/// `snapshots[idx]` is empty for an unreachable or skipped node.
/// Ordinary list items receive their one-based sibling ordinal in the
/// `list-item` entry so marker consumers can reuse it without rescanning.
pub fn counter_snapshots(
    doc: &Document,
    cascade: &CascadeResult,
) -> Result<Vec<CounterSnapshot>, CounterSnapshotLimitExceeded> {
    counter_snapshots_with_budget(doc, cascade, &mut CounterSnapshotBudget::default())
}

/// Counter snapshots shared between the stages that render one layout, or
/// the limit error that building them hit.
pub(crate) type SharedCounterSnapshots =
    Result<std::sync::Arc<Vec<CounterSnapshot>>, CounterSnapshotLimitExceeded>;

impl Document {
    /// The [`counter_snapshots`] of this document for `cascade`, built once
    /// and shared by layout, page projection, paint and generated-content
    /// resolution until the tree changes.
    ///
    /// Counters read only the element cascade, so the snapshots are reused
    /// for every cascade with the same
    /// [`CascadeResult::element_generation`]. A layout with a cascade of
    /// another generation replaces them; outside layout, such a cascade gets
    /// snapshots of its own, which are not kept. The memory
    /// budget is charged once for the shared build, so a multi-page render no
    /// longer pays it per page.
    pub fn shared_counter_snapshots(
        &self,
        cascade: &CascadeResult,
    ) -> Result<std::sync::Arc<Vec<CounterSnapshot>>, CounterSnapshotLimitExceeded> {
        let generation = cascade.element_generation();
        // The vector moves into the `Arc` without a copy, so the build peaks
        // at the one array its budget charged.
        let build = || counter_snapshots(self, cascade).map(std::sync::Arc::new);
        let (cached_generation, cached) = self
            .counter_snapshot_cache
            .get_or_init(|| (generation, build()));
        if *cached_generation == generation {
            cached.clone()
        } else {
            build()
        }
    }
}

/// Build named-counter snapshots while consuming a caller-owned cumulative
/// budget. Reusing one budget across pages bounds the total clone and
/// allocation work for the whole multi-page render.
pub fn counter_snapshots_with_budget(
    doc: &Document,
    cascade: &CascadeResult,
    budget: &mut CounterSnapshotBudget,
) -> Result<Vec<CounterSnapshot>, CounterSnapshotLimitExceeded> {
    budget.charge_base(doc.node_count())?;
    let mut snapshots = vec![HashMap::new(); doc.node_count()];
    let mut scopes = CounterScopes::default();
    let list_item_counter = Symbol::new("list-item");

    enum WalkStep {
        Enter(usize, u32),
        Exit,
    }

    let mut stack = vec![WalkStep::Enter(doc.root, 1)];
    let mut pending_pops: Vec<Vec<Symbol>> = Vec::new();
    while let Some(step) = stack.pop() {
        match step {
            WalkStep::Enter(idx, list_item_ordinal) => {
                let node = &doc.nodes[idx];
                if !node.is_in_document() {
                    continue;
                }

                let cv = cascade.computed.get(idx);
                let display = cv.map(|cv| doc.counter_display(idx, cv.display));
                let is_display_none = display == Some(DisplayValue::None);
                match cv {
                    Some(_) if is_display_none || display == Some(DisplayValue::Contents) => {}
                    Some(cv) => scopes.apply(cv, pending_pops.last_mut()),
                    // cov:ignore: cascade output has one computed value per arena node
                    None => {}
                }
                if is_display_none {
                    // A display:none box is never a counter consumer and its
                    // descendants are not traversed.
                    continue;
                }
                // Keep the element snapshot before its own `::before`
                // directives. The generated pseudo is a child of the element,
                // so a counter scope it creates must be visible to the
                // element's real children, while the pseudo's own content
                // applies those directives locally in paint.
                snapshots[idx] = scopes.snapshot(budget)?;
                if display == Some(DisplayValue::ListItem)
                    && !snapshots[idx].contains_key(&list_item_counter)
                {
                    budget.charge_implicit_list_item()?;
                    snapshots[idx]
                        .insert(list_item_counter.clone(), vec![list_item_ordinal as i32]);
                }
                stack.push(WalkStep::Exit);
                pending_pops.push(Vec::new());
                if let Some(before) = cascade.pseudo.get(&(
                    raikiri_style::StyleNodeId::new(idx as u64),
                    raikiri_style::PseudoElem::Before,
                )) && before.display != DisplayValue::None
                    && before.content.iter().any(|component| {
                        !matches!(component, raikiri_style::property::ContentComponent::None)
                    })
                {
                    scopes.apply(before, pending_pops.last_mut());
                }
                let children_start = stack.len();
                let mut sibling_ordinal = 0_u32;
                for &child in &node.children {
                    let is_list_item = cascade.computed.get(child).is_some_and(|computed| {
                        doc.counter_display(child, computed.display) == DisplayValue::ListItem
                    });
                    if is_list_item {
                        sibling_ordinal = sibling_ordinal.saturating_add(1);
                    }
                    stack.push(WalkStep::Enter(
                        child,
                        if is_list_item { sibling_ordinal } else { 0 },
                    ));
                }
                stack[children_start..].reverse();
            }
            WalkStep::Exit => {
                if let Some(names) = pending_pops.pop() {
                    scopes.pop(&names);
                }
            }
        }
    }
    Ok(snapshots)
}

#[cfg(test)]
mod tests;
