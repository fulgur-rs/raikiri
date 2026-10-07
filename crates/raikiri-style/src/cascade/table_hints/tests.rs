use super::*;
use crate::cascade::cascade;
use crate::property::BorderCollapseValue;
use crate::resolve::ComputedLength;
use crate::ruletree::build_rule_tree;
use crate::test_dom::TestDoc;

#[test]
fn parse_non_negative_integer_follows_the_html_integer_rules() {
    assert_eq!(parse_non_negative_integer("0"), Some(0));
    assert_eq!(parse_non_negative_integer("  12px"), Some(12));
    assert_eq!(parse_non_negative_integer("+3"), Some(3));
    assert_eq!(parse_non_negative_integer("-0"), Some(0));
    assert_eq!(parse_non_negative_integer("7.9"), Some(7));
    assert_eq!(parse_non_negative_integer("99999999999"), Some(u32::MAX));
    assert_eq!(parse_non_negative_integer("-1"), None);
    assert_eq!(parse_non_negative_integer(""), None);
    assert_eq!(parse_non_negative_integer("x1"), None);
    assert_eq!(parse_non_negative_integer("+"), None);
}

#[test]
fn cellspacing_maps_to_border_spacing() {
    let mut doc = TestDoc::new();
    let zero = doc.push_element_with_attrs(0, "table", None, &[("cellspacing", "0")]);
    let five = doc.push_element_with_attrs(0, "table", None, &[("cellspacing", " 5")]);
    let invalid = doc.push_element_with_attrs(0, "table", None, &[("cellspacing", "-3")]);
    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    let spacing = |id: usize| {
        let bs = &r.computed[id].border_spacing;
        (bs.horizontal, bs.vertical)
    };
    assert_eq!(spacing(zero), (ComputedLength(0.0), ComputedLength(0.0)));
    assert_eq!(spacing(five), (ComputedLength(5.0), ComputedLength(5.0)));
    // No hint: the initial value stays.
    assert_eq!(spacing(invalid), (ComputedLength(0.0), ComputedLength(0.0)));
}

#[test]
fn author_border_spacing_beats_cellspacing() {
    let mut doc = TestDoc::new();
    let table = doc.push_element_with_attrs(
        0,
        "table",
        Some("border-spacing: 7px"),
        &[("cellspacing", "0")],
    );
    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(
        r.computed[table].border_spacing.horizontal,
        ComputedLength(7.0)
    );
}

#[test]
fn rules_attribute_collapses_borders_and_hides_the_table_border() {
    let mut doc = TestDoc::new();
    let groups = doc.push_element_with_attrs(0, "table", None, &[("rules", "GROUPS")]);
    let unknown = doc.push_element_with_attrs(0, "table", None, &[("rules", "some")]);
    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    let cv = &r.computed[groups];
    assert_eq!(cv.border_collapse, BorderCollapseValue::Collapse);
    for side in [
        cv.border.top.style,
        cv.border.right.style,
        cv.border.bottom.style,
        cv.border.left.style,
    ] {
        assert_eq!(side, BorderStyle::Hidden);
    }
    assert_eq!(
        r.computed[unknown].border_collapse,
        BorderCollapseValue::Separate
    );
}

#[test]
fn table_hints_apply_only_to_html_table() {
    let mut doc = TestDoc::new();
    let div = doc.push_element_with_attrs(0, "div", None, &[("cellspacing", "4")]);
    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(
        r.computed[div].border_spacing.horizontal,
        ComputedLength(0.0)
    );
}
