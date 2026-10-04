use super::*;
use crate::cascade::cascade;
use crate::cascade::selector_match::{
    match_complex_selector_list, selector_matches_pseudo_element,
};
use crate::cascade::test_support::{BLUE, RED};
use crate::ruletree::{Origin, RuleTree};
use crate::style_dom::{StyleDom, StyleNode, StyleNodeId, StyleNodeKind, StyleQuirksMode};
use crate::test_dom::TestDoc;

fn rule_tree(css: &str) -> RuleTree {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(css, Origin::Author);
    tree
}

fn sorted_rules(tree: &RuleTree) -> Vec<&StyleRule> {
    let mut rules = tree.style_rules.iter().collect::<Vec<_>>();
    rules.sort_unstable_by_key(|rule| rule.source_order);
    rules
}

/// Walks `doc` in pre-order the same way the cascade does and calls `visit`
/// with each in-document element, its ancestor path, and the filter.
fn walk(doc: &TestDoc, mut visit: impl FnMut(StyleNodeId, &[StyleNodeId], &AncestorFilter)) {
    let mut stack = vec![(doc.root_id(), 0usize)];
    let mut path = Vec::new();
    let mut filter = AncestorFilter::new();
    while let Some((id, depth)) = stack.pop() {
        path.truncate(depth);
        filter.truncate(depth);
        assert_eq!(filter.depth(), path.len());
        let node = doc.node(id).expect("node");
        if node.kind() == StyleNodeKind::Element {
            let elem = node.as_element().expect("element");
            visit(id, &path, &filter);
            path.push(id);
            filter.push(&elem);
        }
        let start = stack.len();
        stack.extend(doc.child_ids(id).map(|child| (child, path.len())));
        stack[start..].reverse();
    }
}

/// Rule indices that the full matcher accepts for `id`, directly or through
/// a pseudo-element.
fn brute_force_matches(
    doc: &TestDoc,
    rules: &[&StyleRule],
    id: StyleNodeId,
    path: &[StyleNodeId],
) -> Vec<u32> {
    let node = doc.node(id).expect("node");
    let elem = node.as_element().expect("element");
    let quirks = doc.quirks_mode();
    let mut matched = Vec::new();
    for (idx, rule) in rules.iter().enumerate() {
        let direct =
            match_complex_selector_list(&rule.selectors, doc, &elem, id, path, quirks, None, false)
                .is_some();
        let pseudo = rule.selectors.slice().iter().any(|selector| {
            selector_matches_pseudo_element(doc, selector, &elem, id, path, quirks, false).is_some()
        });
        if direct || pseudo {
            matched.push(idx as u32);
        }
    }
    matched
}

const DIFFERENTIAL_CSS: &str = "
    p { color: red }
    P { color: red }
    #Main { color: red }
    #main { color: red }
    .Note { color: red }
    .note { color: red }
    .note.warn { color: red }
    * { color: red }
    div p { color: red }
    div > p { color: red }
    section .note { color: red }
    #main span { color: red }
    .missing p { color: red }
    .missing > span { color: red }
    article section div p { color: red }
    h1 + p { color: red }
    h1 ~ p { color: red }
    .warn + .note span { color: red }
    :is(.note, .warn) { color: red }
    :not(.note) { color: red }
    div:has(> p) { color: red }
    p::before { color: red }
    .note::after { content: 'x' }
    section::marker { color: red }
    ::before { color: red }
    div p::before { color: red }
    .missing p::after { content: 'y' }
    svg foreignObject { color: red }
    .a .b .c .d .e p { color: red }
    .a .b.c.d.e.f .g p { color: red }
    .a .b .c .d .e .z p { color: red }
    svg foreignobject { color: red }
";

