use super::*;
use crate::cascade::cascade;
use crate::ruletree::build_rule_tree;
use crate::test_dom::TestDoc;

fn list_item(value: i32) -> Vec<(SmolStr, i32)> {
    vec![(SmolStr::new_static(LIST_ITEM_COUNTER), value)]
}

fn list_item_reset(value: CounterResetValue) -> Vec<(SmolStr, CounterResetValue)> {
    vec![(SmolStr::new_static(LIST_ITEM_COUNTER), value)]
}

#[test]
fn ol_start_resets_the_list_item_counter_one_below_start() {
    for (start, expected) in [
        ("5", list_item_reset(CounterResetValue::Integer(4))),
        (" -2", list_item_reset(CounterResetValue::Integer(-3))),
        ("0", list_item_reset(CounterResetValue::Integer(-1))),
        (
            "-99999999999",
            list_item_reset(CounterResetValue::Integer(i32::MIN)),
        ),
        ("x", Vec::new()),
    ] {
        let mut doc = TestDoc::new();
        let ol = doc.push_element_with_attrs(0, "ol", None, &[("start", start)]);
        let result = cascade(&doc, &build_rule_tree(&doc)).unwrap();
        assert_eq!(*result.computed[ol].counter_reset, expected, "{start:?}");
    }
}

#[test]
fn ol_reversed_resets_a_reversed_list_item_counter_one_above_start() {
    let mut doc = TestDoc::new();
    let reversed = doc.push_element_with_attrs(0, "ol", None, &[("reversed", "reversed")]);
    let started =
        doc.push_element_with_attrs(0, "ol", None, &[("start", "5"), ("reversed", "reversed")]);
    let saturated = doc.push_element_with_attrs(
        0,
        "ol",
        None,
        &[("start", "99999999999"), ("reversed", "reversed")],
    );
    let invalid =
        doc.push_element_with_attrs(0, "ol", None, &[("start", "x"), ("reversed", "reversed")]);
    let result = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    for (node, expected) in [
        (reversed, CounterResetValue::Reversed(None)),
        (started, CounterResetValue::Reversed(Some(6))),
        (saturated, CounterResetValue::Reversed(Some(i32::MAX))),
        (invalid, CounterResetValue::Reversed(None)),
    ] {
        assert_eq!(
            *result.computed[node].counter_reset,
            list_item_reset(expected)
        );
    }
}

#[test]
fn ol_start_hint_yields_to_author_css() {
    let mut doc = TestDoc::new();
    let authored = doc.push_element_with_attrs(
        0,
        "ol",
        Some("counter-reset: list-item 7"),
        &[("start", "5"), ("reversed", "reversed")],
    );
    let result = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    assert_eq!(
        *result.computed[authored].counter_reset,
        list_item_reset(CounterResetValue::Integer(7))
    );
}

#[test]
fn li_value_sets_the_list_item_counter() {
    let mut doc = TestDoc::new();
    let ol = doc.push_element(0, "ol", None);
    let valued = doc.push_element_with_attrs(ol, "li", None, &[("value", "10")]);
    let invalid = doc.push_element_with_attrs(ol, "li", None, &[("value", "ten")]);
    let foreign =
        doc.push_element_with_namespace(ol, "li", "http://www.w3.org/2000/svg", &[("value", "3")]);
    let div = doc.push_element_with_attrs(ol, "div", None, &[("value", "3")]);
    let result = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    assert_eq!(*result.computed[valued].counter_set, list_item(10));
    for node in [invalid, foreign, div] {
        assert!(result.computed[node].counter_set.is_empty());
    }
}

#[test]
fn parse_integer_follows_the_html_integer_rules() {
    assert_eq!(parse_integer("0"), Some(0));
    assert_eq!(parse_integer("  12px"), Some(12));
    assert_eq!(parse_integer("+3"), Some(3));
    assert_eq!(parse_integer("-7.9"), Some(-7));
    assert_eq!(parse_integer("99999999999"), Some(i32::MAX));
    assert_eq!(parse_integer("-2147483648"), Some(i32::MIN));
    assert_eq!(parse_integer(""), None);
    assert_eq!(parse_integer("-"), None);
    assert_eq!(parse_integer("x1"), None);
}
