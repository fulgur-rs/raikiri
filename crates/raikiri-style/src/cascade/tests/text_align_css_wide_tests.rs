//! CSS-wide keywords on `text-align` and `text-align-last`
//! (CSS Cascading 4 §7.3 "Explicit Defaulting").

use super::*;
use crate::property::{TextAlign, TextAlignLast};

/// The computed values of a parent styled `parent` and of its child `p`
/// (`id="target"`) under `sheets`, followed by `css` as an author sheet.
fn pair(parent: &str, sheets: &[(&str, Origin)], css: &str) -> (ComputedValues, ComputedValues) {
    let mut doc = TestDoc::new();
    let parent = doc.push_element(0, "div", Some(parent));
    let child = doc.push_element_with_attrs(parent, "p", None, &[("id", "target")]);
    let mut tree = RuleTree::empty();
    for &(sheet, origin) in sheets {
        tree.add_stylesheet(sheet, origin);
    }
    tree.add_stylesheet(css, Origin::Author);
    let result = cascade(&doc, &tree).unwrap();
    (
        result.computed[parent].clone(),
        result.computed[child].clone(),
    )
}

/// The `text-align` and `text-align-last` of the child of a parent that has
/// neither at its initial value.
fn child(css: &str) -> (TextAlign, TextAlignLast) {
    let (_, values) = pair("text-align: center; text-align-last: justify", &[], css);
    (values.text_align, values.text_align_last)
}

#[test]
fn initial_gives_the_initial_values() {
    assert_eq!(
        child("#target { text-align: initial; text-align-last: initial }"),
        (TextAlign::Start, TextAlignLast::Auto)
    );
}

#[test]
fn inherit_and_unset_take_the_parents_values() {
    // Both properties are inherited, so `unset` is `inherit`.
    for keyword in ["inherit", "unset"] {
        assert_eq!(
            child(&format!(
                "p {{ text-align: right; text-align-last: right }} \
                 #target {{ text-align: {keyword}; text-align-last: {keyword} }}"
            )),
            (TextAlign::Center, TextAlignLast::Justify),
            "{keyword}"
        );
    }
    // A parent value of `start` is inherited as `start`, not as the physical
    // side `match-parent` would turn it into.
    let (_, values) = pair(
        "text-align: end",
        &[],
        "#target { text-align: left; text-align: inherit }",
    );
    assert_eq!(values.text_align, TextAlign::End);
}

#[test]
fn a_keyword_overrides_by_specificity_importance_and_order() {
    assert_eq!(
        child("#target { text-align: right; text-align: initial }").0,
        TextAlign::Start
    );
    assert_eq!(
        child("#target { text-align: initial; text-align: right }").0,
        TextAlign::Right
    );
    assert_eq!(
        child("p { text-align: initial } #target { text-align: right }").0,
        TextAlign::Right
    );
    assert_eq!(
        child("p { text-align: initial !important } #target { text-align: right }").0,
        TextAlign::Start
    );
}

#[test]
fn revert_rolls_back_the_origin() {
    // The user agent's declaration is what survives.
    let (_, values) = pair(
        "text-align: right",
        &[("p { text-align: center }", Origin::UserAgent)],
        "#target { text-align: revert }",
    );
    assert_eq!(values.text_align, TextAlign::Center);
    // Nothing underneath: the property is inherited, so `unset` applies.
    let (_, values) = pair(
        "text-align: right; text-align-last: end",
        &[],
        "#target { text-align: revert; text-align-last: revert }",
    );
    assert_eq!(
        (values.text_align, values.text_align_last),
        (TextAlign::Right, TextAlignLast::End)
    );
}

#[test]
fn revert_layer_rolls_back_the_layer() {
    let values = child(
        "@layer a, b; \
         @layer a { p { text-align: left; text-align-last: right } } \
         @layer b { p { text-align: center; text-align-last: center } \
                    #target { text-align: revert-layer; text-align-last: revert-layer } }",
    );
    assert_eq!(values, (TextAlign::Left, TextAlignLast::Right));
}

#[test]
fn substituted_keywords_apply() {
    assert_eq!(
        child(
            "#target { --kw: initial; text-align: var(--kw); \
             text-align-last: var(--missing, initial) }"
        ),
        (TextAlign::Start, TextAlignLast::Auto)
    );
    assert_eq!(
        child("#target { text-align: right; text-align: var(--missing, inherit) }").0,
        TextAlign::Center
    );
}

#[test]
fn match_parent_still_resolves_against_the_parents_direction() {
    let (_, values) = pair(
        "direction: rtl; text-align: start",
        &[],
        "#target { direction: ltr; text-align: match-parent }",
    );
    assert_eq!(values.text_align, TextAlign::Right);
}
