//! How the columns of an auto-layout table share their widths when cells span
//! columns and carry percentages.
//!
//! A spanning cell's percentage goes to the spanned columns that state none, in
//! proportion to their max-content widths; its minimum and maximum widths are
//! shared out over the spanned columns the way the table shares its own width
//! (percentage columns at their share, length columns at their widths, then the
//! auto columns); and what the max-content widths leave over goes to the auto
//! columns by those widths, then the length columns, then the percentage
//! columns by their shares, the percentages being taken in order up to 100%.
//! Every rectangle here is what Chromium reports for the same markup (Ahem
//! 10px, no body margin, no cell padding, no border spacing), give or take the
//! 1/64px its layout units round to.

use crate::document::Document;
use crate::layout::test_support::{absolute_rect, page_box_800x600, with_ahem};
use raikiri_style::{build_rule_tree, cascade};

/// A rectangle as (left, top, width, height).
type Rect = (f32, f32, f32, f32);

/// A table cell with Ahem `text` that spans `span` columns and states `width`
/// (empty for none).
struct Cell {
    text: &'static str,
    span: u16,
    width: &'static str,
}

fn cell(text: &'static str, width: &'static str) -> Cell {
    spanning(text, 1, width)
}

fn spanning(text: &'static str, span: u16, width: &'static str) -> Cell {
    Cell { text, span, width }
}

/// A laid-out table and its cells, row by row.
struct Laid {
    doc: Document,
    table: usize,
    cells: Vec<usize>,
}

impl Laid {
    /// Checks the table and the cells against what Chromium reports.
    fn assert_close(&self, table: Rect, cells: &[Rect]) {
        assert_rect(absolute_rect(&self.doc, self.table), table, "table");
        assert_eq!(self.cells.len(), cells.len(), "cell count");
        for (index, (&node, &expected)) in self.cells.iter().zip(cells).enumerate() {
            assert_rect(
                absolute_rect(&self.doc, node),
                expected,
                &format!("cell {index}"),
            );
        }
    }
}

fn assert_rect(actual: Rect, expected: Rect, what: &str) {
    let close = |a: f32, b: f32| (a - b).abs() <= 0.1;
    assert!(
        close(actual.0, expected.0)
            && close(actual.1, expected.1)
            && close(actual.2, expected.2)
            && close(actual.3, expected.3),
        "{what}: {actual:?}, expected {expected:?}"
    );
}

/// `table(table_css) > cols, rows > cells` under `body`, laid out on an 800x600
/// page. `cols` are the widths of the `<col>` elements (empty for none).
fn lay_out(table_css: &str, cols: &[&str], rows: &[&[Cell]]) -> Laid {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;font-family:Ahem;font-size:10px;line-height:10px"),
    );
    let table = doc.append_element(Some(body), "table", Some(table_css));
    for width in cols {
        let width = if width.is_empty() {
            String::new()
        } else {
            format!(";width:{width}")
        };
        doc.append_element(
            Some(table),
            "col",
            Some(&format!("display:table-column{width}")),
        );
    }
    let mut cells = Vec::new();
    for row in rows {
        let tr = doc.append_element(Some(table), "tr", Some("display:table-row"));
        for cell in *row {
            let width = if cell.width.is_empty() {
                String::new()
            } else {
                format!(";width:{}", cell.width)
            };
            let td = doc.append_element(
                Some(tr),
                "td",
                Some(&format!("display:table-cell;padding:0{width}")),
            );
            if cell.span > 1 {
                doc.set_element_attributes(
                    td,
                    vec![("colspan".into(), cell.span.to_string().into())],
                );
            }
            doc.append_text(td, cell.text);
            cells.push(td);
        }
    }
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    with_ahem(&mut doc);
    crate::layout::layout_single_page(&mut doc, &cascade, page_box_800x600()).expect("layout");
    Laid { doc, table, cells }
}

#[test]
fn spanning_percent_goes_to_the_columns_that_state_none() {
    // The percentage of a spanning cell that its columns do not state goes to the
    // columns that state none: 20% to the first, beside the 30% of the second,
    // and 40:60 fills the 100px the 30% column asks for.
    let laid = lay_out(
        "display:table;border-spacing:0",
        &[],
        &[
            &[spanning("AAAA", 2, "50%")],
            &[cell("B", ""), cell("CCC", "30%")],
        ],
    );
    laid.assert_close(
        (0.0, 0.0, 100.0, 20.0),
        &[
            (0.0, 0.0, 100.0, 10.0),
            (0.0, 10.0, 40.0, 10.0),
            (40.0, 10.0, 60.0, 10.0),
        ],
    );
}

