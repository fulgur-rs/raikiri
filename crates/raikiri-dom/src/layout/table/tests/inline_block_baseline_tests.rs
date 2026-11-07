//! The baseline an inline-block sits on when it holds a table. A table gives
//! the flex container, grid or table cell around it the baseline of its first
//! row, but a block container and the inline-block holding it get no baseline
//! from it (a box with none sits on its bottom margin edge). The values are
//! Chromium's, measured in Ahem at 10px with a line height of 10px.

use super::anonymous_table_tests::{div, laid_out};
use crate::document::Document;
use crate::layout::test_support::absolute_rect;

/// A box as `(x, y, width, height)`.
type Rect = (f32, f32, f32, f32);

const TEXT: &str = "font-family:Ahem;font-size:10px;line-height:10px";

/// A table of one row and one cell holding "aa", the cell `cell_css` more.
fn table(doc: &mut Document, parent: usize, cell_css: &str) -> usize {
    let table = div(doc, parent, "display:table");
    let row = div(doc, table, "display:table-row");
    let cell = div(doc, row, &format!("display:table-cell;{cell_css}"));
    doc.append_text(cell, "aa");
    table
}

/// "yy" and an inline-block that `build` fills, in a paragraph. Returns the
/// top of the "yy" box, which shows where the baseline of the line is, the
/// inline-block and the paragraph.
fn beside_text(build: impl FnOnce(&mut Document, usize)) -> (f32, Rect, Rect) {
    let (mut text, mut inline_block, mut paragraph) = (0, 0, 0);
    let doc = laid_out(TEXT, |doc, body| {
        paragraph = div(doc, body, "display:block");
        text = doc.append_element(Some(paragraph), "span", Some(""));
        doc.append_text(text, "yy");
        inline_block = doc.append_element(Some(paragraph), "span", Some("display:inline-block"));
        build(doc, inline_block);
    });
    (
        absolute_rect(&doc, text).1,
        absolute_rect(&doc, inline_block),
        absolute_rect(&doc, paragraph),
    )
}

#[test]
fn an_inline_block_holding_only_a_table_sits_on_its_bottom_edge() {
    // The line is 2px taller than the 10px box: the descent of the strut
    // hangs below the box, where the table's own baseline would put it on
    // the first row's text.
    let (text_top, inline_block, paragraph) = beside_text(|doc, ib| {
        table(doc, ib, "");
    });
    assert_eq!(text_top, 2.0);
    assert_eq!(inline_block, (20.0, 0.0, 20.0, 10.0));
    assert_eq!(paragraph.3, 12.0);
}

#[test]
fn a_table_inside_a_block_inside_an_inline_block_gives_no_baseline_either() {
    let (text_top, _, paragraph) = beside_text(|doc, ib| {
        let block = div(doc, ib, "display:block");
        table(doc, block, "padding-top:5px");
    });
    assert_eq!(text_top, 7.0);
    assert_eq!(paragraph.3, 17.0);
}

#[test]
fn the_text_of_a_block_next_to_a_table_sets_the_baseline() {
    // The lines in the table's cell do not count, whichever comes last.
    let (text_top, inline_block, _) = beside_text(|doc, ib| {
        let block = div(doc, ib, "display:block");
        doc.append_text(block, "xx");
        table(doc, ib, "padding-top:5px");
    });
    assert_eq!(text_top, 0.0);
    assert_eq!(inline_block.3, 25.0);
    let (text_top, inline_block, _) = beside_text(|doc, ib| {
        table(doc, ib, "padding-top:5px");
        let block = div(doc, ib, "display:block");
        doc.append_text(block, "xx");
    });
    assert_eq!(text_top, 15.0);
    assert_eq!(inline_block.3, 25.0);
}

#[test]
fn a_table_with_a_caption_and_a_loose_row_give_no_baseline() {
    let (text_top, _, paragraph) = beside_text(|doc, ib| {
        let table = div(doc, ib, "display:table");
        let caption = div(doc, table, "display:table-caption");
        doc.append_text(caption, "cc");
        let row = div(doc, table, "display:table-row");
        let cell = div(doc, row, "display:table-cell;padding-top:5px");
        doc.append_text(cell, "aa");
    });
    assert_eq!(text_top, 17.0);
    assert_eq!(paragraph.3, 27.0);
    // A row with no table around it stands in an anonymous one.
    let (text_top, _, paragraph) = beside_text(|doc, ib| {
        let row = div(doc, ib, "display:table-row");
        let cell = div(doc, row, "display:table-cell;padding-top:5px");
        doc.append_text(cell, "aa");
    });
    assert_eq!(text_top, 7.0);
    assert_eq!(paragraph.3, 17.0);
}

#[test]
fn a_flex_container_or_grid_passes_on_the_baseline_of_a_table_item() {
    for display in ["flex", "grid"] {
        let (text_top, _, paragraph) = beside_text(|doc, ib| {
            let container = div(doc, ib, &format!("display:{display}"));
            table(doc, container, "padding-top:5px");
        });
        assert_eq!(text_top, 5.0, "{display}");
        assert_eq!(paragraph.3, 15.0, "{display}");
    }
    // Text in a flex item still sets the baseline.
    let (text_top, _, paragraph) = beside_text(|doc, ib| {
        let container = div(doc, ib, "display:flex");
        let item = div(doc, container, "display:block;padding-top:5px");
        doc.append_text(item, "xx");
    });
    assert_eq!(text_top, 5.0);
    assert_eq!(paragraph.3, 15.0);
}

#[test]
fn a_table_in_a_block_still_gives_its_baseline_to_a_flex_item_or_cell() {
    // "xx" with 5px above it shares its baseline with the first row of the
    // table in the second box, 8px below its top.
    let mut ids = [0; 2];
    let doc = laid_out(TEXT, |doc, body| {
        let flex = div(doc, body, "display:flex;align-items:baseline");
        let text = div(doc, flex, "display:block;padding-top:5px");
        doc.append_text(text, "xx");
        let item = div(doc, flex, "display:block");
        let block = div(doc, item, "display:block");
        table(doc, block, "");
        ids[0] = item;
        let grid = div(doc, body, "display:table");
        let row = div(doc, grid, "display:table-row");
        let text = div(
            doc,
            row,
            "display:table-cell;vertical-align:baseline;padding-top:5px",
        );
        doc.append_text(text, "xx");
        let cell = div(
            doc,
            row,
            "display:table-cell;vertical-align:baseline;height:30px",
        );
        let block = div(doc, cell, "display:block");
        table(doc, block, "");
        ids[1] = block;
    });
    assert_eq!(absolute_rect(&doc, ids[0]).1, 5.0);
    assert_eq!(
        absolute_rect(&doc, ids[1]).1 - absolute_rect(&doc, ids[0]).1,
        15.0
    );
}
