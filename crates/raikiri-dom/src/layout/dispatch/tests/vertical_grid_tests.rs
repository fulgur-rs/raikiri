//! Grid containers in vertical writing modes, laid out on the axes of their
//! own writing mode.
//!
//! In `vertical-rl` the rows of a grid run along the block axis, which is the
//! physical horizontal axis from right to left, and its columns along the
//! inline axis, which is the physical vertical axis from top to bottom
//! (CSS Writing Modes 4 §7.1, CSS Grid 1 §2).

use crate::Document;
use crate::layout::layout_single_page;
use crate::layout::test_support::{absolute_rect, ifc_ahem_fonts, page_box_800x600, with_ahem};

type Rect = (f32, f32, f32, f32);

/// A laid-out document: a grid in a body with an 8px margin, so that the grid
/// starts at (8, 8).
struct Laid {
    doc: Document,
    grid: usize,
    children: Vec<usize>,
}

impl Laid {
    /// The border box of the grid as (x, y, width, height) on the page.
    fn grid_rect(&self) -> Rect {
        absolute_rect(&self.doc, self.grid)
    }

    /// The border boxes of the children, in source order.
    fn rects(&self) -> Vec<Rect> {
        self.children
            .iter()
            .map(|&child| absolute_rect(&self.doc, child))
            .collect()
    }
}

/// Lay out a grid styled `grid_css` with one `display:block` child per entry of
/// `children`, styled with the part of the entry before a `|`. The part after
/// it is the Ahem text of the child.
fn lay_out(grid_css: &str, children: &[&str]) -> Laid {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block;margin:8px"));
    let grid = doc.append_element(
        Some(body),
        "div",
        Some(
            format!("display:grid;font-family:Ahem;font-size:10px;line-height:10px;{grid_css}")
                .as_str(),
        ),
    );
    let children = children
        .iter()
        .map(|entry| {
            let (css, text) = entry.split_once('|').unwrap_or((entry, ""));
            let child = doc.append_element(
                Some(grid),
                "div",
                Some(format!("display:block;{css}").as_str()),
            );
            if !text.is_empty() {
                doc.append_text(child, text);
            }
            child
        })
        .collect();
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    Laid {
        doc,
        grid,
        children,
    }
}

/// A 120x200 grid: two columns of 50px and 70px along the inline axis (200px),
/// two rows of 40px and 30px along the block axis (120px).
const TRACKS: &str =
    "width:120px;height:200px;grid-template-columns:50px 70px;grid-template-rows:40px 30px";

const FOUR: [&str; 4] = ["", "", "", ""];

#[test]
fn vertical_rl_rows_run_from_the_right_and_columns_from_the_top() {
    let laid = lay_out(&format!("writing-mode:vertical-rl;{TRACKS}"), &FOUR);
    assert_eq!(laid.grid_rect(), (8.0, 8.0, 120.0, 200.0));
    // Row 1 is the rightmost 40px, row 2 the next 30px; column 1 is the top 50px.
    assert_eq!(
        laid.rects(),
        [
            (88.0, 8.0, 40.0, 50.0),
            (88.0, 58.0, 40.0, 70.0),
            (58.0, 8.0, 30.0, 50.0),
            (58.0, 58.0, 30.0, 70.0),
        ]
    );
}

#[test]
fn vertical_lr_rows_run_from_the_left() {
    let laid = lay_out(&format!("writing-mode:vertical-lr;{TRACKS}"), &FOUR);
    assert_eq!(
        laid.rects(),
        [
            (8.0, 8.0, 40.0, 50.0),
            (8.0, 58.0, 40.0, 70.0),
            (48.0, 8.0, 30.0, 50.0),
            (48.0, 58.0, 30.0, 70.0),
        ]
    );
}

#[test]
fn sideways_modes_run_their_lines_like_the_vertical_ones_in_the_block_axis() {
    // `sideways-rl` runs lines top to bottom like `vertical-rl`; `sideways-lr`
    // runs them bottom to top, so its columns start at the bottom.
    let rl = lay_out(&format!("writing-mode:sideways-rl;{TRACKS}"), &FOUR);
    assert_eq!(rl.rects()[0], (88.0, 8.0, 40.0, 50.0));
    assert_eq!(rl.rects()[3], (58.0, 58.0, 30.0, 70.0));
    let lr = lay_out(&format!("writing-mode:sideways-lr;{TRACKS}"), &FOUR);
    assert_eq!(
        lr.rects(),
        [
            (8.0, 158.0, 40.0, 50.0),
            (8.0, 88.0, 40.0, 70.0),
            (48.0, 158.0, 30.0, 50.0),
            (48.0, 88.0, 30.0, 70.0),
        ]
    );
}

