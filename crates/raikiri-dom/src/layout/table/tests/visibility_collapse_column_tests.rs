use super::*;

/// A column box under the table: a `<col>`, a `<colgroup>` holding `<col>`
/// children, or a `<colgroup>` of its own `span` columns.
enum Column {
    Col(&'static str, Option<u32>),
    Group(&'static str, Vec<&'static str>),
    EmptyGroup(&'static str, Option<u32>),
    /// A `<colgroup span>` that also holds `<col>` children.
    SpannedGroup(&'static str, u32, Vec<&'static str>),
    /// A comment, which is no node of the document the table sees.
    Comment,
    /// A `display: contents` element that is never rendered, whatever its
    /// style says (`<script>`), holding column boxes.
    NeverRendered(Vec<Column>),
    /// A `display: contents` element whose children are column boxes.
    Contents(Vec<Column>),
    /// A `<colgroup>` (with an optional `span`) holding a
    /// `display: contents` element that holds `<col>` children.
    GroupWithContents(&'static str, Option<u32>, Vec<&'static str>),
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
const FIXED: &str = "display:table;table-layout:fixed;width:200px;border-spacing:10px";
const MIN_WIDTH: &str = "display:table;min-width:300px;border-spacing:10px";
const COL: &str = "display:table-column";
const COLLAPSED_COL: &str = "display:table-column;visibility:collapse";
const COLLAPSED_GROUP: &str = "display:table-column-group;visibility:collapse";
const GROUP: &str = "display:table-column-group";

fn set_attribute(doc: &mut Document, node: usize, name: &str, value: impl ToString) {
    doc.set_element_attributes(node, vec![(name.into(), value.to_string().into())]);
}

/// Append `column` under `parent`.
fn append_column(doc: &mut Document, parent: usize, column: &Column) {
    match column {
        Column::Col(css, span) => {
            let col = doc.append_element(Some(parent), "div", Style::default(), Some(*css));
            if let Some(span) = span {
                set_attribute(doc, col, "span", span);
            }
        }
        Column::Group(css, children) => {
            let group = doc.append_element(Some(parent), "div", Style::default(), Some(*css));
            for child in children {
                doc.append_element(Some(group), "div", Style::default(), Some(*child));
            }
        }
        Column::SpannedGroup(css, span, children) => {
            let group = doc.append_element(Some(parent), "div", Style::default(), Some(*css));
            set_attribute(doc, group, "span", span);
            for child in children {
                doc.append_element(Some(group), "div", Style::default(), Some(*child));
            }
        }
        Column::EmptyGroup(css, span) => {
            let group = doc.append_element(Some(parent), "div", Style::default(), Some(*css));
            if let Some(span) = span {
                set_attribute(doc, group, "span", span);
            }
        }
        Column::Comment => {
            doc.append_comment(Some(parent), "between columns");
        }
        Column::NeverRendered(children) => {
            let wrapper = doc.append_element(
                Some(parent),
                "script",
                Style::default(),
                Some("display:contents"),
            );
            for child in children {
                append_column(doc, wrapper, child);
            }
        }
        Column::Contents(children) => {
            let wrapper = doc.append_element(
                Some(parent),
                "div",
                Style::default(),
                Some("display:contents"),
            );
            for child in children {
                append_column(doc, wrapper, child);
            }
        }
        Column::GroupWithContents(css, span, children) => {
            let group = doc.append_element(Some(parent), "div", Style::default(), Some(*css));
            if let Some(span) = span {
                set_attribute(doc, group, "span", span);
            }
            let wrapper = doc.append_element(
                Some(group),
                "div",
                Style::default(),
                Some("display:contents"),
            );
            for child in children {
                doc.append_element(Some(wrapper), "div", Style::default(), Some(*child));
            }
        }
    }
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
        append_column(&mut doc, table, column);
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
fn a_column_group_ignores_children_that_are_not_columns() {
    let fixture = table_with(
        SEPARATE,
        &[Column::Group(GROUP, vec![COL, "display:block"])],
        &[30.0, 40.0],
        1,
        None,
        "",
    );
    // The stray child is no column, so only the first column has a box.
    assert_eq!(table_size(&fixture).0, 10.0 + 30.0 + 10.0 + 40.0 + 10.0);
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
    for table_css in [SEPARATE, COLLAPSING, FIXED, MIN_WIDTH] {
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

#[test]
fn a_column_group_without_columns_adds_its_span_in_any_table() {
    // Without a collapse, `<colgroup span=3>` still stands for three columns:
    // two empty ones follow the cell's column, each with its own spacing.
    let with_group = table_with(
        SEPARATE,
        &[Column::EmptyGroup(GROUP, Some(3))],
        &[30.0],
        1,
        None,
        "",
    );
    let without = table_with(SEPARATE, &[], &[30.0], 1, None, "");
    assert_eq!(table_size(&without).0, 50.0);
    assert_eq!(table_size(&with_group).0, 70.0);
}

#[test]
fn the_span_of_a_group_with_columns_is_ignored_even_when_a_column_has_no_box() {
    let hidden = "display:none";
    let fixture = table_with(
        SEPARATE,
        &[Column::SpannedGroup(GROUP, 3, vec![hidden])],
        &[30.0],
        1,
        None,
        "",
    );
    assert_eq!(table_size(&fixture).0, 50.0);
}

#[test]
fn a_table_whose_columns_all_collapse_shows_only_its_own_border() {
    let table_css = "display:table;border-collapse:collapse;border:4px solid black";
    let fixture = table_with(
        table_css,
        &[Column::Group(COLLAPSED_GROUP, vec![COL, COL])],
        &[30.0, 40.0],
        1,
        None,
        "border:10px solid black",
    );
    // No cell shows, so no cell border meets the table's.
    assert_eq!(table_size(&fixture).0, 8.0);
}

#[test]
fn a_vertical_table_keeps_its_collapsed_columns() {
    let fixture = table_with(
        "display:table;writing-mode:vertical-rl;border-spacing:10px",
        &[Column::Col(COL, None), Column::Col(COLLAPSED_COL, None)],
        &[30.0, 40.0],
        1,
        None,
        "",
    );
    let layout = fixture.doc.nodes[fixture.cells[0][1]].unrounded_layout;
    assert!(layout.size.width > 0.0 && layout.size.height > 0.0);
    assert!(!fixture.doc.nodes[fixture.cells[0][1]].clips_collapsed_table_span);
}

#[test]
fn a_column_group_without_columns_sizes_the_columns_it_stands_for() {
    // In a fixed table the two columns of `<colgroup span=2 width=50px>` take
    // 50px each, not the 10px of their cells.
    let fixture = table_with(
        FIXED,
        &[Column::EmptyGroup(
            "display:table-column-group;width:50px",
            Some(2),
        )],
        &[10.0, 10.0, 10.0],
        1,
        None,
        "",
    );
    let cells = &fixture.cells[0];
    assert_eq!(column_box(&fixture.doc, cells[0]), (10.0, 50.0));
    assert_eq!(column_box(&fixture.doc, cells[1]), (70.0, 50.0));
}

#[test]
fn a_collapsed_column_under_display_contents_collapses() {
    let fixture = table_with(
        SEPARATE,
        &[
            Column::Col(COL, None),
            Column::Contents(vec![Column::Col(COLLAPSED_COL, None)]),
            Column::Col(COL, None),
        ],
        &[30.0, 40.0, 50.0],
        1,
        None,
        "",
    );
    // Same as the middle `<col>` collapsing as a direct child of the table.
    assert_eq!(table_size(&fixture), (110.0, 40.0));
    let cells = &fixture.cells[0];
    assert_eq!(column_box(&fixture.doc, cells[0]), (10.0, 30.0));
    assert_eq!(column_box(&fixture.doc, cells[1]).1, 0.0);
    assert_eq!(column_box(&fixture.doc, cells[2]), (50.0, 50.0));
}

#[test]
fn a_column_group_under_display_contents_collapses_its_columns() {
    let fixture = table_with(
        SEPARATE,
        &[
            Column::Contents(vec![Column::Group(COLLAPSED_GROUP, vec![COL, COL])]),
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
fn a_column_under_display_contents_in_a_group_collapses() {
    let fixture = table_with(
        SEPARATE,
        &[
            Column::GroupWithContents(COLLAPSED_GROUP, None, vec![COL, COL]),
            Column::Col(COL, None),
        ],
        &[30.0, 40.0, 50.0],
        1,
        None,
        "",
    );
    assert_eq!(table_size(&fixture).0, 70.0);
}

#[test]
fn column_widths_under_display_contents_size_a_fixed_table() {
    // The `<col>` widths reach the fixed table layout through the wrapper:
    // 50px and 70px, not an even split of 200px.
    let fixture = table_with(
        FIXED,
        &[Column::Contents(vec![
            Column::Col("display:table-column;width:50px", None),
            Column::Col("display:table-column;width:70px", None),
        ])],
        &[10.0, 10.0],
        1,
        None,
        "",
    );
    let cells = &fixture.cells[0];
    assert_eq!(column_box(&fixture.doc, cells[0]).1, 50.0);
    assert_eq!(column_box(&fixture.doc, cells[1]).1, 70.0);
}

#[test]
fn nested_display_contents_keep_the_grid_order_of_columns() {
    // The collapsed column is the second one in grid order, behind a
    // `display: contents` that itself sits behind another.
    let fixture = table_with(
        SEPARATE,
        &[
            Column::Col(COL, None),
            Column::Contents(vec![Column::Contents(vec![
                Column::Col(COLLAPSED_COL, None),
                Column::Col(COL, None),
            ])]),
        ],
        &[30.0, 40.0, 50.0],
        1,
        None,
        "",
    );
    assert_eq!(table_size(&fixture).0, 110.0);
    assert_eq!(column_box(&fixture.doc, fixture.cells[0][2]), (50.0, 50.0));
}

#[test]
fn a_column_group_holding_only_an_empty_display_contents_stands_for_its_span() {
    // The wrapper makes no box and holds no column, so the group still
    // stands for its own `span` columns, collapsed here.
    let fixture = table_with(
        SEPARATE,
        &[
            Column::GroupWithContents(COLLAPSED_GROUP, Some(2), vec![]),
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
fn a_comment_among_the_columns_makes_no_column() {
    let fixture = table_with(
        SEPARATE,
        &[
            Column::Col(COL, None),
            Column::Comment,
            Column::Contents(vec![Column::Comment, Column::Col(COLLAPSED_COL, None)]),
            Column::Col(COL, None),
        ],
        &[30.0, 40.0, 50.0],
        1,
        None,
        "",
    );
    assert_eq!(table_size(&fixture), (110.0, 40.0));
    assert_eq!(column_box(&fixture.doc, fixture.cells[0][2]), (50.0, 50.0));
}

#[test]
fn columns_under_an_element_that_is_never_rendered_make_no_column() {
    // The `<script>` is `display: contents` by author CSS, but it and its
    // subtree stay out of the table, so its collapsed column is no column.
    let fixture = table_with(
        SEPARATE,
        &[
            Column::Col(COL, None),
            Column::NeverRendered(vec![Column::Col(COLLAPSED_COL, None)]),
            Column::Col(COL, None),
        ],
        &[30.0, 40.0, 50.0],
        1,
        None,
        "",
    );
    assert_eq!(table_size(&fixture).0, 160.0);
    assert_eq!(column_box(&fixture.doc, fixture.cells[0][1]), (50.0, 40.0));
}