fn differential_doc(quirks_mode: StyleQuirksMode) -> TestDoc {
    let mut doc = TestDoc::new();
    doc.quirks_mode = quirks_mode;
    let article = doc.push_element(0, "article", None);
    let section = doc.push_element_with_attrs(article, "section", None, &[("id", "MAIN")]);
    let div = doc.push_element_with_attrs(section, "div", None, &[("class", "WARN")]);
    let h1 = doc.push_element(div, "h1", None);
    doc.push_text(h1, "title");
    let p = doc.push_element_with_attrs(div, "p", None, &[("class", "note  Note\twarn")]);
    doc.push_element(p, "span", None);
    doc.push_element_with_attrs(div, "P", None, &[("class", "note"), ("id", "main")]);
    let deep = doc.push_element(div, "div", None);
    let deeper = doc.push_element_with_attrs(deep, "div", None, &[("class", "missing")]);
    doc.push_element(deeper, "p", None);
    doc.push_element(deeper, "span", None);
    // A chain with more ancestor requirements than the index keeps.
    let mut chain = article;
    for class in ["a", "b", "c", "d", "e", "b c d e f", "g"] {
        chain = doc.push_element_with_attrs(chain, "div", None, &[("class", class)]);
    }
    doc.push_element(chain, "p", None);
    let svg = doc.push_element_with_namespace(article, "svg", "http://www.w3.org/2000/svg", &[]);
    doc.push_element_with_namespace(svg, "foreignObject", "http://www.w3.org/2000/svg", &[]);
    doc
}

#[test]
fn candidates_are_a_superset_of_real_matches_in_every_quirks_mode() {
    let tree = rule_tree(DIFFERENTIAL_CSS);
    let rules = sorted_rules(&tree);
    let index = RuleIndex::new(rules.iter().copied());
    assert_eq!(index.rule_count(), rules.len());
    for quirks in [
        StyleQuirksMode::NoQuirks,
        StyleQuirksMode::LimitedQuirks,
        StyleQuirksMode::Quirks,
    ] {
        let doc = differential_doc(quirks);
        let mut total_candidates = 0;
        let mut total_pairs = 0;
        let mut total_matches = 0;
        let mut candidates = Vec::new();
        walk(&doc, |id, path, filter| {
            let node = doc.node(id).expect("node");
            let elem = node.as_element().expect("element");
            index.candidate_rules(&elem, filter, &mut candidates);
            assert!(candidates.windows(2).all(|w| w[0] < w[1]));
            let matches = brute_force_matches(&doc, &rules, id, path);
            total_matches += matches.len();
            for matched in matches {
                assert!(
                    candidates.contains(&matched),
                    "{quirks:?}: rule {matched} matches node {id:?} but was filtered out"
                );
            }
            total_candidates += candidates.len();
            total_pairs += rules.len();
        });
        assert!(
            total_matches > 0,
            "{quirks:?}: the workload matched nothing"
        );
        assert!(
            total_candidates < total_pairs,
            "{quirks:?}: the index tried {total_candidates} of {total_pairs} pairs"
        );
    }
}

#[test]
fn unrelated_buckets_are_not_tried() {
    let tree = rule_tree("#a { color: red } .b { color: red } em { color: red } * { color: red }");
    let rules = sorted_rules(&tree);
    let index = RuleIndex::new(rules.iter().copied());
    let mut doc = TestDoc::new();
    doc.push_element(0, "p", None);
    let mut candidates = Vec::new();
    walk(&doc, |id, _, filter| {
        let node = doc.node(id).expect("node");
        let elem = node.as_element().expect("element");
        index.candidate_rules(&elem, filter, &mut candidates);
    });
    // Only the universal rule survives for a bare `<p>`.
    assert_eq!(candidates, vec![3]);
}

#[test]
fn ancestor_filter_rejects_absent_descendant_requirements() {
    let tree = rule_tree(".outer p { color: red } h1 + p { color: red }");
    let rules = sorted_rules(&tree);
    let index = RuleIndex::new(rules.iter().copied());

    let mut doc = TestDoc::new();
    let plain = doc.push_element(0, "div", None);
    let p_plain = doc.push_element(plain, "p", None);
    let outer = doc.push_element_with_attrs(0, "div", None, &[("class", "outer")]);
    let p_outer = doc.push_element(outer, "p", None);

    let mut seen = Vec::new();
    let mut candidates = Vec::new();
    walk(&doc, |id, _, filter| {
        let node = doc.node(id).expect("node");
        let elem = node.as_element().expect("element");
        index.candidate_rules(&elem, filter, &mut candidates);
        seen.push((id, candidates.clone()));
    });
    let candidates_for = |target: usize| {
        seen.iter()
            .find(|(id, _)| *id == StyleNodeId::new(target as u64))
            .map(|(_, c)| c.clone())
            .expect("visited")
    };
    // The sibling-combinator rule carries no ancestor requirement, so it is
    // always a candidate for `p`; the descendant rule needs `.outer` above.
    assert_eq!(candidates_for(p_plain), vec![1]);
    assert_eq!(candidates_for(p_outer), vec![0, 1]);
}