#[test]
fn a_percent_column_before_the_open_one() {
    // The open column is the one after the 10% column and takes the other 40%.
    let laid = lay_out(
        "display:table;border-spacing:0",
        &[],
        &[
            &[spanning("AAAA", 2, "50%")],
            &[cell("B", "10%"), cell("CCC", "")],
        ],
    );
    laid.assert_close(
        (0.0, 0.0, 100.0, 20.0),
        &[
            (0.0, 0.0, 100.0, 10.0),
            (0.0, 10.0, 20.0, 10.0),
            (20.0, 10.0, 80.0, 10.0),
        ],
    );
}

#[test]
fn open_columns_share_it_by_max_content() {
    // Two open columns share the 50% by their max-content widths, 20px and 60px.
    let laid = lay_out(
        "display:table;border-spacing:0",
        &[],
        &[
            &[spanning("AAAA", 2, "50%")],
            &[cell("BB", ""), cell("CCCCCC", "")],
        ],
    );
    laid.assert_close(
        (0.0, 0.0, 160.0, 20.0),
        &[
            (0.0, 0.0, 160.0, 10.0),
            (0.0, 10.0, 40.0, 10.0),
            (40.0, 10.0, 120.0, 10.0),
        ],
    );
}

#[test]
fn no_open_column_means_no_share() {
    // Every spanned column states a percentage, so the cell adds none, and the
    // columns (20% and 20%) fill the table in proportion.
    let laid = lay_out(
        "display:table;border-spacing:0",
        &[],
        &[
            &[spanning("AAAA", 2, "50%")],
            &[cell("B", "20%"), cell("CCC", "20%")],
        ],
    );
    laid.assert_close(
        (0.0, 0.0, 150.0, 20.0),
        &[
            (0.0, 0.0, 150.0, 10.0),
            (0.0, 10.0, 75.0, 10.0),
            (75.0, 10.0, 75.0, 10.0),
        ],
    );
}

#[test]
fn three_columns() {
    // A cell spanning three columns hands its remaining 50% to the two open ones.
    let laid = lay_out(
        "display:table;border-spacing:0",
        &[],
        &[
            &[spanning("AAAA", 3, "60%")],
            &[cell("B", ""), cell("CCC", "10%"), cell("DDDDD", "")],
        ],
    );
    laid.assert_close(
        (0.0, 0.0, 300.0, 20.0),
        &[
            (0.0, 0.0, 300.0, 10.0),
            (0.0, 10.0, 41.66, 10.0),
            (41.66, 10.0, 50.0, 10.0),
            (91.66, 10.0, 208.34, 10.0),
        ],
    );
}

#[test]
fn definite_table_width() {
    // The same table with a width of its own.
    let laid = lay_out(
        "display:table;border-spacing:0;width:300px",
        &[],
        &[
            &[spanning("AAAA", 2, "50%")],
            &[cell("B", ""), cell("CCC", "30%")],
        ],
    );
    laid.assert_close(
        (0.0, 0.0, 300.0, 20.0),
        &[
            (0.0, 0.0, 300.0, 10.0),
            (0.0, 10.0, 120.0, 10.0),
            (120.0, 10.0, 180.0, 10.0),
        ],
    );
}

#[test]
fn col_percent_before_spans() {
    // A `<col>` percentage counts before the spanning cell asks for its own.
    let laid = lay_out(
        "display:table;border-spacing:0",
        &["30%", ""],
        &[
            &[spanning("AAAA", 2, "50%")],
            &[cell("B", ""), cell("CCC", "")],
        ],
    );
    laid.assert_close(
        (0.0, 0.0, 150.0, 20.0),
        &[
            (0.0, 0.0, 150.0, 10.0),
            (0.0, 10.0, 90.0, 10.0),
            (90.0, 10.0, 60.0, 10.0),
        ],
    );
}

#[test]
fn spanning_max_content_goes_to_open_columns() {
    // A spanning cell wider than its columns grows the open column, not the
    // percentage one.
    let laid = lay_out(
        "display:table;border-spacing:0",
        &[],
        &[
            &[spanning("AAAAAAAAAA", 2, "")],
            &[cell("B", ""), cell("CCC", "30%")],
        ],
    );
    laid.assert_close(
        (0.0, 0.0, 100.0, 20.0),
        &[
            (0.0, 0.0, 100.0, 10.0),
            (0.0, 10.0, 70.0, 10.0),
            (70.0, 10.0, 30.0, 10.0),
        ],
    );
}

#[test]
fn spanning_max_content_by_max_content() {
    // Open columns share a spanning cell's width by their max-content widths.
    let laid = lay_out(
        "display:table;border-spacing:0",
        &[],
        &[
            &[spanning("AAAAAAAAAA", 2, "")],
            &[cell("B", ""), cell("CCC", "")],
        ],
    );
    laid.assert_close(
        (0.0, 0.0, 100.0, 20.0),
        &[
            (0.0, 0.0, 100.0, 10.0),
            (0.0, 10.0, 25.0, 10.0),
            (25.0, 10.0, 75.0, 10.0),
        ],
    );
}

