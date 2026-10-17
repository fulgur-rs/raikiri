use super::*;

/// A laid-out table with its row ids and its cell ids in document order.
struct Fixture {
    doc: Document,
    rows: Vec<usize>,
    cells: Vec<usize>,
}

/// A table whose rows take `rows` styles, each holding one 20px-tall cell.
/// With `span` set, the first row also holds a cell spanning that many rows.
fn fixture(table_css: &str, group_css: &str, rows: &[&str], span: Option<u32>) -> Fixture {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;margin:0"),
    );
    let table = doc.append_element(Some(body), "div", Style::default(), Some(table_css));
    let group = doc.append_element(Some(table), "div", Style::default(), Some(group_css));
    let mut row_ids = Vec::new();
    let mut cells = Vec::new();
    for (index, row_css) in rows.iter().enumerate() {
        let row = doc.append_element(Some(group), "div", Style::default(), Some(*row_css));
        row_ids.push(row);
        cells.push(doc.append_element(
            Some(row),
            "div",
            Style::default(),
            Some("display:table-cell;width:20px;height:20px;padding:0"),
        ));
        if index == 0
            && let Some(span) = span
        {
            let cell = doc.append_element(
                Some(row),
                "div",
                Style::default(),
                Some("display:table-cell;width:20px;padding:0"),
            );
            doc.set_element_attributes(cell, vec![("rowspan".into(), span.to_string().into())]);
            cells.push(cell);
        }
    }
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    crate::layout::layout_single_page(&mut doc, &computed, PageBox::A4).unwrap();
    Fixture {
        doc,
        rows: row_ids,
        cells,
    }
}

fn cell_box(doc: &Document, cell: usize) -> (f32, f32) {
    let layout = doc.nodes[cell].unrounded_layout;
    (layout.location.y, layout.size.height)
}

const ROW: &str = "display:table-row";
const COLLAPSED: &str = "display:table-row;visibility:collapse";
const SEPARATE: &str = "display:table;border-spacing:10px";
const GROUP: &str = "display:table-row-group";

#[test]
fn a_collapsed_row_loses_its_height_and_spacing() {
    let fixture = fixture(SEPARATE, GROUP, &[COLLAPSED, ROW], None);
    let doc = &fixture.doc;
    assert_eq!(cell_box(doc, fixture.cells[0]).1, 0.0);
    // The remaining row starts one spacing below the grid edge.
    assert_eq!(cell_box(doc, fixture.cells[1]), (10.0, 20.0));
    assert!(!doc.nodes[fixture.cells[1]].clips_collapsed_table_span);
}

#[test]
fn a_collapsed_row_group_collapses_its_rows() {
    let fixture = fixture(
        SEPARATE,
        "display:table-row-group;visibility:collapse",
        &["display:table-row;visibility:visible", ROW],
        None,
    );
    let doc = &fixture.doc;
    assert_eq!(cell_box(doc, fixture.cells[0]).1, 0.0);
    assert_eq!(cell_box(doc, fixture.cells[1]).1, 0.0);
    assert!(!doc.nodes[fixture.rows[0]].collapses_table_rows);
}

#[test]
fn a_cell_spanning_a_collapsed_row_shrinks_to_the_remaining_rows_and_clips() {
    // Rows 0 and 2 remain; the cell keeps both and the one gap left between
    // them.
    let fixture = fixture(SEPARATE, GROUP, &[ROW, COLLAPSED, ROW], Some(3));
    let doc = &fixture.doc;
    let span = fixture.cells[1];
    assert_eq!(cell_box(doc, span), (10.0, 50.0));
    assert!(doc.nodes[span].clips_collapsed_table_span);
    assert_eq!(cell_box(doc, fixture.cells[3]), (40.0, 20.0));
}

#[test]
fn a_cell_starting_in_a_collapsed_row_moves_to_the_first_remaining_row() {
    let fixture = fixture(SEPARATE, GROUP, &[COLLAPSED, ROW], Some(2));
    let doc = &fixture.doc;
    let span = fixture.cells[1];
    assert_eq!(cell_box(doc, span), (10.0, 20.0));
    assert!(doc.nodes[span].clips_collapsed_table_span);
}

