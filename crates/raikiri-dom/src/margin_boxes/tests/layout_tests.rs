//! Margin-box geometry, decoration and generated content resolved from
//! `@page` rules, on a 400x300 page with 40px page margins.

use super::*;

const AHEM: &str = "font-family: Ahem; font-size: 10px;";

fn boxes_for(css: &str) -> Vec<MarginBox> {
    let (doc, cascade) = page_cascade_fixture(css);
    layout_page(&doc, &cascade, small_page())
}

fn boxes_on_page(css: &str, context: MarginBoxPageContext) -> Vec<MarginBox> {
    let (doc, cascade) = page_cascade_fixture(css);
    page_margin_boxes(&doc, &cascade, &cascade.page, small_page(), context)
}

fn rects(boxes: &[MarginBox]) -> Vec<(PageMarginBoxSlot, [f32; 4])> {
    boxes
        .iter()
        .map(|margin_box| {
            let rect = margin_box.rect;
            (
                margin_box.slot,
                [rect.x, rect.y, rect.width, rect.height].map(|v| (v * 100.0).round() / 100.0),
            )
        })
        .collect()
}

fn single_content(css: &str) -> String {
    let boxes = boxes_for(css);
    let [margin_box] = boxes.as_slice() else {
        panic!("one box: {boxes:?}");
    };
    margin_box.content.clone()
}

#[test]
fn auto_side_boxes_share_a_column_like_the_ac_track() {
    let boxes = boxes_for(&format!(
        "@page {{ margin: 40px; {AHEM} \
           @left-top {{ content: 'a' }} @left-middle {{ content: 'b' }} \
           @left-bottom {{ content: 'c' }} }}"
    ));
    assert_eq!(
        rects(&boxes),
        [
            (PageMarginBoxSlot::LeftTop, [0.0, 40.0, 40.0, 73.33]),
            (PageMarginBoxSlot::LeftMiddle, [0.0, 113.33, 40.0, 73.33]),
            (PageMarginBoxSlot::LeftBottom, [0.0, 186.67, 40.0, 73.33]),
        ]
    );
}

#[test]
fn auto_side_boxes_share_a_column_by_the_height_of_their_lines() {
    // The upper box has two lines and the lower one a single 10px line, so
    // they share the column in the ratio of the heights of their lines.
    let rects_with = |line_height: &str| {
        rects(&boxes_for(&format!(
            "@page {{ margin: 40px; {AHEM} \
               @left-top {{ content: 'a\\A b'; white-space: pre; {line_height} }} \
               @left-bottom {{ content: 'c' }} }}"
        )))
    };
    assert_eq!(
        rects_with("line-height: 3"),
        [
            (PageMarginBoxSlot::LeftTop, [0.0, 40.0, 40.0, 188.57]),
            (PageMarginBoxSlot::LeftBottom, [0.0, 228.57, 40.0, 31.43]),
        ]
    );
    assert_eq!(
        rects_with("line-height: 25px"),
        [
            (PageMarginBoxSlot::LeftTop, [0.0, 40.0, 40.0, 183.33]),
            (PageMarginBoxSlot::LeftBottom, [0.0, 223.33, 40.0, 36.67]),
        ]
    );
}

#[test]
fn auto_side_boxes_share_a_column_by_the_lines_their_text_wraps_into() {
    // The column is 40px wide, so each 20px word of the upper box is a line
    // of its own: 30px against the 10px of the lower box, shared in the
    // ratio of 3 to 1.
    let wrapped = boxes_for(&format!(
        "@page {{ margin: 40px; {AHEM} \
           @left-top {{ content: 'aa aa aa' }} @left-bottom {{ content: 'b' }} }}"
    ));
    assert_eq!(
        rects(&wrapped),
        [
            (PageMarginBoxSlot::LeftTop, [0.0, 40.0, 40.0, 165.0]),
            (PageMarginBoxSlot::LeftBottom, [0.0, 205.0, 40.0, 55.0]),
        ]
    );
    // 'aaaa' is exactly as wide as the column and stays on one line, so
    // both boxes are one line tall and share the column equally.
    let fitting = boxes_for(&format!(
        "@page {{ margin: 40px; {AHEM} \
           @left-top {{ content: 'aaaa' }} @left-bottom {{ content: 'b' }} }}"
    ));
    assert_eq!(
        rects(&fitting),
        [
            (PageMarginBoxSlot::LeftTop, [0.0, 40.0, 40.0, 110.0]),
            (PageMarginBoxSlot::LeftBottom, [0.0, 150.0, 40.0, 110.0]),
        ]
    );
}

#[test]
fn an_empty_box_counts_its_padding_along_horizontal_and_vertical_edges() {
    // The empty box asks for the 20px of its padding along the edge, the
    // other box for the 40px (or 10px) of its text.
    let row = boxes_for(&format!(
        "@page {{ margin: 40px; {AHEM} \
           @top-left {{ content: ''; padding: 10px }} @top-right {{ content: 'aaaa' }} }}"
    ));
    assert_eq!(
        rects(&row),
        [
            (PageMarginBoxSlot::TopLeft, [40.0, 0.0, 106.67, 40.0]),
            (PageMarginBoxSlot::TopRight, [146.67, 0.0, 213.33, 40.0]),
        ]
    );
    let column = boxes_for(&format!(
        "@page {{ margin: 40px; {AHEM} \
           @left-top {{ content: ''; padding: 10px }} @left-bottom {{ content: 'a' }} }}"
    ));
    assert_eq!(
        rects(&column),
        [
            (PageMarginBoxSlot::LeftTop, [0.0, 40.0, 40.0, 146.67]),
            (PageMarginBoxSlot::LeftBottom, [0.0, 186.67, 40.0, 73.33]),
        ]
    );
}

