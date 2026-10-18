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

/// A table row for [`rows_height`]: its style, then its (style, text) cells.
type Row<'a> = (&'a str, &'a [(&'a str, &'a str)]);

const COLLAPSED_ROW: &str = "visibility:collapse";

/// The height of a collapsed-border table with the given column boxes and
/// `rows`, whose cells are set in 10px Ahem. Fixed layout at `width` hands
/// every column an equal share of what the borders leave, so a cell wraps by
/// how much border the table spends.
fn rows_height(width: u32, columns: &[&'static str], rows: &[Row]) -> f32 {
    let (doc, table, _) = laid_out_rows("table-layout:fixed", width, columns, rows);
    doc.nodes[table].unrounded_layout.size.height
}

/// The table [`rows_height`] measures, laid out with `table_css` besides its
/// `width`, and its cells by row.
fn laid_out_rows(
    table_css: &str,
    width: u32,
    columns: &[&'static str],
    rows: &[Row],
) -> (Document, usize, Vec<Vec<usize>>) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;margin:0"),
    );
    let table = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some(format!(
            "display:table;{table_css};border-collapse:collapse;width:{width}px"
        )),
    );
    for css in columns {
        doc.append_element(Some(table), "div", Style::default(), Some(*css));
    }
    let mut by_row = Vec::new();
    for &(row_css, cells) in rows {
        let row = doc.append_element(
            Some(table),
            "div",
            Style::default(),
            Some(format!("display:table-row;{row_css}")),
        );
        let mut ids = Vec::new();
        for &(css, text) in cells {
            let cell = doc.append_element(
                Some(row),
                "div",
                Style::default(),
                Some(format!(
                    "display:table-cell;padding:0;font-family:Ahem;font-size:10px;line-height:10px;{css}"
                )),
            );
            // A text after `#` is set in a block box inside the cell.
            if let Some(text) = text.strip_prefix('#') {
                let block =
                    doc.append_element(Some(cell), "div", Style::default(), Some("display:block"));
                doc.append_text(block, text);
            } else {
                doc.append_text(cell, text);
            }
            ids.push(cell);
        }
        by_row.push(ids);
    }
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    crate::layout::layout_single_page(&mut doc, &computed, PageBox::A4).unwrap();
    (doc, table, by_row)
}

#[test]
fn a_collapsed_edge_column_keeps_its_border_in_the_widths_rows_are_measured_at() {
    // The collapsed first column's 20px left border is the table's left edge
    // until it collapses, and it narrows the columns the row heights are
    // measured at. The text wraps there as in the table that never collapsed.
    let wraps = "X X X X X X";
    let first: [Row; 2] = [
        ("", &[("border-left:20px solid black", "X"), ("", "X")]),
        ("", &[("", wraps), ("", "X")]),
    ];
    let measured = rows_height(120, &[COLLAPSED_COL, COL], &first);
    assert_eq!(measured, rows_height(120, &[COL, COL], &first));
    assert_eq!(measured, 30.0);
    // The same holds at the other edge.
    let last: [Row; 2] = [
        ("", &[("", "X"), ("border-right:20px solid black", "X")]),
        ("", &[("", "X"), ("", wraps)]),
    ];
    let measured = rows_height(120, &[COL, COLLAPSED_COL], &last);
    assert_eq!(measured, rows_height(120, &[COL, COL], &last));
    assert_eq!(measured, 30.0);
}

#[test]
fn a_collapsed_column_keeps_its_overlaps_in_the_widths_rows_are_measured_at() {
    // Borders that meet inside the table overlap by their shared width. With
    // the middle column collapsed, the cells beside it meet each other; the
    // widths the rows are measured at still count the lines on both sides of
    // the collapsed column.
    let bordered = "border:10px solid black";
    let cells: [Row; 1] = [(
        "",
        &[(bordered, "X"), (bordered, "X"), (bordered, "X X X X X X")],
    )];
    let measured = rows_height(231, &[COL, COLLAPSED_COL, COL], &cells);
    assert_eq!(measured, rows_height(231, &[COL, COL, COL], &cells));
    assert_eq!(measured, 50.0);
}

#[test]
fn a_collapsed_column_is_measured_with_the_borders_it_shares_with_a_neighbour() {
    // The neighbour's wide border is also the collapsed cell's right border
    // while the column is in place, which narrows the text there.
    let rows: [Row; 1] = [(
        "",
        &[("", "X X X X X X"), ("border-left:20px solid black", "X")],
    )];
    let measured = rows_height(100, &[COLLAPSED_COL, COL], &rows);
    assert_eq!(measured, rows_height(100, &[COL, COL], &rows));
    assert_eq!(measured, 20.0);
}

/// A wrapping cell and a collapsed cell with a wide border on the side that
/// faces it.
const WRAPS: &str = "X X X X X X";

#[test]
fn a_column_after_a_collapsed_one_is_measured_with_the_borders_it_had_beside_it() {
    // The collapsed cell's wide right border is also the neighbour's left
    // border while the column is in place, and narrows the text there. The
    // row keeps the height it had.
    let rows: [Row; 1] = [(
        "",
        &[
            ("", "X"),
            ("border-right:20px solid black", "X"),
            ("", WRAPS),
        ],
    )];
    let measured = rows_height(80, &[COL, COLLAPSED_COL, COL], &rows);
    assert_eq!(measured, rows_height(80, &[COL, COL, COL], &rows));
    assert_eq!(measured, 60.0);
}

