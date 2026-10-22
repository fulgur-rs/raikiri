use super::*;
use crate::ruletree::{Origin, RuleTree, RuleTreeLimits};
use cssparser::ToCss;

fn assert_equivalent(actual: &RuleTree, expected: &RuleTree) {
    assert_eq!(actual.rules, expected.rules);
    assert_eq!(actual.next_style_order, expected.next_style_order);
    assert_eq!(actual.budget.counts(), expected.budget.counts());
    assert_eq!(
        actual.budget.selector_revalidation,
        expected.budget.selector_revalidation
    );
    assert_eq!(
        format!("{:?}", actual.budget.error()),
        format!("{:?}", expected.budget.error())
    );
    assert_eq!(
        actual.opaque_budget.body_bytes,
        expected.opaque_budget.body_bytes
    );
    assert_eq!(actual.opaque_budget.nodes, expected.opaque_budget.nodes);
    assert_eq!(actual.style_rules.len(), expected.style_rules.len());
    for (a, b) in actual.style_rules.iter().zip(&expected.style_rules) {
        assert_eq!(a.selectors.to_css_string(), b.selectors.to_css_string());
        assert_eq!(a.declarations, b.declarations);
        assert_eq!(
            (a.source_order, a.origin, a.layer),
            (b.source_order, b.origin, b.layer)
        );
    }
}

#[test]
fn prepared_seed_preserves_dropped_rule_budget_and_subsequent_parsing() {
    let source = "a { color:red } :unknown { color:green } p { margin:2px }";
    let seed = PreparedStylesheet::new(source, Origin::UserAgent).unwrap();
    let mut raw = RuleTree::empty();
    raw.add_stylesheet(source, Origin::UserAgent);
    for _ in 0..2 {
        let mut actual = seed.to_rule_tree();
        assert_equivalent(&actual, &raw);
        actual.add_stylesheet("p {color:blue}", Origin::Author);
        let mut expected = RuleTree::empty();
        expected.add_stylesheet(source, Origin::UserAgent);
        expected.add_stylesheet("p {color:blue}", Origin::Author);
        assert_equivalent(&actual, &expected);
    }
    assert_equivalent(&seed.to_rule_tree(), &raw);
}

#[test]
fn prepared_seed_retains_exact_budget_when_limits_are_changed_later() {
    let source = "a {color:red} :unknown {color:blue}";
    let seed = PreparedStylesheet::new(source, Origin::UserAgent).unwrap();
    for field in 0..3 {
        for limit in 0..8 {
            let mut limits = RuleTreeLimits::default();
            match field {
                0 => limits.max_rules = Some(limit),
                1 => limits.max_selectors = Some(limit),
                _ => limits.max_declarations = Some(limit),
            }
            let mut raw = RuleTree::empty();
            raw.add_stylesheet(source, Origin::UserAgent);
            let actual = seed.to_rule_tree().with_limits(limits);
            assert_equivalent(&actual, &raw.with_limits(limits));
        }
    }
}

#[test]
fn prepared_seed_rejects_state_beyond_plain_style_rules() {
    for source in [
        "@page {margin:1px}",
        "@media print {p {color:red}}",
        "@supports (color:red) {p {color:red}}",
        "@namespace svg 'http://www.w3.org/2000/svg'; svg|rect {color:red}",
        "@layer first {p {color:red}}",
        "@font-face {font-family:Test;src:url(test.ttf)}",
        "@counter-style demo {system:cyclic;symbols:'x'}",
        "::highlight(demo) {background-color:red}",
    ] {
        assert!(
            PreparedStylesheet::new(source, Origin::UserAgent).is_none(),
            "{source}"
        );
    }
}
