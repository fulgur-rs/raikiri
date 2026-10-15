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
fn vertical_elements_map_the_block_axis_to_the_physical_x_axis() {
    assert_eq!(
        margins(&format!("writing-mode: vertical-rl; {ALL_FOUR}")),
        sides(1.0, 2.0, 3.0, 4.0)
    );
    assert_eq!(
        margins(&format!("writing-mode: vertical-lr; {ALL_FOUR}")),
        sides(1.0, 4.0, 3.0, 2.0)
    );
    assert_eq!(
        margins(&format!(
            "writing-mode: vertical-rl; direction: rtl; {ALL_FOUR}"
        )),
        sides(3.0, 2.0, 1.0, 4.0)
    );
}

#[test]
fn upright_text_maps_vertical_inline_sides_as_ltr() {
    assert_eq!(
        margins(&format!(
            "writing-mode: vertical-rl; direction: rtl; text-orientation: upright; {ALL_FOUR}"
        )),
        sides(1.0, 2.0, 3.0, 4.0)
    );
}

#[test]
fn sideways_elements_map_like_horizontal_ones() {
    // Block layout stacks the boxes of the sideways modes on horizontal axes.
    assert_eq!(
        margins(&format!(
            "writing-mode: sideways-lr; direction: rtl; {ALL_FOUR}"
        )),
        sides(2.0, 1.0, 4.0, 3.0)
    );
}

#[test]
fn the_writing_mode_and_orientation_are_inherited_from_the_parent() {
    let mut doc = TestDoc::new();
    let parent = doc.push_element(
        0,
        "div",
        Some("writing-mode: vertical-rl; direction: rtl; text-orientation: upright"),
    );
    let child = doc.push_element(
        parent,
        "p",
        Some("margin: 0; margin-block-start: 5px; margin-inline-start: 6px"),
    );
    let tree = build_rule_tree(&doc);
    let result = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(result.computed[child].margin, sides(6.0, 5.0, 0.0, 0.0));
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

#[test]
fn page_var_preferred_sizes_follow_the_page_writing_mode() {
    let page_size = |writing_mode: &str| {
        let mut doc = TestDoc::new();
        let html = doc.push_element(0, "html", Some(writing_mode));
        let style = doc.push_element(html, "style", None);
        doc.push_text(
            style,
            "@page { --i: 30px; --b: 40px; inline-size: var(--i); block-size: var(--b) }",
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
        format!("{:?}", declarations.get(&PropertyKey::Width))
    };
    assert!(page_size("writing-mode: horizontal-tb").contains("30.0"));
    assert!(page_size("writing-mode: vertical-lr").contains("40.0"));
}

fn sheet_margins(css: &str) -> Sides<ComputedLengthPercentageOrAuto> {
    let mut doc = TestDoc::new();
    let html = doc.push_element(0, "html", None);
    let style = doc.push_element(html, "style", None);
    doc.push_text(style, css);
    let div = doc.push_element(html, "div", None);
    let tree = build_rule_tree(&doc);
    let result = cascade(&doc, &tree).expect("cascade Ok");
    result.computed[div].margin
}

#[test]
fn a_rolled_back_logical_margin_falls_back_to_the_physical_side_it_maps_to() {
    let css = |value: &str| {
        format!(
            "@layer low, high; \
             @layer low {{ div {{ margin: 0; margin-right: 7px; margin-inline-end: 5px }} }} \
             @layer high {{ div {{ direction: rtl; margin-inline-start: {value} }} }}"
        )
    };
    // The low layer's `margin-inline-end` maps to the left side under `rtl`.
    assert_eq!(
        sheet_margins(&css("revert-layer")),
        sides(0.0, 7.0, 0.0, 5.0)
    );
    // A surviving flow-relative declaration moves to its own side.
    assert_eq!(
        sheet_margins(
            "@layer low, high; \
             @layer low { div { margin: 0; --m: 3px 4px; margin-inline: var(--m) } } \
             @layer high { div { direction: rtl; margin-right: revert-layer } }"
        ),
        sides(0.0, 3.0, 0.0, 4.0)
    );
}

/// The margins of a vertical `p` styled `ALL_FOUR` under the chain of
/// ancestors styled `ancestors`, outermost first.
fn nested_margins(ancestors: &[&str]) -> Sides<ComputedLengthPercentageOrAuto> {
    let mut doc = TestDoc::new();
    let mut parent = 0;
    for style in ancestors {
        parent = doc.push_element(parent, "div", Some(style));
    }
    let child = doc.push_element(parent, "p", Some(ALL_FOUR));
    let tree = build_rule_tree(&doc);
    let result = cascade(&doc, &tree).expect("cascade Ok");
    result.computed[child].margin
}

#[test]
fn children_placed_on_horizontal_axes_keep_the_horizontal_mapping() {
    // Flex, grid, table and multicol containers place their children on
    // horizontal axes in every writing mode, and so does a block inside a
    // multicol container.
    for ancestors in [
        &["writing-mode: vertical-rl; display: flex"][..],
        &["writing-mode: vertical-rl; display: grid"],
        &["writing-mode: vertical-rl; display: table-cell"],
        &["writing-mode: vertical-rl; column-count: 2"],
        &[
            "writing-mode: vertical-rl; column-width: 10px",
            "display: block",
        ],
    ] {
        assert_eq!(
            nested_margins(ancestors),
            sides(2.0, 3.0, 4.0, 1.0),
            "{ancestors:?}"
        );
    }
    assert_eq!(
        nested_margins(&["writing-mode: vertical-rl; display: block"]),
        sides(1.0, 2.0, 3.0, 4.0)
    );
}

#[test]
fn upright_text_under_a_horizontal_parent_keeps_the_ltr_mapping() {
    // `text-orientation: upright` makes the used direction `ltr` in a
    // vertical writing mode, also when a flex parent maps the logical sides
    // onto horizontal axes.
    assert_eq!(
        nested_margins(&[
            "writing-mode: vertical-rl; direction: rtl; text-orientation: upright; display: flex"
        ]),
        sides(2.0, 3.0, 4.0, 1.0)
    );
    // In a horizontal writing mode `text-orientation` has no effect.
    assert_eq!(
        nested_margins(&["direction: rtl; text-orientation: upright"]),
        sides(2.0, 1.0, 4.0, 3.0)
    );
}