#[test]
fn a_unitless_line_height_multiplies_a_zero_font_size() {
    // A zero font size is shaped at a size too small to see, but a unitless
    // line height still multiplies the zero, so the upper box has no height
    // and the lower one takes the whole column.
    let boxes = boxes_for(&format!(
        "@page {{ margin: 40px; {AHEM} \
           @left-top {{ content: 'a'; font-size: 0; line-height: 3 }} \
           @left-bottom {{ content: 'c' }} }}"
    ));
    assert_eq!(
        rects(&boxes),
        [(PageMarginBoxSlot::LeftBottom, [0.0, 40.0, 40.0, 220.0])]
    );
}

#[test]
fn auto_top_boxes_share_a_row_by_the_width_of_their_vertical_lines() {
    // The upright lines of the left box are 20px wide each, the right box is
    // one 10px character wide, so they share the row in the ratio of 40 to 10.
    let boxes = boxes_for(&format!(
        "@page {{ margin: 40px; {AHEM} \
           @top-left {{ content: 'a\\A b'; white-space: pre; \
                       writing-mode: vertical-rl; line-height: 2 }} \
           @top-right {{ content: 'c' }} }}"
    ));
    assert_eq!(
        rects(&boxes),
        [
            (PageMarginBoxSlot::TopLeft, [40.0, 0.0, 256.0, 40.0]),
            (PageMarginBoxSlot::TopRight, [296.0, 0.0, 64.0, 40.0]),
        ]
    );
}

#[test]
fn fixed_height_side_boxes_center_in_their_thirds() {
    let boxes = boxes_for(
        "@page { margin: 40px; \
           @right-top { content: ''; height: 20px } \
           @right-middle { content: ''; height: 20px } \
           @right-bottom { content: ''; height: 20px } }",
    );
    assert_eq!(
        rects(&boxes),
        [
            (PageMarginBoxSlot::RightTop, [360.0, 66.67, 40.0, 20.0]),
            (PageMarginBoxSlot::RightMiddle, [360.0, 140.0, 40.0, 20.0]),
            (PageMarginBoxSlot::RightBottom, [360.0, 213.33, 40.0, 20.0]),
        ]
    );
}

#[test]
fn a_lone_fixed_height_middle_box_centers_in_the_column() {
    let boxes = boxes_for("@page { margin: 40px; @right-middle { content: ''; height: 20px } }");
    assert_eq!(
        rects(&boxes),
        [(PageMarginBoxSlot::RightMiddle, [360.0, 140.0, 40.0, 20.0])]
    );
}

#[test]
fn fixed_width_top_boxes_sit_at_the_start_the_middle_and_the_end() {
    // 320px lie between the corners, from x = 40 to x = 360.
    let boxes = boxes_for(
        "@page { margin: 40px; \
           @top-left { content: 'x'; width: 50px } \
           @top-center { content: 'x'; width: 100px } \
           @top-right { content: 'x'; width: 60px } }",
    );
    assert_eq!(
        rects(&boxes),
        [
            (PageMarginBoxSlot::TopLeft, [40.0, 0.0, 50.0, 40.0]),
            (PageMarginBoxSlot::TopCenter, [150.0, 0.0, 100.0, 40.0]),
            (PageMarginBoxSlot::TopRight, [300.0, 0.0, 60.0, 40.0]),
        ]
    );
}

#[test]
fn a_center_box_is_sized_against_double_the_larger_side_box() {
    // The sides count as one box twice the size of the larger side, so that
    // the center box can stay centered. 'aaaa' is 40px wide and the center
    // text 160px: the 80px of the sides and the 160px of the center share the
    // 320px in proportion, which leaves the left box what the center leaves
    // it on each side.
    let boxes = boxes_for(&format!(
        "@page {{ margin: 40px; {AHEM} \
           @top-left {{ content: 'aaaa' }} \
           @top-center {{ content: 'bbbbbbbbbbbbbbbb' }} }}"
    ));
    assert_eq!(
        rects(&boxes),
        [
            (PageMarginBoxSlot::TopLeft, [40.0, 0.0, 53.33, 40.0]),
            (PageMarginBoxSlot::TopCenter, [93.33, 0.0, 213.33, 40.0]),
        ]
    );
}

#[test]
fn an_auto_center_box_stays_centered_between_fixed_side_boxes() {
    // The 150px side box is the larger one, so the center box is sized as if
    // it had one of 150px on each side and is centered at x = 200.
    let boxes = boxes_for(&format!(
        "@page {{ margin: 40px; {AHEM} \
           @top-left {{ content: 'x'; width: 50px }} \
           @top-center {{ content: 'x' }} \
           @top-right {{ content: 'x'; width: 150px }} }}"
    ));
    assert_eq!(
        rects(&boxes),
        [
            (PageMarginBoxSlot::TopLeft, [40.0, 0.0, 50.0, 40.0]),
            (PageMarginBoxSlot::TopCenter, [194.84, 0.0, 10.32, 40.0]),
            (PageMarginBoxSlot::TopRight, [210.0, 0.0, 150.0, 40.0]),
        ]
    );
}

#[test]
fn a_fixed_center_box_stays_centered_under_a_wide_side_box() {
    // The left box runs up to x = 190, over the start of the center box.
    let boxes = boxes_for(
        "@page { margin: 40px; \
           @top-left { content: 'x'; width: 150px } \
           @top-center { content: 'x'; width: 100px } }",
    );
    assert_eq!(
        rects(&boxes),
        [
            (PageMarginBoxSlot::TopLeft, [40.0, 0.0, 150.0, 40.0]),
            (PageMarginBoxSlot::TopCenter, [150.0, 0.0, 100.0, 40.0]),
        ]
    );
}

#[test]
fn auto_side_boxes_split_what_a_fixed_center_box_leaves() {
    let boxes = boxes_for(&format!(
        "@page {{ margin: 40px; {AHEM} \
           @top-left {{ content: 'x' }} \
           @top-center {{ content: 'x'; width: 100px }} \
           @top-right {{ content: 'xxxxxxxx' }} }}"
    ));
    assert_eq!(
        rects(&boxes),
        [
            (PageMarginBoxSlot::TopLeft, [40.0, 0.0, 110.0, 40.0]),
            (PageMarginBoxSlot::TopCenter, [150.0, 0.0, 100.0, 40.0]),
            (PageMarginBoxSlot::TopRight, [250.0, 0.0, 110.0, 40.0]),
        ]
    );
}

