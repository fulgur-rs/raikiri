//! `target-*()` / `element()` register-site walker — cascade-time producer
//! that populates a dom-local [`raikiri_traits::TargetRegistry`].
//!
//! Companion to [`crate::running`]'s register-site
//! walker (already landed) — same document-order
//! arena-walk shape, different [`GcpmDirective`] variant.
//! `collect_running_template` in [`mod@crate::running`] already *emits*
//! `CounterIncrement`/`CounterReset`/`CounterSet`/`StringSet` (and
//! `RegisterRunning` is emitted by [`crate::running::build_running_template_store`]
//! itself) into each `position: running(name)` template's own
//! `directives: Vec<GcpmDirective>` — [`crate::phase_b`]'s DOM-tree-walking
//! driver is what *applies* those emitted directives, via
//! `raikiri_traits::page::context::PageContext::apply_directive`. This
//! module's `RegisterTarget` variant has no emit-step counterpart there
//! (see "synthesized here" below) — it *does* now have a real apply-step
//! counterpart on the promoted `PageContext` (its `RegisterTarget` arm registers a
//! counts-only `TargetInfo` from live counter state; see that method's
//! doc), separate from and complementary to this module's own richer
//! text-carrying [`build_target_registry`] walk (wired in via
//! `PageContext::set_targets`, see below). The responsibilities are scoped
//! non-overlapping at the `GcpmDirective`-variant level: `running` and its
//! promoted counterpart own every other variant's emit+apply, this module
//! owns only `RegisterTarget`'s *emit* (there is no `RegisterTarget`
//! producer on the cascade side to begin with — see next section).
//!
//! **Ownership boundary.** [`raikiri_traits::TargetRegistry`] is the
//! canonical shared type (raikiri-traits owns shape + resolve strategy);
//! this module owns only the **producer** — walking the arena and calling
//! [`TargetRegistry::register`]. The `TargetRegistry` instance built here
//! is **dom-local**: [`build_target_registry`] returns an owned
//! `TargetRegistry` to its caller (test setup or a future per-document
//! driver), and that caller wires it into
//! `raikiri_traits::page::context::PageContext.targets` via
//! `PageContext::set_targets` (already landed) — a bulk replace, not a
//! `GcpmDirective`-mediated apply, since this walk never goes through the
//! directive stream in the first place (see "synthesized here" below). No
//! production per-document driver calls `set_targets` yet; that integration is
//! part of a DOM-tree-walking driver that is still to be built.
//!
//! # `GcpmDirective::RegisterTarget` is synthesized here, not consumed
//!
//! Every other producing [`GcpmDirective`] variant
//! (`CounterIncrement`/`CounterReset`/`CounterSet`/`StringSet`) is a direct
//! mirror of a CSS property cascade already resolved onto
//! [`raikiri_style::ComputedValues`] (see `collect_running_template` in [`mod@crate::running`]).
//! `RegisterTarget { fragment_id }` is different: it is **HTML
//! `id`-attribute-driven, not CSS-property-driven** — there is no CSS
//! property whose cascade could produce it, and `raikiri_style::CascadeResult`
//! deliberately carries no `gcpm_directives` field (`crates/raikiri-style/src/cascade.rs`
//! module doc: “`CascadeResult`-level `gcpm_directives` ... now have the
//! downstream (raikiri-dom) responsibility of concatenation per document” — the
//! concept was explicitly moved downstream, not dropped). So this walker
//! **constructs** ("synthesizes") a `GcpmDirective::RegisterTarget` for
//! every in-document element with a non-empty `id` attribute
//! ([`synthesize_register_target`]) and immediately applies it
//! ([`apply_register_target`]) — this is what makes the variant live
//! (previously constructed only in raikiri-traits's own round-trip unit
//! test).
//!
//! # Counter-stack scope model
//!
//! [`raikiri_traits::TargetInfo::counters`] wants each counter's full
//! **nested stack** (outermost scope first, leaf last — see that field's
//! doc). CSS Lists Level 3 §4.3 "Nested counters and scope"
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
//! ([`build_target_registry`]'s `pending_pops` side-stack) — the exit point
//! that keeps the level visible across the instantiating element's own
//! following siblings and their descendants too, matching the
//! following-sibling half of the scope quoted above. A later sibling's own
//! `counter-reset` for the same name then evicts (rather than nests inside)
//! whatever earlier-sibling level is still open, per the obscuring rule
//! quoted above. [`CounterScopes::apply`]'s own doc covers the full
//! mechanism.
//!
//! `counter_increment_continues_across_siblings_sharing_an_ancestor_scope`,
//! `counter_increment_on_following_sibling_of_reset_element_continues_that_scope`,
//! and
//! `counter_increment_without_ancestor_reset_persists_across_siblings`
//! each check one shape of following-sibling continuation: a shared-ancestor
//! scope, a sibling-established `counter-reset` scope, and a
//! no-`counter-reset`-anywhere auto-instantiated scope, respectively.
//! `build_target_registry_counter_scope_resets_between_independent_sections`
//! pins the obscuring rule's counterpart: two independent top-level
//! siblings that both reset the same counter name do not nest — the second
//! section's own reset evicts the first section's still-open level rather
//! than stacking a new one on top of it.
//!
//! A `counter-reset` on this walk's own root element has no parent bucket
//! to record into ([`build_target_registry`]'s `pending_pops` starts empty)
//! and is therefore never popped — inert here, since a [`CounterScopes`]
//! value never outlives the single `build_target_registry` call that owns
//! it (contrast [`crate::phase_b::drive_page`]'s own doc, where the identical
//! root-level non-pop is a real, documented leak, because its
//! `PageContext` state persists across calls).
//!
//! # Text parts — `ContentPart::Content` only
//!
//! [`build_target_info`] populates only [`ContentPart::Content`] (the
//! register-site element's own descendant text, concatenated in document
//! order). `Before`/`After` (pseudo-element content-list resolution) and
//! `FirstLetter` (grapheme segmentation) are NOT populated — both need
//! machinery (content-list resolution, text segmentation) that belongs to
//! paint/layout, not this dom-side arena walk. `TargetInfo::text_part`'s own
//! fallback already resolves missing parts to `""` (CSS Content 3 §2.6.3,
//! documented on `raikiri_traits::page::target`'s `text_parts` field), so
//! leaving them absent here is spec-safe, not a implementation issue.
//!
//! **`collect_descendant_text` scope note.** Gated only on
//! [`raikiri_traits::Node::is_in_document`] (DOM-tree membership), not CSS
//! box generation — text inside a `display: none` descendant (which
//! generates no box) is still concatenated into [`ContentPart::Content`].
//! No CSS Text white-space processing is applied either: `text_content` is
//! concatenated verbatim, so source newlines / runs of whitespace are
//! preserved rather than collapsed. Both are narrowings, not spec
//! violations — CSS Content 3 §2.6.3
//! <https://www.w3.org/TR/css-content-3/#target-text> is itself marked "a
//! very rough draft, and is not ready for implementation" on exactly this
//! point, so there is no normative text to diverge from yet.

