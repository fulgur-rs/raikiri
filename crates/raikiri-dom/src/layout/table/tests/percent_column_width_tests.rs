//! The width of an auto-width table whose columns carry percentage widths.
//!
//! A table that is as wide as its content wants to be (a block-level table, an
//! inline table, a float, an absolutely positioned table) is wide enough for
//! every column to have its max-content width once the percentages are
//! resolved against the table itself: a percentage column of max-content `m`
//! and share `p` needs `m / p`, and the other columns need their total over
//! what the percentages leave. The room of the containing block and a
//! `max-width` still cap it. A flex item and a table cell take the table at its
//! plain max-content width. Every rectangle here is what Chromium reports for
//! the same markup (Ahem 10px, no body margin).

use crate::document::Document;
use crate::layout::test_support::{absolute_rect, page_box_800x600, with_ahem};
use raikiri_style::{build_rule_tree, cascade};

/// A rectangle as (left, top, width, height).
type Rect = (f32, f32, f32, f32);

/// The content of one table cell.
#[derive(Clone, Copy)]
enum Content {
    /// A block of the given width, 10px tall.
    Block(f32),
    /// Ahem text.
    Text(&'static str),
}

/// A table column: a single cell with `content` and, when set, a `width` of
/// that many percent.
#[derive(Clone, Copy)]
struct Column {
    content: Content,
    percent: Option<f32>,
    /// Extra declarations for the cell.
    css: &'static str,
}

fn column(content: f32, percent: Option<f32>) -> Column {
    Column {
        content: Content::Block(content),
        percent,
        css: "",
    }
}

const FLOAT: &str = "display:table;float:left";
const INLINE: &str = "display:inline-table";
const ABSOLUTE: &str = "display:table;position:absolute;top:0";
const BLOCK: &str = "display:table";

/// The table and its cells once the page below is laid out.
struct Laid {
    doc: Document,
    table: usize,
    cells: Vec<usize>,
}

impl Laid {
    fn table(&self) -> Rect {
        absolute_rect(&self.doc, self.table)
    }

    fn cells(&self) -> Vec<Rect> {
        self.cells
            .iter()
            .map(|&cell| absolute_rect(&self.doc, cell))
            .collect()
    }
}

/// A document with `html > body` on Ahem text, returning it and the body.
fn page() -> (Document, usize) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;font-family:Ahem;font-size:10px;line-height:10px"),
    );
    (doc, body)
}

/// Appends `table > row > cells` under `parent` and returns the table and the
/// cells.
fn append_table(
    doc: &mut Document,
    parent: usize,
    table_css: &str,
    columns: &[Column],
) -> (usize, Vec<usize>) {
    let table = doc.append_element(Some(parent), "div", Some(table_css));
    let row = doc.append_element(Some(table), "div", Some("display:table-row"));
    let cells = columns
        .iter()
        .map(|column| {
            let width = column
                .percent
                .map_or(String::new(), |percent| format!(";width:{percent}%"));
            let cell = doc.append_element(
                Some(row),
                "div",
                Some(&format!(
                    "display:table-cell{width};{css}",
                    css = column.css
                )),
            );
            match column.content {
                Content::Block(width) => {
                    doc.append_element(
                        Some(cell),
                        "div",
                        Some(&format!("display:block;width:{width}px;height:10px")),
                    );
                }
                Content::Text(text) => {
                    doc.append_text(cell, text);
                }
            }
            cell
        })
        .collect();
    (table, cells)
}

/// Lays `doc` out on an 800x600 page.
fn finish(mut doc: Document, table: usize, cells: Vec<usize>) -> Laid {
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    with_ahem(&mut doc);
    crate::layout::layout_single_page(&mut doc, &cascade, page_box_800x600()).expect("layout");
    Laid { doc, table, cells }
}

