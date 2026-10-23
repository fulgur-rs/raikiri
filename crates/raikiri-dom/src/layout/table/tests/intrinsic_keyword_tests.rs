use super::*;

fn cell_with_text(limits: &str) -> (Document, usize) {
    let mut doc = Document::new();
    crate::layout::test_support::with_ahem(&mut doc);
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let cell = doc.append_element(
        Some(html),
        "td",
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

fn compute_table_intrinsic_width(limits: &str, available_space: AvailableSpace) -> f32 {
    compute_table_width_with_cells(limits, available_space, 1, 1, "", false)
}

fn compute_table_width_with_cells(
    limits: &str,
    available_space: AvailableSpace,
    rows: usize,
    columns: usize,
    cell_limits: &str,
    collapse_first_column: bool,
) -> f32 {
    let mut doc = Document::new();
    crate::layout::test_support::with_ahem(&mut doc);
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;width:200px"),
    );
    let table = doc.append_element(
        Some(body),
        "table",
        Some(&format!("display:table;border-spacing:0;{limits}")),
    );
    if collapse_first_column {
        for column in 0..columns {
            let visibility = if column == 0 { "collapse" } else { "visible" };
            doc.append_element(
                Some(table),
                "col",
                Some(&format!("display:table-column;visibility:{visibility}")),
            );
        }
    }
    for _ in 0..rows {
        let row = doc.append_element(Some(table), "tr", Some("display:table-row"));
        for _ in 0..columns {
            let cell = doc.append_element(
                Some(row), "td", Some(&format!("display:table-cell;padding:0;font-family:Ahem;font-size:10px;line-height:10px;{cell_limits}")),
            );
            doc.append_text(cell, "AAAA BBBB");
        }
    }
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    crate::layout::apply_computed_to_style(&mut doc, &computed).unwrap();
    let inputs = LayoutInput {
        run_mode: RunMode::ComputeSize,
        axis: RequestedAxis::Horizontal,
        parent_size: Size {
            width: Some(200.0),
            height: None,
        },
        available_space: Size {
            width: available_space,
            height: AvailableSpace::MaxContent,
        },
        ..super::column_probe_input()
    };

    super::with_layout_session(&mut doc, |session| {
        super::compute_table_layout_in_session(session, NodeId::from(table), inputs)
            .size
            .width
    })
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
    let first = doc.append_element(Some(html), "td", Some("display:table-cell"));
    let second = doc.append_element(Some(html), "td", Some("display:table-cell"));
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
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;width:200px"),
    );
    let table = doc.append_element(
        Some(body),
        "table",
        Some("display:table;border-spacing:0;max-width:min-content"),
    );
    let row = doc.append_element(Some(table), "tr", Some("display:table-row"));
    let cell = doc.append_element(
        Some(row),
        "td",
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
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;width:200px"),
    );
    let table = doc.append_element(
        Some(body),
        "table",
        Some("display:table;border-spacing:0;min-width:max-content;max-width:min-content"),
    );
    let row = doc.append_element(Some(table), "tr", Some("display:table-row"));
    let cell = doc.append_element(
        Some(row),
        "td",
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
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;width:200px"),
    );
    let table = doc.append_element(Some(body), "table", Some("display:table;border-spacing:0"));
    let caption = doc.append_element(
        Some(table),
        "caption",
        Some("display:table-caption;min-width:max-content;font-family:Ahem;font-size:10px;line-height:10px"),
    );
    doc.append_text(caption, "AAAA BBBB");
    let row = doc.append_element(Some(table), "tr", Some("display:table-row"));
    doc.append_element(
        Some(row),
        "td",
        Some("display:table-cell;width:10px;height:10px;padding:0"),
    );

    finish_ahem_layout(&mut doc);

    assert!(
        doc.nodes[table].unrounded_layout.size.width >= 90.0,
        "caption min-width:max-content should raise table CAPMIN to 90px, got {}",
        doc.nodes[table].unrounded_layout.size.width
    );
}

#[test]
fn table_wrapper_limits_constrain_min_content_and_max_content_measurements() {
    assert_eq!(
        compute_table_intrinsic_width("min-width:max-content", AvailableSpace::MinContent),
        90.0
    );
    assert_eq!(
        compute_table_intrinsic_width("max-width:min-content", AvailableSpace::MaxContent),
        40.0
    );
}

