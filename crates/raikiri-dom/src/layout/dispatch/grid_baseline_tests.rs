//! The first baseline a grid container exports to the flex or grid container
//! that aligns it by a baseline: the baseline of the first line of an item in
//! its first row, not the bottom edge of that item.
//!
//! The expected tops are those Chromium gives the same markup.

use crate::layout::ifc::test_support::block_fixture;
use crate::layout::layout_single_page;
use crate::layout::test_support::{absolute_rect, page_box_800x600, with_ahem};

/// The offsets `(x, y)` of the first and of the last item of the container
/// `container_css`, relative to the container, where the middle item is the
/// grid `grid_css` holding `grid_children`, a list of `(style, text)` pairs.
/// The two outer items are paragraphs of Ahem at 10px with a line height of
/// 10px and of 30px, whose baselines lie 8px and 18px below their tops in a
/// horizontal writing mode.
fn outer_offsets(
    container_css: &str,
    grid_css: &str,
    grid_children: &[(&str, &str)],
) -> [(f32, f32); 2] {
    let mut ends = (0, 0);
    let fixture = block_fixture(container_css, |doc, root| {
        let first = doc.append_element(
            Some(root),
            "div",
            Some("display:block;line-height:10px;margin:0"),
        );
        doc.append_text(first, "X");
        let grid = doc.append_element(Some(root), "div", Some(grid_css));
        for &(style, text) in grid_children {
            let item = doc.append_element(Some(grid), "div", Some(style));
            doc.append_text(item, text);
        }
        let last = doc.append_element(
            Some(root),
            "div",
            Some("display:block;line-height:30px;margin:0"),
        );
        doc.append_text(last, "X");
        ends = (first, last);
    });
    let root = fixture.root;
    let mut doc = fixture.doc;
    layout_single_page(with_ahem(&mut doc), &fixture.cascade, page_box_800x600()).expect("layout");
    let (left, top, _, _) = absolute_rect(&doc, root);
    [ends.0, ends.1].map(|end| {
        let (x, y, _, _) = absolute_rect(&doc, end);
        (x - left, y - top)
    })
}

/// The top of the first and of the last item, as [`outer_offsets`] places
/// them.
fn outer_tops(container_css: &str, grid_css: &str, grid_children: &[(&str, &str)]) -> (f32, f32) {
    let [first, last] = outer_offsets(container_css, grid_css, grid_children);
    (first.1, last.1)
}

const ROW: &str = "display:flex;align-items:baseline;width:300px;height:100px";
const GRID_ROW: &str = "display:grid;grid-auto-flow:column;align-items:baseline;width:300px";
const GRID: &str = "display:grid";
const COLUMNS: &str = "display:grid;grid-template-columns:20px 20px";
/// A block whose single 10px line sits in a 40px line box, so that its
/// baseline is 23px below its top.
const TALL: (&str, &str) = ("display:block;line-height:40px", "X");

/// The tops of the two outer items when the grid holds `children`, in a flex
/// row and in a grid row, which must agree.
fn tops(grid_css: &str, children: &[(&str, &str)]) -> (f32, f32) {
    let flex = outer_tops(ROW, grid_css, children);
    let grid = outer_tops(GRID_ROW, grid_css, children);
    assert_eq!(flex, grid, "{grid_css} | {children:?}");
    flex
}

#[test]
fn a_grid_item_that_stretches_gives_the_baseline_of_its_first_line() {
    // The grid's only item has a 40px line, whose baseline is 23px down. The
    // 10px paragraph before it sits 15px lower to share that baseline, and
    // the 30px one 5px lower.
    assert_eq!(tops(GRID, &[TALL]), (15.0, 5.0));
}

#[test]
fn an_item_that_aligns_by_baseline_alone_gives_its_baseline() {
    // Taffy shares baselines among two or more items of a row only.
    assert_eq!(
        tops("display:grid;align-items:baseline", &[TALL]),
        (15.0, 5.0)
    );
}

#[test]
fn an_item_that_aligns_by_baseline_with_another_gives_the_shared_baseline() {
    let items = [TALL, ("display:block;line-height:20px", "X")];
    assert_eq!(
        tops(
            "display:grid;grid-auto-flow:column;align-items:baseline",
            &items
        ),
        (15.0, 5.0)
    );
    assert_eq!(
        tops("display:grid;grid-auto-flow:column", &items),
        (15.0, 5.0)
    );
}

