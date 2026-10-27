use super::super::*;
use super::class_index_probe_tests::fixture;
use raikiri_style::{SelectorQuery, StyleNodeId, cascade};

const RED: raikiri_style::CssColor = raikiri_style::CssColor {
    r: 255,
    g: 0,
    b: 0,
    a: 255,
};

#[test]
fn native_class_index_reflects_attribute_replacement_and_removal() {
    let (mut doc, tree, ids) = fixture(1, 128, 32, false);
    let id = ids[0];
    assert_eq!(cascade(&doc, &tree).unwrap().computed[id].color, RED);
    let query =
        SelectorQuery::parse(&(0..128).map(|i| format!(".c{i}")).collect::<String>()).unwrap();
    assert!(query.matches(&doc, StyleNodeId::new(id as u64), &[]));
    doc.set_element_attributes(id, vec![("class".into(), "c0".into())]);
    assert!(!query.matches(&doc, StyleNodeId::new(id as u64), &[]));
    assert_ne!(cascade(&doc, &tree).unwrap().computed[id].color, RED);
    doc.set_element_attributes(id, vec![]);
    let element = ElementRef {
        node: &doc.nodes[id],
    };
    assert_eq!(
        StyleElement::class_attribute_for_matching(&element),
        Some("")
    );
}

#[test]
fn native_class_index_bulk_failure_is_sticky_on_reusable_public_budget() {
    let (doc, _, ids) = fixture(1, 128, 0, false);
    let id = StyleNodeId::new(ids[0] as u64);
    let query = SelectorQuery::parse(&".c127".repeat(16)).unwrap();
    let budget = raikiri_style::SelectorWorkBudget::new(Some(16));
    let matcher = query.bounded_matcher(&doc, None, &budget);
    let first = matcher.try_matches(id, &[]).unwrap_err();
    assert!(matches!(
        first,
        raikiri_style::CascadeError::LimitExceeded {
            kind: raikiri_style::CascadeLimitKind::SelectorWork,
            ..
        }
    ));
    assert_eq!(budget.count(), 10);
    assert_eq!(
        matcher.try_matches(id, &[]).unwrap_err().to_string(),
        first.to_string()
    );
    let cheap = SelectorQuery::parse("*").unwrap();
    assert_eq!(
        cheap
            .bounded_matcher(&doc, None, &budget)
            .try_matches(id, &[])
            .unwrap_err()
            .to_string(),
        first.to_string()
    );
    assert_eq!(budget.count(), 10);
}