#[test]
fn the_inline_direction_reverses_the_columns() {
    let laid = lay_out(
        &format!("writing-mode:vertical-rl;direction:rtl;{TRACKS}"),
        &FOUR,
    );
    // Column 1 is the bottom 50px.
    assert_eq!(
        laid.rects(),
        [
            (88.0, 158.0, 40.0, 50.0),
            (88.0, 88.0, 40.0, 70.0),
            (58.0, 158.0, 30.0, 50.0),
            (58.0, 88.0, 30.0, 70.0),
        ]
    );
}

#[test]
fn gaps_and_padding_follow_the_axes() {
    // `row-gap` separates the rows along the block axis, `column-gap` the
    // columns along the inline axis. The top and bottom padding is 5px, the
    // left and right 7px.
    let laid = lay_out(
        &format!("writing-mode:vertical-rl;padding:5px 7px;gap:10px 4px;{TRACKS}"),
        &FOUR,
    );
    assert_eq!(laid.grid_rect(), (8.0, 8.0, 134.0, 210.0));
    assert_eq!(
        laid.rects(),
        [
            (95.0, 13.0, 40.0, 50.0),
            (95.0, 67.0, 40.0, 70.0),
            (55.0, 13.0, 30.0, 50.0),
            (55.0, 67.0, 30.0, 70.0),
        ]
    );
}

#[test]
fn auto_tracks_size_to_content_and_stretch_along_their_own_axes() {
    // Column 1 holds 40px and 60px of text along the inline axis and column 2
    // 20px and 10px; the rows hold one 10px line each. The 120px of free
    // inline space and the 100px of free block space are shared equally by
    // the two auto tracks of an axis.
    let laid = lay_out(
        "writing-mode:vertical-rl;width:120px;height:200px;grid-template-columns:auto auto;grid-template-rows:auto auto",
        &["|aaaa", "|bb", "|cccccc", "|d"],
    );
    assert_eq!(
        laid.rects(),
        [
            (68.0, 8.0, 60.0, 120.0),
            (68.0, 128.0, 60.0, 80.0),
            (8.0, 8.0, 60.0, 120.0),
            (8.0, 128.0, 60.0, 80.0),
        ]
    );
}

#[test]
fn fr_tracks_divide_the_free_space_of_each_axis() {
    let laid = lay_out(
        "writing-mode:vertical-rl;width:120px;height:200px;grid-template-columns:1fr 3fr;grid-template-rows:1fr 2fr",
        &FOUR,
    );
    assert_eq!(
        laid.rects(),
        [
            (88.0, 8.0, 40.0, 50.0),
            (88.0, 58.0, 40.0, 150.0),
            (8.0, 8.0, 80.0, 50.0),
            (8.0, 58.0, 80.0, 150.0),
        ]
    );
}

#[test]
fn item_alignment_uses_the_block_axis_for_align_and_the_inline_axis_for_justify() {
    // `align-items` centers in the block axis (the physical x axis), and
    // `justify-items: end` puts the item at the end of the inline axis (the
    // bottom of its column).
    let laid = lay_out(
        "writing-mode:vertical-rl;width:120px;height:200px;grid-template-columns:100px 100px;grid-template-rows:60px 60px;align-items:center;justify-items:end",
        &["width:20px;height:30px", "width:20px;height:30px"],
    );
    assert_eq!(
        laid.rects(),
        [(88.0, 78.0, 20.0, 30.0), (88.0, 178.0, 20.0, 30.0)]
    );
}

#[test]
fn content_alignment_uses_the_inline_axis_for_justify_and_the_block_axis_for_align() {
    // The columns (100px of 200px) are centered along the inline axis; the
    // rows (70px of 120px) move to the block-end side, the left.
    let laid = lay_out(
        "writing-mode:vertical-rl;width:120px;height:200px;grid-template-columns:50px 50px;grid-template-rows:40px 30px;justify-content:center;align-content:end",
        &FOUR,
    );
    assert_eq!(
        laid.rects(),
        [
            (38.0, 58.0, 40.0, 50.0),
            (38.0, 108.0, 40.0, 50.0),
            (8.0, 58.0, 30.0, 50.0),
            (8.0, 108.0, 30.0, 50.0),
        ]
    );
}

#[test]
fn spans_and_column_auto_flow_follow_the_logical_rows_and_columns() {
    let laid = lay_out(
        &format!("writing-mode:vertical-rl;grid-auto-flow:column;{TRACKS}"),
        &["grid-row:1 / span 2", "", ""],
    );
    assert_eq!(
        laid.rects(),
        [
            (58.0, 8.0, 70.0, 50.0),
            (88.0, 58.0, 40.0, 70.0),
            (58.0, 58.0, 30.0, 70.0),
        ]
    );
}

