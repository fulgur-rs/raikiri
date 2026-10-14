//! Flow-relative margin and padding longhands map through the element's own
//! writing mode and direction (CSS Logical Properties 1 §4).

use super::*;
use crate::resolve::ComputedLengthPercentageOrAuto;

fn margins(style: &str) -> Sides<ComputedLengthPercentageOrAuto> {
    let mut doc = TestDoc::new();
    let root = doc.push_element(0, "div", Some(style));
    let tree = build_rule_tree(&doc);
    let result = cascade(&doc, &tree).expect("cascade Ok");
    result.computed[root].margin
}

fn px(value: f32) -> ComputedLengthPercentageOrAuto {
    ComputedLengthPercentageOrAuto::Px(value)
}

fn sides(top: f32, right: f32, bottom: f32, left: f32) -> Sides<ComputedLengthPercentageOrAuto> {
    Sides {
        top: px(top),
        right: px(right),
        bottom: px(bottom),
        left: px(left),
    }
}

const ALL_FOUR: &str = "margin: 0; margin-inline-start: 1px; margin-block-start: 2px; \
                        margin-inline-end: 3px; margin-block-end: 4px";

#[test]
fn horizontal_ltr_maps_as_the_parser_did() {
    assert_eq!(margins(ALL_FOUR), sides(2.0, 3.0, 4.0, 1.0));
}

#[test]
fn horizontal_rtl_swaps_the_inline_sides() {
    assert_eq!(
        margins(&format!("direction: rtl; {ALL_FOUR}")),
        sides(2.0, 1.0, 4.0, 3.0)
    );
}

#[test]
fn vertical_elements_map_like_horizontal_ones_while_blocks_stack_vertically() {
    // Block layout runs in horizontal-tb, so the block axis stays vertical.
    assert_eq!(
        margins(&format!("writing-mode: vertical-rl; {ALL_FOUR}")),
        sides(2.0, 3.0, 4.0, 1.0)
    );
    assert_eq!(
        margins(&format!(
            "writing-mode: sideways-lr; direction: rtl; {ALL_FOUR}"
        )),
        sides(2.0, 1.0, 4.0, 3.0)
    );
}

#[test]
fn the_direction_is_inherited_from_the_parent() {
    let mut doc = TestDoc::new();
    let parent = doc.push_element(0, "div", Some("direction: rtl"));
    let child = doc.push_element(parent, "p", Some("margin: 0; margin-inline-start: 5px"));
    let tree = build_rule_tree(&doc);
    let result = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(result.computed[child].margin, sides(0.0, 5.0, 0.0, 0.0));
}

#[test]
fn a_var_direction_is_substituted_before_mapping() {
    assert_eq!(
        margins("--d: rtl; direction: var(--d); margin: 0; margin-inline-end: 5px"),
        sides(0.0, 0.0, 0.0, 5.0)
    );
}

#[test]
fn siblings_with_a_physical_and_a_logical_margin_do_not_share_styles() {
    let mut doc = TestDoc::new();
    let parent = doc.push_element(0, "div", Some("direction: rtl"));
    let first = doc.push_element(parent, "p", Some("margin: 0; margin-left: 5px"));
    let second = doc.push_element(parent, "p", Some("margin: 0; margin-inline-start: 5px"));
    let tree = build_rule_tree(&doc);
    let result = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(result.computed[first].margin, sides(0.0, 0.0, 0.0, 5.0));
    assert_eq!(result.computed[second].margin, sides(0.0, 5.0, 0.0, 0.0));
}

#[test]
fn shorthands_map_both_sides() {
    assert_eq!(
        margins("direction: rtl; margin: 0; margin-block: 1px 2px; margin-inline: 3px 4px"),
        sides(1.0, 3.0, 2.0, 4.0)
    );
}

#[test]
fn a_later_physical_declaration_overrides_the_side_a_logical_one_maps_to() {
    assert_eq!(
        margins("direction: rtl; margin: 0; margin-inline-start: 1px; margin-right: 2px"),
        sides(0.0, 2.0, 0.0, 0.0)
    );
    assert_eq!(
        margins("direction: rtl; margin: 0; margin-right: 2px; margin-inline-start: 1px"),
        sides(0.0, 1.0, 0.0, 0.0)
    );
}

