use crate::document::Document;
use crate::layout::context::LayoutSession;
use crate::layout::protocol::{
    AvailableSpace, LayoutInput, LayoutOutput, NodeId, OofCandidates, OofPosition, RequestedAxis,
    RunMode, SizingMode,
};
use raikiri_layout_geometry::{Edges, Line, Point, Size};
use raikiri_style::property::{
    Border, BorderColor, BorderStyle, CssColor, DisplayValue, Length, Sides,
};
use raikiri_style::{ComputedLength, ResolveContext, build_rule_tree, cascade, resolve_border};
use raikiri_traits::PageBox;

use raikiri_layout_style::values::{Dimension, LengthPercentage, LengthPercentageAuto};

mod ancestor_walk_tests;
mod anonymous_table_tests;
mod authored_length_tests;
pub(super) mod border_scope_tests;
mod border_state_tests;
mod cache_tests;
mod cell_index_tests;
pub(super) mod cell_scope_tests;
mod intrinsic_keyword_tests;
mod part_containing_block_tests;
mod rowspan_tests;
mod visibility_collapse_column_tests;
mod visibility_collapse_tests;

fn with_layout_session<R>(
    document: &mut Document,
    layout: impl FnOnce(&mut LayoutSession<'_>) -> R,
) -> R {
    crate::layout::test_support::with_bridged_session(document, layout)
}

pub(super) fn build_table_grid_in_document(
    document: &mut Document,
    table: usize,
) -> Result<super::TableGrid, raikiri_traits::LayoutError> {
    with_layout_session(document, |session| super::build_table_grid(session, table))
}

/// A length written as a pass override: it persists across every style scope,
/// as the authored value of a box does.
fn length_override(
    property: raikiri_layout_style::LengthProperty,
    value: raikiri_style::ComputedLengthPercentageOrAuto,
) -> raikiri_layout_style::OwnedPropertyOverride {
    raikiri_layout_style::OwnedPropertyOverride::length(property, value, false).unwrap()
}

fn width_override(
    width: raikiri_style::ComputedLengthPercentageOrAuto,
) -> raikiri_layout_style::OwnedPropertyOverride {
    length_override(raikiri_layout_style::LengthProperty::Width, width)
}

fn margin_left_override(
    margin: raikiri_style::ComputedLengthPercentageOrAuto,
) -> raikiri_layout_style::OwnedPropertyOverride {
    length_override(raikiri_layout_style::LengthProperty::MarginLeft, margin)
}

/// The border widths each of `boxes` has right now, read the way table layout
/// reads them: through the style view with scoped overrides.
fn borders_in_document(document: &mut Document, boxes: &[usize]) -> Vec<Edges<LengthPercentage>> {
    with_layout_session(document, |session| {
        boxes
            .iter()
            .map(|&idx| session.table_style(idx).border())
            .collect()
    })
}

fn border_in_document(document: &mut Document, idx: usize) -> Edges<LengthPercentage> {
    borders_in_document(document, &[idx])[0]
}

fn compute_table_layout_in_session(
    session: &mut LayoutSession<'_>,
    table: NodeId,
    inputs: LayoutInput,
) -> LayoutOutput {
    super::compute_table_layout(session, table, inputs)
}

fn resolve_column_widths_in_session(
    document: &mut Document,
    grid: &super::TableGrid,
    inputs: LayoutInput,
    padding_border_size: Size<f32>,
    spacing: f32,
) -> Vec<f32> {
    with_layout_session(document, |session| {
        super::resolve_column_widths(session, grid, inputs, padding_border_size, spacing)
    })
}

fn resolve_row_heights_in_session(
    document: &mut Document,
    grid: &mut super::TableGrid,
    column_widths: &[f32],
    spacing: (f32, f32),
) -> Result<Vec<f32>, raikiri_traits::LayoutError> {
    with_layout_session(document, |session| {
        super::resolve_row_heights(session, grid, column_widths, spacing)
    })
}

fn place_cells_in_session(
    document: &mut Document,
    grid: &mut super::TableGrid,
    column_widths: &[f32],
    row_tracks: (&[f32], &[f32], &[bool]),
    origins: (&[f32], &[f32]),
    spacing: (f32, f32),
    vertical_writing: bool,
) -> Result<(), raikiri_traits::LayoutError> {
    let gaps = vec![spacing.0; column_widths.len()];
    let collapsed = vec![false; column_widths.len()];
    let (row_heights, row_gaps, collapsed_rows) = row_tracks;
    with_layout_session(document, |session| {
        let mut oof_candidates = OofCandidates::new();
        super::place_cells(
            session,
            grid,
            super::CellPlacementTracks {
                column_widths,
                column_gaps: &gaps,
                collapsed_columns: &collapsed,
                row_heights,
                row_gaps,
                collapsed_rows,
                column_origins: origins.0,
                row_origins: origins.1,
                row_spacing: spacing.1,
                vertical_writing,
            },
            &mut oof_candidates,
        )
    })
}

fn append_grid_part(
    doc: &mut Document,
    parent: usize,
    tag: &str,
    display: DisplayValue,
    css: &str,
) -> usize {
    let id = doc.append_element(Some(parent), tag, Some(css));
    doc.set_display(id, display);
    id
}

#[test]
fn rowspans_reserve_columns_in_native_and_prepared_grids() {
    for prepared in [false, true] {
        let mut doc = Document::new();
        let table = append_grid_part(&mut doc, 0, "table", DisplayValue::Table, "display:table");
        let row1 = append_grid_part(
            &mut doc,
            table,
            "tr",
            DisplayValue::TableRow,
            "display:table-row",
        );
        let span = append_grid_part(
            &mut doc,
            row1,
            "td",
            DisplayValue::TableCell,
            "display:table-cell",
        );
        doc.set_element_attribute(span, "rowspan", "2").unwrap();
        append_grid_part(
            &mut doc,
            row1,
            "td",
            DisplayValue::TableCell,
            "display:table-cell",
        );
        let row2 = append_grid_part(
            &mut doc,
            table,
            "tr",
            DisplayValue::TableRow,
            "display:table-row",
        );
        append_grid_part(
            &mut doc,
            row2,
            "td",
            DisplayValue::TableCell,
            "display:table-cell",
        );
        doc.mark_in_document_flags();
        if prepared {
            let cascade = cascade(&doc, &build_rule_tree(&doc)).unwrap();
            crate::layout::apply_computed_to_style(&mut doc, &cascade).unwrap();
        }
        let grid = build_table_grid_in_document(&mut doc, table).unwrap();
        assert_eq!(grid.n_cols, 2);
        assert_eq!(
            grid.cells
                .iter()
                .map(|cell| (cell.row, cell.col_start))
                .collect::<Vec<_>>(),
            [(0, 0), (0, 1), (1, 1)]
        );
    }
}

#[test]
fn a_rowspan_that_fills_the_column_limit_rejects_a_cell_beyond_it() {
    let mut doc = Document::new();
    let table = append_grid_part(&mut doc, 0, "table", DisplayValue::Table, "display:table");
    let row1 = append_grid_part(
        &mut doc,
        table,
        "tr",
        DisplayValue::TableRow,
        "display:table-row",
    );
    for colspan in std::iter::repeat_n(1000, 65).chain([535]) {
        let cell = append_grid_part(
            &mut doc,
            row1,
            "td",
            DisplayValue::TableCell,
            "display:table-cell",
        );
        doc.set_element_attribute(cell, "rowspan", "2").unwrap();
        doc.set_element_attribute(cell, "colspan", colspan.to_string())
            .unwrap();
    }
    let row2 = append_grid_part(
        &mut doc,
        table,
        "tr",
        DisplayValue::TableRow,
        "display:table-row",
    );
    append_grid_part(
        &mut doc,
        row2,
        "td",
        DisplayValue::TableCell,
        "display:table-cell",
    );
    doc.mark_in_document_flags();
    let error = build_table_grid_in_document(&mut doc, table).unwrap_err();
    assert!(
        matches!(error, raikiri_traits::LayoutError::Internal { message } if message == "table column grid exceeds supported bounds")
    );
}

#[test]
fn paged_projection_does_not_change_the_next_single_page_cell_coordinates() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", Some("margin:0"));
    let table = doc.append_element(Some(body), "table", Some("display:table;border-spacing:0"));
    let mut cells = Vec::new();
    let mut rows = Vec::new();
    for _ in 0..2 {
        let row = doc.append_element(Some(table), "tr", Some("display:table-row"));
        rows.push(row);
        cells.push(doc.append_element(
            Some(row),
            "td",
            Some("display:table-cell;width:10px;height:10px"),
        ));
    }
    doc.mark_in_document_flags();
    let cascade = cascade(&doc, &build_rule_tree(&doc)).unwrap();

    for _ in 0..2 {
        crate::layout::layout_single_page(&mut doc, &cascade, PageBox::A4).unwrap();
        assert_eq!(doc.nodes[rows[1]].layout.location.y, 0.0);
        assert_eq!(doc.nodes[cells[1]].layout.location.y, 10.0);

        crate::layout::layout_pages(&mut doc, &cascade, PageBox::A4).unwrap();
        assert_eq!(doc.nodes[rows[1]].layout.location.y, 10.0);
        assert_eq!(doc.nodes[cells[1]].layout.location.y, 0.0);
    }
}

#[test]
fn a_proper_table_keeps_its_cell_geometry_through_the_native_callback() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block"));
    let table = doc.append_element(
        Some(body),
        "table",
        Some("display:table;width:40px;border-spacing:0"),
    );
    let row = doc.append_element(Some(table), "tr", Some("display:table-row"));
    let children: Vec<usize> = (0..2)
        .map(|_| {
            doc.append_element(
                Some(row),
                "td",
                Some("display:table-cell;width:20px;height:10px"),
            )
        })
        .collect();
    doc.mark_in_document_flags();
    let grid = build_table_grid_in_document(&mut doc, table).unwrap();
    assert_eq!(grid.n_cols, 2);
    assert_eq!(grid.rows.len(), 1);
    assert_eq!(
        grid.cells
            .iter()
            .map(|cell| cell.node_id)
            .collect::<Vec<_>>(),
        children
    );
    let mut inputs = column_probe_input();
    inputs.known_dimensions.width = Some(40.0);
    let (output, error_none) = with_layout_session(&mut doc, |session| {
        let output = compute_table_layout_in_session(session, NodeId::from(table), inputs);
        (output, session.context.table_layout_error.is_none())
    });
    assert_eq!(
        output.size,
        Size {
            width: 40.0,
            height: 10.0
        }
    );
    assert!(error_none);
    for (column, &cell) in children.iter().enumerate() {
        let layout = &doc.nodes[cell].layout;
        assert_eq!(
            layout.size,
            Size {
                width: 20.0,
                height: 10.0
            }
        );
        assert_eq!(
            layout.location,
            Point {
                x: column as f32 * 20.0,
                y: 0.0
            }
        );
    }
    assert_eq!(doc.nodes[table].children, [row]);
    assert_eq!(doc.nodes[row].children, children);
    assert!(doc.table_objects.rows.contains_key(&table));
}

#[test]
fn anonymous_block_direction_reaches_the_native_callback() {
    for direction in ["ltr", "rtl"] {
        let mut doc = Document::new();
        let table = doc.append_element(
            Some(0),
            "div",
            Some(
                format!("display:table;width:100px;border-spacing:0;direction:{direction}")
                    .as_str(),
            ),
        );
        let row = doc.append_element(Some(table), "div", Some("display:table-row"));
        let block = doc.append_element(
            Some(row),
            "div",
            Some("display:block;width:20px;height:10px"),
        );
        doc.mark_in_document_flags();
        let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
        crate::layout::apply_computed_to_style(&mut doc, &computed).unwrap();
        assert!(doc.ifc.is_none());
        let mut inputs = column_probe_input();
        inputs.known_dimensions.width = Some(100.0);
        let output = with_layout_session(&mut doc, |session| {
            compute_table_layout_in_session(session, NodeId::from(table), inputs)
        });
        assert_eq!(
            output.size,
            Size {
                width: 100.0,
                height: 10.0
            }
        );
        assert_eq!(
            doc.nodes[block].layout.location.x,
            if direction == "rtl" { 80.0 } else { 0.0 }
        );
    }
}

#[test]
fn laying_out_an_anonymous_cell_leaves_its_source_owner_alone() {
    let mut doc = Document::new();
    let table = doc.append_element(Some(0), "div", Some("display:table;border-spacing:0"));
    let row = doc.append_element(Some(table), "div", Some("display:table-row"));
    let block = doc.append_element(
        Some(row),
        "div",
        Some("display:block;width:20px;height:10px"),
    );
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    crate::layout::apply_computed_to_style(&mut doc, &computed).unwrap();
    assert_eq!(doc.table_objects.cells.len(), 1);
    assert_eq!(doc.table_objects.cells[0].owner, row);
    let key = doc
        .anonymous_cell_of(block)
        .expect("block wrapped in a cell");
    let view_children = doc.nodes[key].children.clone();
    let row_children = doc.nodes[row].children.clone();
    let row_display = doc.nodes[row].display();
    let row_layout = doc.nodes[row].layout;
    let mut inputs = column_probe_input();
    inputs.known_dimensions.width = Some(100.0);
    inputs.run_mode = RunMode::PerformLayout;
    let output = {
        let session_cascade = crate::layout::test_support::cascaded(&doc);
        let mut session =
            crate::layout::context::LayoutSession::bridged(&mut doc, &session_cascade);
        session.compute_child_layout(NodeId::from(key), inputs)
    };
    assert_eq!(
        output.size,
        Size {
            width: 100.0,
            height: 10.0
        }
    );
    // The cell is laid out under its own key: the source row keeps its
    // node and its layout, and the cell's view keeps its children.
    assert_eq!(doc.nodes[row].children, row_children);
    assert_eq!(doc.nodes[row].display(), row_display);
    assert_eq!(doc.nodes[row].layout, row_layout);
    assert_eq!(doc.nodes[key].children, view_children);
    assert_eq!(doc.nodes[block].layout.size.width, 20.0);
    assert_eq!(doc.parent_of(block), Some(row));
    assert_eq!(doc.taffy_parent_of(block), Some(key));
}

#[test]
fn anonymous_blocks_keep_source_coordinates_in_the_table_callback_without_fonts() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block"));
    let table = doc.append_element(Some(body), "div", Some("display:table;border-spacing:0"));
    let group = doc.append_element(Some(table), "div", Some("display:table-row-group"));
    let mut children = Vec::new();
    for _ in 0..2 {
        let row = doc.append_element(Some(group), "div", Some("display:table-row"));
        for _ in 0..2 {
            children.push(doc.append_element(
                Some(row),
                "div",
                Some("display:block;width:20px;height:10px"),
            ));
        }
    }
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    crate::layout::apply_computed_to_style(&mut doc, &computed).unwrap();
    assert!(doc.ifc.is_none());
    assert_eq!(doc.table_objects.cells.len(), 2);
    assert!(
        doc.nodes
            .anonymous_cells()
            .iter()
            .all(|cell| !cell.is_ifc_root())
    );
    let (output, error_none) = with_layout_session(&mut doc, |session| {
        let output =
            compute_table_layout_in_session(session, NodeId::from(table), column_probe_input());
        (output, session.context.table_layout_error.is_none())
    });
    assert_eq!(
        output.size,
        Size {
            width: 20.0,
            height: 40.0
        }
    );
    assert!(error_none);
    assert!(doc.ifc.is_none());
    for (index, child) in children.into_iter().enumerate() {
        assert_eq!(
            doc.nodes[child].layout.location,
            Point {
                x: 0.0,
                y: (index % 2) as f32 * 10.0
            }
        );
    }
}

#[test]
fn spanning_only_cell_floors_the_first_row_at_its_natural_baseline() {
    let mut doc = Document::new();
    crate::layout::test_support::with_ahem(&mut doc);
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block"));
    let table = doc.append_element(
        Some(body),
        "div",
        Some("display:table;width:40px;border-spacing:0"),
    );
    let row = doc.append_element(Some(table), "div", Some("display:table-row"));
    let cell = doc.append_element(Some(row), "div", Some("display:table-cell;width:40px;vertical-align:baseline;font-family:Ahem;font-size:40px;line-height:40px"));
    doc.set_element_attributes(cell, vec![("rowspan".into(), "2".into())]);
    doc.append_text(cell, "X");
    doc.append_element(Some(table), "div", Some("display:table-row"));
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    crate::layout::layout_single_page(&mut doc, &computed, PageBox::A4).unwrap();
    let mut grid = build_table_grid_in_document(&mut doc, table).unwrap();
    let heights = with_layout_session(&mut doc, |session| {
        resolve_row_heights_in_session(session, &mut grid, &[40.0], (0.0, 0.0))
    })
    .unwrap();
    assert_eq!(grid.cells[0].baseline, 32.0);
    // The baseline floors the first row at 32, and that height is what the
    // remaining 8 follow into; with the second row empty it takes none.
    assert_eq!(heights, [40.0, 0.0]);
}

