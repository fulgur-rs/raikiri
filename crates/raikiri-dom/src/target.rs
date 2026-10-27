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

use raikiri_style::property::DisplayValue;
use raikiri_style::{CascadeResult, CounterResetValue, PseudoElem};
use raikiri_traits::Symbol;
use smol_str::SmolStr;

use crate::document::Document;
use crate::generated_content::QuoteDepth;

mod compact;
pub(crate) use compact::SharedCompactCounterSnapshots;
pub use compact::{
    CompactCounterSnapshots, compact_counter_snapshots, compact_counter_snapshots_with_budget,
};

/// The counter that list items increment implicitly (CSS Lists 3 §4.6).
const LIST_ITEM_COUNTER: &str = "list-item";

/// The computed style of the `::before` or `::after` of `element`, when that
/// pseudo-element generates a box.
fn generated_pseudo(
    cascade: &CascadeResult,
    element: usize,
    pseudo: PseudoElem,
) -> Option<&raikiri_style::ComputedValues> {
    cascade
        .pseudo
        .get(&(raikiri_style::StyleNodeId::new(element as u64), pseudo))
        .map(|style| style.as_ref())
        .filter(|style| {
            style.display != DisplayValue::None
                && style.content.iter().any(|component| {
                    !matches!(component, raikiri_style::property::ContentComponent::None)
                })
        })
}

/// The `list-item 1` increment a list item applies on top of its own
/// `counter-increment` (CSS Lists 3 §4.6), or `None` when `list_item` is
/// false or the element's `counter-increment` names `list-item` itself.
pub(crate) fn implicit_list_item_increment(
    cv: &raikiri_style::ComputedValues,
    list_item: bool,
) -> Option<(SmolStr, i32)> {
    (list_item
        && !cv
            .counter_increment
            .iter()
            .any(|(name, _)| name.as_str() == LIST_ITEM_COUNTER))
    .then(|| (SmolStr::new_static(LIST_ITEM_COUNTER), 1))
}

/// [`implicit_list_item_increment`] for a style seen on its own, as the
/// local counter view of a pseudo-element sees it: the increment is -1 when
/// the style's own `counter-reset` makes `list-item` a reversed counter.
/// A reversed `list-item` counter inherited from the element is not known
/// here.
pub(crate) fn local_implicit_list_item_increment(
    cv: &raikiri_style::ComputedValues,
    list_item: bool,
) -> Option<(SmolStr, i32)> {
    let reversed = cv
        .counter_reset
        .iter()
        .rev()
        .find(|(name, _)| name.as_str() == LIST_ITEM_COUNTER)
        .is_some_and(|(_, value)| value.is_reversed());
    implicit_list_item_increment(cv, list_item)
        .map(|(name, delta)| (name, if reversed { -delta } else { delta }))
}

/// The starting value that the `counter-reset` entry `value` of `cv` gives
/// the counter `name` in the local counter view of a pseudo-element. A
/// reversed counter without a given starting value is calculated from `cv`'s
/// own directives, as if nothing else in its scope counted.
pub(crate) fn local_reset_value(
    cv: &raikiri_style::ComputedValues,
    list_item: bool,
    name: &str,
    value: CounterResetValue,
) -> i32 {
    value.starting_value().unwrap_or_else(|| {
        let implicit = local_implicit_list_item_increment(cv, list_item);
        let (increment, set) = counter_directives(cv, implicit.as_ref(), name);
        let mut tally = ReversedStartTally::default();
        tally.record(increment, set);
        tally.starting_value()
    })
}

/// The total increment that `cv` (with its implicit list-item increment)
/// applies to the counter `name`, and the value its last `counter-set`
/// entry for `name` gives, if any.
fn counter_directives(
    cv: &raikiri_style::ComputedValues,
    implicit_list_item: Option<&(SmolStr, i32)>,
    name: &str,
) -> (i32, Option<i32>) {
    let increment = cv
        .counter_increment
        .iter()
        .chain(implicit_list_item)
        .filter(|(counter, _)| counter.as_str() == name)
        .fold(0_i32, |total, (_, delta)| total.saturating_add(*delta));
    let set = cv
        .counter_set
        .iter()
        .rev()
        .find(|(counter, _)| counter.as_str() == name)
        .map(|(_, value)| *value);
    (increment, set)
}