#[test]
fn a_lone_auto_box_takes_what_the_other_box_leaves() {
    let right_fixed = boxes_for(
        "@page { margin: 40px; \
           @top-left { content: 'x' } \
           @top-right { content: 'x'; width: 50px } }",
    );
    assert_eq!(
        rects(&right_fixed),
        [
            (PageMarginBoxSlot::TopLeft, [40.0, 0.0, 270.0, 40.0]),
            (PageMarginBoxSlot::TopRight, [310.0, 0.0, 50.0, 40.0]),
        ]
    );
    let left_fixed = boxes_for(
        "@page { margin: 40px; \
           @top-left { content: 'x'; width: 50px } \
           @top-right { content: 'x' } }",
    );
    assert_eq!(
        rects(&left_fixed),
        [
            (PageMarginBoxSlot::TopLeft, [40.0, 0.0, 50.0, 40.0]),
            (PageMarginBoxSlot::TopRight, [90.0, 0.0, 270.0, 40.0]),
        ]
    );
}

#[test]
fn two_auto_boxes_share_the_edge_by_their_content_when_it_fits() {
    // 40px and 80px of text leave 200px, which the boxes share in proportion
    // to the widths of their text.
    let boxes = boxes_for(&format!(
        "@page {{ margin: 40px; {AHEM} \
           @top-left {{ content: 'aaaa' }} \
           @top-right {{ content: 'cccccccc' }} }}"
    ));
    assert_eq!(
        rects(&boxes),
        [
            (PageMarginBoxSlot::TopLeft, [40.0, 0.0, 106.67, 40.0]),
            (PageMarginBoxSlot::TopRight, [146.67, 0.0, 213.33, 40.0]),
        ]
    );
}

#[test]
fn two_auto_boxes_that_do_not_fit_wrap_in_proportion_to_what_can_wrap() {
    // The text is 110px and 260px wide on one line but breaks into 20px
    // words, so each box starts at 20px and takes a share of the remaining
    // 280px in proportion to how much it can still grow: 90px and 240px.
    let boxes = boxes_for(&format!(
        "@page {{ margin: 40px; {AHEM} \
           @top-left {{ content: 'aa aa aa aa' }} \
           @top-right {{ content: 'cc cc cc cc cc cc cc cc cc' }} }}"
    ));
    assert_eq!(
        rects(&boxes),
        [
            (PageMarginBoxSlot::TopLeft, [40.0, 0.0, 96.36, 40.0]),
            (PageMarginBoxSlot::TopRight, [136.36, 0.0, 223.64, 40.0]),
        ]
    );
}

#[test]
fn two_auto_boxes_that_cannot_wrap_shrink_in_proportion_to_their_size() {
    // 300px and 100px of unbreakable text have 80px too many between them.
    let boxes = boxes_for(&format!(
        "@page {{ margin: 40px; {AHEM} \
           @top-left {{ content: 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa' }} \
           @top-right {{ content: 'cccccccccc' }} }}"
    ));
    assert_eq!(
        rects(&boxes),
        [
            (PageMarginBoxSlot::TopLeft, [40.0, 0.0, 240.0, 40.0]),
            (PageMarginBoxSlot::TopRight, [280.0, 0.0, 80.0, 40.0]),
        ]
    );
}

#[test]
fn empty_top_boxes_share_the_edge_equally() {
    // Nothing in the boxes asks for any width, so each of the three gets the
    // same, and a lone center box has the edge to itself.
    let boxes = boxes_for(
        "@page { margin: 40px; \
           @top-left { content: '' } @top-center { content: '' } \
           @top-right { content: '' } }",
    );
    assert_eq!(
        rects(&boxes),
        [
            (PageMarginBoxSlot::TopLeft, [40.0, 0.0, 106.67, 40.0]),
            (PageMarginBoxSlot::TopCenter, [146.67, 0.0, 106.67, 40.0]),
            (PageMarginBoxSlot::TopRight, [253.33, 0.0, 106.67, 40.0]),
        ]
    );
    let lone = boxes_for("@page { margin: 40px; @top-center { content: '' } }");
    assert_eq!(
        rects(&lone),
        [(PageMarginBoxSlot::TopCenter, [40.0, 0.0, 320.0, 40.0])]
    );
}

#[test]
fn a_lone_fixed_width_box_keeps_to_its_slot_on_the_bottom_edge() {
    let end = boxes_for("@page { margin: 40px; @bottom-right { content: 'x'; width: 100px } }");
    assert_eq!(
        rects(&end),
        [(PageMarginBoxSlot::BottomRight, [260.0, 260.0, 100.0, 40.0])]
    );
    let middle = boxes_for("@page { margin: 40px; @bottom-center { content: 'x'; width: 100px } }");
    assert_eq!(
        rects(&middle),
        [(PageMarginBoxSlot::BottomCenter, [150.0, 260.0, 100.0, 40.0])]
    );
}

#[test]
fn fixed_height_side_boxes_sit_at_the_start_and_the_end_of_the_column() {
    // 220px lie between the corners, from y = 40 to y = 260.
    let both = boxes_for(
        "@page { margin: 40px; \
           @left-top { content: 'x'; height: 20px } \
           @left-bottom { content: 'x'; height: 30px } }",
    );
    assert_eq!(
        rects(&both),
        [
            (PageMarginBoxSlot::LeftTop, [0.0, 40.0, 40.0, 20.0]),
            (PageMarginBoxSlot::LeftBottom, [0.0, 230.0, 40.0, 30.0]),
        ]
    );
    let end = boxes_for("@page { margin: 40px; @right-bottom { content: 'x'; height: 20px } }");
    assert_eq!(
        rects(&end),
        [(PageMarginBoxSlot::RightBottom, [360.0, 240.0, 40.0, 20.0])]
    );
}