#[test]
fn implicit_rows_are_added_on_the_block_end_side() {
    let laid = lay_out(
        "writing-mode:vertical-rl;width:200px;height:100px;grid-template-columns:50px 50px;grid-auto-rows:30px",
        &["", "", "", "", ""],
    );
    assert_eq!(
        laid.rects(),
        [
            (178.0, 8.0, 30.0, 50.0),
            (178.0, 58.0, 30.0, 50.0),
            (148.0, 8.0, 30.0, 50.0),
            (148.0, 58.0, 30.0, 50.0),
            (118.0, 8.0, 30.0, 50.0),
        ]
    );
}

#[test]
fn item_margins_are_placed_by_the_axes_of_the_grid() {
    // The item hugs the block-start (right) and inline-start (top) edges of
    // its area by its margin box: 5px from the right, 3px from the top.
    let area = "writing-mode:vertical-rl;width:120px;height:200px;grid-template-columns:100px;grid-template-rows:100px";
    let laid = lay_out(area, &["margin:3px 5px 7px 11px;width:20px;height:20px"]);
    assert_eq!(laid.rects(), [(103.0, 11.0, 20.0, 20.0)]);
    // Auto margins center the item in both axes.
    let laid = lay_out(area, &["margin:auto;width:20px;height:20px"]);
    assert_eq!(laid.rects(), [(68.0, 48.0, 20.0, 20.0)]);
}

#[test]
fn percentage_sizes_resolve_against_the_grid_area_of_each_axis() {
    // The area is 60px along the block axis (the width) and 100px along the
    // inline axis (the height).
    let laid = lay_out(
        "writing-mode:vertical-rl;width:120px;height:200px;grid-template-columns:100px;grid-template-rows:60px",
        &["width:50%;height:50%"],
    );
    assert_eq!(laid.rects(), [(98.0, 8.0, 30.0, 50.0)]);
}

#[test]
fn a_grid_nested_in_a_vertical_grid_lays_out_in_its_own_writing_mode() {
    let nested = "grid-template-columns:30px 40px;grid-template-rows:10px 20px";
    let rows = "writing-mode:vertical-rl;width:200px;height:200px;grid-template-columns:100px 100px;grid-template-rows:100px";
    // A horizontal grid is an orthogonal flow in a vertical one; both stretch
    // to the area, which is 100px by 100px.
    let laid = lay_out(
        rows,
        &[&format!("display:grid;writing-mode:horizontal-tb;{nested}")],
    );
    assert_eq!(laid.rects(), [(108.0, 8.0, 100.0, 100.0)]);
    let laid = lay_out(rows, &[&format!("display:grid;{nested}")]);
    assert_eq!(laid.rects(), [(108.0, 8.0, 100.0, 100.0)]);
}

#[test]
fn an_orthogonal_item_sizes_the_rows_by_its_inline_size() {
    // A horizontal item's inline size runs along the grid's block axis. Its
    // max-content width (9 glyphs) is the row; its min-content width (4) is
    // the row when the grid is sized to its min-content.
    let item = &["writing-mode:horizontal-tb|aaaa bbbb"];
    let laid = lay_out(
        "writing-mode:vertical-rl;width:max-content;height:100px;grid-template-columns:100px;grid-template-rows:auto",
        item,
    );
    assert_eq!(laid.grid_rect(), (8.0, 8.0, 90.0, 100.0));
    assert_eq!(laid.rects(), [(8.0, 8.0, 90.0, 100.0)]);
    let laid = lay_out(
        "writing-mode:vertical-rl;width:min-content;height:100px;grid-template-columns:100px;grid-template-rows:auto",
        item,
    );
    assert_eq!(laid.grid_rect(), (8.0, 8.0, 40.0, 100.0));
    assert_eq!(laid.rects(), [(8.0, 8.0, 40.0, 100.0)]);
}

#[test]
fn orthogonal_items_in_separate_rows_add_up_in_the_block_axis() {
    let laid = lay_out(
        "writing-mode:vertical-rl;width:max-content;height:100px;grid-template-columns:100px;grid-template-rows:auto auto",
        &[
            "writing-mode:horizontal-tb|aaaa bbbb",
            "writing-mode:horizontal-tb|cc",
        ],
    );
    // 90px and 20px of text.
    assert_eq!(laid.grid_rect(), (8.0, 8.0, 110.0, 100.0));
    assert_eq!(
        laid.rects(),
        [(28.0, 8.0, 90.0, 100.0), (8.0, 8.0, 20.0, 100.0)]
    );
}

