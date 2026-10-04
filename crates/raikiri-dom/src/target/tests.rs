use super::*;

use raikiri_style::{build_rule_tree, cascade};
use smol_str::SmolStr;
use taffy::Style;

#[test]
fn counter_snapshot_limit_error_has_stable_display() {
    let error = CounterSnapshotLimitExceeded {
        limit: 32,
        actual: 33,
    };
    assert_eq!(
        error.to_string(),
        "counter snapshot memory limit exceeded: 33 bytes (limit 32)"
    );
}

#[test]
fn target_counter_page_style_resolution_smoke() {
    let mut registry = TargetRegistry::default();
    let directive = synthesize_register_target("chapter-2");
    let mut info = TargetInfo::default();
    info.counters.insert(Symbol::new("page"), vec![2]);
    apply_register_target(directive, info, &mut registry);

    assert_eq!(
        registry.resolve_target_counter(
            "#chapter-2",
            Symbol::new("page"),
            raikiri_style::property::CounterStyle::Decimal,
        ),
        raikiri_traits::ResolveOutcome::Resolved("2".to_owned())
    );
}

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
#[test]
fn counter_snapshots_resolve_named_reset_and_following_increments() {
    let mut doc = Document::new();
    let scope = doc.append_element(
        Some(0),
        "section",
        Style::default(),
        Some("counter-reset: step 4"),
    );
    let first = doc.append_element(
        Some(scope),
        "div",
        Style::default(),
        Some("counter-increment: step"),
    );
    let second = doc.append_element(
        Some(scope),
        "div",
        Style::default(),
        Some("counter-increment: step"),
    );
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    let snapshots =
        counter_snapshots(&doc, &cr).expect("test counter snapshots stay within budget");
    assert_eq!(snapshots[scope][&Symbol::new("step")], vec![4]);
    assert_eq!(snapshots[first][&Symbol::new("step")], vec![5]);
    assert_eq!(snapshots[second][&Symbol::new("step")], vec![6]);
}

#[test]
fn counter_snapshot_budget_accounts_for_nested_stack_capacity_before_clone() {
    const DEPTH: usize = 32;

    let mut doc = Document::new();
    let mut nested_nodes = Vec::with_capacity(DEPTH);
    let mut parent = doc.root_index();
    for _ in 0..DEPTH {
        parent = doc.append_element(
            Some(parent),
            "div",
            Style::default(),
            Some("counter-reset: nested"),
        );
        nested_nodes.push(parent);
    }
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");

    let mut scopes = CounterScopes::default();
    let mut shallow_cost = None;
    for (index, node) in nested_nodes.into_iter().enumerate() {
        scopes.apply(&cr.computed[node], None);
        if index == 0 {
            shallow_cost = scopes.estimated_snapshot_bytes();
        }
    }
    let deep_cost = scopes.estimated_snapshot_bytes().expect("bounded fixture");
    let shallow_cost = shallow_cost.expect("first nested scope");
    assert!(
        deep_cost > shallow_cost,
        "nested counter values must contribute to the snapshot estimate"
    );

    let mut budget = CounterSnapshotBudget::new(deep_cost - 1);
    let error = scopes
        .snapshot(&mut budget)
        .expect_err("deep snapshot must be rejected before cloning");
    assert_eq!(error.limit, deep_cost - 1);
    assert_eq!(error.actual, deep_cost);
}

#[test]
fn counter_snapshots_include_before_pseudo_scope_for_descendants() {
    let mut doc = Document::new();
    let style = doc.append_element(
        Some(doc.root_index()),
        "style",
        Style::default(),
        None::<&str>,
    );
    doc.append_text(
        style,
        r#"div::before { content: counters(test, "."); counter-reset: test }"#,
    );
    let outer = doc.append_element(
        Some(doc.root_index()),
        "div",
        Style::default(),
        None::<&str>,
    );
    let inner = doc.append_element(Some(outer), "div", Style::default(), None::<&str>);
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    let snapshots =
        counter_snapshots(&doc, &cr).expect("test counter snapshots stay within budget");

    assert!(!snapshots[outer].contains_key(&Symbol::new("test")));
    assert_eq!(snapshots[inner][&Symbol::new("test")], vec![0]);
}

