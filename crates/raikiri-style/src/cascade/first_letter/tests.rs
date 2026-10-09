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

#[test]
fn enclosing_first_lines_preserve_relative_metrics_and_ordinary_custom_properties() {
    let mut doc = TestDoc::new();
    let sheet = doc.push_element(0, "style", None);
    doc.push_text(sheet,"body::first-line{font-size:30px;color:red;--ink:blue} div::first-line{font-size:2em;color:inherit;direction:rtl} div::first-letter{font-size:2em;color:var(--ink)}");
    let outer = doc.push_element(0, "body", Some("font-size:10px;--ink:red;direction:ltr"));
    let inner = doc.push_element(outer, "div", Some("font-size:20px;--ink:green"));
    let span = doc.push_element(inner, "span", Some("font-size:150%"));
    doc.push_text(span, "AB");
    let result = crate::cascade(&doc, &crate::build_rule_tree(&doc)).unwrap();
    let parent = result
        .first_letter_parent_with_first_lines(
            &doc,
            StyleNodeId::new(inner as u64),
            &[
                StyleNodeId::new(outer as u64),
                StyleNodeId::new(inner as u64),
            ],
            StyleNodeId::new(span as u64),
            None,
        )
        .unwrap();
    assert_eq!(parent.font_size.px(), 90.0);
    assert_eq!(
        (parent.color.r, parent.color.g, parent.color.b),
        (255, 0, 0)
    );
    assert_eq!(parent.direction, crate::property::Direction::Ltr);
    let letter = result
        .resolve_first_letter_style(StyleNodeId::new(inner as u64), &parent)
        .unwrap();
    assert_eq!(letter.font_size.px(), 180.0);
    assert_eq!(
        (letter.color.r, letter.color.g, letter.color.b),
        (0, 128, 0)
    );
}

#[test]
fn nested_first_line_resolves_its_own_variables_without_leaking_to_the_letter() {
    for letter_declarations in ["font-size:2em", "font-size:2em;color:var(--ink)"] {
        for inline_child in [false, true] {
            let mut doc = TestDoc::new();
            let sheet = doc.push_element(0, "style", None);
            doc.push_text(
                sheet,
                &format!(
                    "body::first-line{{color:red}} div::first-line{{--ink:green;color:var(--ink)}} div::first-letter{{{letter_declarations}}}"
                ),
            );
            let outer = doc.push_element(0, "body", Some("font-size:10px"));
            let inner = doc.push_element(outer, "div", Some("--ink:blue"));
            let actual = if inline_child {
                doc.push_element(inner, "span", None)
            } else {
                inner
            };
            doc.push_text(actual, "A");
            let result = crate::cascade(&doc, &crate::build_rule_tree(&doc)).unwrap();
            let parent = result
                .first_letter_parent_with_first_lines(
                    &doc,
                    StyleNodeId::new(inner as u64),
                    &[
                        StyleNodeId::new(outer as u64),
                        StyleNodeId::new(inner as u64),
                    ],
                    StyleNodeId::new(actual as u64),
                    None,
                )
                .unwrap();
            assert_eq!(
                (parent.color.r, parent.color.g, parent.color.b),
                (0, 128, 0)
            );
            assert_eq!(
                parent.resolved_custom_property("--ink").as_deref(),
                Some("blue")
            );
            let letter = result
                .resolve_first_letter_style(StyleNodeId::new(inner as u64), &parent)
                .unwrap();
            let expected = if letter_declarations.contains("color:") {
                (0, 0, 255)
            } else {
                (0, 128, 0)
            };
            assert_eq!((letter.color.r, letter.color.g, letter.color.b), expected);
            assert_eq!(letter.font_size.px(), 20.0);
            assert_eq!(
                letter.resolved_custom_property("--ink").as_deref(),
                Some("blue")
            );
        }
    }
}

#[test]
fn nested_first_line_recomputation_keeps_all_revert_layer() {
    // The first line of `div`, nested in the first line of `body`, is
    // recomputed from its own candidates; `all: revert-layer` rolls the
    // color of its layer back to the lower layer's there.
    let mut doc = TestDoc::new();
    let sheet = doc.push_element(0, "style", None);
    doc.push_text(
        sheet,
        "@layer low, high; body::first-line{font-size:30px} \
         @layer low {div::first-line{color:red}} \
         @layer high {div::first-line{color:blue;all:revert-layer}} \
         div::first-letter{font-size:2em}",
    );
    let outer = doc.push_element(0, "body", Some("font-size:10px"));
    let inner = doc.push_element(outer, "div", None);
    let span = doc.push_element(inner, "span", None);
    doc.push_text(span, "AB");
    let result = crate::cascade(&doc, &crate::build_rule_tree(&doc)).unwrap();
    let parent = result
        .first_letter_parent_with_first_lines(
            &doc,
            StyleNodeId::new(inner as u64),
            &[
                StyleNodeId::new(outer as u64),
                StyleNodeId::new(inner as u64),
            ],
            StyleNodeId::new(span as u64),
            None,
        )
        .unwrap();
    assert_eq!(
        (parent.color.r, parent.color.g, parent.color.b),
        (255, 0, 0)
    );
}
