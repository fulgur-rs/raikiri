//! Collapsed borders of row groups, columns and column groups (CSS 2.1
//! §17.6.2.1). The cases follow Chromium's rendering of the same markup,
//! except where a test says otherwise: the cell sides take the winning
//! border, and the parts paint nothing of their own.

use super::*;

struct Fixture {
    doc: Document,
    cells: Vec<Vec<usize>>,
    parts: Vec<usize>,
}

/// A collapsed table with `columns` (column or column-group styles placed
/// before the rows) and `groups` of rows; each group lists its rows as cell
/// counts. Cells carry `cell_css`.
fn layout(
    table_css: &str,
    columns: &[(&str, &str)],
    groups: &[(&str, &[usize])],
    cell_css: &str,
) -> Fixture {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block;margin:0"));
    let table = doc.append_element(
        Some(body),
        "table",
        Some(&format!("display:table;{table_css}")),
    );
    let mut parts = Vec::new();
    for &(tag, css) in columns {
        let display = if tag == "col" {
            "table-column"
        } else {
            "table-column-group"
        };
        parts.push(doc.append_element(Some(table), tag, Some(&format!("display:{display};{css}"))));
    }
    let mut cells = Vec::new();
    for &(css, rows) in groups {
        let group = doc.append_element(
            Some(table),
            "tbody",
            Some(&format!("display:table-row-group;{css}")),
        );
        parts.push(group);
        for &count in rows {
            let row = doc.append_element(Some(group), "tr", Some("display:table-row"));
            cells.push(
                (0..count)
                    .map(|_| {
                        doc.append_element(
                            Some(row),
                            "td",
                            Some(&format!(
                                "display:table-cell;width:20px;height:20px;{cell_css}"
                            )),
                        )
                    })
                    .collect(),
            );
        }
    }
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    crate::layout::layout_single_page(&mut doc, &computed, PageBox::A4).unwrap();
    Fixture { doc, cells, parts }
}

/// Lay out `html` as the children of a collapsed table, with 20px cells, and
/// return the document and the cells in tree order.
fn layout_markup(
    table_css: &str,
    build: impl FnOnce(&mut Document, usize),
) -> (Document, Vec<usize>) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block;margin:0"));
    let table = doc.append_element(
        Some(body),
        "table",
        Some(&format!(
            "display:table;border-collapse:collapse;{table_css}"
        )),
    );
    build(&mut doc, table);
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    crate::layout::layout_single_page(&mut doc, &computed, PageBox::A4).unwrap();
    let cells = (0..doc.nodes.len())
        .filter(|&idx| doc.nodes[idx].display() == DisplayValue::TableCell)
        .collect();
    (doc, cells)
}

const CELL: &str = "display:table-cell;width:20px;height:20px";
const GREEN: BorderColor = BorderColor::Resolved(CssColor {
    r: 0,
    g: 128,
    b: 0,
    a: 255,
});
const RED: BorderColor = BorderColor::Resolved(CssColor {
    r: 255,
    g: 0,
    b: 0,
    a: 255,
});

/// The painted widths of a cell, as `[top, right, bottom, left]`.
fn widths(doc: &Document, cell: usize) -> [f32; 4] {
    let sides = doc.nodes[cell].collapsed_border.expect("collapsed paint");
    [sides.top, sides.right, sides.bottom, sides.left].map(|side| side.width().px())
}

#[test]
fn row_group_borders_reach_the_cells_on_the_group_edges() {
    let f = layout(
        "border-collapse:collapse",
        &[],
        &[("border:6px solid", &[2, 2]), ("border:2px solid", &[2])],
        "",
    );
    // The first group's outer sides go to its cells; inner lines stay empty.
    assert_eq!(widths(&f.doc, f.cells[0][0]), [6.0, 0.0, 0.0, 6.0]);
    assert_eq!(widths(&f.doc, f.cells[0][1]), [6.0, 6.0, 0.0, 0.0]);
    // Between the groups the wider bottom of the first group wins.
    assert_eq!(widths(&f.doc, f.cells[1][0]), [0.0, 0.0, 6.0, 6.0]);
    assert_eq!(widths(&f.doc, f.cells[2][0]), [6.0, 0.0, 2.0, 2.0]);
    // The groups themselves paint nothing.
    for &group in &f.parts {
        let sides = f.doc.nodes[group]
            .collapsed_border
            .expect("suppressed group");
        assert_eq!(sides.top.width().px(), 0.0);
        assert_eq!(sides.left.width().px(), 0.0);
    }
}