use std::collections::HashMap;
use std::mem::size_of;

use raikiri_style::CascadeResult;
use raikiri_style::property::{ContentPart, DisplayValue};
use raikiri_traits::{Dom, Element as _, GcpmDirective, Node as _, NodeId, Symbol};
use raikiri_traits::{TargetInfo, TargetRegistry};

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
}

/// Counter-scope tracker implementing CSS Lists 3 §4.3's nested-scope model
/// (self-nesting, following-sibling inclusion, and sibling obscuring) for
/// one arena-local walk — see module doc "Counter-stack scope model" and
/// [`Self::apply`]'s own doc for the mechanism.
#[derive(Debug, Default)]
struct CounterScopes {
    /// counter name → currently active nested stack (outermost first, leaf
    /// last) — mirrors [`TargetInfo::counters`]'s own shape directly, so
    /// [`Self::snapshot`] is a plain clone.
    stacks: HashMap<Symbol, Vec<i32>>,
}

impl CounterScopes {
    /// Apply one element's `counter-reset` / `counter-increment` /
    /// `counter-set` directives, in CSS Lists 3 §4 processing order (reset →
    /// increment → set — the same order
    /// `collect_running_template` in [`mod@crate::running`] already pins for the
    /// sibling `GcpmDirective`-emit walk, and for the same reason: §4.2's
    /// note that `counter-set` is applied after `counter-increment`).
    ///
    /// Every counter name this call newly *instantiates* — a `counter-reset`
    /// unconditionally, or a `counter-increment`/`counter-set` on a name
    /// with no active level yet (CSS Lists 3 §4.4.2) — is recorded into
    /// `parent_bucket`, not tracked against this element itself: CSS Lists 3
    /// §4.3 scopes a counter-reset to "the element's descendants and its
    /// following siblings with their descendants", so the pop must wait for
    /// the *parent's* subtree exit, not this element's own.
    /// [`build_target_registry`]'s `pending_pops` side-stack passes its own
    /// top-of-stack bucket — the one this element's parent pushed — as
    /// `parent_bucket` here, and pops it (via [`Self::pop`]) when that
    /// parent's own `Exit` step runs. `parent_bucket` is `None` only when
    /// this element is the walk's root (no `Enter` has pushed a bucket
    /// yet); a root-level instantiation is then left untracked and never
    /// popped — harmless here, since a [`CounterScopes`] value never
    /// outlives the one [`build_target_registry`] call that owns it
    /// (contrast [`crate::phase_b::drive_page`]'s own doc, where the same
    /// root-level non-pop is a real, documented leak because its
    /// `PageContext` state persists across calls).
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
    /// [`build_target_registry`]'s `pending_pops` side-stack — invoked when
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