#[test]
fn a_fixed_size_box_longer_than_its_edge_overflows_away_from_its_slot() {
    // The last box stays flush with the end of the edge, so it runs out
    // past the start.
    let boxes = boxes_for("@page { margin: 40px; @top-right { content: 'x'; width: 400px } }");
    assert_eq!(
        rects(&boxes),
        [(PageMarginBoxSlot::TopRight, [-40.0, 0.0, 400.0, 40.0])]
    );
}

#[test]
fn empty_side_boxes_split_the_column_evenly() {
    let boxes = boxes_for(
        "@page { margin: 40px; @right-top { content: '' } @right-bottom { content: '' } }",
    );
    assert_eq!(
        rects(&boxes),
        [
            (PageMarginBoxSlot::RightTop, [360.0, 40.0, 40.0, 110.0]),
            (PageMarginBoxSlot::RightBottom, [360.0, 150.0, 40.0, 110.0]),
        ]
    );
}

#[test]
fn fixed_width_left_boxes_hug_the_page_area_unless_centered() {
    let boxes = boxes_for(
        "@page { margin: 40px; \
           @left-top { content: ''; width: 20px; height: 20px } \
           @left-bottom { content: ''; width: 20px; height: 20px; margin: auto } }",
    );
    assert_eq!(
        rects(&boxes),
        [
            (PageMarginBoxSlot::LeftTop, [20.0, 40.0, 20.0, 20.0]),
            (PageMarginBoxSlot::LeftBottom, [10.0, 240.0, 20.0, 20.0]),
        ]
    );
}

#[test]
fn corner_boxes_fill_or_align_in_their_cells() {
    let boxes = boxes_for(
        "@page { margin: 40px; \
           @top-left-corner { content: ''; width: 20px; height: 20px } \
           @top-right-corner { content: ''; width: 20px; height: 20px; margin: auto } \
           @bottom-left-corner { content: '' } \
           @bottom-right-corner { content: ''; height: 20px } }",
    );
    assert_eq!(
        rects(&boxes),
        [
            (PageMarginBoxSlot::TopLeftCorner, [20.0, 20.0, 20.0, 20.0]),
            (PageMarginBoxSlot::TopRightCorner, [370.0, 10.0, 20.0, 20.0]),
            (
                PageMarginBoxSlot::BottomRightCorner,
                [360.0, 260.0, 40.0, 20.0]
            ),
            (
                PageMarginBoxSlot::BottomLeftCorner,
                [0.0, 260.0, 40.0, 40.0]
            ),
        ]
    );
}

#[test]
fn three_auto_top_boxes_keep_the_center_box_centered() {
    let boxes = boxes_for(&format!(
        "@page {{ margin: 40px; {AHEM} \
           @top-left {{ content: 'a' }} @top-center {{ content: 'bb' }} \
           @top-right {{ content: 'a' }} }}"
    ));
    assert_eq!(
        rects(&boxes),
        [
            (PageMarginBoxSlot::TopLeft, [40.0, 0.0, 80.0, 40.0]),
            (PageMarginBoxSlot::TopCenter, [120.0, 0.0, 160.0, 40.0]),
            (PageMarginBoxSlot::TopRight, [280.0, 0.0, 80.0, 40.0]),
        ]
    );
}

#[test]
fn an_empty_center_box_anchors_the_side_boxes_to_half_the_row() {
    let boxes = boxes_for(&format!(
        "@page {{ margin: 40px; {AHEM} \
           @top-left {{ content: 'abc' }} @top-center {{ content: '' }} }}"
    ));
    assert_eq!(
        rects(&boxes),
        [(PageMarginBoxSlot::TopLeft, [40.0, 0.0, 160.0, 40.0])]
    );
}

#[test]
fn an_empty_center_box_leaves_a_fixed_side_box_its_width() {
    // Only the auto-sized side takes half of what the empty center box
    // leaves; the box with a width keeps it.
    let boxes = boxes_for(&format!(
        "@page {{ margin: 40px; {AHEM} \
           @top-left {{ content: 'x'; width: 100px }} \
           @top-center {{ content: '' }} \
           @top-right {{ content: 'x' }} }}"
    ));
    assert_eq!(
        rects(&boxes),
        [
            (PageMarginBoxSlot::TopLeft, [40.0, 0.0, 100.0, 40.0]),
            (PageMarginBoxSlot::TopRight, [200.0, 0.0, 160.0, 40.0]),
        ]
    );
}

#[test]
fn an_empty_auto_box_takes_what_a_fixed_box_leaves() {
    let boxes = boxes_for(
        "@page { margin: 40px; \
           @top-left { content: 'x'; width: 100px } \
           @top-right { content: '' } }",
    );
    assert_eq!(
        rects(&boxes),
        [
            (PageMarginBoxSlot::TopLeft, [40.0, 0.0, 100.0, 40.0]),
            (PageMarginBoxSlot::TopRight, [140.0, 0.0, 220.0, 40.0]),
        ]
    );
}

#[test]
fn an_empty_middle_box_anchors_the_side_boxes_to_half_the_column() {
    let boxes = boxes_for(&format!(
        "@page {{ margin: 40px; {AHEM} \
           @left-top {{ content: 'abc' }} @left-middle {{ content: '' }} }}"
    ));
    assert_eq!(
        rects(&boxes),
        [(PageMarginBoxSlot::LeftTop, [0.0, 40.0, 40.0, 110.0])]
    );
}

