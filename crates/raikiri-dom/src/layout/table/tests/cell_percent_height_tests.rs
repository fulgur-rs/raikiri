//! Boxes with a percentage `height` in a table cell that states its height.
//!
//! A cell's row is measured with the cell's height left out, so a child that
//! asks for a percentage of it has nothing to resolve against. A child that
//! scrolls in the block direction (`overflow-y: auto | scroll`) then asks the
//! row for no more than its margins, padding, border and `min-height` length,
//! where any other child asks for its content. Once the row is known, a cell
//! that states a height gives it to those children as the basis of their
//! percentages, even when the cell's own content set it. Every rectangle here
//! is what Chromium reports for the same markup (Ahem 10px, no body margin).

use crate::document::Document;
use crate::layout::test_support::{absolute_rect, page_box_800x600, with_ahem};
use raikiri_style::{build_rule_tree, cascade};

/// A rectangle as (left, top, width, height).
type Rect = (f32, f32, f32, f32);

/// A laid-out `table > row > cells`, where `children` are the boxes under the
/// first cell.
struct Laid {
    doc: Document,
    table: usize,
    cells: Vec<usize>,
    children: Vec<usize>,
}

impl Laid {
    fn table(&self) -> Rect {
        absolute_rect(&self.doc, self.table)
    }

    fn cell(&self, index: usize) -> Rect {
        absolute_rect(&self.doc, self.cells[index])
    }

    fn child(&self, index: usize) -> Rect {
        absolute_rect(&self.doc, self.children[index])
    }
}

/// One cell, 100px wide, with `cell_css` and one child, `child_css`, that holds
/// a 200px tall block.
fn lay_out(cell_css: &str, child_css: &str) -> Laid {
    lay_out_cells(cell_css, &[child_css], "")
}

/// A cell with `cell_css` and the children `children_css`, each of which holds
/// a 200px tall block (but for a replaced `img|` one), and then `neighbour`, which is the content of another
/// cell, 100px wide, when it is not empty.
fn lay_out_cells(cell_css: &str, children_css: &[&str], neighbour: &str) -> Laid {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;font-family:Ahem;font-size:10px;line-height:10px"),
    );
    let table = doc.append_element(Some(body), "div", Some("display:table"));
    let row = doc.append_element(Some(table), "div", Some("display:table-row"));
    let cell = doc.append_element(
        Some(row),
        "div",
        Some(&format!("display:table-cell;width:100px;{cell_css}")),
    );
    let mut cells = vec![cell];
    let mut children = Vec::new();
    for css in children_css {
        // `img|` makes a replaced box, which holds nothing.
        let (tag, css) = css
            .strip_prefix("img|")
            .map_or(("div", *css), |css| ("img", css));
        let child = doc.append_element(Some(cell), tag, Some(&format!("display:block;{css}")));
        if tag == "div" {
            doc.append_element(Some(child), "div", Some("display:block;height:200px"));
        }
        children.push(child);
    }
    if !neighbour.is_empty() {
        let other = doc.append_element(
            Some(row),
            "div",
            Some("display:table-cell;width:100px;vertical-align:top"),
        );
        doc.append_element(
            Some(other),
            "div",
            Some(&format!("display:block;{neighbour}")),
        );
        cells.push(other);
    }
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    with_ahem(&mut doc);
    crate::layout::layout_single_page(&mut doc, &cascade, page_box_800x600()).expect("layout");
    Laid {
        doc,
        table,
        cells,
        children,
    }
}

#[test]
fn a_scrollable_child_sized_to_its_cell_leaves_the_cell_at_its_height() {
    for overflow in ["auto", "scroll"] {
        let laid = lay_out("height:100px", &format!("overflow:{overflow};height:100%"));
        assert_eq!(laid.cell(0), (0.0, 0.0, 100.0, 100.0), "{overflow}");
        assert_eq!(laid.child(0), (0.0, 0.0, 100.0, 100.0), "{overflow}");
        assert_eq!(laid.table(), (0.0, 0.0, 100.0, 100.0), "{overflow}");
    }
    // A smaller share and a calc() resolve against the stated height.
    let laid = lay_out("height:100px", "overflow:auto;height:50%");
    assert_eq!(laid.cell(0), (0.0, 0.0, 100.0, 100.0));
    assert_eq!(laid.child(0), (0.0, 0.0, 100.0, 50.0));
    let laid = lay_out("height:100px", "overflow:auto;height:calc(100% - 30px)");
    assert_eq!(laid.cell(0), (0.0, 0.0, 100.0, 100.0));
    assert_eq!(laid.child(0), (0.0, 0.0, 100.0, 70.0));
    // A cell taller than it states fits the share of the height it states.
    let laid = lay_out("height:150px", "overflow:auto;height:100%");
    assert_eq!(laid.cell(0), (0.0, 0.0, 100.0, 150.0));
    assert_eq!(laid.child(0), (0.0, 0.0, 100.0, 150.0));
}

#[test]
fn only_a_percentage_height_that_scrolls_in_the_block_direction_is_left_out() {
    // `overflow-y` decides, not `overflow-x`.
    let laid = lay_out(
        "height:100px",
        "overflow-x:auto;overflow-y:visible;height:100%",
    );
    assert_eq!(laid.cell(0), (0.0, 0.0, 100.0, 100.0));
    for css in [
        "overflow-y:hidden;overflow-x:auto;height:100%",
        "overflow-y:clip;overflow-x:visible;height:100%",
        "overflow:hidden;height:100%",
        "overflow:auto;height:auto",
        "overflow:auto;min-height:100%",
    ] {
        let laid = lay_out("height:100px", css);
        assert_eq!(laid.cell(0).3, 200.0, "{css}");
    }
    // A cell that does not state its height measures the child as it is.
    let laid = lay_out("", "overflow:auto;height:100%");
    assert_eq!(laid.cell(0), (0.0, 0.0, 100.0, 200.0));
    assert_eq!(laid.child(0), (0.0, 0.0, 100.0, 200.0));
}

