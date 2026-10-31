//! The grid box of an absolutely positioned table with a caption.
//!
//! Taffy sizes an out-of-flow table by measuring its wrapper box, caption
//! included, and then lays the table out at that size. The grid box takes what
//! the caption leaves of it, so it is as tall as its rows and spacing and not
//! as tall as the whole wrapper. Every rectangle here is what Chromium reports
//! for the same markup (Ahem 10px, no body margin).

use crate::document::Document;
use crate::layout::test_support::{absolute_rect, page_box_800x600, with_ahem};
use raikiri_style::{build_rule_tree, cascade};

/// A rectangle as (left, top, width, height).
type Rect = (f32, f32, f32, f32);

struct Laid {
    doc: Document,
    table: usize,
    caption: usize,
    cells: Vec<usize>,
}

impl Laid {
    fn table(&self) -> Rect {
        absolute_rect(&self.doc, self.table)
    }

    fn caption(&self) -> Rect {
        absolute_rect(&self.doc, self.caption)
    }

    fn cells(&self) -> Vec<Rect> {
        self.cells
            .iter()
            .map(|&cell| absolute_rect(&self.doc, cell))
            .collect()
    }

    /// The box that paints the table's background and borders, in page
    /// coordinates.
    fn grid_box(&self) -> Rect {
        let grid = self.doc.nodes[self.table]
            .table_grid_box
            .expect("a laid out table has a grid box");
        let (x, y, _, _) = self.table();
        (x + grid.x, y + grid.y, grid.width, grid.height)
    }
}

/// `body > div(position:relative; 200x100) > table(position:absolute;
/// table_css) > [caption(caption_css), row > cell > 10x10 block, ...]` with
/// `rows` rows, laid out on an 800x600 page.
fn lay_out(table_css: &str, caption_css: &str, rows: usize) -> Laid {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block;margin:0"));
    let wrapper = doc.append_element(
        Some(body),
        "div",
        Some("display:block;position:relative;width:200px;height:100px"),
    );
    let table = doc.append_element(
        Some(wrapper),
        "div",
        Some(&format!("display:table;position:absolute;{table_css}")),
    );
    let caption = doc.append_element(
        Some(table),
        "div",
        Some(&format!(
            "display:table-caption;height:10px;width:30px;{caption_css}"
        )),
    );
    let mut cells = Vec::new();
    for _ in 0..rows {
        let row = doc.append_element(Some(table), "div", Some("display:table-row"));
        let cell = doc.append_element(Some(row), "div", Some("display:table-cell"));
        doc.append_element(
            Some(cell),
            "div",
            Some("display:block;width:10px;height:10px"),
        );
        cells.push(cell);
    }
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    with_ahem(&mut doc);
    crate::layout::layout_single_page(&mut doc, &cascade, page_box_800x600()).expect("layout");
    Laid {
        doc,
        table,
        caption,
        cells,
    }
}

#[test]
fn a_caption_above_the_rows_leaves_the_grid_as_tall_as_its_rows() {
    let laid = lay_out("top:0;left:0", "", 1);
    assert_eq!(laid.table(), (0.0, 0.0, 30.0, 20.0));
    assert_eq!(laid.caption(), (0.0, 0.0, 30.0, 10.0));
    assert_eq!(laid.grid_box(), (0.0, 10.0, 30.0, 10.0));
    assert_eq!(laid.cells(), [(0.0, 10.0, 30.0, 10.0)]);
}

#[test]
fn a_caption_below_the_rows_leaves_the_grid_as_tall_as_its_rows() {
    let laid = lay_out("top:0;left:0;caption-side:bottom", "", 1);
    assert_eq!(laid.table(), (0.0, 0.0, 30.0, 20.0));
    assert_eq!(laid.caption(), (0.0, 10.0, 30.0, 10.0));
    assert_eq!(laid.grid_box(), (0.0, 0.0, 30.0, 10.0));
    assert_eq!(laid.cells(), [(0.0, 0.0, 30.0, 10.0)]);
}

#[test]
fn padding_borders_and_spacing_stay_inside_the_grid() {
    let laid = lay_out("top:0;left:0;border:3px solid;padding:4px", "", 1);
    assert_eq!(laid.table(), (0.0, 0.0, 30.0, 34.0));
    assert_eq!(laid.grid_box(), (0.0, 10.0, 30.0, 24.0));
    assert_eq!(laid.cells(), [(7.0, 17.0, 16.0, 10.0)]);

    let laid = lay_out("top:0;left:0;border-spacing:5px", "", 1);
    assert_eq!(laid.table(), (0.0, 0.0, 30.0, 30.0));
    assert_eq!(laid.grid_box(), (0.0, 10.0, 30.0, 20.0));
    assert_eq!(laid.cells(), [(5.0, 15.0, 20.0, 10.0)]);
}

#[test]
fn several_rows_and_a_tall_caption_follow_the_same_rule() {
    let laid = lay_out("top:0;left:0", "", 2);
    assert_eq!(laid.table(), (0.0, 0.0, 30.0, 30.0));
    assert_eq!(laid.grid_box(), (0.0, 10.0, 30.0, 20.0));

    let laid = lay_out("top:0;left:0", "height:25px;width:50px", 1);
    assert_eq!(laid.table(), (0.0, 0.0, 50.0, 35.0));
    assert_eq!(laid.grid_box(), (0.0, 25.0, 50.0, 10.0));
}

#[test]
fn offsets_do_not_stretch_the_grid() {
    // `top` and `bottom` together do not make the table taller.
    let laid = lay_out("top:0;bottom:0;left:0", "", 1);
    assert_eq!(laid.table(), (0.0, 0.0, 30.0, 20.0));
    assert_eq!(laid.grid_box(), (0.0, 10.0, 30.0, 10.0));
    // `bottom` places the whole wrapper against the bottom edge.
    let laid = lay_out("bottom:0;left:0", "", 1);
    assert_eq!(laid.table(), (0.0, 80.0, 30.0, 20.0));
    assert_eq!(laid.grid_box(), (0.0, 90.0, 30.0, 10.0));
}

#[test]
fn an_authored_minimum_height_sizes_the_grid_alone() {
    // The grid takes the height and the caption sits on top of it.
    let laid = lay_out("top:0;left:0;min-height:40px", "", 1);
    assert_eq!(laid.table(), (0.0, 0.0, 30.0, 50.0));
    assert_eq!(laid.grid_box(), (0.0, 10.0, 30.0, 40.0));
    assert_eq!(laid.cells(), [(0.0, 10.0, 30.0, 40.0)]);
}

#[test]
fn a_table_without_rows_keeps_only_its_borders_in_the_grid() {
    let laid = lay_out("top:0;left:0;border:3px solid", "", 0);
    assert_eq!(laid.table(), (0.0, 0.0, 30.0, 16.0));
    assert_eq!(laid.caption(), (0.0, 0.0, 30.0, 10.0));
    assert_eq!(laid.grid_box(), (0.0, 10.0, 30.0, 6.0));

    let laid = lay_out("top:0;left:0;border:3px solid;caption-side:bottom", "", 0);
    assert_eq!(laid.table(), (0.0, 0.0, 30.0, 16.0));
    assert_eq!(laid.caption(), (0.0, 6.0, 30.0, 10.0));
    assert_eq!(laid.grid_box(), (0.0, 0.0, 30.0, 6.0));
}
