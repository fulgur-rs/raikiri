use super::*;
use crate::cascade::cascade;
use crate::ruletree::build_rule_tree;
use crate::test_dom::TestDoc;

fn list_item(value: i32) -> Vec<(SmolStr, i32)> {
    vec![(SmolStr::new_static(LIST_ITEM_COUNTER), value)]
}

#[test]
fn ol_start_resets_the_list_item_counter_one_below_start() {
    for (start, expected) in [
        ("5", list_item(4)),
        (" -2", list_item(-3)),
        ("0", list_item(-1)),
        ("-99999999999", list_item(i32::MIN)),
        ("x", Vec::new()),
    ] {
        let mut doc = TestDoc::new();
        let ol = doc.push_element_with_attrs(0, "ol", None, &[("start", start)]);
        let result = cascade(&doc, &build_rule_tree(&doc)).unwrap();
        assert_eq!(*result.computed[ol].counter_reset, expected, "{start:?}");
    }
}

#[test]
fn ol_start_hint_yields_to_reversed_and_author_css() {
    let mut doc = TestDoc::new();
    let reversed =
        doc.push_element_with_attrs(0, "ol", None, &[("start", "5"), ("reversed", "reversed")]);
    let authored = doc.push_element_with_attrs(
        0,
        "ol",
        Some("counter-reset: list-item 7"),
        &[("start", "5")],
    );
    let result = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    assert!(result.computed[reversed].counter_reset.is_empty());
    assert_eq!(*result.computed[authored].counter_reset, list_item(7));
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