#[test]
fn a_spanning_cells_missing_height_goes_where_the_rule_for_the_spanned_rows_says() {
    // (heights, fixed, starts_span, deficit, expected). `starts_span` and
    // `fixed` stand for the rows the spanning cell crosses; the first entry of
    // `starts_span` is the cell's own row and never counts.
    type Case<'a> = (&'a [f32], &'a [bool], &'a [bool], f32, &'a [f32]);
    let cases: &[Case<'_>] = &[
        // Equally among the later rows where another span starts, whatever
        // their heights or authored heights.
        (
            &[10.0, 0.0, 30.0],
            &[false, false, true],
            &[true, true, true],
            60.0,
            &[10.0, 30.0, 60.0],
        ),
        // In proportion to the rows without an authored height.
        (
            &[10.0, 20.0, 40.0],
            &[false, true, false],
            &[false, false, false],
            50.0,
            &[20.0, 20.0, 80.0],
        ),
        // The authored rows weigh when the others have no height.
        (
            &[0.0, 20.0, 30.0],
            &[false, true, true],
            &[false, false, false],
            50.0,
            &[0.0, 40.0, 60.0],
        ),
        // Every row weighs when all of them are authored.
        (
            &[20.0, 30.0],
            &[true, true],
            &[false, false],
            50.0,
            &[40.0, 60.0],
        ),
        // The last row takes it when all the rows are empty.
        (
            &[0.0, 0.0, 0.0],
            &[false, true, false],
            &[false, false, false],
            12.0,
            &[0.0, 0.0, 12.0],
        ),
    ];
    for &(heights, fixed, starts_span, deficit, expected) in cases {
        let mut heights = heights.to_vec();
        super::share_span_deficit(&mut heights, fixed, starts_span, deficit);
        assert_eq!(heights, expected);
    }
}

fn oversized_columns_document(explicit_row: bool, columns: bool) -> Document {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let table = doc.append_element(Some(body), "div", Some("display: table"));
    let parent = if explicit_row {
        doc.append_element(Some(table), "div", Some("display: table-row"))
    } else {
        table
    };
    for _ in 0..66 {
        let cell = doc.append_element(
            Some(parent),
            "div",
            Some(if columns {
                "display: table-column"
            } else {
                "display: table-cell"
            }),
        );
        doc.set_element_attributes(
            cell,
            vec![(
                if columns { "span" } else { "colspan" }.into(),
                "1000".into(),
            )],
        );
    }
    doc.mark_in_document_flags();
    doc
}

fn assert_oversized_table_is_rejected(explicit_row: bool, columns: bool) {
    let mut doc = oversized_columns_document(explicit_row, columns);
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).unwrap();
    let result = crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4);
    assert!(matches!(
        result,
        Err(raikiri_traits::LayoutError::Internal { .. })
    ));
}

#[test]
fn oversized_explicit_table_columns_return_layout_error() {
    assert_oversized_table_is_rejected(true, false);
}

#[test]
fn oversized_anonymous_table_columns_return_layout_error() {
    assert_oversized_table_is_rejected(false, false);
}

#[test]
fn oversized_column_element_spans_return_layout_error() {
    assert_oversized_table_is_rejected(false, true);
}

#[test]
fn table_column_grid_accepts_exact_representable_boundary() {
    for (explicit, columns) in [(true, false), (false, false), (false, true)] {
        let mut doc = oversized_columns_document(explicit, columns);
        let table = 3;
        let last = doc.nodes.len() - 1;
        let attribute = if columns { "span" } else { "colspan" };
        doc.set_element_attributes(last, vec![(attribute.into(), "535".into())]);
        let grid = build_table_grid_in_document(&mut doc, table).unwrap();
        assert_eq!(grid.n_cols, u16::MAX);
        if columns {
            assert_eq!(grid.col_widths.len(), usize::from(u16::MAX));
        } else {
            let cell = grid.cells.last().unwrap();
            assert_eq!(
                super::cell_column_range(cell, usize::from(grid.n_cols)).unwrap(),
                65000..65535
            );
        }
        doc.set_element_attributes(last, vec![(attribute.into(), "536".into())]);
        let cr = cascade(&doc, &build_rule_tree(&doc)).unwrap();
        assert!(matches!(
            crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4),
            Err(raikiri_traits::LayoutError::Internal { .. })
        ));
    }
}

#[test]
fn layout_error_drops_scratch_before_document_reuse() {
    let mut doc = oversized_columns_document(true, false);
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).unwrap();
    for _ in 0..2 {
        assert!(crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4).is_err());
    }
    for cell in 5..doc.nodes.len() {
        doc.set_element_attributes(cell, vec![("colspan".into(), "1".into())]);
    }
    doc.set_element_inline_style(
        5,
        Some("display: table-cell; width: 10px; height: 10px".into()),
    );
    let cr = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4).unwrap();
    assert!(doc.nodes[3].layout.size.width > 0.0);
    assert!(doc.nodes[5].layout.size.width > 0.0);
}

#[test]
fn invalid_cell_column_ranges_are_rejected_before_both_slice_sinks() {
    let mut doc = Document::new();
    for (start, span) in [(2, 1), (1, 2), (0, 0)] {
        let mut grid = super::TableGrid {
            n_cols: 2,
            rows: vec![0],
            cells: vec![fixed_cell(0, start, span, Dimension::auto())],
            col_widths: vec![],
        };
        let session_cascade = crate::layout::test_support::cascaded(&doc);
        let mut session = LayoutSession::bridged(&mut doc, &session_cascade);
        assert!(
            resolve_row_heights_in_session(&mut session, &mut grid, &[1.0, 1.0], (0.0, 0.0))
                .is_err()
        );
        assert!(
            place_cells_in_session(
                &mut session,
                &mut grid,
                &[1.0, 1.0],
                (&[1.0], &[0.0], &[false]),
                (&[0.0, 1.0, 2.0], &[0.0, 1.0]),
                (0.0, 0.0),
                false
            )
            .is_err()
        );
    }
}

fn collapsed_candidate(
    style: BorderStyle,
    width: f32,
    color: CssColor,
) -> super::CollapsedBorderCandidate {
    let mut specified = Border::new();
    specified.width = Length::Px(width);
    specified.style = style;
    specified.color = BorderColor::Resolved(color);
    let border = resolve_border(
        specified,
        ComputedLength(16.0),
        None,
        &ResolveContext::initial(),
    );
    super::CollapsedBorderCandidate {
        border,
        taffy_width: LengthPercentage::length(border.width().px()),
    }
}

#[test]
fn collapsed_border_conflict_prefers_hidden_and_non_none_styles() {
    let solid = collapsed_candidate(BorderStyle::Solid, 8.0, CssColor::BLACK);
    let hidden = collapsed_candidate(BorderStyle::Hidden, 1.0, CssColor::BLACK);
    let none = collapsed_candidate(BorderStyle::None, 12.0, CssColor::BLACK);

    let hidden_winner = super::collapsed_border_winner(solid, hidden);
    assert_eq!(hidden_winner.border.style(), BorderStyle::Hidden);
    assert_eq!(hidden_winner.border.width(), ComputedLength::ZERO);

    let hidden_leading_winner = super::collapsed_border_winner(hidden, solid);
    assert_eq!(hidden_leading_winner.border.style(), BorderStyle::Hidden);

    let solid_winner = super::collapsed_border_winner(none, solid);
    assert_eq!(solid_winner.border.style(), BorderStyle::Solid);
    assert_eq!(solid_winner.border.width(), ComputedLength(8.0));

    let solid_leading_winner = super::collapsed_border_winner(solid, none);
    assert_eq!(solid_leading_winner.border.style(), BorderStyle::Solid);
}

#[test]
fn collapsed_border_conflict_prefers_width_before_style() {
    let wide_solid = collapsed_candidate(BorderStyle::Solid, 5.0, CssColor::BLACK);
    let narrow_double = collapsed_candidate(BorderStyle::Double, 4.0, CssColor::BLACK);
    let winner = super::collapsed_border_winner(wide_solid, narrow_double);
    assert_eq!(winner.border.style(), BorderStyle::Solid);
    assert_eq!(winner.border.width(), ComputedLength(5.0));

    let same_width_double = collapsed_candidate(BorderStyle::Double, 5.0, CssColor::BLACK);
    let winner = super::collapsed_border_winner(wide_solid, same_width_double);
    assert_eq!(winner.border.style(), BorderStyle::Double);
    assert_eq!(winner.border.width(), ComputedLength(5.0));

    let narrower_solid = collapsed_candidate(BorderStyle::Solid, 4.0, CssColor::BLACK);
    let winner = super::collapsed_border_winner(wide_solid, narrower_solid);
    assert_eq!(winner.border.width(), ComputedLength(5.0));

    let same_width_solid = collapsed_candidate(BorderStyle::Solid, 5.0, CssColor::BLACK);
    let winner = super::collapsed_border_winner(wide_solid, same_width_solid);
    assert_eq!(winner.border.color, wide_solid.border.color);
}

#[test]
fn collapsed_border_style_precedence_matches_css_order() {
    let styles = [
        BorderStyle::Double,
        BorderStyle::Solid,
        BorderStyle::Dashed,
        BorderStyle::Dotted,
        BorderStyle::Ridge,
        BorderStyle::Outset,
        BorderStyle::Groove,
        BorderStyle::Inset,
    ];
    for pair in styles.windows(2) {
        let stronger = collapsed_candidate(pair[0], 5.0, CssColor::BLACK);
        let weaker = collapsed_candidate(pair[1], 5.0, CssColor::BLACK);
        assert_eq!(
            super::collapsed_border_winner(stronger, weaker)
                .border
                .style(),
            pair[0]
        );
    }
    assert_eq!(super::border_style_rank(BorderStyle::Hidden), 9);
    assert_eq!(super::border_style_rank(BorderStyle::None), 0);
}

#[test]
fn collapsed_border_conflict_prefers_the_leading_cell_on_a_tie() {
    let left = collapsed_candidate(
        BorderStyle::Solid,
        5.0,
        CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255,
        },
    );
    let right = collapsed_candidate(
        BorderStyle::Solid,
        5.0,
        CssColor {
            r: 0,
            g: 0,
            b: 255,
            a: 255,
        },
    );

    assert_eq!(
        super::collapsed_border_winner(left, right).border.color,
        left.border.color
    );
}

#[test]
fn resolve_collapsed_cell_borders_harmonizes_both_grid_directions() {
    let mut doc = Document::new();
    let widths = [3.0, 5.0, 7.0, 9.0];
    let cells = widths.map(|width| {
        collapsed_cell(
            &mut doc,
            collapsed_candidate(BorderStyle::Solid, width, CssColor::BLACK),
        )
    });
    let grid = super::TableGrid {
        n_cols: 2,
        rows: vec![0, 0],
        cells: vec![
            make_cell(cells[0], 0, 0, 1, 1, Dimension::auto()),
            make_cell(cells[1], 0, 1, 1, 1, Dimension::auto()),
            make_cell(cells[2], 1, 0, 1, 1, Dimension::auto()),
            make_cell(cells[3], 1, 1, 1, 1, Dimension::auto()),
        ],
        col_widths: vec![],
    };

    let used =
        resolve_collapsed_cell_borders_in_document(&mut doc, &grid, &super::CellIndex::new(&grid));

    let borders = cells.map(|cell| used[&cell]);
    assert_eq!(borders[0].right, LengthPercentage::length(5.0));
    assert_eq!(borders[1].left, LengthPercentage::length(5.0));
    assert_eq!(borders[2].right, LengthPercentage::length(9.0));
    assert_eq!(borders[3].left, LengthPercentage::length(9.0));
    assert_eq!(borders[0].bottom, LengthPercentage::length(7.0));
    assert_eq!(borders[2].top, LengthPercentage::length(7.0));
    assert_eq!(borders[1].bottom, LengthPercentage::length(9.0));
    assert_eq!(borders[3].top, LengthPercentage::length(9.0));

    resolve_collapsed_border_pair_in_document(
        &mut doc,
        cells[0],
        super::CellBorderSide::Top,
        cells[1],
        super::CellBorderSide::Right,
    );
    resolve_collapsed_border_pair_in_document(
        &mut doc,
        cells[0],
        super::CellBorderSide::Left,
        cells[1],
        super::CellBorderSide::Bottom,
    );
    let missing_border = bordered_cell(&mut doc, 1.0);
    resolve_collapsed_border_pair_in_document(
        &mut doc,
        cells[0],
        super::CellBorderSide::Left,
        missing_border,
        super::CellBorderSide::Right,
    );
}

/// A `tag` box of the given `display` whose four borders are all `border`.
fn collapsed_box(
    doc: &mut Document,
    tag: &str,
    display: &str,
    border: super::CollapsedBorderCandidate,
) -> usize {
    collapsed_box_in(doc, 0, tag, display, border)
}

/// Like [`collapsed_box`], below `parent`. The root element is blockified, so
/// a box whose display matters goes below some other element.
fn collapsed_box_in(
    doc: &mut Document,
    parent: usize,
    tag: &str,
    display: &str,
    border: super::CollapsedBorderCandidate,
) -> usize {
    let style = format!("{:?}", border.border.style()).to_lowercase();
    let color = match border.border.color {
        BorderColor::Resolved(color) => format!(
            "rgba({},{},{},{})",
            color.r,
            color.g,
            color.b,
            f32::from(color.a) / 255.0
        ),
        _ => "currentcolor".to_owned(),
    };
    doc.append_element(
        Some(parent),
        tag,
        Some(format!(
            "display:{display};border:{}px {style} {color}",
            border.border.width().px()
        )),
    )
}

/// A cell whose four borders are all `border`.
fn collapsed_cell(doc: &mut Document, border: super::CollapsedBorderCandidate) -> usize {
    collapsed_box(doc, "td", "table-cell", border)
}

#[test]
fn both_cells_paint_the_winner_of_their_shared_border() {
    let mut doc = Document::new();
    let purple = CssColor {
        r: 128,
        g: 0,
        b: 128,
        a: 255,
    };
    let blue = CssColor {
        r: 0,
        g: 0,
        b: 255,
        a: 255,
    };
    let above = collapsed_cell(
        &mut doc,
        collapsed_candidate(BorderStyle::Solid, 5.0, purple),
    );
    let below = collapsed_cell(
        &mut doc,
        collapsed_candidate(BorderStyle::Dashed, 5.0, blue),
    );

    resolve_collapsed_border_pair_in_document(
        &mut doc,
        above,
        super::CellBorderSide::Bottom,
        below,
        super::CellBorderSide::Top,
    );

    let painted_bottom = doc.nodes[above]
        .collapsed_border()
        .expect("resolved")
        .bottom;
    let painted_top = doc.nodes[below].collapsed_border().expect("resolved").top;
    assert_eq!(painted_top, painted_bottom);
    assert_eq!(painted_top.style(), BorderStyle::Solid);
    assert_eq!(painted_top.color, BorderColor::Resolved(purple));
    // Sides that took part in no conflict keep their own borders.
    assert_eq!(
        doc.nodes[below]
            .collapsed_border()
            .expect("resolved")
            .bottom
            .style(),
        BorderStyle::Dashed
    );
}

#[test]
fn an_edge_cell_side_yields_to_a_winning_table_border() {
    let mut doc = Document::new();
    let table = collapsed_cell(
        &mut doc,
        collapsed_candidate(BorderStyle::Solid, 5.0, CssColor::BLACK),
    );
    let losing = collapsed_cell(
        &mut doc,
        collapsed_candidate(BorderStyle::Outset, 5.0, CssColor::BLACK),
    );
    let tied = collapsed_cell(
        &mut doc,
        collapsed_candidate(BorderStyle::Solid, 5.0, CssColor::BLACK),
    );
    let grid = super::TableGrid {
        n_cols: 2,
        rows: vec![0],
        cells: vec![
            make_cell(losing, 0, 0, 1, 1, Dimension::auto()),
            make_cell(tied, 0, 1, 1, 1, Dimension::auto()),
        ],
        col_widths: vec![],
    };

    let used = resolve_collapsed_table_edges_in_document(&mut doc, &grid, table);

    let losing_bottom_border = used[&losing].bottom;
    let losing_sides = doc.nodes[losing].collapsed_border().expect("resolved");
    assert_eq!(losing_sides.bottom.style(), BorderStyle::None);
    assert_eq!(losing_sides.bottom.width().px(), 0.0);
    assert_eq!(losing_bottom_border, LengthPercentage::length(0.0));
    // The right side is an inner boundary, not a table edge.
    assert_eq!(losing_sides.right.style(), BorderStyle::Outset);
    // A cell wins a tie with the table.
    assert!(doc.nodes[tied].collapsed_border().is_none());
}

