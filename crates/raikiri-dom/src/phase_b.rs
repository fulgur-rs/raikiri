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
//! # Counter-scope exit (CSS Lists 3 §4.3, §4.4.2)
//!
//! CSS Lists 3 §4.3 <https://www.w3.org/TR/css-lists-3/#auto-numbering>
//! scopes a counter's nested-scope frame to "the element's descendants and
//! its following siblings with their descendants". [`walk_directives`]
//! models this by popping the pushed frame at the *instantiating element's
//! own parent's* subtree exit, not at the instantiating element's own exit
//! — popping at the element's own exit would only keep the scope visible to
//! its descendants, losing the following-sibling half CSS Lists 3 §4.3
//! grants it (the same narrowing `crate::target`'s `CounterScopes`
//! documents as a known, separately-tracked gap in its own
//! ancestor-chain-only model — that dom-local type is a different code path
//! from this driver and is not affected by this section).
//!
//! An element can instantiate a counter's frame two ways: `counter-reset`
//! always pushes a fresh frame unconditionally (CSS Lists 3 §4.1), while
//! `counter-increment` and `counter-set` only push one when the counter has
//! no active frame yet — CSS Lists 3 §4.4.2
//! <https://www.w3.org/TR/css-lists-3/#instantiating-counters> ("also when
//! not otherwise present if named in counter-increment, counter-set, or the
//! counter() or counters() notations") — otherwise they mutate the innermost
//! existing frame in place (`CounterStack::increment` / `CounterStack::set`).
//! Only the frame-creating case needs a scheduled pop; mutating an inherited
//! frame is not this element's scope to close. The quoted rule's third
//! trigger — a `counter()`/`counters()` *read* of an absent counter also
//! instantiating it — is not modeled by this mechanism at all: the private
//! `resolve_content_source` helper behind
//! [`PageContext::apply_directive`]'s `StringSet` arm (in `raikiri-traits`,
//! not this crate) already defaults an absent counter to `0` directly,
//! without emitting any `GcpmDirective` of its own, so there is no
//! directive here for `walk_directives` to intercept and no frame ever gets
//! pushed for that case.
//!
//! The mechanism is a `pending_pops: Vec<Vec<Symbol>>` side-stack,
//! structurally parallel to `walk_directives`'s own `Enter`/`Exit` stack:
//! every `Enter(idx)` pushes a fresh bucket for `idx` right where it pushes
//! `idx`'s matching `Exit` marker (so the two stacks stay 1:1, for leaf and
//! non-leaf nodes alike), and every directive that instantiates a new frame
//! for `idx` records its name not into that just-pushed bucket but into the
//! bucket beneath it — the one `idx`'s own parent pushed — so the name is
//! popped when the parent's subtree (not `idx`'s own subtree) finishes.
//! Each `Exit` pops its matching bucket and calls
//! [`raikiri_traits::PageContext::pop_counter_scope`] for every name in it.
//! A root-level frame-creating directive (applied while `pending_pops` is
//! still empty, i.e. before any bucket exists) has no ancestor exit inside
//! this function to pop at and is intentionally left open for the whole
//! [`drive_document`]/[`drive_page`] call — see [`drive_page`]'s own doc
//! for why that frame in fact stays open for `ctx`'s entire lifetime, not
//! just the one call: nothing pops it later, so it is still there,
//! unconditionally, whether or not a second [`drive_page`] call ever
//! touches the same `ctx` — a second such call is only what turns that
//! standing frame into an *observable* correctness problem.
//!
//! Telling "creates a new frame" apart from "mutates an inherited one" for
//! `counter-increment`/`counter-set` needs a look at `ctx`'s counter state
//! *before* the directive is applied — [`GcpmDirective::CounterIncrement`]
//! and [`GcpmDirective::CounterSet`] carry only the delta/value, not whether
//! the target counter is currently absent — so [`walk_directives`] reads
//! [`raikiri_traits::PageContext::counter`] first and treats an absent
//! counter, or one whose `CounterStack::values` is empty, as the
//! frame-creating case. `crate::target::CounterScopes::apply` (the sibling
//! ancestor-chain-only tracker mentioned above) makes this same
//! frame-creating-or-not decision too, but fuses the check and the mutation
//! into one `match stack.last_mut()` on its own owned `stacks` map — it can
//! do that because it holds the map directly. This driver cannot: its
//! counter state lives behind [`PageContext::apply_directive`]'s
//! `()`-returning interface, which offers no way to learn after the fact
//! whether a call happened to create a frame, so the check has to be a
//! separate peek taken beforehand instead.