#[test]
fn counter_snapshots_ignore_none_before_pseudo_scope() {
    let mut doc = Document::new();
    let style = doc.append_element(
        Some(doc.root_index()),
        "style",
        Style::default(),
        None::<&str>,
    );
    doc.append_text(
        style,
        r#"div::before { content: none; counter-reset: test }"#,
    );
    let outer = doc.append_element(
        Some(doc.root_index()),
        "div",
        Style::default(),
        None::<&str>,
    );
    let inner = doc.append_element(Some(outer), "div", Style::default(), None::<&str>);
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    let snapshots =
        counter_snapshots(&doc, &cr).expect("test counter snapshots stay within budget");

    assert!(!snapshots[inner].contains_key(&Symbol::new("test")));
}

#[test]
fn counter_snapshots_skip_display_none_and_apply_display_contents_descendants() {
    let mut doc = Document::new();
    let hidden = doc.append_element(
        Some(0),
        "div",
        Style::default(),
        Some("display: none; counter-reset: hidden 7"),
    );
    let hidden_child = doc.append_element(
        Some(hidden),
        "div",
        Style::default(),
        Some("counter-increment: hidden"),
    );
    let contents = doc.append_element(
        Some(0),
        "div",
        Style::default(),
        Some("display: contents; counter-reset: visible 2"),
    );
    let visible_child = doc.append_element(
        Some(contents),
        "div",
        Style::default(),
        Some("counter-increment: visible"),
    );
    let template = doc.append_element(Some(0), "template", Style::default(), None::<&str>);
    // The probe lives in the detached contents fragment (the parser's
    // shape): it is unreachable, so snapshots skip it. An ordinary
    // light-DOM child appended directly under the template element would
    // stay in-document instead.
    let frag = doc.allocate_template_fragment_root(template);
    let inert_child = doc.append_element(
        Some(frag),
        "div",
        Style::default(),
        Some("counter-increment: inert"),
    );
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    let snapshots =
        counter_snapshots(&doc, &cr).expect("test counter snapshots stay within budget");
    assert!(snapshots[hidden].is_empty());
    assert!(snapshots[hidden_child].is_empty());
    assert!(snapshots[contents].is_empty());
    assert_eq!(snapshots[visible_child][&Symbol::new("visible")], vec![1]);
    assert!(snapshots[inert_child].is_empty());
}

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

    let mut registry =
        build_target_registry(&doc, &cr).expect("counter snapshots remain within budget");
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

    let mut registry =
        build_target_registry(&doc, &cr).expect("counter snapshots remain within budget");
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

    let mut registry =
        build_target_registry(&doc, &cr).expect("counter snapshots remain within budget");
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

    let mut registry =
        build_target_registry(&doc, &cr).expect("counter snapshots remain within budget");
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

    let mut registry =
        build_target_registry(&doc, &cr).expect("counter snapshots remain within budget");
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

    let mut registry =
        build_target_registry(&doc, &cr).expect("counter snapshots remain within budget");
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

    let mut registry =
        build_target_registry(&doc, &cr).expect("counter snapshots remain within budget");
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

    let mut registry =
        build_target_registry(&doc, &cr).expect("counter snapshots remain within budget");
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

    let mut registry =
        build_target_registry(&doc, &cr).expect("counter snapshots remain within budget");
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

    let mut registry =
        build_target_registry(&doc, &cr).expect("counter snapshots remain within budget");
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

    let mut registry =
        build_target_registry(&doc, &cr).expect("counter snapshots remain within budget");
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

    let mut registry =
        build_target_registry(&doc, &cr).expect("counter snapshots remain within budget");
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

    let mut registry =
        build_target_registry(&doc, &cr).expect("counter snapshots remain within budget");
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

    let mut registry =
        build_target_registry(&doc, &cr).expect("counter snapshots remain within budget");
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

    let mut registry =
        build_target_registry(&doc, &cr).expect("counter snapshots remain within budget");
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

    let mut registry =
        build_target_registry(&doc, &cr).expect("counter snapshots remain within budget");
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

    let mut registry =
        build_target_registry(&doc, &cr).expect("counter snapshots remain within budget");
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

    let mut registry =
        build_target_registry(&doc, &cr).expect("counter snapshots remain within budget");
    let out = registry.resolve_target_text("#title", ContentPart::Content);
    assert_eq!(
        out,
        raikiri_traits::ResolveOutcome::Resolved("Visible".to_owned())
    );
}