#[test]
fn a_row_border_reaches_the_cells_on_it_and_the_row_paints_nothing() {
    let mut doc = Document::new();
    let red = CssColor {
        r: 255,
        g: 0,
        b: 0,
        a: 255,
    };
    let thin = collapsed_candidate(BorderStyle::Solid, 1.0, CssColor::BLACK);
    let host = doc.append_element(Some(0), "div", None::<&str>);
    let rows: Vec<usize> = (0..3)
        .map(|index| {
            let color = if index == 2 { red } else { CssColor::BLACK };
            collapsed_box_in(
                &mut doc,
                host,
                "tr",
                "table-row",
                collapsed_candidate(BorderStyle::Solid, 5.0, color),
            )
        })
        .collect();
    let spanning = collapsed_box_in(&mut doc, host, "td", "table-cell", thin);
    let grid = super::TableGrid {
        n_cols: 2,
        rows: rows.clone(),
        cells: vec![make_cell(spanning, 0, 0, 1, 3, Dimension::auto())],
        col_widths: vec![],
    };

    let used = resolve_collapsed_row_borders_in_document(&mut doc, &grid);

    // The spanning cell's bottom lies on the last row's bottom border.
    let bottom = doc.nodes[spanning]
        .collapsed_border()
        .expect("resolved")
        .bottom;
    assert_eq!(bottom.color, BorderColor::Resolved(red));
    assert_eq!(bottom.width().px(), 5.0);
    assert_eq!(used[&spanning].bottom, LengthPercentage::length(5.0));
    for row in rows {
        let painted = doc.nodes[row].collapsed_border().expect("row resolved");
        assert_eq!(painted.top.style(), BorderStyle::None);
        assert_eq!(painted.bottom.width().px(), 0.0);
    }
}

#[test]
fn a_spanning_cell_does_not_carry_one_segment_winner_into_the_next() {
    let mut doc = Document::new();
    let red = CssColor {
        r: 255,
        g: 0,
        b: 0,
        a: 255,
    };
    let blue = CssColor {
        r: 0,
        g: 0,
        b: 255,
        a: 255,
    };
    let spanning = collapsed_cell(
        &mut doc,
        collapsed_candidate(BorderStyle::Solid, 1.0, CssColor::BLACK),
    );
    let wide = collapsed_cell(&mut doc, collapsed_candidate(BorderStyle::Solid, 5.0, red));
    let narrow = collapsed_cell(&mut doc, collapsed_candidate(BorderStyle::Solid, 2.0, blue));
    let grid = super::TableGrid {
        n_cols: 2,
        rows: vec![0, 0],
        cells: vec![
            make_cell(spanning, 0, 0, 2, 1, Dimension::auto()),
            make_cell(wide, 1, 0, 1, 1, Dimension::auto()),
            make_cell(narrow, 1, 1, 1, 1, Dimension::auto()),
        ],
        col_widths: vec![],
    };

    resolve_collapsed_cell_borders_in_document(&mut doc, &grid, &super::CellIndex::new(&grid));

    // The spanning cell paints the strongest winner along its bottom.
    let bottom = doc.nodes[spanning]
        .collapsed_border()
        .expect("resolved")
        .bottom;
    assert_eq!(bottom.color, BorderColor::Resolved(red));
    // The narrow cell's segment is decided by the spanning cell's own border.
    let top = doc.nodes[narrow].collapsed_border().expect("resolved").top;
    assert_eq!(top.color, BorderColor::Resolved(blue));
    assert_eq!(top.width().px(), 2.0);
}

#[test]
fn a_cell_paints_the_earlier_cell_border_it_ties_with() {
    let mut doc = Document::new();
    let purple = CssColor {
        r: 128,
        g: 0,
        b: 128,
        a: 255,
    };
    let blue = CssColor {
        r: 0,
        g: 0,
        b: 255,
        a: 255,
    };
    let above = collapsed_cell(
        &mut doc,
        collapsed_candidate(BorderStyle::Solid, 5.0, purple),
    );
    let below = collapsed_cell(&mut doc, collapsed_candidate(BorderStyle::Solid, 5.0, blue));
    let grid = super::TableGrid {
        n_cols: 1,
        rows: vec![0, 0],
        cells: vec![
            make_cell(above, 0, 0, 1, 1, Dimension::auto()),
            make_cell(below, 1, 0, 1, 1, Dimension::auto()),
        ],
        col_widths: vec![],
    };

    resolve_collapsed_cell_borders_in_document(&mut doc, &grid, &super::CellIndex::new(&grid));

    // CSS 2.1 §17.6.2.1: on an exact tie the cell further up wins.
    let top = doc.nodes[below].collapsed_border().expect("resolved").top;
    assert_eq!(top.color, BorderColor::Resolved(purple));
}

#[test]
fn an_anonymous_row_does_not_turn_its_first_cell_into_a_row() {
    let mut doc = Document::new();
    let red = CssColor {
        r: 255,
        g: 0,
        b: 0,
        a: 255,
    };
    let first = collapsed_cell(&mut doc, collapsed_candidate(BorderStyle::Solid, 5.0, red));
    let second = collapsed_cell(
        &mut doc,
        collapsed_candidate(BorderStyle::Solid, 1.0, CssColor::BLACK),
    );
    let grid = super::TableGrid {
        n_cols: 2,
        // An anonymous row is recorded by its first cell.
        rows: vec![first],
        cells: vec![
            make_cell(first, 0, 0, 1, 1, Dimension::auto()),
            make_cell(second, 0, 1, 1, 1, Dimension::auto()),
        ],
        col_widths: vec![],
    };

    resolve_collapsed_row_borders_in_document(&mut doc, &grid);
    resolve_collapsed_cell_borders_in_document(&mut doc, &grid, &super::CellIndex::new(&grid));

    let first_sides = doc.nodes[first].collapsed_border().expect("resolved");
    assert_eq!(first_sides.top.color, BorderColor::Resolved(red));
    assert_eq!(first_sides.top.width().px(), 5.0);
    let second_sides = doc.nodes[second].collapsed_border().expect("resolved");
    assert_eq!(second_sides.left.color, BorderColor::Resolved(red));
    assert_eq!(second_sides.top.width().px(), 1.0);
}

#[test]
fn collapsed_resolution_skips_boxes_without_computed_borders() {
    let mut doc = Document::new();
    let bare_row = collapsed_box(
        &mut doc,
        "tr",
        "table-row",
        collapsed_candidate(BorderStyle::Solid, 2.0, CssColor::BLACK),
    );
    let bare_table = bordered_cell(&mut doc, 2.0);
    let bare_cell = bordered_cell(&mut doc, 2.0);
    let cell = collapsed_cell(
        &mut doc,
        collapsed_candidate(BorderStyle::Solid, 3.0, CssColor::BLACK),
    );
    let grid = super::TableGrid {
        n_cols: 1,
        rows: vec![bare_row],
        cells: vec![make_cell(cell, 0, 0, 1, 1, Dimension::auto())],
        col_widths: vec![],
    };
    let pair = super::TableGrid {
        n_cols: 2,
        rows: vec![bare_row],
        cells: vec![
            make_cell(cell, 0, 0, 1, 1, Dimension::auto()),
            make_cell(bare_cell, 0, 1, 1, 1, Dimension::auto()),
        ],
        col_widths: vec![],
    };

    with_layout_session(&mut doc, |session| {
        // The bridge gives every element computed borders, so strip the ones
        // that stand for boxes it did not reach.
        for bare in [bare_row, bare_table, bare_cell] {
            session.nodes[bare].computed_border = None;
        }
        with_retained_border_state_in(session, &grid, |session, state| {
            super::resolve_collapsed_row_borders(session, state, &grid)
        });
        with_retained_border_state_in(session, &grid, |session, state| {
            super::resolve_collapsed_table_edges(session, state, &grid, bare_table)
        });
        with_retained_border_state_in(session, &pair, |session, state| {
            super::resolve_collapsed_cell_borders(
                session,
                state,
                &pair,
                &super::CellIndex::new(&pair),
            )
        });

        assert!(session.nodes[cell].collapsed_border().is_none());
        assert!(session.nodes[bare_cell].collapsed_border().is_none());
        assert!(session.nodes[bare_row].collapsed_border().is_none());
        assert_eq!(
            session.table_style(cell).border().top,
            LengthPercentage::length(3.0)
        );
    });
}

fn build_simple_2x2() -> Document {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let table = doc.append_element(Some(body), "table", Some("display: table"));
    let tr1 = doc.append_element(Some(table), "tr", Some("display: table-row"));
    let td1 = doc.append_element(Some(tr1), "td", Some("display: table-cell"));
    let td2 = doc.append_element(Some(tr1), "td", Some("display: table-cell"));
    let _t1 = doc.append_text(td1, "cell1");
    let _t2 = doc.append_text(td2, "cell2");
    let tr2 = doc.append_element(Some(table), "tr", Some("display: table-row"));
    let td3 = doc.append_element(Some(tr2), "td", Some("display: table-cell"));
    let td4 = doc.append_element(Some(tr2), "td", Some("display: table-cell"));
    let _t3 = doc.append_text(td3, "cell3");
    let _t4 = doc.append_text(td4, "cell4");
    doc
}

#[test]
fn table_simple_2x2_columns_and_rows() {
    let mut doc = build_simple_2x2();
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade");
    crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4).expect("layout");
    let td_idxs: Vec<usize> = doc
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.tag_name() == Some("td"))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(td_idxs.len(), 4);
    let l0 = doc.nodes[td_idxs[0]].layout;
    let l1 = doc.nodes[td_idxs[1]].layout;
    let l2 = doc.nodes[td_idxs[2]].layout;
    assert!(
        (l1.location.x - l0.location.x).abs() > 1.0,
        "columns must separate: {} vs {}",
        l1.location.x,
        l0.location.x
    );
    assert!(
        (l2.location.x - l0.location.x).abs() < 1.0,
        "first column x must align vertically"
    );
    assert!(
        (l2.location.y - l0.location.y).abs() > 1.0,
        "rows must separate"
    );
    // table outer size should cover at least sum of two columns
    let table_idx = doc
        .nodes
        .iter()
        .position(|n| n.tag_name() == Some("table"))
        .unwrap();
    let tl = doc.nodes[table_idx].layout;
    let expected_w = l0.size.width + l1.size.width;
    assert!(
        tl.size.width >= expected_w - 1.0,
        "table width {} should >= content {}",
        tl.size.width,
        expected_w
    );
}

#[test]
fn table_vertical_writing_mode_maps_columns_to_inline_axis() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let table = doc.append_element(
        Some(body),
        "table",
        Some("display: table; writing-mode: vertical-lr"),
    );
    let row = doc.append_element(Some(table), "tr", Some("display: table-row"));
    let first = doc.append_element(Some(row), "td", Some("display: table-cell"));
    let second = doc.append_element(Some(row), "td", Some("display: table-cell"));
    let _first_child = doc.append_element(Some(first), "div", Some("width: 50px; height: 100px"));
    let _second_child = doc.append_element(Some(second), "div", Some("width: 50px; height: 100px"));
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade");
    crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4).expect("layout");

    let first_layout = doc.nodes[first].layout;
    let second_layout = doc.nodes[second].layout;
    let table_layout = doc.nodes[table].layout;
    assert!(
        (first_layout.location.x - second_layout.location.x).abs() < 0.5,
        "vertical columns share the block-axis origin: {:?} vs {:?}", // cov:ignore: assertion diagnostic is evaluated only on failure.
        first_layout.location,
        second_layout.location
    );
    assert!(
        second_layout.location.y - first_layout.location.y >= 99.5,
        "vertical columns advance along the inline axis: {:?} vs {:?}", // cov:ignore: assertion diagnostic is evaluated only on failure.
        first_layout.location,
        second_layout.location
    );
    assert!(
        table_layout.size.height >= first_layout.size.height + second_layout.size.height - 0.5,
        "vertical table height covers its inline tracks: {:?} vs {:?}", // cov:ignore: assertion diagnostic is evaluated only on failure.
        table_layout.size,
        (first_layout.size, second_layout.size) // cov:ignore: assertion diagnostic is evaluated only on failure.
    );
}

#[test]
fn empty_table_caption_is_laid_out_and_painted() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let table = doc.append_element(Some(body), "table", Some("display: table"));
    let caption = doc.append_element(
        Some(table),
        "caption",
        Some(
            "display: table-caption; width: 100px; height: 50px; margin-left: 200px; \
             position: relative; left: -200px",
        ),
    );
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade");
    crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4).expect("layout");

    let caption_layout = doc.nodes[caption].layout;
    let table_layout = doc.nodes[table].layout;
    assert!((caption_layout.size.width - 100.0).abs() < 0.5);
    assert!((caption_layout.size.height - 50.0).abs() < 0.5);
    assert!((caption_layout.location.x - 200.0).abs() < 0.5);
    assert!(table_layout.size.height >= 50.0);
}

#[test]
fn table_colspan_wide_equals_sum_of_spanned_columns() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let table = doc.append_element(Some(body), "table", Some("display: table"));
    let tr1 = doc.append_element(Some(table), "tr", Some("display: table-row"));
    let td_wide = doc.append_element(Some(tr1), "td", Some("display: table-cell"));
    doc.set_element_attributes(td_wide, vec![("colspan".into(), "2".into())]);
    let _t = doc.append_text(td_wide, "wide");
    let tr2 = doc.append_element(Some(table), "tr", Some("display: table-row"));
    let td_a = doc.append_element(Some(tr2), "td", Some("display: table-cell"));
    let td_b = doc.append_element(Some(tr2), "td", Some("display: table-cell"));
    let _ta = doc.append_text(td_a, "a");
    let _tb = doc.append_text(td_b, "b");
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).unwrap();
    crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4).unwrap();
    let wide = doc.nodes[td_wide].layout;
    let a = doc.nodes[td_a].layout;
    let b = doc.nodes[td_b].layout;
    assert!(
        (wide.size.width - (a.size.width + b.size.width)).abs() < 2.0,
        "wide {} vs a+b {} {}",
        wide.size.width,
        a.size.width + b.size.width,
        (wide.size.width - (a.size.width + b.size.width)).abs()
    );
}

#[test]
fn table_rowspan_distributes_height() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let table = doc.append_element(Some(body), "table", Some("display: table"));
    let tr1 = doc.append_element(Some(table), "tr", Some("display: table-row"));
    let td_span = doc.append_element(Some(tr1), "td", Some("display: table-cell"));
    doc.set_element_attributes(td_span, vec![("rowspan".into(), "2".into())]);
    let _t = doc.append_text(td_span, "span");
    let td1 = doc.append_element(Some(tr1), "td", Some("display: table-cell"));
    let _t1 = doc.append_text(td1, "a");
    let tr2 = doc.append_element(Some(table), "tr", Some("display: table-row"));
    let td2 = doc.append_element(Some(tr2), "td", Some("display: table-cell"));
    let _t2 = doc.append_text(td2, "b");
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).unwrap();
    crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4).unwrap();
    // rowspan cell should span at least as tall as two rows combined
    let span = doc.nodes[td_span].layout;
    let a = doc.nodes[td1].layout;
    let b = doc.nodes[td2].layout;
    assert!(
        span.size.height >= a.size.height - 0.5,
        "rowspan height {} should >= single row {}",
        span.size.height,
        a.size.height
    );
    assert!(
        (span.size.height - (a.size.height + b.size.height)).abs() < 5.0
            || span.size.height >= b.size.height,
        "rowspan approx"
    );
}

#[test]
fn nested_table_depth_cap_does_not_panic() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let mut parent = body;
    for _ in 0..10 {
        let table = doc.append_element(Some(parent), "table", Some("display: table"));
        let tr = doc.append_element(Some(table), "tr", Some("display: table-row"));
        let td = doc.append_element(Some(tr), "td", Some("display: table-cell"));
        let _t = doc.append_text(td, "x");
        parent = td;
    }
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).unwrap();
    let res = crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4);
    assert!(res.is_ok(), "nested table should not panic");
}