#[test]
fn an_orthogonal_item_that_is_not_stretched_fits_its_content_to_the_row() {
    // The row is 150px wide, wider than the 90px the text needs: the item
    // takes the text's width and one line of height.
    let laid = lay_out(
        "writing-mode:vertical-rl;width:200px;height:200px;grid-template-columns:100px;grid-template-rows:150px;align-items:start;justify-items:start",
        &["writing-mode:horizontal-tb|aaaa bbbb"],
    );
    assert_eq!(laid.rects(), [(118.0, 8.0, 90.0, 10.0)]);
    // A 60px row wraps the text into two lines.
    let laid = lay_out(
        "writing-mode:vertical-rl;width:120px;height:200px;grid-template-columns:100px;grid-template-rows:60px;align-items:start;justify-items:start",
        &["writing-mode:horizontal-tb|aaaa bbbb"],
    );
    assert_eq!(laid.rects(), [(68.0, 8.0, 60.0, 20.0)]);
}

#[test]
fn keyword_limits_of_an_orthogonal_item_resolve_against_the_area_on_physical_axes() {
    // `fit-content(50%)` of the 60px row (the area's width) is 30px, which the
    // 40px min-content width of the text exceeds. The 100px of the area's
    // height would have given 50px.
    let laid = lay_out(
        "writing-mode:vertical-rl;width:120px;height:200px;grid-template-columns:100px;grid-template-rows:60px",
        &["writing-mode:horizontal-tb;max-width:fit-content(50%)|aaaa bbbb cccc"],
    );
    assert_eq!(laid.rects(), [(88.0, 8.0, 40.0, 100.0)]);
}

#[test]
fn pagination_metadata_is_recorded_in_grid_rows_and_columns() {
    // The first item is in row 2 and column 1, the second in row 1 and column
    // 2: the rows and columns are the grid's, not the page's.
    let laid = lay_out(
        &format!("writing-mode:vertical-rl;{TRACKS}"),
        &["grid-row:2;grid-column:1", "grid-row:1;grid-column:2"],
    );
    assert_eq!(
        laid.rects(),
        [(58.0, 8.0, 30.0, 50.0), (88.0, 58.0, 40.0, 70.0)]
    );
    let grid = &laid.doc.nodes[laid.grid];
    let [first, second] = [laid.children[0], laid.children[1]];
    assert_eq!(&*grid.grid_item_row_starts, [(first, 2), (second, 1)]);
    assert_eq!(&*grid.grid_item_column_starts, [(first, 1), (second, 2)]);
    assert_eq!(grid.grid_column_count, 2);
}

#[test]
fn an_item_with_a_percentage_block_size_in_an_auto_row_has_no_baseline() {
    // The block size of an item is its width here. A percentage height is an
    // inline size and does not make the row depend on the item.
    let laid = lay_out(
        "writing-mode:vertical-rl;width:120px;height:200px;grid-template-columns:100px 100px;grid-template-rows:auto",
        &["width:50%", "height:50%"],
    );
    let grid = &laid.doc.nodes[laid.grid];
    assert_eq!(&*grid.grid_items_without_baseline, [laid.children[0]]);
}

/// The border box of an absolutely positioned child styled `child_css` of a
/// positioned `TRACKS` grid styled `grid_css`.
fn abspos(grid_css: &str, child_css: &str) -> Rect {
    let laid = lay_out(
        &format!("position:relative;{grid_css};{TRACKS}"),
        &[&format!("position:absolute;{child_css}")],
    );
    laid.rects()[0]
}

#[test]
fn a_static_position_is_aligned_by_the_axes_of_the_grid() {
    let size = "width:30px;height:10px";
    // `align-items` places the box in the block axis (centered: 45px from the
    // right edge), `justify-items` in the inline axis (at the bottom).
    assert_eq!(
        abspos(
            "writing-mode:vertical-rl;align-items:center;justify-items:end",
            size
        ),
        (53.0, 198.0, 30.0, 10.0)
    );
    // Without alignment the box is at the block-start (right) and inline-start
    // (top) corner.
    assert_eq!(
        abspos("writing-mode:vertical-rl", size),
        (98.0, 8.0, 30.0, 10.0)
    );
    assert_eq!(
        abspos("writing-mode:vertical-lr", size),
        (8.0, 8.0, 30.0, 10.0)
    );
    // The inline-start edge is the bottom edge under `direction: rtl`.
    assert_eq!(
        abspos("writing-mode:vertical-rl;direction:rtl", size),
        (98.0, 198.0, 30.0, 10.0)
    );
}

