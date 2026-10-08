use super::*;
use crate::cascade::cascade;
use crate::property::BorderCollapseValue;
use crate::resolve::ComputedLength;
use crate::ruletree::build_rule_tree;
use crate::test_dom::TestDoc;

#[test]
fn col_width_attribute_maps_html_dimensions_below_author_css() {
    use crate::resolve::ComputedLengthPercentageOrAuto as Width;
    for (attribute, authored, expected) in [
        ("40", None, Width::Px(40.0)),
        ("50%", None, Width::Percent(50.0)),
        (" 10.5px", None, Width::Px(10.5)),
        ("0", None, Width::Px(0.0)),
        ("-1", None, Width::Auto),
        ("", None, Width::Auto),
        ("40", Some("width:12px"), Width::Px(12.0)),
        ("40", Some("width:auto"), Width::Auto),
    ] {
        let mut doc = TestDoc::new();
        let column = doc.push_element_with_attrs(0, "col", authored, &[("width", attribute)]);
        let result = cascade(&doc, &build_rule_tree(&doc)).unwrap();
        assert_eq!(
            result.computed[column].width, expected,
            "{attribute:?} / {authored:?}"
        );
    }
}

#[test]
fn col_width_hint_is_scoped_to_html_columns() {
    let mut doc = TestDoc::new();
    let foreign =
        doc.push_element_with_namespace(0, "col", "http://www.w3.org/2000/svg", &[("width", "40")]);
    let div = doc.push_element_with_attrs(0, "div", None, &[("width", "40")]);
    let result = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    for node in [foreign, div] {
        assert_eq!(
            result.computed[node].width,
            crate::resolve::ComputedLengthPercentageOrAuto::Auto
        );
    }
}

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

#[test]
fn cellpadding_uses_the_nearest_html_table_and_preserves_author_padding() {
    use crate::resolve::ComputedLengthPercentage as Padding;
    for (attribute, expected) in [("0", 0.0), (" 5px", 5.0), ("-1", 0.0), ("", 0.0)] {
        let mut doc = TestDoc::new();
        let table = doc.push_element_with_attrs(0, "TABLE", None, &[("cellpadding", attribute)]);
        let row = doc.push_element(table, "tr", None);
        let cell = doc.push_element(row, "TD", None);
        let header = doc.push_element(row, "th", Some("padding-left:9px"));
        let nested = doc.push_element(cell, "table", None);
        let nested_cell = doc.push_element(nested, "td", None);
        let padded_nested =
            doc.push_element_with_attrs(cell, "table", None, &[("cellpadding", "3")]);
        let padded_cell = doc.push_element(padded_nested, "td", None);
        let foreign = doc.push_element_with_namespace(row, "td", "http://www.w3.org/2000/svg", &[]);
        let unrelated = doc.push_element(row, "div", None);
        let orphan = doc.push_element(0, "td", None);
        let result = cascade(&doc, &build_rule_tree(&doc)).unwrap();
        let cv = &result.computed[cell];
        for side in [
            cv.padding.top,
            cv.padding.right,
            cv.padding.bottom,
            cv.padding.left,
        ] {
            assert_eq!(side, Padding::Px(expected), "{attribute:?}");
        }
        assert_eq!(result.computed[header].padding.top, Padding::Px(expected));
        assert_eq!(result.computed[header].padding.left, Padding::Px(9.0));
        assert_eq!(result.computed[padded_cell].padding.left, Padding::Px(3.0));
        for node in [nested_cell, foreign, unrelated, orphan] {
            assert_eq!(result.computed[node].padding.left, Padding::Px(0.0));
        }
    }
}