#[test]
fn table_with_tbody_anonymization() {
    // Ensure html5ever tbody auto-insertion not required: manually use tbody group
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let table = doc.append_element(Some(body), "table", Some("display: table"));
    let tbody = doc.append_element(Some(table), "tbody", Some("display: table-row-group"));
    let tr = doc.append_element(Some(tbody), "tr", Some("display: table-row"));
    let td = doc.append_element(Some(tr), "td", Some("display: table-cell"));
    let _t = doc.append_text(td, "inside tbody");
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).unwrap();
    crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4).unwrap();
    let td_l = doc.nodes[td].layout;
    assert!(
        td_l.size.width > 1.0 && td_l.size.height > 1.0,
        "tbody wrapped cell should layout"
    );
}

#[test]
fn table_parse_path_via_raikiri_html_uses_ua_display() {
    // Simulate parse path UA injection: inject minimal UA CSS manually.
    use raikiri_style::Origin;
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    // No inline display: rely on UA
    let table = doc.append_element(Some(body), "table", None::<&str>);
    let tr = doc.append_element(Some(table), "tr", None::<&str>);
    let td1 = doc.append_element(Some(tr), "td", None::<&str>);
    let td2 = doc.append_element(Some(tr), "td", None::<&str>);
    let _t1 = doc.append_text(td1, "hi");
    let _t2 = doc.append_text(td2, "bye");
    doc.mark_in_document_flags();
    let mut rules = build_rule_tree(&doc);
    rules.add_stylesheet(raikiri_html::ua::MINIMAL_UA_CSS, Origin::UserAgent);
    let cr = cascade(&doc, &rules).unwrap();
    let table_idx = doc
        .nodes
        .iter()
        .position(|n| n.tag_name() == Some("table"))
        .expect("table");
    assert_eq!(
        cr.computed[table_idx].display,
        raikiri_style::property::DisplayValue::Table,
        "UA should set table display"
    );
    crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4).unwrap();
    let tds: Vec<usize> = doc
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.tag_name() == Some("td"))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(tds.len(), 2);
    let l0 = doc.nodes[tds[0]].layout;
    let l1 = doc.nodes[tds[1]].layout;
    assert!(
        (l1.location.x - l0.location.x).abs() > 1.0,
        "parsed table columns must separate"
    );
}

#[test]
fn fixed_layout_honors_first_row_widths_and_ignores_content() {
    // CSS 2.1 §17.5.2.1: table 400px, first-row cell 100px fixed →
    // columns 100 / 300 regardless of the long content in column 2.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let table = doc.append_element(
        Some(body),
        "table",
        Some("display: table; table-layout: fixed; width: 400px"),
    );
    let tr = doc.append_element(Some(table), "tr", Some("display: table-row"));
    let td1 = doc.append_element(Some(tr), "td", Some("display: table-cell; width: 100px"));
    let td2 = doc.append_element(Some(tr), "td", Some("display: table-cell"));
    let _t1 = doc.append_text(td1, "x");
    let _t2 = doc.append_text(
        td2,
        "a very long content string that must not widen its column under fixed layout",
    );
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).unwrap();
    let table_idx = doc
        .nodes
        .iter()
        .position(|n| n.tag_name() == Some("table"))
        .unwrap();
    assert_eq!(
        cr.computed[table_idx].table_layout,
        raikiri_style::property::TableLayoutValue::Fixed
    );
    crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4).unwrap();
    let l0 = doc.nodes[td1].layout;
    let l1 = doc.nodes[td2].layout;
    assert!(
        (l0.size.width - 100.0).abs() < 2.0,
        "fixed first column should be 100px, got {}",
        l0.size.width
    );
    assert!(
        (l1.size.width - 300.0).abs() < 2.0,
        "fixed second column should take the 300px remainder, got {}",
        l1.size.width
    );
}

#[test]
fn fixed_layout_ignores_non_first_row_widths() {
    // CSS 2.1 §17.5.2.1: only the first row sets widths — a 300px width
    // on a second-row cell must not move the columns off 200/200.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let table = doc.append_element(
        Some(body),
        "table",
        Some("display: table; table-layout: fixed; width: 400px"),
    );
    let tr1 = doc.append_element(Some(table), "tr", Some("display: table-row"));
    let td1 = doc.append_element(Some(tr1), "td", Some("display: table-cell"));
    let td2 = doc.append_element(Some(tr1), "td", Some("display: table-cell"));
    let _a = doc.append_text(td1, "a");
    let _b = doc.append_text(td2, "b");
    let tr2 = doc.append_element(Some(table), "tr", Some("display: table-row"));
    let td3 = doc.append_element(Some(tr2), "td", Some("display: table-cell; width: 300px"));
    let td4 = doc.append_element(Some(tr2), "td", Some("display: table-cell"));
    let _c = doc.append_text(td3, "c");
    let _d = doc.append_text(td4, "d");
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).unwrap();
    crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4).unwrap();
    let l0 = doc.nodes[td1].layout;
    let l1 = doc.nodes[td2].layout;
    assert!(
        (l0.size.width - 200.0).abs() < 2.0 && (l1.size.width - 200.0).abs() < 2.0,
        "unfixed columns should split 400px equally, got {} and {}",
        l0.size.width,
        l1.size.width
    );
}

#[test]
fn collapse_overlaps_adjoining_borders_by_pairwise_min() {
    // CSS 2.1 §17.6.2 (simplified max-width resolution): two 4px-bordered
    // cells collapse the interior line to max(4, 4) = 4 by overlapping
    // the boxes by min(4, 4) = 4.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let table = doc.append_element(
        Some(body),
        "table",
        Some("display: table; border-collapse: collapse"),
    );
    let tr = doc.append_element(Some(table), "tr", Some("display: table-row"));
    let td1 = doc.append_element(
        Some(tr),
        "td",
        Some("display: table-cell; border: 4px solid black"),
    );
    let td2 = doc.append_element(
        Some(tr),
        "td",
        Some("display: table-cell; border: 4px solid black"),
    );
    let _t1 = doc.append_text(td1, "a");
    let _t2 = doc.append_text(td2, "b");
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).unwrap();
    let table_idx = doc
        .nodes
        .iter()
        .position(|n| n.tag_name() == Some("table"))
        .unwrap();
    assert_eq!(
        cr.computed[table_idx].border_collapse,
        raikiri_style::property::BorderCollapseValue::Collapse
    );
    crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4).unwrap();

    let l0 = doc.nodes[td1].layout;
    let l1 = doc.nodes[td2].layout;
    let overlap = (l0.location.x + l0.size.width) - l1.location.x;
    assert!(
        (overlap - 4.0).abs() < 1.0,
        "collapsed interior line should overlap by 4px, got {overlap}"
    );
    let tl = doc.nodes[table_idx].layout;
    // Right-flush invariant: the last cell's far edge plus the 4px
    // collapsed outer border (no padding, no table border) meets the
    // table's right edge — the interior overlap must not leave slack.
    assert!(
        (tl.size.width - (l1.location.x + l1.size.width + 4.0)).abs() < 2.0,
        "table right edge should flush with last cell + outer collapsed border, got {}",
        tl.size.width
    );
}

#[test]
fn separate_cells_abut_without_overlap() {
    // Counterpart check: without `border-collapse: collapse` the same
    // bordered cells abut exactly (no absorbed line).
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let table = doc.append_element(Some(body), "table", Some("display: table"));
    let tr = doc.append_element(Some(table), "tr", Some("display: table-row"));
    let td1 = doc.append_element(
        Some(tr),
        "td",
        Some("display: table-cell; border: 4px solid black"),
    );
    let td2 = doc.append_element(
        Some(tr),
        "td",
        Some("display: table-cell; border: 4px solid black"),
    );
    let _t1 = doc.append_text(td1, "a");
    let _t2 = doc.append_text(td2, "b");
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).unwrap();
    crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4).unwrap();
    let l0 = doc.nodes[td1].layout;
    let l1 = doc.nodes[td2].layout;
    let overlap = (l0.location.x + l0.size.width) - l1.location.x;
    assert!(
        overlap.abs() < 1.0,
        "separate cells should abut exactly, got overlap {overlap}"
    );
}

#[test]
fn explicit_single_row_height_resolves_percentage_child() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let table = doc.append_element(
        Some(body),
        "table",
        Some("display: table; width: 150px; height: 100px; border: 5px solid black"),
    );
    let row = doc.append_element(Some(table), "tr", Some("display: table-row"));
    let cell = doc.append_element(
        Some(row),
        "td",
        Some("display: table-cell; padding: 5px; border: 2px solid magenta"),
    );
    let child = doc.append_element(Some(cell), "div", Some("width: 100%; height: 100%"));
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).unwrap();
    crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4).unwrap();

    let cell_layout = doc.nodes[cell].layout;
    let child_layout = doc.nodes[child].layout;
    assert!(
        cell_layout.size.height >= 99.0,
        "authored table height should stretch the single cell, got {}", // cov:ignore: assertion diagnostic is evaluated only on failure.
        cell_layout.size.height
    );
    assert!(
        child_layout.size.height >= 80.0,
        "cell child should receive a non-zero containing block, got {}", // cov:ignore: assertion diagnostic is evaluated only on failure.
        child_layout.size.height
    );
}

#[test]
fn explicit_table_height_respects_box_sizing() {
    for (box_sizing, expected_outer) in [("border-box", 100.0), ("content-box", 140.0)] {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", None::<&str>);
        let body = doc.append_element(Some(html), "body", None::<&str>);
        let table_style = format!(
            "display: table; box-sizing: {box_sizing}; border: 20px solid black; border-collapse: collapse; height: 100px"
        );
        let table = doc.append_element(Some(body), "table", Some(table_style.as_str()));
        let row = doc.append_element(Some(table), "tr", Some("display: table-row"));
        doc.append_element(Some(row), "td", Some("display: table-cell"));
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).unwrap();
        crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4).unwrap();
        let table_layout = doc.nodes[table].layout;
        assert!(
            (table_layout.size.height - expected_outer).abs() < 1.0,
            "{box_sizing}: table outer height should be {expected_outer}, got {}", // cov:ignore: assertion diagnostic is evaluated only on failure.
            table_layout.size.height
        );
    }
}

#[test]
fn table_calc_minimum_sizes_resolve_against_the_containing_block() {
    // The table algorithm receives indefinite known dimensions, so it must
    // resolve its own calc bounds rather than rely on an outer Taffy clamp.
    for (box_sizing, expected) in [
        (
            "content-box",
            Size {
                width: 76.0,
                height: 66.0,
            },
        ),
        (
            "border-box",
            Size {
                width: 60.0,
                height: 50.0,
            },
        ),
    ] {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", None::<&str>);
        let body = doc.append_element(Some(html), "body", None::<&str>);
        let table = doc.append_element(
            Some(body),
            "table",
            Some(&format!(
                "display:table;border-spacing:0;box-sizing:{box_sizing};\
                 padding:5px;border:3px solid black;\
                 min-width:calc(50% + 10px);min-height:calc(50% + 10px)"
            )),
        );
        let row = doc.append_element(Some(table), "tr", Some("display:table-row"));
        let cell = doc.append_element(Some(row), "td", Some("display:table-cell"));
        doc.mark_in_document_flags();
        let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
        crate::layout::apply_computed_to_style(&mut doc, &computed).unwrap();
        let mut inputs = column_probe_input();
        inputs.parent_size = Size {
            width: Some(100.0),
            height: Some(80.0),
        };
        let output = with_layout_session(&mut doc, |session| {
            compute_table_layout_in_session(session, NodeId::from(table), inputs)
        });
        assert_eq!(output.size, expected);
        assert_eq!(
            doc.nodes[cell].layout.size,
            Size {
                width: expected.width - 16.0,
                height: expected.height - 16.0,
            }
        );
    }
}

// WPT css/css-values/calc-width-table-fixed-1 and
// css/css-tables/fixed-layout-calc-width-001: a calc() width that sums a
// percentage and a length leaves its column auto even in fixed layout.
#[test]
fn fixed_table_first_row_mixed_calc_width_leaves_the_column_auto() {
    assert_eq!(
        fixed_table_calc_column_geometry(false, "calc(25% + 10px)"),
        (100.0, 100.0, 100.0)
    );
}

#[test]
fn fixed_table_col_mixed_calc_width_leaves_the_column_auto() {
    assert_eq!(
        fixed_table_calc_column_geometry(true, "calc(25% + 10px)"),
        (100.0, 100.0, 100.0)
    );
}

// A calc() width with a single kind of term folds into the plain length or
// percentage it stands for, which fixes its column.
#[test]
fn fixed_table_length_only_calc_width_fixes_the_column() {
    assert_eq!(
        fixed_table_calc_column_geometry(false, "calc(2 * 30px)"),
        (60.0, 140.0, 60.0)
    );
}

#[test]
fn fixed_table_percentage_only_calc_width_fixes_the_column() {
    assert_eq!(
        fixed_table_calc_column_geometry(true, "calc(2 * 12.5%)"),
        (50.0, 150.0, 50.0)
    );
}

fn fixed_table_calc_column_geometry(from_column: bool, width: &str) -> (f32, f32, f32) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let table = doc.append_element(
        Some(body),
        "table",
        Some("display:table;table-layout:fixed;width:200px;border-spacing:0"),
    );
    if from_column {
        doc.append_element(
            Some(table),
            "col",
            Some(&format!("display:table-column;width:{width}")),
        );
    }
    let row = doc.append_element(Some(table), "tr", Some("display:table-row"));
    let first = doc.append_element(
        Some(row),
        "td",
        Some(&if from_column {
            "display:table-cell".to_owned()
        } else {
            format!("display:table-cell;width:{width}")
        }),
    );
    let second = doc.append_element(Some(row), "td", Some("display:table-cell"));
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    crate::layout::apply_computed_to_style(&mut doc, &computed).unwrap();
    let mut inputs = column_probe_input();
    inputs.parent_size.width = Some(200.0);
    let output = with_layout_session(&mut doc, |session| {
        compute_table_layout_in_session(session, NodeId::from(table), inputs)
    });
    assert_eq!(output.size.width, 200.0);
    (
        doc.nodes[first].layout.size.width,
        doc.nodes[second].layout.size.width,
        doc.nodes[second].layout.location.x,
    )
}

#[test]
fn table_min_size_respects_box_sizing() {
    // WPT css/css-tables/min-max-size-table-content-box (CSS Sizing 3
    // §3.3, csswg-drafts#5336 / Mozilla bug 1651530): with
    // `box-sizing: content-box`, min-width/min-height apply to the
    // content box — a 50px min with 3px border + 5px padding (16 total
    // insets) grows the content to 50 (outer 66). `border-box` keeps
    // the outer clamp (outer 50, content 34).
    for (box_sizing, expect_outer, expect_cell) in
        [("content-box", 66.0, 50.0), ("border-box", 50.0, 34.0)]
    {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", None::<&str>);
        let body = doc.append_element(Some(html), "body", None::<&str>);
        let table_style = format!(
            "display: table; box-sizing: {box_sizing}; border: 3px solid black; \
             padding: 5px; min-width: 50px; min-height: 50px"
        );
        let table = doc.append_element(Some(body), "table", Some(table_style.as_str()));
        let tr = doc.append_element(Some(table), "tr", Some("display: table-row"));
        let td = doc.append_element(Some(tr), "td", Some("display: table-cell"));
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).unwrap();
        crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4).unwrap();
        let tl = doc.nodes[table].layout;
        let cl = doc.nodes[td].layout;
        assert!(
            (tl.size.width - expect_outer).abs() < 1.5,
            "{box_sizing}: table outer width should be {expect_outer}, got {}",
            tl.size.width
        );
        assert!(
            (tl.size.height - expect_outer).abs() < 1.5,
            "{box_sizing}: table outer height should be {expect_outer}, got {}",
            tl.size.height
        );
        assert!(
            (cl.size.width - expect_cell).abs() < 1.5,
            "{box_sizing}: cell width should be {expect_cell}, got {}",
            cl.size.width
        );
        assert!(
            (cl.size.height - expect_cell).abs() < 1.5,
            "{box_sizing}: cell height should be {expect_cell}, got {}",
            cl.size.height
        );
    }
}

#[test]
fn distribute_columns_preserves_authored_tracks_for_spanning_excess() {
    let widths = super::distribute_columns_with_authored(
        110.0,
        &[5.0, 95.0, 0.0, 5.0],
        &[5.0, 95.0, 0.0, 5.0],
        &[0.0, 0.0, 0.0, 0.0],
        &[true, false, false, true],
    );
    assert!((widths[0] - 5.0).abs() < 0.01);
    assert!((widths[3] - 5.0).abs() < 0.01);
    assert!((widths.iter().sum::<f32>() - 110.0).abs() < 0.01);
}