#[test]
fn vertical_fit_content_limit_without_basis_uses_intrinsic_available_space() {
    let mut doc = Document::new();
    super::with_layout_session(&mut doc, |session| {
        let inputs = LayoutInput {
            parent_size: Size {
                width: None,
                height: None,
            },
            ..super::column_probe_input()
        };
        for (available_space, expected) in [
            (AvailableSpace::MinContent, 20.0),
            (AvailableSpace::MaxContent, 80.0),
        ] {
            let context = super::super::IntrinsicSizeContext {
                min_content: 20.0,
                max_content: 80.0,
                inputs: &inputs,
                available_space,
                horizontal: false,
            };
            assert_eq!(
                super::super::resolve_intrinsic_limit(
                    session,
                    0,
                    Dimension::fit_content_percent(0.5),
                    context,
                ),
                Some(expected)
            );
        }
    });
}

#[test]
fn vertical_caption_min_height_max_content_contributes_to_table_inline_size() {
    let mut doc = Document::new();
    crate::layout::test_support::with_ahem(&mut doc);
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;width:200px"),
    );
    let table = doc.append_element(
        Some(body),
        "table",
        Some("display:table;border-spacing:0;writing-mode:vertical-rl"),
    );
    let caption = doc.append_element(
        Some(table),
        "caption",
        Some(
            "display:table-caption;min-height:max-content;margin-top:3px;margin-bottom:4px;\
             font-family:Ahem;font-size:10px;line-height:10px",
        ),
    );
    doc.append_text(caption, "AAAA BBBB");
    let row = doc.append_element(Some(table), "tr", Some("display:table-row"));
    doc.append_element(
        Some(row),
        "td",
        Some("display:table-cell;width:10px;height:10px;padding:0"),
    );

    finish_ahem_layout(&mut doc);

    assert!(
        doc.nodes[table].unrounded_layout.size.height >= 97.0,
        "vertical caption min-height:max-content and its margins should contribute at least 97px, got {}",
        doc.nodes[table].unrounded_layout.size.height
    );
}

#[test]
fn ordinary_table_skips_wrapper_column_probes() {
    for limits in [
        "",
        "min-height:max-content",
        "min-width:20px;max-width:180px",
    ] {
        super::COLUMN_RESOLUTIONS.with(|count| count.set(0));
        assert_eq!(
            compute_table_intrinsic_width(limits, AvailableSpace::MaxContent),
            90.0
        );
        assert_eq!(
            super::COLUMN_RESOLUTIONS.with(|count| count.get()),
            1,
            "{limits}"
        );
    }
}

#[test]
fn fixed_table_skips_wrapper_column_probes() {
    super::COLUMN_RESOLUTIONS.with(|count| count.set(0));
    assert_eq!(
        compute_table_intrinsic_width(
            "table-layout:fixed;width:160px",
            AvailableSpace::Definite(200.0)
        ),
        160.0
    );
    assert_eq!(super::COLUMN_RESOLUTIONS.with(|count| count.get()), 0);
}

#[test]
fn large_table_skips_wrapper_column_probes_with_cell_keywords() {
    for (rows, columns) in [(1, 1), (64, 8)] {
        super::COLUMN_RESOLUTIONS.with(|count| count.set(0));
        let width = compute_table_width_with_cells(
            "",
            AvailableSpace::MaxContent,
            rows,
            columns,
            "min-width:max-content",
            false,
        );
        assert_eq!(width, 90.0 * columns as f32);
        assert_eq!(super::COLUMN_RESOLUTIONS.with(|count| count.get()), 1);
    }
}

#[test]
fn width_keyword_keeps_wrapper_column_probes() {
    for limits in [
        "min-width:min-content",
        "min-width:max-content",
        "max-width:fit-content",
        "max-width:fit-content(60px)",
    ] {
        super::COLUMN_RESOLUTIONS.with(|count| count.set(0));
        compute_table_intrinsic_width(limits, AvailableSpace::MaxContent);
        assert_eq!(
            super::COLUMN_RESOLUTIONS.with(|count| count.get()),
            3,
            "{limits}"
        );
    }
}

#[test]
fn wrapper_min_content_limit_preserves_min_content_available_space() {
    assert_eq!(
        compute_table_intrinsic_width("min-width:min-content", AvailableSpace::MinContent),
        40.0
    );
}

#[test]
fn wrapper_width_keywords_measure_only_visible_columns() {
    for limits in ["min-width:max-content", "max-width:min-content"] {
        let width =
            compute_table_width_with_cells(limits, AvailableSpace::MaxContent, 1, 2, "", true);
        let expected = if limits.starts_with("min-") {
            90.0
        } else {
            40.0
        };
        assert_eq!(width, expected, "{limits}");
    }
}
