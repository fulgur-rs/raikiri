use super::*;
use raikiri_style::ComputedLengthPercentageOrAuto;

use super::super::{BORDER_RESOLUTIONS, GRID_BUILDS};

pub(super) fn fixture(rows: usize, columns: usize) -> (Document, usize, usize) {
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
    for (rows, columns) in [(2, 2), (32, 16), (64, 16)] {
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
    session.record_pass_override(
        table,
        width_override(ComputedLengthPercentageOrAuto::Percent(100.0)),
    );
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
    session.record_pass_override(
        first,
        width_override(ComputedLengthPercentageOrAuto::Px(100.0)),
    );
    session.table_layout_node_mut(first).cache.clear();
    input.run_mode = taffy::tree::RunMode::PerformLayout;
    let wide = compute_table_layout_in_session(&mut session, table.into(), input);
    assert!(wide.size.width > narrow.size.width);
    assert_eq!(session.nodes[first].unrounded_layout.size.width, 100.0);
    session.record_pass_override(
        first,
        width_override(ComputedLengthPercentageOrAuto::Px(20.0)),
    );
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
    let height = session.table_style(first).size().height;
    session.nodes[first].children.push(usize::MAX - 1);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        super::super::measure_cell(&mut session, &mut grid.cells[0], 20.0);
    }));
    assert!(result.is_err());
    assert_eq!(session.table_style(first).size().height, height);
}

/// A table with a rowspan, a colspan, block and flex descendants and a nested
/// table, so a cache miss can change the geometry of every kind of box.
///
/// Returns the document, its cascade, the table, a block child and a flex item.
fn descendant_fixture() -> (Document, raikiri_style::CascadeResult, usize, usize, usize) {
    let mut doc = Document::new();
    let table = append_grid_part(
        &mut doc,
        0,
        "table",
        DisplayValue::Table,
        "display:table;width:200px;border-collapse:collapse;border:3px solid red",
    );
    for css in ["display:table-column;width:60px", "display:table-column"] {
        append_grid_part(&mut doc, table, "col", DisplayValue::TableColumn, css);
    }
    let cell_css = "display:table-cell;border:2px solid blue;padding:3px";
    let first_row = append_grid_part(
        &mut doc,
        table,
        "tr",
        DisplayValue::TableRow,
        "display:table-row;height:40px",
    );
    let tall = append_grid_part(&mut doc, first_row, "td", DisplayValue::TableCell, cell_css);
    doc.set_element_attribute(tall, "rowspan", "2").unwrap();
    let block = doc.append_element(
        Some(tall),
        "div",
        Style::default(),
        Some("display:block;width:50%;height:12px;margin:2px"),
    );
    let flex = doc.append_element(
        Some(tall),
        "div",
        Style::default(),
        Some("display:flex;width:100%;height:10px"),
    );
    let item = doc.append_element(
        Some(flex),
        "div",
        Style::default(),
        Some("display:block;width:20px;height:6px;flex-grow:1"),
    );
    doc.append_element(
        Some(flex),
        "div",
        Style::default(),
        Some("display:block;width:10px;height:6px"),
    );
    append_grid_part(
        &mut doc,
        first_row,
        "td",
        DisplayValue::TableCell,
        "display:table-cell;border:4px solid green;height:25px",
    );
    let second_row = append_grid_part(
        &mut doc,
        table,
        "tr",
        DisplayValue::TableRow,
        "display:table-row",
    );
    append_grid_part(
        &mut doc,
        second_row,
        "td",
        DisplayValue::TableCell,
        "display:table-cell;border:1px solid black;height:15px",
    );
    let third_row = append_grid_part(
        &mut doc,
        table,
        "tr",
        DisplayValue::TableRow,
        "display:table-row",
    );
    let wide = append_grid_part(&mut doc, third_row, "td", DisplayValue::TableCell, cell_css);
    doc.set_element_attribute(wide, "colspan", "2").unwrap();
    let nested = append_grid_part(
        &mut doc,
        wide,
        "table",
        DisplayValue::Table,
        "display:table;border-collapse:collapse;border:1px solid black",
    );
    let nested_row = append_grid_part(
        &mut doc,
        nested,
        "tr",
        DisplayValue::TableRow,
        "display:table-row",
    );
    append_grid_part(
        &mut doc,
        nested_row,
        "td",
        DisplayValue::TableCell,
        "display:table-cell;width:30px;height:9px;border:2px solid black",
    );
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    crate::layout::apply_computed_to_style(&mut doc, &computed).unwrap();
    (doc, computed, table, block, item)
}

/// The geometry and collapsed paint of every box under `root`, in tree order.
fn subtree_geometry(tree: &LayoutSession<'_>, root: usize) -> Vec<String> {
    let mut geometry = Vec::new();
    let mut pending = vec![root];
    while let Some(idx) = pending.pop() {
        let node = &tree.nodes[idx];
        geometry.push(format!(
            "{idx}: {:?} paint={:?}",
            node.unrounded_layout, node.collapsed_border
        ));
        pending.extend(node.children.iter().rev().copied());
    }
    geometry
}

#[test]
fn table_descendant_geometry_agrees_across_cold_warm_and_forced_miss() {
    for views in [false, true] {
        let (mut doc, cascade, table, block, item) = descendant_fixture();
        let mut tree = if views {
            LayoutSession::with_cascade(&mut doc, &cascade)
        } else {
            LayoutSession::new(&mut doc)
        };
        if views {
            crate::layout::style_views::tests::prepare_all_layout_views(&mut tree);
        }
        let id = taffy::NodeId::from(table);
        let input = column_probe_input();
        GRID_BUILDS.with(|count| count.set(0));
        let cold_output = tree.compute_child_layout_with_block_ctx(id, input, None);
        let cold = subtree_geometry(&tree, table);
        let cold_builds = GRID_BUILDS.with(|count| count.get());
        assert!(
            cold_builds >= 2,
            "{views}: the outer and nested tables build grids"
        );
        assert!(
            cold.len() > 15,
            "{views}: cells, rows and descendants laid out"
        );
        assert_eq!(tree.nodes[block].unrounded_layout.size.height, 12.0);
        assert_eq!(tree.nodes[item].unrounded_layout.size.height, 6.0);
        assert!(tree.nodes[item].unrounded_layout.size.width > 20.0);

        let warm_output = tree.compute_child_layout_with_block_ctx(id, input, None);
        assert_eq!(
            format!("{cold_output:?}"),
            format!("{warm_output:?}"),
            "{views}"
        );
        assert_eq!(subtree_geometry(&tree, table), cold, "{views}: warm");
        assert_eq!(
            GRID_BUILDS.with(|count| count.get()),
            cold_builds,
            "{views}: the warm pass hits the cache"
        );

        tree.layout_cache_context.invalidate();
        tree.context.table_cache.clear();
        let missed_output = tree.compute_child_layout_with_block_ctx(id, input, None);
        assert_eq!(
            format!("{cold_output:?}"),
            format!("{missed_output:?}"),
            "{views}"
        );
        assert_eq!(subtree_geometry(&tree, table), cold, "{views}: forced miss");
        assert!(
            GRID_BUILDS.with(|count| count.get()) >= 2 * cold_builds,
            "{views}: the forced miss lays the tables out again"
        );
        assert_eq!(tree.override_checkpoint(), 0);
    }
}