#[test]
fn distribute_columns_falls_back_to_all_tracks_when_all_are_authored() {
    let widths = super::distribute_columns_with_authored(
        12.0,
        &[5.0, 5.0],
        &[5.0, 5.0],
        &[0.0, 0.0],
        &[true, true],
    );
    assert_eq!(widths, [6.0, 6.0]);
}

#[test]
fn distribute_columns_covers_growth_and_shrink_phases() {
    let grown = super::distribute_columns_with_authored(10.0, &[1.0], &[10.0], &[0.0], &[false]);
    assert_eq!(grown, [10.0]);

    let percentage =
        super::distribute_columns_with_authored(10.0, &[0.0], &[0.0], &[0.5], &[false]);
    assert_eq!(percentage, [10.0]);

    let shrunk = super::distribute_columns_with_authored(
        4.0,
        &[0.0, 5.0],
        &[10.0, 5.0],
        &[1.0, 0.0],
        &[false, false],
    );
    assert_eq!(shrunk, [0.0, 5.0]);
}

#[test]
fn table_colspan_percent_distribution_uses_definite_table_width() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let table = doc.append_element(Some(body), "table", Some("display: table; width: 110px"));
    let tr1 = doc.append_element(Some(table), "tr", Some("display: table-row"));
    let wide = doc.append_element(Some(tr1), "td", Some("display: table-cell; width: 50%"));
    doc.set_element_attributes(wide, vec![("colspan".into(), "2".into())]);
    let tr2 = doc.append_element(Some(table), "tr", Some("display: table-row"));
    let left = doc.append_element(Some(tr2), "td", Some("display: table-cell; width: 5px"));
    let right = doc.append_element(Some(tr2), "td", Some("display: table-cell; width: 5px"));
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).unwrap();
    crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4).unwrap();
    let wide_layout = doc.nodes[wide].layout;
    let left_layout = doc.nodes[left].layout;
    let right_layout = doc.nodes[right].layout;
    assert!((wide_layout.size.width - 110.0).abs() < 1.0);
    assert!((left_layout.size.width - 55.0).abs() < 1.0);
    assert!((right_layout.size.width - 55.0).abs() < 1.0);
}

#[test]
fn absolute_auto_table_uses_definite_containing_block_width() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let container = doc.append_element(
        Some(body),
        "div",
        Some("width: 100px; height: 100px; position: relative"),
    );
    let table = doc.append_element(
        Some(container),
        "table",
        Some("display: table; position: absolute; left: -100px; height: 100px"),
    );
    let row = doc.append_element(Some(table), "tr", Some("display: table-row"));
    let cell = doc.append_element(Some(row), "td", Some("display: table-cell"));
    doc.append_text(cell, "x");
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).unwrap();
    crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4).unwrap();
    let table_layout = doc.nodes[table].layout;
    assert!(
        (table_layout.size.width - 100.0).abs() < 1.0,
        "table width was {}",
        table_layout.size.width
    );
}

// -----------------------------------------------------------------
// distribute_columns_with_authored with no authored tracks — pure column
// distributor (avail/min/max/pct). With all-false authored flags this matches
// the old plain distributor exactly.
// -----------------------------------------------------------------

#[test]
fn distribute_columns_exact_fit_uses_min_widths_unchanged() {
    // avail exactly matches the summed min: neither the grow nor the
    // shrink branch fires, so the result is just `min`.
    let widths = super::distribute_columns_with_authored(
        30.0,
        &[10.0, 20.0],
        &[50.0, 50.0],
        &[0.0, 0.0],
        &[false, false],
    );
    assert_eq!(widths, [10.0, 20.0]);
}

#[test]
fn distribute_columns_grows_toward_max_then_shares_remainder_equally() {
    // avail exceeds the summed max: every column first grows to its
    // own max, then the still-leftover space is split equally.
    let widths = super::distribute_columns_with_authored(
        30.0,
        &[0.0, 0.0],
        &[10.0, 10.0],
        &[0.0, 0.0],
        &[false, false],
    );
    assert_eq!(widths, [15.0, 15.0]);
}

#[test]
fn distribute_columns_percentage_over_100_percent_is_scaled_down() {
    // pct sums to 120% of avail: the `psum > 1.0` branch scales every
    // percentage down by 1/psum before applying it.
    let widths = super::distribute_columns_with_authored(
        100.0,
        &[0.0, 0.0],
        &[100.0, 100.0],
        &[0.6, 0.6],
        &[false, false],
    );
    assert!((widths[0] - 50.0).abs() < 0.01, "widths: {widths:?}");
    assert!((widths[1] - 50.0).abs() < 0.01, "widths: {widths:?}");
}

#[test]
fn distribute_columns_shrinks_percentage_driven_column_when_over_avail() {
    // The pct-driven column (50px) plus the min-floored column (20px)
    // exceed avail (50px): shrink comes only out of the pct column's
    // slack above its own min (column 1 already sits at its min).
    let widths = super::distribute_columns_with_authored(
        50.0,
        &[0.0, 20.0],
        &[100.0, 100.0],
        &[1.0, 0.0],
        &[false, false],
    );
    assert_eq!(widths, [30.0, 20.0]);
}

#[test]
fn distribute_columns_min_floor_holds_when_shrink_budget_is_zero() {
    // Over-constrained input (avail below the summed min) with no
    // percentage columns: every column already sits at its own min,
    // so the shrink budget is zero and the table overflows rather
    // than compressing columns below their intrinsic minimum.
    let widths = super::distribute_columns_with_authored(
        10.0,
        &[20.0, 30.0],
        &[20.0, 30.0],
        &[0.0, 0.0],
        &[false, false],
    );
    assert_eq!(widths, [20.0, 30.0]);
}

// -----------------------------------------------------------------
// distribute_extra — pure grower for resolved columns and rows.
// -----------------------------------------------------------------

#[test]
fn distribute_extra_grows_columns_proportionally_to_current_width() {
    let mut widths = [10.0, 30.0];
    super::distribute_extra(&mut widths, None, 80.0);
    assert_eq!(widths, [20.0, 60.0]);
}

#[test]
fn distribute_extra_splits_equally_when_all_columns_are_zero() {
    let mut widths = [0.0, 0.0, 0.0];
    super::distribute_extra(&mut widths, None, 30.0);
    assert_eq!(widths, [10.0, 10.0, 10.0]);
}

#[test]
fn distribute_extra_is_noop_when_current_already_meets_target() {
    let mut widths = [50.0, 50.0]; // sum 100.0, above the 80.0 target
    super::distribute_extra(&mut widths, None, 80.0);
    assert_eq!(widths, [50.0, 50.0]);
}

#[test]
fn distribute_extra_is_noop_on_empty_slice() {
    let mut widths: [f32; 0] = [];
    super::distribute_extra(&mut widths, None, 30.0);
    assert!(widths.is_empty());
}

#[test]
fn distribute_extra_skips_hidden_tracks() {
    let mut heights = [10.0, 0.0, 30.0];
    super::distribute_extra(&mut heights, Some(&[false, true, false]), 80.0);
    assert_eq!(heights, [20.0, 0.0, 60.0]);
    let mut heights = [0.0, 0.0, 0.0];
    super::distribute_extra(&mut heights, Some(&[true, false, false]), 30.0);
    assert_eq!(heights, [0.0, 15.0, 15.0]);
}

#[test]
fn distribute_extra_is_noop_when_every_track_is_hidden() {
    let mut heights = [0.0, 0.0];
    super::distribute_extra(&mut heights, Some(&[true, true]), 30.0);
    assert_eq!(heights, [0.0, 0.0]);
}

// -----------------------------------------------------------------
// resolve_fixed_column_widths — CSS 2.1 §17.5.2.1 fixed algorithm.
// -----------------------------------------------------------------

fn fixed_cell(row: u16, col_start: u16, col_span: u16, width: Dimension) -> super::CellPlacement {
    super::CellPlacement {
        node_id: 0,
        row,
        col_start,
        col_span,
        row_span: 1,
        specified_width: width,
        specified_min_width: Dimension::auto(),
        resolved: None,
        natural_height: 0.0,
        baseline: 0.0,
        row_pass: None,
    }
}

#[test]
fn resolve_fixed_column_widths_first_row_length_fixes_column_remainder_splits() {
    let grid = super::TableGrid {
        n_cols: 2,
        rows: vec![],
        cells: vec![
            fixed_cell(0, 0, 1, Dimension::length(100.0)),
            fixed_cell(0, 1, 1, Dimension::auto()),
        ],
        col_widths: vec![],
    };
    let widths = super::resolve_fixed_column_widths(&grid, 400.0);
    assert_eq!(widths, [100.0, 300.0]);
}

#[test]
fn fixed_auto_columns_keep_free_space_while_respecting_first_row_floors() {
    for (minimum, expected) in [
        (10.0, [25.0, 25.0]),
        (40.0, [40.0, 10.0]),
        (60.0, [60.0, 0.0]),
    ] {
        let mut first = fixed_cell(0, 0, 1, Dimension::auto());
        first.specified_min_width = Dimension::length(minimum);
        let grid = super::TableGrid {
            n_cols: 2,
            rows: vec![],
            cells: vec![first, fixed_cell(0, 1, 1, Dimension::auto())],
            col_widths: vec![],
        };
        assert_eq!(super::resolve_fixed_column_widths(&grid, 50.0), expected);
    }
}

#[test]
fn fixed_first_row_colspan_minimum_grows_its_columns() {
    let mut span = fixed_cell(0, 0, 2, Dimension::auto());
    span.specified_min_width = Dimension::length(60.0);
    let grid = super::TableGrid {
        n_cols: 2,
        rows: vec![],
        cells: vec![span],
        col_widths: vec![],
    };
    assert_eq!(
        super::resolve_fixed_column_widths(&grid, 40.0),
        [30.0, 30.0]
    );
}

#[test]
fn first_cell_baseline_descends_through_multiple_block_containers() {
    let mut doc = Document::new();
    crate::layout::test_support::with_ahem(&mut doc);
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block"));
    let table = doc.append_element(
        Some(body),
        "div",
        Some("display:table;width:20px;border-spacing:0"),
    );
    let row = doc.append_element(Some(table), "div", Some("display:table-row"));
    let cell = doc.append_element(Some(row), "div", Some("display:table-cell;vertical-align:top;font-family:Ahem;font-size:10px;line-height:10px"));
    let outer = doc.append_element(
        Some(cell),
        "div",
        Some("display:block;margin-top:3px;padding-top:2px"),
    );
    let inner = doc.append_element(
        Some(outer),
        "div",
        Some("display:block;margin-top:5px;padding-top:4px"),
    );
    doc.append_text(inner, "X");
    doc.append_text(cell, "later inline content");
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    let mut session = LayoutSession::with_cascade(&mut doc, &computed);
    crate::layout::page_pipeline::layout_single_page_in_session(
        &mut session,
        &computed,
        PageBox::A4,
    )
    .unwrap();
    assert_eq!(super::first_cell_baseline(&session, cell, 0.0), Some(22.0));
}

#[test]
fn table_oof_candidates_use_the_border_area() {
    let mut doc = Document::new();
    let table = doc.append_element(
        Some(0),
        "table",
        Some("display:table;border:2px solid;overflow-x:scroll;overflow-y:scroll"),
    );
    let fixed = doc.append_element(
        Some(table),
        "div",
        Some("position:fixed;width:10px;height:10px"),
    );
    let absolute = doc.append_element(
        Some(table),
        "div",
        Some("position:absolute;width:10px;height:10px"),
    );
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    crate::layout::apply_computed_to_style(&mut doc, &computed).unwrap();

    let output = with_layout_session(&mut doc, |session| {
        super::table_layout_output(
            session,
            table,
            Size {
                width: 100.0,
                height: 100.0,
            },
            OofCandidates::new(),
            RunMode::PerformLayout,
        )
    });

    let area = output.oof_positioning_area.unwrap();
    assert_eq!(
        area.size,
        Size {
            width: 96.0,
            height: 96.0
        }
    );
    assert_eq!(area.offset, Point { x: 2.0, y: 2.0 });
    let candidates = output.oof_candidates.to_vec();
    assert_eq!(candidates.len(), 2);
    assert_eq!(candidates[0].node, NodeId::from(fixed));
    assert_eq!(candidates[0].position, OofPosition::Fixed);
    assert_eq!(candidates[0].order, 0);
    assert_eq!(candidates[1].node, NodeId::from(absolute));
    assert_eq!(candidates[1].position, OofPosition::Absolute);
    assert_eq!(candidates[1].order, 1);
}

#[test]
fn in_flow_caption_bubbles_uncontained_fixed_candidates() {
    let mut doc = Document::new();
    let table = doc.append_element(
        Some(0),
        "table",
        Some("display:table;width:100px;height:50px"),
    );
    let caption = doc.append_element(
        Some(table),
        "caption",
        Some("display:table-caption;width:20px;height:10px;margin-left:17px"),
    );
    let fixed = doc.append_element(
        Some(caption),
        "div",
        Some("display:block;position:fixed;left:11px;top:13px;width:5px;height:6px"),
    );
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    crate::layout::apply_computed_to_style(&mut doc, &computed).unwrap();

    let mut inputs = column_probe_input();
    inputs.known_dimensions = Size {
        width: Some(100.0),
        height: Some(50.0),
    };
    inputs.known_dimensions_are_definite = Size {
        width: true,
        height: true,
    };
    inputs.parent_size = Size {
        width: Some(100.0),
        height: Some(100.0),
    };
    inputs.available_space = Size {
        width: AvailableSpace::Definite(100.0),
        height: AvailableSpace::Definite(100.0),
    };
    let output = with_layout_session(&mut doc, |session| {
        compute_table_layout_in_session(session, NodeId::from(table), inputs)
    });

    let candidate = output
        .oof_candidates
        .iter()
        .find(|candidate| candidate.node == NodeId::from(fixed))
        .expect("fixed descendant bubbles out of the in-flow caption");
    assert_eq!(candidate.static_position.x.area.start, 17.0);
}

#[test]
fn resolve_fixed_column_widths_first_row_percent_resolves_against_avail() {
    let grid = super::TableGrid {
        n_cols: 2,
        rows: vec![],
        cells: vec![
            fixed_cell(0, 0, 1, Dimension::percent(0.25)),
            fixed_cell(0, 1, 1, Dimension::auto()),
        ],
        col_widths: vec![],
    };
    let widths = super::resolve_fixed_column_widths(&grid, 400.0);
    assert_eq!(widths, [100.0, 300.0]);
}

#[test]
fn resolve_fixed_column_widths_ignores_non_first_row_width() {
    // A length on a *second*-row cell must not fix its column.
    let grid = super::TableGrid {
        n_cols: 2,
        rows: vec![],
        cells: vec![
            fixed_cell(0, 0, 1, Dimension::auto()),
            fixed_cell(0, 1, 1, Dimension::auto()),
            fixed_cell(1, 0, 1, Dimension::length(300.0)),
        ],
        col_widths: vec![],
    };
    let widths = super::resolve_fixed_column_widths(&grid, 400.0);
    assert_eq!(widths, [200.0, 200.0]);
}

#[test]
fn resolve_fixed_column_widths_ignores_colspan_first_row_width() {
    // A first-row cell with colspan > 1 never fixes a column — only
    // colspan == 1 first-row cells participate in §17.5.2.1 fixing.
    let grid = super::TableGrid {
        n_cols: 2,
        rows: vec![],
        cells: vec![fixed_cell(0, 0, 2, Dimension::length(300.0))],
        col_widths: vec![],
    };
    let widths = super::resolve_fixed_column_widths(&grid, 400.0);
    assert_eq!(widths, [200.0, 200.0]);
}

#[test]
fn resolve_fixed_column_widths_col_element_length_fixes_unspecified_column() {
    // No first-row cell width at all: a `<col>` length still fixes
    // the column (the §17.5.2.1 `<col>` fallback).
    let grid = super::TableGrid {
        n_cols: 2,
        rows: vec![],
        cells: vec![
            fixed_cell(0, 0, 1, Dimension::auto()),
            fixed_cell(0, 1, 1, Dimension::auto()),
        ],
        col_widths: vec![
            super::ColSizing {
                width: Dimension::length(60.0),
                min_width: Dimension::auto(),
                max_width: Dimension::auto(),
            },
            super::ColSizing {
                width: Dimension::auto(),
                min_width: Dimension::auto(),
                max_width: Dimension::auto(),
            },
        ],
    };
    let widths = super::resolve_fixed_column_widths(&grid, 400.0);
    assert_eq!(widths, [60.0, 340.0]);
}