/// Maximum estimated memory retained and cloned for counter snapshots.
///
/// The estimate includes the per-node snapshot vector, hash-table capacity,
/// counter-stack capacities, and a conservative allowance for allocation
/// metadata. It is a hard cap independent of `RenderLimits`.
#[doc(hidden)]
pub const MAX_COUNTER_SNAPSHOT_ESTIMATED_BYTES: u64 = 128 * 1024 * 1024;

/// Counter snapshots would exceed their cumulative estimated memory budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
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
#[doc(hidden)]
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
        let Some(actual) = additional.and_then(|bytes| self.estimated_bytes.checked_add(bytes))
        else {
            return Err(CounterSnapshotLimitExceeded {
                limit: self.limit,
                actual: u64::MAX,
            });
        };
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

    /// Charge one entry of a hash table of [`CounterSnapshots`] that is
    /// `entry_bytes` large, with an allowance for hash-table capacity slack
    /// and control bytes, as for a counter map entry.
    fn charge_table_entry(
        &mut self,
        entry_bytes: usize,
    ) -> Result<(), CounterSnapshotLimitExceeded> {
        let bytes = (entry_bytes as u64)
            .checked_mul(2)
            .and_then(|bytes| bytes.checked_add(16));
        self.charge(bytes)
    }

    /// Charge one entry of [`CounterSnapshots`]' pseudo-element table.
    fn charge_pseudo_entry(&mut self) -> Result<(), CounterSnapshotLimitExceeded> {
        self.charge_table_entry(size_of::<((usize, PseudoElem), CounterSnapshot)>())
    }

    /// Charge one entry of [`CounterSnapshots`]' quote-depth table.
    fn charge_quote_entry(&mut self) -> Result<(), CounterSnapshotLimitExceeded> {
        self.charge_table_entry(size_of::<((usize, PseudoElem), usize)>())
    }
}

/// Counter-scope tracker implementing CSS Lists 3 §4.3's nested-scope model
/// (self-nesting, following-sibling inclusion, and sibling obscuring) for
/// one arena-local walk — see module doc "Counter-stack scope model" and
/// [`Self::apply`]'s own doc for the mechanism.
#[derive(Debug, Default)]
struct CounterScopes {
    compact: Option<std::cell::RefCell<compact::CompactBuilder>>,
    /// counter name → currently active nested stack (outermost first, leaf
    /// last) — the [`CounterSnapshot`] shape directly, so
    /// [`Self::snapshot`] is a plain clone.
    stacks: HashMap<Symbol, Vec<i32>>,
    /// counter name → what each level of the same-named entry of
    /// [`Self::stacks`] is, pushed and popped in lockstep with it.
    levels: HashMap<Symbol, Vec<CounterLevel>>,
    /// How reversed counters without a given starting value start.
    reversed: ReversedStarts,
    /// How many levels in [`Self::levels`] have a
    /// [`CounterLevel::calculated`] tally, so elements outside every such
    /// scope skip the tally bookkeeping.
    live_calculated: usize,
}

/// Whether one counter level is reversed, and which calculated starting
/// value it feeds when its `counter-reset` gave none.
#[derive(Clone, Copy, Debug, Default)]
struct CounterLevel {
    reversed: bool,
    /// While [`ReversedStarts::Collect`] runs, the index of this level's
    /// tally when it is a reversed counter without a given starting value.
    calculated: Option<usize>,
}

/// The starting values of reversed counters instantiated without one.
///
/// CSS Lists 3 §4.4.2 <https://www.w3.org/TR/css-lists-3/#instantiating-counters>
/// calculates such a value from the increments and sets that the elements in
/// the counter's scope apply to it. Those directives never depend on the
/// counter's own value, so [`counter_snapshots_with_budget`] walks the tree
/// once in [`Self::Collect`] mode, then again with the [`Self::Resolved`]
/// values.
#[derive(Debug)]
enum ReversedStarts {
    /// Record each counter's directives; the counters start at 0 meanwhile.
    Collect(Vec<ReversedStartTally>),
    /// The calculated starting values in document order, and the index of
    /// the next one to use.
    Resolved { starts: Vec<i32>, next: usize },
}

impl Default for ReversedStarts {
    fn default() -> Self {
        Self::Collect(Vec::new())
    }
}

/// The running state of CSS Lists 3 §4.4.2's algorithm for one reversed
/// counter: `num` and `lastNonZeroIncrementNegated`, plus whether a
/// `counter-set` already ended the loop.
#[derive(Clone, Copy, Debug, Default)]
struct ReversedStartTally {
    num: i32,
    last_non_zero_increment_negated: i32,
    done: bool,
}

