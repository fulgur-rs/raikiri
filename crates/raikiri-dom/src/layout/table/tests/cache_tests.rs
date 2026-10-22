use super::*;

use super::super::{BORDER_RESOLUTIONS, GRID_BUILDS};

fn fixture(rows: usize, columns: usize) -> (Document, usize, usize) {
    let mut doc = Document::new();
    let table = append_grid_part(
        &mut doc,
        0,
        "table",
        DisplayValue::Table,
        "display:table;border-collapse:collapse;border:3px solid red",
    );
    let mut first = 0;
    for _ in 0..rows {
        let row = append_grid_part(
            &mut doc,
            table,
            "tr",
            DisplayValue::TableRow,
            "display:table-row",
        );
        for _ in 0..columns {
            let cell = append_grid_part(
                &mut doc,
                row,
                "td",
                DisplayValue::TableCell,
                "display:table-cell;width:20px;height:10px;border:2px solid blue",
            );
            if first == 0 {
                first = cell;
            }
        }
    }
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    crate::layout::apply_computed_to_style(&mut doc, &computed).unwrap();
    (doc, table, first)
}

#[test]
fn repeated_table_probes_build_grid_and_resolve_borders_once() {
    for (rows, columns) in [(2, 2), (32, 16)] {
        let (mut doc, table, _) = fixture(rows, columns);
        GRID_BUILDS.with(|count| count.set(0));
        BORDER_RESOLUTIONS.with(|count| count.set(0));
        let mut session = LayoutSession::new(&mut doc);
        let input = column_probe_input();
        let first = compute_table_layout_in_session(&mut session, table.into(), input);
        for _ in 0..4 {
            let output = compute_table_layout_in_session(&mut session, table.into(), input);
            assert_eq!(output.size, first.size);
        }
        assert!(session.context.table_layout_error.is_none());
        let builds = GRID_BUILDS.with(|count| count.get());
        let resolutions = BORDER_RESOLUTIONS.with(|count| count.get());
        eprintln!(
            "{rows}x{columns}, five probes: grid builds={builds}, border resolutions={resolutions}"
        );
        assert_eq!(builds, 1);
        assert_eq!(resolutions, 1);
    }
}

#[test]
fn width_changes_recompute_borders_but_keep_the_grid() {
    let (mut doc, table, _) = fixture(2, 2);
    GRID_BUILDS.with(|count| count.set(0));
    BORDER_RESOLUTIONS.with(|count| count.set(0));
    let mut session = LayoutSession::new(&mut doc);
    session.layout_style_for_key_mut(table).unwrap().size.width = Dimension::percent(1.0);
    let mut input = column_probe_input();
    let mut widths = Vec::new();
    for basis in [100.0, 200.0, 100.0] {
        input.parent_size.width = Some(basis);
        input.known_dimensions.width = Some(basis);
        widths.push(
            compute_table_layout_in_session(&mut session, table.into(), input)
                .size
                .width,
        );
    }
    assert_eq!(widths, [100.0, 200.0, 100.0]);
    assert_eq!(GRID_BUILDS.with(|count| count.get()), 1);
    assert_eq!(BORDER_RESOLUTIONS.with(|count| count.get()), 3);
}

#[test]
fn size_probe_does_not_cache_cell_layout_or_temporary_widths() {
    let (mut doc, table, first) = fixture(1, 1);
    let mut session = LayoutSession::new(&mut doc);
    let mut input = column_probe_input();
    input.run_mode = taffy::tree::RunMode::ComputeSize;
    let narrow = compute_table_layout_in_session(&mut session, table.into(), input);
    session.layout_style_for_key_mut(first).unwrap().size.width = Dimension::length(100.0);
    session.table_layout_node_mut(first).cache.clear();
    input.run_mode = taffy::tree::RunMode::PerformLayout;
    let wide = compute_table_layout_in_session(&mut session, table.into(), input);
    assert!(wide.size.width > narrow.size.width);
    assert_eq!(session.nodes[first].unrounded_layout.size.width, 100.0);
    session.layout_style_for_key_mut(first).unwrap().size.width = Dimension::length(20.0);
    session.table_layout_node_mut(first).cache.clear();
    let restored = compute_table_layout_in_session(&mut session, table.into(), input);
    assert_eq!(restored.size, narrow.size);
    assert_eq!(session.nodes[first].unrounded_layout.size.width, 20.0);
}