#[test]
fn resolve_fixed_column_widths_col_element_takes_priority_over_first_row_cell() {
    let grid = super::TableGrid {
        n_cols: 2,
        rows: vec![],
        cells: vec![
            fixed_cell(0, 0, 1, Dimension::length(100.0)),
            fixed_cell(0, 1, 1, Dimension::auto()),
        ],
        col_widths: vec![
            super::ColSizing {
                width: Dimension::length(50.0),
                min_width: Dimension::auto(),
                max_width: Dimension::auto(),
            },
            super::ColSizing {
                width: Dimension::auto(),
                min_width: Dimension::auto(),
                max_width: Dimension::auto(),
            },
        ],
    };

    assert_eq!(
        super::resolve_fixed_column_widths(&grid, 400.0),
        [50.0, 350.0]
    );
}

#[test]
fn resolve_fixed_column_widths_col_element_percent_resolves_against_avail() {
    let grid = super::TableGrid {
        n_cols: 2,
        rows: vec![],
        cells: vec![],
        col_widths: vec![
            super::ColSizing {
                width: Dimension::percent(0.5),
                min_width: Dimension::auto(),
                max_width: Dimension::auto(),
            },
            super::ColSizing {
                width: Dimension::auto(),
                min_width: Dimension::auto(),
                max_width: Dimension::auto(),
            },
        ],
    };
    let widths = super::resolve_fixed_column_widths(&grid, 400.0);
    assert_eq!(widths, [200.0, 200.0]);
}

#[test]
fn resolve_fixed_column_widths_col_element_min_only_floors_column() {
    // A `<col>` with no width but a length min-width still fixes the
    // column (`v.is_none() && is_len(min_width)` fallback).
    let grid = super::TableGrid {
        n_cols: 2,
        rows: vec![],
        cells: vec![],
        col_widths: vec![
            super::ColSizing {
                width: Dimension::auto(),
                min_width: Dimension::length(50.0),
                max_width: Dimension::auto(),
            },
            super::ColSizing {
                width: Dimension::auto(),
                min_width: Dimension::auto(),
                max_width: Dimension::auto(),
            },
        ],
    };
    let widths = super::resolve_fixed_column_widths(&grid, 400.0);
    assert_eq!(widths, [50.0, 350.0]);
}

#[test]
fn resolve_fixed_column_widths_col_element_width_clamped_by_max_width() {
    let grid = super::TableGrid {
        n_cols: 2,
        rows: vec![],
        cells: vec![],
        col_widths: vec![
            super::ColSizing {
                width: Dimension::length(100.0),
                min_width: Dimension::auto(),
                max_width: Dimension::length(60.0),
            },
            super::ColSizing {
                width: Dimension::auto(),
                min_width: Dimension::auto(),
                max_width: Dimension::auto(),
            },
        ],
    };
    let widths = super::resolve_fixed_column_widths(&grid, 400.0);
    assert_eq!(widths, [60.0, 340.0]);
}

#[test]
fn resolve_fixed_column_widths_underflow_clamps_unfixed_columns_to_zero() {
    // The single fixed column already exceeds avail: the leftover
    // for the unfixed column clamps to zero rather than going
    // negative — the table overflows instead of shrinking it.
    let grid = super::TableGrid {
        n_cols: 2,
        rows: vec![],
        cells: vec![fixed_cell(0, 0, 1, Dimension::length(500.0))],
        col_widths: vec![],
    };
    let widths = super::resolve_fixed_column_widths(&grid, 400.0);
    assert_eq!(widths, [500.0, 0.0]);
}

// -----------------------------------------------------------------
// collect_col_widths / collect_rows_inner / flush_pending — grid
// construction helpers, called directly on a minimal html/body/table
// shell.
// -----------------------------------------------------------------

fn table_shell() -> (Document, usize) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let table = doc.append_element(Some(body), "table", Some("display:table"));
    (doc, table)
}

#[test]
fn collect_col_widths_expands_span_and_reads_colgroup_and_bare_col() {
    let (mut doc, table) = table_shell();
    let colgroup = doc.append_element(Some(table), "colgroup", Some("display:table-column-group"));

    doc.append_element(
        Some(colgroup),
        "col",
        Some("display:table-column;width:40px"),
    );
    let col_b = doc.append_element(
        Some(colgroup),
        "col",
        Some("display:table-column;min-width:20px"),
    );
    doc.set_element_attributes(col_b, vec![("span".into(), "2".into())]);

    // A `<tr>` sibling of the colgroup must be skipped, not treated
    // as a column.
    doc.append_element(Some(table), "tr", Some("display:table-row"));

    // A bare `<col>` directly under the table (anonymous colgroup).
    doc.append_element(Some(table), "col", Some("display:table-column;width:25%"));

    doc.mark_in_document_flags();
    let sizing = with_layout_session(&mut doc, |session| {
        super::collect_col_widths(session, table)
    })
    .unwrap();

    assert_eq!(sizing.len(), 4, "col_a + col_b(span=2) + col_c = 4 entries");
    assert_eq!(sizing[0].width, Dimension::length(40.0));
    assert_eq!(sizing[1].min_width, Dimension::length(20.0));
    assert_eq!(
        sizing[2].min_width,
        Dimension::length(20.0),
        "span=2 expands to two identical entries"
    );
    assert_eq!(sizing[3].width, Dimension::percent(0.25));
}

#[test]
fn collect_rows_wraps_bare_cells_in_anonymous_row() {
    let (mut doc, table) = table_shell();
    doc.append_element(Some(table), "td", Some("display:table-cell"));
    doc.append_element(Some(table), "td", Some("display:table-cell"));
    doc.mark_in_document_flags();

    let mut rows = Vec::new();
    let mut cells = Vec::new();
    let mut n_cols = 0u16;
    with_layout_session(&mut doc, |session| {
        super::collect_rows(session, table, &mut rows, &mut cells, &mut n_cols)
    })
    .unwrap();

    assert_eq!(rows.len(), 1, "bare cells wrap into a single anonymous row");
    assert_eq!(cells.len(), 2);
    assert_eq!(cells[0].col_start, 0);
    assert_eq!(cells[1].col_start, 1);
    assert_eq!(n_cols, 2);
}

fn head_body_fixture() -> (Document, usize, usize, usize) {
    let (mut doc, table) = table_shell();
    // DOM order: tbody first, thead second — reorder must float the
    // header group up regardless.
    let tbody = doc.append_element(Some(table), "tbody", Some("display:table-row-group"));
    let tr_body = doc.append_element(Some(tbody), "tr", Some("display:table-row"));
    let body_cell = doc.append_element(Some(tr_body), "td", Some("display:table-cell"));

    let thead = doc.append_element(Some(table), "thead", Some("display:table-header-group"));
    let tr_head = doc.append_element(Some(thead), "tr", Some("display:table-row"));
    let head_cell = doc.append_element(Some(tr_head), "td", Some("display:table-cell"));

    doc.mark_in_document_flags();
    (doc, table, head_cell, body_cell)
}

#[test]
fn collect_rows_floats_first_header_group_to_top() {
    let (mut doc, table, head_cell, body_cell) = head_body_fixture();
    let mut rows = Vec::new();
    let mut cells = Vec::new();
    let mut n_cols = 0u16;
    with_layout_session(&mut doc, |session| {
        super::collect_rows(session, table, &mut rows, &mut cells, &mut n_cols)
    })
    .unwrap();

    assert_eq!(cells.len(), 2);
    assert_eq!(
        cells[0].node_id, head_cell,
        "thead row must come first despite DOM order"
    );
    assert_eq!(cells[1].node_id, body_cell);
}

#[test]
fn collect_rows_inner_no_reorder_keeps_dom_order_for_nested_call() {
    let (mut doc, table, head_cell, body_cell) = head_body_fixture();
    let mut rows = Vec::new();
    let mut cells = Vec::new();
    let mut n_cols = 0u16;
    with_layout_session(&mut doc, |session| {
        super::collect_rows_inner(session, table, &mut rows, &mut cells, &mut n_cols, false)
    })
    .unwrap();

    assert_eq!(cells.len(), 2);
    assert_eq!(
        cells[0].node_id, body_cell,
        "DOM order preserved: tbody before thead"
    );
    assert_eq!(cells[1].node_id, head_cell);
}

#[test]
fn collect_rows_inner_skips_character_data_between_rows() {
    let (mut doc, table) = table_shell();
    let tr1 = doc.append_element(Some(table), "tr", Some("display:table-row"));
    doc.append_element(Some(tr1), "td", Some("display:table-cell"));

    doc.append_text(table, "   ");

    let tr2 = doc.append_element(Some(table), "tr", Some("display:table-row"));
    doc.append_element(Some(tr2), "td", Some("display:table-cell"));

    doc.mark_in_document_flags();
    let mut rows = Vec::new();
    let mut cells = Vec::new();
    let mut n_cols = 0u16;
    with_layout_session(&mut doc, |session| {
        super::collect_rows(session, table, &mut rows, &mut cells, &mut n_cols)
    })
    .unwrap();

    assert_eq!(
        rows.len(),
        2,
        "inter-row whitespace must not create a spurious row"
    );
    assert_eq!(cells.len(), 2);
}

#[test]
fn collect_rows_inner_recurses_into_non_row_group_wrapper() {
    let (mut doc, table) = table_shell();
    // A plain wrapper (e.g. an authoring-error `<div>` directly under
    // `<table>`) is neither a row, a cell, nor a row-group — the
    // catch-all branch still recurses into it looking for rows.
    let wrapper = doc.append_element(Some(table), "div", None::<&str>);
    let tr = doc.append_element(Some(wrapper), "tr", Some("display:table-row"));
    let td = doc.append_element(Some(tr), "td", Some("display:table-cell"));

    doc.mark_in_document_flags();
    let mut rows = Vec::new();
    let mut cells = Vec::new();
    let mut n_cols = 0u16;
    with_layout_session(&mut doc, |session| {
        super::collect_rows(session, table, &mut rows, &mut cells, &mut n_cols)
    })
    .unwrap();

    assert_eq!(rows.len(), 1);
    assert_eq!(cells.len(), 1);
    assert_eq!(cells[0].node_id, td);
}

#[test]
fn flush_pending_assigns_sequential_columns_respecting_colspan() {
    let (mut doc, table) = table_shell();
    let td_a = doc.append_element(Some(table), "td", None::<&str>);
    doc.set_element_attributes(td_a, vec![("colspan".into(), "2".into())]);
    let td_b = doc.append_element(Some(table), "td", None::<&str>);
    doc.mark_in_document_flags();

    let mut pending = vec![td_a, td_b];
    let mut rows = Vec::new();
    let mut cells = Vec::new();
    let mut n_cols = 0u16;
    with_layout_session(&mut doc, |session| {
        super::flush_pending(session, &mut pending, &mut rows, &mut cells, &mut n_cols)
    })
    .unwrap();

    assert!(pending.is_empty());
    assert_eq!(rows, vec![td_a]);
    assert_eq!(cells.len(), 2);
    assert_eq!((cells[0].col_start, cells[0].col_span), (0, 2));
    assert_eq!((cells[1].col_start, cells[1].col_span), (2, 1));
    assert_eq!(n_cols, 3);
}

#[test]
fn flush_pending_on_empty_pending_is_noop() {
    let mut doc = Document::new();
    let mut pending: Vec<usize> = Vec::new();
    let mut rows = Vec::new();
    let mut cells = Vec::new();
    let mut n_cols = 0u16;
    with_layout_session(&mut doc, |session| {
        super::flush_pending(session, &mut pending, &mut rows, &mut cells, &mut n_cols)
    })
    .unwrap();
    assert!(rows.is_empty());
    assert!(cells.is_empty());
    assert_eq!(n_cols, 0);
}

// -----------------------------------------------------------------
// compute_collapsed_lines — CSS 2.1 §17.6.2 simplified max-width
// conflict resolution (max of adjoining borders; absorbed overlap
// is the pairwise min).
// -----------------------------------------------------------------

fn make_cell(
    node_id: usize,
    row: u16,
    col_start: u16,
    col_span: u16,
    row_span: u16,
    width: Dimension,
) -> super::CellPlacement {
    super::CellPlacement {
        node_id,
        row,
        col_start,
        col_span,
        row_span,
        specified_width: width,
        specified_min_width: Dimension::auto(),
        resolved: None,
        natural_height: 0.0,
        baseline: 0.0,
        row_pass: None,
    }
}

/// A table cell with a solid border of `px` on every side.
///
/// The cell hangs off the document root, as only boxes in the tree take part
/// in the cascade.
fn bordered_cell(doc: &mut Document, px: f32) -> usize {
    doc.append_element(
        Some(0),
        "td",
        Some(format!("display:table-cell;border:{px}px solid")),
    )
}

#[test]
fn compute_collapsed_lines_resolves_2x2_grid_overlaps_and_outers() {
    let mut doc = Document::new();
    let c00 = bordered_cell(&mut doc, 4.0);
    let c01 = bordered_cell(&mut doc, 2.0);
    let c10 = bordered_cell(&mut doc, 6.0);
    let c11 = bordered_cell(&mut doc, 3.0);
    let grid = super::TableGrid {
        n_cols: 2,
        // Only the row *count* matters here (`compute_collapsed_lines`
        // reads `grid.rows.len()` for its boundary loop bounds; row
        // node ids are never dereferenced), so these are placeholders.
        rows: vec![0, 0],
        cells: vec![
            make_cell(c00, 0, 0, 1, 1, Dimension::auto()),
            make_cell(c01, 0, 1, 1, 1, Dimension::auto()),
            make_cell(c10, 1, 0, 1, 1, Dimension::auto()),
            make_cell(c11, 1, 1, 1, 1, Dimension::auto()),
        ],
        col_widths: vec![],
    };
    let table_border = Edges {
        left: 1.0,
        right: 1.0,
        top: 1.0,
        bottom: 1.0,
    };
    let lines = compute_collapsed_lines_in_document(
        &mut doc,
        &grid,
        &super::CellIndex::new(&grid),
        &table_border,
        None,
    );

    assert_eq!(
        lines.col_overlaps,
        vec![3.0],
        "row0 min(4,2)=2 vs row1 min(6,3)=3 -> max 3"
    );
    assert_eq!(
        lines.row_overlaps,
        vec![4.0],
        "col0 min(4,6)=4 vs col1 min(2,3)=2 -> max 4"
    );
    assert_eq!(lines.outer_left, 6.0);
    assert_eq!(lines.outer_right, 3.0);
    assert_eq!(lines.outer_top, 4.0);
    assert_eq!(lines.outer_bottom, 6.0);
}

#[test]
fn compute_collapsed_lines_colspan_cell_only_absorbs_from_its_own_row() {
    // 3 columns x 2 rows: row 0 has one cell spanning columns 0-1
    // (colspan=2, border 6) plus a column-2 cell (border 2); row 1
    // has three ordinary cells (borders 4, 1, 1). The interior
    // boundary under the span (boundary 0) has no adjoining pair in
    // row 0 (both sides belong to the same cell) so it comes only
    // from row 1; boundary 1 sees the span's own right edge in row 0.
    let mut doc = Document::new();
    let span = bordered_cell(&mut doc, 6.0);
    let r0c2 = bordered_cell(&mut doc, 2.0);
    let r1c0 = bordered_cell(&mut doc, 4.0);
    let r1c1 = bordered_cell(&mut doc, 1.0);
    let r1c2 = bordered_cell(&mut doc, 1.0);
    let grid = super::TableGrid {
        n_cols: 3,
        rows: vec![0, 0], // row count only — see note above.
        cells: vec![
            make_cell(span, 0, 0, 2, 1, Dimension::auto()),
            make_cell(r0c2, 0, 2, 1, 1, Dimension::auto()),
            make_cell(r1c0, 1, 0, 1, 1, Dimension::auto()),
            make_cell(r1c1, 1, 1, 1, 1, Dimension::auto()),
            make_cell(r1c2, 1, 2, 1, 1, Dimension::auto()),
        ],
        col_widths: vec![],
    };
    let lines = compute_collapsed_lines_in_document(
        &mut doc,
        &grid,
        &super::CellIndex::new(&grid),
        &Edges::ZERO,
        None,
    );

    assert_eq!(
        lines.col_overlaps,
        vec![1.0, 2.0],
        "boundary 0 comes only from row 1 (min(4,1)=1); boundary 1 comes from \
         the wider row-0 span edge (min(6,2)=2) over row 1's min(1,1)=1"
    );
}