#[test]
fn a_cell_border_wins_a_tie_with_its_row_group() {
    let f = layout(
        "border-collapse:collapse",
        &[],
        &[("border:4px dotted", &[1])],
        "border:4px solid",
    );
    let sides = f.doc.nodes[f.cells[0][0]].collapsed_border.unwrap();
    assert_eq!(sides.top.style(), BorderStyle::Solid);
    // A wider group border beats the cell.
    let f = layout(
        "border-collapse:collapse",
        &[],
        &[("border:8px dotted", &[1])],
        "border:4px solid",
    );
    let sides = f.doc.nodes[f.cells[0][0]].collapsed_border.unwrap();
    assert_eq!(sides.top.style(), BorderStyle::Dotted);
    assert_eq!(sides.top.width().px(), 8.0);
}

#[test]
fn a_group_line_reaches_cells_beside_an_empty_slot() {
    // The second row of the first group has one cell; the slot beside it is
    // empty, yet the cell above the next group still takes the line there.
    let f = layout(
        "border-collapse:collapse",
        &[],
        &[("border-bottom:5px solid", &[2, 1]), ("", &[2])],
        "",
    );
    assert_eq!(widths(&f.doc, f.cells[1][0])[2], 5.0);
    assert_eq!(widths(&f.doc, f.cells[2][1])[0], 5.0);
}

#[test]
fn column_and_column_group_borders_reach_their_cells() {
    let f = layout(
        "border-collapse:collapse",
        &[
            ("col", "border:3px solid"),
            ("colgroup", "border:7px solid"),
        ],
        &[("", &[2, 2])],
        "",
    );
    // Column 0 is a <col>; its sides reach the cells in it.
    assert_eq!(widths(&f.doc, f.cells[0][0]), [3.0, 7.0, 0.0, 3.0]);
    assert_eq!(widths(&f.doc, f.cells[1][0]), [0.0, 7.0, 3.0, 3.0]);
    // Column 1 is an empty <colgroup>: its wider left side wins the line.
    assert_eq!(widths(&f.doc, f.cells[0][1]), [7.0, 7.0, 0.0, 7.0]);
    assert_eq!(widths(&f.doc, f.cells[1][1]), [0.0, 7.0, 7.0, 7.0]);
}

#[test]
fn a_row_group_wins_over_a_column_of_the_same_border() {
    let f = layout(
        "border-collapse:collapse",
        &[("col", "border:4px solid rgb(0, 0, 255)")],
        &[("border:4px solid rgb(0, 128, 0)", &[1])],
        "",
    );
    let sides = f.doc.nodes[f.cells[0][0]].collapsed_border.unwrap();
    let green = BorderColor::Resolved(CssColor {
        r: 0,
        g: 128,
        b: 0,
        a: 255,
    });
    assert_eq!(sides.top.color, green);
    assert_eq!(sides.left.color, green);
}

#[test]
fn separated_row_groups_leave_their_cells_alone() {
    let f = layout(
        "border-collapse:separate",
        &[("col", "border:3px solid")],
        &[("border:6px solid", &[1])],
        "",
    );
    let cell = f.cells[0][0];
    assert!(f.doc.nodes[cell].collapsed_border.is_none());
}

#[test]
fn a_cell_border_wins_a_tie_with_the_group_across_the_line() {
    // The upper group's bottom and the lower cell's top are the same border
    // but for colour; the cell's wins, on both cells of the line.
    let (doc, cells) = layout_markup("", |doc, table| {
        let upper = doc.append_element(
            Some(table),
            "tbody",
            Some("display:table-row-group;border-bottom:4px solid rgb(0, 128, 0)"),
        );
        let row = doc.append_element(Some(upper), "tr", Some("display:table-row"));
        doc.append_element(Some(row), "td", Some(CELL));
        let lower = doc.append_element(Some(table), "tbody", Some("display:table-row-group"));
        let row = doc.append_element(Some(lower), "tr", Some("display:table-row"));
        doc.append_element(
            Some(row),
            "td",
            Some(&format!("{CELL};border-top:4px solid rgb(255, 0, 0)")),
        );
    });
    let upper = doc.nodes[cells[0]].collapsed_border.unwrap();
    let lower = doc.nodes[cells[1]].collapsed_border.unwrap();
    assert_eq!(upper.bottom.color, RED);
    assert_eq!(lower.top.color, RED);
    assert_eq!(lower.top.width().px(), 4.0);
}