#[test]
fn a_column_before_a_collapsed_one_is_measured_with_the_borders_it_had_beside_it() {
    let rows: [Row; 1] = [(
        "",
        &[
            ("", WRAPS),
            ("border-left:20px solid black", "X"),
            ("", "X"),
        ],
    )];
    let measured = rows_height(80, &[COL, COLLAPSED_COL, COL], &rows);
    assert_eq!(measured, rows_height(80, &[COL, COL, COL], &rows));
    assert_eq!(measured, 60.0);
}

#[test]
fn an_automatic_table_sizes_the_columns_in_place_with_the_borders_they_had() {
    // The wide border adds to the next cell's max-content width while the
    // column is in place, so it changes how the columns the rows are measured
    // at share the table.
    let rows: [Row; 1] = [(
        "",
        &[
            ("", "X"),
            ("border-right:20px solid black", "X"),
            ("", WRAPS),
        ],
    )];
    let height = |columns: &[&'static str]| {
        let (doc, table, _) = laid_out_rows("table-layout:auto", 100, columns, &rows);
        doc.nodes[table].unrounded_layout.size.height
    };
    let measured = height(&[COL, COLLAPSED_COL, COL]);
    assert_eq!(measured, height(&[COL, COL, COL]));
    assert_eq!(measured, 10.0);
}

#[test]
fn a_block_cell_beside_a_collapsed_column_is_sized_with_the_borders_it_keeps() {
    // A cell holding a block box keeps the intrinsic widths it was measured
    // with, which must not be the ones measured with the borders it had while
    // the column was in place: the table is as wide as one without the column.
    let with: [Row; 1] = [(
        "",
        &[
            ("", "#X X"),
            ("border-right:20px solid black", "#X"),
            ("", "#X X X X X X"),
        ],
    )];
    let without: [Row; 1] = [("", &[("", "#X X"), ("", "#X X X X X X")])];
    let (collapsed, table, cells) =
        laid_out_rows("table-layout:auto", 100, &[COL, COLLAPSED_COL, COL], &with);
    let (reference, ref_table, ref_cells) =
        laid_out_rows("table-layout:auto", 100, &[COL, COL], &without);
    let widths = |doc: &Document, cells: &[usize]| -> Vec<f32> {
        cells
            .iter()
            .map(|&cell| doc.nodes[cell].unrounded_layout.size.width)
            .collect()
    };
    assert_eq!(
        widths(&collapsed, &[cells[0][0], cells[0][2]]),
        widths(&reference, &ref_cells[0]),
    );
    assert_eq!(
        collapsed.nodes[table].unrounded_layout.size.width,
        reference.nodes[ref_table].unrounded_layout.size.width,
    );
}

#[test]
fn a_cell_beside_a_collapsed_column_breaks_its_lines_at_the_borders_it_keeps() {
    // The cell is as wide when the rows are measured as when it is laid out,
    // but its left border is not: the lines of the measure were broken at the
    // border it had beside the collapsed column, and are not kept.
    let rows: [Row; 1] = [(
        "",
        &[
            ("", "X"),
            ("border-right:20px solid black", "X"),
            ("", WRAPS),
        ],
    )];
    let columns = [
        "display:table-column;width:40px",
        "display:table-column;visibility:collapse;width:40px",
        "display:table-column;width:40px",
    ];
    let (doc, _, cells) = laid_out_rows("table-layout:fixed", 120, &columns, &rows);
    let cell = cells[0][2];
    let lines = doc.nodes[cell]
        .ifc
        .as_ref()
        .and_then(|root| root.lines.as_ref())
        .expect("the cell breaks its text into lines");
    assert_eq!(doc.nodes[cell].unrounded_layout.size.width, 40.0);
    assert_eq!(lines.width, 40.0);
}

#[test]
fn a_border_only_a_collapsed_column_gives_the_rows_is_not_counted_twice() {
    // While the column is in place its cell's bottom border is the line
    // between the rows, and the cell below takes it as its top border. The
    // rows that remain overlap no such line, so measuring both cells with it
    // would leave the table a border taller than before the collapse.
    let rows: [Row; 2] = [
        (
            "",
            &[
                ("", "X"),
                ("border-bottom:20px solid black", "X"),
                ("", "X"),
            ],
        ),
        ("", &[("", "X"), ("", "X"), ("", "X")]),
    ];
    let measured = rows_height(120, &[COL, COLLAPSED_COL, COL], &rows);
    assert_eq!(measured, rows_height(120, &[COL, COL, COL], &rows));
    assert_eq!(measured, 40.0);
}

#[test]
fn a_collapsed_row_stays_out_of_the_borders_measured_with_the_columns_in_place() {
    // The collapsed row's wide border never takes part in resolution, so it
    // does not narrow the widths the other rows are measured at.
    let kept: [Row; 2] = [
        ("", &[("border-left:20px solid black", "X"), ("", "X")]),
        ("", &[("", "X X X X X X"), ("", "X")]),
    ];
    let with_collapsed: [Row; 3] = [
        kept[0],
        kept[1],
        (
            COLLAPSED_ROW,
            &[("border-left:60px solid black", "X"), ("", "X")],
        ),
    ];
    let measured = rows_height(90, &[COLLAPSED_COL, COL], &with_collapsed);
    assert_eq!(measured, rows_height(90, &[COLLAPSED_COL, COL], &kept));
    assert_eq!(measured, 30.0);
}

#[test]
fn a_table_whose_rows_and_columns_all_collapse_has_no_height() {
    let only: [Row; 1] = [(
        COLLAPSED_ROW,
        &[("border-left:20px solid black", "X"), ("", "X")],
    )];
    assert_eq!(rows_height(120, &[COLLAPSED_COL, COL], &only), 0.0);
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
