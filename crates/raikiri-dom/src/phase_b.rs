//! Phase B main-document driver — the DOM-tree/per-page walking driver that
//! consumes raikiri-traits's promoted `PageContext`
//! (`raikiri_traits::page::context::PageContext`) rather than this crate's
//! own dom-local `crate::gcpm` mirror (see that module's doc for why the
//! mirror still exists dom-locally rather than being collapsed onto this
//! driver yet).
//!
//! # Per-document setup, then one [`drive_page`] call per page
//!
//! [`raikiri_traits::TargetRegistry`] is *document*-scoped, not page-scoped
//! — [`PageContext::begin_page`]'s own doc states its `resolved` /
//! `pending_slots` maps "persist across pages too" and that "a single
//! `PageContext` instance must live for the whole document". Building and
//! wiring it is therefore a one-time, per-document step, kept separate from
//! the per-page advance:
//!
//! 1. [`drive_document`] builds the whole document's
//!    [`raikiri_traits::TargetRegistry`]
//!    ([`crate::target::build_target_registry`]) and wires it in exactly
//!    once, via [`PageContext::set_targets`], *before* the first
//!    [`drive_page`] call. `set_targets`'s wholesale-replace semantics
//!    (see [`PageContext::begin_page`]'s own doc "Residual gap this method
//!    does not close") is exactly why this must happen only once per
//!    document: a second `set_targets` call mid-document would silently
//!    discard any `pending_slots` a caller had queued while processing an
//!    earlier page — [`drive_page`] itself never calls `set_targets`, so
//!    calling it more than once per document is a caller error, not
//!    something this driver can do for you a second time safely.
//! 2. [`drive_page`] is then called once per page. Each call advances
//!    `page_index`, every tracked `NamedStringState`, and the *already-
//!    wired* registry's own page-local sequence numbering — all three
//!    together, via [`PageContext::begin_page`] — then walks `page_root`'s
//!    subtree in document order
//!    ([`crate::running::derive_element_directives`] per element, in CSS
//!    Lists 3 §4 order), applying every derived directive via
//!    [`PageContext::apply_directive`]. It does **not** touch `targets`
//!    itself; calling it before `ctx.targets` has been wired via
//!    `set_targets` leaves target resolution running against an empty,
//!    default `TargetRegistry` for the whole document.
//!
//! # Counter-scope exit (CSS Lists 3 §4.3) is not wired yet
//!
//! CSS Lists 3 §4.3 <https://www.w3.org/TR/css-lists-3/#auto-numbering>
//! scopes a `counter-reset` to "the element's descendants and its following
//! siblings with their descendants". Modeling that correctly means popping
//! the pushed nested-scope frame at the *resetting element's own parent's*
//! subtree exit, not at the resetting element's own exit — popping at the
//! element's own exit would only keep the scope visible to its descendants,
//! losing the following-sibling half CSS Lists 3 §4.3 grants it (the same
//! narrowing `crate::target`'s `CounterScopes` documents as a known,
//! separately-tracked gap in its own ancestor-chain-only model).
//!
//! [`raikiri_traits::PageContext`] has no way to reach into a
//! tracked counter stack and pop it from outside raikiri-traits:
//! `PageContext::counter` is read-only, and the only mutation path is
//! `apply_directive`'s exhaustive match over the existing `GcpmDirective`
//! variants, none of which pop a scope. Extending that surface is a
//! raikiri-traits public-API decision, not something this driver can settle
//! on its own — until it lands, every `CounterReset` this walker applies
//! accumulates depth monotonically rather than resetting at the correct
//! same-parent scope, the same documented limitation
//! [`crate::gcpm::CounterStack`]'s own doc already describes for its
//! dom-local mirror. [`walk_directives`] still tracks Enter/Exit
//! structurally (mirroring [`crate::target::build_target_registry`]'s own
//! shape) so wiring the pop, once a mutation path exists, is a small
//! addition here rather than a rewrite — the `Exit` step currently does
//! nothing.

use raikiri_style::CascadeResult;
use raikiri_style::property::DisplayValue;
use raikiri_traits::PageContext;

use crate::document::Document;
use crate::running::derive_element_directives;
use crate::target::build_target_registry;

