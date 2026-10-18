//! The order margin boxes are drawn in (CSS Page 3 §3.1): by `z-index`, and
//! clockwise from the top-left corner among boxes of one level.

use super::layout_tests::boxes_for;
use super::*;

fn slots(boxes: &[MarginBox]) -> Vec<PageMarginBoxSlot> {
    boxes.iter().map(|margin_box| margin_box.slot).collect()
}

/// The slots of the boxes `rules` declares, in drawing order.
fn drawn(rules: &str) -> Vec<PageMarginBoxSlot> {
    slots(&boxes_for(&format!("@page {{ margin: 40px; {rules} }}")))
}

#[test]
fn boxes_are_drawn_clockwise_from_the_top_left_corner() {
    // Declared in the opposite order to the one they are expected in.
    let rules = [
        "left-top",
        "left-middle",
        "left-bottom",
        "bottom-left-corner",
        "bottom-left",
        "bottom-center",
        "bottom-right",
        "bottom-right-corner",
        "right-bottom",
        "right-middle",
        "right-top",
        "top-right-corner",
        "top-right",
        "top-center",
        "top-left",
        "top-left-corner",
    ]
    .map(|slot| format!("@{slot} {{ content: 'x' }}"))
    .join(" ");
    assert_eq!(
        drawn(&rules),
        [
            PageMarginBoxSlot::TopLeftCorner,
            PageMarginBoxSlot::TopLeft,
            PageMarginBoxSlot::TopCenter,
            PageMarginBoxSlot::TopRight,
            PageMarginBoxSlot::TopRightCorner,
            PageMarginBoxSlot::RightTop,
            PageMarginBoxSlot::RightMiddle,
            PageMarginBoxSlot::RightBottom,
            PageMarginBoxSlot::BottomRightCorner,
            PageMarginBoxSlot::BottomRight,
            PageMarginBoxSlot::BottomCenter,
            PageMarginBoxSlot::BottomLeft,
            PageMarginBoxSlot::BottomLeftCorner,
            PageMarginBoxSlot::LeftBottom,
            PageMarginBoxSlot::LeftMiddle,
            PageMarginBoxSlot::LeftTop,
        ]
    );
}

#[test]
fn a_higher_z_index_is_drawn_later() {
    assert_eq!(
        drawn(
            "@top-left { content: 'x'; z-index: 2 } \
             @top-center { content: 'x' } \
             @top-right { content: 'x'; z-index: 1 }"
        ),
        [
            PageMarginBoxSlot::TopCenter,
            PageMarginBoxSlot::TopRight,
            PageMarginBoxSlot::TopLeft,
        ]
    );
}

#[test]
fn boxes_of_one_z_index_keep_the_clockwise_order() {
    assert_eq!(
        drawn(
            "@left-top { content: 'x'; z-index: 3 } \
             @top-left { content: 'x'; z-index: 3 } \
             @bottom-center { content: 'x'; z-index: 3 } \
             @top-center { content: 'x'; z-index: -3 } \
             @right-middle { content: 'x'; z-index: -3 }"
        ),
        [
            PageMarginBoxSlot::TopCenter,
            PageMarginBoxSlot::RightMiddle,
            PageMarginBoxSlot::TopLeft,
            PageMarginBoxSlot::BottomCenter,
            PageMarginBoxSlot::LeftTop,
        ]
    );
}

#[test]
fn a_negative_z_index_is_drawn_before_boxes_at_level_zero() {
    let boxes = boxes_for(
        "@page { margin: 40px; \
           @top-left { content: 'x' } @top-right { content: 'x'; z-index: -1 } }",
    );
    assert_eq!(
        slots(&boxes),
        [PageMarginBoxSlot::TopRight, PageMarginBoxSlot::TopLeft]
    );
    assert_eq!(
        boxes
            .iter()
            .map(|margin_box| margin_box.z_index)
            .collect::<Vec<_>>(),
        [-1, 0]
    );
}

#[test]
fn an_auto_z_index_is_level_zero() {
    let boxes = boxes_for(
        "@page { margin: 40px; \
           @top-left { content: 'x'; z-index: auto } @top-center { content: 'x'; z-index: 0 } }",
    );
    assert_eq!(
        slots(&boxes),
        [PageMarginBoxSlot::TopLeft, PageMarginBoxSlot::TopCenter]
    );
    assert!(boxes.iter().all(|margin_box| margin_box.z_index == 0));
}
