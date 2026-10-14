use super::*;

use raikiri_style::{build_rule_tree, cascade};
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

// ── CounterScopes ────────────────────────────────────────────
//
// `raikiri_style::ComputedValues` is `#[non_exhaustive]` cross-crate
// (raikiri-dom is not its defining crate), so it cannot be struct-
// literal-constructed here even with every field supplied — there is no
// builder either (matching `crate::running`'s own tests, which never
// construct `ComputedValues` directly). `CounterScopes` is therefore
// exercised only through real `cascade()` output, below, via
// `counter_snapshots`.

// ── counter_snapshots (arena-walk integration) ──────────────────
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
fn shared_counter_snapshots_are_reused_until_the_tree_changes() {
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
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");

    let shared = doc.shared_counter_snapshots(&cr).expect("snapshots");
    assert_eq!(shared[first][&Symbol::new("step")], vec![5]);
    let again = doc.shared_counter_snapshots(&cr).expect("snapshots");
    assert!(std::sync::Arc::ptr_eq(&shared, &again));

    // Another element cascade gets its own snapshots and leaves the shared
    // ones in place.
    let other = cascade(&doc, &rules).expect("cascade Ok");
    let separate = doc.shared_counter_snapshots(&other).expect("snapshots");
    assert!(!std::sync::Arc::ptr_eq(&shared, &separate));
    assert_eq!(*separate, *shared);
    let kept = doc.shared_counter_snapshots(&cr).expect("snapshots");
    assert!(std::sync::Arc::ptr_eq(&shared, &kept));

    // Laying the document out with another element cascade replaces them.
    crate::layout_single_page(&mut doc, &other, raikiri_traits::PageBox::A4).ok();
    let relaid = doc.shared_counter_snapshots(&other).expect("snapshots");
    let reused = doc.shared_counter_snapshots(&other).expect("snapshots");
    assert!(std::sync::Arc::ptr_eq(&relaid, &reused));
    assert!(!std::sync::Arc::ptr_eq(&relaid, &shared));

    // A tree mutation drops them, even for the same cascade.
    doc.detach_from_parent(first);
    doc.mark_in_document_flags();
    let rebuilt = doc.shared_counter_snapshots(&cr).expect("snapshots");
    assert!(rebuilt[first].is_empty());
    assert_eq!(rebuilt[scope][&Symbol::new("step")], vec![4]);
}