/// Advance `ctx` to `page_index` and walk `page_root`'s subtree in document
/// order, applying every element's derived `GcpmDirective`s.
///
/// **Precondition: `ctx.targets` must already be wired** via
/// [`PageContext::set_targets`] — this function never builds or replaces
/// `targets` itself. See the module-level doc "Per-document setup, then one
/// `drive_page` call per page" for why that wiring is a one-time,
/// per-document step this function deliberately stays out of, and
/// [`drive_document`] for the sanctioned way to satisfy this precondition
/// for a whole document in one call.
#[allow(
    dead_code,
    reason = "No production per-document caller wires this into a real \
              multi-page pagination flow yet — that fragmentation (deciding \
              which DOM content belongs to which page) is a future \
              PageStream concern (design §7.3/§9.1) this driver \
              deliberately does not implement. Exercised via \
              drive_document and this module's own tests until a \
              real per-page caller exists."
)]
pub(crate) fn drive_page(
    ctx: &mut PageContext,
    doc: &Document,
    cascade: &CascadeResult,
    page_index: u32,
    page_root: usize,
) {
    ctx.begin_page(page_index);
    walk_directives(ctx, doc, cascade, page_root);
}

/// Build and wire the whole document's [`raikiri_traits::TargetRegistry`],
/// then drive it as a single page (`page_index = 0`). This is the sole
/// sanctioned one-shot entry point for a single-page document; a real
/// multi-page caller should replicate steps 1 (build + [`PageContext::set_targets`],
/// once) and 2 ([`drive_page`], once per page) of the module-level doc
/// itself, rather than calling this function per page — calling it more
/// than once per document would re-wire `targets` and silently discard any
/// `pending_slots` queued in between, exactly the hazard the module doc's
/// "one-time, per-document step" framing exists to rule out.
#[allow(
    dead_code,
    reason = "No production per-document driver calls this yet — the \
              caller that would (a single-page render path, or a future \
              PageStream for the multi-page case) has not landed. \
              Exercised via this module's own tests."
)]
pub(crate) fn drive_document(ctx: &mut PageContext, doc: &Document, cascade: &CascadeResult) {
    ctx.set_targets(build_target_registry(doc, cascade));
    drive_page(ctx, doc, cascade, 0, doc.root);
}

