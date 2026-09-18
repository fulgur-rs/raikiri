//! `target-*()` / `element()` register-site walker — cascade-time producer
//! that populates a dom-local [`raikiri_traits::TargetRegistry`].
//!
//! Companion to [`crate::running`]'s register-site
//! walker (already landed) — same document-order
//! arena-walk shape, different [`GcpmDirective`] variant. `running`'s
//! [`mod@crate::running`] の `collect_running_template` already *emits*
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
//! [`raikiri_style::ComputedValues`] (see [`mod@crate::running`] の `collect_running_template`).
//! `RegisterTarget { fragment_id }` is different: it is **HTML
//! `id`-attribute-driven, not CSS-property-driven** — there is no CSS
//! property whose cascade could produce it, and `raikiri_style::CascadeResult`
//! deliberately carries no `gcpm_directives` field (`crates/raikiri-style/src/cascade.rs`
//! module doc: "`CascadeResult`-level の `gcpm_directives` ... は下流
//! (raikiri-dom) で per-document に concatenate される責務に移った" — the
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

use raikiri_style::CascadeResult;
use raikiri_style::property::{ContentPart, DisplayValue};
use raikiri_traits::{Dom, Element as _, GcpmDirective, Node as _, NodeId, Symbol};
use raikiri_traits::{TargetInfo, TargetRegistry};

