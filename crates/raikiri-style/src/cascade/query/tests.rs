use crate::test_dom::TestDoc;
use crate::{SelectorQuery, StyleDom, StyleNodeId};

fn doc() -> (TestDoc, StyleNodeId, StyleNodeId) {
    // <div class="a"><p id="x"></p></div>, `div` being the document element
    // (its parent is the Document node itself, so it has no element
    // ancestors).
    let mut dom = TestDoc::new();
    let div = dom.push_element(0, "div", None);
    dom.set_attr(div, "class", "a");
    let p = dom.push_element(div, "p", None);
    dom.set_attr(p, "id", "x");
    (
        dom,
        StyleNodeId::new(div as u64),
        StyleNodeId::new(p as u64),
    )
}

#[test]
fn matches_descendant_combinator_with_supplied_ancestors() {
    let (dom, div, p) = doc();
    let query = SelectorQuery::parse(".a > #x").unwrap();
    assert!(query.matches(&dom, p, &[div]));
    assert!(!query.matches(&dom, div, &[]));
}

#[test]
fn selector_list_matches_when_any_member_matches() {
    let (dom, div, p) = doc();
    let query = SelectorQuery::parse("span, p").unwrap();
    assert!(query.matches(&dom, p, &[div]));
}

#[test]
fn invalid_selector_is_an_error() {
    assert!(SelectorQuery::parse("p[").is_err());
    assert!(SelectorQuery::parse("").is_err());
}

#[test]
fn root_pseudo_class_matches_the_document_element_only() {
    let (dom, div, p) = doc();
    let query = SelectorQuery::parse(":root").unwrap();
    assert!(query.matches(&dom, div, &[]));
    assert!(!query.matches(&dom, p, &[div]));
}

#[test]
fn missing_ancestor_breaks_a_child_combinator_match() {
    let (dom, _div, p) = doc();
    let query = SelectorQuery::parse(".a > #x").unwrap();
    // `p`'s real parent (`div`) is omitted from `ancestors`, so the `>`
    // combinator has no candidate to test `.a` against — this fails for a
    // different reason than `matches_descendant_combinator_with_supplied_ancestors`'s
    // negative case, which fails on the rightmost compound alone.
    assert!(!query.matches(&dom, p, &[]));
}

#[test]
fn non_element_id_never_matches() {
    let (dom, _div, _p) = doc();
    let query = SelectorQuery::parse("*").unwrap();
    // Node 0 is the Document node itself, not an element.
    assert!(!query.matches(&dom, StyleNodeId::new(0), &[]));
}

#[test]
fn unknown_id_never_matches() {
    let (dom, _div, _p) = doc();
    let query = SelectorQuery::parse("*").unwrap();
    // One past the arena's last valid index — `dom.node()` returns `None`
    // for it, distinct from `non_element_id_never_matches`'s in-range but
    // non-element id.
    assert!(!query.matches(&dom, StyleNodeId::new(dom.node_count() as u64), &[]));
}

#[test]
fn scope_pseudo_class_matches_only_the_bound_scope_element() {
    let (dom, div, p) = doc();
    let query = SelectorQuery::parse(":scope").unwrap();
    assert!(query.matches_scoped(&dom, p, &[div], Some(p)));
    assert!(!query.matches_scoped(&dom, div, &[], Some(p)));
}

#[test]
fn scope_pseudo_class_combines_with_a_combinator() {
    let (dom, div, p) = doc();
    let query = SelectorQuery::parse(":scope > p").unwrap();
    assert!(query.matches_scoped(&dom, p, &[div], Some(div)));
    // `div` is bound as scope, but `div` itself is not a `p` -- `:scope`
    // matching `div` doesn't make `div` match the whole `:scope > p` selector.
    assert!(!query.matches_scoped(&dom, div, &[], Some(div)));
}

#[test]
fn scope_pseudo_class_falls_back_to_root_semantics_with_no_bound_scope() {
    let (dom, div, p) = doc();
    let query = SelectorQuery::parse(":scope").unwrap();
    // [`SelectorQuery::matches`] never binds a scope element -- `:scope`
    // then behaves exactly like `:root` (matches only the document element).
    assert!(query.matches(&dom, div, &[]));
    assert!(!query.matches(&dom, p, &[div]));
    // `matches_scoped` with an explicit `None` is the same call `matches`
    // makes internally.
    assert!(query.matches_scoped(&dom, div, &[], None));
    assert!(!query.matches_scoped(&dom, p, &[div], None));
}
