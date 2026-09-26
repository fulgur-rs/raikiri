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

/// `:scope` inside `:not(...)` (CSS Selectors L4 §14.3.3, §4.9): the bound
/// scope element is threaded straight through `:not()`'s inner selector
/// list, the same as every other selector combinator threads it through.
#[test]
fn scope_pseudo_class_inside_negation() {
    let (dom, div, p) = doc();
    let query = SelectorQuery::parse(":not(:scope)").unwrap();
    // `div` is the bound scope -- `:scope` matches it, so `:not(:scope)`
    // must not.
    assert!(!query.matches_scoped(&dom, div, &[], Some(div)));
    // `p` is not the bound scope.
    assert!(query.matches_scoped(&dom, p, &[div], Some(div)));
}

/// `:scope` inside `:has(...)`'s relative selector list refers to the
/// *outer* bound scope element, never to `:has()`'s own anchor (the element
/// being tested) -- confirmed against WPT
/// `css/selectors/has-argument-with-explicit-scope.html`, which relies on
/// exactly this to make `:has(:scope)` match nothing among a scope
/// element's own descendants (none of them can equal the scope element
/// itself). This fixture instead anchors `:has(:scope)` on an *ancestor* of
/// the bound scope, which the two possible meanings of `:scope` tell apart:
/// under "outer scope" semantics the scope element is a real descendant of
/// that ancestor and the match succeeds; if `:scope` were instead rebound to
/// the `:has()` anchor, the anchor could never be its own descendant and the
/// match would incorrectly fail.
#[test]
fn scope_pseudo_class_inside_has_refers_to_the_outer_scope_not_the_has_anchor() {
    let mut dom = TestDoc::new();
    let outer = dom.push_element(0, "div", None);
    let scope_el = dom.push_element(outer, "div", None);
    dom.push_element(scope_el, "p", None);

    let query = SelectorQuery::parse(":has(:scope)").unwrap();
    let scope_el = StyleNodeId::new(scope_el as u64);
    let outer = StyleNodeId::new(outer as u64);
    assert!(query.matches_scoped(&dom, outer, &[], Some(scope_el)));
    // The bound scope element itself has no descendant equal to itself, so
    // `:has()` never matches on it (regardless of which meaning `:scope`
    // takes here).
    assert!(!query.matches_scoped(&dom, scope_el, &[outer], Some(scope_el)));
}

/// `:scope` inside `:nth-child(An+B of S)`'s filter selector list `S`: the
/// bound scope element is threaded through the sibling-filter match the same
/// as any other selector, so `:nth-child(1 of :scope)` matches only the one
/// sibling that both is the bound scope element and is first among siblings
/// matching `:scope` (trivially true whenever it matches at all, since a
/// single element can never equal two different siblings).
#[test]
fn scope_pseudo_class_inside_nth_child_of_selector_list() {
    let mut dom = TestDoc::new();
    let ul = dom.push_element(0, "ul", None);
    let li1 = dom.push_element(ul, "li", None);
    let li2 = dom.push_element(ul, "li", None);
    let li3 = dom.push_element(ul, "li", None);

    let query = SelectorQuery::parse(":nth-child(1 of :scope)").unwrap();
    let ancestors = &[StyleNodeId::new(ul as u64)];
    let scope = Some(StyleNodeId::new(li2 as u64));
    assert!(query.matches_scoped(&dom, StyleNodeId::new(li2 as u64), ancestors, scope));
    assert!(!query.matches_scoped(&dom, StyleNodeId::new(li1 as u64), ancestors, scope));
    assert!(!query.matches_scoped(&dom, StyleNodeId::new(li3 as u64), ancestors, scope));
}