#[test]
fn var_values_map_after_substitution() {
    assert_eq!(
        margins(
            "--a: 6px; --b: 7px 8px; direction: rtl; margin: 0; \
             margin-block-start: var(--a); margin-inline: var(--b)"
        ),
        sides(6.0, 7.0, 0.0, 8.0)
    );
}

#[test]
fn padding_maps_like_margin() {
    let mut doc = TestDoc::new();
    let root = doc.push_element(
        0,
        "div",
        Some("direction: rtl; padding-block-start: 1px; padding-inline-end: 2px"),
    );
    let tree = build_rule_tree(&doc);
    let result = cascade(&doc, &tree).expect("cascade Ok");
    let padding = &result.computed[root].padding;
    assert_eq!(padding.top, ComputedLengthPercentageOrCalc::Px(1.0));
    assert_eq!(padding.left, ComputedLengthPercentageOrCalc::Px(2.0));
    assert_eq!(padding.right, ComputedLengthPercentageOrCalc::Px(0.0));
}

#[test]
fn page_logical_margins_follow_the_root_writing_mode() {
    let mut doc = TestDoc::new();
    let html = doc.push_element(0, "html", Some("writing-mode: vertical-rl"));
    let style = doc.push_element(html, "style", None);
    doc.push_text(
        style,
        "@page { margin: 0; margin-inline-start: 1px; margin-block-start: 2px; \
         padding-block-end: 3px; inline-size: 40px; block-size: 50px }",
    );
    let tree = build_rule_tree(&doc);
    let result = cascade_with_media_context_for_page(
        &doc,
        &tree,
        &MediaContext::default(),
        &crate::page::PageContextQuery::default(),
    )
    .expect("page cascade Ok");
    let declarations = result.page.declarations();
    assert_eq!(
        declarations.get(&PropertyKey::MarginTop),
        Some(&PropertyValue::MarginTop(LengthOrAuto::Length(Length::Px(
            1.0
        ))))
    );
    assert_eq!(
        declarations.get(&PropertyKey::MarginRight),
        Some(&PropertyValue::MarginRight(LengthOrAuto::Length(
            Length::Px(2.0)
        )))
    );
    assert_eq!(
        declarations.get(&PropertyKey::MarginLeft),
        Some(&PropertyValue::MarginLeft(LengthOrAuto::Length(
            Length::Px(0.0)
        )))
    );
    assert!(matches!(
        declarations.get(&PropertyKey::PaddingLeft),
        Some(PropertyValue::PaddingLeft(_))
    ));
    assert!(matches!(
        declarations.get(&PropertyKey::Width),
        Some(PropertyValue::Width(_))
    ));
    assert!(matches!(
        declarations.get(&PropertyKey::Height),
        Some(PropertyValue::Height(_))
    ));
}

#[test]
fn page_writing_mode_overrides_the_root_one() {
    let mut doc = TestDoc::new();
    let html = doc.push_element(0, "html", None);
    let style = doc.push_element(html, "style", None);
    doc.push_text(
        style,
        "@page { writing-mode: vertical-lr; direction: rtl; margin: 0; \
         margin-inline-start: 1px; margin-block-end: var(--x, 2px) }",
    );
    let tree = build_rule_tree(&doc);
    let result = cascade_with_media_context_for_page(
        &doc,
        &tree,
        &MediaContext::default(),
        &crate::page::PageContextQuery::default(),
    )
    .expect("page cascade Ok");
    let declarations = result.page.declarations();
    assert_eq!(
        declarations.get(&PropertyKey::MarginBottom),
        Some(&PropertyValue::MarginBottom(LengthOrAuto::Length(
            Length::Px(1.0)
        )))
    );
    assert_eq!(
        declarations.get(&PropertyKey::MarginRight),
        Some(&PropertyValue::MarginRight(LengthOrAuto::Length(
            Length::Px(2.0)
        )))
    );
}
