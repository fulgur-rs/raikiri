use super::*;

/// A table of `rows` row styles, each holding one 20px-tall cell (or an
/// extra spanning cell in the first row when `span` is set). Returns the
/// table, its row ids and its cell ids in document order.
struct Fixture {
    doc: Document,
    rows: Vec<usize>,
    cells: Vec<usize>,
}

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

#[test]
fn collapsed_rows_join_the_lines_on_either_side_in_the_collapsing_model() {
    // 2px cell borders collapse to 2px lines. The middle row's two lines
    // become one, and a collapsed last row leaves the line before it alone.
    let table = "display:table;border-collapse:collapse";
    for (rows, middle_collapsed) in [
        (&[ROW, COLLAPSED, ROW][..], true),
        (&[ROW, ROW, COLLAPSED][..], false),
    ] {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
        let body = doc.append_element(
            Some(html),
            "body",
            Style::default(),
            Some("display:block;margin:0"),
        );
        let table = doc.append_element(Some(body), "div", Style::default(), Some(table));
        let cells: Vec<_> = rows
            .iter()
            .map(|row_css| {
                let row = doc.append_element(Some(table), "div", Style::default(), Some(*row_css));
                doc.append_element(
                    Some(row),
                    "div",
                    Style::default(),
                    Some("display:table-cell;width:20px;height:20px;padding:0;border:2px solid black;box-sizing:border-box"),
                )
            })
            .collect();
        doc.mark_in_document_flags();
        let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
        crate::layout::layout_single_page(&mut doc, &computed, PageBox::A4).unwrap();
        let first = cell_box(&doc, cells[0]);
        let second = cell_box(&doc, cells[1]);
        let third = cell_box(&doc, cells[2]);
        // Adjoining rows overlap by the 2px line between them.
        if middle_collapsed {
            assert_eq!(second.1, 0.0);
            assert_eq!(third.0 - first.0, 18.0);
        } else {
            assert_eq!(second.0 - first.0, 18.0);
            assert_eq!(third.1, 0.0);
        }
    }
}
