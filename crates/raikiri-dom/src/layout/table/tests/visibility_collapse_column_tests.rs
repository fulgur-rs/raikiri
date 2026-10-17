use super::*;

/// A column box under the table: a `<col>`, a `<colgroup>` holding `<col>`
/// children, or a `<colgroup>` of its own `span` columns.
enum Column {
    Col(&'static str, Option<u32>),
    Group(&'static str, Vec<&'static str>),
    EmptyGroup(&'static str, Option<u32>),
}

struct ColumnFixture {
    doc: Document,
    table: usize,
    /// Cells by row, then by column.
    cells: Vec<Vec<usize>>,
    /// The cell of the extra last row that spans every column.
    spanning: Option<usize>,
}

const SEPARATE: &str = "display:table;border-spacing:10px";
const COLLAPSING: &str = "display:table;border-collapse:collapse";
const COL: &str = "display:table-column";
const COLLAPSED_COL: &str = "display:table-column;visibility:collapse";
const COLLAPSED_GROUP: &str = "display:table-column-group;visibility:collapse";
const GROUP: &str = "display:table-column-group";

fn set_attribute(doc: &mut Document, node: usize, name: &str, value: impl ToString) {
    doc.set_element_attributes(node, vec![(name.into(), value.to_string().into())]);
}

/// A table with `columns` as its column boxes and `rows` rows of one 20px
/// tall cell per entry of `widths`. With `spanning` set, a last row holds a
/// single cell of that authored width spanning every column. `cell_css` is
/// appended to each cell's style.
fn table_with(
    table_css: &str,
    columns: &[Column],
    widths: &[f32],
    rows: usize,
    spanning: Option<f32>,
    cell_css: &str,
) -> ColumnFixture {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;margin:0"),
    );
    let table = doc.append_element(Some(body), "div", Style::default(), Some(table_css));
    for column in columns {
        match column {
            Column::Col(css, span) => {
                let col = doc.append_element(Some(table), "div", Style::default(), Some(*css));
                if let Some(span) = span {
                    set_attribute(&mut doc, col, "span", span);
                }
            }
            Column::Group(css, children) => {
                let group = doc.append_element(Some(table), "div", Style::default(), Some(*css));
                for child in children {
                    doc.append_element(Some(group), "div", Style::default(), Some(*child));
                }
            }
            Column::EmptyGroup(css, span) => {
                let group = doc.append_element(Some(table), "div", Style::default(), Some(*css));
                if let Some(span) = span {
                    set_attribute(&mut doc, group, "span", span);
                }
            }
        }
    }
    let mut cells = Vec::new();
    for _ in 0..rows {
        let row = doc.append_element(
            Some(table),
            "div",
            Style::default(),
            Some("display:table-row"),
        );
        cells.push(
            widths
                .iter()
                .map(|width| {
                    doc.append_element(
                        Some(row),
                        "div",
                        Style::default(),
                        Some(format!(
                            "display:table-cell;width:{width}px;height:20px;padding:0;box-sizing:border-box;{cell_css}"
                        )),
                    )
                })
                .collect(),
        );
    }
    let spanning = spanning.map(|width| {
        let row = doc.append_element(Some(table), "div", Style::default(), Some("display:table-row"));
        let cell = doc.append_element(
            Some(row),
            "div",
            Style::default(),
            Some(format!(
                "display:table-cell;width:{width}px;height:20px;padding:0;box-sizing:border-box;{cell_css}"
            )),
        );
        set_attribute(&mut doc, cell, "colspan", widths.len());
        cell
    });
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    crate::layout::layout_single_page(&mut doc, &computed, PageBox::A4).unwrap();
    ColumnFixture {
        doc,
        table,
        cells,
        spanning,
    }
}

fn three_columns(table_css: &str, middle: &'static str) -> ColumnFixture {
    table_with(
        table_css,
        &[
            Column::Col(COL, None),
            Column::Col(middle, None),
            Column::Col(COL, None),
        ],
        &[30.0, 40.0, 50.0],
        1,
        None,
        "",
    )
}

/// The cell's left edge and width.
fn column_box(doc: &Document, cell: usize) -> (f32, f32) {
    let layout = doc.nodes[cell].unrounded_layout;
    (layout.location.x, layout.size.width)
}

fn table_size(fixture: &ColumnFixture) -> (f32, f32) {
    let layout = fixture.doc.nodes[fixture.table].unrounded_layout;
    (layout.size.width, layout.size.height)
}

#[test]
fn a_collapsed_column_loses_its_width_and_spacing() {
    let fixture = three_columns(SEPARATE, COLLAPSED_COL);
    // 10px spacing around the two columns that remain, none for the third.
    assert_eq!(table_size(&fixture), (110.0, 40.0));
    let cells = &fixture.cells[0];
    assert_eq!(column_box(&fixture.doc, cells[0]), (10.0, 30.0));
    assert_eq!(column_box(&fixture.doc, cells[1]).1, 0.0);
    assert_eq!(column_box(&fixture.doc, cells[2]), (50.0, 50.0));
}

#[test]
fn collapsing_the_first_column_leaves_one_leading_gap() {
    let fixture = table_with(
        SEPARATE,
        &[
            Column::Col(COLLAPSED_COL, None),
            Column::Col(COL, None),
            Column::Col(COL, None),
        ],
        &[30.0, 40.0, 50.0],
        1,
        None,
        "",
    );
    assert_eq!(table_size(&fixture).0, 120.0);
    let cells = &fixture.cells[0];
    assert_eq!(column_box(&fixture.doc, cells[1]), (10.0, 40.0));
    assert_eq!(column_box(&fixture.doc, cells[2]), (60.0, 50.0));
}

#[test]
fn collapsing_the_last_column_leaves_one_trailing_gap() {
    let fixture = table_with(
        SEPARATE,
        &[
            Column::Col(COL, None),
            Column::Col(COL, None),
            Column::Col(COLLAPSED_COL, None),
        ],
        &[30.0, 40.0, 50.0],
        1,
        None,
        "",
    );
    assert_eq!(table_size(&fixture).0, 100.0);
}

#[test]
fn a_column_group_collapses_its_columns() {
    let fixture = table_with(
        SEPARATE,
        &[
            Column::Group(COLLAPSED_GROUP, vec![COL, COL]),
            Column::Col(COL, None),
        ],
        &[30.0, 40.0, 50.0],
        1,
        None,
        "",
    );
    assert_eq!(table_size(&fixture).0, 70.0);
    assert_eq!(column_box(&fixture.doc, fixture.cells[0][2]), (10.0, 50.0));
}

#[test]
fn a_column_group_without_columns_stands_for_its_span() {
    let fixture = table_with(
        SEPARATE,
        &[
            Column::EmptyGroup(COLLAPSED_GROUP, Some(2)),
            Column::Group(GROUP, vec![COL]),
        ],
        &[30.0, 40.0, 50.0],
        1,
        None,
        "",
    );
    assert_eq!(table_size(&fixture).0, 70.0);
    // One empty group is one column.
    let single = table_with(
        SEPARATE,
        &[Column::EmptyGroup(COLLAPSED_GROUP, None)],
        &[30.0, 40.0],
        1,
        None,
        "",
    );
    assert_eq!(table_size(&single).0, 60.0);
}

#[test]
fn a_table_whose_columns_all_collapse_keeps_no_spacing_and_its_height() {
    let fixture = table_with(
        SEPARATE,
        &[Column::Group(COLLAPSED_GROUP, vec![COL, COL])],
        &[30.0, 40.0],
        1,
        None,
        "",
    );
    assert_eq!(table_size(&fixture), (0.0, 40.0));
}

#[test]
fn a_cell_spanning_a_collapsed_column_shrinks_and_clips_without_widening_the_rest() {
    let fixture = table_with(
        SEPARATE,
        &[
            Column::Col(COL, None),
            Column::Col(COLLAPSED_COL, None),
            Column::Col(COL, None),
        ],
        &[30.0, 40.0, 50.0],
        1,
        Some(300.0),
        "",
    );
    let doc = &fixture.doc;
    let span = fixture.spanning.unwrap();
    // The remaining columns and the gap between them.
    assert_eq!(column_box(doc, span), (10.0, 30.0 + 10.0 + 50.0));
    assert!(doc.nodes[span].clips_collapsed_table_span);
    assert_eq!(table_size(&fixture).0, 110.0);
    // Cells that span nothing collapsed do not clip.
    assert!(!doc.nodes[fixture.cells[0][0]].clips_collapsed_table_span);
}

#[test]
fn cells_in_a_collapsed_column_have_no_extent_and_clip() {
    let fixture = table_with(
        SEPARATE,
        &[
            Column::Col(COL, None),
            Column::Col(COLLAPSED_COL, None),
            Column::Col(COL, None),
        ],
        &[30.0, 40.0, 50.0],
        1,
        None,
        "padding:0 5px;border:3px solid black",
    );
    let doc = &fixture.doc;
    let cell = doc.nodes[fixture.cells[0][1]].unrounded_layout;
    assert_eq!(cell.size.width, 0.0);
    assert_eq!(cell.padding.left + cell.padding.right, 0.0);
    assert_eq!(cell.border.left + cell.border.right, 0.0);
    assert!(doc.nodes[fixture.cells[0][1]].clips_collapsed_table_span);
}

#[test]
fn a_collapsed_column_leaves_the_row_height_it_gave() {
    // The cell of the collapsed column would wrap onto many lines at no
    // width; the row keeps the single line it measured before the collapse.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;margin:0"),
    );
    let table = doc.append_element(Some(body), "div", Style::default(), Some(SEPARATE));
    doc.append_element(Some(table), "div", Style::default(), Some(COLLAPSED_COL));
    doc.append_element(Some(table), "div", Style::default(), Some(COL));
    let row = doc.append_element(
        Some(table),
        "div",
        Style::default(),
        Some("display:table-row"),
    );
    for text in ["XXXX XXXX XXXX", "X"] {
        let cell = doc.append_element(
            Some(row),
            "div",
            Style::default(),
            Some("display:table-cell;padding:0;font-family:Ahem;font-size:20px;line-height:20px"),
        );
        doc.append_text(cell, text);
    }
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    crate::layout::layout_single_page(&mut doc, &computed, PageBox::A4).unwrap();
    let layout = doc.nodes[table].unrounded_layout;
    assert_eq!(layout.size.height, 40.0);
}

