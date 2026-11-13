//! Collapsed borders of row groups, columns and column groups (CSS 2.1
//! §17.6.2.1). Each case was checked against Chromium's rendering of the
//! same markup: the cell sides take the winning border, and the parts paint
//! nothing of their own.

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
