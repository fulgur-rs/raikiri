use crate::test_dom::TestDoc;
use crate::{SelectorQuery, StyleNodeId};

fn doc() -> (TestDoc, StyleNodeId, StyleNodeId) {
    // <div class="a"><p id="x"></p></div>
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
    assert!(query.matches(&dom, p, &[StyleNodeId::new(0), div]));
    assert!(!query.matches(&dom, div, &[StyleNodeId::new(0)]));
}

#[test]
fn selector_list_matches_when_any_member_matches() {
    let (dom, div, p) = doc();
    let query = SelectorQuery::parse("span, p").unwrap();
    assert!(query.matches(&dom, p, &[StyleNodeId::new(0), div]));
}

#[test]
fn invalid_selector_is_an_error() {
    assert!(SelectorQuery::parse("p[").is_err());
    assert!(SelectorQuery::parse("").is_err());
}