impl ReversedStartTally {
    /// One element that increments the counter by `increment` (0 when it only
    /// sets it) and, when `set` is given, sets it to that value.
    fn record(&mut self, increment: i32, set: Option<i32>) {
        if self.done {
            return;
        }
        let increment_negated = increment.saturating_neg();
        if increment_negated != 0 {
            self.last_non_zero_increment_negated = increment_negated;
        }
        if let Some(value) = set {
            self.num = self.num.saturating_add(value);
            self.done = true;
        } else {
            self.num = self.num.saturating_add(increment_negated);
        }
    }

    fn starting_value(self) -> i32 {
        self.num
            .saturating_add(self.last_non_zero_increment_negated)
    }
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
    ///
    /// `list_item` marks an element whose box is a list item. CSS Lists 3
    /// §4.6 <https://www.w3.org/TR/css-lists-3/#list-item-counter>: "in
    /// addition to any explicitly defined counter-increment, list items
    /// automatically increment the special list-item counter, unless the
    /// counter-increment property explicitly specifies a different
    /// increment for the list-item counter", so such an element gets an
    /// implicit `list-item 1` increment after its resets and before its sets,
    /// or `list-item -1` when the innermost `list-item` counter is reversed.
    ///
    /// A reversed counter without a given starting value takes the one
    /// [`ReversedStarts`] calculated, and the directives applied to it are fed
    /// back to that calculation while it is being collected.
    fn apply(
        &mut self,
        cv: &raikiri_style::ComputedValues,
        list_item: bool,
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
        let mut reset_values: HashMap<Symbol, CounterResetValue> = HashMap::new();
        let mut reset_order: Vec<Symbol> = Vec::new();
        for (name, value) in cv.counter_reset.iter() {
            let sym = Symbol::new(name.clone());
            if reset_values.insert(sym.clone(), *value).is_none() {
                reset_order.push(sym);
            }
        }
        for sym in reset_order {
            let reset = reset_values[&sym];
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
            let mut level = CounterLevel {
                reversed: reset.is_reversed(),
                calculated: None,
            };
            let value = match reset.starting_value() {
                Some(value) => value,
                None => self.calculated_start(&mut level),
            };
            // counter-reset always instantiates a *new* nested scope level
            // (CSS Lists 3 §4.3 "self-nesting", module doc), regardless of
            // whether an ancestor scope for `name` already exists.
            self.push_level(sym.clone(), value, level);
            if let Some(bucket) = parent_bucket.as_deref_mut() {
                bucket.push(sym);
            }
        }
        // CSS Lists 3 §4.6: a reversed `list-item` counter is incremented by
        // -1 on every list item instead.
        let implicit_list_item =
            implicit_list_item_increment(cv, list_item).map(|(name, delta)| {
                let reversed = self
                    .levels
                    .get(&Symbol::new(LIST_ITEM_COUNTER))
                    .and_then(|levels| levels.last())
                    .is_some_and(|level| level.reversed);
                (name, if reversed { -delta } else { delta })
            });
        for (name, delta) in cv.counter_increment.iter().chain(implicit_list_item.iter()) {
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
                Some(top) => {
                    *top = top.saturating_add(*delta);
                    if let Some(builder) = &self.compact {
                        builder.borrow_mut().change(&sym, Some(*top), false);
                    }
                }
                None => {
                    // No active scope for `name` yet anywhere in scope —
                    // local auto-instantiation (CSS Lists 3 §4.4.2), tracked
                    // into `parent_bucket` exactly like a counter-reset's
                    // own push so it too survives across following
                    // siblings.
                    self.push_level(sym.clone(), *delta, CounterLevel::default());
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
                Some(top) => {
                    *top = *value;
                    if let Some(builder) = &self.compact {
                        builder.borrow_mut().change(&sym, Some(*top), false);
                    }
                }
                None => {
                    self.push_level(sym.clone(), *value, CounterLevel::default());
                    if let Some(bucket) = parent_bucket.as_deref_mut() {
                        bucket.push(sym);
                    }
                }
            }
        }
        self.record_reversed_directives(cv, implicit_list_item.as_ref());
    }

