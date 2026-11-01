//! Margin-box geometry, decoration and generated content resolved from
//! `@page` rules, on a 400x300 page with 40px page margins.

use super::*;

const AHEM: &str = "font-family: Ahem; font-size: 10px;";

pub(super) fn boxes_for(css: &str) -> Vec<MarginBox> {
    let (doc, cascade) = page_cascade_fixture(css);
    layout_page(&doc, &cascade, small_page())
}

fn boxes_on_page(css: &str, context: MarginBoxPageContext) -> Vec<MarginBox> {
    let (doc, cascade) = page_cascade_fixture(css);
    page_margin_boxes(&doc, &cascade, &cascade.page, small_page(), context)
}

pub(super) fn rects(boxes: &[MarginBox]) -> Vec<(PageMarginBoxSlot, [f32; 4])> {
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
            (PageMarginBoxSlot::LeftBottom, [0.0, 186.67, 40.0, 73.33]),
            (PageMarginBoxSlot::LeftMiddle, [0.0, 113.33, 40.0, 73.33]),
            (PageMarginBoxSlot::LeftTop, [0.0, 40.0, 40.0, 73.33]),
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
            (PageMarginBoxSlot::LeftBottom, [0.0, 228.57, 40.0, 31.43]),
            (PageMarginBoxSlot::LeftTop, [0.0, 40.0, 40.0, 188.57]),
        ]
    );
    assert_eq!(
        rects_with("line-height: 25px"),
        [
            (PageMarginBoxSlot::LeftBottom, [0.0, 223.33, 40.0, 36.67]),
            (PageMarginBoxSlot::LeftTop, [0.0, 40.0, 40.0, 183.33]),
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
            (PageMarginBoxSlot::LeftBottom, [0.0, 205.0, 40.0, 55.0]),
            (PageMarginBoxSlot::LeftTop, [0.0, 40.0, 40.0, 165.0]),
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
            (PageMarginBoxSlot::LeftBottom, [0.0, 150.0, 40.0, 110.0]),
            (PageMarginBoxSlot::LeftTop, [0.0, 40.0, 40.0, 110.0]),
        ]
    );
}