#[test]
fn the_first_item_that_aligns_by_baseline_gives_the_baseline() {
    // The 20px line of the second item has its baseline 13px down; the
    // stretched first item is passed over for it.
    let items = [
        TALL,
        ("display:block;line-height:20px;align-self:baseline", "X"),
    ];
    assert_eq!(
        tops("display:grid;grid-auto-flow:column", &items),
        (10.0, 0.0)
    );
}

#[test]
fn an_item_in_a_later_row_does_not_give_the_baseline() {
    // The 50px line of the item in the second row would put the baseline 33px
    // down.
    let items = [
        ("display:block;line-height:50px;grid-row:2", "X"),
        ("display:block;line-height:20px;grid-row:1", "X"),
    ];
    assert_eq!(tops(GRID, &items), (10.0, 0.0));
}

#[test]
fn the_first_item_of_the_first_row_by_column_gives_the_baseline() {
    // The item in the first column is later in the source and has the shorter
    // line, whose baseline is 13px down.
    let items = [
        (
            "display:block;line-height:40px;grid-row:1;grid-column:2",
            "X",
        ),
        (
            "display:block;line-height:20px;grid-row:1;grid-column:1",
            "X",
        ),
    ];
    assert_eq!(tops(COLUMNS, &items), (10.0, 0.0));
    // The order of the items does not move one to the first column.
    let items = [
        (
            "display:block;line-height:20px;grid-row:1;grid-column:1;order:1",
            "X",
        ),
        (
            "display:block;line-height:40px;grid-row:1;grid-column:2",
            "X",
        ),
    ];
    assert_eq!(tops(COLUMNS, &items), (10.0, 0.0));
}

#[test]
fn the_edges_of_the_grid_and_the_margin_of_the_item_count() {
    // The item's baseline lies 5px lower under a 5px margin, and 7px lower
    // inside the padding and border of the grid.
    let margin = [("display:block;line-height:40px;margin-top:5px", "X")];
    assert_eq!(tops(GRID, &margin), (20.0, 10.0));
    let aligned = [("display:block;line-height:40px;margin-top:5px", "X")];
    assert_eq!(
        tops("display:grid;align-items:baseline", &aligned),
        (20.0, 10.0)
    );
    assert_eq!(
        tops("display:grid;padding:4px;border:3px solid", &[TALL]),
        (22.0, 12.0)
    );
}

#[test]
fn an_item_without_a_line_gives_the_bottom_of_its_border_box() {
    let empty = [("display:block;height:40px", "")];
    assert_eq!(tops(GRID, &empty), (32.0, 22.0));
}

#[test]
fn a_relative_offset_does_not_move_the_baseline() {
    for position in ["relative;top:7px", "relative;bottom:-7px", "sticky;top:7px"] {
        let shifted = [(
            &format!("display:block;line-height:40px;position:{position}")[..],
            "X",
        )];
        assert_eq!(tops(GRID, &shifted), (15.0, 5.0), "{position}");
    }
}

#[test]
fn an_item_aligned_to_the_end_of_its_area_brings_the_baseline_down() {
    // The 40px item sits at the bottom of the 60px grid, 20px down.
    let ended = [("display:block;line-height:40px;align-self:end", "X")];
    assert_eq!(tops("display:grid;height:60px", &ended), (35.0, 25.0));
}

#[test]
fn a_scroll_container_item_gives_a_baseline_within_its_border_box() {
    // The line lies 23px down in a box 30px tall, and in one 20px tall the
    // baseline stays at its bottom edge.
    let tall = [(
        "display:block;overflow:hidden;height:30px;line-height:40px",
        "X",
    )];
    assert_eq!(tops(GRID, &tall), (15.0, 5.0));
    let short = [(
        "display:block;overflow:hidden;height:20px;line-height:40px",
        "X",
    )];
    assert_eq!(tops(GRID, &short), (12.0, 2.0));
}

#[test]
fn layout_containment_leaves_no_baseline_to_the_item_or_the_grid() {
    // Both stand in the bottom edge of the 40px item.
    let contained = [("display:block;contain:layout;line-height:40px", "X")];
    assert_eq!(tops(GRID, &contained), (32.0, 22.0));
    assert_eq!(tops("display:grid;contain:layout", &[TALL]), (32.0, 22.0));
}

