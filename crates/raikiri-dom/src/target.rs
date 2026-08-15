//! `target-*()` / `element()` register-site walker — cascade-time producer
//! that populates a dom-local [`raikiri_traits::TargetRegistry`].
//!
//! Companion to [`crate::running`]'s register-site
//! walker (already landed) — same document-order
//! arena-walk shape, different [`GcpmDirective`] variant. `running`'s
//! [`crate::running::collect_running_template`] already *emits*
//! `CounterIncrement`/`CounterReset`/`CounterSet`/`StringSet` (and
//! `RegisterRunning` is emitted by [`crate::running::build_running_template_store`]
//! itself) into each `position: running(name)` template's own
//! `directives: Vec<GcpmDirective>` — a dom-local Phase B walk (landed as
//! [`crate::gcpm`]) is what *applies* those emitted directives to
//! dom-local counter/string/running state, and (also landed)
//! `raikiri_traits::page::context::PageContext::apply_directive`
//! is the promoted, canonical counterpart. This module's `RegisterTarget`
//! variant has no emit-step counterpart in either of those (see "synthesized
//! here" below) — it *does* now have a real apply-step counterpart on the
//! promoted `PageContext` (its `RegisterTarget` arm registers a
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
//! `TargetRegistry` to its caller (test harness or a future per-document
//! driver), and that caller wires it into
//! `raikiri_traits::page::context::PageContext.targets` via
//! `PageContext::set_targets` (already landed) — a bulk replace, not a
//! `GcpmDirective`-mediated apply, since this walk never goes through the
//! directive stream in the first place (see "synthesized here" below). No
//! production per-document driver calls `set_targets` yet; that wiring is
//! part of a DOM-tree-walking driver that is still to be built.
//!
//! # `GcpmDirective::RegisterTarget` is synthesized here, not consumed
//!
//! Every other producing [`GcpmDirective`] variant
//! (`CounterIncrement`/`CounterReset`/`CounterSet`/`StringSet`) is a direct
//! mirror of a CSS property cascade already resolved onto
//! [`raikiri_style::ComputedValues`] (see [`crate::running::collect_running_template`]).
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
//! # Counter-stack scope model (deliberately narrowed — see divergence note)
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
//! following siblings with their descendants" (same section).
//!
//! [`CounterScopes`] implements an **ancestor-chain-only** subset of that
//! model: a scope established by an *ancestor* of the current element (or
//! by the current element itself) is visible, and correctly continues,
//! across every descendant walked while that ancestor's own subtree is
//! still open — including across multiple children under that ancestor
//! (siblings of each other that all share the scope-establishing ancestor).
//! `counter-reset` pushes a new stack level on entering an element and the
//! level is popped the moment the walker finishes that element's *own*
//! subtree. This correctly produces the "1", "1.1", "1.1.1" nesting for the
//! target-counters() use case this task exists for (parent → child nesting,
//! and mutation of `counter-increment`/`counter-set` within a scope that is
//! *still open* — i.e. an ancestor of the current element, or the current
//! element itself).
//!
//! **What's deferred — scope a *sibling itself* establishes, propagating to
//! that sibling's own following siblings.** §4.3's scope text quoted above
//! is explicit that a counter's scope "includes the element's descendants
//! **and its following siblings** with their descendants" — not just
//! descendants of a shared ancestor. This module does not implement that
//! specific case: because a `counter-reset`'s pushed level is popped at the
//! *resetting element's own* subtree exit (before any of *that element's
//! own* following siblings are visited), a scope a sibling itself created
//! is invisible to elements after it, even though the spec puts those later
//! siblings in scope too. Concretely, `<div style="counter-reset: c
//! 5"></div><p style="counter-increment: c" id="p">` (both children of the
//! same parent, with no counter-reset on that parent) should resolve `p`'s
//! `c` to `6` (still in the div's scope, per following-sibling inclusion);
//! this module resolves it to `1` (fresh local auto-instantiation, module
//! doc's [`CounterScopes::apply`] "no ancestor counter-reset scope" branch)
//! — pinned by `counter_increment_on_following_sibling_of_reset_element_is_not_in_scope`.
//! The ancestor-established case this module *does* model correctly (two
//! children of a common counter-reset-bearing parent, as opposed to two
//! top-level siblings where one of them is itself the resetting element) is
//! pinned by `counter_increment_continues_across_siblings_sharing_an_ancestor_scope`.
//! The same gap subsumes CSS Lists 3 §4.4.2 "Instantiating counters"
//! <https://www.w3.org/TR/css-lists-3/#auto-numbering>'s narrower case of a
//! counter with *no* `counter-reset` anywhere, which per spec is
//! instantiated once and *continues* across later, unrelated siblings
//! (pinned by
//! `counter_increment_without_ancestor_reset_does_not_persist_across_siblings`).
//! This is a fail-closed narrowing (原則 3), not a silent divergence — the
//! full cross-subtree accounting is `raikiri_traits::PageContext.counters`'s
//! job (design doc §7.2, `HashMap<Symbol, CounterStack>`; the `PageContext`
//! promotion itself has already landed) once a later Phase B walk actually
//! drives it — which in turn depends on the DOM-tree-walking driver still
//! being built.
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
//! leaving them absent here is spec-safe, not a stub bug.
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