    /// Snapshot the current live stacks — directly the shape
    /// [`TargetInfo::counters`] stores (outermost first, leaf last).
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

/// `id` attribute of the element at arena index `idx`, or `None` if the node
/// is not an element or the `id` attribute is absent/empty.
///
/// Routed through `raikiri_traits::{Dom, Node, Element}` (rather than
/// hand-matching `NodeData::Element` + attribute list directly, the way
/// [`crate::layout`] / [`crate::document`] do for their own internal needs)
/// specifically to reuse [`raikiri_traits::Element::id`]'s already-tested
/// empty-value filter (`crates/raikiri-dom/src/lib.rs`'s
/// `element_id_treats_empty_value_as_none_while_attr_preserves_presence`
/// pins the same contract at the trait-impl layer) instead of re-deriving
/// it here.
fn element_id(doc: &Document, idx: usize) -> Option<String> {
    let node = doc.node(NodeId::new(idx as u64))?;
    let element = node.as_element()?;
    element.id().map(str::to_owned)
}

/// Concatenate the document-order text content of every in-document Text
/// descendant of `root_idx` (the register-site element itself contributes no
/// text — only its descendants do).
///
/// Same reverse-push-children iterative DFS shape as
/// `collect_running_template` in [`mod@crate::running`] (and, one level up,
/// [`build_target_registry`] itself) — see that function's doc for the
/// document-order rationale. Nested per register-site element, same
/// per-subtree walk cost tradeoff `collect_running_template` already
/// accepts for its own per-template subtree walk.
#[allow(
    dead_code,
    reason = "Helper for build_target_registry; \
              same not-yet-production-driven status."
)]
fn collect_descendant_text(doc: &Document, root_idx: usize) -> String {
    let mut text = String::new();
    let mut stack: Vec<usize> = vec![root_idx];
    while let Some(idx) = stack.pop() {
        let node = &doc.nodes[idx];
        if !node.is_in_document() {
            continue;
        }
        if let crate::node::NodeData::Text(t) = &node.data {
            text.push_str(t.text_content.as_str());
        }
        for &child in node.children.iter().rev() {
            stack.push(child);
        }
    }
    text
}

/// Construct a `GcpmDirective::RegisterTarget` for `fragment_id` — see
/// module doc "`GcpmDirective::RegisterTarget` is synthesized here, not
/// consumed" for why this walker builds the directive itself rather than
/// reading one off cascade output.
fn synthesize_register_target(fragment_id: &str) -> GcpmDirective {
    GcpmDirective::RegisterTarget {
        fragment_id: Symbol::new(fragment_id),
    }
}