use crate::document::Document;

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
    /// [`mod@crate::running`] の `collect_running_template` already pins for the
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
    fn snapshot(&self) -> HashMap<Symbol, Vec<i32>> {
        self.stacks.clone()
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
/// [`mod@crate::running`] の `collect_running_template` (and, one level up,
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
fn build_target_info(doc: &Document, idx: usize, scopes: &CounterScopes) -> TargetInfo {
    let mut info = TargetInfo::default();
    info.counters = scopes.snapshot();
    info.text_parts
        .push((ContentPart::Content, collect_descendant_text(doc, idx)));
    info
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
pub(crate) fn build_target_registry(doc: &Document, cascade: &CascadeResult) -> TargetRegistry {
    let mut registry = TargetRegistry::default();
    let mut scopes = CounterScopes::default();

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
                    let info = build_target_info(doc, idx, &scopes);
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

    registry
}

#[cfg(test)]
mod tests {
    use super::*;

    use raikiri_style::{build_rule_tree, cascade};
    use smol_str::SmolStr;
    use taffy::Style;

    // ── synthesize_register_target / apply_register_target ─────────

    #[test]
    fn synthesize_register_target_builds_register_target_variant() {
        let directive = synthesize_register_target("chapter-1");
        assert_eq!(
            directive,
            GcpmDirective::RegisterTarget {
                fragment_id: Symbol::new("chapter-1"),
            }
        );
    }

    #[test]
    fn apply_register_target_calls_registry_register() {
        let mut registry = TargetRegistry::default();
        let directive = synthesize_register_target("chapter-1");
        let mut info = TargetInfo::default();
        info.counters.insert(Symbol::new("chapter"), vec![1]);
        apply_register_target(directive, info, &mut registry);

        let out = registry.resolve_target_counter(
            "#chapter-1",
            Symbol::new("chapter"),
            raikiri_style::property::CounterStyle::Decimal,
        );
        assert_eq!(
            out,
            raikiri_traits::ResolveOutcome::Resolved("1".to_owned())
        );
    }

    // ── CounterScopes ────────────────────────────────────────────
    //
    // `raikiri_style::ComputedValues` is `#[non_exhaustive]` cross-crate
    // (raikiri-dom is not its defining crate), so it cannot be struct-
    // literal-constructed here even with every field supplied — there is no
    // builder either (matching `crate::running`'s own tests, which never
    // construct `ComputedValues` directly). `CounterScopes` is therefore
    // exercised only through real `cascade()` output, below, via
    // `build_target_registry`'s integration tests.

    // ── build_target_registry (arena-walk integration) ──────────────

    fn set_id(doc: &mut Document, idx: usize, id: &str) {
        doc.set_element_attributes(idx, vec![(SmolStr::new("id"), SmolStr::new(id))]);
    }

    #[test]
    fn build_target_registry_registers_element_with_id() {
        let mut doc = Document::new();
        let h1 = doc.append_element(Some(0), "h1", Style::default(), None::<&str>);
        set_id(&mut doc, h1, "intro");
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let mut registry = build_target_registry(&doc, &cr);
        let out = registry.resolve_target_counter(
            "#intro",
            Symbol::new("nonexistent"),
            raikiri_style::property::CounterStyle::Decimal,
        );
        // Missing counter on an existing (registered) target resolves to
        // "0" immediately, not Pending — proves `#intro` landed in
        // `resolved`.
        assert_eq!(
            out,
            raikiri_traits::ResolveOutcome::Resolved("0".to_owned())
        );
    }

    #[test]
    fn build_target_registry_ignores_elements_without_id() {
        let mut doc = Document::new();
        doc.append_element(Some(0), "div", Style::default(), None::<&str>);
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let mut registry = build_target_registry(&doc, &cr);
        let out = registry.resolve_target_counter(
            "#anything",
            Symbol::new("c"),
            raikiri_style::property::CounterStyle::Decimal,
        );
        // A fresh registry's first dispatch is page 0 / sequence 0.
        assert_eq!(
            out,
            raikiri_traits::ResolveOutcome::Pending(raikiri_traits::TargetSlotId {
                page_index: 0,
                sequence: 0,
            })
        );
    }

    #[test]
    fn element_id_returns_none_for_empty_id_attribute() {
        // Empty id="" must not register — mirrors
        // element_id_treats_empty_value_as_none_while_attr_preserves_presence
        // (crates/raikiri-dom/src/lib.rs) at the walker level. Asserted
        // directly against `element_id` (not through a resolve() round trip
        // — an empty id would key the registry under `Symbol::new("")`,
        // which happens to be unobservable via resolve_target_counter/etc.
        // since every target-* URL parses to a non-empty fragment or `None`,
        // so a resolve-based assertion here would pass whether or not the
        // empty id actually got filtered).
        let mut doc = Document::new();
        let div = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
        set_id(&mut doc, div, "");

        assert_eq!(element_id(&doc, div), None);
    }

    #[test]
    fn build_target_registry_first_wins_on_duplicate_id_in_tree_order() {
        // The DOM Standard's getElementById algorithm defines
        // first-in-tree-order resolution
        // (https://dom.spec.whatwg.org/#dom-nonelementparentnode-getelementbyid);
        // HTML §3.2.6.1 only establishes that duplicate ids are
        // non-conforming in the first place
        // (https://html.spec.whatwg.org/multipage/dom.html#the-id-attribute).
        // This walker must preserve document-order registration so
        // TargetRegistry::register's entry().or_insert first-wins policy
        // lands on the right element.
        let mut doc = Document::new();
        let first = doc.append_element(Some(0), "h1", Style::default(), None::<&str>);
        set_id(&mut doc, first, "dup");
        doc.append_text(first, "First");

        let second = doc.append_element(Some(0), "h1", Style::default(), None::<&str>);
        set_id(&mut doc, second, "dup");
        doc.append_text(second, "Second");

        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let mut registry = build_target_registry(&doc, &cr);
        let out = registry.resolve_target_text("#dup", ContentPart::Content);
        assert_eq!(
            out,
            raikiri_traits::ResolveOutcome::Resolved("First".to_owned()),
            "first element in tree order must win the duplicate id"
        );
    }

    #[test]
    fn build_target_registry_same_name_nested_reset_produces_two_level_stack() {
        // CSS Lists 3 §4.3 "self-nesting": a SECOND counter-reset for the
        // *same* name, encountered while an ancestor's scope for that name
        // is still active, must push a NESTED level rather than replace it
        // — outermost first, leaf last (module doc "Counter-stack scope
        // model"), which resolve_target_counters's hierarchical join
        // ("1.1") makes directly observable.
        let mut doc = Document::new();
        let outer = doc.append_element(
            Some(0),
            "section",
            Style::default(),
            Some("counter-reset: sec 1"),
        );
        let inner = doc.append_element(
            Some(outer),
            "section",
            Style::default(),
            Some("counter-reset: sec 1"),
        );
        let leaf = doc.append_element(Some(inner), "h2", Style::default(), None::<&str>);
        set_id(&mut doc, leaf, "leaf");
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let mut registry = build_target_registry(&doc, &cr);
        let out = registry.resolve_target_counters(
            "#leaf",
            Symbol::new("sec"),
            ".",
            raikiri_style::property::CounterStyle::Decimal,
        );
        assert_eq!(
            out,
            raikiri_traits::ResolveOutcome::Resolved("1.1".to_owned())
        );
    }

    #[test]
    fn build_target_registry_increment_mutates_top_level_not_a_new_one() {
        // counter-reset and counter-increment for the SAME name on the SAME
        // element: increment must mutate the level counter-reset just
        // pushed, not push an additional nested level. resolve_target_counter
        // (leaf-only read) can't distinguish a correct single-level stack
        // "7" from a wrongly-pushed two-level stack "5, 2" (both read "2" as
        // the leaf) — resolve_target_counters over the full stack is
        // required to check this.
        let mut doc = Document::new();
        let el = doc.append_element(
            Some(0),
            "h2",
            Style::default(),
            Some("counter-reset: c 5; counter-increment: c 2"),
        );
        set_id(&mut doc, el, "el");
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let mut registry = build_target_registry(&doc, &cr);
        let out = registry.resolve_target_counters(
            "#el",
            Symbol::new("c"),
            ".",
            raikiri_style::property::CounterStyle::Decimal,
        );
        assert_eq!(
            out,
            raikiri_traits::ResolveOutcome::Resolved("7".to_owned())
        );
    }

    #[test]
    fn build_target_registry_increment_saturates_instead_of_panicking_or_wrapping_on_overflow() {
        // Regression check: spec-legal CSS driving
        // counter-increment past i32::MAX must saturate, not panic (debug
        // builds) or wrap to i32::MIN (release builds). Same finding, same
        // fix, as `CounterStack::increment` in `raikiri-traits`'s
        // `page::context` — this walker's top-of-stack increment shared
        // the exact same unbounded `+=` bug.
        //
        // reset is deliberately `i32::MAX - 1`, not `i32::MAX`, so the
        // expected result (`i32::MAX`) is reachable ONLY by actually adding
        // `delta` — a dropped/no-op increment would leave the leaf at
        // `2147483646`, not `i32::MAX`, so this also pins that the
        // increment declaration is applied at all, not just that it
        // saturates.
        let mut doc = Document::new();
        let el = doc.append_element(
            Some(0),
            "h2",
            Style::default(),
            Some("counter-reset: c 2147483646; counter-increment: c 5"),
        );
        set_id(&mut doc, el, "el");
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let mut registry = build_target_registry(&doc, &cr);
        let out = registry.resolve_target_counter(
            "#el",
            Symbol::new("c"),
            raikiri_style::property::CounterStyle::Decimal,
        );
        assert_eq!(
            out,
            raikiri_traits::ResolveOutcome::Resolved(i32::MAX.to_string()),
            "increment past i32::MAX must saturate, not panic or wrap"
        );
    }

    #[test]
    fn build_target_registry_duplicate_name_in_one_counter_reset_keeps_only_last_value() {
        // CSS Lists 3 §4.1 <https://www.w3.org/TR/css-lists-3/#counter-reset>:
        // "If multiple instances of the same <counter-name> occur in the
        // property value, only the last one is honored." A single
        // `counter-reset: c 1 c 2` must push exactly ONE scope level with
        // value 2 — resolve_target_counters over the full stack (not just
        // the leaf) is required to distinguish a correct one-level "2" from
        // a wrongly-pushed two-level "1.2".
        let mut doc = Document::new();
        let el = doc.append_element(
            Some(0),
            "h2",
            Style::default(),
            Some("counter-reset: c 1 c 2"),
        );
        set_id(&mut doc, el, "el");
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let mut registry = build_target_registry(&doc, &cr);
        let out = registry.resolve_target_counters(
            "#el",
            Symbol::new("c"),
            ".",
            raikiri_style::property::CounterStyle::Decimal,
        );
        assert_eq!(
            out,
            raikiri_traits::ResolveOutcome::Resolved("2".to_owned()),
            "only the last counter-reset occurrence for a duplicated name is honored"
        );
    }

    #[test]
    fn build_target_registry_display_none_ancestor_skips_the_entire_descendant_subtree() {
        // CSS Display 3 <https://www.w3.org/TR/css-display-3/#typedef-display-box>
        // omits the element's entire subtree from the box tree — none of
        // its descendants generate a box either, regardless of their own
        // computed `display` (`display` is not an inherited property, so
        // `target`'s own computed display below is not none). CSS Lists 3
        // §4.5's "must have no effect" for counters
        // (<https://www.w3.org/TR/css-lists-3/#counters-in-elements-that-do-not-generate-boxes>)
        // therefore extends to the whole subtree: `target` must not
        // register as a target at all — not merely register with its own
        // directive suppressed — matching `Node::is_display_none()`'s
        // paint-stage subtree-skip convention. `hidden` (the display:none
        // element itself) is given an `id` too and asserted `Resolved` in
        // this same document, pinning the exact boundary this function's
        // own doc draws ("This makes a display:none descendant's own `id`
        // unaddressable ... unlike the display:none element itself") rather
        // than merely showing SOME failure to register `target`.
        let mut doc = Document::new();
        let hidden = doc.append_element(
            Some(0),
            "div",
            Style::default(),
            Some("display: none; counter-reset: c 5"),
        );
        set_id(&mut doc, hidden, "hidden");
        let target = doc.append_element(
            Some(hidden),
            "span",
            Style::default(),
            Some("counter-increment: c"),
        );
        set_id(&mut doc, target, "target");
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let mut registry = build_target_registry(&doc, &cr);
        let hidden_out = registry.resolve_target_counter(
            "#hidden",
            Symbol::new("c"),
            raikiri_style::property::CounterStyle::Decimal,
        );
        let target_out = registry.resolve_target_counter(
            "#target",
            Symbol::new("c"),
            raikiri_style::property::CounterStyle::Decimal,
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            hidden_out,
            raikiri_traits::ResolveOutcome::Resolved("0".to_owned()),
            "the display:none element itself must still register (its own \
             counter-reset has no effect on itself either, per CSS Lists 3 \
             §4.5)"
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            matches!(target_out, raikiri_traits::ResolveOutcome::Pending(_)),
            "a display:none ancestor's descendant must never be visited by the \
             walk at all, so it never registers as a target — got {target_out:?}"
        );
    }

    #[test]
    fn build_target_registry_display_none_element_itself_still_registers_as_target() {
        // Companion to build_target_registry_display_none_ancestor_skips_the_entire_descendant_subtree:
        // that test pins that a display:none element's DESCENDANT is never
        // visited or registered at all (while the display:none element
        // itself still resolves there too); this one isolates that second
        // half — a display:none element with no descendant of its own
        // still registers as a target, so skipping directive application
        // doesn't also accidentally skip TARGET registration for the
        // display:none element itself. CSS Lists 3 §4.5 only withdraws
        // counter set/reset/increment from non-box-generating elements —
        // it says nothing about target-*() addressability, which is purely
        // id-based (see build_target_registry's own "Elements with
        // display: none" doc note).
        let mut doc = Document::new();
        let hidden = doc.append_element(Some(0), "div", Style::default(), Some("display: none"));
        set_id(&mut doc, hidden, "hidden");
        doc.append_text(hidden, "Hidden but addressable");
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let mut registry = build_target_registry(&doc, &cr);
        let out = registry.resolve_target_text("#hidden", ContentPart::Content);
        assert_eq!(
            out,
            raikiri_traits::ResolveOutcome::Resolved("Hidden but addressable".to_owned()),
            "a display:none element with no counter properties must still register \
             as a target — display:none only withdraws counter effects, not \
             target-*() addressability"
        );
    }

    #[test]
    fn build_target_registry_display_contents_element_does_not_affect_counter_stack() {
        // display:contents also generates no box for the element itself
        // (CSS Display 3 §2.5), so CSS Lists 3 §4.5's "no effect" rule
        // applies here too — but unlike display:none
        // (build_target_registry_display_none_ancestor_skips_the_entire_descendant_subtree),
        // display:contents does not remove its descendants from the box
        // tree, so `target` below is still walked and registered normally.
        //
        // Regression-check shape: the id-bearing probe is a *descendant* of
        // the display:contents element, registered while that element's
        // scope (if any had wrongly been pushed) would still be directly
        // observable — the most direct way to check "this element's own
        // counter-reset never even ran".
        let mut doc = Document::new();
        let contents = doc.append_element(
            Some(0),
            "div",
            Style::default(),
            Some("display: contents; counter-reset: c 5"),
        );
        let target = doc.append_element(Some(contents), "span", Style::default(), None::<&str>);
        set_id(&mut doc, target, "target");
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let mut registry = build_target_registry(&doc, &cr);
        let out = registry.resolve_target_counter(
            "#target",
            Symbol::new("c"),
            raikiri_style::property::CounterStyle::Decimal,
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            out,
            raikiri_traits::ResolveOutcome::Resolved("0".to_owned()),
            "a display:contents ancestor's own counter-reset must have no effect at all, \
             even observed from inside its still-open (but skipped) scope"
        );
    }

    #[test]
    fn counter_increment_continues_across_siblings_sharing_an_ancestor_scope() {
        // The shared-ancestor shape of following-sibling continuation
        // (module doc "Counter-stack scope model"): two <p> children of a
        // common <section> that itself carries counter-reset both see, and
        // continue, that still-open ancestor scope — contrast with
        // counter_increment_on_following_sibling_of_reset_element_continues_that_scope,
        // where the reset is on a SIBLING rather than a shared ancestor.
        let mut doc = Document::new();
        let section = doc.append_element(
            Some(0),
            "section",
            Style::default(),
            Some("counter-reset: item"),
        );
        let p1 = doc.append_element(
            Some(section),
            "p",
            Style::default(),
            Some("counter-increment: item"),
        );
        set_id(&mut doc, p1, "p1");
        let p2 = doc.append_element(
            Some(section),
            "p",
            Style::default(),
            Some("counter-increment: item"),
        );
        set_id(&mut doc, p2, "p2");
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let mut registry = build_target_registry(&doc, &cr);
        let out1 = registry.resolve_target_counter(
            "#p1",
            Symbol::new("item"),
            raikiri_style::property::CounterStyle::Decimal,
        );
        let out2 = registry.resolve_target_counter(
            "#p2",
            Symbol::new("item"),
            raikiri_style::property::CounterStyle::Decimal,
        );
        assert_eq!(
            out1,
            raikiri_traits::ResolveOutcome::Resolved("1".to_owned())
        );
        assert_eq!(
            out2,
            raikiri_traits::ResolveOutcome::Resolved("2".to_owned()),
            "p2 must continue p1's increment within the shared section's still-open scope"
        );
    }

    #[test]
    fn build_target_registry_counter_increment_visible_across_cross_branch_cousin() {
        // CSS Lists 3 §4.4.1 "Inheriting Counters"
        // <https://www.w3.org/TR/css-lists-3/#inheriting-counters>'s own
        // worked example: `#baz` "inherits the example counter from the
        // #foo element, its previous sibling. However, rather than
        // inheriting the value 1 from #foo along with the counter, it
        // inherits the value 2 from #bar, the previous element in tree
        // order" — `#bar` is `#foo`'s own child, so `#baz` (a plain sibling
        // of `#foo`, not a descendant of it) must see the mutation `#bar`
        // made two levels deeper than `#baz` itself sits, not merely
        // `#foo`'s own top-level increment.
        let mut doc = Document::new();
        let ul = doc.append_element(
            Some(0),
            "ul",
            Style::default(),
            Some("counter-reset: example 0"),
        );
        let foo = doc.append_element(
            Some(ul),
            "li",
            Style::default(),
            Some("counter-increment: example"),
        );
        set_id(&mut doc, foo, "foo");
        let bar = doc.append_element(
            Some(foo),
            "div",
            Style::default(),
            Some("counter-increment: example"),
        );
        set_id(&mut doc, bar, "bar");
        let baz = doc.append_element(Some(ul), "li", Style::default(), None::<&str>);
        set_id(&mut doc, baz, "baz");
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let mut registry = build_target_registry(&doc, &cr);
        let out = registry.resolve_target_counter(
            "#baz",
            Symbol::new("example"),
            raikiri_style::property::CounterStyle::Decimal,
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            out,
            raikiri_traits::ResolveOutcome::Resolved("2".to_owned()),
            "#baz must inherit the shared <ul> ancestor scope's CURRENT value, as \
             mutated by #bar (its own preceding sibling #foo's child), not #foo's \
             own top-level increment value"
        );
    }

    #[test]
    fn build_target_registry_counter_scope_resets_between_independent_sections() {
        // Two independent top-level <section>s, each with their own
        // counter-reset: chapter — the second section's <h2 id> must see a
        // FRESH scope (not the first section's leftover value). Pins CSS
        // Lists 3 §4.3's obscuring rule (module doc "Counter-stack scope
        // model"): sec2's own counter-reset evicts sec1's still-open
        // same-name level (kept open, per the following-sibling half of
        // §4.3, until their shared parent's subtree exit) instead of
        // nesting a new level inside it — without the obscuring rule this
        // would instead read "1.5" via `resolve_target_counters`, even
        // though the leaf-only `resolve_target_counter` used below can't
        // tell the two outcomes apart (see
        // `build_target_registry_counter_reset_obscures_earlier_sibling_scope_even_under_plural_read`).
        let mut doc = Document::new();
        let sec1 = doc.append_element(
            Some(0),
            "section",
            Style::default(),
            Some("counter-reset: chapter 1"),
        );
        let h2_1 = doc.append_element(Some(sec1), "h2", Style::default(), None::<&str>);
        set_id(&mut doc, h2_1, "ch1");

        let sec2 = doc.append_element(
            Some(0),
            "section",
            Style::default(),
            Some("counter-reset: chapter 5"),
        );
        let h2_2 = doc.append_element(Some(sec2), "h2", Style::default(), None::<&str>);
        set_id(&mut doc, h2_2, "ch2");

        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let mut registry = build_target_registry(&doc, &cr);
        let out1 = registry.resolve_target_counter(
            "#ch1",
            Symbol::new("chapter"),
            raikiri_style::property::CounterStyle::Decimal,
        );
        assert_eq!(
            out1,
            raikiri_traits::ResolveOutcome::Resolved("1".to_owned())
        );

        let out2 = registry.resolve_target_counter(
            "#ch2",
            Symbol::new("chapter"),
            raikiri_style::property::CounterStyle::Decimal,
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            out2,
            raikiri_traits::ResolveOutcome::Resolved("5".to_owned()),
            "second section's own counter-reset must obscure (evict), not nest inside, \
             sec1's still-open scope for the same name"
        );
    }

    #[test]
    fn build_target_registry_counter_reset_obscures_earlier_sibling_scope_even_under_plural_read() {
        // Companion to
        // build_target_registry_counter_scope_resets_between_independent_sections,
        // reading through `resolve_target_counters` (every stack level,
        // joined) instead of `resolve_target_counter` (innermost level
        // only). The two reads cannot be told apart by the singular query
        // above: whether sec2's own counter-reset evicted sec1's still-open
        // level (correct, per CSS Lists 3 §4.3's obscuring rule) or merely
        // nested a new level on top of it (incorrect self-nesting — §4.3's
        // self-nesting rule applies to an ancestor's inherited counter, not
        // a sibling's), `#ch2`'s innermost value is "5" either way. The
        // plural read is not: a wrongly-nested stack would join to "1.5",
        // not "5".
        let mut doc = Document::new();
        let sec1 = doc.append_element(
            Some(0),
            "section",
            Style::default(),
            Some("counter-reset: chapter 1"),
        );
        doc.append_element(Some(sec1), "h2", Style::default(), None::<&str>);

        let sec2 = doc.append_element(
            Some(0),
            "section",
            Style::default(),
            Some("counter-reset: chapter 5"),
        );
        let h2_2 = doc.append_element(Some(sec2), "h2", Style::default(), None::<&str>);
        set_id(&mut doc, h2_2, "ch2");

        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let mut registry = build_target_registry(&doc, &cr);
        let out = registry.resolve_target_counters(
            "#ch2",
            Symbol::new("chapter"),
            ".",
            raikiri_style::property::CounterStyle::Decimal,
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            out,
            raikiri_traits::ResolveOutcome::Resolved("5".to_owned()),
            "sec1's obscured level must not still be on the stack underneath sec2's own \
             (a wrongly-nested stack would join to \"1.5\", not \"5\")"
        );
    }

    #[test]
    fn counter_increment_without_ancestor_reset_persists_across_siblings() {
        // CSS Lists 3 §4.4.2 <https://www.w3.org/TR/css-lists-3/#instantiating-counters>:
        // a counter with no `counter-reset` anywhere is still instantiated
        // the first time something references it (here, p1's own
        // counter-increment) and that instantiation's scope covers p1's
        // following siblings too, same as an explicit counter-reset's scope
        // would (module doc "Counter-stack scope model") — so p2 must
        // continue p1's value (1, then 2), not restart its own independent
        // local auto-instantiation (which would read 1, then 1 again).
        let mut doc = Document::new();
        let p1 = doc.append_element(Some(0), "p", Style::default(), Some("counter-increment: x"));
        set_id(&mut doc, p1, "p1");
        let p2 = doc.append_element(Some(0), "p", Style::default(), Some("counter-increment: x"));
        set_id(&mut doc, p2, "p2");
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let mut registry = build_target_registry(&doc, &cr);
        let out1 = registry.resolve_target_counter(
            "#p1",
            Symbol::new("x"),
            raikiri_style::property::CounterStyle::Decimal,
        );
        let out2 = registry.resolve_target_counter(
            "#p2",
            Symbol::new("x"),
            raikiri_style::property::CounterStyle::Decimal,
        );
        assert_eq!(
            out1,
            raikiri_traits::ResolveOutcome::Resolved("1".to_owned())
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            out2,
            raikiri_traits::ResolveOutcome::Resolved("2".to_owned()),
            "p2 must continue p1's un-reset counter (CSS Lists 3 §4.4.2 \
             cross-sibling persistence)"
        );
    }

    #[test]
    fn counter_increment_on_following_sibling_of_reset_element_continues_that_scope() {
        // CSS Lists 3 §4.3 puts a counter-reset's scope over "the element's
        // descendants and its following siblings with their descendants" —
        // so a following sibling of the resetting <div> (both children of
        // the same implicit parent) must see and continue its scope: the
        // div's own push is popped at the shared parent's subtree exit, not
        // the div's own, so it is still open when the sibling <p> is
        // walked.
        let mut doc = Document::new();
        doc.append_element(Some(0), "div", Style::default(), Some("counter-reset: c 5"));
        let p = doc.append_element(Some(0), "p", Style::default(), Some("counter-increment: c"));
        set_id(&mut doc, p, "p");
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let mut registry = build_target_registry(&doc, &cr);
        let out = registry.resolve_target_counter(
            "#p",
            Symbol::new("c"),
            raikiri_style::property::CounterStyle::Decimal,
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            out,
            raikiri_traits::ResolveOutcome::Resolved("6".to_owned()),
            "p must see and continue the div's following-sibling-inclusive scope \
             (CSS Lists 3 §4.3)"
        );
    }

    #[test]
    fn build_target_registry_captures_own_descendant_text() {
        let mut doc = Document::new();
        let h1 = doc.append_element(Some(0), "h1", Style::default(), None::<&str>);
        set_id(&mut doc, h1, "title");
        doc.append_text(h1, "Chapter ");
        doc.append_text(h1, "One");
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let mut registry = build_target_registry(&doc, &cr);
        let out = registry.resolve_target_text("#title", ContentPart::Content);
        assert_eq!(
            out,
            raikiri_traits::ResolveOutcome::Resolved("Chapter One".to_owned())
        );
    }

    #[test]
    fn build_target_registry_skips_not_in_document_nodes() {
        // Same is_in_document() gate contract as
        // crate::running::build_running_template_store_and_collect_running_template_skip_not_in_document_nodes.
        // A comment node defaults IS_IN_DOCUMENT=true at construction but
        // mark_in_document_flags clears it; this walker's `Enter` step must
        // honor that (it has no `id` attribute concept anyway, but the gate
        // also protects text collection from wandering into stray subtrees).
        let mut doc = Document::new();
        let h1 = doc.append_element(Some(0), "h1", Style::default(), None::<&str>);
        set_id(&mut doc, h1, "title");
        doc.append_comment(Some(h1), "note");
        doc.append_text(h1, "Visible");
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let mut registry = build_target_registry(&doc, &cr);
        let out = registry.resolve_target_text("#title", ContentPart::Content);
        assert_eq!(
            out,
            raikiri_traits::ResolveOutcome::Resolved("Visible".to_owned())
        );
    }
}