#[test]
fn a_left_out_child_still_asks_for_its_margin_padding_and_min_height_length() {
    let laid = lay_out("height:100px", "overflow:auto;height:100%;min-height:150px");
    assert_eq!(laid.cell(0), (0.0, 0.0, 100.0, 150.0));
    assert_eq!(laid.child(0), (0.0, 0.0, 100.0, 150.0));
    // A percentage `min-height` has nothing to resolve against meanwhile.
    let laid = lay_out("height:100px", "overflow:auto;height:100%;min-height:150%");
    assert_eq!(laid.cell(0), (0.0, 0.0, 100.0, 100.0));
    let laid = lay_out("height:100px", "overflow:auto;height:100%;margin-top:150px");
    assert_eq!(laid.cell(0), (0.0, 0.0, 100.0, 150.0));
    assert_eq!(laid.child(0), (0.0, 150.0, 100.0, 150.0));
    let laid = lay_out(
        "height:100px",
        "overflow:auto;height:100%;padding:0 0 80px 0",
    );
    assert_eq!(laid.cell(0), (0.0, 0.0, 100.0, 100.0));
    let laid = lay_out("height:100px", "overflow:auto;height:100%;padding:80px 0");
    assert_eq!(laid.cell(0), (0.0, 0.0, 100.0, 160.0));
}

#[test]
fn a_left_out_child_takes_the_row_height_another_cell_set() {
    let laid = lay_out_cells("height:50px", &["overflow:auto;height:100%"], "height:80px");
    assert_eq!(laid.cell(0), (0.0, 0.0, 100.0, 80.0));
    assert_eq!(laid.cell(1), (100.0, 0.0, 100.0, 80.0));
    assert_eq!(laid.child(0), (0.0, 0.0, 100.0, 80.0));
    // Content in the cell that sets the row height does too.
    let laid = lay_out_cells(
        "height:100px",
        &["height:150px", "overflow:auto;height:100%"],
        "",
    );
    assert_eq!(laid.cell(0), (0.0, 0.0, 100.0, 150.0));
    assert_eq!(laid.child(1), (0.0, 150.0, 100.0, 150.0));
}

#[test]
fn a_child_that_does_not_scroll_fills_the_row_its_content_set() {
    for (css, expected) in [
        ("overflow:hidden;height:100%", 200.0),
        ("overflow:visible;height:100%", 200.0),
        ("overflow:visible;height:50%", 100.0),
        ("overflow:hidden;height:calc(100% - 10px)", 190.0),
        ("overflow:hidden;height:200px;max-height:100%", 200.0),
    ] {
        let laid = lay_out("height:100px", css);
        assert_eq!(laid.cell(0), (0.0, 0.0, 100.0, 200.0), "{css}");
        assert_eq!(laid.child(0).3, expected, "{css}");
    }
    // So does a float, which the row measure never left out.
    let laid = lay_out(
        "height:100px",
        "float:left;width:50px;overflow:auto;height:100%",
    );
    assert_eq!(laid.cell(0), (0.0, 0.0, 100.0, 200.0));
    assert_eq!(laid.child(0), (0.0, 0.0, 50.0, 200.0));
}

#[test]
fn a_cell_without_a_stated_height_leaves_a_percentage_height_auto() {
    let laid = lay_out_cells("", &["height:50%;width:20px"], "height:200px");
    assert_eq!(laid.cell(0), (0.0, 0.0, 100.0, 200.0));
    // The child holds a 200px block, so its auto height is that block's.
    assert_eq!(laid.child(0), (0.0, 0.0, 20.0, 200.0));
}

#[test]
fn a_replaced_child_keeps_taking_its_percentage_of_the_stated_height() {
    // A block of 150px, an image of half the stated 100px and one of that less
    // 30px, in a cell that they make 270px tall.
    let laid = lay_out_cells(
        "height:100px",
        &[
            "height:150px",
            "img|width:50px;height:50%",
            "img|width:50px;height:calc(100% - 30px)",
        ],
        "",
    );
    assert_eq!(laid.cell(0), (0.0, 0.0, 100.0, 270.0));
    assert_eq!(laid.child(1), (0.0, 150.0, 50.0, 50.0));
    assert_eq!(laid.child(2), (0.0, 200.0, 50.0, 70.0));
}

#[test]
fn a_replaced_child_that_scrolls_is_not_left_out_of_the_row() {
    // `overflow` does not make an image a scrolling box: it still asks the row
    // for its half of the stated 100px, after a 150px block.
    for overflow in ["auto", "scroll"] {
        let laid = lay_out_cells(
            "height:100px",
            &[
                "height:150px",
                &format!("img|width:50px;overflow:{overflow};height:50%"),
            ],
            "",
        );
        assert_eq!(laid.cell(0), (0.0, 0.0, 100.0, 200.0), "{overflow}");
        assert_eq!(laid.child(1), (0.0, 150.0, 50.0, 50.0), "{overflow}");
    }
}