/// Apply a `GcpmDirective::RegisterTarget` to `registry` by calling
/// [`TargetRegistry::register`].
///
/// Takes the directive (rather than just a `Symbol`) so the synthesize →
/// apply round trip in [`build_target_registry`] genuinely flows through the
/// `GcpmDirective` enum, matching the shape a future real Phase B directive
/// dispatcher (applying directives read off `ParsedRunningTemplate` /
/// similar, rather than synthesized locally) would use.
fn apply_register_target(
    directive: GcpmDirective,
    info: TargetInfo,
    registry: &mut TargetRegistry,
) {
    // `if let` rather than an exhaustive `match` with a wildcard arm
    // (clippy::single_match): this function's sole caller
    // (`build_target_registry`) only ever passes a directive built by
    // `synthesize_register_target`, which always constructs
    // `RegisterTarget` — the non-matching case is unreachable in practice,
    // not a real dispatch branch worth spelling out.
    if let GcpmDirective::RegisterTarget { fragment_id } = directive {
        registry.register(fragment_id, info);
    }
}

/// Build the [`TargetInfo`] for the register-site element at arena index
/// `idx`: `scopes`'s current live counter stacks (module doc "Counter-stack
/// scope model") plus the element's own descendant text
/// ([`collect_descendant_text`], stored under [`ContentPart::Content`] — see
/// module doc "Text parts").
#[allow(
    dead_code,
    reason = "Helper for build_target_registry; \
              same not-yet-production-driven status."
)]
fn build_target_info(
    doc: &Document,
    idx: usize,
    scopes: &CounterScopes,
    budget: &mut CounterSnapshotBudget,
) -> Result<TargetInfo, CounterSnapshotLimitExceeded> {
    let mut info = TargetInfo::default();
    info.counters = scopes.snapshot(budget)?;
    info.text_parts
        .push((ContentPart::Content, collect_descendant_text(doc, idx)));
    Ok(info)
}