#[test]
fn a_cell_spanning_only_collapsed_rows_has_no_block_extent() {
    let fixture = fixture(SEPARATE, GROUP, &[COLLAPSED, COLLAPSED, ROW], Some(2));
    let doc = &fixture.doc;
    let span = fixture.cells[1];
    let layout = doc.nodes[span].unrounded_layout;
    assert_eq!(layout.size.height, 0.0);
    assert_eq!(layout.padding.top + layout.padding.bottom, 0.0);
    assert!(doc.nodes[span].clips_collapsed_table_span);
    assert_eq!(cell_box(doc, fixture.cells[3]), (10.0, 20.0));
}

/// A collapsing-border table with one 20px cell per `(row, cell)` style
/// pair. Returns the document, the table and the cells in row order.
fn collapsing_table(rows: &[(&str, &str)]) -> (Document, usize, Vec<usize>) {
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
        Some("display:table;border-collapse:collapse"),
    );
    let cells = rows
        .iter()
        .map(|(row_css, cell_css)| {
            let row = doc.append_element(Some(table), "div", Style::default(), Some(*row_css));
            doc.append_element(
                Some(row),
                "div",
                Style::default(),
                Some(format!(
                    "display:table-cell;width:20px;height:20px;padding:0;box-sizing:border-box;{cell_css}"
                )),
            )
        })
        .collect();
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    crate::layout::layout_single_page(&mut doc, &computed, PageBox::A4).unwrap();
    (doc, table, cells)
}

const BORDERED: &str = "border:2px solid black";

#[test]
fn collapsed_rows_join_the_lines_on_either_side_in_the_collapsing_model() {
    // Adjoining rows overlap by the 2px line between them; a run of
    // collapsed rows leaves one line between the rows around it.
    let (doc, _, cells) = collapsing_table(&[
        (ROW, BORDERED),
        (COLLAPSED, BORDERED),
        (COLLAPSED, BORDERED),
        (ROW, BORDERED),
    ]);
    assert_eq!(cell_box(&doc, cells[1]).1, 0.0);
    assert_eq!(
        cell_box(&doc, cells[3]).0 - cell_box(&doc, cells[0]).0,
        18.0
    );
}

#[test]
fn the_joined_line_is_the_narrowest_line_in_the_collapsed_run() {
    // The last row has no top border, so it shares no line with the first.
    let (doc, _, cells) = collapsing_table(&[
        (ROW, "border-bottom:4px solid black"),
        (COLLAPSED, "border-top:4px solid black"),
        (ROW, ""),
    ]);
    assert_eq!(
        cell_box(&doc, cells[2]).0 - cell_box(&doc, cells[0]).0,
        20.0
    );
}

#[test]
fn collapsed_rows_at_the_end_leave_the_last_remaining_row_whole() {
    let (doc, table, cells) = collapsing_table(&[
        (ROW, BORDERED),
        (COLLAPSED, BORDERED),
        (COLLAPSED, BORDERED),
    ]);
    assert_eq!(cell_box(&doc, cells[2]).1, 0.0);
    // The table is as tall as one holding only the first row.
    let (single, single_table, _) = collapsing_table(&[(ROW, BORDERED)]);
    assert_eq!(
        doc.nodes[table].unrounded_layout.size.height,
        single.nodes[single_table].unrounded_layout.size.height
    );
}

#[test]
fn an_anonymous_row_collapses_with_its_row_group() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;margin:0"),
    );
    let table = doc.append_element(Some(body), "div", Style::default(), Some(SEPARATE));
    let mut cells = Vec::new();
    for group_css in [
        "display:table-row-group;visibility:collapse",
        "display:table-row-group",
    ] {
        let group = doc.append_element(Some(table), "div", Style::default(), Some(group_css));
        cells.push(doc.append_element(
            Some(group),
            "div",
            Style::default(),
            Some("display:table-cell;width:20px;height:20px;padding:0"),
        ));
    }
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    crate::layout::layout_single_page(&mut doc, &computed, PageBox::A4).unwrap();
    assert_eq!(cell_box(&doc, cells[0]).1, 0.0);
    // Only the second group's row and its two spacings remain.
    assert_eq!(doc.nodes[table].unrounded_layout.size.height, 40.0);
}

