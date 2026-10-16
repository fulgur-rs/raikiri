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
    let list = doc.append_element(
        Some(doc.root_index()),
        "ol",
        Style::default(),
        Some("counter-reset: list-item"),
    );
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
    let explicit_list = doc.append_element(
        Some(doc.root_index()),
        "ol",
        Style::default(),
        Some("counter-reset: list-item"),
    );
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
    let nested_list = doc.append_element(
        Some(first),
        "ol",
        Style::default(),
        Some("counter-reset: list-item"),
    );
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
    // An explicit reset on a list item nests inside the list's own level and
    // does not suppress the item's implicit increment (CSS Lists 3 §4.6), so
    // numbering continues from it.
    assert_eq!(snapshots[explicit][&Symbol::new("list-item")], vec![0, 8]);
    assert_eq!(
        snapshots[explicit_following][&Symbol::new("list-item")],
        vec![0, 9]
    );
    assert_eq!(snapshots[nested][&Symbol::new("list-item")], vec![1, 1]);
    for (node, expected) in [
        (first, "1. "),
        (second, "2. "),
        (third, "3. "),
        (explicit, "8. "),
        (explicit_following, "9. "),
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
        scopes.apply(&cr.computed[node], false, None);
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
    let snapshots = counter_snapshots(doc, &cr).expect("test counter snapshots stay within budget");
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

#[test]
fn list_items_increment_list_item_before_their_counter_set() {
    let mut doc = Document::new();
    let list = doc.append_element(
        Some(doc.root_index()),
        "ol",
        Style::default(),
        Some("counter-reset: list-item 4"),
    );
    let item = |doc: &mut Document, style: &str| {
        doc.append_element(
            Some(list),
            "li",
            Style::default(),
            Some(format!("display: list-item; {style}").as_str()),
        )
    };
    let first = item(&mut doc, "");
    let set = item(&mut doc, "counter-set: list-item 10");
    let after_set = item(&mut doc, "");
    let explicit = item(&mut doc, "counter-increment: list-item 5");
    let not_list_item =
        doc.append_element(Some(list), "div", Style::default(), Some("display: block"));
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    let snapshots = counter_snapshots(&doc, &cr).expect("counter snapshots");
    let list_item = Symbol::new("list-item");
    assert_eq!(snapshots[first][&list_item], vec![5]);
    assert_eq!(snapshots[set][&list_item], vec![10]);
    assert_eq!(snapshots[after_set][&list_item], vec![11]);
    // An explicit list-item increment replaces the implicit one.
    assert_eq!(snapshots[explicit][&list_item], vec![16]);
    assert_eq!(snapshots[not_list_item][&list_item], vec![16]);
}

#[test]
fn list_item_before_pseudo_increments_list_item_for_itself_and_children() {
    let mut doc = Document::new();
    let style = doc.append_element(
        Some(doc.root_index()),
        "style",
        Style::default(),
        None::<&str>,
    );
    doc.append_text(
        style,
        "div::before { display: list-item; list-style: none; content: counter(list-item) }",
    );
    let host = doc.append_element(
        Some(doc.root_index()),
        "div",
        Style::default(),
        Some("counter-reset: list-item 4"),
    );
    let child = doc.append_element(Some(host), "span", Style::default(), None::<&str>);
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    let snapshots = counter_snapshots(&doc, &cr).expect("counter snapshots");
    let (_, text) = crate::generated_content::generated_text(
        &doc,
        &cr,
        host,
        raikiri_style::PseudoElem::Before,
        &snapshots,
    )
    .expect("::before content");
    assert_eq!(text, "5");
    assert_eq!(snapshots[child][&Symbol::new("list-item")], vec![5]);
}

#[test]
fn list_item_after_pseudo_increments_carry_to_following_siblings() {
    let mut doc = Document::new();
    let style = doc.append_element(
        Some(doc.root_index()),
        "style",
        Style::default(),
        None::<&str>,
    );
    doc.append_text(
        style,
        "div::after { display: list-item; list-style: none; content: counter(list-item) }",
    );
    let list = doc.append_element(
        Some(doc.root_index()),
        "section",
        Style::default(),
        Some("counter-reset: list-item"),
    );
    let first = doc.append_element(Some(list), "div", Style::default(), None::<&str>);
    let second = doc.append_element(Some(list), "div", Style::default(), None::<&str>);
    let following = doc.append_element(Some(list), "span", Style::default(), None::<&str>);
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    let snapshots = counter_snapshots(&doc, &cr).expect("counter snapshots");
    for (node, expected) in [(first, "1"), (second, "2")] {
        let (_, text) = crate::generated_content::generated_text(
            &doc,
            &cr,
            node,
            raikiri_style::PseudoElem::After,
            &snapshots,
        )
        .expect("::after content");
        assert_eq!(text, expected);
    }
    assert_eq!(snapshots[following][&Symbol::new("list-item")], vec![2]);
}

/// Counter `name`'s innermost value in each of `nodes`' snapshots.
fn innermost_values(doc: &Document, nodes: &[usize], name: &str) -> Vec<i32> {
    nodes
        .iter()
        .map(|&node| *stack_at(doc, node, name).last().expect("counter in scope"))
        .collect()
}

#[test]
fn reversed_counter_starting_value_follows_the_increments_up_to_the_first_set() {
    // CSS Lists 3 §4.4.2: the starting value is the sum of the negated
    // increments up to the first `counter-set` (excluded), plus that set's
    // value, plus the last non-zero negated increment.
    let cases: [(&[&str], [i32; 1], &[i32]); 4] = [
        (
            &[
                "counter-increment: foo -2",
                "counter-increment: foo -2",
                "counter-increment: foo -1; counter-set: foo 5",
                "counter-increment: foo -2",
            ],
            [10],
            &[8, 6, 5, 3],
        ),
        (
            &[
                "counter-increment: foo -3",
                "counter-increment: foo -2; counter-set: foo 5",
                "counter-increment: foo -1",
                "counter-increment: foo -8",
            ],
            [10],
            &[7, 5, 4, -4],
        ),
        (
            &[
                "counter-increment: foo 2; counter-set: foo 5",
                "counter-increment: foo 2",
                "counter-increment: foo 2",
            ],
            [3],
            &[5, 7, 9],
        ),
        (
            &[
                "counter-increment: foo 3",
                "counter-increment: foo 1",
                "counter-increment: foo 8; counter-set: foo 5",
            ],
            [-7],
            &[-4, -3, 5],
        ),
    ];
    for (items, start, expected) in cases {
        let mut doc = Document::new();
        let list = doc.append_element(
            Some(doc.root_index()),
            "div",
            Style::default(),
            Some("counter-reset: reversed(foo)"),
        );
        let nodes: Vec<usize> = items
            .iter()
            .map(|style| doc.append_element(Some(list), "div", Style::default(), Some(*style)))
            .collect();
        doc.mark_in_document_flags();
        assert_eq!(innermost_values(&doc, &[list], "foo"), start, "{items:?}");
        assert_eq!(innermost_values(&doc, &nodes, "foo"), expected, "{items:?}");
    }
}

#[test]
fn reversed_counter_counts_following_siblings_and_skips_display_none() {
    let mut doc = Document::new();
    let list = doc.append_element(
        Some(doc.root_index()),
        "div",
        Style::default(),
        Some("counter-reset: reversed(foo)"),
    );
    let inner = doc.append_element(
        Some(list),
        "div",
        Style::default(),
        Some("counter-increment: foo -1"),
    );
    doc.append_element(
        Some(list),
        "div",
        Style::default(),
        Some("display: none; counter-increment: foo -5"),
    );
    let following = doc.append_element(
        Some(doc.root_index()),
        "div",
        Style::default(),
        Some("counter-increment: foo -2"),
    );
    doc.mark_in_document_flags();
    // num = 1 + 2, plus the last non-zero negated increment 2.
    assert_eq!(
        innermost_values(&doc, &[list, inner, following], "foo"),
        [5, 4, 2]
    );
}

#[test]
fn reversed_list_item_counter_counts_list_items_down() {
    let mut doc = Document::new();
    let list = doc.append_element(
        Some(doc.root_index()),
        "ol",
        Style::default(),
        Some("counter-reset: reversed(list-item)"),
    );
    let item = |doc: &mut Document, style: &str| {
        doc.append_element(
            Some(list),
            "li",
            Style::default(),
            Some(format!("display: list-item; {style}").as_str()),
        )
    };
    let first = item(&mut doc, "");
    let nested = doc.append_element(
        Some(first),
        "ol",
        Style::default(),
        Some("counter-reset: list-item"),
    );
    let nested_item = doc.append_element(
        Some(nested),
        "li",
        Style::default(),
        Some("display: list-item"),
    );
    let second = item(&mut doc, "");
    let third = item(&mut doc, "");
    doc.mark_in_document_flags();
    assert_eq!(
        innermost_values(&doc, &[first, second, third], "list-item"),
        [3, 2, 1]
    );
    // A nested ascending list-item counter still counts up.
    assert_eq!(stack_at(&doc, nested_item, "list-item"), [3, 1]);
}

#[test]
fn reversed_counter_with_a_given_starting_value_keeps_it() {
    let mut doc = Document::new();
    let list = doc.append_element(
        Some(doc.root_index()),
        "ol",
        Style::default(),
        Some("counter-reset: reversed(list-item) 10"),
    );
    let item = doc.append_element(
        Some(list),
        "li",
        Style::default(),
        Some("display: list-item"),
    );
    doc.mark_in_document_flags();
    assert_eq!(innermost_values(&doc, &[list, item], "list-item"), [10, 9]);
}

#[test]
fn pseudo_local_reversed_counter_is_calculated_from_its_own_directives() {
    let mut doc = Document::new();
    let style = doc.append_element(
        Some(doc.root_index()),
        "style",
        Style::default(),
        None::<&str>,
    );
    doc.append_text(
        style,
        "div::before { counter-reset: reversed(c); counter-increment: c -2; content: counter(c) } \
         p::before { display: list-item; list-style: none; \
         counter-reset: reversed(list-item); content: counter(list-item) }",
    );
    let div = doc.append_element(
        Some(doc.root_index()),
        "div",
        Style::default(),
        None::<&str>,
    );
    let p = doc.append_element(Some(doc.root_index()), "p", Style::default(), None::<&str>);
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    let snapshots = counter_snapshots(&doc, &cr).expect("counter snapshots");
    for (node, expected) in [(div, "2"), (p, "1")] {
        let (_, text) = crate::generated_content::generated_text(
            &doc,
            &cr,
            node,
            raikiri_style::PseudoElem::Before,
            &snapshots,
        )
        .expect("::before content");
        assert_eq!(text, expected);
    }
}