#[test]
fn new_session_and_rebridging_observe_dom_spans_and_changed_borders() {
    let (mut doc, table, first) = fixture(1, 2);
    let input = column_probe_input();
    let original = with_layout_session(&mut doc, |session| {
        compute_table_layout_in_session(session, table.into(), input)
    });
    doc.set_element_attribute(first, "colspan", "2").unwrap();
    doc.set_element_attribute(
        first,
        "style",
        "display:table-cell;width:60px;height:10px;border:8px solid green",
    )
    .unwrap();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    let mut session = LayoutSession::new(&mut doc);
    crate::layout::bridge::apply_computed_to_style_in_session(&mut session, &computed).unwrap();
    let changed = compute_table_layout_in_session(&mut session, table.into(), input);
    assert!(changed.size.width > original.size.width);
    assert_eq!(
        session.nodes[first]
            .collapsed_border
            .unwrap()
            .left
            .width()
            .px(),
        8.0
    );
    // Re-bridging a live session can replace anonymous objects and reuse keys.
    session
        .set_element_attribute(first, "colspan", "1")
        .unwrap();
    session
        .set_element_attribute(
            first,
            "style",
            "display:table-cell;width:20px;height:10px;border:2px solid blue",
        )
        .unwrap();
    let computed = cascade(&*session, &build_rule_tree(&*session)).unwrap();
    crate::layout::bridge::apply_computed_to_style_in_session(&mut session, &computed).unwrap();
    let restored = compute_table_layout_in_session(&mut session, table.into(), input);
    assert_eq!(restored.size, original.size);
}

#[test]
fn collapsed_column_probes_restore_the_same_borders_and_cell_geometry() {
    let (mut doc, table, first) = fixture(2, 3);
    append_grid_part(
        &mut doc,
        table,
        "col",
        DisplayValue::TableColumn,
        "display:table-column;visibility:collapse",
    );
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    crate::layout::apply_computed_to_style(&mut doc, &computed).unwrap();
    let mut session = LayoutSession::new(&mut doc);
    let input = column_probe_input();
    let original = compute_table_layout_in_session(&mut session, table.into(), input);
    let geometry = session.nodes[first + 1].unrounded_layout;
    let grid = super::super::build_table_grid(&session, table).unwrap();
    let borders = super::super::snapshot_borders(&session, &grid);
    for _ in 0..3 {
        let repeated = compute_table_layout_in_session(&mut session, table.into(), input);
        assert_eq!(repeated.size, original.size);
        assert_eq!(session.nodes[first + 1].unrounded_layout, geometry);
        let repeated_borders = super::super::snapshot_borders(&session, &grid);
        assert_eq!(repeated_borders.cells, borders.cells);
        assert_eq!(repeated_borders.rows, borders.rows);
    }
}

#[test]
fn rebridging_anonymous_tables_does_not_reuse_old_cell_placements() {
    let mut doc = Document::new();
    let root = doc.append_element(
        Some(0),
        "html",
        Style::default(),
        Some("display:block;border-spacing:0"),
    );
    let row = append_grid_part(
        &mut doc,
        root,
        "div",
        DisplayValue::TableRow,
        "display:table-row",
    );
    append_grid_part(
        &mut doc,
        row,
        "div",
        DisplayValue::TableCell,
        "display:table-cell;width:20px;height:10px",
    );
    let hidden = doc.append_element(
        Some(row),
        "div",
        Style::default(),
        Some("display:none;width:50px;height:10px"),
    );
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    let mut session = LayoutSession::new(&mut doc);
    crate::layout::bridge::apply_computed_to_style_in_session(&mut session, &computed).unwrap();
    assert_eq!(session.table_objects.tables.len(), 1);
    let table = session.nodes.anonymous_table_key(0);
    let input = column_probe_input();
    for _ in 0..2 {
        assert_eq!(
            compute_table_layout_in_session(&mut session, table.into(), input)
                .size
                .width,
            20.0
        );
    }
    session
        .set_element_attribute(hidden, "style", "display:table-cell;width:50px;height:10px")
        .unwrap();
    session.mark_in_document_flags();
    let computed = cascade(&*session, &build_rule_tree(&*session)).unwrap();
    crate::layout::bridge::apply_computed_to_style_in_session(&mut session, &computed).unwrap();
    assert_eq!(session.nodes.anonymous_table_key(0), table);
    assert_eq!(
        compute_table_layout_in_session(&mut session, table.into(), input)
            .size
            .width,
        70.0
    );
}

#[test]
fn temporary_table_edges_rebuild_topology_on_entry_and_exit() {
    let (mut doc, table, _) = fixture(1, 1);
    GRID_BUILDS.with(|count| count.set(0));
    let mut session = LayoutSession::new(&mut doc);
    super::super::cache::table_grid(&mut session, table).unwrap();
    session.with_resolved_edges(table, 100.0, true, |session| {
        super::super::cache::table_grid(session, table).unwrap();
    });
    super::super::cache::table_grid(&mut session, table).unwrap();
    assert_eq!(GRID_BUILDS.with(|count| count.get()), 3);
}

#[test]
fn cell_measurement_unwind_restores_authored_height() {
    let (mut doc, table, first) = fixture(1, 1);
    let mut session = LayoutSession::new(&mut doc);
    let mut grid = super::super::cache::table_grid(&mut session, table).unwrap();
    let height = session.layout_style_for_key(first).unwrap().size.height;
    session.nodes[first].children.push(usize::MAX - 1);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        super::super::measure_cell(&mut session, &mut grid.cells[0], 20.0);
    }));
    assert!(result.is_err());
    assert_eq!(
        session.layout_style_for_key(first).unwrap().size.height,
        height
    );
}