#[test]
fn ancestor_filter_forgets_popped_ancestors() {
    let mut doc = TestDoc::new();
    let a = doc.push_element_with_attrs(0, "div", None, &[("class", "gone")]);
    let node = doc.node(StyleNodeId::new(a as u64)).expect("node");
    let elem = node.as_element().expect("element");
    let mut filter = AncestorFilter::new();
    filter.push(&elem);
    assert!(filter.might_contain_all(&[class_hash("gone"), tag_hash("DIV")]));
    filter.truncate(0);
    assert_eq!(filter.depth(), 0);
    assert!(!filter.might_contain_all(&[class_hash("gone")]));
    // Truncating past the current depth is a no-op.
    filter.truncate(3);
    assert_eq!(filter.depth(), 0);
}

#[test]
fn cascade_results_follow_case_folding_rules() {
    // Quirks mode folds class/id case; no-quirks mode does not. The index
    // folds unconditionally, so the matcher must still reject no-quirks
    // mismatches.
    let css = ".FOO { color: red } #BAR { background-color: blue } P { color: blue }";
    for (quirks, expect_class) in [
        (StyleQuirksMode::Quirks, true),
        (StyleQuirksMode::NoQuirks, false),
    ] {
        let mut doc = TestDoc::new();
        doc.quirks_mode = quirks;
        let style = doc.push_element(0, "style", None);
        doc.push_text(style, css);
        let span = doc.push_element_with_attrs(0, "span", None, &[("class", "foo"), ("id", "bar")]);
        let p = doc.push_element(0, "p", None);
        let tree = crate::ruletree::build_rule_tree(&doc);
        let result = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            result.computed[span].color == RED,
            expect_class,
            "{quirks:?}"
        );
        assert_eq!(
            result.computed[span].background_color == BLUE,
            expect_class,
            "{quirks:?}"
        );
        // Type selectors are ASCII case-insensitive in every mode.
        assert_eq!(result.computed[p].color, BLUE);
    }
}

#[test]
fn deep_descendant_chain_still_matches() {
    let mut doc = TestDoc::new();
    let style = doc.push_element(0, "style", None);
    doc.push_text(
        style,
        "#top .mid p { color: red } .absent p { color: blue }",
    );
    let top = doc.push_element_with_attrs(0, "div", None, &[("id", "top")]);
    let mut parent = top;
    for depth in 0..40 {
        let class = if depth == 20 { "mid" } else { "filler" };
        parent = doc.push_element_with_attrs(parent, "div", None, &[("class", class)]);
    }
    let p = doc.push_element(parent, "p", None);
    let tree = crate::ruletree::build_rule_tree(&doc);
    let result = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(result.computed[p].color, RED);
}

#[test]
fn compound_with_several_simple_selectors_uses_the_strongest_key() {
    let tree = rule_tree("p.x#y { color: red } p.x { color: red } .x { color: red }");
    let rules = sorted_rules(&tree);
    let index = RuleIndex::new(rules.iter().copied());
    assert_eq!(index.by_id.len(), 1);
    assert_eq!(index.by_class.len(), 1);
    assert_eq!(index.by_class.values().next().map(Vec::len), Some(2));
    assert!(index.by_tag.is_empty());
    assert!(index.universal.is_empty());
}

#[test]
fn shorthands_are_expanded_once_per_index() {
    let tree = rule_tree("p { margin: 1px }");
    let rules = sorted_rules(&tree);
    let index = RuleIndex::new(rules.iter().copied());
    let rule = index.rule(0);
    assert_eq!(rule.declarations.len(), 4);
    assert!(rule.has_element_selector);
    assert!(!rule.has_pseudo_selector);
}