#[test]
fn vertical_text_in_side_boxes_wraps_at_the_column_like_text_in_top_boxes() {
    // The upper text is 80px tall on one line and the lower one 230px, which
    // is more than the 220px column. Both break into 20px words, so each box
    // starts at 20px and takes a share of the remaining 180px in proportion
    // to how much it can still grow: 60px and 210px.
    let boxes = boxes_for(&format!(
        "@page {{ margin: 40px; {AHEM} writing-mode: vertical-rl; \
           @left-top {{ content: 'ab cd ef' }} \
           @left-bottom {{ content: 'ab cd ef gh ij kl mn op' }} }}"
    ));
    assert_eq!(
        rects(&boxes),
        [
            (PageMarginBoxSlot::LeftTop, [0.0, 40.0, 40.0, 60.0]),
            (PageMarginBoxSlot::LeftBottom, [0.0, 100.0, 40.0, 160.0]),
        ]
    );
}

#[test]
fn fixed_height_edge_boxes_sit_against_the_page_area_or_center() {
    let boxes = boxes_for(
        "@page { margin: 40px; \
           @top-center { content: ''; height: 20px } \
           @bottom-center { content: ''; height: 20px; margin: auto } }",
    );
    assert_eq!(
        rects(&boxes),
        [
            (PageMarginBoxSlot::TopCenter, [40.0, 20.0, 320.0, 20.0]),
            (PageMarginBoxSlot::BottomCenter, [40.0, 270.0, 320.0, 20.0]),
        ]
    );
}

#[test]
fn box_lengths_resolve_against_their_basis() {
    let px = |value| length_or_auto_px(value, 200.0);
    assert_eq!(px(ComputedLengthPercentageOrAuto::Auto), None);
    assert_eq!(px(ComputedLengthPercentageOrAuto::Px(5.0)), Some(5.0));
    assert_eq!(
        px(ComputedLengthPercentageOrAuto::Percent(50.0)),
        Some(100.0)
    );
    assert_eq!(px(ComputedLengthPercentageOrAuto::Px(-5.0)), Some(-5.0));
    assert_eq!(px(ComputedLengthPercentageOrAuto::Px(f32::NAN)), Some(0.0));
    assert_eq!(px(ComputedLengthPercentageOrAuto::MinContent), None);
    let calc = |px, percent| {
        ComputedLengthPercentageOrAuto::Calc(raikiri_style::property::CalcLengthPercentage {
            px,
            percent,
        })
    };
    assert_eq!(px(calc(10.0, 10.0)), Some(30.0));
    assert_eq!(px(calc(f32::NAN, f32::NAN)), Some(0.0));
    assert_eq!(px(calc(-50.0, 0.0)), Some(-50.0));
}

#[test]
fn margin_box_margins_may_be_negative_and_zero_fonts_stay_tiny() {
    let x = |declarations: &str| {
        let boxes = boxes_for(&format!(
            "@page {{ margin: 40px; @top-left {{ content: 'a'; {AHEM} width: 30px; \
               {declarations} }} }}"
        ));
        let [head] = boxes.as_slice() else {
            panic!("one box: {boxes:?}");
        };
        (
            head.rect.x,
            head.text.as_ref().map(|text| text.shaped().width()),
        )
    };
    let (origin, width) = x("");
    assert_eq!(x("margin-left: -6px").0, origin - 6.0);
    assert_eq!(width, Some(10.0));
    let (_, tiny) = x("font-size: 0");
    assert!(tiny.is_some_and(|width| width < 1.0), "{tiny:?}");
}

#[test]
fn ch_lengths_use_the_zero_advance_of_the_margin_box_font() {
    // Ahem's `0` is one em wide, where the style layer's fallback is half
    // an em.
    let boxes = boxes_for(&format!(
        "@page {{ margin: 40px; @top-left {{ content: 'a'; {AHEM} width: 3ch; \
           padding-left: 1ch; margin-left: calc(1ch + 2px); }} }}"
    ));
    let [head] = boxes.as_slice() else {
        panic!("one box: {boxes:?}");
    };
    // The page margin plus the 12px left margin; the 30px content plus the
    // 10px left padding.
    assert_eq!(head.rect.x, 52.0);
    assert_eq!(head.rect.width, 40.0);
    assert_eq!(head.padding.left, 10.0);
}

#[test]
fn box_sizes_paddings_and_margins_accept_calc_and_longhands() {
    let boxes = boxes_for(
        "@page { margin: 40px; \
           @top-left { content: ''; width: 10%; height: 20px; \
                       padding: 1px 2px 3px 4px; padding-left: 5px; \
                       margin: 0; margin-left: 2px; margin-top: 1px } }",
    );
    let [margin_box] = boxes.as_slice() else {
        panic!("one box: {boxes:?}");
    };
    // 10% of the 320px strip, plus 7px of horizontal padding.
    assert_eq!(margin_box.rect.width, 39.0);
    assert_eq!(margin_box.rect.x, 42.0);
    // 20px plus 4px of vertical padding, against the page area.
    assert_eq!(margin_box.rect.height, 24.0);
    assert_eq!(margin_box.rect.y, 40.0 - 25.0 + 1.0);
    assert_eq!(
        (
            margin_box.padding.top,
            margin_box.padding.right,
            margin_box.padding.bottom,
            margin_box.padding.left
        ),
        (1.0, 2.0, 3.0, 5.0)
    );
}

#[test]
fn border_longhands_override_the_shorthand_and_skip_unsupported_styles() {
    let boxes = boxes_for(
        "@page { margin: 40px; color: blue; \
           @top-left { content: ''; border: 1px solid red; \
                       border-top-width: 3px; border-right-style: dashed; \
                       border-bottom-color: currentcolor; border-left-color: lime } }",
    );
    let [margin_box] = boxes.as_slice() else {
        panic!("one box: {boxes:?}");
    };
    let [top, right, bottom, left] = margin_box.borders;
    assert_eq!(
        top.map(|border| (border.width, border.color.r)),
        Some((3.0, 255))
    );
    assert!(right.is_none(), "dashed borders are not drawn");
    assert_eq!(bottom.map(|border| border.color.b), Some(255));
    assert_eq!(left.map(|border| border.color.g), Some(255));
    assert_eq!(margin_box.color.b, 255, "the page color is inherited");
}