/// Document-order arena walk that registers every in-document element with a
/// non-empty `id` attribute into a fresh, **dom-local**
/// [`TargetRegistry`] — the "register-site walker" this module
/// builds (design doc §7.2's `TargetRegistry`/`TargetInfo` canonical shape;
/// see module doc for the full ownership boundary — this function itself
/// does NOT wire the returned registry into `raikiri_traits::PageContext`;
/// `PageContext::set_targets` was added for that, to be
/// called by this function's future per-document caller).
///
/// Walks `doc` from its root in document order (iterative DFS with explicit
/// `Enter`/`Exit` steps — same reverse-push-children shape as
/// [`crate::layout::find_body`] / [`crate::running::build_running_template_store`],
/// extended with an `Exit` step and a `pending_pops: Vec<Vec<Symbol>>`
/// side-stack — the same shape [`crate::phase_b`]'s `walk_directives` uses
/// against the promoted `PageContext` type — so [`CounterScopes`] pops each
/// pushed scope level at the *pushing element's parent's* `Exit`, not the
/// pushing element's own; see [`CounterScopes::apply`]'s doc for why). For
/// every in-document element with a non-empty `id`
/// ([`element_id`]), synthesizes a `GcpmDirective::RegisterTarget`
/// ([`synthesize_register_target`]), builds a [`TargetInfo`]
/// ([`build_target_info`]), and applies the directive
/// ([`apply_register_target`]) — which calls
/// [`TargetRegistry::register`]. `register`'s own `entry().or_insert`
/// first-wins semantics (see that method's doc) combined with this walker's
/// document-order traversal reproduces the DOM Standard's `getElementById`
/// first-in-tree-order id resolution
/// (<https://dom.spec.whatwg.org/#dom-nonelementparentnode-getelementbyid>,
/// "must return the first element, in tree order ... whose ID is
/// elementId") without this walker needing to check for duplicate ids
/// itself — HTML §3.2.6.1 only establishes that such duplicates are
/// non-conforming in the first place
/// (<https://html.spec.whatwg.org/multipage/dom.html#the-id-attribute>).
///
/// A `display: none` element never runs its own counter directives (CSS
/// Lists 3 §4.5, see the `Enter` step's own comment below) — but it can
/// still register as a target, since `target-*()` addresses fragments by
/// `id`, not by box generation. Its descendants get neither: CSS Display 3
/// <https://www.w3.org/TR/css-display-3/#typedef-display-box> omits the
/// element's entire subtree from the box tree, so none of them generate a
/// box either regardless of their own computed `display` (`display` is not
/// an inherited property) — the `Enter` step skips a `display: none`
/// element's whole subtree, registering only the element itself before
/// doing so.
///
/// **This makes a display:none descendant's own `id` unaddressable by
/// `target-*()`**, unlike the display:none element itself — a narrowing of
/// the `id`-based addressability argument above, not something CSS Lists 3
/// §4.5 (which only withdraws counter effects) requires. The alternative —
/// walking into the subtree only far enough to register ids while still
/// suppressing every counter directive there — would need a second,
/// separate traversal mode; this walker instead skips the whole subtree in
/// one pass, matching `Node::is_display_none()`'s paint-stage convention
/// (see above). CSS Content 3 §2.6.3's `target-text()` addressing has no
/// display/box-generation dependency at all, so this narrowing is tracked
/// as a known limitation, not fixed here.
///
/// # Precondition
///
/// Same as [`crate::running::build_running_template_store`]: `doc`'s
/// `IS_IN_DOCUMENT` flags must be up to date (see
/// [`Document::mark_in_document_flags`]), matching
/// [`raikiri_style::cascade()`]'s own precondition, since this walker is meant
/// to run against the very `cascade` output produced from `doc`.
#[allow(
    dead_code,
    reason = "Register-site walker landed; \
              PageContext::set_targets exists to \
              receive this fn's output, but no production per-document \
              driver calls either one yet — that driver is still to be \
              built (see module doc's \
              ownership boundary). Exercised via unit tests until then, \
              same not-yet-production-driven status \
              crate::running::build_running_template_store carries."
)]
pub(crate) fn build_target_registry(
    doc: &Document,
    cascade: &CascadeResult,
) -> Result<TargetRegistry, CounterSnapshotLimitExceeded> {
    let mut registry = TargetRegistry::default();
    let mut scopes = CounterScopes::default();
    let mut budget = CounterSnapshotBudget::default();

    enum WalkStep {
        Enter(usize),
        Exit,
    }

    let mut stack = vec![WalkStep::Enter(doc.root)];
    // `pending_pops[i]` holds the counter names to pop (via
    // `CounterScopes::pop`) at the `Exit` matching the `Enter` that pushed
    // bucket `i`. A name lands in the *parent's* bucket — the one on top of
    // `pending_pops` at the moment the instantiating element is entered,
    // i.e. `pending_pops.last_mut()` passed into `CounterScopes::apply` as
    // `parent_bucket` — rather than a bucket of the instantiating element's
    // own, so it is popped at the parent's `Exit` and stays visible to the
    // instantiating element's own following siblings (CSS Lists 3 §4.3, see
    // `CounterScopes::apply`'s doc). Same shape as
    // `crate::phase_b::walk_directives`'s identically-named mechanism
    // against the promoted `PageContext` type.
    let mut pending_pops: Vec<Vec<Symbol>> = Vec::new();
    while let Some(step) = stack.pop() {
        match step {
            WalkStep::Enter(idx) => {
                let node = &doc.nodes[idx];
                if !node.is_in_document() {
                    continue;
                }

                let cv = cascade.computed.get(idx);
                let is_display_none = matches!(cv, Some(cv) if cv.display == DisplayValue::None);

                match cv {
                    Some(cv) if is_display_none || cv.display == DisplayValue::Contents => {
                        // CSS Lists 3 §4.5
                        // <https://www.w3.org/TR/css-lists-3/#counters-in-elements-that-do-not-generate-boxes>:
                        // an element that does not generate a box "cannot
                        // set, reset, or increment a counter ... they must
                        // have no effect." `display: none` (whole-subtree
                        // box omission, CSS Display 3
                        // <https://www.w3.org/TR/css-display-3/#typedef-display-box>)
                        // and `display: contents` (element generates no box
                        // of its own, CSS Display 3 §2.5) both qualify —
                        // skip this element's own directive application
                        // either way; it may still register as a target
                        // below (target-* doesn't require box generation,
                        // only an id).
                    }
                    Some(cv) => scopes.apply(cv, pending_pops.last_mut()),
                    // cov:ignore: `raikiri_style::cascade`'s own contract
                    // (`computed.len() == doc.node_count()`) guarantees
                    // `Some` for every valid arena index when `cascade` was
                    // produced from this `doc` — same defensive shape
                    // `crate::running::collect_running_template`'s matching
                    // `.get(idx)` note documents.
                    None => {}
                }

                if let Some(fragment_id) = element_id(doc, idx) {
                    let directive = synthesize_register_target(&fragment_id);
                    let info = build_target_info(doc, idx, &scopes, &mut budget)?;
                    apply_register_target(directive, info, &mut registry);
                }

                if is_display_none {
                    // CSS Display 3
                    // <https://www.w3.org/TR/css-display-3/#typedef-display-box>
                    // omits the element's entire subtree from the box
                    // tree — none of its descendants generate a box
                    // either, regardless of their own computed `display`
                    // (`display` is not an inherited property), so CSS
                    // Lists 3 §4.5's "must have no effect" for counters
                    // extends to the whole subtree, not just this element.
                    // Don't push this element's `Exit` step, `pending_pops`
                    // bucket, or any child onto the walk, matching
                    // `Node::is_display_none()`'s paint-stage subtree-skip
                    // convention — unlike `display: contents` above, which
                    // still walks its subtree below. Skipping the bucket
                    // push is itself a no-op either way: the match above
                    // never calls `scopes.apply` for this arm, so no bucket
                    // this element's descendants might have pushed into
                    // could exist regardless.
                    continue;
                }

                stack.push(WalkStep::Exit);
                // This element's OWN bucket — accumulates any newly
                // instantiated counter-scope names its own children push
                // (see the `Enter` arm above), to be popped when this
                // element's `Exit` (just pushed above) is reached. An
                // id-less element (e.g. a bare `counter-reset` with no id)
                // still needs one, since its *children* may push into it.
                pending_pops.push(Vec::new());

                for &child in node.children.iter().rev() {
                    stack.push(WalkStep::Enter(child));
                }
            }
            WalkStep::Exit => {
                // `pending_pops.pop()` is `None` only if this `Exit` has no
                // matching `Enter` bucket — structurally impossible: every
                // `Enter(idx)` path that reaches this point pushes exactly
                // one `WalkStep::Exit` and one `pending_pops` bucket
                // together (the `!is_in_document()` and `display: none`
                // early `continue`s skip both). Kept as a graceful no-op
                // rather than `.expect()` so a violation, if one ever
                // existed, can't panic on parsed DOM input.
                if let Some(names) = pending_pops.pop() {
                    scopes.pop(&names);
                }
            }
        }
    }

    Ok(registry)
}