    /// The starting value of a reversed counter level whose `counter-reset`
    /// gave none, numbering the level for [`ReversedStarts`].
    fn calculated_start(&mut self, level: &mut CounterLevel) -> i32 {
        match &mut self.reversed {
            ReversedStarts::Collect(tallies) => {
                level.calculated = Some(tallies.len());
                tallies.push(ReversedStartTally::default());
                self.live_calculated += 1;
                0
            }
            ReversedStarts::Resolved { starts, next } => {
                // Both walks visit the same levels in the same order, so the
                // index is in range.
                let value = starts.get(*next).copied().unwrap_or(0);
                *next += 1;
                value
            }
        }
    }

    /// Feed the increments and sets that `cv` applied to the innermost level
    /// of each counter to that level's [`ReversedStartTally`], when the level
    /// is a reversed counter whose starting value is being calculated.
    fn record_reversed_directives(
        &mut self,
        cv: &raikiri_style::ComputedValues,
        implicit_list_item: Option<&(SmolStr, i32)>,
    ) {
        // Only the collecting walk numbers calculated levels.
        let tallies = match &mut self.reversed {
            ReversedStarts::Collect(tallies) if self.live_calculated > 0 => tallies,
            _ => return,
        };
        let mut seen: Vec<&str> = Vec::new();
        let names = cv
            .counter_increment
            .iter()
            .chain(implicit_list_item)
            .chain(cv.counter_set.iter())
            .map(|(name, _)| name.as_str());
        for name in names {
            if seen.contains(&name) {
                continue;
            }
            seen.push(name);
            let Some(index) = self
                .levels
                .get(&Symbol::new(name))
                .and_then(|levels| levels.last())
                .and_then(|level| level.calculated)
            else {
                continue;
            };
            let (increment, set) = counter_directives(cv, implicit_list_item, name);
            tallies[index].record(increment, set);
        }
    }

    /// Push one new innermost level for `name`.
    fn push_level(&mut self, name: Symbol, value: i32, level: CounterLevel) {
        if let Some(builder) = &self.compact {
            builder.borrow_mut().change(&name, Some(value), true);
        }
        self.stacks.entry(name.clone()).or_default().push(value);
        self.levels.entry(name).or_default().push(level);
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
            if let Some(builder) = &self.compact {
                builder.borrow_mut().change(name, None, false);
            }
            stack.pop();
            if stack.is_empty() {
                self.stacks.remove(name);
            }
        }
        let level = self.levels.get_mut(name).and_then(Vec::pop);
        if self.levels.get(name).is_some_and(Vec::is_empty) {
            self.levels.remove(name);
        }
        if level.is_some_and(|level| level.calculated.is_some()) {
            self.live_calculated -= 1;
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
/// List items always carry a `list-item` entry, because each one increments
/// that counter implicitly (CSS Lists 3 §4.6).
pub type CounterSnapshot = HashMap<Symbol, Vec<i32>>;

/// The named-counter snapshots of a document: one [`CounterSnapshot`] per
/// arena node (the slice this dereferences to, indexed by node), and one
/// for each `::before` / `::after` whose content reads a counter.
///
/// A pseudo-element's snapshot is taken right after the walk applies that
/// pseudo-element's own directives, so it carries the directives before it
/// in document order: a `::before` follows its element's, and an `::after`
/// follows the `::before` and every real child.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CounterSnapshots {
    nodes: Vec<CounterSnapshot>,
    pseudos: HashMap<(usize, PseudoElem), CounterSnapshot>,
    quote_starts: HashMap<(usize, PseudoElem), usize>,
}

impl CounterSnapshots {
    /// The counters that the content of the `pseudo` of `element` reads, or
    /// `None` when the walk took no snapshot for it: the pseudo-element
    /// generates no box, or its content reads no counter.
    pub fn pseudo(&self, element: usize, pseudo: PseudoElem) -> Option<&CounterSnapshot> {
        self.pseudos.get(&(element, pseudo))
    }

    /// How many quotes are open where the content of the `pseudo` of
    /// `element` starts: the `open-quote`s of the generated text before it,
    /// less its `close-quote`s (CSS Content 3 §2.4.2). Zero when no quote is
    /// open, and for a pseudo-element whose content holds no quote.
    pub(crate) fn quote_depth(&self, element: usize, pseudo: PseudoElem) -> usize {
        self.quote_starts
            .get(&(element, pseudo))
            .copied()
            .unwrap_or_default()
    }
}

impl From<Vec<CounterSnapshot>> for CounterSnapshots {
    fn from(nodes: Vec<CounterSnapshot>) -> Self {
        Self {
            nodes,
            pseudos: HashMap::new(),
            quote_starts: HashMap::new(),
        }
    }
}

impl std::ops::Deref for CounterSnapshots {
    type Target = [CounterSnapshot];