#[test]
fn a_collapsed_column_takes_its_part_out_of_the_lines() {
    // The first column collapses; the bordered second column is then the
    // first one shown, and its border goes to the cells in it.
    let (doc, cells) = layout_markup("", |doc, table| {
        doc.append_element(
            Some(table),
            "col",
            Some("display:table-column;visibility:collapse"),
        );
        doc.append_element(
            Some(table),
            "col",
            Some("display:table-column;border:4px solid rgb(0, 128, 0)"),
        );
        let row = doc.append_element(Some(table), "tr", Some("display:table-row"));
        for _ in 0..3 {
            doc.append_element(Some(row), "td", Some(CELL));
        }
    });
    assert_eq!(widths(&doc, cells[1]), [4.0, 4.0, 4.0, 4.0]);
    assert_eq!(widths(&doc, cells[2]), [0.0, 0.0, 0.0, 4.0]);
    assert_eq!(
        doc.nodes[cells[1]].collapsed_border.unwrap().top.color,
        GREEN
    );
}

#[test]
fn a_spanning_cell_takes_the_top_of_every_column_it_spans() {
    // Chromium draws the column's top over that column only. A cell paints
    // one border per side, so the spanning cell takes the strongest of the
    // columns' tops rather than none of them.
    let (doc, cells) = layout_markup("", |doc, table| {
        doc.append_element(Some(table), "col", Some("display:table-column"));
        doc.append_element(
            Some(table),
            "col",
            Some("display:table-column;border-top:6px solid"),
        );
        let row = doc.append_element(Some(table), "tr", Some("display:table-row"));
        let cell = doc.append_element(Some(row), "td", Some(CELL));
        doc.set_element_attribute(cell, "colspan", "2".to_string())
            .unwrap();
        let row = doc.append_element(Some(table), "tr", Some("display:table-row"));
        doc.append_element(Some(row), "td", Some(CELL));
        doc.append_element(Some(row), "td", Some(CELL));
    });
    assert_eq!(widths(&doc, cells[0])[0], 6.0);
}

#[test]
fn a_spanning_col_makes_a_line_between_its_columns() {
    // `<col span=2>` stands for two columns, each with the border, as
    // Chromium draws it; a column group has no line inside it.
    let (doc, cells) = layout_markup("", |doc, table| {
        let col = doc.append_element(
            Some(table),
            "col",
            Some("display:table-column;border:4px solid"),
        );
        doc.set_element_attribute(col, "span", "2".to_string())
            .unwrap();
        let row = doc.append_element(Some(table), "tr", Some("display:table-row"));
        doc.append_element(Some(row), "td", Some(CELL));
        doc.append_element(Some(row), "td", Some(CELL));
    });
    assert_eq!(widths(&doc, cells[0]), [4.0, 4.0, 4.0, 4.0]);
    assert_eq!(widths(&doc, cells[1]), [4.0, 4.0, 4.0, 4.0]);
}

#[test]
fn a_collapsed_first_row_hands_the_group_top_to_the_next_row() {
    // With the first row collapsed, the second row lies on the group's top
    // edge and takes its border, as Chromium draws it.
    let (doc, cells) = layout_markup("", |doc, table| {
        let group = doc.append_element(
            Some(table),
            "tbody",
            Some("display:table-row-group;border:4px solid"),
        );
        let row = doc.append_element(
            Some(group),
            "tr",
            Some("display:table-row;visibility:collapse"),
        );
        doc.append_element(Some(row), "td", Some(CELL));
        let row = doc.append_element(Some(group), "tr", Some("display:table-row"));
        doc.append_element(Some(row), "td", Some(CELL));
    });
    assert_eq!(widths(&doc, cells[1]), [4.0, 4.0, 4.0, 4.0]);
}