// -----------------------------------------------------------------
// resolve_column_widths — auto column-sizing aggregation (colspan
// excess distribution, `<col>` authored floors, percent handling).
// -----------------------------------------------------------------

fn column_probe_input() -> LayoutInput {
    LayoutInput {
        run_mode: RunMode::PerformLayout,
        sizing_mode: SizingMode::InherentSize,
        axis: RequestedAxis::Horizontal,
        known_dimensions: Size {
            width: None,
            height: None,
        },
        parent_size: Size {
            width: None,
            height: None,
        },
        available_space: Size {
            width: AvailableSpace::MaxContent,
            height: AvailableSpace::MaxContent,
        },
        known_dimensions_are_definite: Size {
            width: false,
            height: false,
        },
        vertical_margins_are_collapsible: Line::FALSE,
    }
}

#[test]
fn resolve_column_widths_colspan_one_percent_cell_sets_column_percentage() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let pct_cell = doc.append_element(Some(html), "td", None::<&str>);
    let auto_cell = doc.append_element(Some(html), "td", None::<&str>);
    doc.mark_in_document_flags();

    let grid = super::TableGrid {
        n_cols: 2,
        rows: vec![],
        cells: vec![
            make_cell(pct_cell, 0, 0, 1, 1, Dimension::percent(0.4)),
            make_cell(auto_cell, 0, 1, 1, 1, Dimension::auto()),
        ],
        col_widths: vec![],
    };
    let inputs = LayoutInput {
        known_dimensions: Size {
            width: Some(200.0),
            height: None,
        },
        ..column_probe_input()
    };
    let widths = resolve_column_widths_in_session(
        &mut doc,
        &grid,
        inputs,
        Size {
            width: 0.0,
            height: 0.0,
        },
        0.0,
    );
    assert!(
        (widths[0] - 80.0).abs() < 0.5,
        "40% of 200 -> 80: {widths:?}"
    );
}

#[test]
fn resolve_column_widths_col_percent_sets_percentage_of_occupied_column() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let first = doc.append_element(Some(html), "td", None::<&str>);
    let second = doc.append_element(Some(html), "td", None::<&str>);
    doc.mark_in_document_flags();

    let grid = super::TableGrid {
        n_cols: 2,
        rows: vec![],
        cells: vec![
            make_cell(first, 0, 0, 1, 1, Dimension::auto()),
            make_cell(second, 0, 1, 1, 1, Dimension::auto()),
        ],
        col_widths: vec![
            super::ColSizing {
                width: Dimension::percent(0.4),
                min_width: Dimension::auto(),
                max_width: Dimension::auto(),
            },
            super::ColSizing {
                width: Dimension::auto(),
                min_width: Dimension::auto(),
                max_width: Dimension::auto(),
            },
        ],
    };
    let inputs = LayoutInput {
        known_dimensions: Size {
            width: Some(200.0),
            height: None,
        },
        ..column_probe_input()
    };
    let widths = resolve_column_widths_in_session(
        &mut doc,
        &grid,
        inputs,
        Size {
            width: 0.0,
            height: 0.0,
        },
        0.0,
    );

    // The `<col>` percentage drives its column like a cell percentage does,
    // where two empty auto columns would share the 200px equally.
    assert!((widths[0] - 80.0).abs() < 0.5, "40% of 200: {widths:?}");
    assert!((widths[1] - 120.0).abs() < 0.5, "the rest: {widths:?}");
}

#[test]
fn resolve_column_widths_colspan_excess_distributes_min_and_max_across_targets() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let span_cell = doc.append_element(Some(html), "td", None::<&str>);
    doc.append_element(
        Some(span_cell),
        "div",
        Some("display:flex;box-sizing:border-box;width:100px;height:10px"),
    );
    let c0 = doc.append_element(Some(html), "td", None::<&str>);
    let c1 = doc.append_element(Some(html), "td", None::<&str>);
    doc.mark_in_document_flags();

    let grid = super::TableGrid {
        n_cols: 2,
        rows: vec![],
        cells: vec![
            make_cell(span_cell, 0, 0, 2, 1, Dimension::auto()),
            make_cell(c0, 1, 0, 1, 1, Dimension::auto()),
            make_cell(c1, 1, 1, 1, 1, Dimension::auto()),
        ],
        col_widths: vec![],
    };
    let inputs = LayoutInput {
        known_dimensions: Size {
            width: Some(300.0),
            height: None,
        },
        ..column_probe_input()
    };
    let widths = resolve_column_widths_in_session(
        &mut doc,
        &grid,
        inputs,
        Size {
            width: 0.0,
            height: 0.0,
        },
        0.0,
    );

    assert!((widths[0] - 150.0).abs() < 1.0, "widths: {widths:?}");
    assert!((widths[1] - 150.0).abs() < 1.0, "widths: {widths:?}");
    assert!(
        (widths[0] - widths[1]).abs() < 0.01,
        "a colspan cell's min/max excess splits evenly across its non-authored targets"
    );
}

#[test]
fn resolve_column_widths_definite_avail_nonfit_keeps_colspan_minimum() {
    // None + Definite(50) with a colspan=2 cell forcing 100px: the
    // max-full sum (100) exceeds avail (50), so the non-fit path runs.
    // It must keep the colspan-derived minimum (50 per column), matching
    // the known-width branch, not shrink to 25 per column.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let span_cell = doc.append_element(Some(html), "td", None::<&str>);
    doc.append_element(
        Some(span_cell),
        "div",
        Some("display:flex;box-sizing:border-box;width:100px;height:10px"),
    );
    let c0 = doc.append_element(Some(html), "td", None::<&str>);
    let c1 = doc.append_element(Some(html), "td", None::<&str>);
    doc.mark_in_document_flags();

    let grid = super::TableGrid {
        n_cols: 2,
        rows: vec![],
        cells: vec![
            make_cell(span_cell, 0, 0, 2, 1, Dimension::auto()),
            make_cell(c0, 1, 0, 1, 1, Dimension::auto()),
            make_cell(c1, 1, 1, 1, 1, Dimension::auto()),
        ],
        col_widths: vec![],
    };
    let inputs = LayoutInput {
        known_dimensions: Size {
            width: None,
            height: None,
        },
        available_space: Size {
            width: AvailableSpace::Definite(50.0),
            height: AvailableSpace::MaxContent,
        },
        ..column_probe_input()
    };
    let widths = resolve_column_widths_in_session(
        &mut doc,
        &grid,
        inputs,
        Size {
            width: 0.0,
            height: 0.0,
        },
        0.0,
    );

    assert!((widths[0] - 50.0).abs() < 1.0, "widths: {widths:?}");
    assert!((widths[1] - 50.0).abs() < 1.0, "widths: {widths:?}");
    assert!(
        widths.iter().sum::<f32>() >= 100.0 - 1.0,
        "colspan minimum must hold even when avail is smaller: {widths:?}"
    );
}

#[test]
fn resolve_column_widths_col_percent_ignored_for_unoccupied_column() {
    let mut doc = Document::new();
    let grid = super::TableGrid {
        n_cols: 2,
        rows: vec![], // no cells at all: column 1 is unoccupied
        cells: vec![],
        col_widths: vec![
            super::ColSizing {
                width: Dimension::length(60.0),
                min_width: Dimension::auto(),
                max_width: Dimension::auto(),
            },
            super::ColSizing {
                width: Dimension::percent(0.25),
                min_width: Dimension::auto(),
                max_width: Dimension::auto(),
            },
        ],
    };
    let inputs = LayoutInput {
        known_dimensions: Size {
            width: Some(200.0),
            height: None,
        },
        ..column_probe_input()
    };
    let widths = resolve_column_widths_in_session(
        &mut doc,
        &grid,
        inputs,
        Size {
            width: 0.0,
            height: 0.0,
        },
        0.0,
    );

    // If the 25% were (incorrectly) applied despite no cell occupying
    // the column, column 1 would be pct-driven and would stop
    // receiving a share of the leftover space; instead it behaves as
    // a plain auto column here.
    assert!((widths[0] - 130.0).abs() < 1.0, "widths: {widths:?}");
    assert!((widths[1] - 70.0).abs() < 1.0, "widths: {widths:?}");
}

#[test]
fn resolve_column_widths_col_length_floors_even_unoccupied_column() {
    let mut doc = Document::new();
    let grid = super::TableGrid {
        n_cols: 2,
        rows: vec![],
        cells: vec![],
        col_widths: vec![
            super::ColSizing {
                width: Dimension::length(90.0),
                min_width: Dimension::auto(),
                max_width: Dimension::auto(),
            },
            super::ColSizing {
                width: Dimension::auto(),
                min_width: Dimension::auto(),
                max_width: Dimension::auto(),
            },
        ],
    };
    let inputs = LayoutInput {
        known_dimensions: Size {
            width: Some(200.0),
            height: None,
        },
        ..column_probe_input()
    };
    let widths = resolve_column_widths_in_session(
        &mut doc,
        &grid,
        inputs,
        Size {
            width: 0.0,
            height: 0.0,
        },
        0.0,
    );

    assert!(widths[0] > widths[1], "widths: {widths:?}");
    assert!((widths[0] - 145.0).abs() < 1.0, "widths: {widths:?}");
    assert!((widths[1] - 55.0).abs() < 1.0, "widths: {widths:?}");
}

#[test]
fn resolve_column_widths_col_min_over_max_resolves_to_min() {
    // CSS Sizing 3 §4/§5: min > max resolves to min (max is
    // ignored), even though a base `width` is also specified.
    let mut doc = Document::new();
    let grid = super::TableGrid {
        n_cols: 2,
        rows: vec![],
        cells: vec![],
        col_widths: vec![
            super::ColSizing {
                width: Dimension::length(50.0),
                min_width: Dimension::length(80.0),
                max_width: Dimension::length(30.0),
            },
            super::ColSizing {
                width: Dimension::auto(),
                min_width: Dimension::auto(),
                max_width: Dimension::auto(),
            },
        ],
    };
    let inputs = LayoutInput {
        known_dimensions: Size {
            width: Some(80.0),
            height: None,
        },
        ..column_probe_input()
    };
    let widths = resolve_column_widths_in_session(
        &mut doc,
        &grid,
        inputs,
        Size {
            width: 0.0,
            height: 0.0,
        },
        0.0,
    );

    assert_eq!(widths, [80.0, 0.0]);
}

#[test]
fn resolve_column_widths_col_width_clamped_by_max_width() {
    let mut doc = Document::new();
    let grid = super::TableGrid {
        n_cols: 2,
        rows: vec![],
        cells: vec![],
        col_widths: vec![
            super::ColSizing {
                width: Dimension::length(100.0),
                min_width: Dimension::auto(),
                max_width: Dimension::length(60.0),
            },
            super::ColSizing {
                width: Dimension::auto(),
                min_width: Dimension::auto(),
                max_width: Dimension::auto(),
            },
        ],
    };
    let inputs = LayoutInput {
        known_dimensions: Size {
            width: Some(60.0),
            height: None,
        },
        ..column_probe_input()
    };
    let widths = resolve_column_widths_in_session(
        &mut doc,
        &grid,
        inputs,
        Size {
            width: 0.0,
            height: 0.0,
        },
        0.0,
    );

    assert_eq!(widths, [60.0, 0.0]);
}

// -----------------------------------------------------------------
// compute_table_layout — higher-level integration coverage for
// branches not exercised by the fixtures above.
// -----------------------------------------------------------------

#[test]
fn compute_table_layout_empty_table_as_flex_item_uses_container_width() {
    // An empty table (no rows at all) inside a flex container must
    // fill the flex-resolved main size rather than shrink-wrapping
    // to its padding/border (the `parent_is_flex_or_grid` branch).
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let flex = doc.append_element(Some(body), "div", Some("display: flex; width: 300px"));
    let table = doc.append_element(Some(flex), "table", Some("display: table; flex-grow: 1"));
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).unwrap();
    crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4).unwrap();

    let table_layout = doc.nodes[table].layout;
    assert!(
        table_layout.size.width >= 299.0,
        "empty table as sole flex item should fill the 300px flex container, got {}",
        table_layout.size.width
    );
}

#[test]
fn compute_table_layout_max_width_never_shrinks_below_intrinsic_content() {
    // csswg-drafts#5336 / Mozilla bug 1651530: max-width never
    // shrinks a table below its intrinsic content width — only
    // min-width grows it. A max-width smaller than the natural
    // content width is therefore a no-op.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let table = doc.append_element(Some(body), "table", Some("display: table; max-width: 60px"));
    let tr = doc.append_element(Some(table), "tr", Some("display: table-row"));
    let td1 = doc.append_element(Some(tr), "td", Some("display: table-cell"));
    doc.append_element(Some(td1), "div", Some("width: 100px; height: 10px"));
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).unwrap();
    crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4).unwrap();

    let table_layout = doc.nodes[table].layout;
    assert!(
        table_layout.size.width >= 99.0,
        "max-width below intrinsic content must not shrink the table, got {}",
        table_layout.size.width
    );
}

#[test]
fn compute_table_layout_single_column_separate_border_spacing_adds_both_gaps() {
    // CSS 2.2 §17.6.1: the spacing separates the outer cell from the
    // table edge on both sides, so a single-column table is the cell plus
    // two gaps wide, with the cell inset by one gap.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let table = doc.append_element(
        Some(body),
        "table",
        Some("display: table; border-spacing: 10px"),
    );
    let tr = doc.append_element(Some(table), "tr", Some("display: table-row"));
    let td = doc.append_element(Some(tr), "td", Some("display: table-cell"));
    doc.append_element(Some(td), "div", Some("width: 40px; height: 10px"));
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).unwrap();
    crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4).unwrap();

    let td_layout = doc.nodes[td].layout;
    assert_eq!(td_layout.size.width, 40.0);
    assert_eq!(td_layout.location.x, 10.0);
    assert_eq!(td_layout.location.y, 10.0);
    let table_layout = doc.nodes[table].layout;
    assert_eq!(table_layout.size.width, 60.0);
    assert_eq!(table_layout.size.height, 30.0);
}

// -----------------------------------------------------------------
// get_colspan — WHATWG HTML 4.9.12.1 ("Forming a table", "Cells" step):
// missing / unparseable / zero defaults to 1, values above 1000 clamp
// to 1000. Regression: values above u16::MAX (e.g. "100000") used to
// fail `parse::<u16>` and fall back to 1 instead of clamping to 1000.
// -----------------------------------------------------------------

fn colspan_of(value: Option<&str>) -> u16 {
    let mut doc = Document::new();
    let td = doc.append_element(None, "td", None::<&str>);
    if let Some(v) = value {
        doc.set_element_attributes(td, vec![("colspan".into(), v.into())]);
    }
    super::get_colspan(&doc, td)
}

#[test]
fn get_colspan_normal_value_passes_through() {
    assert_eq!(colspan_of(Some("2")), 2);
    assert_eq!(colspan_of(Some("1000")), 1000);
}

#[test]
fn get_colspan_missing_zero_and_invalid_default_to_one() {
    assert_eq!(colspan_of(None), 1);
    assert_eq!(colspan_of(Some("0")), 1);
    assert_eq!(colspan_of(Some("abc")), 1);
}

#[test]
fn get_colspan_clamps_values_above_1000() {
    assert_eq!(colspan_of(Some("1001")), 1000);
    assert_eq!(colspan_of(Some("65535")), 1000);
}

#[test]
fn get_colspan_huge_value_clamps_to_1000_not_one() {
    // "100000" overflows u16: the old `parse::<u16>` failed and fell
    // back to 1; parsing wide (u32) clamps it to 1000 per spec.
    assert_eq!(colspan_of(Some("70000")), 1000);
    assert_eq!(colspan_of(Some("100000")), 1000);
}

#[test]
fn fixed_auto_width_shrink_wraps_to_content() {
    // CSS 2.1 §17.5.2.1: a fixed-layout table with `width: auto` uses the
    // automatic layout algorithm; §17.5.2: such a table does not fill its
    // containing block. One 50px cell must shrink-wrap, not fill A4.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let table = doc.append_element(
        Some(body),
        "table",
        Some("display: table; table-layout: fixed"),
    );
    let tr = doc.append_element(Some(table), "tr", Some("display: table-row"));
    let td = doc.append_element(Some(tr), "td", Some("display: table-cell"));
    doc.append_element(Some(td), "div", Some("width: 50px; height: 10px"));
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).unwrap();
    crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4).unwrap();
    let table_layout = doc.nodes[table].layout;
    assert!(
        (table_layout.size.width - 50.0).abs() < 2.0,
        "fixed+auto table should shrink-wrap to 50px content, got {}",
        table_layout.size.width
    );
}