#[test]
fn counter_snapshots_include_implicit_list_item_ordinals() {
    let mut doc = Document::new();
    let list = doc.append_element(Some(doc.root_index()), "ol", Style::default(), None::<&str>);
    let first = doc.append_element(
        Some(list),
        "li",
        Style::default(),
        Some("display: list-item; list-style-type: decimal"),
    );
    doc.append_element(Some(list), "span", Style::default(), Some("display: block"));
    let second = doc.append_element(
        Some(list),
        "li",
        Style::default(),
        Some("display: list-item; list-style-type: decimal"),
    );
    let third = doc.append_element(
        Some(list),
        "li",
        Style::default(),
        Some("display: list-item; list-style-type: decimal"),
    );
    let explicit_list =
        doc.append_element(Some(doc.root_index()), "ol", Style::default(), None::<&str>);
    let explicit = doc.append_element(
        Some(explicit_list),
        "li",
        Style::default(),
        Some("display: list-item; list-style-type: decimal; counter-reset: list-item 7"),
    );
    let explicit_following = doc.append_element(
        Some(explicit_list),
        "li",
        Style::default(),
        Some("display: list-item; list-style-type: decimal"),
    );
    let nested_list = doc.append_element(Some(first), "ol", Style::default(), None::<&str>);
    let nested = doc.append_element(
        Some(nested_list),
        "li",
        Style::default(),
        Some("display: list-item; list-style-type: decimal"),
    );
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    let snapshots = counter_snapshots(&doc, &cr).expect("counter snapshots");

    assert_eq!(snapshots[first][&Symbol::new("list-item")], vec![1]);
    assert_eq!(snapshots[second][&Symbol::new("list-item")], vec![2]);
    assert_eq!(snapshots[third][&Symbol::new("list-item")], vec![3]);
    assert_eq!(snapshots[explicit][&Symbol::new("list-item")], vec![7]);
    assert_eq!(
        snapshots[explicit_following][&Symbol::new("list-item")],
        vec![7]
    );
    assert_eq!(snapshots[nested][&Symbol::new("list-item")], vec![1]);
    for (node, expected) in [
        (first, "1. "),
        (second, "2. "),
        (third, "3. "),
        (explicit, "7. "),
        (explicit_following, "7. "),
        (nested, "1. "),
    ] {
        let (_, marker) = crate::generated_content::markers::marker_render_info_with_snapshots(
            &doc, &cr, node, &snapshots,
        )
        .expect("list marker");
        assert_eq!(marker, expected);
    }
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

/// Counter stack of `name` in `node`'s snapshot, outermost level first.
fn stack_at(doc: &Document, node: usize, name: &str) -> Vec<i32> {
    let rules = build_rule_tree(doc);
    let cr = cascade(doc, &rules).expect("cascade Ok");
    let snapshots =
        counter_snapshots(doc, &cr).expect("test counter snapshots stay within budget");
    snapshots[node]
        .get(&Symbol::new(name))
        .cloned()
        .unwrap_or_default()
}

#[test]
fn counter_snapshots_same_name_nested_reset_produces_two_level_stack() {
    // CSS Lists 3 §4.3 "self-nesting": a second counter-reset for the same
    // name, encountered while an ancestor's scope for that name is still
    // active, pushes a nested level rather than replacing it.
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
    doc.mark_in_document_flags();
    assert_eq!(stack_at(&doc, leaf, "sec"), vec![1, 1]);
}

#[test]
fn counter_snapshots_increment_mutates_top_level_not_a_new_one() {
    // counter-reset and counter-increment for the same name on the same
    // element: the increment mutates the level the reset just pushed.
    let mut doc = Document::new();
    let el = doc.append_element(
        Some(0),
        "h2",
        Style::default(),
        Some("counter-reset: c 5; counter-increment: c 2"),
    );
    doc.mark_in_document_flags();
    assert_eq!(stack_at(&doc, el, "c"), vec![7]);
}

#[test]
fn counter_snapshots_increment_saturates_instead_of_panicking_or_wrapping_on_overflow() {
    // Spec-legal CSS driving counter-increment past i32::MAX must saturate.
    // The reset is `i32::MAX - 1`, so the expected value is reachable only
    // when the increment is actually applied.
    let mut doc = Document::new();
    let el = doc.append_element(
        Some(0),
        "h2",
        Style::default(),
        Some("counter-reset: c 2147483646; counter-increment: c 5"),
    );
    doc.mark_in_document_flags();
    assert_eq!(stack_at(&doc, el, "c"), vec![i32::MAX]);
}

#[test]
fn counter_snapshots_duplicate_name_in_one_counter_reset_keeps_only_last_value() {
    // CSS Lists 3 §4.1 <https://www.w3.org/TR/css-lists-3/#counter-reset>:
    // "If multiple instances of the same <counter-name> occur in the
    // property value, only the last one is honored."
    let mut doc = Document::new();
    let el = doc.append_element(
        Some(0),
        "h2",
        Style::default(),
        Some("counter-reset: c 1 c 2"),
    );
    doc.mark_in_document_flags();
    assert_eq!(stack_at(&doc, el, "c"), vec![2]);
}

#[test]
fn counter_increment_continues_across_siblings_sharing_an_ancestor_scope() {
    // Two children of a common <section> that carries counter-reset both
    // see, and continue, that still-open ancestor scope.
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
    let p2 = doc.append_element(
        Some(section),
        "p",
        Style::default(),
        Some("counter-increment: item"),
    );
    doc.mark_in_document_flags();
    assert_eq!(stack_at(&doc, p1, "item"), vec![1]);
    assert_eq!(stack_at(&doc, p2, "item"), vec![2]);
}

#[test]
fn counter_snapshots_counter_increment_visible_across_cross_branch_cousin() {
    // CSS Lists 3 §4.4.1 <https://www.w3.org/TR/css-lists-3/#inheriting-counters>'s
    // worked example: `#baz` inherits the counter from its previous sibling
    // `#foo`, but takes the value 2 from `#bar` (`#foo`'s child), the
    // previous element in tree order.
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
    doc.append_element(
        Some(foo),
        "div",
        Style::default(),
        Some("counter-increment: example"),
    );
    let baz = doc.append_element(Some(ul), "li", Style::default(), None::<&str>);
    doc.mark_in_document_flags();
    assert_eq!(stack_at(&doc, baz, "example"), vec![2]);
}

#[test]
fn counter_snapshots_counter_reset_obscures_earlier_sibling_scope() {
    // CSS Lists 3 §4.3's obscuring rule: the second section's own
    // counter-reset evicts the first section's still-open same-name level
    // instead of nesting inside it, so the full stack is `[5]`, not `[1, 5]`.
    let mut doc = Document::new();
    let sec1 = doc.append_element(
        Some(0),
        "section",
        Style::default(),
        Some("counter-reset: chapter 1"),
    );
    let h2_1 = doc.append_element(Some(sec1), "h2", Style::default(), None::<&str>);
    let sec2 = doc.append_element(
        Some(0),
        "section",
        Style::default(),
        Some("counter-reset: chapter 5"),
    );
    let h2_2 = doc.append_element(Some(sec2), "h2", Style::default(), None::<&str>);
    doc.mark_in_document_flags();
    assert_eq!(stack_at(&doc, h2_1, "chapter"), vec![1]);
    assert_eq!(stack_at(&doc, h2_2, "chapter"), vec![5]);
}

#[test]
fn counter_increment_without_ancestor_reset_persists_across_siblings() {
    // CSS Lists 3 §4.4.2 <https://www.w3.org/TR/css-lists-3/#instantiating-counters>:
    // a counter with no counter-reset anywhere is instantiated by p1's
    // increment, and that scope covers p1's following siblings too.
    let mut doc = Document::new();
    let p1 = doc.append_element(Some(0), "p", Style::default(), Some("counter-increment: x"));
    let p2 = doc.append_element(Some(0), "p", Style::default(), Some("counter-increment: x"));
    doc.mark_in_document_flags();
    assert_eq!(stack_at(&doc, p1, "x"), vec![1]);
    assert_eq!(stack_at(&doc, p2, "x"), vec![2]);
}

#[test]
fn counter_increment_on_following_sibling_of_reset_element_continues_that_scope() {
    // CSS Lists 3 §4.3 puts a counter-reset's scope over "the element's
    // descendants and its following siblings with their descendants".
    let mut doc = Document::new();
    doc.append_element(Some(0), "div", Style::default(), Some("counter-reset: c 5"));
    let p = doc.append_element(Some(0), "p", Style::default(), Some("counter-increment: c"));
    doc.mark_in_document_flags();
    assert_eq!(stack_at(&doc, p, "c"), vec![6]);
}