/// Snapshot of every live named counter stack after applying the directives on
/// the corresponding node.
///
/// The outer-to-inner stack shape matches the `counters()` data consumed by
/// marker and generated-content paint. Nodes that do not generate a box do not
/// apply their own directives; descendants of `display: none` are omitted.
/// This is an owned snapshot so paint can resolve counters without borrowing
/// the DOM walk's transient scope state.
pub type CounterSnapshot = HashMap<Symbol, Vec<i32>>;

/// Build named-counter snapshots in document order for marker and generated
/// content consumers.
///
/// The traversal deliberately shares [`CounterScopes`] with
/// [`build_target_registry`], including sibling obscuring, self-nesting,
/// saturating arithmetic, and `display: none` / `display: contents` rules.
/// `snapshots[idx]` is empty for an unreachable or skipped node.
pub fn counter_snapshots(
    doc: &Document,
    cascade: &CascadeResult,
) -> Result<Vec<CounterSnapshot>, CounterSnapshotLimitExceeded> {
    counter_snapshots_with_budget(doc, cascade, &mut CounterSnapshotBudget::default())
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

    enum WalkStep {
        Enter(usize),
        Exit,
    }

    let mut stack = vec![WalkStep::Enter(doc.root)];
    let mut pending_pops: Vec<Vec<Symbol>> = Vec::new();
    while let Some(step) = stack.pop() {
        match step {
            WalkStep::Enter(idx) => {
                let node = &doc.nodes[idx];
                if !node.is_in_document() {
                    continue;
                }

                let cv = cascade.computed.get(idx);
                let is_display_none = matches!(cv, Some(cv) if cv.display == DisplayValue::None);
                match cv {
                    Some(cv) if is_display_none || cv.display == DisplayValue::Contents => {}
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
                for &child in node.children.iter().rev() {
                    stack.push(WalkStep::Enter(child));
                }
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