/// Ancestor-chain-only counter-scope tracker — see module doc "Counter-stack
/// scope model" for exactly what this does and does not model.
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
    /// [`crate::running::collect_running_template`] already pins for the
    /// sibling `GcpmDirective`-emit walk, and for the same reason: §4.2's
    /// note that `counter-set` is applied after `counter-increment`).
    ///
    /// Returns the counter names for which *this call* pushed a brand-new
    /// stack level, so the caller can pop exactly those when this element's
    /// subtree walk finishes ([`Self::pop`]) — see module doc.
    ///
    /// A duplicate `<counter-name>` within `cv.counter_reset` itself (e.g.
    /// `counter-reset: foo 1 foo 2`) is deduplicated to the *last*
    /// occurrence's value before pushing — CSS Lists 3 §4.1, see the
    /// dedup step's own comment below.
    fn apply(&mut self, cv: &raikiri_style::ComputedValues) -> Vec<Symbol> {
        let mut pushed = Vec::new();

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
            // counter-reset always instantiates a *new* nested scope level
            // (CSS Lists 3 §4.3 "self-nesting", module doc), regardless of
            // whether an ancestor scope for `name` already exists.
            self.stacks.entry(sym.clone()).or_default().push(value);
            pushed.push(sym);
        }
        for (name, delta) in cv.counter_increment.iter() {
            let sym = Symbol::new(name.clone());
            let stack = self.stacks.entry(sym.clone()).or_default();
            match stack.last_mut() {
                // Saturating, not wrapping/panicking, on overflow — the
                // same fix as
                // `raikiri_traits::page::context::CounterStack::increment`
                // and `crate::gcpm::CounterStack::increment` — this walker's
                // top-of-stack increment shared the exact same unbounded
                // `+=` bug. `counter-reset: c 2147483647; counter-increment:
                // c 1` is spec-legal CSS and would panic (debug) or
                // silently wrap to `i32::MIN` (release) on plain `+=`.
                Some(top) => *top = top.saturating_add(*delta),
                None => {
                    // No ancestor counter-reset scope for `name` — local
                    // auto-instantiation (module doc's divergence note
                    // covers how this differs from §4.4.2's cross-sibling
                    // continuation).
                    stack.push(*delta);
                    pushed.push(sym);
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
                    pushed.push(sym);
                }
            }
        }

        pushed
    }

    /// Pop exactly the scope levels a prior [`Self::apply`] call pushed
    /// (invoked when the walker finishes that element's subtree — see
    /// [`build_target_registry`]'s `Exit` step).
    fn pop(&mut self, names: &[Symbol]) {
        for name in names {
            if let Some(stack) = self.stacks.get_mut(name) {
                stack.pop();
                if stack.is_empty() {
                    self.stacks.remove(name);
                }
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
/// `element_id_and_attr_treat_empty_value_as_none` pins the same contract at
/// the trait-impl layer) instead of re-deriving it here.
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
/// [`crate::running::collect_running_template`] (and, one level up,
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
/// extended with an `Exit` step so [`CounterScopes`] can pop exactly the
/// scope levels each element pushed once its subtree is fully walked). For
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
/// Elements with `display: none` never run their counter directives (CSS
/// Lists 3 §4.5, see the `Enter` step's own comment below) — but they can
/// still register as a target, since `target-*()` addresses fragments by
/// `id`, not by box generation.
///
/// # Precondition
///
/// Same as [`crate::running::build_running_template_store`]: `doc`'s
/// `IS_IN_DOCUMENT` flags must be up to date (see
/// [`Document::mark_in_document_flags`]), matching
/// [`raikiri_style::cascade`]'s own precondition, since this walker is meant
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
        Exit(Vec<Symbol>),
    }

    let mut stack = vec![WalkStep::Enter(doc.root)];
    while let Some(step) = stack.pop() {
        match step {
            WalkStep::Enter(idx) => {
                let node = &doc.nodes[idx];
                if !node.is_in_document() {
                    continue;
                }

                let pushed = match cascade.computed.get(idx) {
                    Some(cv) if cv.display == DisplayValue::None => {
                        // CSS Lists 3 §4.5
                        // <https://www.w3.org/TR/css-lists-3/#counters-in-elements-that-do-not-generate-boxes>:
                        // an element that does not generate a box "cannot
                        // set, reset, or increment a counter ... they must
                        // have no effect." Skip directive application
                        // entirely — the element may still register as a
                        // target below (target-* doesn't require box
                        // generation, only an id).
                        Vec::new()
                    }
                    Some(cv) => scopes.apply(cv),
                    // cov:ignore: `raikiri_style::cascade`'s own contract
                    // (`computed.len() == doc.node_count()`) guarantees
                    // `Some` for every valid arena index when `cascade` was
                    // produced from this `doc` — same defensive shape
                    // `crate::running::collect_running_template`'s matching
                    // `.get(idx)` note documents.
                    None => Vec::new(),
                };
                // Push the Exit step before the id/register check below —
                // even an id-less element's pushed scopes (e.g. from a bare
                // counter-reset with no id) must still be popped on the way
                // back out.
                stack.push(WalkStep::Exit(pushed));

                if let Some(fragment_id) = element_id(doc, idx) {
                    let directive = synthesize_register_target(&fragment_id);
                    let info = build_target_info(doc, idx, &scopes);
                    apply_register_target(directive, info, &mut registry);
                }

                for &child in node.children.iter().rev() {
                    stack.push(WalkStep::Enter(child));
                }
            }
            WalkStep::Exit(pushed_names) => {
                scopes.pop(&pushed_names);
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
        // element_id_and_attr_treat_empty_value_as_none
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
        // required to pin this.
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
        // Regression pin: spec-legal CSS driving
        // counter-increment past i32::MAX must saturate, not panic (debug
        // builds) or wrap to i32::MIN (release builds). Same finding, same
        // fix, as `CounterStack::increment` in `raikiri-traits`'s
        // `page::context` and `raikiri-dom`'s `gcpm` — this walker's
        // top-of-stack increment shared the exact same unbounded `+=` bug.
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
    fn build_target_registry_display_none_element_does_not_affect_counter_stack() {
        // CSS Lists 3 §4.5
        // <https://www.w3.org/TR/css-lists-3/#counters-in-elements-that-do-not-generate-boxes>:
        // an element that does not generate a box "cannot set, reset, or
        // increment a counter ... they must have no effect."
        //
        // Regression-pin shape: the id-bearing probe must be a *descendant*
        // of the display:none element, registered *before* that element's
        // own subtree-exit pop — a sibling-after probe would read "0"
        // either way (the ancestor-chain-only pop-on-subtree-exit model
        // already discards the reset's scope by the time a later sibling is
        // visited, fix or no fix — see
        // counter_increment_on_following_sibling_of_reset_element_is_not_in_scope),
        // so it can't distinguish "correctly skipped" from "wrongly applied
        // then popped". A descendant, seen *while the scope is still open*,
        // can.
        let mut doc = Document::new();
        let hidden = doc.append_element(
            Some(0),
            "div",
            Style::default(),
            Some("display: none; counter-reset: c 5"),
        );
        let target = doc.append_element(Some(hidden), "span", Style::default(), None::<&str>);
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
        assert_eq!(
            out,
            raikiri_traits::ResolveOutcome::Resolved("0".to_owned()),
            "a display:none ancestor's own counter-reset must have no effect at all, \
             even observed from inside its still-open (but skipped) scope"
        );
    }

    #[test]
    fn build_target_registry_display_none_element_itself_still_registers_as_target() {
        // Companion to build_target_registry_display_none_element_does_not_affect_counter_stack:
        // that test pins that a display:none element's counter directives
        // have no effect on OTHERS; this one pins that skipping directive
        // application doesn't also accidentally skip TARGET registration
        // for the display:none element itself. CSS Lists 3 §4.5 only
        // withdraws counter set/reset/increment from non-box-generating
        // elements — it says nothing about target-*() addressability, which
        // is purely id-based (see build_target_registry's own "Elements
        // with display: none" doc note).
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
    fn counter_increment_continues_across_siblings_sharing_an_ancestor_scope() {
        // The case the ancestor-chain-only model correctly handles (module
        // doc "Counter-stack scope model"): two <p> children of a common
        // <section> that itself carries counter-reset both see, and
        // continue, that still-open ancestor scope — contrast with
        // counter_increment_on_following_sibling_of_reset_element_is_not_in_scope,
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
    fn build_target_registry_counter_scope_resets_between_independent_sections() {
        // Two independent top-level <section>s, each with their own
        // counter-reset: chapter — the second section's <h2 id> must see a
        // FRESH scope (not the first section's leftover value), pinning the
        // push/pop-on-subtree-exit behavior module doc describes as
        // correctly handled (as opposed to the no-ancestor-reset case, which
        // is the documented divergence).
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
        assert_eq!(
            out2,
            raikiri_traits::ResolveOutcome::Resolved("5".to_owned()),
            "second section's own counter-reset must win, not leak sec1's popped scope"
        );
    }

    #[test]
    fn counter_increment_without_ancestor_reset_does_not_persist_across_siblings() {
        // Documented divergence pin (module doc "What's deferred"): per CSS
        // Lists 3 §4.4.2, two unrelated elements incrementing a
        // never-explicitly-reset counter should see a CONTINUING value (1,
        // then 2) — this walker's ancestor-chain-only model instead resets
        // per independent local auto-instantiation (1, then 1 again). This
        // test pins the CURRENT (narrowed) behavior so a silent behavior
        // change is caught, not to assert it's spec-correct.
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
        assert_eq!(
            out2,
            raikiri_traits::ResolveOutcome::Resolved("1".to_owned()),
            "documented narrowing: p2 does NOT continue p1's un-reset counter \
             (true spec behavior would be \"2\" — see module doc)"
        );
    }

    #[test]
    fn counter_increment_on_following_sibling_of_reset_element_is_not_in_scope() {
        // Documented divergence pin (module doc "What's deferred"): CSS
        // Lists 3 §4.3 puts a counter-reset's scope over "the element's
        // descendants and its following siblings with their descendants" —
        // so a following sibling of the resetting <div> should see and
        // continue its scope (spec-correct answer: "6"). This walker's
        // ancestor-chain-only model pops the div's scope at its own subtree
        // exit, so the sibling <p> never observes it and instead gets a
        // fresh local auto-instantiation. Pins the CURRENT (narrowed)
        // behavior, not spec correctness.
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
        assert_eq!(
            out,
            raikiri_traits::ResolveOutcome::Resolved("1".to_owned()),
            "documented narrowing: p does NOT see the div's following-sibling-\
             inclusive scope (true spec behavior would be \"6\" — see module doc)"
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