#[test]
fn an_item_with_an_auto_margin_still_gives_the_baseline_when_it_is_the_first() {
    // The auto margin keeps it out of baseline alignment, not out of the
    // choice of the first item.
    let item = [("display:block;line-height:40px;margin-top:auto", "X")];
    assert_eq!(
        tops("display:grid;align-items:baseline", &item),
        (15.0, 5.0)
    );
}

#[test]
fn an_item_that_aligns_by_its_last_baseline_still_gives_the_first_baseline() {
    assert_eq!(
        tops("display:grid;align-items:last baseline", &[TALL]),
        (15.0, 5.0)
    );
}

const VERTICAL_RL: &str =
    "display:flex;align-items:baseline;writing-mode:vertical-rl;width:300px;height:300px";
const VERTICAL_LR: &str =
    "display:flex;align-items:baseline;writing-mode:vertical-lr;width:300px;height:300px";

/// The left edges of the two outer items of a vertical container.
fn lefts(container_css: &str, grid_css: &str, children: &[(&str, &str)]) -> (f32, f32) {
    let [first, last] = outer_offsets(container_css, grid_css, children);
    (first.0, last.0)
}

#[test]
fn a_vertical_grid_gives_the_baseline_of_the_first_line_of_its_item() {
    // The central baseline of the 40px line is 20px from the block-start edge
    // and that of the 10px one 5px, so the 10px item sits 15px in. The 30px
    // item sits 5px in.
    assert_eq!(lefts(VERTICAL_RL, GRID, &[TALL]), (275.0, 265.0));
    assert_eq!(lefts(VERTICAL_LR, GRID, &[TALL]), (15.0, 5.0));
}

#[test]
fn a_vertical_grid_takes_the_first_item_that_aligns_by_baseline() {
    let items = [
        TALL,
        ("display:block;line-height:20px;align-self:baseline", "X"),
    ];
    let grid = "display:grid;grid-auto-flow:column";
    assert_eq!(lefts(VERTICAL_RL, grid, &items), (280.0, 270.0));
    assert_eq!(lefts(VERTICAL_LR, grid, &items), (10.0, 0.0));
}

#[test]
fn a_relative_offset_does_not_move_the_baseline_of_a_vertical_grid() {
    for offset in ["left:7px;top:3px", "right:7px;bottom:3px", "left:-7px"] {
        let shifted = [(
            &format!("display:block;line-height:40px;position:relative;{offset}")[..],
            "X",
        )];
        assert_eq!(
            lefts(VERTICAL_RL, GRID, &shifted),
            (275.0, 265.0),
            "{offset}"
        );
        assert_eq!(lefts(VERTICAL_LR, GRID, &shifted), (15.0, 5.0), "{offset}");
    }
}

#[test]
fn a_grid_nested_in_a_grid_passes_the_baseline_up() {
    // The inner grid takes the baseline of its anonymous item, which the
    // outer one takes in turn.
    let nested = [("display:grid;line-height:40px", "X")];
    assert_eq!(tops(GRID, &nested), (15.0, 5.0));
}

#[test]
fn a_vertical_item_with_layout_containment_has_its_baseline_synthesized_from_its_box() {
    // The central baseline of a box 30px wide is 15px from its block-start
    // edge, though the 20px line inside it has its own 10px from there.
    let contained = [(
        "display:block;contain:layout;width:30px;line-height:20px",
        "X",
    )];
    assert_eq!(lefts(VERTICAL_RL, GRID, &contained), (280.0, 270.0));
    assert_eq!(lefts(VERTICAL_LR, GRID, &contained), (10.0, 0.0));
}

#[test]
fn a_vertical_scroll_container_item_gives_a_baseline_within_its_border_box() {
    // The line's central baseline is 20px from the block-start edge of a box
    // 10px wide.
    let scrolling = [(
        "display:block;overflow:hidden;width:10px;line-height:40px",
        "X",
    )];
    assert_eq!(lefts(VERTICAL_RL, GRID, &scrolling), (280.0, 270.0));
    assert_eq!(lefts(VERTICAL_LR, GRID, &scrolling), (10.0, 0.0));
}