#[test]
fn a_line_feed_that_ends_the_text_does_not_start_another_line() {
    // `\A ` is a line feed. The text of the upper box is two lines with or
    // without the one at its end, so the column is shared the same way.
    let left = |content: &str| {
        rects(&boxes_for(&format!(
            "@page {{ margin: 40px; {AHEM} \
               @left-top {{ content: '{content}'; white-space: pre }} \
               @left-bottom {{ content: 'c' }} }}"
        )))
    };
    assert_eq!(left("a\\A b\\A "), left("a\\A b"));
    // A text of nothing but a line feed keeps it, and is the single line it
    // makes: as tall as the one line of 'a'.
    assert_eq!(left("\\A "), left("a"));
    // Only the last of two line feeds ends the text; the first ends the line
    // before the empty one, which is as tall as the line of 'b'.
    assert_eq!(left("a\\A\\A "), left("a\\A b"));
    // Where the line feed collapses, the shaper adds no empty line after it.
    let collapsed = |content: &str| {
        rects(&boxes_for(&format!(
            "@page {{ margin: 40px; {AHEM} \
               @left-top {{ content: '{content}' }} \
               @left-bottom {{ content: 'c' }} }}"
        )))
    };
    assert_eq!(collapsed("a\\A "), collapsed("a"));
    // Vertical lines share a row the same way: one column per line.
    let top = |content: &str| {
        rects(&boxes_for(&format!(
            "@page {{ margin: 40px; {AHEM} \
               @top-left {{ content: '{content}'; white-space: pre; writing-mode: vertical-rl }} \
               @top-right {{ content: 'c' }} }}"
        )))
    };
    assert_eq!(top("a\\A b\\A "), top("a\\A b"));
    // The empty column between two line feeds is one of the columns.
    assert_eq!(top("a\\A\\A "), top("a\\A b"));
    // A line feed that does not end the text still counts: three lines
    // against the one line of the lower box, shared in the ratio of 3 to 1.
    assert_eq!(
        left("a\\A b\\A c"),
        [
            (PageMarginBoxSlot::LeftBottom, [0.0, 205.0, 40.0, 55.0]),
            (PageMarginBoxSlot::LeftTop, [0.0, 40.0, 40.0, 165.0]),
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
            (PageMarginBoxSlot::LeftBottom, [0.0, 186.67, 40.0, 73.33]),
            (PageMarginBoxSlot::LeftTop, [0.0, 40.0, 40.0, 146.67]),
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
fn fixed_height_side_boxes_sit_at_the_start_the_middle_and_the_end() {
    // 220px lie between the corners, from y = 40 to y = 260.
    let boxes = boxes_for(
        "@page { margin: 40px; \
           @right-top { content: ''; height: 60px } \
           @right-middle { content: ''; height: 60px } \
           @right-bottom { content: ''; height: 60px } }",
    );
    assert_eq!(
        rects(&boxes),
        [
            (PageMarginBoxSlot::RightTop, [360.0, 40.0, 40.0, 60.0]),
            (PageMarginBoxSlot::RightMiddle, [360.0, 120.0, 40.0, 60.0]),
            (PageMarginBoxSlot::RightBottom, [360.0, 200.0, 40.0, 60.0]),
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
    // it had one of 150px on each side and is centered at x = 200. Fixed
    // sides have no space to give, so the center box takes all that is left
    // of the 320px: 20px.
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
            (PageMarginBoxSlot::TopCenter, [190.0, 0.0, 20.0, 40.0]),
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
            (PageMarginBoxSlot::LeftBottom, [0.0, 230.0, 40.0, 30.0]),
            (PageMarginBoxSlot::LeftTop, [0.0, 40.0, 40.0, 20.0]),
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
            (PageMarginBoxSlot::LeftBottom, [10.0, 240.0, 20.0, 20.0]),
            (PageMarginBoxSlot::LeftTop, [20.0, 40.0, 20.0, 20.0]),
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
            (PageMarginBoxSlot::LeftBottom, [0.0, 100.0, 40.0, 160.0]),
            (PageMarginBoxSlot::LeftTop, [0.0, 40.0, 40.0, 60.0]),
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
    let px = |value: &ComputedLengthPercentageOrAuto| length_or_auto_px(value, 200.0);
    assert_eq!(px(&ComputedLengthPercentageOrAuto::Auto), None);
    assert_eq!(px(&ComputedLengthPercentageOrAuto::Px(5.0)), Some(5.0));
    assert_eq!(
        px(&ComputedLengthPercentageOrAuto::Percent(50.0)),
        Some(100.0)
    );
    assert_eq!(px(&ComputedLengthPercentageOrAuto::Px(-5.0)), Some(-5.0));
    assert_eq!(px(&ComputedLengthPercentageOrAuto::Px(f32::NAN)), Some(0.0));
    assert_eq!(px(&ComputedLengthPercentageOrAuto::MinContent), None);
    assert_eq!(px(&ComputedLengthPercentageOrAuto::MaxContent), None);
    assert_eq!(px(&ComputedLengthPercentageOrAuto::FitContent), None);
    let calc = |px, percent| {
        ComputedLengthPercentageOrAuto::Calc(raikiri_style::property::CalcLengthPercentage {
            px,
            percent,
        })
    };
    assert_eq!(px(&calc(10.0, 10.0)), Some(30.0));
    assert_eq!(px(&calc(f32::NAN, f32::NAN)), Some(0.0));
    assert_eq!(px(&calc(-50.0, 0.0)), Some(-50.0));
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
            (PageMarginBoxSlot::RightTop, (375.0, 48.0)),
            (PageMarginBoxSlot::LeftBottom, (15.0, 250.0 + 8.0)),
            (PageMarginBoxSlot::LeftMiddle, (15.0, 145.0 + 8.0)),
            (PageMarginBoxSlot::LeftTop, (15.0, 48.0)),
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
    // The keywords that compute to `none` count the same way as `none`.
    for keyword in ["initial", "unset"] {
        assert_eq!(
            content(&format!("counter-reset: {keyword}"), "", third),
            "2|0|6",
            "{keyword}"
        );
    }
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
    // The other CSS-wide keywords reset to `none` too, and a later one
    // supersedes an earlier `counter-reset` of the box.
    for keyword in ["initial", "unset", "revert"] {
        assert_eq!(
            content(
                "",
                &format!(
                    "counter-reset: chapter 4; counter-reset: {keyword}; counter-increment: chapter"
                ),
                third
            ),
            "3|1|6",
            "{keyword}"
        );
    }
}

#[test]
fn page_counter_base_comes_from_the_root_element_counter_reset() {
    let third = MarginBoxPageContext::new(2, 6, false);
    let content = |css: &str| {
        boxes_on_page(css, third)
            .first()
            .unwrap_or_else(|| panic!("no box for {css:?}"))
            .content
            .clone()
    };
    let page_rule = "@page { margin: 40px; @top-left { content: counter(page) } }";
    assert_eq!(content(page_rule), "3");
    assert_eq!(
        content(&format!("html {{ counter-reset: page 7 }} {page_rule}")),
        "10"
    );
    // The page context's own reset wins over the root element's, and applies
    // on every page (20 + 1), not once for the document.
    assert_eq!(
        content(
            "html { counter-reset: page 7 } \
             @page { margin: 40px; counter-reset: page 20; @top-left { content: counter(page) } }"
        ),
        "21"
    );
}

#[test]
fn page_counter_base_is_zero_when_the_root_element_has_no_reset() {
    // The first element child of the document is the root element, whatever
    // its name; one without a `counter-reset` leaves the base at zero.
    let mut doc = engine_document();
    let style = doc.append_element(Some(doc.root_index()), "style", None::<&str>);
    doc.append_text(
        style,
        "@page { margin: 40px; @top-left { content: counter(page) } }",
    );
    let rules = build_rule_tree(&doc);
    let styled = cascade(&doc, &rules).expect("cascade Ok");
    let boxes = page_margin_boxes(
        &doc,
        &styled,
        &styled.page,
        small_page(),
        MarginBoxPageContext::new(2, 6, false),
    );
    assert_eq!(boxes.first().unwrap().content, "3");

    // A document with no element at all falls back to the document node.
    let mut empty = engine_document();
    empty.append_text(empty.root_index(), "text only");
    let rules = build_rule_tree(&empty);
    let styled = cascade(&empty, &rules).expect("cascade Ok");
    assert_eq!(
        computed_counter_reset_value(&styled, "page"),
        0,
        "no element, no reset"
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
fn margin_box_quotes_follow_auto_and_the_css_wide_keywords() {
    let content = "content: open-quote 'q' close-quote no-open-quote no-close-quote";
    let page = |root: &str, page: &str, margin_box: &str| {
        single_content(&format!(
            "html {{ {root} }} @page {{ margin: 40px; {page} @top-left {{ {content}; {margin_box} }} }}"
        ))
    };
    // `auto` is the initial value, so it ends what an ancestor or the page set.
    assert_eq!(page("quotes: '[' ']'", "quotes: auto;", ""), "“q”");
    assert_eq!(page("", "quotes: none;", "quotes: auto"), "“q”");
    // `inherit` takes the page's value, `none` included, and `initial` and
    // `unset` on a box whose page set a list start over or inherit from it.
    assert_eq!(page("quotes: '[' ']'", "", "quotes: inherit"), "[q]");
    assert_eq!(page("quotes: none", "", "quotes: inherit"), "q");
    assert_eq!(page("quotes: '[' ']'", "", "quotes: initial"), "“q”");
    assert_eq!(page("quotes: '[' ']'", "", "quotes: unset"), "[q]");
    assert_eq!(page("", "quotes: '<' '>';", "quotes: inherit"), "<q>");
}

#[test]
fn margin_box_quotes_repeat_the_last_pair_and_nest_the_auto_marks() {
    let content =
        "content: open-quote open-quote open-quote 'x' close-quote close-quote close-quote";
    let page = |margin_box: &str| {
        single_content(&format!(
            "@page {{ margin: 40px; @top-left {{ {content}; {margin_box} }} }}"
        ))
    };
    // A depth past the last pair of a list repeats that pair.
    assert_eq!(page("quotes: '<' '>'"), "<<<x>>>");
    assert_eq!(page("quotes: '<' '>' '[' ']'"), "<[[x]]>");
    // `auto` nests the way body content does: the outer marks at depth zero
    // and the inner marks at every depth below it.
    assert_eq!(page("quotes: auto"), "“‘‘x’’”");
    assert_eq!(page("quotes: none"), "x");
}

#[test]
fn a_margin_box_close_quote_with_no_quote_open_inserts_nothing() {
    // A margin box counts its own quotes from zero, so a close with no open
    // before it in the same value is ignored and leaves the depth at zero.
    assert_eq!(
        single_content(
            "@page { margin: 40px; @top-left { content: close-quote 'x' open-quote close-quote; \
             quotes: '<' '>' } }"
        ),
        "x<>"
    );
}

#[test]
fn named_strings_and_running_elements_resolve_to_document_text() {
    let mut doc = engine_document();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let head = doc.append_element(Some(html), "head", Some("display:none"));
    let style = doc.append_element(Some(head), "style", None::<&str>);
    doc.append_text(
        style,
        "h1 { string-set: title 'Ch. ' content() } \
         div { position: running(hdr) } \
         @page { margin: 40px; \
           @top-left { content: string(title) } \
           @top-right { content: element(hdr) } \
           @bottom-left { content: string(missing) element(missing) attr(id) } }",
    );
    let body = doc.append_element(Some(html), "body", Some("display:block"));
    let h1 = doc.append_element(Some(body), "h1", None::<&str>);
    doc.append_text(h1, "  One \n Two ");
    let hdr = doc.append_element(Some(body), "div", None::<&str>);
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
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let head = doc.append_element(Some(html), "head", Some("display:none"));
    let style = doc.append_element(Some(head), "style", None::<&str>);
    doc.append_text(
        style,
        "h1 { string-set: title content() } \
         div { position: running(hdr) } \
         @page { margin: 40px; \
           @top-left { content: string(title) } \
           @top-right { content: element(hdr) } }",
    );
    let body = doc.append_element(Some(html), "body", Some("display:block"));
    let h1 = doc.append_element(Some(body), "h1", None::<&str>);
    doc.append_text(h1, " A\u{A0}\u{A0}B ");
    let hdr = doc.append_element(Some(body), "div", None::<&str>);
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
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let head = doc.append_element(Some(html), "head", Some("display:none"));
    let style = doc.append_element(Some(head), "style", None::<&str>);
    doc.append_text(
        style,
        "h1 { string-set: title content(), title 'Last ' content() } \
         div { position: running(hdr) } \
         @page { margin: 40px; \
           @top-left { content: string(title) } \
           @top-right { content: element(hdr) } }",
    );
    let body = doc.append_element(Some(html), "body", Some("display:block"));
    for (heading, running) in [("One", "First head"), ("Two", "Second head")] {
        let h1 = doc.append_element(Some(body), "h1", None::<&str>);
        doc.append_text(h1, heading);
        let hdr = doc.append_element(Some(body), "div", None::<&str>);
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
fn shared_named_values_read_the_attributes_and_counters_of_the_last_assignment() {
    let mut doc = engine_document();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let head = doc.append_element(Some(html), "head", Some("display:none"));
    let style = doc.append_element(Some(head), "style", None::<&str>);
    doc.append_text(
        style,
        "body { counter-reset: chapter } \
         h1 { counter-increment: chapter; \
              string-set: title 'Chapter ' counter(chapter) ': ' attr(data-title) } \
         @page { margin: 40px; @top-left { content: string(title) } }",
    );
    let body = doc.append_element(Some(html), "body", Some("display:block"));
    for title in ["Origins", "Aftermath"] {
        let h1 = doc.append_element(Some(body), "h1", None::<&str>);
        doc.set_element_attributes(h1, vec![(SmolStr::new("data-title"), SmolStr::new(title))]);
    }
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let contents: Vec<_> = layout_page(&doc, &cascade, small_page())
        .into_iter()
        .map(|margin_box| (margin_box.slot, margin_box.content))
        .collect();
    assert_eq!(
        contents,
        [(
            PageMarginBoxSlot::TopLeft,
            "Chapter 2: Aftermath".to_owned()
        )]
    );
}

/// `boxes_for` for a document that knows `green.png` is 100x50, as the pages of
/// the render pipeline do once its images are loaded.
fn boxes_with_images(css: &str) -> Vec<MarginBox> {
    let (mut doc, cascade) = page_cascade_fixture(css);
    doc.margin_box_images.insert(
        "green.png".to_owned(),
        raikiri_traits::ImageRasterSize {
            width: 100.0,
            height: 50.0,
        },
    );
    layout_page(&doc, &cascade, small_page())
}

fn image_rects(margin_box: &MarginBox) -> Vec<(&str, [f32; 4])> {
    margin_box
        .content_images
        .iter()
        .map(|image| {
            let rect = image.rect;
            (
                image.url.as_str(),
                [rect.x, rect.y, rect.width, rect.height].map(|v| (v * 100.0).round() / 100.0),
            )
        })
        .collect()
}

#[test]
fn a_content_image_follows_the_text_on_its_baseline() {
    let boxes = boxes_with_images(&format!(
        "@page {{ margin: 40px; @top-left {{ content: 'ab' url(green.png); \
           vertical-align: top; {AHEM} }} }}"
    ));
    let [margin_box] = boxes.as_slice() else {
        panic!("one box: {boxes:?}");
    };
    assert_eq!(margin_box.content, "ab");
    // The image is as large as itself and sits on the baseline of a line that
    // the image makes 50px tall above it, so the text drops to the baseline.
    assert_eq!(
        image_rects(margin_box),
        [("green.png", [60.0, 0.0, 100.0, 50.0])]
    );
    let text = margin_box.text.as_ref().expect("the text is shaped");
    let first_glyph = glyph_positions(text)[0];
    assert_eq!(first_glyph.0, 40.0);
    // The Ahem ascent of 10px text is 8px, over a baseline 50px down.
    assert_eq!(first_glyph.1, 50.0);
}

#[test]
fn a_content_image_alone_in_a_box_is_laid_out() {
    let boxes = boxes_with_images(&format!(
        "@page {{ margin: 40px; @top-left {{ content: url(green.png); \
           vertical-align: top; {AHEM} }} }}"
    ));
    let [margin_box] = boxes.as_slice() else {
        panic!("one box: {boxes:?}");
    };
    assert_eq!(margin_box.content, "");
    assert_eq!(
        image_rects(margin_box),
        [("green.png", [40.0, 0.0, 100.0, 50.0])]
    );
}

#[test]
fn a_content_image_moves_with_the_alignment_of_its_box() {
    // The line is 52px tall: the 50px of the image over the baseline, and the
    // 2px of the strut under it. The box is 40px tall.
    let image_y = |vertical_align: &str| {
        let boxes = boxes_with_images(&format!(
            "@page {{ margin: 40px; @top-left {{ content: url(green.png); \
               vertical-align: {vertical_align}; {AHEM} }} }}"
        ));
        let [margin_box] = boxes.as_slice() else {
            panic!("one box: {boxes:?}");
        };
        image_rects(margin_box)[0].1[1]
    };
    assert_eq!(image_y("top"), 0.0);
    assert_eq!(image_y("middle"), -6.0);
    assert_eq!(image_y("bottom"), -12.0);
    // Horizontally, `text-align` places the line of the image.
    let boxes = boxes_with_images(&format!(
        "@page {{ margin: 40px; @top-left {{ content: url(green.png); \
           text-align: right; width: 300px; {AHEM} }} }}"
    ));
    assert_eq!(image_rects(&boxes[0])[0].1[0], 240.0);
}

#[test]
fn content_images_keep_their_order_and_their_authored_urls() {
    let (mut doc, cascade) = page_cascade_fixture(&format!(
        "@page {{ margin: 40px; @top-left {{ content: url(a.png) 'x' url(b.png) 'y'; \
           vertical-align: top; {AHEM} }} }}"
    ));
    for (url, width, height) in [("a.png", 30.0, 20.0), ("b.png", 40.0, 10.0)] {
        doc.margin_box_images.insert(
            url.to_owned(),
            raikiri_traits::ImageRasterSize { width, height },
        );
    }
    let boxes = layout_page(&doc, &cascade, small_page());
    let [margin_box] = boxes.as_slice() else {
        panic!("one box: {boxes:?}");
    };
    assert_eq!(margin_box.content, "xy");
    // The images are 20px and 10px tall, so the line is 20px over the
    // baseline: 'x' follows the first image, and the second follows 'x'.
    assert_eq!(
        image_rects(margin_box),
        [
            ("a.png", [40.0, 0.0, 30.0, 20.0]),
            ("b.png", [80.0, 10.0, 40.0, 10.0]),
        ]
    );
}

#[test]
fn an_image_that_could_not_be_loaded_represents_nothing() {
    // The document knows no size for `green.png`, so the box is that of its
    // text alone.
    let with_image = boxes_for(&format!(
        "@page {{ margin: 40px; @top-left {{ content: 'ab' url(green.png); {AHEM} }} }}"
    ));
    let without = boxes_for(&format!(
        "@page {{ margin: 40px; @top-left {{ content: 'ab'; {AHEM} }} }}"
    ));
    let [margin_box] = with_image.as_slice() else {
        panic!("one box: {with_image:?}");
    };
    assert!(margin_box.content_images.is_empty());
    assert_eq!(rects(&with_image), rects(&without));
    // A box with nothing else shows nothing, but is still a box.
    let alone = boxes_for(&format!(
        "@page {{ margin: 40px; @top-left {{ content: url(green.png); \
           background: red; {AHEM} }} }}"
    ));
    assert_eq!(alone.len(), 1);
    assert!(alone[0].text.is_none());
    assert!(alone[0].content_images.is_empty());
}

#[test]
fn an_overflowing_content_image_keeps_its_intrinsic_extent() {
    let boxes = boxes_with_images(&format!(
        "@page {{ margin: 40px; @top-left {{ content: 'abcdefghij' url(green.png); \
           vertical-align: top; {AHEM} width: 20px; height: 20px }} }}"
    ));
    let [margin_box] = boxes.as_slice() else {
        panic!("one box: {boxes:?}");
    };
    assert!(margin_box.overflow_clip.is_none());
    // The box is 20px tall and sits against the page area, at y = 20. The
    // word fills a line of 10px, and the image that cannot fit beside it takes
    // the next one, wider and taller than the box.
    assert_eq!(
        image_rects(margin_box),
        [("green.png", [40.0, 30.0, 100.0, 50.0])]
    );
}

#[test]
fn a_clipping_box_clips_its_content_images() {
    let boxes = boxes_with_images(&format!(
        "@page {{ margin: 40px; @top-left {{ content: url(green.png); \
           overflow: hidden; width: 20px; {AHEM} }} }}"
    ));
    let [margin_box] = boxes.as_slice() else {
        panic!("one box: {boxes:?}");
    };
    assert!(margin_box.overflow_clip.is_some());
    assert_eq!(margin_box.content_images.len(), 1);
}

#[test]
fn a_content_image_counts_in_the_sizes_of_the_box_along_a_top_edge() {
    // The image is 100px wide after the 20px of 'ab', and cannot be broken.
    let row = |width: &str| {
        rects(&boxes_with_images(&format!(
            "@page {{ margin: 40px; {AHEM} \
               @top-left {{ content: 'ab' url(green.png); width: {width} }} \
               @top-right {{ content: 'x' }} }}"
        )))
    };
    for width in ["max-content", "min-content"] {
        let expected_width = if width == "max-content" { 120.0 } else { 100.0 };
        assert_eq!(
            row(width),
            [
                (
                    PageMarginBoxSlot::TopLeft,
                    [40.0, 0.0, expected_width, 40.0]
                ),
                (
                    PageMarginBoxSlot::TopRight,
                    [40.0 + expected_width, 0.0, 320.0 - expected_width, 40.0]
                ),
            ],
            "{width}"
        );
    }
    // A box with nothing but the image is as wide as the image.
    assert_eq!(
        rects(&boxes_with_images(&format!(
            "@page {{ margin: 40px; {AHEM} \
               @top-left {{ content: url(green.png); width: max-content }} }}"
        ))),
        [(PageMarginBoxSlot::TopLeft, [40.0, 0.0, 100.0, 40.0])]
    );
}

#[test]
fn a_content_image_counts_in_the_height_of_a_left_box() {
    // The line of an image is the 50px of the image over its baseline and the
    // 2px of the strut under it. In the 40px column of the box, the 'a' left of
    // the image goes on a line of its own, 10px tall.
    for (content, expected) in [("url(green.png)", 52.0), ("'a' url(green.png)", 62.0)] {
        for height in ["max-content", "min-content"] {
            assert_eq!(
                rects(&boxes_with_images(&format!(
                    "@page {{ margin: 40px; {AHEM} \
                       @left-top {{ content: {content}; height: {height} }} }}"
                ))),
                [(PageMarginBoxSlot::LeftTop, [0.0, 40.0, 40.0, expected])],
                "{content} {height}"
            );
        }
    }
}

#[test]
fn a_sizing_keyword_ignores_negative_margins_that_empty_the_outer_size() {
    // The outer size of the 10px line is below zero with these margins, but
    // the box still has the content of one line.
    let height = |keyword: &str| {
        let boxes = boxes_for(&format!(
            "@page {{ margin: 40px; {AHEM} \
               @top-left {{ content: 'x'; height: {keyword}; \
                 margin-top: -20px; margin-bottom: -20px }} }}"
        ));
        let [margin_box] = boxes.as_slice() else {
            panic!("one box: {boxes:?}");
        };
        margin_box.rect.height
    };
    for keyword in ["max-content", "min-content", "fit-content"] {
        assert_eq!(height(keyword), 10.0, "{keyword}");
    }
}

#[test]
fn a_box_sized_along_its_edge_keeps_its_size_whatever_its_negative_margins() {
    // The margins take 40px off a 10px box, so its outer size is below zero.
    // The border box is still the 10px the box asks for, and it sits where the
    // margins put it: 20px before the start of the strip, or of the column.
    for size in ["10px", "max-content"] {
        assert_eq!(
            rects(&boxes_for(&format!(
                "@page {{ margin: 40px; {AHEM} \
                   @top-left {{ content: 'x'; width: {size}; \
                     margin-left: -20px; margin-right: -20px }} }}"
            ))),
            [(PageMarginBoxSlot::TopLeft, [20.0, 0.0, 10.0, 40.0])],
            "width: {size}"
        );
        assert_eq!(
            rects(&boxes_for(&format!(
                "@page {{ margin: 40px; {AHEM} \
                   @left-top {{ content: 'x'; height: {size}; \
                     margin-top: -20px; margin-bottom: -20px }} }}"
            ))),
            [(PageMarginBoxSlot::LeftTop, [0.0, 20.0, 40.0, 10.0])],
            "height: {size}"
        );
    }
}

#[test]
fn a_centered_box_with_negative_margins_is_centered_by_its_negative_outer_size() {
    // The margin box is 30px inverted around the middle of the 320px strip, so
    // the 10px border box is centered on it too.
    assert_eq!(
        rects(&boxes_for(&format!(
            "@page {{ margin: 40px; {AHEM} \
               @top-center {{ content: 'x'; width: 10px; \
                 margin-left: -20px; margin-right: -20px }} }}"
        ))),
        [(PageMarginBoxSlot::TopCenter, [195.0, 0.0, 10.0, 40.0])]
    );
}

#[test]
fn a_box_next_to_one_with_negative_margins_takes_the_edge_space_they_free() {
    // The 10px box has a margin box of -30px, so the `auto` box beside it
    // takes the 320px of the strip and the 30px that box gives back, and the
    // two margin boxes meet where the first one ends.
    assert_eq!(
        rects(&boxes_for(&format!(
            "@page {{ margin: 40px; {AHEM} \
               @top-left {{ content: 'x'; width: 10px; \
                 margin-left: -20px; margin-right: -20px }} \
               @top-right {{ content: 'y' }} }}"
        ))),
        [
            (PageMarginBoxSlot::TopLeft, [20.0, 0.0, 10.0, 40.0]),
            (PageMarginBoxSlot::TopRight, [10.0, 0.0, 350.0, 40.0]),
        ]
    );
}

#[test]
fn named_values_follow_document_order_after_the_tree_changes() {
    // Moving the first heading after the second, and detaching a third,
    // leaves the arena order unlike the document order.
    let mut doc = engine_document();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let head = doc.append_element(Some(html), "head", Some("display:none"));
    let style = doc.append_element(Some(head), "style", None::<&str>);
    doc.append_text(
        style,
        "h1 { string-set: title content() } \
         div { position: running(hdr) } \
         @page { margin: 40px; \
           @top-left { content: string(title) } \
           @top-right { content: element(hdr) } }",
    );
    let body = doc.append_element(Some(html), "body", Some("display:block"));
    let mut moved = Vec::new();
    for (heading, running) in [("Last", "Last head"), ("First", "First head")] {
        let h1 = doc.append_element(Some(body), "h1", None::<&str>);
        doc.append_text(h1, heading);
        let hdr = doc.append_element(Some(body), "div", None::<&str>);
        doc.append_text(hdr, running);
        moved.extend([h1, hdr]);
    }
    for &node in &moved[..2] {
        doc.detach_from_parent(node);
        doc.append_child(body, node).unwrap();
    }
    let detached = doc.append_element(Some(body), "h1", None::<&str>);
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

#[test]
fn margin_box_clips_its_content_only_when_overflow_clips() {
    let clip_of = |declarations: &str| {
        let boxes = boxes_for(&format!(
            "@page {{ margin: 40px; @top-left-corner {{ content: 'x'; \
               border: 2px solid; {declarations} }} }}"
        ));
        let [margin_box] = boxes.as_slice() else {
            panic!("one box: {boxes:?}");
        };
        margin_box.overflow_clip
    };

    // The initial `overflow: visible` lets text and shadows overflow the box.
    assert_eq!(clip_of(""), None);

    // A clipping value clips to the padding box: inside the borders.
    let clip = clip_of("overflow: hidden").expect("hidden clips");
    assert_eq!(
        [clip.rect.x, clip.rect.y, clip.rect.width, clip.rect.height],
        [2.0, 2.0, 36.0, 36.0]
    );
    assert!(clip.clip_x && clip.clip_y);
    assert_eq!(clip.corner_radii, None);

    // Each axis clips on its own.
    let clip = clip_of("overflow-x: clip").expect("clip on x");
    assert!(clip.clip_x && !clip.clip_y);
}

#[test]
fn margin_box_overflow_clip_margin_moves_the_clip_edge() {
    let boxes = boxes_for(
        "@page { margin: 40px; @top-left-corner { content: 'x'; border: 2px solid; \
         overflow: clip; overflow-clip-margin: border-box 3px } }",
    );
    let [margin_box] = boxes.as_slice() else {
        panic!("one box: {boxes:?}");
    };
    let clip = margin_box.overflow_clip.expect("clip overflow");

    assert_eq!(
        [clip.rect.x, clip.rect.y, clip.rect.width, clip.rect.height],
        [-3.0, -3.0, 46.0, 46.0]
    );
}

/// Text properties that make lines 10px tall and keep line feeds.
const PRE_WRAP: &str = "line-height: 10px; white-space: pre-wrap;";

#[test]
fn a_fit_content_corner_box_with_auto_margins_is_centered_in_the_corner() {
    // Three lines of up to 'x x' (30px): 30px by 30px in a 40px corner.
    let boxes = boxes_for(&format!(
        "@page {{ margin: 40px; {AHEM} {PRE_WRAP} \
           @top-left-corner {{ content: 'x x\\a \\a0x\\a x x'; margin: auto; \
             width: fit-content; height: fit-content }} }}"
    ));
    assert_eq!(
        rects(&boxes),
        [(PageMarginBoxSlot::TopLeftCorner, [5.0, 5.0, 30.0, 30.0])]
    );
}

#[test]
fn width_keywords_size_a_box_along_a_top_edge_from_its_text() {
    // 'xxxx xx' is 70px on one line and 40px broken at its space. A box with
    // a size of its own leaves the rest of the 320px to the auto box.
    let row = |width: &str| {
        rects(&boxes_for(&format!(
            "@page {{ margin: 40px; {AHEM} \
               @top-left {{ content: 'xxxx xx'; width: {width} }} \
               @top-right {{ content: 'x' }} }}"
        )))
    };
    assert_eq!(
        row("max-content"),
        [
            (PageMarginBoxSlot::TopLeft, [40.0, 0.0, 70.0, 40.0]),
            (PageMarginBoxSlot::TopRight, [110.0, 0.0, 250.0, 40.0]),
        ]
    );
    assert_eq!(
        row("min-content"),
        [
            (PageMarginBoxSlot::TopLeft, [40.0, 0.0, 40.0, 40.0]),
            (PageMarginBoxSlot::TopRight, [80.0, 0.0, 280.0, 40.0]),
        ]
    );
}

#[test]
fn fit_content_keeps_the_space_between_the_min_and_max_content_sizes() {
    // 390px on one line, 40px broken at every space, in 320px less the margin.
    let long = "xxxx xxxx xxxx xxxx xxxx xxxx xxxx xxxx";
    let alone = |css: &str| {
        rects(&boxes_for(&format!(
            "@page {{ margin: 40px; {AHEM} @top-left {{ content: '{long}'; {css} }} }}"
        )))
    };
    assert_eq!(
        alone("width: fit-content; margin-left: 20px"),
        [(PageMarginBoxSlot::TopLeft, [60.0, 0.0, 300.0, 40.0])]
    );
    // The limit of fit-content() stands in for the room the box has, and
    // never goes below the min-content size.
    assert_eq!(
        alone("width: fit-content(100px)"),
        [(PageMarginBoxSlot::TopLeft, [40.0, 0.0, 100.0, 40.0])]
    );
    assert_eq!(
        alone("width: fit-content(30px)"),
        [(PageMarginBoxSlot::TopLeft, [40.0, 0.0, 40.0, 40.0])]
    );
    // Short text stays at its max-content size in any room.
    let short = rects(&boxes_for(&format!(
        "@page {{ margin: 40px; {AHEM} \
           @top-left {{ content: 'xxxx xx'; width: fit-content }} }}"
    )));
    assert_eq!(
        short,
        [(PageMarginBoxSlot::TopLeft, [40.0, 0.0, 70.0, 40.0])]
    );
}

#[test]
fn a_height_keyword_sizes_a_top_box_to_the_lines_at_its_width() {
    // 'xxx xxx' breaks into two lines in 40px, so the box is 20px tall; an
    // `auto` height would fill the 40px of the page margin. The auto bottom
    // margin leaves the 5px top margin as it is.
    let top = |height: &str| {
        rects(&boxes_for(&format!(
            "@page {{ margin: 40px; {AHEM} {PRE_WRAP} \
               @top-center {{ content: 'xxx xxx'; width: 40px; margin-top: 5px; \
                 margin-bottom: auto; height: {height} }} }}"
        )))
    };
    for height in ["min-content", "max-content", "fit-content"] {
        assert_eq!(
            top(height),
            [(PageMarginBoxSlot::TopCenter, [180.0, 5.0, 40.0, 20.0])],
            "height: {height}"
        );
    }
    assert_eq!(
        top("auto"),
        [(PageMarginBoxSlot::TopCenter, [180.0, 5.0, 40.0, 35.0])]
    );
}

#[test]
fn a_height_keyword_follows_the_width_the_edge_gives_the_box() {
    // The auto box takes the 50px the fixed one leaves of the 320px, which
    // hold 'xxxx xxxx' on two lines, so it is 20px tall and not as tall as
    // the page margin.
    let boxes = boxes_for(&format!(
        "@page {{ margin: 40px; {AHEM} {PRE_WRAP} \
           @top-left {{ content: 'xxxx xxxx'; height: max-content; margin-bottom: auto }} \
           @top-right {{ content: 'x'; width: 270px; height: max-content; \
             margin-bottom: auto }} }}"
    ));
    assert_eq!(
        rects(&boxes),
        [
            (PageMarginBoxSlot::TopLeft, [40.0, 0.0, 50.0, 20.0]),
            (PageMarginBoxSlot::TopRight, [90.0, 0.0, 270.0, 10.0]),
        ]
    );
}

#[test]
fn a_height_keyword_sizes_a_left_box_along_the_column() {
    // Two lines of 10px in the 40px column; the auto box below takes the
    // rest of the 220px between the corners.
    let boxes = boxes_for(&format!(
        "@page {{ margin: 40px; {AHEM} {PRE_WRAP} \
           @left-top {{ content: 'x\\a x'; height: max-content }} \
           @left-bottom {{ content: 'x' }} }}"
    ));
    assert_eq!(
        rects(&boxes),
        [
            (PageMarginBoxSlot::LeftBottom, [0.0, 60.0, 40.0, 200.0]),
            (PageMarginBoxSlot::LeftTop, [0.0, 40.0, 40.0, 20.0]),
        ]
    );
}

#[test]
fn a_width_keyword_sizes_a_left_box_across_the_column() {
    // 'xx' is 20px wide in the 40px column, so the box is pushed against the
    // page area like any box that is too narrow for its margins.
    let boxes = boxes_for(&format!(
        "@page {{ margin: 40px; {AHEM} @left-top {{ content: 'xx'; width: max-content }} }}"
    ));
    assert_eq!(
        rects(&boxes),
        [(PageMarginBoxSlot::LeftTop, [20.0, 40.0, 20.0, 220.0])]
    );
}

#[test]
fn sizing_keywords_follow_the_direction_the_text_runs_in() {
    // Vertical text runs along the height, so the 'xxxx' of the box is 40px
    // tall however wide the box becomes, and its three lines make columns 30px
    // wide in all.
    let tall = boxes_for(&format!(
        "@page {{ margin: 60px 40px; {AHEM} {PRE_WRAP} \
           @top-left {{ content: 'xxxx'; writing-mode: vertical-rl; \
             height: max-content; margin-bottom: auto }} }}"
    ));
    assert_eq!(
        rects(&tall),
        [(PageMarginBoxSlot::TopLeft, [40.0, 0.0, 320.0, 40.0])]
    );
    let columns = boxes_for(&format!(
        "@page {{ margin: 40px; {AHEM} {PRE_WRAP} \
           @top-left {{ content: 'x\\a x\\a x'; writing-mode: vertical-rl; \
             width: max-content }} }}"
    ));
    assert_eq!(
        rects(&columns),
        [(PageMarginBoxSlot::TopLeft, [40.0, 0.0, 30.0, 40.0])]
    );
}

#[test]
fn an_auto_center_box_takes_what_a_larger_fixed_side_box_leaves() {
    // In the 200px between the corners, the 40px fixed box is larger than the
    // 'x' box of its own size, even when that one shares out the space (to
    // 57.14px as a double). The sides count as one box of 80px, and the
    // center box gets the other 120px.
    let boxes = boxes_for(&format!(
        "@page {{ margin: 40px 100px; {AHEM} \
           @top-left {{ content: 'x' }} \
           @top-center {{ content: 'xxxxx' }} \
           @top-right {{ content: 'x'; width: 40px }} }}"
    ));
    assert_eq!(
        rects(&boxes),
        [
            (PageMarginBoxSlot::TopLeft, [100.0, 0.0, 40.0, 40.0]),
            (PageMarginBoxSlot::TopCenter, [140.0, 0.0, 120.0, 40.0]),
            (PageMarginBoxSlot::TopRight, [260.0, 0.0, 40.0, 40.0]),
        ]
    );
    // With 320px to share out, the same box doubled grows past 80px, and the
    // side boxes flex like the middle one: 'x' and 'xxxxx' split the space.
    let wide = boxes_for(&format!(
        "@page {{ margin: 40px; {AHEM} \
           @top-left {{ content: 'x' }} \
           @top-center {{ content: 'xxxxx' }} \
           @top-right {{ content: 'x'; width: 40px }} }}"
    ));
    assert_eq!(
        rects(&wide),
        [
            (PageMarginBoxSlot::TopLeft, [40.0, 0.0, 45.71, 40.0]),
            (PageMarginBoxSlot::TopCenter, [85.71, 0.0, 228.57, 40.0]),
            (PageMarginBoxSlot::TopRight, [320.0, 0.0, 40.0, 40.0]),
        ]
    );
}

#[test]
fn an_auto_center_box_shares_the_space_with_a_larger_auto_side_box() {
    // The 60px of the left text is more than the 20px of the fixed box, so
    // the sides count as one box of 120px that shares the 320px with the
    // 40px of the center text, in proportion: 240px and 80px.
    let boxes = boxes_for(&format!(
        "@page {{ margin: 40px; {AHEM} \
           @top-left {{ content: 'xxxxxx' }} \
           @top-center {{ content: 'xxxx' }} \
           @top-right {{ content: 'x'; width: 20px }} }}"
    ));
    assert_eq!(
        rects(&boxes),
        [
            (PageMarginBoxSlot::TopLeft, [40.0, 0.0, 120.0, 40.0]),
            (PageMarginBoxSlot::TopCenter, [160.0, 0.0, 80.0, 40.0]),
            (PageMarginBoxSlot::TopRight, [340.0, 0.0, 20.0, 40.0]),
        ]
    );
}

#[test]
fn percentages_of_a_corner_box_resolve_against_the_corner() {
    // The 40px by 40px corner is the containing block: 50% is 20px and 25% is
    // 10px, and the box is pushed against the page area at both its inner
    // sides like any box with no margin to spare.
    let boxes = boxes_for(&format!(
        "@page {{ margin: 40px; {AHEM} \
           @top-left-corner {{ content: 'x'; width: 50%; height: 25% }} }}"
    ));
    assert_eq!(
        rects(&boxes),
        [(PageMarginBoxSlot::TopLeftCorner, [20.0, 30.0, 20.0, 10.0])]
    );
    // The limit of fit-content() is a percentage of the corner as well: 20px
    // of the 80px the text takes on one line, which is 20px broken at spaces.
    let fit = boxes_for(&format!(
        "@page {{ margin: 40px; {AHEM} \
           @top-left-corner {{ content: 'xx xx xx'; width: fit-content(50%); \
             height: 10px }} }}"
    ));
    assert_eq!(
        rects(&fit),
        [(PageMarginBoxSlot::TopLeftCorner, [20.0, 30.0, 20.0, 10.0])]
    );
}

/// The flow-relative margins and sizes of a margin box follow its own
/// writing mode: in `vertical-rl` its block start is the right side, its
/// `block-size` the width and its `inline-size` the height.
#[test]
fn vertical_margin_box_flow_relative_properties_land_on_their_physical_sides() {
    let boxes = boxes_for(&format!(
        "@page {{ margin: 40px; {AHEM} \
           @left-middle {{ content: 'x'; writing-mode: vertical-rl; \
             block-size: 20px; inline-size: 30px; margin-block-start: 10px }} }}"
    ));
    // The 20px wide box keeps 10px to its right, so it ends 10px short of
    // the page area and starts at x = 10 of the 40px page margin.
    assert_eq!(
        rects(&boxes),
        [(PageMarginBoxSlot::LeftMiddle, [10.0, 135.0, 20.0, 30.0])]
    );
    // The page context's vertical writing mode reaches the margin boxes,
    // which declare their own over it.
    for (own, expected) in [
        ("", [10.0, 135.0, 20.0, 30.0]),
        // The 10px top margin goes above the 20px height of the box.
        ("writing-mode: horizontal-tb;", [10.0, 145.0, 30.0, 20.0]),
    ] {
        let boxes = boxes_for(&format!(
            "@page {{ margin: 40px; {AHEM} writing-mode: vertical-rl; \
               @left-middle {{ content: 'x'; {own} \
                 block-size: 20px; inline-size: 30px; margin-block-start: 10px }} }}"
        ));
        let [left] = boxes.as_slice() else {
            panic!("one box: {boxes:?}");
        };
        let rect = left.rect;
        assert_eq!([rect.x, rect.y, rect.width, rect.height], expected, "{own}");
    }
}

/// Vertical text thicker than the box overflows toward the block start when
/// it is aligned `top`, toward the block end when aligned `bottom`, and on
/// both sides when aligned `middle`.
#[test]
fn vertical_text_wider_than_its_box_overflows_away_from_its_alignment() {
    let origin_x = |align: &str| {
        let boxes = boxes_for(&format!(
            "@page {{ margin: 40px; {AHEM} \
               @left-middle {{ content: 'a\\A b\\A c'; white-space: pre; \
                 writing-mode: vertical-rl; width: 10px; vertical-align: {align} }} }}"
        ));
        let [left] = boxes.as_slice() else {
            panic!("one box: {boxes:?}");
        };
        (
            left.rect.x,
            left.text.as_ref().expect("shaped text").origin().0,
        )
    };
    // The 10px box sits against the page area, from x = 30, and the three
    // 10px lines are 20px thicker than it.
    assert_eq!(origin_x("top"), (30.0, 10.0));
    assert_eq!(origin_x("middle"), (30.0, 20.0));
    assert_eq!(origin_x("bottom"), (30.0, 30.0));
}

/// Horizontal text taller than its box overflows the same way.
#[test]
fn horizontal_text_taller_than_its_box_overflows_away_from_its_alignment() {
    let origin_y = |align: &str| {
        let boxes = boxes_for(&format!(
            "@page {{ margin: 40px; {AHEM} \
               @top-left {{ content: 'a\\A b'; white-space: pre; \
                 height: 10px; vertical-align: {align} }} }}"
        ));
        let [top] = boxes.as_slice() else {
            panic!("one box: {boxes:?}");
        };
        (
            top.rect.y,
            top.text.as_ref().expect("shaped text").origin().1,
        )
    };
    // The 10px box sits against the page area, at y = 30, and the two 10px
    // lines are 10px taller than it.
    assert_eq!(origin_y("top"), (30.0, 30.0));
    assert_eq!(origin_y("middle"), (30.0, 25.0));
    assert_eq!(origin_y("bottom"), (30.0, 20.0));
}