#[test]
fn a_collapsed_column_lays_out_like_a_table_without_it() {
    // Both border models: the table is as wide as one that never had the
    // column, and the columns that remain sit where they would there.
    for table_css in [SEPARATE, COLLAPSING] {
        for cell_css in ["", "border:2px solid black"] {
            let collapsed = table_with(
                table_css,
                &[
                    Column::Col(COL, None),
                    Column::Col(COLLAPSED_COL, None),
                    Column::Col(COL, None),
                ],
                &[30.0, 40.0, 50.0],
                2,
                None,
                cell_css,
            );
            let without = table_with(
                table_css,
                &[Column::Col(COL, None), Column::Col(COL, None)],
                &[30.0, 50.0],
                2,
                None,
                cell_css,
            );
            assert_eq!(
                table_size(&collapsed),
                table_size(&without),
                "{table_css} {cell_css}"
            );
            for row in 0..2 {
                assert_eq!(
                    column_box(&collapsed.doc, collapsed.cells[row][0]),
                    column_box(&without.doc, without.cells[row][0]),
                    "{table_css} {cell_css}"
                );
                assert_eq!(
                    column_box(&collapsed.doc, collapsed.cells[row][2]),
                    column_box(&without.doc, without.cells[row][1]),
                    "{table_css} {cell_css}"
                );
            }
        }
    }
}

#[test]
fn collapsed_columns_resolve_borders_against_the_table_edge() {
    // The first column's wide border would otherwise decide the table's left
    // edge; once it collapses the column after it meets the table border.
    let fixture = table_with(
        COLLAPSING,
        &[Column::Col(COLLAPSED_COL, None), Column::Col(COL, None)],
        &[30.0, 50.0],
        1,
        None,
        "",
    );
    let wide = fixture.doc.nodes[fixture.cells[0][0]].unrounded_layout;
    assert_eq!(wide.size.width, 0.0);
    let reference = three_columns(COLLAPSING, COL);
    assert!(table_size(&fixture).0 < table_size(&reference).0);
}