#[test]
fn a_placed_box_is_laid_out_in_its_grid_area() {
    // `grid-column: 2` starts at the second column and `grid-row: 1` at the
    // first row; the end of each is the padding edge.
    assert_eq!(
        abspos(
            "writing-mode:vertical-rl",
            "width:30px;height:10px;grid-column:2;grid-row:1;top:0;left:0;right:0;bottom:0",
        ),
        (98.0, 58.0, 30.0, 10.0)
    );
    // A box with every inset zero and no size fills its area: column 1 is the
    // top 50px, row 2 the 30px left of the 40px first row.
    let fill = "grid-column:1 / 2;grid-row:2 / 3;top:0;left:0;right:0;bottom:0";
    assert_eq!(
        abspos("writing-mode:vertical-rl", fill),
        (58.0, 8.0, 30.0, 50.0)
    );
    assert_eq!(
        abspos("writing-mode:vertical-lr", fill),
        (48.0, 8.0, 30.0, 50.0)
    );
    assert_eq!(
        abspos("writing-mode:vertical-rl;direction:rtl", fill),
        (58.0, 158.0, 30.0, 50.0)
    );
}

#[test]
fn a_placed_box_with_auto_insets_keeps_its_static_position_in_the_grid_area() {
    // Column 2 and row 2 hold the box at their block-start and inline-start
    // corner: the right edge of row 2 and the top of column 2.
    assert_eq!(
        abspos(
            "writing-mode:vertical-rl",
            "width:30px;height:10px;grid-column:2;grid-row:2"
        ),
        (58.0, 58.0, 30.0, 10.0)
    );
}

#[test]
fn a_placed_fixed_box_is_not_laid_out_by_the_grid() {
    // The grid is not the containing block of a fixed box, so the box keeps
    // out of its grid area and is placed against the page, while the absolute
    // box beside it is placed in the area.
    let laid = lay_out(
        &format!("position:relative;writing-mode:vertical-rl;{TRACKS}"),
        &[
            "position:fixed;width:30px;height:10px;grid-column:2;grid-row:2;left:3px;top:4px",
            "position:absolute;width:30px;height:10px;grid-column:2;grid-row:2",
        ],
    );
    assert_eq!(
        laid.rects(),
        [(3.0, 4.0, 30.0, 10.0), (58.0, 58.0, 30.0, 10.0)]
    );
}

#[test]
fn insets_and_alignment_of_a_placed_box_use_its_grid_area() {
    // Both insets are definite in each axis, so `align-self` and
    // `justify-self` place the box in the area: centered along the block axis
    // of row 2 and at the end of column 2's inline axis.
    assert_eq!(
        abspos(
            "writing-mode:vertical-rl",
            "width:20px;height:10px;grid-column:2 / 3;grid-row:2 / 3;top:0;left:0;right:0;bottom:0;align-self:center;justify-self:end",
        ),
        (63.0, 118.0, 20.0, 10.0)
    );
}

#[test]
fn the_scrollable_overflow_covers_boxes_placed_by_the_grid_axes() {
    // The box reaches 20px past the bottom (the inline end) of the grid, and
    // 15px past its left (the block end). There is no other content.
    let laid = lay_out(
        &format!("position:relative;writing-mode:vertical-rl;{TRACKS}"),
        &["position:absolute;width:30px;height:30px;right:105px;top:190px"],
    );
    assert_eq!(laid.rects(), [(-7.0, 198.0, 30.0, 30.0)]);
    let overflow = laid.doc.nodes[laid.grid].layout.scrollable_overflow_rect;
    assert_eq!(
        (overflow.left, overflow.top, overflow.right, overflow.bottom),
        (-15.0, 0.0, 15.0, 220.0)
    );
}

#[test]
fn the_scrollable_overflow_of_the_tracks_follows_the_block_direction() {
    // Two 80px rows fill 160px of the 120px block axis. The second row lies
    // 40px past the block end: to the left of the box in `vertical-rl`, to the
    // right of it in `vertical-lr`.
    let overflow = |mode: &str| {
        let laid = lay_out(
            &format!(
                "writing-mode:{mode};width:120px;height:200px;\
                 grid-template-columns:50px;grid-template-rows:80px 80px"
            ),
            &["", ""],
        );
        let overflow = laid.doc.nodes[laid.grid].layout.scrollable_overflow_rect;
        (overflow.left, overflow.top, overflow.right, overflow.bottom)
    };
    assert_eq!(overflow("vertical-rl"), (-40.0, 0.0, 120.0, 50.0));
    assert_eq!(overflow("vertical-lr"), (0.0, 0.0, 160.0, 50.0));
}