/// Document-order walk applying every in-document element's derived
/// directives to `ctx`. Same iterative reverse-push-children DFS shape as
/// [`crate::target::build_target_registry`], extended with an `Exit` step —
/// currently a no-op (see module doc "Counter-scope exit is not wired yet"),
/// kept structurally in place for that future wiring.
fn walk_directives(ctx: &mut PageContext, doc: &Document, cascade: &CascadeResult, root: usize) {
    enum WalkStep {
        Enter(usize),
        Exit,
    }

    let mut stack = vec![WalkStep::Enter(root)];
    let mut directives = Vec::new();
    while let Some(step) = stack.pop() {
        match step {
            WalkStep::Enter(idx) => {
                let node = &doc.nodes[idx];
                if !node.is_in_document() {
                    continue;
                }

                match cascade.computed.get(idx) {
                    // CSS Lists 3 §4.5
                    // <https://www.w3.org/TR/css-lists-3/#counters-in-elements-that-do-not-generate-boxes>:
                    // an element that does not generate a box "cannot set,
                    // reset, or increment a counter ... they must have no
                    // effect." Same skip
                    // crate::target::build_target_registry's own `Enter`
                    // step already applies (see that function's doc).
                    Some(cv) if cv.display == DisplayValue::None => {}
                    Some(cv) => {
                        directives.clear();
                        derive_element_directives(doc, idx, cv, &mut directives);
                        for directive in &directives {
                            ctx.apply_directive(directive);
                        }
                    }
                    // cov:ignore: `raikiri_style::cascade`'s own contract
                    // (`computed.len() == doc.node_count()`) guarantees
                    // `Some` for every valid arena index when `cascade` was
                    // produced from this `doc` — same defensive shape
                    // crate::target::build_target_registry's matching
                    // `.get(idx)` note documents.
                    None => {}
                }

                stack.push(WalkStep::Exit);
                for &child in node.children.iter().rev() {
                    stack.push(WalkStep::Enter(child));
                }
            }
            WalkStep::Exit => {
                // Parent-exit-pop for CSS Lists 3 §4.3 counter scoping is
                // not wired yet — see module doc "Counter-scope exit is not
                // wired yet".
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use raikiri_style::property::CounterStyle;
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::{ResolveOutcome, Symbol};
    use smol_str::SmolStr;
    use taffy::Style;

    fn set_id(doc: &mut Document, idx: usize, id: &str) {
        doc.set_element_attributes(idx, vec![(SmolStr::new("id"), SmolStr::new(id))]);
    }

    #[test]
    fn drive_document_applies_counter_directives_in_document_order() {
        let mut doc = Document::new();
        doc.append_element(
            Some(0),
            "h2",
            Style::default(),
            Some("counter-reset: c 1; counter-increment: c 1"),
        );
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let mut ctx = PageContext::default();
        drive_document(&mut ctx, &doc, &cr);

        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            ctx.counter(&Symbol::new("c")).and_then(|c| c.current()),
            Some(2),
            "reset(1) then increment(1) on the same element must yield 2"
        );
    }

    #[test]
    fn drive_document_applies_string_set_directives() {
        let mut doc = Document::new();
        doc.append_element(
            Some(0),
            "h1",
            Style::default(),
            Some("string-set: title \"Chapter\""),
        );
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let mut ctx = PageContext::default();
        drive_document(&mut ctx, &doc, &cr);

        assert_eq!(
            ctx.string_state(&Symbol::new("title"))
                .and_then(|s| s.running()),
            Some("Chapter")
        );
    }

    #[test]
    fn drive_document_walks_in_document_order_not_arena_insertion_order() {
        // Document order must hold both across depth (parent before
        // children) and across siblings (first child before second child).
        // Two siblings doing the same commutative +1 increment couldn't
        // distinguish a sibling-order regression (the sum is order-
        // independent) — this uses a non-commutative pair instead
        // (increment then set) specifically so a reversed sibling order
        // produces a different final value: correct order yields
        // increment(1) then set(99) = 99; a reversed walk would yield
        // set(99) then increment(1) = 100 instead.
        let mut doc = Document::new();
        let parent = doc.append_element(
            Some(0),
            "section",
            Style::default(),
            Some("counter-reset: item 0"),
        );
        doc.append_element(
            Some(parent),
            "p",
            Style::default(),
            Some("counter-increment: item 1"),
        );
        doc.append_element(
            Some(parent),
            "p",
            Style::default(),
            Some("counter-set: item 99"),
        );
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let mut ctx = PageContext::default();
        drive_document(&mut ctx, &doc, &cr);

        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            ctx.counter(&Symbol::new("item")).and_then(|c| c.current()),
            Some(99),
            "the second sibling's counter-set must apply AFTER the first \
             sibling's counter-increment (document order) — a reversed \
             sibling walk would yield 100 instead"
        );
    }

    #[test]
    fn drive_document_skips_directives_on_display_none_elements() {
        let mut doc = Document::new();
        doc.append_element(
            Some(0),
            "div",
            Style::default(),
            Some("display: none; counter-reset: c 5"),
        );
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let mut ctx = PageContext::default();
        drive_document(&mut ctx, &doc, &cr);

        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            ctx.counter(&Symbol::new("c")),
            None,
            "a display:none element's counter-reset must have no effect \
             (CSS Lists 3 §4.5)"
        );
    }

    #[test]
    fn drive_document_wires_the_target_registry_before_walking() {
        let mut doc = Document::new();
        let h1 = doc.append_element(Some(0), "h1", Style::default(), None::<&str>);
        set_id(&mut doc, h1, "intro");
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let mut ctx = PageContext::default();
        drive_document(&mut ctx, &doc, &cr);

        let mut registry = ctx.targets().clone();
        let out = registry.resolve_target_counter(
            "#intro",
            Symbol::new("nonexistent"),
            CounterStyle::Decimal,
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            out,
            ResolveOutcome::Resolved("0".to_owned()),
            "the id-bearing element registered by build_target_registry must \
             already be wired into PageContext.targets after drive_document"
        );
    }

    #[test]
    fn drive_page_stamps_the_already_wired_registry_to_the_given_page_index() {
        let mut doc = Document::new();
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let mut ctx = PageContext::default();
        // Pre-wire, as drive_document would before the first drive_page
        // call — drive_page itself never builds or wires targets (see its
        // own doc's "Precondition").
        ctx.set_targets(build_target_registry(&doc, &cr));
        drive_page(&mut ctx, &doc, &cr, 3, doc.root);

        assert_eq!(ctx.page_index, 3);

        let mut registry = ctx.targets().clone();
        let out = registry.resolve_target_counter(
            "#not-yet-registered",
            Symbol::new("c"),
            CounterStyle::Decimal,
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            out,
            ResolveOutcome::Pending(raikiri_traits::TargetSlotId {
                page_index: 3,
                sequence: 0,
            }),
            "a slot minted after drive_page(.., 3, ..) must be stamped with \
             page_index 3, proving PageContext::begin_page's internal \
             TargetRegistry::begin_page forwarding ran"
        );
    }

    #[test]
    fn drive_page_does_not_rewire_targets_on_a_second_call() {
        // The bug this pins: an earlier version of drive_page rebuilt and
        // wholesale-replaced ctx.targets on *every* call (not just the
        // first), which silently discarded whatever the document's initial
        // wiring had registered the moment a second page was driven —
        // exactly the "targets persist across pages" contract violation
        // PageContext::begin_page's own doc warns about. Proven here by
        // driving page 1 with a *different* doc/cascade pair (one with no
        // id-bearing elements at all) than the one drive_document wired
        // targets from for page 0 — if drive_page still rebuilt targets
        // internally, it would rebuild from this second, id-less doc and
        // "#intro" would stop resolving.
        let mut doc0 = Document::new();
        let h1 = doc0.append_element(Some(0), "h1", Style::default(), None::<&str>);
        set_id(&mut doc0, h1, "intro");
        doc0.mark_in_document_flags();
        let rules0 = build_rule_tree(&doc0);
        let cr0 = cascade(&doc0, &rules0).expect("cascade Ok");

        let mut ctx = PageContext::default();
        drive_document(&mut ctx, &doc0, &cr0);

        let mut empty_doc = Document::new();
        empty_doc.mark_in_document_flags();
        let empty_rules = build_rule_tree(&empty_doc);
        let empty_cr = cascade(&empty_doc, &empty_rules).expect("cascade Ok");
        drive_page(&mut ctx, &empty_doc, &empty_cr, 1, empty_doc.root);

        let mut registry = ctx.targets().clone();
        let out = registry.resolve_target_counter(
            "#intro",
            Symbol::new("nonexistent"),
            CounterStyle::Decimal,
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            out,
            ResolveOutcome::Resolved("0".to_owned()),
            "\"#intro\", registered by drive_document's initial wiring from \
             doc0, must still resolve after a second drive_page call driven \
             by an unrelated, id-less doc/cascade pair — proving drive_page \
             did not rebuild ctx.targets from that second call's own doc"
        );
    }

    #[test]
    fn drive_document_skips_directives_inside_a_template_subtree() {
        // walk_directives's `if !node.is_in_document() { continue; }` check
        // must actually skip a subtree that's reachable via `node.children`
        // but flagged out of the flat tree by `mark_in_document_flags` — the
        // `<template>` case is the one shape of this in an ordinary parsed
        // document (the `<template>` element itself stays in_document, but
        // everything inside it gets cleared). A node that's merely absent
        // from the tree (e.g. built with `parent: None`) would never reach
        // this check at all, since the walk wouldn't descend into it in the
        // first place — this test needs the reachable-but-flagged-out case
        // specifically.
        let mut doc = Document::new();
        let tmpl = doc.append_element(Some(0), "template", Style::default(), None::<&str>);
        doc.append_element(
            Some(tmpl),
            "p",
            Style::default(),
            Some("counter-reset: hidden 9"),
        );
        doc.append_element(
            Some(0),
            "p",
            Style::default(),
            Some("counter-reset: visible 1"),
        );
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let mut ctx = PageContext::default();
        drive_document(&mut ctx, &doc, &cr);

        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            ctx.counter(&Symbol::new("hidden")),
            None,
            "a counter-reset inside a <template> subtree must have no \
             effect — the subtree is reachable via node.children but \
             cleared by mark_in_document_flags"
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            ctx.counter(&Symbol::new("visible"))
                .and_then(|c| c.current()),
            Some(1),
            "a sibling counter-reset outside the <template> must still \
             apply normally"
        );
    }

    #[test]
    fn drive_page_across_two_pages_carries_named_string_state_forward() {
        let mut doc = Document::new();
        doc.append_element(
            Some(0),
            "h1",
            Style::default(),
            Some("string-set: title \"Ch. 1\""),
        );
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let mut ctx = PageContext::default();
        // Pre-wire, satisfying drive_page's own documented precondition
        // (this is what drive_document does internally before its first
        // drive_page call).
        ctx.set_targets(build_target_registry(&doc, &cr));
        drive_page(&mut ctx, &doc, &cr, 0, doc.root);

        // Second page has no string-set of its own — begin_page's carry-
        // forward (on_page_start) must reflect page 0's value.
        let mut empty_doc = Document::new();
        empty_doc.mark_in_document_flags();
        let empty_rules = build_rule_tree(&empty_doc);
        let empty_cr = cascade(&empty_doc, &empty_rules).expect("cascade Ok");
        drive_page(&mut ctx, &empty_doc, &empty_cr, 1, empty_doc.root);

        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            ctx.string_state(&Symbol::new("title"))
                .and_then(|s| s.on_page_start()),
            Some("Ch. 1"),
            "page 1's on_page_start must carry forward page 0's running value"
        );
    }
}