/// `body > div(position:relative; 200px wide) > table(table_css) > row >
/// cells`, laid out on an 800x600 page. `container_css` is added to the
/// containing block and `wrapper_css`, when given, is a box between the
/// containing block and the table.
fn lay_out_in(
    container_css: &str,
    wrapper_css: Option<&str>,
    table_css: &str,
    columns: &[Column],
) -> Laid {
    let (mut doc, body) = page();
    let mut parent = doc.append_element(
        Some(body),
        "div",
        Some(&format!(
            "display:block;position:relative;width:200px;height:100px;{container_css}"
        )),
    );
    if let Some(wrapper_css) = wrapper_css {
        parent = doc.append_element(Some(parent), "div", Some(wrapper_css));
    }
    let (table, cells) = append_table(&mut doc, parent, table_css, columns);
    finish(doc, table, cells)
}

fn lay_out(table_css: &str, columns: &[Column]) -> Laid {
    lay_out_in("", None, table_css, columns)
}

/// Asserts that two rectangles agree to within float rounding, which shows
/// when a share such as 10% is not exact.
fn assert_close(got: Rect, expected: Rect, message: &str) {
    let close = |a: f32, b: f32| (a - b).abs() < 0.001;
    assert!(
        close(got.0, expected.0)
            && close(got.1, expected.1)
            && close(got.2, expected.2)
            && close(got.3, expected.3),
        "{message}: {got:?} is not {expected:?}"
    );
}

/// The table's box and its cells' boxes as (table, cells).
fn rects(table_css: &str, columns: &[Column]) -> (Rect, Vec<Rect>) {
    let laid = lay_out(table_css, columns);
    (laid.table(), laid.cells())
}

#[test]
fn a_percentage_column_widens_the_table_to_fit_the_other_columns() {
    // The 30px column is the other half of the table, so the table is 60px
    // and the 50% column takes 30px of it.
    for table_css in [BLOCK, INLINE, FLOAT, ABSOLUTE] {
        let (table, cells) = rects(table_css, &[column(10.0, Some(50.0)), column(30.0, None)]);
        assert_eq!(table, (0.0, 0.0, 60.0, 10.0), "{table_css}");
        assert_eq!(
            cells,
            [(0.0, 0.0, 30.0, 10.0), (30.0, 0.0, 30.0, 10.0)],
            "{table_css}"
        );
    }
}

#[test]
fn the_widest_demand_of_any_percentage_column_or_the_rest_sets_the_width() {
    let cases = [
        // 20% of the table has to hold 10px (50px), the rest 50px of 80% (62.5px).
        (
            vec![
                column(10.0, Some(20.0)),
                column(20.0, None),
                column(30.0, None),
            ],
            (0.0, 0.0, 62.5, 10.0),
            vec![
                (0.0, 0.0, 12.5, 10.0),
                (12.5, 0.0, 20.0, 10.0),
                (32.5, 0.0, 30.0, 10.0),
            ],
        ),
        // Two percentage columns: 30px of 50% is 60px.
        (
            vec![column(10.0, Some(50.0)), column(30.0, Some(50.0))],
            (0.0, 0.0, 60.0, 10.0),
            vec![(0.0, 0.0, 30.0, 10.0), (30.0, 0.0, 30.0, 10.0)],
        ),
        // A column that is almost the whole table: 10px of 90% is 11px, the
        // other column's 10px of the remaining 10% is 100px.
        (
            vec![column(10.0, Some(90.0)), column(10.0, None)],
            (0.0, 0.0, 100.0, 10.0),
            vec![(0.0, 0.0, 90.0, 10.0), (90.0, 0.0, 10.0, 10.0)],
        ),
        // Without other content the percentage column alone sets the width.
        (
            vec![column(0.0, Some(50.0)), column(30.0, None)],
            (0.0, 0.0, 60.0, 10.0),
            vec![(0.0, 0.0, 30.0, 10.0), (30.0, 0.0, 30.0, 10.0)],
        ),
        (
            vec![column(10.0, Some(50.0)), column(0.0, None)],
            (0.0, 0.0, 20.0, 10.0),
            vec![(0.0, 0.0, 10.0, 10.0), (10.0, 0.0, 10.0, 10.0)],
        ),
    ];
    for (columns, table, cells) in cases {
        for table_css in [BLOCK, FLOAT] {
            let (got_table, got_cells) = rects(table_css, &columns);
            assert_close(got_table, table, table_css);
            assert_eq!(got_cells.len(), cells.len(), "{table_css}");
            for (got, expected) in got_cells.into_iter().zip(cells.iter().copied()) {
                assert_close(got, expected, table_css);
            }
        }
    }
}