    fn deref(&self) -> &Self::Target {
        &self.nodes
    }
}

/// What the walk hands to a consumer, and where it is when it does.
#[derive(Clone, Copy)]
enum CounterTarget {
    /// Its counters right after the element's own directives, before its
    /// `::before`.
    Element(usize),
    /// Its counters right after the directives of this pseudo-element of the
    /// element.
    Pseudo(usize, PseudoElem),
    /// How many quotes are open (not zero) where the content of this
    /// pseudo-element of the element starts. The pseudo-element's content
    /// holds a quote.
    QuoteStart(usize, PseudoElem, usize),
}

/// Whether the content of the pseudo-element `cv`, or its alternative text,
/// reads a counter.
fn content_reads_counters(cv: &raikiri_style::ComputedValues) -> bool {
    use raikiri_style::property::ContentComponent;
    cv.content
        .iter()
        .chain(cv.content.alt().into_iter().flatten())
        .any(|component| {
            matches!(
                component,
                ContentComponent::Counter { .. } | ContentComponent::Counters { .. }
            )
        })
}

/// Whether layout draws no CSS box for anything in the subtree of `node`,
/// whose computed values are `cv`: a non-rendered HTML element (see
/// [`crate::Node::is_non_rendered_html_element`]) is left out with its
/// subtree, an inline SVG is drawn as a unit, its elements without generated
/// content, and a table column or column group only describes tracks.
fn draws_no_boxes_below(node: &crate::Node, cv: Option<&raikiri_style::ComputedValues>) -> bool {
    node.is_non_rendered_html_element()
        || node.is_inline_svg_root()
        || cv.is_some_and(|cv| {
            matches!(
                cv.display,
                DisplayValue::TableColumn | DisplayValue::TableColumnGroup
            )
        })
}

/// Whether the generated text of the element `idx` takes part in the quote
/// depth. `unrendered` counts the elements around and including `idx` that
/// [`draws_no_boxes_below`] holds for. A replaced element has no `::before`
/// or `::after` box, its content being outside CSS, and neither has a line
/// break; layout draws no generated content for them.
fn counts_quotes(doc: &Document, idx: usize, unrendered: usize) -> bool {
    unrendered == 0
        && !crate::paint_rules::is_replaced_element(doc, idx)
        && doc.nodes[idx].tag_name() != Some("br")
}

/// Whether the content of the pseudo-element `cv` holds a quote keyword.
/// Alternative text takes none (CSS Content 3 §2).
fn content_has_quotes(cv: &raikiri_style::ComputedValues) -> bool {
    use raikiri_style::property::ContentComponent;
    cv.content
        .iter()
        .any(|component| matches!(component, ContentComponent::Quote(_)))
}

/// Whether the element `idx` scopes the quote depth to its subtree: it has
/// style containment and a box of its own (CSS Containment 2, style
/// containment scopes `open-quote` and `close-quote` like counters). The
/// quotes inside start at the depth the element is at, and the depth is back
/// to that value after the element, whatever the quotes of its subtree opened
/// or closed.
fn scopes_quote_depth(doc: &Document, cascade: &CascadeResult, idx: usize) -> bool {
    cascade.computed.get(idx).is_some_and(|cv| {
        cv.contain.contains(raikiri_style::property::Contain::STYLE)
            && doc.counter_display(idx, cv.display) != DisplayValue::Contents
    })
}

/// Build named-counter snapshots in document order for marker and generated
/// content consumers.
///
/// The traversal applies [`CounterScopes`]'s sibling obscuring,
/// self-nesting and saturating arithmetic, and skips the directives of
/// `display: none` / `display: contents` elements (CSS Lists 3 §4.5).
/// `snapshots[idx]` is empty for an unreachable or skipped node.
/// Each list item applies the implicit `list-item` increment, so its snapshot
/// holds the full `list-item` stack that its marker reads.
#[doc(hidden)]
pub fn counter_snapshots(
    doc: &Document,
    cascade: &CascadeResult,
) -> Result<CounterSnapshots, CounterSnapshotLimitExceeded> {
    counter_snapshots_with_budget(doc, cascade, &mut CounterSnapshotBudget::default())
}

/// Counter snapshots shared between the stages that render one layout, or
/// the limit error that building them hit.
pub(crate) type SharedCounterSnapshots =
    Result<std::sync::Arc<CounterSnapshots>, CounterSnapshotLimitExceeded>;

impl Document {
    /// Legacy owned counter snapshots for `cascade`, built once and shared
    /// by explicit owned callers until the tree changes. Built-in rendering
    /// uses [`Document::shared_compact_counter_snapshots`] instead.
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
    ) -> Result<std::sync::Arc<CounterSnapshots>, CounterSnapshotLimitExceeded> {
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
///
/// When a style instantiates a reversed counter without a starting value, a
/// first walk without snapshots collects the directives that calculate it.
#[doc(hidden)]
pub fn counter_snapshots_with_budget(
    doc: &Document,
    cascade: &CascadeResult,
    budget: &mut CounterSnapshotBudget,
) -> Result<CounterSnapshots, CounterSnapshotLimitExceeded> {
    budget.charge_base(doc.node_count())?;
    let mut scopes = resolved_scopes(doc, cascade)?;
    let mut snapshots = CounterSnapshots::from(vec![HashMap::new(); doc.node_count()]);
    walk_counter_scopes(doc, cascade, &mut scopes, |target, scopes| {
        match target {
            CounterTarget::Element(idx) => snapshots.nodes[idx] = scopes.snapshot(budget)?,
            CounterTarget::Pseudo(idx, pseudo) => {
                let snapshot = scopes.snapshot(budget)?;
                budget.charge_pseudo_entry()?;
                snapshots.pseudos.insert((idx, pseudo), snapshot);
            }
            CounterTarget::QuoteStart(idx, pseudo, open_quotes) => {
                budget.charge_quote_entry()?;
                snapshots.quote_starts.insert((idx, pseudo), open_quotes);
            }
        }
        Ok(())
    })?;
    Ok(snapshots)
}

fn resolved_scopes(
    doc: &Document,
    cascade: &CascadeResult,
) -> Result<CounterScopes, CounterSnapshotLimitExceeded> {
    let mut scopes = CounterScopes::default();
    if has_calculated_reversed_reset(cascade) {
        let mut collect = CounterScopes::default();
        walk_counter_scopes(doc, cascade, &mut collect, |_, _| Ok(()))?;
        if let ReversedStarts::Collect(tallies) = collect.reversed {
            scopes.reversed = ReversedStarts::Resolved {
                starts: tallies
                    .into_iter()
                    .map(ReversedStartTally::starting_value)
                    .collect(),
                next: 0,
            };
        }
    }
    Ok(scopes)
}

/// Whether any style instantiates a reversed counter without a starting
/// value, so the walk must calculate one first.
fn has_calculated_reversed_reset(cascade: &CascadeResult) -> bool {
    cascade
        .computed
        .iter()
        .chain(cascade.pseudo.values().map(|style| style.as_ref()))
        .any(|cv| {
            cv.counter_reset
                .iter()
                .any(|(_, value)| value.starting_value().is_none())
        })
}

/// Hand `visit` the quote depth where the content of the `pseudo` of
/// `element` starts when a quote is open and the content holds a quote, then
/// move `depth` past the quotes of that content. `cv` is the style of a
/// pseudo-element that generates a box.
fn walk_quotes(
    cv: &raikiri_style::ComputedValues,
    (element, pseudo): (usize, PseudoElem),
    depth: &mut QuoteDepth,
    scopes: &CounterScopes,
    visit: &mut impl FnMut(CounterTarget, &CounterScopes) -> Result<(), CounterSnapshotLimitExceeded>,
) -> Result<(), CounterSnapshotLimitExceeded> {
    use raikiri_style::property::ContentComponent;
    if !content_has_quotes(cv) {
        return Ok(());
    }
    if depth.open_quotes() != 0 {
        visit(
            CounterTarget::QuoteStart(element, pseudo, depth.open_quotes()),
            scopes,
        )?;
    }
    for component in cv.content.iter() {
        if let ContentComponent::Quote(keyword) = component {
            depth.advance(*keyword);
        }
    }
    Ok(())
}

/// Walk the tree in document order, applying every counter directive to
/// `scopes` and calling `visit` with each element whose box takes part in
/// counting, right after its own directives and before its `::before`, and
/// with each `::before` / `::after` whose content reads a counter, right
/// after that pseudo-element's directives.
///
/// The walk also follows the quote nesting through the generated text in the
/// same order: a list item's `::marker`, then `::before`, the children and
/// `::after`. It calls `visit` with the depth where a pseudo-element's
/// content starts when that content holds a quote and a quote is open.
fn walk_counter_scopes(
    doc: &Document,
    cascade: &CascadeResult,
    scopes: &mut CounterScopes,
    mut visit: impl FnMut(CounterTarget, &CounterScopes) -> Result<(), CounterSnapshotLimitExceeded>,
) -> Result<(), CounterSnapshotLimitExceeded> {
    enum WalkStep {
        Enter(usize),
        Exit(usize),
    }

    let mut stack = vec![WalkStep::Enter(doc.root)];
    let mut pending_pops: Vec<Vec<Symbol>> = Vec::new();
    let mut quote_depth = QuoteDepth::default();
    // The quote depth at which each element that scopes it was entered.
    let mut quote_scopes: Vec<QuoteDepth> = Vec::new();
    // How many elements the walk is inside that draw no boxes below them.
    let mut unrendered = 0_usize;
    while let Some(step) = stack.pop() {
        match step {
            WalkStep::Enter(idx) => {
                let node = &doc.nodes[idx];
                if !node.is_in_document() {
                    continue;
                }

                let cv = cascade.computed.get(idx);
                let display = cv.map(|cv| doc.counter_display(idx, cv.display));
                let is_display_none = display == Some(DisplayValue::None);
                match cv {
                    Some(_) if is_display_none || display == Some(DisplayValue::Contents) => {}
                    Some(cv) => scopes.apply(
                        cv,
                        display == Some(DisplayValue::ListItem),
                        pending_pops.last_mut(),
                    ),
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
                // element's real children.
                visit(CounterTarget::Element(idx), scopes)?;
                stack.push(WalkStep::Exit(idx));
                pending_pops.push(Vec::new());
                if scopes_quote_depth(doc, cascade, idx) {
                    quote_scopes.push(quote_depth);
                }
                if draws_no_boxes_below(node, cv) {
                    unrendered += 1;
                }
                if display == Some(DisplayValue::ListItem)
                    && unrendered == 0
                    && let Some(marker) = generated_pseudo(cascade, idx, PseudoElem::Marker)
                {
                    walk_quotes(
                        marker,
                        (idx, PseudoElem::Marker),
                        &mut quote_depth,
                        scopes,
                        &mut visit,
                    )?;
                }
                if let Some(before) = generated_pseudo(cascade, idx, PseudoElem::Before) {
                    scopes.apply(
                        before,
                        before.display == DisplayValue::ListItem,
                        pending_pops.last_mut(),
                    );
                    if content_reads_counters(before) {
                        visit(CounterTarget::Pseudo(idx, PseudoElem::Before), scopes)?;
                    }
                    if counts_quotes(doc, idx, unrendered) {
                        walk_quotes(
                            before,
                            (idx, PseudoElem::Before),
                            &mut quote_depth,
                            scopes,
                            &mut visit,
                        )?;
                    }
                }
                let children_start = stack.len();
                stack.extend(node.children.iter().map(|&child| WalkStep::Enter(child)));
                stack[children_start..].reverse();
            }
            WalkStep::Exit(idx) => {
                // `::after` is the element's last child: its directives
                // follow the real children and stay in scope for the
                // element's following siblings.
                if let Some(after) = generated_pseudo(cascade, idx, PseudoElem::After) {
                    scopes.apply(
                        after,
                        after.display == DisplayValue::ListItem,
                        pending_pops.last_mut(),
                    );
                    if content_reads_counters(after) {
                        visit(CounterTarget::Pseudo(idx, PseudoElem::After), scopes)?;
                    }
                    if counts_quotes(doc, idx, unrendered) {
                        walk_quotes(
                            after,
                            (idx, PseudoElem::After),
                            &mut quote_depth,
                            scopes,
                            &mut visit,
                        )?;
                    }
                }
                if scopes_quote_depth(doc, cascade, idx)
                    && let Some(entered) = quote_scopes.pop()
                {
                    quote_depth = entered;
                }
                if draws_no_boxes_below(&doc.nodes[idx], cascade.computed.get(idx)) {
                    unrendered -= 1;
                }
                if let Some(names) = pending_pops.pop() {
                    scopes.pop(&names);
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
