use super::*;

fn cell_with_text(limits: &str) -> (Document, usize) {
    let mut doc = Document::new();
    crate::layout::test_support::with_ahem(&mut doc);
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let cell = doc.append_element(
        Some(html),
        "td",
        Style::default(),
        Some(&format!(
            "display:table-cell;font-family:Ahem;font-size:10px;line-height:10px;{limits}"
        )),
    );
    doc.append_text(cell, "AAAA BBBB");
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    crate::layout::apply_computed_to_style(&mut doc, &computed).unwrap();
    (doc, cell)
}

fn resolve_cell_width(doc: &mut Document, cell: usize, available_space: AvailableSpace) -> f32 {
    let grid = super::super::TableGrid {
        n_cols: 1,
        rows: vec![],
        cells: vec![super::make_cell(cell, 0, 0, 1, 1, Dimension::auto())],
        col_widths: vec![],
    };
    let inputs = LayoutInput {
        known_dimensions: Size::NONE,
        available_space: Size {
            width: available_space,
            height: AvailableSpace::MaxContent,
        },
        ..super::column_probe_input()
    };
    super::resolve_column_widths_in_session(
        doc,
        &grid,
        inputs,
        Size {
            width: 0.0,
            height: 0.0,
        },
        0.0,
    )[0]
}

fn finish_ahem_layout(doc: &mut Document) {
    doc.mark_in_document_flags();
    let computed = cascade(doc, &build_rule_tree(doc)).unwrap();
    crate::layout::layout_single_page(doc, &computed, PageBox::A4).unwrap();
}

#[test]
fn cell_min_width_max_content_raises_column_minimum() {
    let (mut doc, cell) = cell_with_text("min-width:max-content");

    let minimum = resolve_cell_width(&mut doc, cell, AvailableSpace::MinContent);
    assert!(
        minimum >= 90.0,
        "max-content min-width should raise the column minimum to 90px, got {minimum}"
    );
}

#[test]
fn cell_max_width_min_content_caps_maximum_without_shrinking_minimum() {
    let (mut doc, cell) = cell_with_text("max-width:min-content");

    let minimum = resolve_cell_width(&mut doc, cell, AvailableSpace::MinContent);
    let maximum = resolve_cell_width(&mut doc, cell, AvailableSpace::MaxContent);

    assert_eq!(minimum, 40.0, "the min-content floor must remain intact");
    assert_eq!(
        maximum, 40.0,
        "max-width:min-content should cap max-content"
    );
}

#[test]
fn cell_fit_content_uses_available_column_room() {
    let (mut doc, cell) = cell_with_text("max-width:fit-content");

    let maximum = resolve_cell_width(&mut doc, cell, AvailableSpace::Definite(60.0));

    assert_eq!(maximum, 60.0, "fit-content should use the 60px column room");
}

#[test]
fn spanning_cell_min_width_max_content_distributes_across_tracks() {
    let (mut doc, spanning_cell) = cell_with_text("min-width:max-content");
    let html = doc.nodes[spanning_cell].parent.unwrap();
    let first = doc.append_element(
        Some(html),
        "td",
        Style::default(),
        Some("display:table-cell"),
    );
    let second = doc.append_element(
        Some(html),
        "td",
        Style::default(),
        Some("display:table-cell"),
    );
    let grid = super::super::TableGrid {
        n_cols: 2,
        rows: vec![],
        cells: vec![
            super::make_cell(spanning_cell, 0, 0, 2, 1, Dimension::auto()),
            super::make_cell(first, 1, 0, 1, 1, Dimension::auto()),
            super::make_cell(second, 1, 1, 1, 1, Dimension::auto()),
        ],
        col_widths: vec![],
    };
    let widths = super::resolve_column_widths_in_session(
        &mut doc,
        &grid,
        super::column_probe_input(),
        Size {
            width: 0.0,
            height: 0.0,
        },
        0.0,
    );

    assert!(
        widths.iter().sum::<f32>() >= 90.0,
        "the spanning cell's max-content minimum should distribute to both tracks: {widths:?}"
    );
}

#[test]
fn cell_min_width_keyword_counts_padding_and_border_once() {
    let (mut doc, cell) = cell_with_text(
        "min-width:max-content;padding-left:5px;padding-right:5px;\
         border-left:1px solid black;border-right:1px solid black",
    );

    let minimum = resolve_cell_width(&mut doc, cell, AvailableSpace::MinContent);

    assert_eq!(
        minimum, 102.0,
        "90px content plus 10px padding and 2px border"
    );
}

#[test]
fn wrapper_max_width_min_content_bounds_columns_before_distribution() {
    let mut doc = Document::new();
    crate::layout::test_support::with_ahem(&mut doc);
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;margin:0;width:200px"),
    );
    let table = doc.append_element(
        Some(body),
        "table",
        Style::default(),
        Some("display:table;border-spacing:0;max-width:min-content"),
    );
    let row = doc.append_element(
        Some(table),
        "tr",
        Style::default(),
        Some("display:table-row"),
    );
    let cell = doc.append_element(
        Some(row),
        "td",
        Style::default(),
        Some("display:table-cell;padding:0;font-family:Ahem;font-size:10px;line-height:10px"),
    );
    doc.append_text(cell, "AAAA BBBB");

    finish_ahem_layout(&mut doc);

    assert_eq!(doc.nodes[cell].unrounded_layout.size.width, 40.0);
    assert_eq!(doc.nodes[table].unrounded_layout.size.width, 40.0);
}

#[test]
fn wrapper_min_width_max_content_wins_over_max_min_content() {
    let mut doc = Document::new();
    crate::layout::test_support::with_ahem(&mut doc);
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;margin:0;width:200px"),
    );
    let table = doc.append_element(
        Some(body),
        "table",
        Style::default(),
        Some("display:table;border-spacing:0;min-width:max-content;max-width:min-content"),
    );
    let row = doc.append_element(
        Some(table),
        "tr",
        Style::default(),
        Some("display:table-row"),
    );
    let cell = doc.append_element(
        Some(row),
        "td",
        Style::default(),
        Some("display:table-cell;padding:0;font-family:Ahem;font-size:10px;line-height:10px"),
    );
    doc.append_text(cell, "AAAA BBBB");

    finish_ahem_layout(&mut doc);

    assert_eq!(doc.nodes[cell].unrounded_layout.size.width, 90.0);
    assert_eq!(doc.nodes[table].unrounded_layout.size.width, 90.0);
}

#[test]
fn caption_min_width_max_content_contributes_to_table_minimum() {
    let mut doc = Document::new();
    crate::layout::test_support::with_ahem(&mut doc);
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;margin:0;width:200px"),
    );
    let table = doc.append_element(
        Some(body),
        "table",
        Style::default(),
        Some("display:table;border-spacing:0"),
    );
    let caption = doc.append_element(
        Some(table),
        "caption",
        Style::default(),
        Some("display:table-caption;min-width:max-content;font-family:Ahem;font-size:10px;line-height:10px"),
    );
    doc.append_text(caption, "AAAA BBBB");
    let row = doc.append_element(
        Some(table),
        "tr",
        Style::default(),
        Some("display:table-row"),
    );
    doc.append_element(
        Some(row),
        "td",
        Style::default(),
        Some("display:table-cell;width:10px;height:10px;padding:0"),
    );

    finish_ahem_layout(&mut doc);

    assert!(
        doc.nodes[table].unrounded_layout.size.width >= 90.0,
        "caption min-width:max-content should raise table CAPMIN to 90px, got {}",
        doc.nodes[table].unrounded_layout.size.width
    );
}