#[test]
fn a_lone_percentage_column_is_as_wide_as_its_share_needs() {
    // 10px is 30% of 33.33px.
    let (table, cells) = rects(FLOAT, &[column(10.0, Some(30.0))]);
    assert!((table.2 - 100.0 / 3.0).abs() < 0.01, "{table:?}");
    assert_eq!(cells, [(0.0, 0.0, table.2, 10.0)]);
    // A column of 100% needs exactly its own content.
    let (table, _) = rects(FLOAT, &[column(10.0, Some(100.0))]);
    assert_eq!(table, (0.0, 0.0, 10.0, 10.0));
}

#[test]
fn text_in_a_percentage_column_counts_at_its_max_content_width() {
    // The second column is 90px of text on one line, and 25% is left for the
    // first: 90 / 0.75 = 120.
    let columns = [
        Column {
            content: Content::Text("xx"),
            percent: Some(25.0),
            css: "",
        },
        Column {
            content: Content::Text("xxxx xxxx"),
            percent: None,
            css: "",
        },
    ];
    let (table, cells) = rects(INLINE, &columns);
    assert_eq!(table, (0.0, 0.0, 120.0, 10.0));
    assert_eq!(cells, [(0.0, 0.0, 30.0, 10.0), (30.0, 0.0, 90.0, 10.0)]);
}

#[test]
fn an_authored_column_width_counts_with_the_columns_that_have_no_percentage() {
    // 10px + 30px of lengths take half the table.
    let columns = [
        column(10.0, Some(50.0)),
        column(10.0, None),
        Column {
            content: Content::Block(10.0),
            percent: None,
            css: "width:30px",
        },
    ];
    let (table, cells) = rects(FLOAT, &columns);
    assert_eq!(table, (0.0, 0.0, 80.0, 10.0));
    assert_eq!(
        cells,
        [
            (0.0, 0.0, 40.0, 10.0),
            (40.0, 0.0, 10.0, 10.0),
            (50.0, 0.0, 30.0, 10.0)
        ]
    );
}

#[test]
fn percentages_that_take_everything_fill_the_room() {
    // No width is enough for 30px of content in 0% of the table, so the table
    // takes the whole 200px of its containing block.
    let (table, cells) = rects(FLOAT, &[column(10.0, Some(100.0)), column(30.0, None)]);
    assert_eq!(table, (0.0, 0.0, 200.0, 10.0));
    assert_eq!(cells, [(0.0, 0.0, 170.0, 10.0), (170.0, 0.0, 30.0, 10.0)]);
    let (table, _) = rects(
        FLOAT,
        &[
            column(10.0, Some(50.0)),
            column(10.0, Some(50.0)),
            column(30.0, None),
        ],
    );
    assert_eq!(table.2, 200.0);
}

#[test]
fn shares_past_one_hundred_percent_in_total_get_nothing() {
    // The second 60% gets the 40% that is left, so 10px of it is 25px, and the
    // cells follow the same split.
    let (table, cells) = rects(FLOAT, &[column(10.0, Some(60.0)), column(10.0, Some(60.0))]);
    assert_close(table, (0.0, 0.0, 25.0, 10.0), "two 60% columns");
    assert_close(cells[0], (0.0, 0.0, 15.0, 10.0), "first of two 60% columns");
    assert_close(
        cells[1],
        (15.0, 0.0, 10.0, 10.0),
        "second of two 60% columns",
    );
    // The first 60% has to hold its 90px, which sets the width, and the
    // column is not squeezed to half of it by the second share.
    let (table, cells) = rects(FLOAT, &[column(90.0, Some(60.0)), column(10.0, Some(60.0))]);
    assert_close(table, (0.0, 0.0, 150.0, 10.0), "wide first 60% column");
    assert_close(cells[0], (0.0, 0.0, 90.0, 10.0), "wide first 60% column");
    assert_close(
        cells[1],
        (90.0, 0.0, 60.0, 10.0),
        "narrow second 60% column",
    );
    // A column after the first 100% has nothing left, like one without a
    // share: its 30px have no width that is enough, and the table fills the
    // room.
    let (table, _) = rects(
        FLOAT,
        &[column(10.0, Some(100.0)), column(30.0, Some(10.0))],
    );
    assert_eq!(table, (0.0, 0.0, 200.0, 10.0));
    let (table, _) = rects(
        FLOAT,
        &[column(30.0, Some(10.0)), column(10.0, Some(100.0))],
    );
    assert_eq!(table, (0.0, 0.0, 200.0, 10.0));
}