#[test]
fn margin_boxes_inherit_page_fonts_and_an_explicitly_inherited_text_align() {
    // `vertical-align` is not inherited, so the page's value does not reach
    // the box.
    let boxes = boxes_for(
        "@page { margin: 40px; font-family: Ahem; font-size: 20px; text-align: center; \
           vertical-align: text-top; \
           @top-left { content: 'ab'; font-size: 50%; text-align: inherit; \
                       vertical-align: text-bottom } }",
    );
    let [margin_box] = boxes.as_slice() else {
        panic!("one box: {boxes:?}");
    };
    let text = margin_box.text.as_ref().expect("the box has text");
    let (x, y) = text.origin();
    assert_eq!(text.shaped().width(), 20.0);
    // Centered on the 320px strip and pushed to the bottom of the 40px row.
    assert_eq!(x, 40.0);
    assert_eq!(y, 30.0);
    let glyphs = glyph_positions(text);
    assert_eq!(glyphs[0].0, 40.0 + 150.0);
}

/// The `text-align` and `vertical-align` the spec's default table gives each
/// slot, by at-rule name.
const SLOT_DEFAULT_ALIGNMENT: [(&str, StandaloneAlign, MarginTextVerticalAlign); 16] = {
    use MarginTextVerticalAlign::{Bottom, Middle, Top};
    use StandaloneAlign::{Center, Left, Right};
    [
        ("top-left-corner", Right, Middle),
        ("top-left", Left, Middle),
        ("top-center", Center, Middle),
        ("top-right", Right, Middle),
        ("top-right-corner", Left, Middle),
        ("right-top", Center, Top),
        ("right-middle", Center, Middle),
        ("right-bottom", Center, Bottom),
        ("bottom-right-corner", Left, Middle),
        ("bottom-right", Right, Middle),
        ("bottom-center", Center, Middle),
        ("bottom-left", Left, Middle),
        ("bottom-left-corner", Right, Middle),
        ("left-bottom", Center, Bottom),
        ("left-middle", Center, Middle),
        ("left-top", Center, Top),
    ]
};

fn alignment_of(rule_css: &str) -> (StandaloneAlign, MarginTextVerticalAlign) {
    let (doc, cascade) = page_cascade_fixture(rule_css);
    let slot = cascade.page.margin_boxes().first().unwrap().slot;
    let rule = cascade.page.cascade_margin_box(slot).unwrap();
    let spec = margin_box_spec(
        &doc,
        &cascade,
        &cascade.page,
        &rule,
        100.0,
        40.0,
        MarginBoxPageContext::new(0, 1, false),
        &MarginBoxNamedValues::new(),
    )
    .unwrap();
    (spec.alignment, spec.vertical_align)
}

#[test]
fn every_slot_starts_from_its_default_alignment_whatever_the_page_sets() {
    for (slot, text_align, vertical_align) in SLOT_DEFAULT_ALIGNMENT {
        // The page's own `text-align` is inherited, but the defaults act as
        // UA style sheet rules on the margin box itself.
        for page_rule in ["", "text-align: justify; vertical-align: text-bottom;"] {
            assert_eq!(
                alignment_of(&format!(
                    "@page {{ {page_rule} @{slot} {{ content: 'a' }} }}"
                )),
                (text_align, vertical_align),
                "{slot} {page_rule}"
            );
        }
    }
}

#[test]
fn author_alignment_overrides_the_slot_default() {
    use MarginTextVerticalAlign::{Bottom, Middle, Top};
    use StandaloneAlign::{Center, Left, Right};
    let top_left = |declarations: &str| {
        alignment_of(&format!(
            "@page {{ text-align: right; @top-left {{ content: 'a'; {declarations} }} }}"
        ))
    };
    assert_eq!(top_left(""), (Left, Middle));
    assert_eq!(top_left("text-align: center"), (Center, Middle));
    assert_eq!(top_left("text-align: inherit"), (Right, Middle));
    assert_eq!(top_left("vertical-align: top"), (Left, Top));
    assert_eq!(top_left("vertical-align: bottom"), (Left, Bottom));
    assert_eq!(top_left("vertical-align: text-bottom"), (Left, Bottom));
    // Anything but `middle` and the bottom keywords places the text at the top.
    assert_eq!(top_left("vertical-align: baseline"), (Left, Top));
    assert_eq!(top_left("vertical-align: text-top"), (Left, Top));
    assert_eq!(top_left("vertical-align: 5px"), (Left, Top));
    // A declaration that is invalid at computed-value time behaves as `unset`
    // rather than falling back to the slot default: `text-align` is inherited
    // from the page and `vertical-align` takes its initial value.
    assert_eq!(top_left("text-align: var(--missing)"), (Right, Middle));
    assert_eq!(top_left("vertical-align: var(--missing)"), (Left, Top));
    assert_eq!(
        top_left("--align: center; text-align: var(--align)"),
        (Center, Middle)
    );
    // A rule that follows a lower-priority duplicate still wins over the
    // defaults.
    assert_eq!(
        alignment_of(
            "@page { @right-top { content: 'a'; vertical-align: middle } \
               @right-top { text-align: right } }"
        ),
        (Right, Middle)
    );
}

#[test]
fn top_boxes_align_their_text_along_the_strip_by_default() {
    // Three 100px boxes sit at the start, the middle and the end of the 320px
    // strip, and each text run is 10px wide.
    let boxes = boxes_for(&format!(
        "@page {{ margin: 40px; {AHEM} \
           @top-left {{ content: 'a'; width: 100px }} \
           @top-center {{ content: 'b'; width: 100px }} \
           @top-right {{ content: 'c'; width: 100px }} }}"
    ));
    let runs: Vec<_> = boxes
        .iter()
        .map(|margin_box| {
            (
                margin_box.slot,
                margin_box.rect.x,
                margin_box.text_runs()[0].origin,
            )
        })
        .collect();
    assert_eq!(
        runs,
        [
            (PageMarginBoxSlot::TopLeft, 40.0, (40.0, 23.0)),
            (PageMarginBoxSlot::TopCenter, 150.0, (195.0, 23.0)),
            (PageMarginBoxSlot::TopRight, 260.0, (350.0, 23.0)),
        ]
    );
}