#[test]
fn fixed_specified_width_fills_to_specified_width() {
    // CSS 2.1 §17.5.2.1: a fixed-layout table with an explicit width uses
    // that width as the distribution basis. Guards on the auto fallback
    // must not change this path.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let table = doc.append_element(
        Some(body),
        "table",
        Some("display: table; table-layout: fixed; width: 400px"),
    );
    let tr = doc.append_element(Some(table), "tr", Some("display: table-row"));
    let td = doc.append_element(Some(tr), "td", Some("display: table-cell"));
    doc.append_element(Some(td), "div", Some("width: 50px; height: 10px"));
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).unwrap();
    crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4).unwrap();
    let table_layout = doc.nodes[table].layout;
    assert!(
        (table_layout.size.width - 400.0).abs() < 2.0,
        "fixed+400px table should fill to 400px, got {}",
        table_layout.size.width
    );
}

#[test]
fn auto_width_shrink_wraps_to_content_baseline() {
    // Baseline: auto layout with no specified width shrink-wraps
    // (CSS 2.1 §17.5.2.2). Fixed+auto must match this after the fix.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let table = doc.append_element(
        Some(body),
        "table",
        Some("display: table; table-layout: auto"),
    );
    let tr = doc.append_element(Some(table), "tr", Some("display: table-row"));
    let td = doc.append_element(Some(tr), "td", Some("display: table-cell"));
    doc.append_element(Some(td), "div", Some("width: 50px; height: 10px"));
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).unwrap();
    crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4).unwrap();
    let table_layout = doc.nodes[table].layout;
    assert!(
        (table_layout.size.width - 50.0).abs() < 2.0,
        "auto+auto table should shrink-wrap to 50px content, got {}",
        table_layout.size.width
    );
}

/// Appends a `display: table-cell` with one block child of the given size.
fn sized_cell(doc: &mut Document, row: usize, width: f32, height: f32) -> usize {
    let td = doc.append_element(Some(row), "td", Some("display: table-cell"));
    doc.append_element(
        Some(td),
        "div",
        Some(format!("width: {width}px; height: {height}px").as_str()),
    );
    td
}

#[test]
fn separate_border_spacing_gaps_columns_rows_and_spanned_cells() {
    // CSS 2.2 §17.6.1: n tracks take n + 1 gaps, and a spanning cell
    // covers the gaps between the tracks it spans.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let table = doc.append_element(
        Some(body),
        "table",
        Some("display: table; border-spacing: 5px"),
    );
    let row =
        |doc: &mut Document| doc.append_element(Some(table), "tr", Some("display: table-row"));
    let tr1 = row(&mut doc);
    let wide = sized_cell(&mut doc, tr1, 10.0, 10.0);
    doc.set_element_attributes(wide, vec![("colspan".into(), "2".into())]);
    let tr2 = row(&mut doc);
    let a = sized_cell(&mut doc, tr2, 20.0, 10.0);
    let b = sized_cell(&mut doc, tr2, 30.0, 10.0);
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).unwrap();
    crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4).unwrap();

    let at = |id: usize| {
        let l = doc.nodes[id].layout;
        (l.location.x, l.location.y, l.size.width, l.size.height)
    };
    assert_eq!(at(wide), (5.0, 5.0, 55.0, 10.0));
    assert_eq!(at(a), (5.0, 20.0, 20.0, 10.0));
    assert_eq!(at(b), (30.0, 20.0, 30.0, 10.0));
    let t = doc.nodes[table].layout.size;
    assert_eq!((t.width, t.height), (65.0, 35.0));
}

#[test]
fn separate_border_spacing_counts_spanned_gaps_toward_a_rowspan_height() {
    // A 40px cell spanning two rows already gets the 5px gap between
    // them, so the two rows only need to share the remaining 35px.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let table = doc.append_element(
        Some(body),
        "table",
        Some("display: table; border-spacing: 5px"),
    );
    let row =
        |doc: &mut Document| doc.append_element(Some(table), "tr", Some("display: table-row"));
    // The spanning cell is the last column, so the second row's cell
    // takes the first column without depending on slot reservation.
    let tr1 = row(&mut doc);
    let top = sized_cell(&mut doc, tr1, 10.0, 10.0);
    let tall = sized_cell(&mut doc, tr1, 10.0, 40.0);
    doc.set_element_attributes(tall, vec![("rowspan".into(), "2".into())]);
    let tr2 = row(&mut doc);
    let bottom = sized_cell(&mut doc, tr2, 10.0, 10.0);
    // Keep this spacing contract independent of empty-cell baseline alignment.
    for (cell, height) in [(top, 10), (tall, 40), (bottom, 10)] {
        doc.set_element_inline_style(
            cell,
            Some(
                format!("display:table-cell;width:10px;height:{height}px;vertical-align:top")
                    .into(),
            ),
        );
    }
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).unwrap();
    crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4).unwrap();

    let at = |id: usize| {
        let l = doc.nodes[id].layout;
        (l.location.x, l.location.y, l.size.width, l.size.height)
    };
    let (top, tall, bottom) = (at(top), at(tall), at(bottom));
    assert_eq!((top.0, top.1), (5.0, 5.0));
    assert_eq!((tall.0, tall.1, tall.3), (20.0, 5.0, 40.0));
    assert_eq!((bottom.0, bottom.1), (5.0, top.1 + top.3 + 5.0));
    assert_eq!(top.3 + 5.0 + bottom.3, 40.0);
    let t = doc.nodes[table].layout.size;
    assert_eq!((t.width, t.height), (35.0, 50.0));
}

#[test]
fn separate_border_spacing_widens_a_table_narrower_than_its_columns() {
    // CSS 2.1 §17.5.2: the table is at least its columns plus the spacing,
    // so the cell keeps its gap on both sides instead of overflowing.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let table = doc.append_element(
        Some(body),
        "table",
        Some("display: table; border-spacing: 2px; width: 10px"),
    );
    let tr = doc.append_element(Some(table), "tr", Some("display: table-row"));
    let td = sized_cell(&mut doc, tr, 10.0, 10.0);
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).unwrap();
    crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4).unwrap();

    let cell = doc.nodes[td].layout;
    assert_eq!((cell.location.x, cell.size.width), (2.0, 10.0));
    assert_eq!(doc.nodes[table].layout.size.width, 14.0);
}

#[test]
fn separate_border_spacing_gaps_the_stacked_tracks_of_a_vertical_table() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let table = doc.append_element(
        Some(body),
        "table",
        Some("display: table; writing-mode: vertical-lr; border-spacing: 3px 5px"),
    );
    let tr = doc.append_element(Some(table), "tr", Some("display: table-row"));
    let first = sized_cell(&mut doc, tr, 50.0, 100.0);
    let second = sized_cell(&mut doc, tr, 50.0, 100.0);
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).unwrap();
    crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4).unwrap();

    let first = doc.nodes[first].layout;
    let second = doc.nodes[second].layout;
    let table = doc.nodes[table].layout.size;
    assert_eq!((first.location.x, first.location.y), (3.0, 5.0));
    assert_eq!(
        (second.location.x, second.location.y),
        (3.0, 5.0 + first.size.height + 5.0)
    );
    assert_eq!(
        table.height,
        5.0 + first.size.height + 5.0 + second.size.height + 5.0
    );
    assert_eq!(table.width, 3.0 + first.size.width + 3.0);
}

#[test]
fn separate_border_spacing_grows_a_table_shorter_than_its_rows() {
    // CSS 2.2 §17.5.3: a specified table height is a minimum, so the rows
    // and their gaps still fit.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let table = doc.append_element(
        Some(body),
        "table",
        Some("display: table; border-spacing: 2px; height: 10px"),
    );
    let tr = doc.append_element(Some(table), "tr", Some("display: table-row"));
    let td = sized_cell(&mut doc, tr, 10.0, 10.0);
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).unwrap();
    crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4).unwrap();

    let cell = doc.nodes[td].layout;
    assert_eq!((cell.location.y, cell.size.height), (2.0, 10.0));
    assert_eq!(doc.nodes[table].layout.size.height, 14.0);
}

#[test]
fn fixed_colspan_minimum_reclaims_only_unconstrained_column_space() {
    for (minimum, last_floor, last_width, expected) in [
        (80.0, 0.0, Dimension::auto(), [40.0, 40.0, 10.0]),
        (100.0, 0.0, Dimension::auto(), [50.0, 50.0, 0.0]),
        (80.0, 15.0, Dimension::auto(), [40.0, 40.0, 15.0]),
        (80.0, 0.0, Dimension::length(40.0), [40.0, 40.0, 40.0]),
    ] {
        let mut span = fixed_cell(0, 0, 2, Dimension::auto());
        span.specified_min_width = Dimension::length(minimum);
        let mut last = fixed_cell(0, 2, 1, last_width);
        last.specified_min_width = Dimension::length(last_floor);
        let grid = super::TableGrid {
            n_cols: 3,
            rows: vec![],
            cells: vec![span, last],
            col_widths: vec![],
        };
        assert_eq!(super::resolve_fixed_column_widths(&grid, 90.0), expected);
    }
}

#[test]
fn fixed_span_redistribution_preserves_another_spans_minimum() {
    let mut first = fixed_cell(0, 0, 2, Dimension::auto());
    first.specified_min_width = Dimension::length(30.0);
    let mut second = fixed_cell(0, 2, 2, Dimension::auto());
    second.specified_min_width = Dimension::length(80.0);
    let grid = super::TableGrid {
        n_cols: 4,
        rows: vec![],
        cells: vec![first, second],
        col_widths: vec![],
    };
    assert_eq!(
        super::resolve_fixed_column_widths(&grid, 100.0),
        [15.0, 15.0, 40.0, 40.0]
    );
}

#[test]
fn a_satisfied_span_minimum_keeps_all_authored_column_widths() {
    let mut span = fixed_cell(0, 0, 2, Dimension::auto());
    span.specified_min_width = Dimension::length(80.0);
    let grid = super::TableGrid {
        n_cols: 3,
        rows: vec![],
        cells: vec![span, fixed_cell(0, 2, 1, Dimension::auto())],
        col_widths: [40.0, 40.0, 10.0]
            .map(|width| super::ColSizing {
                width: Dimension::length(width),
                min_width: Dimension::auto(),
                max_width: Dimension::auto(),
            })
            .to_vec(),
    };
    assert_eq!(
        super::resolve_fixed_column_widths(&grid, 90.0),
        [40.0, 40.0, 10.0]
    );
}

#[test]
fn negative_absolute_insets_do_not_expand_the_available_table_width() {
    use crate::layout::test_support::{ifc_ahem_fonts, page_box_800x600};
    for inset in ["left:-100px", "right:-100px"] {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Some("display:block"));
        let body = doc.append_element(Some(html), "body", Some("display:block;margin:0"));
        let container = doc.append_element(
            Some(body),
            "div",
            Some("display:block;position:relative;width:100px;height:100px"),
        );
        let table = doc.append_element(Some(container), "table", Some(format!("display:table;position:absolute;{inset};height:100px;border-spacing:0;font-family:Ahem;font-size:10px;line-height:10px")));
        let row = doc.append_element(Some(table), "tr", Some("display:table-row"));
        let cell = doc.append_element(Some(row), "td", Some("display:table-cell;padding:0"));
        doc.append_text(cell, "x x x x x x x x x");
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cascade = cascade(&doc, &rules).expect("cascade");
        doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
        crate::layout::layout_single_page(&mut doc, &cascade, page_box_800x600()).expect("layout");
        assert_eq!(doc.nodes[table].layout.size.width, 100.0, "{inset}");
    }
}

thread_local! {
    pub(super) static AUTHORED_LENGTH_CELL_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static COLUMN_RESOLUTIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn with_retained_border_state_in(
    session: &mut LayoutSession<'_>,
    grid: &super::TableGrid,
    resolve: impl FnOnce(&LayoutSession<'_>, &mut super::BorderState),
) {
    let mut state = super::BorderState::retained(session, grid);
    resolve(session, &mut state);
    publish_border_state_for_assertions(session, &state);
}

/// The border widths boxes have once a resolution has been published.
type UsedBorders = std::collections::HashMap<usize, Edges<LengthPercentage>>;

/// The border widths the cells and rows of `grid`, and `extra`, have in
/// `session`.
fn used_borders(
    session: &LayoutSession<'_>,
    grid: &super::TableGrid,
    extra: Option<usize>,
) -> UsedBorders {
    grid.cells
        .iter()
        .map(|cell| cell.node_id)
        .chain(grid.rows.iter().copied())
        .chain(extra)
        .map(|idx| (idx, session.table_style(idx).border()))
        .collect()
}

/// Resolves in a session of its own and returns the widths it gave the boxes.
///
/// A resolution is published as overrides of the session, which end with it,
/// so the widths are read before then.
fn with_retained_border_state(
    doc: &mut Document,
    grid: &super::TableGrid,
    extra: Option<usize>,
    resolve: impl FnOnce(&LayoutSession<'_>, &mut super::BorderState),
) -> UsedBorders {
    with_layout_session(doc, |session| {
        with_retained_border_state_in(session, grid, resolve);
        used_borders(session, grid, extra)
    })
}
fn resolve_collapsed_cell_borders_in_document(
    doc: &mut Document,
    grid: &super::TableGrid,
    index: &super::CellIndex,
) -> UsedBorders {
    with_retained_border_state(doc, grid, None, |doc, state| {
        super::resolve_collapsed_cell_borders(doc, state, grid, index)
    })
}
fn resolve_collapsed_row_borders_in_document(
    doc: &mut Document,
    grid: &super::TableGrid,
) -> UsedBorders {
    with_retained_border_state(doc, grid, None, |doc, state| {
        super::resolve_collapsed_row_borders(doc, state, grid)
    })
}
fn resolve_collapsed_table_edges_in_document(
    doc: &mut Document,
    grid: &super::TableGrid,
    table: usize,
) -> UsedBorders {
    with_retained_border_state(doc, grid, Some(table), |doc, state| {
        super::resolve_collapsed_table_edges(doc, state, grid, table)
    })
}
fn compute_collapsed_lines_in_document(
    doc: &mut Document,
    grid: &super::TableGrid,
    index: &super::CellIndex,
    border: &Edges<f32>,
    basis: Option<f32>,
) -> super::CollapsedLines {
    with_layout_session(doc, |session| {
        let state = super::BorderState::retained(session, grid);
        super::compute_collapsed_lines(session, &state, grid, index, border, basis)
    })
}
fn resolve_collapsed_border_pair_in_document(
    doc: &mut Document,
    leading: usize,
    leading_side: super::CellBorderSide,
    trailing: usize,
    trailing_side: super::CellBorderSide,
) {
    let grid = super::TableGrid {
        n_cols: 2,
        rows: vec![],
        cells: vec![
            make_cell(leading, 0, 0, 1, 1, Dimension::auto()),
            make_cell(trailing, 0, 1, 1, 1, Dimension::auto()),
        ],
        col_widths: vec![],
    };
    with_retained_border_state(doc, &grid, None, |doc, state| {
        if let (Some(left), Some(right)) = (
            state.candidate(doc, leading, leading_side),
            state.candidate(doc, trailing, trailing_side),
        ) {
            let winner = super::collapsed_border_winner(left, right);
            state.set_side(leading, leading_side, winner.border, winner.taffy_width);
            state.set_side(trailing, trailing_side, winner.border, winner.taffy_width);
        }
    });
}

// Materialize the pure resolver result so assertions can read it back.
fn publish_border_state_for_assertions(
    session: &mut LayoutSession<'_>,
    state: &super::BorderState,
) {
    let entries = state.entries(session);
    let changed: Vec<_> = state
        .cells
        .iter()
        .filter(|cell| session.table_style(cell.node_id).border() != cell.widths)
        .map(|cell| cell.node_id)
        .collect();
    if !changed.is_empty() {
        session.layout_cache_context.invalidate();
    }
    for (key, entry) in entries {
        let idx = session.nodes.layout_index(key).expect("border key is live");
        session.record_pass_override(idx, entry);
    }
    for cell in &state.cells {
        session.table_layout_node_mut(cell.node_id).collapsed_border = cell.paint;
    }
    for &(row, paint) in &state.rows {
        session.nodes[row].collapsed_border = paint;
    }
    for cell in changed {
        session.table_layout_node_mut(cell).cache.clear();
    }
}