#[test]
fn the_room_of_the_containing_block_caps_the_width() {
    let columns = [column(10.0, Some(50.0)), column(30.0, None)];
    // 50px of room: 25px for the percentage column would leave 25px for the
    // 30px one, which cannot shrink, so the percentage column gives way.
    let laid = lay_out_in("width:50px", None, FLOAT, &columns);
    assert_eq!(laid.table(), (0.0, 0.0, 50.0, 10.0));
    assert_eq!(
        laid.cells(),
        [(0.0, 0.0, 20.0, 10.0), (20.0, 0.0, 30.0, 10.0)]
    );
    // 70px is room enough.
    let laid = lay_out_in("width:70px", None, FLOAT, &columns);
    assert_eq!(laid.table(), (0.0, 0.0, 60.0, 10.0));
    // Content that is wider than the room keeps its width.
    let laid = lay_out(FLOAT, &[column(100.0, Some(10.0)), column(100.0, None)]);
    assert_eq!(laid.table(), (0.0, 0.0, 200.0, 10.0));
    assert_eq!(
        laid.cells(),
        [(0.0, 0.0, 100.0, 10.0), (100.0, 0.0, 100.0, 10.0)]
    );
}

#[test]
fn text_in_a_narrow_room_wraps_before_the_percentage_column_gives_way() {
    let columns = [
        Column {
            content: Content::Text("xx xx"),
            percent: Some(25.0),
            css: "",
        },
        Column {
            content: Content::Text("xxx xxx xxx"),
            percent: None,
            css: "",
        },
    ];
    let laid = lay_out_in("width:60px", None, FLOAT, &columns);
    assert_eq!(laid.table(), (0.0, 0.0, 60.0, 30.0));
    assert_eq!(
        laid.cells(),
        [(0.0, 0.0, 20.0, 30.0), (20.0, 0.0, 40.0, 30.0)]
    );
}

#[test]
fn padding_borders_and_spacing_are_added_around_the_columns() {
    let columns = [column(10.0, Some(50.0)), column(30.0, None)];
    let (table, cells) = rects(
        "display:table;float:left;border:3px solid;padding:4px;border-spacing:5px",
        &columns,
    );
    assert_eq!(table, (0.0, 0.0, 89.0, 34.0));
    assert_eq!(cells, [(12.0, 12.0, 30.0, 10.0), (47.0, 12.0, 30.0, 10.0)]);
}

#[test]
fn a_width_or_a_percentage_free_table_is_not_widened() {
    // An authored width fills as before.
    let (table, cells) = rects(
        "display:table;float:left;width:100px",
        &[column(10.0, Some(50.0)), column(30.0, None)],
    );
    assert_eq!(table, (0.0, 0.0, 100.0, 10.0));
    assert_eq!(cells, [(0.0, 0.0, 50.0, 10.0), (50.0, 0.0, 50.0, 10.0)]);
    // Columns without percentages keep their max-content widths.
    let (table, cells) = rects(FLOAT, &[column(10.0, None), column(30.0, None)]);
    assert_eq!(table, (0.0, 0.0, 40.0, 10.0));
    assert_eq!(cells, [(0.0, 0.0, 10.0, 10.0), (10.0, 0.0, 30.0, 10.0)]);
}