#[test]
fn the_first_remaining_row_carries_the_table_baseline() {
    let mut doc = Document::new();
    crate::layout::test_support::with_ahem(&mut doc);
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;margin:0"),
    );
    let table = doc.append_element(Some(body), "div", Style::default(), Some(SEPARATE));
    for row_css in [COLLAPSED, ROW] {
        let row = doc.append_element(Some(table), "div", Style::default(), Some(row_css));
        let cell = doc.append_element(
            Some(row),
            "div",
            Style::default(),
            Some("display:table-cell;padding:0;font-family:Ahem;font-size:20px;line-height:20px"),
        );
        doc.append_text(cell, "X");
    }
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    crate::layout::layout_single_page(&mut doc, &computed, PageBox::A4).unwrap();
    // The second row starts one spacing below the grid edge; Ahem's
    // baseline sits 16px into its 20px line.
    assert_eq!(doc.nodes[table].table_first_baseline, Some(26.0));
}

#[test]
fn collapsed_end_rows_leave_the_outer_edge_to_the_rows_that_remain() {
    let (single, single_table, _) = collapsing_table(&[(ROW, "")]);
    let single_height = single.nodes[single_table].unrounded_layout.size.height;
    for rows in [
        [(COLLAPSED, "border-top:10px solid black"), (ROW, "")],
        [(ROW, ""), (COLLAPSED, "border-bottom:10px solid black")],
    ] {
        let (doc, table, _) = collapsing_table(&rows);
        assert_eq!(
            doc.nodes[table].unrounded_layout.size.height, single_height,
            "{rows:?}"
        );
    }
    // With no row left, only the table's own borders remain.
    let (doc, table, _) = collapsing_table(&[(COLLAPSED, "border:10px solid black")]);
    assert_eq!(doc.nodes[table].unrounded_layout.size.height, 0.0);
}

#[test]
fn a_table_whose_rows_all_collapse_keeps_no_spacing() {
    let fixture = fixture(SEPARATE, GROUP, &[COLLAPSED, COLLAPSED], None);
    let group = fixture.doc.nodes[fixture.rows[0]].parent.unwrap();
    let table = fixture.doc.nodes[group].parent.unwrap();
    assert_eq!(fixture.doc.nodes[table].unrounded_layout.size.height, 0.0);
}

#[test]
fn the_row_after_a_collapsed_first_row_resolves_against_the_table_edge() {
    // The collapsed row's wide bottom border would win the line between the
    // rows; once it is gone the second row's top border meets the table's.
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
        Some("display:table;border-collapse:collapse;border-top:2px solid black"),
    );
    let mut cells = Vec::new();
    for (row_css, cell_css) in [
        (COLLAPSED, "border-bottom:10px solid black"),
        (ROW, "border-top:4px solid black"),
    ] {
        let row = doc.append_element(Some(table), "div", Style::default(), Some(row_css));
        cells.push(doc.append_element(
            Some(row),
            "div",
            Style::default(),
            Some(format!(
                "display:table-cell;width:20px;height:20px;{cell_css}"
            )),
        ));
    }
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    crate::layout::layout_single_page(&mut doc, &computed, PageBox::A4).unwrap();
    let top = doc.nodes[cells[1]].collapsed_border().unwrap().top;
    assert_eq!(top.width(), ComputedLength(4.0));
}

#[test]
fn a_spanning_cell_moved_past_a_collapsed_row_aligns_with_its_new_row() {
    let mut doc = Document::new();
    crate::layout::test_support::with_ahem(&mut doc);
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;margin:0"),
    );
    let table = doc.append_element(Some(body), "div", Style::default(), Some(SEPARATE));
    let mut cells = Vec::new();
    for (row_css, font_size, span) in [(COLLAPSED, 40, true), (ROW, 20, false)] {
        let row = doc.append_element(Some(table), "div", Style::default(), Some(row_css));
        let cell = doc.append_element(
            Some(row),
            "div",
            Style::default(),
            Some(format!(
                "display:table-cell;padding:0;vertical-align:baseline;font-family:Ahem;font-size:{font_size}px;line-height:{font_size}px"
            )),
        );
        if span {
            doc.set_element_attributes(cell, vec![("rowspan".into(), "2".into())]);
        }
        doc.append_text(cell, "X");
        cells.push(cell);
    }
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    crate::layout::layout_single_page(&mut doc, &computed, PageBox::A4).unwrap();
    // The moved 40px cell's baseline sits 32px down and now sets the second
    // row's baseline, so the 20px cell there (16px down) shifts by 16px.
    assert_eq!(doc.nodes[cells[1]].unrounded_layout.padding.top, 16.0);
}
