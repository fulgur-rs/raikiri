use super::*;
use crate::test_dom::TestDoc;

fn filter(source: &str) -> NthOfSelectorData<RaikiriSelectorImpl> {
    let list = crate::parse_selector_list(source).unwrap();
    match list.slice()[0].iter_raw_match_order().next().unwrap() {
        selectors::parser::Component::NthOf(data) => data.clone(),
        _ => panic!("expected nth-of component"),
    }
}

fn fixture() -> (TestDoc, StyleNodeId, StyleNodeId) {
    let mut doc = TestDoc::new();
    let parent = doc.push_element(0, "div", None);
    let mut first = 0;
    for i in 0..16 {
        let child = doc.push_element(parent, "p", None);
        doc.set_attr(child, "class", "visible");
        if i == 0 {
            first = child;
        }
    }
    (
        doc,
        StyleNodeId::new(parent as u64),
        StyleNodeId::new(first as u64),
    )
}

#[test]
fn retained_filter_identity_survives_source_selector_drop() {
    use cssparser::ToCss;
    let (doc, parent, child) = fixture();
    let caches = MatchCaches::default();
    let ctx = MatchContext::new(&doc, StyleQuirksMode::NoQuirks, None, false, &caches);
    let original = filter(":nth-child(n of .visible)");
    let identity = original.selectors().as_ptr() as usize;
    assert_eq!(
        position(ctx, parent, child, &original, &[parent]),
        Some((1, 16, 16))
    );
    drop(original);
    {
        let cache = caches.filtered_nth.borrow();
        let retained = cache.entries.values().next().unwrap();
        assert_eq!(retained._filter.selectors().as_ptr() as usize, identity);
        assert_eq!(retained._filter.selectors()[0].to_css_string(), ".visible");
    }
    let other = filter(":nth-child(n of .absent)");
    assert_eq!(
        position(ctx, parent, child, &other, &[parent]),
        Some((0, 1, 0))
    );
    assert_eq!(caches.filtered_nth.borrow().entries.len(), 2);
}

#[test]
fn recursive_construction_rechecks_both_budget_limits() {
    for (max_entries, max_children) in [(1, 32), (2, 16)] {
        let (doc, parent, child) = fixture();
        let caches = MatchCaches::default();
        *caches.filtered_nth.borrow_mut() =
            FilteredNthCache::with_limits(max_entries, max_children);
        let ctx = MatchContext::new(&doc, StyleQuirksMode::NoQuirks, None, false, &caches);
        let data = filter(":nth-child(n of :nth-child(2n of .visible))");
        assert_eq!(
            position(ctx, parent, child, &data, &[parent]),
            Some((0, 9, 8))
        );
        let cache = caches.filtered_nth.borrow();
        assert_eq!(cache.entries.len(), 1);
        assert_eq!(cache.charged_children, 16);
        assert!(cache.charged_children <= cache.max_children);
    }
}