#[test]
fn side_boxes_centre_their_text_and_stack_it_from_top_to_bottom_by_default() {
    // The three 73.33px tall boxes of a side share the strip, and the 10px
    // line is centred across their 40px width.
    let boxes = boxes_for(&format!(
        "@page {{ margin: 40px; {AHEM} \
           @left-top {{ content: 'a' }} @left-middle {{ content: 'b' }} \
           @left-bottom {{ content: 'c' }} @right-top {{ content: 'd' }} }}"
    ));
    let runs: Vec<_> = boxes
        .iter()
        .map(|margin_box| (margin_box.slot, margin_box.text_runs()[0].origin))
        .collect();
    assert_eq!(
        runs,
        [
            (PageMarginBoxSlot::LeftTop, (15.0, 48.0)),
            (PageMarginBoxSlot::LeftMiddle, (15.0, 145.0 + 8.0)),
            (PageMarginBoxSlot::LeftBottom, (15.0, 250.0 + 8.0)),
            (PageMarginBoxSlot::RightTop, (375.0, 48.0)),
        ]
    );
}

#[test]
fn every_text_align_keyword_reaches_the_spec() {
    for (keyword, expected) in [
        ("left", StandaloneAlign::Left),
        ("right", StandaloneAlign::Right),
        ("center", StandaloneAlign::Center),
        ("end", StandaloneAlign::End),
        ("justify", StandaloneAlign::Justify),
        ("start", StandaloneAlign::Start),
    ] {
        let (doc, cascade) = page_cascade_fixture(&format!(
            "@page {{ @top-left {{ content: 'a'; text-align: {keyword}; vertical-align: text-top }} }}"
        ));
        let rule = cascade.page.margin_boxes().first().unwrap();
        let spec = margin_box_spec(
            &doc,
            &cascade,
            &cascade.page,
            rule,
            100.0,
            40.0,
            MarginBoxPageContext::new(0, 1, false),
            &MarginBoxNamedValues::new(),
        )
        .unwrap();
        assert_eq!(spec.alignment, expected, "{keyword}");
        assert_eq!(spec.vertical_align, MarginTextVerticalAlign::Top);
    }
}

#[test]
fn page_counters_follow_page_resets_and_increments() {
    let content = |page_rule: &str, box_rule: &str, context: MarginBoxPageContext| {
        let boxes = boxes_on_page(
            &format!(
                "@page {{ margin: 40px; {page_rule}; \
                   @top-left {{ content: counter(page) '|' counters(chapter, '.') '|' \
                                counter(pages); {box_rule} }} }}"
            ),
            context,
        );
        boxes
            .first()
            .unwrap_or_else(|| panic!("no box for {page_rule:?} {box_rule:?}"))
            .content
            .clone()
    };
    let third = MarginBoxPageContext::new(2, 6, false);
    assert_eq!(content("", "", third), "3|0|6");
    assert_eq!(content("counter-reset: page 10", "", third), "11|0|6");
    assert_eq!(
        content("counter-increment: page 2 chapter", "", third),
        "6|3|6"
    );
    assert_eq!(content("counter-reset: none", "", third), "2|0|6");
    assert_eq!(content("counter-increment: page 3", "", third), "6|0|6");
    assert_eq!(
        content(
            "counter-increment: page 0",
            "",
            MarginBoxPageContext::new(4, 6, false).with_paired_page_increment(Some(2)),
        ),
        "4|0|6"
    );
    assert_eq!(
        content(
            "",
            "counter-reset: chapter 4; counter-increment: chapter",
            third
        ),
        "3|5|6"
    );
    assert_eq!(
        content("", "counter-reset: inherit; counter-increment: page", third),
        "4|0|6"
    );
}

#[test]
fn margin_box_quotes_come_from_the_box_the_page_or_the_root() {
    let content = "content: open-quote 'q' close-quote no-open-quote no-close-quote";
    assert_eq!(
        single_content(&format!(
            "@page {{ margin: 40px; @top-left {{ {content}; quotes: '<' '>' }} }}"
        )),
        "<q>"
    );
    assert_eq!(
        single_content(&format!(
            "@page {{ margin: 40px; quotes: none; @top-left {{ {content} }} }}"
        )),
        "q"
    );
    assert_eq!(
        single_content(&format!(
            "@page {{ margin: 40px; @top-left {{ {content} }} }}"
        )),
        "“q”"
    );
    assert_eq!(
        single_content(&format!(
            "html {{ quotes: '[' ']' }} @page {{ margin: 40px; @top-left {{ {content} }} }}"
        )),
        "[q]"
    );
    assert_eq!(
        single_content(&format!(
            "html {{ quotes: none }} @page {{ margin: 40px; @top-left {{ {content} }} }}"
        )),
        "q"
    );
}

#[test]
fn named_strings_and_running_elements_resolve_to_document_text() {
    let mut doc = engine_document();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let head = doc.append_element(Some(html), "head", Style::default(), Some("display:none"));
    let style = doc.append_element(Some(head), "style", Style::default(), None::<&str>);
    doc.append_text(
        style,
        "h1 { string-set: title 'Ch. ' content() } \
         div { position: running(hdr) } \
         @page { margin: 40px; \
           @top-left { content: string(title) } \
           @top-right { content: element(hdr) } \
           @bottom-left { content: string(missing) element(missing) attr(id) } }",
    );
    let body = doc.append_element(Some(html), "body", Style::default(), Some("display:block"));
    let h1 = doc.append_element(Some(body), "h1", Style::default(), None::<&str>);
    doc.append_text(h1, "  One \n Two ");
    let hdr = doc.append_element(Some(body), "div", Style::default(), None::<&str>);
    doc.append_text(hdr, "Running head");
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let contents: Vec<_> = layout_page(&doc, &cascade, small_page())
        .into_iter()
        .map(|margin_box| (margin_box.slot, margin_box.content))
        .collect();
    assert_eq!(
        contents,
        [
            (PageMarginBoxSlot::TopLeft, "Ch. One Two".to_owned()),
            (PageMarginBoxSlot::TopRight, "Running head".to_owned()),
            (PageMarginBoxSlot::BottomLeft, String::new()),
        ]
    );
}

