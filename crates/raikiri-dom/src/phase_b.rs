//! Phase B main-document driver — the DOM-tree/per-page walking driver that
//! consumes raikiri-traits's promoted `PageContext`
//! (`raikiri_traits::page::context::PageContext`) rather than this crate's
//! own dom-local `crate::gcpm` mirror (see that module's doc for why the
//! mirror still exists dom-locally rather than being collapsed onto this
//! driver yet).
//!
//! # Per-page call order
//!
//! [`drive_page`] advances `PageContext` to a page in this order:
//!
//! 1. Build this page's [`raikiri_traits::TargetRegistry`]
//!    ([`crate::target::build_target_registry`]) and stamp it to
//!    `page_index` via [`raikiri_traits::TargetRegistry::begin_page`] —
//!    *before* wiring it in. A freshly built registry starts at page index
//!    0 / sequence 0; stamping it first means the registry [`PageContext`]
//!    ends up holding already carries the right page-local sequence
//!    numbering, rather than being wired in stale and needing a second,
//!    redundant transition.
//! 2. Wire it in ([`PageContext::set_targets`]) — before any
//!    `RegisterTarget`-directive-driven [`PageContext::apply_directive`]
//!    call for this page, per `apply_directive`'s own `RegisterTarget` arm
//!    doc (`set_targets`'s wholesale-replace semantics would otherwise
//!    clobber richer registrations already applied).
//! 3. Advance `page_index` and every tracked `NamedStringState`
//!    ([`PageContext::begin_page`]). The registry's own
//!    `TargetRegistry::begin_page(page_index)` call this method makes
//!    internally is a same-index no-op at this point — step 1 already
//!    stamped the registry now wired in — so this step's only *new* effect
//!    here is the `page_index` field write and the named-string page-boundary
//!    snapshot. Calling `begin_page` *before* `set_targets` instead would
//!    advance the *previous* page's (about-to-be-discarded) registry and
//!    leave the freshly wired-in one un-stamped — the exact desync
//!    `PageContext::begin_page`'s own doc "Residual gap this method does not
//!    close" warns about.
//! 4. Walk `page_root`'s subtree in document order
//!    ([`crate::running::derive_element_directives`] per element, in CSS
//!    Lists 3 §4 order), applying every derived directive via
//!    [`PageContext::apply_directive`].
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
//! [`raikiri_traits::page::context::PageContext`] has no way to reach into a
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
/// order, applying every element's derived `GcpmDirective`s. See the
/// module-level doc "Per-page call order" for why the 4 steps below run in
/// exactly this sequence.
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
    let mut registry = build_target_registry(doc, cascade);
    registry.begin_page(page_index);
    ctx.set_targets(registry);
    ctx.begin_page(page_index);
    walk_directives(ctx, doc, cascade, page_root);
}

/// Convenience wrapper treating the whole document as a single page
/// (`page_index = 0`). Real multi-page fragmentation is a future PageStream
/// concern this driver does not implement (see [`drive_page`]'s doc) —
/// callers with a real per-page content split should call [`drive_page`]
/// once per page instead.
#[allow(
    dead_code,
    reason = "No production per-document driver calls this yet — the \
              caller that would (a single-page render path, or a future \
              PageStream for the multi-page case) has not landed. \
              Exercised via this module's own tests."
)]
pub(crate) fn drive_document(ctx: &mut PageContext, doc: &Document, cascade: &CascadeResult) {
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
        // A later sibling's counter-increment must observe an earlier
        // sibling's counter-reset — this only holds if the walk visits
        // elements in document order (reset before increment), not
        // insertion order (which happens to coincide with document order
        // for this simple tree, but the reverse-push-children Exit-tracking
        // walk is what's actually under test — a regression here would be
        // an unstable/wrong traversal order, e.g. children before parent).
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
            Some("counter-increment: item 1"),
        );
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let mut ctx = PageContext::default();
        drive_document(&mut ctx, &doc, &cr);

        assert_eq!(
            ctx.counter(&Symbol::new("item")).and_then(|c| c.current()),
            Some(2),
            "both increments must observe the reset that precedes them in \
             document order"
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

        assert_eq!(
            ctx.counter(&Symbol::new("c")),
            None,
            "a display:none element's counter-reset must have no effect \
             (CSS Lists 3 §4.5)"
        );
    }

    #[test]
    fn drive_page_wires_the_page_scoped_target_registry_before_walking() {
        let mut doc = Document::new();
        let h1 = doc.append_element(Some(0), "h1", Style::default(), None::<&str>);
        set_id(&mut doc, h1, "intro");
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let mut ctx = PageContext::default();
        drive_page(&mut ctx, &doc, &cr, 0, doc.root);

        let mut registry = ctx.targets().clone();
        let out = registry.resolve_target_counter(
            "#intro",
            Symbol::new("nonexistent"),
            CounterStyle::Decimal,
        );
        assert_eq!(
            out,
            ResolveOutcome::Resolved("0".to_owned()),
            "the id-bearing element registered by build_target_registry must \
             already be wired into PageContext.targets after drive_page"
        );
    }

    #[test]
    fn drive_page_stamps_the_registry_to_the_given_page_index_before_wiring() {
        let mut doc = Document::new();
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let mut ctx = PageContext::default();
        drive_page(&mut ctx, &doc, &cr, 3, doc.root);

        assert_eq!(ctx.page_index, 3);

        let mut registry = ctx.targets().clone();
        let out = registry.resolve_target_counter(
            "#not-yet-registered",
            Symbol::new("c"),
            CounterStyle::Decimal,
        );
        assert_eq!(
            out,
            ResolveOutcome::Pending(raikiri_traits::TargetSlotId {
                page_index: 3,
                sequence: 0,
            }),
            "a slot minted after drive_page(.., 3, ..) must be stamped with \
             page_index 3, proving the registry was stamped before being \
             wired in rather than left at its freshly-built default of 0"
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
        drive_page(&mut ctx, &doc, &cr, 0, doc.root);

        // Second page has no string-set of its own — begin_page's carry-
        // forward (on_page_start) must reflect page 0's value.
        let mut empty_doc = Document::new();
        empty_doc.mark_in_document_flags();
        let empty_rules = build_rule_tree(&empty_doc);
        let empty_cr = cascade(&empty_doc, &empty_rules).expect("cascade Ok");
        drive_page(&mut ctx, &empty_doc, &empty_cr, 1, empty_doc.root);

        assert_eq!(
            ctx.string_state(&Symbol::new("title"))
                .and_then(|s| s.on_page_start()),
            Some("Ch. 1"),
            "page 1's on_page_start must carry forward page 0's running value"
        );
    }
}
