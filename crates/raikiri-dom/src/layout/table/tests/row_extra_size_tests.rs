//! How a table shares the size it is given beyond its rows' content.
//!
//! A table that a flex or grid container stretches or grows, or whose block
//! size states or floors, hands the extra to its rows: a row that states a
//! percentage takes that share of the table, a row that states a size keeps
//! it, and the rest share what is left. A vertical table does the same along
//! the page, where its rows run across the width. Every rectangle here is what
//! Chromium reports for the same markup (Ahem 10px, no body margin).

use crate::document::Document;
use crate::layout::test_support::{absolute_rect, page_box_800x600, with_ahem};
use raikiri_style::{build_rule_tree, cascade};

/// A row of one cell holding a box of `size`; `css` styles the row, `cell_css`
/// the cell.
struct Row {
    css: &'static str,
    cell_css: &'static str,
    size: &'static str,
}

/// A laid-out table: the table and the cell of each row.
struct Laid {
    doc: Document,
    table: usize,
    cells: Vec<usize>,
}

/// A table with `table_css` and one row for each of `rows`, inside a `parent_css`
/// box when that is not empty.
fn lay_out(parent_css: &str, table_css: &str, rows: &[Row]) -> Laid {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;font-family:Ahem;font-size:10px;line-height:10px"),
    );
    let parent = if parent_css.is_empty() {
        body
    } else {
        doc.append_element(Some(body), "div", Some(parent_css))
    };
    let table = doc.append_element(Some(parent), "table", Some(table_css));
    let mut cells = Vec::new();
    for row in rows {
        let tr = doc.append_element(
            Some(table),
            "tr",
            Some(&format!("display:table-row;{}", row.css)),
        );
        let td = doc.append_element(
            Some(tr),
            "td",
            Some(&format!("display:table-cell;padding:0;{}", row.cell_css)),
        );
        doc.append_element(
            Some(td),
            "div",
            Some(&format!("display:block;{}", row.size)),
        );
        cells.push(td);
    }
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    with_ahem(&mut doc);
    crate::layout::layout_single_page(&mut doc, &cascade, page_box_800x600()).expect("layout");
    Laid { doc, table, cells }
}

/// Asserts that the table and its cells sit at the `(start, size)` pairs given
/// along the block axis: the top and height of a horizontal table, the left
/// and width of a vertical one.
fn assert_blocks(laid: &Laid, vertical: bool, table: (f32, f32), cells: &[(f32, f32)]) {
    let along = |node: usize| {
        let (x, y, width, height) = absolute_rect(&laid.doc, node);
        if vertical { (x, width) } else { (y, height) }
    };
    let close = |actual: (f32, f32), expected: (f32, f32), what: &str| {
        assert!(
            (actual.0 - expected.0).abs() < 0.05 && (actual.1 - expected.1).abs() < 0.05,
            "{what}: {actual:?}, expected {expected:?}"
        );
    };
    close(along(laid.table), table, "table");
    assert_eq!(laid.cells.len(), cells.len());
    for (index, (&cell, &expected)) in laid.cells.iter().zip(cells).enumerate() {
        close(along(cell), expected, &format!("row {index}"));
    }
}

const FLEX_ROW: &str = "display:flex;height:300px";
const TABLE: &str = "display:table;border-spacing:0";
const TWENTY_THEN_HALF: [Row; 2] = [
    Row {
        css: "",
        cell_css: "",
        size: "height:20px",
    },
    Row {
        css: "height:50%",
        cell_css: "",
        size: "height:10px",
    },
];

#[test]
fn a_flex_or_grid_container_gives_its_table_the_size_the_percentage_rows_take_a_share_of() {
    // 20px of content and a row of half the table: 150 and 150, not the
    // 200 and 100 that sharing the extra by content would give.
    for (parent, table) in [
        // A flex row stretches its item, a grid does too, and a flex column
        // grows it.
        (FLEX_ROW, TABLE),
        ("display:grid;height:300px", TABLE),
        (
            "display:flex;flex-direction:column;height:300px",
            "display:table;border-spacing:0;flex:1",
        ),
    ] {
        let laid = lay_out(parent, table, &TWENTY_THEN_HALF);
        assert_blocks(&laid, false, (0.0, 300.0), &[(0.0, 150.0), (150.0, 150.0)]);
    }
}

#[test]
fn rows_with_a_percentage_settle_in_order_before_the_free_rows_share_the_rest() {
    let rows = [
        Row {
            css: "height:50%",
            cell_css: "",
            size: "height:20px",
        },
        Row {
            css: "",
            cell_css: "",
            size: "height:20px",
        },
        Row {
            css: "height:25%",
            cell_css: "",
            size: "height:10px",
        },
    ];
    let laid = lay_out("display:grid;height:300px", TABLE, &rows);
    assert_blocks(
        &laid,
        false,
        (0.0, 300.0),
        &[(0.0, 150.0), (150.0, 75.0), (225.0, 75.0)],
    );
}

#[test]
fn a_percentage_resolves_against_the_rows_without_the_border_padding_and_edge_spacing() {
    let border = lay_out(
        FLEX_ROW,
        "display:table;border:3px solid;border-spacing:2px",
        &TWENTY_THEN_HALF,
    );
    assert_blocks(
        &border,
        false,
        (0.0, 300.0),
        &[(5.0, 143.0), (150.0, 145.0)],
    );
    let padding = lay_out(
        FLEX_ROW,
        "display:table;padding:5px;border-spacing:0",
        &TWENTY_THEN_HALF,
    );
    assert_blocks(
        &padding,
        false,
        (0.0, 300.0),
        &[(5.0, 145.0), (150.0, 145.0)],
    );
}