#[test]
fn min_width_and_max_width_bound_the_widened_table() {
    let columns = [column(10.0, Some(50.0)), column(30.0, None)];
    // `min-width` grows the table by the percentages too.
    let (table, cells) = rects("display:table;float:left;min-width:80px", &columns);
    assert_eq!(table, (0.0, 0.0, 80.0, 10.0));
    assert_eq!(cells, [(0.0, 0.0, 40.0, 10.0), (40.0, 0.0, 40.0, 10.0)]);
    // `max-width` stops the widening, but not below the plain max-content width.
    let (table, cells) = rects("display:table;float:left;max-width:50px", &columns);
    assert_eq!(table, (0.0, 0.0, 50.0, 10.0));
    assert_eq!(cells, [(0.0, 0.0, 20.0, 10.0), (20.0, 0.0, 30.0, 10.0)]);
    let (table, _) = rects("display:table;float:left;max-width:30px", &columns);
    assert_eq!(table, (0.0, 0.0, 40.0, 10.0));
}

#[test]
fn a_fixed_layout_table_without_a_width_is_widened_like_an_auto_one() {
    let (table, cells) = rects(
        "display:table;float:left;table-layout:fixed",
        &[column(10.0, Some(50.0)), column(30.0, None)],
    );
    assert_eq!(table, (0.0, 0.0, 60.0, 10.0));
    assert_eq!(cells, [(0.0, 0.0, 30.0, 10.0), (30.0, 0.0, 30.0, 10.0)]);
}

#[test]
fn a_flex_column_item_is_widened_like_a_block_level_table() {
    // The cross size of a column flex item is the room of the container, so
    // the table shrink-wraps in it like it does in a block.
    let (_, cells) = rects(BLOCK, &[column(10.0, Some(50.0)), column(30.0, None)]);
    let laid = lay_out_in(
        "",
        Some("display:flex;flex-direction:column;align-items:flex-start"),
        BLOCK,
        &[column(10.0, Some(50.0)), column(30.0, None)],
    );
    assert_eq!(laid.table(), (0.0, 0.0, 60.0, 10.0));
    assert_eq!(laid.cells(), cells);
}

/// The inner table of `outer > row > cell > table > row > cells`, with the
/// outer table styled by `outer_css`.
fn table_in_a_cell(outer_css: &str, columns: &[Column]) -> Laid {
    let (mut doc, body) = page();
    let outer = doc.append_element(Some(body), "div", Some(outer_css));
    let outer_row = doc.append_element(Some(outer), "div", Some("display:table-row"));
    let outer_cell = doc.append_element(Some(outer_row), "div", Some("display:table-cell"));
    let (table, cells) = append_table(&mut doc, outer_cell, BLOCK, columns);
    finish(doc, table, cells)
}

#[test]
fn a_table_in_a_cell_is_taken_at_its_plain_max_content_width() {
    let columns = [column(10.0, Some(50.0)), column(30.0, None)];
    // The cell is as wide as the inner table's max-content width, and the
    // table fills it, with the percentage column squeezed to its content.
    let laid = table_in_a_cell("display:table;float:left", &columns);
    assert_eq!(laid.table(), (0.0, 0.0, 40.0, 10.0));
    assert_eq!(
        laid.cells(),
        [(0.0, 0.0, 10.0, 10.0), (10.0, 0.0, 30.0, 10.0)]
    );
    // A wider cell lets the table take the width its percentages need.
    let laid = table_in_a_cell("display:table;width:300px", &columns);
    assert_eq!(laid.table(), (0.0, 0.0, 60.0, 10.0));
    assert_eq!(
        laid.cells(),
        [(0.0, 0.0, 30.0, 10.0), (30.0, 0.0, 30.0, 10.0)]
    );
}

#[test]
fn a_flex_item_is_taken_at_its_plain_max_content_width() {
    let columns = [column(10.0, Some(50.0)), column(30.0, None)];
    for (wrapper, table_css) in [
        ("display:flex", BLOCK),
        ("display:flex;align-items:flex-start", BLOCK),
        ("display:inline-flex", BLOCK),
        ("display:flex;width:50px", BLOCK),
    ] {
        let laid = lay_out_in("", Some(wrapper), table_css, &columns);
        assert_eq!(laid.table(), (0.0, 0.0, 40.0, 10.0), "{wrapper}");
        assert_eq!(
            laid.cells(),
            [(0.0, 0.0, 10.0, 10.0), (10.0, 0.0, 30.0, 10.0)],
            "{wrapper}"
        );
    }
}