#[test]
fn percent_columns_fill_the_table() {
    // Percentage columns that do not add up to 100% grow in proportion to them.
    let laid = lay_out(
        "display:table;border-spacing:0;width:300px",
        &[],
        &[&[cell("B", "20%"), cell("CCC", "40%")]],
    );
    laid.assert_close(
        (0.0, 0.0, 300.0, 10.0),
        &[(0.0, 0.0, 100.0, 10.0), (100.0, 0.0, 200.0, 10.0)],
    );
}

#[test]
fn length_column_takes_the_rest_before_percent() {
    // A column that states a length takes what is left before a percentage column
    // does, so the 20% column stays at 20%.
    let laid = lay_out(
        "display:table;border-spacing:0;width:300px",
        &[],
        &[&[cell("B", "20%"), cell("CCC", "50px")]],
    );
    laid.assert_close(
        (0.0, 0.0, 300.0, 10.0),
        &[(0.0, 0.0, 60.0, 10.0), (60.0, 0.0, 240.0, 10.0)],
    );
}

#[test]
fn percent_stops_at_the_whole() {
    // The percentages are taken in order and stop at 100%: 60% and 60% are 60% and
    // 40%.
    let laid = lay_out(
        "display:table;border-spacing:0;width:300px",
        &[],
        &[&[cell("B", "60%"), cell("CCC", "60%")]],
    );
    laid.assert_close(
        (0.0, 0.0, 300.0, 10.0),
        &[(0.0, 0.0, 180.0, 10.0), (180.0, 0.0, 120.0, 10.0)],
    );
}

#[test]
fn auto_columns_share_by_max_content() {
    // Auto columns share what their max-content widths leave by those widths.
    let laid = lay_out(
        "display:table;border-spacing:0;width:300px",
        &[],
        &[&[cell("B", ""), cell("CCC", "")]],
    );
    laid.assert_close(
        (0.0, 0.0, 300.0, 10.0),
        &[(0.0, 0.0, 75.0, 10.0), (75.0, 0.0, 225.0, 10.0)],
    );
}

#[test]
fn spanning_length_over_percent_column() {
    // A spanning cell with a length grows the open column, not the percentage one.
    let laid = lay_out(
        "display:table;border-spacing:0",
        &[],
        &[
            &[spanning("AAAA", 2, "100px")],
            &[cell("B", ""), cell("CCC", "30%")],
        ],
    );
    laid.assert_close(
        (0.0, 0.0, 100.0, 20.0),
        &[
            (0.0, 0.0, 100.0, 10.0),
            (0.0, 10.0, 70.0, 10.0),
            (70.0, 10.0, 30.0, 10.0),
        ],
    );
}

#[test]
fn a_length_column_is_not_widened_by_the_content_of_its_other_cells() {
    // The 50px column is as wide as the widest length it states, so the long text
    // in its other cell wraps inside it.
    let laid = lay_out(
        "display:table;border-spacing:0",
        &[],
        &[&[cell("A", "50px")], &[cell("AA AA AA AA", "")]],
    );
    laid.assert_close(
        (0.0, 0.0, 50.0, 30.0),
        &[(0.0, 0.0, 50.0, 10.0), (0.0, 10.0, 50.0, 20.0)],
    );
}

#[test]
fn unbreakable_content_widens_a_length_column() {
    // Content that cannot wrap is wider than the length, and the column follows it.
    let laid = lay_out(
        "display:table;border-spacing:0",
        &[],
        &[&[cell("A", "50px")], &[cell("AAAAAAAA", "")]],
    );
    laid.assert_close(
        (0.0, 0.0, 80.0, 20.0),
        &[(0.0, 0.0, 80.0, 10.0), (0.0, 10.0, 80.0, 10.0)],
    );
}

#[test]
fn a_spanning_maximum_is_shared_over_the_maximums_before_the_minimum_raised_them() {
    // The spanning cell's minimum (100px) lifts the 30px column past its own
    // maximum before its maximum (200px) is shared out. Chromium shares that
    // maximum over the old maximums, so the first column ends up far wider than
    // the cell asks for and the 30% column makes the table 566.66px wide.
    let laid = lay_out(
        "display:table;border-spacing:0",
        &[],
        &[
            &[cell("A", "30px"), cell("A A", "30%")],
            &[spanning("AAAAAAAAAA AAAAAAAAA", 2, "")],
        ],
    );
    laid.assert_close(
        (0.0, 0.0, 566.66, 20.0),
        &[
            (0.0, 0.0, 396.67, 10.0),
            (396.67, 0.0, 169.98, 10.0),
            (0.0, 10.0, 566.66, 10.0),
        ],
    );
}