#[test]
fn a_percentage_on_a_cell_or_a_pure_percentage_calc_counts_for_its_row() {
    let rows = [
        Row {
            css: "",
            cell_css: "height:50%",
            size: "height:10px",
        },
        Row {
            css: "",
            cell_css: "",
            size: "height:20px",
        },
    ];
    let cell = lay_out(FLEX_ROW, TABLE, &rows);
    assert_blocks(&cell, false, (0.0, 300.0), &[(0.0, 150.0), (150.0, 150.0)]);
    let rows = [
        Row {
            css: "height:calc(25% * 2)",
            cell_css: "",
            size: "height:20px",
        },
        Row {
            css: "",
            cell_css: "",
            size: "height:10px",
        },
    ];
    let calc = lay_out(FLEX_ROW, TABLE, &rows);
    assert_blocks(&calc, false, (0.0, 300.0), &[(0.0, 150.0), (150.0, 150.0)]);
}

#[test]
fn a_table_that_gets_no_extra_height_keeps_its_rows_at_their_content() {
    // A flex column that does not grow its item, and a row that centres it,
    // leave nothing for a percentage to take.
    let column = lay_out(
        "display:flex;flex-direction:column;height:300px",
        TABLE,
        &TWENTY_THEN_HALF,
    );
    assert_blocks(&column, false, (0.0, 30.0), &[(0.0, 20.0), (20.0, 10.0)]);
    let centred = lay_out(
        "display:flex;height:300px;align-items:center",
        TABLE,
        &TWENTY_THEN_HALF,
    );
    assert_blocks(
        &centred,
        false,
        (135.0, 30.0),
        &[(135.0, 20.0), (155.0, 10.0)],
    );
}

const VERTICAL: &str = "display:table;writing-mode:vertical-rl;border-spacing:0";

#[test]
fn the_rows_of_a_vertical_table_take_their_percentage_of_its_width() {
    // Rows run across the width of a vertical table: the first row is the
    // rightmost. A percentage `width` is the share of the table.
    let rows = [
        Row {
            css: "",
            cell_css: "",
            size: "width:20px;height:10px",
        },
        Row {
            css: "width:50%",
            cell_css: "",
            size: "width:10px;height:10px",
        },
    ];
    let half = lay_out("", &format!("{VERTICAL};width:200px"), &rows);
    assert_blocks(&half, true, (0.0, 200.0), &[(100.0, 100.0), (0.0, 100.0)]);
    let left_to_right = lay_out(
        "",
        "display:table;writing-mode:vertical-lr;border-spacing:0;width:200px",
        &rows,
    );
    assert_blocks(
        &left_to_right,
        true,
        (0.0, 200.0),
        &[(0.0, 100.0), (100.0, 100.0)],
    );
    // The spacing between rows is part of what the percentage is a share of,
    // the edges are not.
    let spaced = lay_out(
        "",
        "display:table;writing-mode:vertical-rl;border-spacing:4px;width:200px",
        &rows,
    );
    assert_blocks(&spaced, true, (0.0, 200.0), &[(104.0, 92.0), (4.0, 96.0)]);
    // A stretched table has a width as well.
    let stretched = lay_out(
        "display:flex;flex-direction:column;width:300px",
        &format!("{VERTICAL};align-self:stretch"),
        &rows,
    );
    assert_blocks(
        &stretched,
        true,
        (0.0, 300.0),
        &[(150.0, 150.0), (0.0, 150.0)],
    );
}

#[test]
fn vertical_rows_settle_their_percentages_in_order_and_leave_the_free_row_the_rest() {
    let rows = [
        Row {
            css: "width:50%",
            cell_css: "",
            size: "width:20px;height:10px",
        },
        Row {
            css: "",
            cell_css: "",
            size: "width:10px;height:10px",
        },
        Row {
            css: "width:20%",
            cell_css: "",
            size: "width:10px;height:10px",
        },
    ];
    let laid = lay_out("", &format!("{VERTICAL};width:300px"), &rows);
    assert_blocks(
        &laid,
        true,
        (0.0, 300.0),
        &[(150.0, 150.0), (60.0, 90.0), (0.0, 60.0)],
    );
}

#[test]
fn a_vertical_row_keeps_the_width_it_states_and_is_never_narrower() {
    let rows = [
        Row {
            css: "width:50px",
            cell_css: "",
            size: "width:20px;height:10px",
        },
        Row {
            css: "",
            cell_css: "",
            size: "width:10px;height:10px",
        },
    ];
    // In a table of a stated width the free row takes the rest.
    let sized = lay_out("", &format!("{VERTICAL};width:200px"), &rows);
    assert_blocks(&sized, true, (0.0, 200.0), &[(150.0, 50.0), (0.0, 150.0)]);
    // In one that shrinks to fit, the row is as wide as it states.
    let shrunk = lay_out("", VERTICAL, &rows);
    assert_blocks(&shrunk, true, (0.0, 60.0), &[(10.0, 50.0), (0.0, 10.0)]);
}

#[test]
fn a_height_percentage_means_nothing_to_the_rows_of_a_vertical_table() {
    // A row's `height` is its extent along the table's inline axis, which the
    // row always fills; only its content decides how the width is shared.
    let rows = [
        Row {
            css: "height:50%",
            cell_css: "",
            size: "width:20px;height:10px",
        },
        Row {
            css: "",
            cell_css: "",
            size: "width:10px;height:10px",
        },
    ];
    let laid = lay_out("", &format!("{VERTICAL};height:200px;width:100px"), &rows);
    assert_blocks(&laid, true, (0.0, 100.0), &[(33.34, 66.66), (0.0, 33.34)]);
}
