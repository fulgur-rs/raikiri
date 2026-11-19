//! A cell, or a flex or grid item, takes its baseline from its first line in
//! flow, even when that line sits in a block nested inside a block that holds
//! lines of its own after it. The neighbouring cell or item aligned by
//! baseline then shares that line. The values are Chromium's, measured in
//! Ahem at 10px with a line height of 20px, so a glyph sits 5px below the top
//! of its line.

use super::anonymous_table_tests::{div, laid_out};
use crate::document::Document;
use crate::layout::test_support::absolute_rect;

const TEXT: &str = "font-family:Ahem;font-size:10px;line-height:20px";

/// A table row whose cells `build` fills, followed by an explicit cell
/// holding "C" in a span. Returns the top of that span.
fn baseline_cell_text_top(build: impl FnOnce(&mut Document, usize)) -> f32 {
    let mut span = 0;
    let doc = laid_out(TEXT, |doc, body| {
        let table = div(doc, body, "display:table");
        let row = div(doc, table, "display:table-row");
        build(doc, row);
        let cell = div(doc, row, "display:table-cell");
        span = doc.append_element(Some(cell), "span", Some(""));
        doc.append_text(span, "C");
    });
    absolute_rect(&doc, span).1
}

/// `<div><div style=first_css>A</div>U U</div>` inside `parent`.
fn block_with_nested_first_line(doc: &mut Document, parent: usize, first_css: &str) {
    let outer = div(doc, parent, "display:block");
    let first = div(doc, outer, &format!("display:block;{first_css}"));
    doc.append_text(first, "A");
    doc.append_text(outer, "U U");
}

#[test]
fn an_anonymous_cell_takes_the_line_of_a_nested_first_block() {
    let top = baseline_cell_text_top(|doc, row| {
        block_with_nested_first_line(doc, row, "");
        let d = div(doc, row, "display:block");
        doc.append_text(d, "D");
    });
    assert_eq!(top, 5.0);
}

#[test]
fn an_explicit_cell_takes_the_line_of_a_nested_first_block() {
    let top = baseline_cell_text_top(|doc, row| {
        let cell = div(doc, row, "display:table-cell");
        block_with_nested_first_line(doc, cell, "");
    });
    assert_eq!(top, 5.0);
}

#[test]
fn the_first_line_is_found_through_several_levels_of_blocks() {
    let top = baseline_cell_text_top(|doc, row| {
        let cell = div(doc, row, "display:table-cell");
        let outer = div(doc, cell, "display:block");
        block_with_nested_first_line(doc, outer, "padding-top:7px");
        doc.append_text(outer, "V");
    });
    assert_eq!(top, 12.0);
}

#[test]
fn a_nested_first_block_without_lines_leaves_the_baseline_to_the_next_line() {
    let top = baseline_cell_text_top(|doc, row| {
        let cell = div(doc, row, "display:table-cell");
        let outer = div(doc, cell, "display:block");
        div(doc, outer, "display:block;height:13px");
        doc.append_text(outer, "U U");
    });
    assert_eq!(top, 18.0);
}

#[test]
fn a_cell_with_lines_of_its_own_takes_the_line_of_a_block_before_them() {
    let top = baseline_cell_text_top(|doc, row| {
        let cell = div(doc, row, "display:table-cell");
        let padded = div(doc, cell, "display:block;padding-top:3px");
        let a = div(doc, padded, "display:block");
        doc.append_text(a, "A");
        doc.append_text(cell, "U U");
    });
    assert_eq!(top, 8.0);
}

/// A container `display` whose first item is a block with a nested first
/// line under 7px of padding, and whose second item holds "C" in a span.
/// Returns the top of that span.
fn baseline_item_text_top(display: &str) -> f32 {
    let mut span = 0;
    let doc = laid_out(TEXT, |doc, body| {
        let container = div(doc, body, display);
        let item = div(doc, container, "display:block");
        block_with_nested_first_line(doc, item, "padding-top:7px");
        let other = div(doc, container, "display:block");
        span = doc.append_element(Some(other), "span", Some(""));
        doc.append_text(span, "C");
    });
    absolute_rect(&doc, span).1
}

#[test]
fn a_flex_item_aligns_by_the_line_of_a_nested_first_block() {
    assert_eq!(
        baseline_item_text_top("display:flex;align-items:baseline"),
        12.0
    );
}

#[test]
fn a_grid_item_aligns_by_the_line_of_a_nested_first_block() {
    let display = "display:grid;grid-template-columns:50px 50px;align-items:baseline";
    assert_eq!(baseline_item_text_top(display), 12.0);
}

#[test]
fn a_flex_item_with_a_float_in_columns_aligns_by_its_first_block() {
    let mut span = 0;
    let doc = laid_out(TEXT, |doc, body| {
        let columns = div(
            doc,
            body,
            "columns:2;column-gap:0;height:100px;column-fill:auto",
        );
        let flex = div(doc, columns, "display:flex;align-items:baseline");
        let item = div(doc, flex, "display:block;width:40px");
        div(doc, item, "float:right;width:5px;height:5px");
        let first = div(doc, item, "display:block;padding-top:7px");
        doc.append_text(first, "A");
        doc.append_text(item, "U U");
        let other = div(doc, flex, "display:block");
        span = doc.append_element(Some(other), "span", Some(""));
        doc.append_text(span, "C");
    });
    assert_eq!(absolute_rect(&doc, span).1, 12.0);
}
