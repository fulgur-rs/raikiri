use super::*;
use crate::test_dom::TestDoc;

#[test]
fn all_revert_layer_keeps_first_letter_rollback_markers() {
    let mut doc = TestDoc::new();
    let style = doc.push_element(0, "style", None);
    doc.push_text(style, "@layer low, high; @layer low {div::first-letter {color:red}} @layer high {div::first-letter {color:blue;all:revert-layer}}");
    let root = doc.push_element(0, "div", Some("color:black"));
    let computed = crate::cascade(&doc, &crate::build_rule_tree(&doc)).unwrap();
    let letter = computed
        .resolve_first_letter_style(StyleNodeId::new(root as u64), &computed.computed[root])
        .unwrap();
    assert_eq!(
        letter.color,
        crate::property::CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255
        }
    );
}

#[test]
fn actual_parent_metrics_custom_values_and_inapplicable_geometry_are_preserved() {
    let mut doc = TestDoc::new();
    let style = doc.push_element(0, "style", None);
    doc.push_text(
        style,
        "div::first-letter { font-size:2em; color:var(--ink); width:99px; display:none }",
    );
    let root = doc.push_element(0, "div", Some("font-size:10px;--ink:black"));
    let inner = doc.push_element(root, "span", Some("font-size:20px;--ink:red"));
    let result = crate::cascade(&doc, &crate::build_rule_tree(&doc)).unwrap();
    let cv = result
        .resolve_first_letter_style(StyleNodeId::new(root as u64), &result.computed[inner])
        .unwrap();
    assert_eq!(cv.font_size.0, 40.0);
    assert_eq!(
        cv.color,
        crate::property::CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255
        }
    );
    assert_eq!(
        cv.width,
        crate::resolve::ComputedLengthPercentageOrAuto::Auto
    );
    assert_eq!(cv.display, crate::property::DisplayValue::Inline);
    assert_eq!(cv.resolved_custom_property("--ink").as_deref(), Some("red"));
    assert_eq!(cv.local_resolved_custom_property("--ink"), None);
    assert!(
        result
            .resolve_first_letter_style(StyleNodeId::new(inner as u64), &result.computed[inner])
            .is_none()
    );
}

#[test]
fn sibling_shared_pseudo_inputs_keep_relative_font_units() {
    let mut doc = TestDoc::new();
    let style = doc.push_element(0, "style", None);
    doc.push_text(style, "div::first-letter {font-size:2em}");
    let parent = doc.push_element(0, "section", Some("font-size:10px"));
    let first = doc.push_element(parent, "div", None);
    let second = doc.push_element(parent, "div", None);
    let result = crate::cascade(&doc, &crate::build_rule_tree(&doc)).unwrap();
    for node in [first, second] {
        assert_eq!(
            result
                .resolve_first_letter_style(StyleNodeId::new(node as u64), &result.computed[node])
                .unwrap()
                .font_size
                .0,
            20.0
        );
    }
}

#[test]
fn locally_declared_custom_values_resolve_in_the_letter_environment() {
    let mut doc = TestDoc::new();
    let style = doc.push_element(0, "style", None);
    doc.push_text(style, "div::first-letter {--ink:red;color:var(--ink)}");
    let root = doc.push_element(0, "div", Some("--ink:blue"));
    let computed = crate::cascade(&doc, &crate::build_rule_tree(&doc)).unwrap();
    let letter = computed
        .resolve_first_letter_style(StyleNodeId::new(root as u64), &computed.computed[root])
        .unwrap();
    assert_eq!(
        letter.color,
        crate::property::CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255
        }
    );
    assert_eq!(
        letter.local_resolved_custom_property("--ink").as_deref(),
        Some("red")
    );
    assert_eq!(
        computed.computed[root]
            .resolved_custom_property("--ink")
            .as_deref(),
        Some("blue")
    );
}