use raikiri_style::CascadeResult;
use raikiri_style::property::DisplayValue;
use raikiri_traits::{GcpmDirective, PageContext, Symbol};

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
///
/// **A `counter-reset` on `page_root` itself is never popped, by this call
/// or any later one.** Module doc "Counter-scope exit (CSS Lists 3 §4.3,
/// §4.4.2)" explains why: a root-level frame-creating directive has no
/// ancestor `Exit` inside this function's walk to close it at, so the
/// frame it opens stays open on `ctx` for as long as `ctx` lives, not just
/// for this one call — the `pending_pops` side-stack that tracks
/// everything else is local to this function and carries nothing across
/// calls. Calling `drive_page` again on the same `ctx` — for the same
/// `page_root`, or a different one whose own directives reset the same
/// counter name — stacks another open frame on top of the first,
/// permanently, since nothing outside a single call is watching which
/// root-level frames are still open. [`PageContext::counter`]'s
/// `CounterStack::values()` (what `counters()`-function rendering reads)
/// returns every open frame, so a later `counters(name, ..)` reference
/// would join whatever earlier frame(s) leaked this way together with the
/// frame the current page actually meant to expose. A caller driving more
/// than one page against one `ctx` must therefore give every page a
/// `page_root` whose own directives don't carry a `counter-reset` meant to
/// be scoped to that page alone — this driver has no mechanism to close a
/// root-level reset at any page boundary, so in practice it behaves as if
/// open for `ctx`'s whole lifetime, not just the one page, regardless of
/// which `page_root` it happens to be attached to.
///
/// **Most other counter scopes are already closed by the time this call
/// returns.** Every scope other than the `page_root`-level one above
/// closes at some `Exit` reached inside this same call (module doc
/// "Counter-scope exit"), so [`PageContext::counter`] read after this
/// function returns observes only whatever frame, if any, is still open at
/// `page_root`'s own level — not the value a nested element saw while its
/// own scope was still live. A caller that needs a counter's value from a
/// specific point in the walk should capture it from inside directive
/// processing instead, via a `string-set: <name> counter(<counter-name>)`
/// directive resolved into a [`PageContext::string_state`] entry (the
/// pattern this module's own tests use): `NamedStringState` freezes the
/// value at resolution time and, unlike `counters`, this walk never pops
/// it.
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
///
/// The root-level counter-reset leak documented on [`drive_page`] applies
/// here too, one level up: this function always walks from `doc.root`, so
/// a `counter-reset` applied by `doc.root` itself is never popped by this
/// call either, and the resulting standing frame is on `ctx` unconditionally
/// — the same single call this function is meant to make already leaves it
/// there, not only a hypothetical second call. It stays harmless as long as
/// nothing reads `counters()`-backed state off `ctx` afterward; see
/// [`drive_page`]'s own doc for the full explanation, including how a
/// caller should read a counter's value from inside the walk instead of
/// from `ctx` after this function returns.
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
/// [`crate::target::build_target_registry`], extended with an `Exit` step
/// that pops nested counter scopes at the *instantiating element's parent's*
/// exit — see module doc "Counter-scope exit (CSS Lists 3 §4.3, §4.4.2)" for
/// the `pending_pops` side-stack this uses to track which names to pop at
/// each `Exit`.
fn walk_directives(ctx: &mut PageContext, doc: &Document, cascade: &CascadeResult, root: usize) {
    enum WalkStep {
        Enter(usize),
        Exit,
    }

    let mut stack = vec![WalkStep::Enter(root)];
    // `pending_pops[i]` holds the counter names to pop at the `Exit`
    // matching the `Enter` that pushed bucket `i` — see module doc
    // "Counter-scope exit (CSS Lists 3 §4.3, §4.4.2)" for why a name lands
    // in the *parent's* bucket (index `pending_pops.len() - 1` at the moment
    // the instantiating element is entered, i.e. the bucket the parent
    // itself pushed) rather than the instantiating element's own bucket.
    let mut pending_pops: Vec<Vec<Symbol>> = Vec::new();
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
                    // effect." `display: none` (whole-subtree box omission)
                    // and `display: contents` (element generates no box of
                    // its own, CSS Display 3 §2.5) both qualify — same skip
                    // crate::target::build_target_registry's own `Enter`
                    // step already applies (see that function's doc).
                    Some(cv)
                        if matches!(cv.display, DisplayValue::None | DisplayValue::Contents) => {}
                    Some(cv) => {
                        directives.clear();
                        derive_element_directives(doc, idx, cv, &mut directives);
                        for directive in &directives {
                            // Which counter, if any, this directive is about
                            // to instantiate a brand new frame for — decided
                            // from `ctx`'s state *before* applying it, since
                            // `apply_directive` mutates `ctx` and returns
                            // `()`. `counter-reset` (CSS Lists 3 §4.1)
                            // always instantiates unconditionally;
                            // `counter-increment`/`counter-set` (CSS Lists 3
                            // §4.4.2) only do when the counter has no active
                            // frame yet, otherwise they mutate the innermost
                            // existing (possibly ancestor-inherited) frame
                            // in place and open no new scope of their own.
                            let instantiated_name = match directive {
                                GcpmDirective::CounterReset { name, .. } => Some(name),
                                GcpmDirective::CounterIncrement { name, .. }
                                | GcpmDirective::CounterSet { name, .. }
                                    if ctx
                                        .counter(name)
                                        .is_none_or(|stack| stack.values().is_empty()) =>
                                {
                                    Some(name)
                                }
                                _ => None,
                            };

                            ctx.apply_directive(directive);

                            // Every scope newly instantiated by this
                            // element's own directive must be popped at this
                            // element's PARENT's exit, not this element's
                            // own exit (CSS Lists 3 §4.3 grants the scope to
                            // following siblings too, not just
                            // descendants) — so record it in the bucket the
                            // parent already pushed, not a bucket of this
                            // element's own. `pending_pops` is empty only
                            // for `root` itself (no `Enter` has pushed a
                            // bucket yet at that point), in which case a
                            // root-level instantiating directive has no
                            // ancestor exit inside this call to pop at and
                            // is intentionally left open for the whole walk.
                            if let Some(name) = instantiated_name
                                && let Some(parent_bucket) = pending_pops.last_mut()
                            {
                                parent_bucket.push(name.clone());
                            }
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
                // This element's OWN bucket — accumulates any newly
                // instantiated counter-scope names pushed by its children
                // (see the `Enter` branch above) as they're processed, to be
                // popped when this element's own `Exit` (just pushed above)
                // is reached.
                pending_pops.push(Vec::new());
                for &child in node.children.iter().rev() {
                    stack.push(WalkStep::Enter(child));
                }
            }
            WalkStep::Exit => {
                // Pop the bucket pushed by the matching `Enter` and close
                // every counter scope it accumulated. `pending_pops.pop()`
                // is `None` only if this `Exit` has no matching `Enter`
                // bucket — structurally impossible: every `Enter(idx)` path
                // pushes exactly one `WalkStep::Exit` and one
                // `pending_pops` bucket together (the `!is_in_document()`
                // early `continue` skips both; every other path — the
                // `display:none` arm, the defensive `None` arm, and the
                // normal directive-applying arm — falls through to both),
                // so the two stacks stay 1:1 for every node, leaf or not.
                //
                // cov:ignore: the `None` branch below is unreachable per
                // the invariant above; kept as a graceful no-op rather than
                // `.expect()` so a violation (if one ever existed) can't
                // panic on parsed DOM input, matching the "defensive, not
                // panicking" shape `crate::target::build_target_registry`'s
                // own `.get(idx)` handling already uses nearby.
                if let Some(names) = pending_pops.pop() {
                    for name in names {
                        ctx.pop_counter_scope(&name);
                    }
                }
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
    fn drive_page_applies_counter_directives_from_a_subtree_root() {
        let mut doc = Document::new();
        let h2 = doc.append_element(
            Some(0),
            "h2",
            Style::default(),
            Some("counter-reset: c 1; counter-increment: c 1"),
        );
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let mut ctx = PageContext::default();
        // Walk `h2` itself as the page root, not `doc.root`: a CounterReset
        // applied by the walked root's own element has no enclosing
        // `pending_pops` bucket to be recorded into (see module doc
        // "Counter-scope exit"), so it is never popped within this call and
        // the fully composed value survives to be read back below. Walking
        // from `doc.root` instead would correctly pop this scope by the
        // time the call returns, since `h2` would then be a plain child
        // whose reset gets popped at its real parent's exit — this is
        // exactly what CSS Lists 3 §4.3 requires, it just means a
        // whole-document walk isn't the right vantage point to observe a
        // single element's own composed value from the outside.
        ctx.set_targets(build_target_registry(&doc, &cr));
        drive_page(&mut ctx, &doc, &cr, 0, h2);

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
    fn drive_page_walks_in_document_order_not_arena_insertion_order() {
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
        // Walk `section` (the resetting element) itself as the page root,
        // not `doc.root`: `section`'s own CounterReset would otherwise be
        // popped at `doc.root`'s exit — the very last step of a
        // whole-document walk — before this assertion could ever observe
        // the composed value. See module doc "Counter-scope exit".
        ctx.set_targets(build_target_registry(&doc, &cr));
        drive_page(&mut ctx, &doc, &cr, 0, parent);

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
    fn drive_document_skips_directives_on_display_contents_elements() {
        // Companion to drive_document_skips_directives_on_display_none_elements
        // above — display:contents also generates no box for the element
        // itself (CSS Display 3 §2.5), so CSS Lists 3 §4.5's "no effect"
        // rule applies to it the same way.
        let mut doc = Document::new();
        doc.append_element(
            Some(0),
            "div",
            Style::default(),
            Some("display: contents; counter-reset: c 5"),
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
            "a display:contents element's counter-reset must have no effect \
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
            // string-set is the probe that actually discriminates a broken
            // skip here: `hidden`'s own counter-reset gets popped at tmpl's
            // Exit regardless of whether the skip worked (tmpl itself is
            // always walked, so its Exit always fires and drains whatever
            // pending_pops bucket accumulated under it) — so a bare
            // `ctx.counter(&hidden) == None` check below can no longer tell
            // "skip worked, nothing ever ran" apart from "skip broken, but
            // got cleaned up on the way out". `string_state`, unlike
            // `counters`, is never popped by walk_directives at all, so its
            // presence is unambiguous proof the templated element was
            // walked.
            Some("counter-reset: hidden 9; string-set: hidden_probe counter(hidden)"),
        );
        // This sibling's parent is `doc.root` itself, so its implicitly
        // instantiated scope (CSS Lists 3 §4.4.2) closes at `doc.root`'s own
        // exit — the very last step of this whole-document walk — same as
        // `hidden`'s reset closing at `tmpl`'s exit above. Same fix as
        // `hidden_probe`: a `string-set` probe freezes the value while the
        // scope is still open, rather than reading `counter-increment`'s
        // target back through `ctx.counter()` after `drive_document`
        // returns, which would observe it only after it has already been
        // popped.
        doc.append_element(
            Some(0),
            "p",
            Style::default(),
            Some("counter-increment: visible 1; string-set: visible_probe counter(visible)"),
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
        // The discriminating assertion: string_state is never popped by
        // walk_directives, so its presence would be unambiguous proof the
        // templated element was walked at all — unlike the counter check
        // above, which a broken skip could still pass (see this test's
        // tmpl <p> fixture comment).
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            ctx.string_state(&Symbol::new("hidden_probe")),
            None,
            "a string-set inside a <template> subtree must never execute at \
             all — its presence (even a since-popped counter value) would \
             prove the <template> skip failed to prevent the subtree from \
             being walked"
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            ctx.string_state(&Symbol::new("visible_probe"))
                .and_then(|s| s.running()),
            Some("1"),
            "a sibling counter-increment outside the <template> must still \
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

    // ── Counter-scope exit (CSS Lists 3 §4.3, §4.4.2) ───────────────
    //
    // These tests need to observe a counter's value at a point *inside* a
    // walk, before some later `Exit` step pops it away — `ctx.counter()`
    // alone cannot do that from outside this driver (see the tests above,
    // which sidestep the same problem by walking a subtree's own root
    // directly, so its top-level scope is never popped within that call).
    // Here, `string-set: <probe-name> counter(<name>)` is used instead: it
    // freezes whatever `<name>` currently reads into a `NamedStringState`,
    // which `walk_directives` never pops (only `counters` gets that
    // treatment), so the frozen value survives to be read back after the
    // whole walk completes, from wherever in the tree it was taken.

    #[test]
    fn sibling_elements_resetting_the_same_counter_get_independent_same_depth_scopes() {
        // The bug this pins (previously described by this module's own doc
        // as a known limitation for as long as `Exit` was a no-op): two
        // sibling elements each doing `counter-reset` for the same name
        // must each start their own independent, same-depth nested scope,
        // not accumulate on top of each other or leak past their shared
        // parent's subtree.
        let mut doc = Document::new();
        let section_a = doc.append_element(
            Some(0),
            "section",
            Style::default(),
            Some("counter-reset: c 0"),
        );
        doc.append_element(
            Some(section_a),
            "p",
            Style::default(),
            Some("counter-increment: c 1; string-set: probe_a counter(c)"),
        );
        let section_b = doc.append_element(
            Some(0),
            "section",
            Style::default(),
            Some("counter-reset: c 0"),
        );
        doc.append_element(
            Some(section_b),
            "p",
            Style::default(),
            Some("counter-increment: c 1; string-set: probe_b counter(c)"),
        );
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let mut ctx = PageContext::default();
        drive_document(&mut ctx, &doc, &cr);

        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            ctx.string_state(&Symbol::new("probe_a"))
                .and_then(|s| s.running()),
            Some("1"),
            "section A's own <p> must see section A's independent scope (0 + 1 = 1)"
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            ctx.string_state(&Symbol::new("probe_b"))
                .and_then(|s| s.running()),
            Some("1"),
            "section B's own <p> must see section B's OWN independent, same-depth \
             scope (0 + 1 = 1) — not 2, which is what accumulating on top of \
             section A's still-open scope instead of starting a fresh one would \
             produce"
        );
        // Both sections are direct children of doc.root, so both scopes are
        // recorded into doc.root's own bucket and popped at doc.root's exit
        // — the terminal step of this call. A stack that failed to unwind
        // both nested frames (the "never pops at all" bug this whole
        // feature fixes) would leave a leftover value here instead.
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            ctx.counter(&Symbol::new("c")).and_then(|c| c.current()),
            None,
            "both sections' scopes must be fully closed once their shared parent's \
             (doc.root's) subtree walk completes — no leaked/accumulating depth"
        );
    }

    #[test]
    fn a_later_sibling_still_sees_an_earlier_siblings_reset_scope() {
        // The "following siblings" half of CSS Lists 3 §4.3 specifically:
        // a reset on an early sibling must remain visible to a LATER
        // sibling (not a descendant of the resetting element) — this is
        // what distinguishes popping at the resetting element's own exit
        // (wrong: would already have closed the scope by the time the next
        // sibling runs) from popping at the resetting element's PARENT's
        // exit (correct: stays open through every following sibling too).
        let mut doc = Document::new();
        doc.append_element(Some(0), "p", Style::default(), Some("counter-reset: c 10"));
        doc.append_element(
            Some(0),
            "p",
            Style::default(),
            Some("counter-increment: c 5; string-set: probe counter(c)"),
        );
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let mut ctx = PageContext::default();
        drive_document(&mut ctx, &doc, &cr);

        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            ctx.string_state(&Symbol::new("probe"))
                .and_then(|s| s.running()),
            Some("15"),
            "the later sibling's counter-increment must mutate the still-open \
             scope the earlier sibling's counter-reset established (10 + 5 = 15) \
             — popping at the earlier sibling's own exit would have already \
             closed it, giving 5 (implicit re-creation) instead"
        );
    }

    #[test]
    fn root_level_counter_reset_is_never_erroneously_popped_mid_walk() {
        // A counter-reset applied by the walk's own `root` argument (rather
        // than by a descendant reached from it) has no enclosing
        // `pending_pops` bucket to be recorded into — see module doc
        // "Counter-scope exit". It must stay open for the whole call, even
        // once its own descendants have been fully processed.
        let mut doc = Document::new();
        let page_root =
            doc.append_element(Some(0), "div", Style::default(), Some("counter-reset: c 5"));
        doc.append_element(
            Some(page_root),
            "p",
            Style::default(),
            Some("counter-increment: c 1"),
        );
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let mut ctx = PageContext::default();
        ctx.set_targets(build_target_registry(&doc, &cr));
        drive_page(&mut ctx, &doc, &cr, 0, page_root);

        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            ctx.counter(&Symbol::new("c")).and_then(|c| c.current()),
            Some(6),
            "a counter-reset on the walk's own root element must never be popped \
             within that same call — there is no ancestor exit inside the call to \
             pop it at, so the reset (5) plus its descendant's increment (1) must \
             still read back as 6"
        );
    }

    #[test]
    fn nested_resets_of_the_same_name_scope_independently_at_each_level() {
        // A reset nested inside another reset for the SAME name must (a)
        // give its own descendants (and following siblings, within its own
        // parent) an independent inner scope, and (b) not corrupt the outer
        // scope once the inner one closes — the outer value must reappear
        // unchanged for whatever comes after the inner scope's parent
        // exits. `CounterStack`'s own unit tests already pin the underlying
        // push/pop mechanics directly; this is the integration-level
        // version through this driver's actual document-order walk.
        let mut doc = Document::new();
        let outer = doc.append_element(
            Some(0),
            "section",
            Style::default(),
            Some("counter-reset: c 1"),
        );
        let inner = doc.append_element(
            Some(outer),
            "section",
            Style::default(),
            Some("counter-reset: c 100"),
        );
        doc.append_element(
            Some(inner),
            "p",
            Style::default(),
            Some("counter-increment: c 1; string-set: probe_leaf counter(c)"),
        );
        // A later sibling of `outer` itself (not of `inner`) — by the time
        // this runs, `inner`'s scope has already closed (at `outer`'s own
        // exit, which happens before this element is even entered, since
        // it comes after the whole `outer` subtree in document order), but
        // `outer`'s own scope is still open (it only closes at doc.root's
        // exit).
        doc.append_element(
            Some(0),
            "p",
            Style::default(),
            Some("counter-increment: c 1; string-set: probe_outer_sibling counter(c)"),
        );
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let mut ctx = PageContext::default();
        drive_document(&mut ctx, &doc, &cr);

        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            ctx.string_state(&Symbol::new("probe_leaf"))
                .and_then(|s| s.running()),
            Some("101"),
            "the innermost element must see the INNER reset's own scope (100 + 1 \
             = 101), nested on top of (not replacing) the outer scope"
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            ctx.string_state(&Symbol::new("probe_outer_sibling"))
                .and_then(|s| s.running()),
            Some("2"),
            "once the inner scope has closed (at `outer`'s own exit), a later \
             sibling of `outer` must see OUTER's own scope re-emerge unchanged by \
             the inner scope's activity (1 + 1 = 2), not the inner scope's value \
             or some corrupted combination of the two"
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            ctx.counter(&Symbol::new("c")).and_then(|c| c.current()),
            None,
            "outer's own scope must in turn be fully closed once doc.root's \
             subtree walk completes"
        );
    }

    #[test]
    fn implicit_counter_from_increment_does_not_leak_past_its_parents_subtree() {
        // The bug this pins: CSS Lists 3 §4.4.2
        // <https://www.w3.org/TR/css-lists-3/#instantiating-counters> lets
        // `counter-increment` instantiate a counter when none exists yet,
        // exactly like `counter-reset` does — but an earlier version of
        // `walk_directives` only ever recorded `CounterReset` names into
        // `pending_pops`, so a `counter-increment`-only instantiation was
        // never scheduled for any pop at all and stayed visible for the
        // rest of the whole-document walk, including to an uncle element
        // entirely outside its instantiating parent's subtree.
        let mut doc = Document::new();
        let parent = doc.append_element(Some(0), "section", Style::default(), None::<&str>);
        doc.append_element(
            Some(parent),
            "p",
            Style::default(),
            Some("counter-increment: c 1"),
        );
        // `uncle` is a following sibling of `parent`, not of the
        // instantiating `p` — CSS Lists 3 §4.3 grants the scope to
        // `parent`'s descendants and `parent`'s own following siblings, but
        // `uncle` is neither: it is outside `parent`'s subtree altogether,
        // so it must never observe the scope `parent`'s descendant opened.
        doc.append_element(
            Some(0),
            "p",
            Style::default(),
            Some("string-set: probe counter(c)"),
        );
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let mut ctx = PageContext::default();
        drive_document(&mut ctx, &doc, &cr);

        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            ctx.string_state(&Symbol::new("probe"))
                .and_then(|s| s.running()),
            Some("0"),
            "an uncle outside the implicitly-instantiating element's parent \
             subtree must read the counter as absent (formatted \"0\"), not \
             the leaked value 1 — the scope must have already closed at \
             `parent`'s own exit"
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            ctx.counter(&Symbol::new("c")).and_then(|c| c.current()),
            None,
            "the implicitly-instantiated scope must be fully closed by the \
             time the whole-document walk completes, same as an explicit \
             counter-reset would be"
        );
    }

    #[test]
    fn implicit_counter_from_set_does_not_leak_past_its_parents_subtree() {
        // Same bug, same fix, but for `counter-set` rather than
        // `counter-increment` — CSS Lists 3 §4.4.2 grants both the same
        // instantiate-when-absent behavior, and `walk_directives`'s
        // frame-creating check is a single `match` arm shared by both
        // `GcpmDirective::CounterIncrement` and `GcpmDirective::CounterSet`,
        // but nothing structurally guarantees the two stay in sync — this
        // pins `CounterSet` on its own rather than relying on the
        // `counter-increment` test above to cover it by proxy.
        let mut doc = Document::new();
        let parent = doc.append_element(Some(0), "section", Style::default(), None::<&str>);
        doc.append_element(
            Some(parent),
            "p",
            Style::default(),
            Some("counter-set: c 1"),
        );
        // Same uncle-outside-the-parent-subtree shape as the
        // `counter-increment` version above.
        doc.append_element(
            Some(0),
            "p",
            Style::default(),
            Some("string-set: probe counter(c)"),
        );
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let mut ctx = PageContext::default();
        drive_document(&mut ctx, &doc, &cr);

        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            ctx.string_state(&Symbol::new("probe"))
                .and_then(|s| s.running()),
            Some("0"),
            "an uncle outside the implicitly-instantiating element's parent \
             subtree must read the counter as absent (formatted \"0\"), not \
             the leaked value 1 — the scope must have already closed at \
             `parent`'s own exit"
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            ctx.counter(&Symbol::new("c")).and_then(|c| c.current()),
            None,
            "the implicitly-instantiated scope must be fully closed by the \
             time the whole-document walk completes, same as an explicit \
             counter-reset would be"
        );
    }

    #[test]
    fn implicit_counter_from_increment_is_visible_to_a_following_sibling() {
        // Companion to the leak-prevention test above: the "following
        // siblings" half of CSS Lists 3 §4.3 applies to an implicitly
        // instantiated counter (CSS Lists 3 §4.4.2) exactly as it does to an
        // explicit `counter-reset` — a following sibling *within the same
        // parent* as the instantiating element must still see the scope,
        // not just its own descendants.
        let mut doc = Document::new();
        let parent = doc.append_element(Some(0), "section", Style::default(), None::<&str>);
        doc.append_element(
            Some(parent),
            "p",
            Style::default(),
            Some("counter-increment: c 1"),
        );
        doc.append_element(
            Some(parent),
            "p",
            Style::default(),
            Some("string-set: probe counter(c)"),
        );
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let mut ctx = PageContext::default();
        drive_document(&mut ctx, &doc, &cr);

        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            ctx.string_state(&Symbol::new("probe"))
                .and_then(|s| s.running()),
            Some("1"),
            "a following sibling under the SAME parent as the implicitly \
             instantiating element must still inherit the scope (read back \
             as 1), since it has not yet closed at that shared parent's exit"
        );
    }
}
