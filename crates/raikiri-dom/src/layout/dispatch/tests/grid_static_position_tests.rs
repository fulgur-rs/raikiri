//! The static position of an absolutely positioned child of a grid container
//! that is its containing block (CSS Grid 1 §9.1, §9.2).
//!
//! Every expectation is the position of the child relative to the border box
//! of the grid, measured in Chromium.

use crate::Document;
use crate::layout::layout_single_page;
use crate::layout::test_support::{absolute_rect, page_box_800x600};

/// A grid whose padding box starts at (3, 3) and ends at (213, 193) in its
/// 216x196 border box, with the content box inset by the padding from there.
/// Its two columns are 50px wide and its two rows 40px high.
const GRID: &str = "display:grid;position:relative;width:150px;height:150px;\
    padding:10px 20px 30px 40px;border:3px solid;\
    grid-template-columns:50px 50px;grid-template-rows:40px 40px";

/// The border box of the absolutely positioned `child_css` box in the grid
/// styled `grid_css`, as (x, y, width, height) from the border box of the grid.
fn laid_out(grid_css: &str, child_css: &str) -> (f32, f32, f32, f32) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block;margin:8px"));
    let grid = doc.append_element(Some(body), "div", Some(grid_css));
    let child = doc.append_element(
        Some(grid),
        "div",
        Some(format!("position:absolute;width:20px;height:20px;{child_css}").as_str()),
    );
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    layout_single_page(&mut doc, &cascade, page_box_800x600()).expect("layout");
    let (grid_x, grid_y, ..) = absolute_rect(&doc, grid);
    let (x, y, width, height) = absolute_rect(&doc, child);
    (x - grid_x, y - grid_y, width, height)
}

fn at(x: f32, y: f32) -> (f32, f32, f32, f32) {
    (x, y, 20.0, 20.0)
}

#[test]
fn a_box_without_grid_lines_starts_at_the_padding_edge() {
    assert_eq!(laid_out(GRID, ""), at(3.0, 3.0));
}

#[test]
fn a_line_that_is_not_auto_starts_its_axis_at_the_track() {
    // The row is still `auto`, which stays at the padding edge.
    assert_eq!(laid_out(GRID, "grid-column:1"), at(43.0, 3.0));
    assert_eq!(laid_out(GRID, "grid-row:1"), at(3.0, 13.0));
    assert_eq!(laid_out(GRID, "grid-column:2;grid-row:2"), at(93.0, 53.0));
    assert_eq!(laid_out(GRID, "grid-column:1 / auto"), at(43.0, 3.0));
    assert_eq!(laid_out(GRID, "grid-column:auto / 2"), at(3.0, 3.0));
    assert_eq!(laid_out(GRID, "grid-column:2 / auto"), at(93.0, 3.0));
    assert_eq!(laid_out(GRID, "grid-column:1 / span 2"), at(43.0, 3.0));
}

#[test]
fn alignment_follows_the_area_of_the_lines() {
    let end = format!("{GRID};justify-items:end;align-items:end");
    // Both lines of the column are tracks: the area ends at the second track.
    assert_eq!(laid_out(&end, "grid-column:1 / 3"), at(123.0, 173.0));
    // An `auto` end line ends the area at the padding edge.
    assert_eq!(laid_out(&end, "grid-row:1"), at(193.0, 173.0));
    assert_eq!(laid_out(&end, "grid-column:1"), at(193.0, 173.0));
    assert_eq!(laid_out(&end, ""), at(193.0, 193.0 - 20.0));
    assert_eq!(
        laid_out(GRID, "justify-self:end;align-self:center"),
        at(193.0, 88.0)
    );
}

#[test]
fn an_inset_keeps_the_static_position_of_the_other_axis() {
    // An inset is measured from the padding edge.
    assert_eq!(laid_out(GRID, "left:5px"), at(8.0, 3.0));
    assert_eq!(laid_out(GRID, "top:5px"), at(3.0, 8.0));
}

#[test]
fn the_inline_axis_of_the_grid_decides_the_edge() {
    let rtl = format!("{GRID};direction:rtl");
    assert_eq!(laid_out(&rtl, ""), at(193.0, 3.0));
    assert_eq!(
        laid_out(&format!("{rtl};justify-items:end"), ""),
        at(3.0, 3.0)
    );
}

#[test]
fn a_grid_that_is_not_the_containing_block_uses_its_content_box() {
    let plain = GRID.replace("position:relative;", "");
    assert_eq!(laid_out(&plain, ""), at(43.0, 13.0));
    // The grid lines do not apply: the grid is not the containing block.
    assert_eq!(laid_out(&plain, "grid-column:1"), at(43.0, 13.0));
}

#[test]
fn a_scroll_container_starts_at_the_padding_edge_too() {
    assert_eq!(
        laid_out(&format!("{GRID};overflow:scroll"), ""),
        at(3.0, 3.0)
    );
}

#[test]
fn a_vertical_grid_starts_at_its_block_start_padding_edge() {
    // The block-start edge of `vertical-rl` is the right padding edge, the
    // inline-start edge the top one.
    assert_eq!(
        laid_out(&format!("{GRID};writing-mode:vertical-rl"), ""),
        at(193.0, 3.0)
    );
    assert_eq!(
        laid_out(&format!("{GRID};writing-mode:vertical-lr"), ""),
        at(3.0, 3.0)
    );
    assert_eq!(
        laid_out(
            &format!("{GRID};writing-mode:vertical-rl;align-items:end;justify-items:end"),
            ""
        ),
        at(3.0, 173.0)
    );
}

#[test]
fn the_margin_box_is_aligned_in_the_padding_box() {
    // The margin box is 30px wide and 24px high; the box sits 6px and 2px in.
    assert_eq!(
        laid_out(GRID, "margin:2px 4px 6px 6px"),
        at(3.0 + 6.0, 3.0 + 2.0)
    );
    assert_eq!(
        laid_out(
            &format!("{GRID};justify-items:end;align-items:end"),
            "margin:2px 4px 6px 6px"
        ),
        at(213.0 - 4.0 - 20.0, 193.0 - 6.0 - 20.0)
    );
}

#[test]
fn a_stretched_box_fills_the_padding_box_less_its_margins() {
    let stretch = "width:stretch;height:stretch";
    assert_eq!(laid_out(GRID, stretch), (3.0, 3.0, 210.0, 190.0));
    // Alignment leaves no free space to distribute.
    assert_eq!(
        laid_out(
            &format!("{GRID};justify-items:center;align-items:end"),
            stretch
        ),
        (3.0, 3.0, 210.0, 190.0)
    );
    assert_eq!(
        laid_out(GRID, &format!("{stretch};margin:2px 4px 6px 6px")),
        (9.0, 5.0, 200.0, 182.0)
    );
    assert_eq!(
        laid_out(&format!("{GRID};writing-mode:vertical-rl"), stretch),
        (3.0, 3.0, 210.0, 190.0)
    );
    // A line that is not `auto` starts the area at its track and still ends
    // it at the padding edge.
    assert_eq!(
        laid_out(GRID, &format!("{stretch};grid-column:1;grid-row:1")),
        (43.0, 13.0, 170.0, 180.0)
    );
    assert_eq!(
        laid_out(GRID, &format!("{stretch};grid-column:2 / auto")),
        (93.0, 3.0, 120.0, 190.0)
    );
}