#[test]
fn named_strings_and_running_elements_keep_no_break_spaces_and_form_feeds() {
    // CSS Text 3 §4.1 collapses only spaces, tabs and segment breaks;
    // U+00A0 and U+000C are ordinary text in the string value.
    let mut doc = engine_document();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let head = doc.append_element(Some(html), "head", Style::default(), Some("display:none"));
    let style = doc.append_element(Some(head), "style", Style::default(), None::<&str>);
    doc.append_text(
        style,
        "h1 { string-set: title content() } \
         div { position: running(hdr) } \
         @page { margin: 40px; \
           @top-left { content: string(title) } \
           @top-right { content: element(hdr) } }",
    );
    let body = doc.append_element(Some(html), "body", Style::default(), Some("display:block"));
    let h1 = doc.append_element(Some(body), "h1", Style::default(), None::<&str>);
    doc.append_text(h1, " A\u{A0}\u{A0}B ");
    let hdr = doc.append_element(Some(body), "div", Style::default(), None::<&str>);
    doc.append_text(hdr, "C\u{C}D");
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let contents: Vec<_> = layout_page(&doc, &cascade, small_page())
        .into_iter()
        .map(|margin_box| (margin_box.slot, margin_box.content))
        .collect();
    assert_eq!(
        contents,
        [
            (PageMarginBoxSlot::TopLeft, "A\u{A0}\u{A0}B".to_owned()),
            (PageMarginBoxSlot::TopRight, "C\u{C}D".to_owned()),
        ]
    );
}

#[test]
fn shared_named_values_take_the_last_assignment_on_every_page() {
    let mut doc = engine_document();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let head = doc.append_element(Some(html), "head", Style::default(), Some("display:none"));
    let style = doc.append_element(Some(head), "style", Style::default(), None::<&str>);
    doc.append_text(
        style,
        "h1 { string-set: title content(), title 'Last ' content() } \
         div { position: running(hdr) } \
         @page { margin: 40px; \
           @top-left { content: string(title) } \
           @top-right { content: element(hdr) } }",
    );
    let body = doc.append_element(Some(html), "body", Style::default(), Some("display:block"));
    for (heading, running) in [("One", "First head"), ("Two", "Second head")] {
        let h1 = doc.append_element(Some(body), "h1", Style::default(), None::<&str>);
        doc.append_text(h1, heading);
        let hdr = doc.append_element(Some(body), "div", Style::default(), None::<&str>);
        doc.append_text(hdr, running);
    }
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let values = MarginBoxNamedValues::new();
    for page_index in 0..2 {
        let contents: Vec<_> = page_margin_boxes_with_values(
            &doc,
            &cascade,
            &cascade.page,
            small_page(),
            MarginBoxPageContext::new(page_index, 2, page_index % 2 == 1),
            &values,
        )
        .into_iter()
        .map(|margin_box| (margin_box.slot, margin_box.content))
        .collect();
        assert_eq!(
            contents,
            [
                (PageMarginBoxSlot::TopLeft, "Last Two".to_owned()),
                (PageMarginBoxSlot::TopRight, "Second head".to_owned()),
            ],
            "page {page_index}"
        );
    }
}

#[test]
fn a_lime_content_image_is_placed_after_the_text() {
    let boxes = boxes_for(&format!(
        "@page {{ margin: 40px; @top-left {{ content: 'ab' url(green.png); {AHEM} }} }}"
    ));
    let [margin_box] = boxes.as_slice() else {
        panic!("one box: {boxes:?}");
    };
    assert_eq!(margin_box.content, "ab");
    assert_eq!(
        margin_box.lime_content_image,
        Some(PaintRect::new(60.0, 0.0, 100.0, 40.0))
    );
}

#[test]
fn named_values_follow_document_order_after_the_tree_changes() {
    // Moving the first heading after the second, and detaching a third,
    // leaves the arena order unlike the document order.
    let mut doc = engine_document();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let head = doc.append_element(Some(html), "head", Style::default(), Some("display:none"));
    let style = doc.append_element(Some(head), "style", Style::default(), None::<&str>);
    doc.append_text(
        style,
        "h1 { string-set: title content() } \
         div { position: running(hdr) } \
         @page { margin: 40px; \
           @top-left { content: string(title) } \
           @top-right { content: element(hdr) } }",
    );
    let body = doc.append_element(Some(html), "body", Style::default(), Some("display:block"));
    let mut moved = Vec::new();
    for (heading, running) in [("Last", "Last head"), ("First", "First head")] {
        let h1 = doc.append_element(Some(body), "h1", Style::default(), None::<&str>);
        doc.append_text(h1, heading);
        let hdr = doc.append_element(Some(body), "div", Style::default(), None::<&str>);
        doc.append_text(hdr, running);
        moved.extend([h1, hdr]);
    }
    for &node in &moved[..2] {
        doc.detach_from_parent(node);
        doc.append_child(body, node).unwrap();
    }
    let detached = doc.append_element(Some(body), "h1", Style::default(), None::<&str>);
    doc.append_text(detached, "Detached");
    doc.detach_from_parent(detached);
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let contents: Vec<_> = layout_page(&doc, &cascade, small_page())
        .into_iter()
        .map(|margin_box| (margin_box.slot, margin_box.content))
        .collect();
    assert_eq!(
        contents,
        [
            (PageMarginBoxSlot::TopLeft, "Last".to_owned()),
            (PageMarginBoxSlot::TopRight, "Last head".to_owned()),
        ]
    );
}
