use super::*;
use std::cell::Cell;
use taffy::CoreStyle;

fn fixture(
    widths: &[u8],
    collapse_middle: bool,
) -> (Document, raikiri_style::CascadeResult, usize, Vec<usize>) {
    let mut doc = Document::new();
    let table = append_grid_part(
        &mut doc,
        0,
        "table",
        DisplayValue::Table,
        "display:table;border-collapse:collapse",
    );
    let row = append_grid_part(
        &mut doc,
        table,
        "tr",
        DisplayValue::TableRow,
        "display:table-row",
    );
    let cells = widths
        .iter()
        .map(|width| {
            append_grid_part(
                &mut doc,
                row,
                "td",
                DisplayValue::TableCell,
                &format!("display:table-cell;width:20px;height:10px;border:{width}px solid black"),
            )
        })
        .collect();
    if collapse_middle {
        for index in 0..widths.len() {
            append_grid_part(
                &mut doc,
                table,
                "col",
                DisplayValue::TableColumn,
                if index == 1 {
                    "display:table-column;visibility:collapse"
                } else {
                    "display:table-column"
                },
            );
        }
    }
    doc.mark_in_document_flags();
    let cascade = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    crate::layout::apply_computed_to_style(&mut doc, &cascade).unwrap();
    (doc, cascade, table, cells)
}

thread_local! {
    static FIRST_CELL: Cell<Option<usize>> = const { Cell::new(None) };
    static WIDTH_FAILURE_AFTER: Cell<usize> = const { Cell::new(2) };
    static PAINT_FAILURE_AFTER: Cell<Option<usize>> = const { Cell::new(None) };
    static SAW_IN_PLACE: Cell<bool> = const { Cell::new(false) };
}

#[test]
fn border_scope_table_restores_width_and_commits_paint() {
    fn inspect(tree: &mut LayoutSession<'_>, idx: usize) {
        if FIRST_CELL.get() == Some(idx) {
            assert_eq!(
                tree.layout_style_for_key(idx).unwrap().border.right,
                LengthPercentage::length(9.0)
            );
            if tree.style_views.is_some() {
                assert_eq!(
                    tree.style_view(idx).border().right,
                    LengthPercentage::length(9.0)
                );
            }
        }
    }
    for views in [false, true] {
        let (mut doc, cascade, table, cells) = fixture(&[2, 9], false);
        let first = cells[0];
        let authored = doc.layout_style_for_key(first).unwrap().border;
        let mut tree = if views {
            LayoutSession::with_cascade(&mut doc, &cascade)
        } else {
            LayoutSession::new(&mut doc)
        };
        if views {
            crate::layout::style_views::tests::prepare_all_layout_views(&mut tree);
            assert!(!tree.layout_dirty);
        }
        FIRST_CELL.set(Some(first));
        super::cell_scope_tests::with_inspector(inspect, || {
            super::super::compute_table_layout_checked(
                &mut tree,
                table.into(),
                column_probe_input(),
            )
        })
        .unwrap();
        FIRST_CELL.set(None);
        assert_eq!(tree.layout_style_for_key(first).unwrap().border, authored);
        assert_eq!(
            tree.nodes[first]
                .collapsed_border
                .unwrap()
                .right
                .width()
                .px(),
            9.0
        );
        assert_eq!(tree.override_checkpoint(), 0);
    }
}

#[test]
fn border_scope_in_place_panic_restores_remaining() {
    fn fail(tree: &mut LayoutSession<'_>, idx: usize) {
        if FIRST_CELL.get() == Some(idx) {
            assert_eq!(
                tree.layout_style_for_key(idx).unwrap().border.right,
                LengthPercentage::length(9.0)
            );
            assert_eq!(
                tree.nodes[idx].collapsed_border.unwrap().right.width().px(),
                9.0
            );
            SAW_IN_PLACE.set(true);
            panic!("in-place descendant failure");
        }
    }
    let (mut doc, _, table, cells) = fixture(&[2, 9, 3], true);
    let first = cells[0];
    doc.nodes[first].collapsed_border = Some(Sides {
        right: collapsed_candidate(BorderStyle::Solid, 3.0, CssColor::BLACK).border,
        ..Sides::all(collapsed_candidate(BorderStyle::Solid, 2.0, CssColor::BLACK).border)
    });
    let mut tree = LayoutSession::new(&mut doc);
    let mut grid = super::super::cache::table_grid(&mut tree, table).unwrap();
    let prepared = super::super::cache::prepared_borders(&mut tree, table, &grid, None, Rect::ZERO);
    let columns = prepared.columns_in_place.as_ref().unwrap();
    let entries = columns.remaining.entries(&tree);
    FIRST_CELL.set(Some(first));
    SAW_IN_PLACE.set(false);
    tree.with_overrides(entries, |tree| {
        let checkpoint = tree.override_checkpoint();
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            super::cell_scope_tests::with_inspector(fail, || {
                super::super::measured_in_place(tree, Some(columns), |tree| {
                    super::super::resolve_row_heights(
                        tree,
                        &mut grid,
                        &[30.0, 30.0, 30.0],
                        (0.0, 0.0),
                    )
                })
            })
        }))
        .unwrap_err();
        assert_eq!(
            panic.downcast_ref::<&str>(),
            Some(&"in-place descendant failure")
        );
        assert!(SAW_IN_PLACE.get());
        assert_eq!(
            tree.layout_style_for_key(first).unwrap().border.right,
            LengthPercentage::length(3.0)
        );
        assert_eq!(
            tree.nodes[first]
                .collapsed_border
                .unwrap()
                .right
                .width()
                .px(),
            3.0
        );
        assert_eq!(tree.override_checkpoint(), checkpoint);
    });
    FIRST_CELL.set(None);
}

pub(in crate::layout::table) fn after_paint_entry(
    _tree: &LayoutSession<'_>,
    _idx: usize,
    count: usize,
) {
    if PAINT_FAILURE_AFTER.get() == Some(count) {
        panic!("paint setup failure");
    }
}

fn prepared_state(
    tree: &mut LayoutSession<'_>,
    table: usize,
) -> std::rc::Rc<super::super::cache::PreparedBorders> {
    let grid = super::super::cache::table_grid(tree, table).unwrap();
    super::super::cache::prepared_borders(tree, table, &grid, None, Rect::ZERO)
}

#[test]
fn border_scope_setup_failure_rolls_back_width_and_paint() {
    fn width_failure(_tree: &LayoutSession<'_>, _idx: usize, count: usize) {
        if count == WIDTH_FAILURE_AFTER.get() {
            panic!("width setup failure");
        }
    }
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            PAINT_FAILURE_AFTER.set(None);
        }
    }
    for (paint, count) in [(false, 2), (false, 5), (true, 1), (true, 3)] {
        let (mut doc, _, table, cells) = fixture(&[2, 9], false);
        let row = doc.nodes[cells[0]].parent.unwrap();
        let row_paint = doc.nodes[row].collapsed_border;
        let originals: Vec<_> = cells
            .iter()
            .map(|&idx| {
                (
                    idx,
                    doc.layout_style_for_key(idx).unwrap().border,
                    doc.nodes[idx].collapsed_border,
                )
            })
            .collect();
        let mut tree = LayoutSession::new(&mut doc);
        let prepared = prepared_state(&mut tree, table);
        let state = prepared.borders.as_ref().unwrap();
        let _reset = Reset;
        WIDTH_FAILURE_AFTER.set(count);
        if paint {
            PAINT_FAILURE_AFTER.set(Some(count));
        }
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut run = || {
                super::super::border_scopes::with_table_borders(
                    &mut tree,
                    state,
                    |_| -> Result<(), ()> { panic!("unreachable body") },
                )
            };
            if paint {
                run()
            } else {
                crate::layout::style_views::tests::with_setup_inspector(width_failure, run)
            }
        }))
        .unwrap_err();
        assert_eq!(
            panic.downcast_ref::<&str>(),
            Some(&if paint {
                "paint setup failure"
            } else {
                "width setup failure"
            })
        );
        assert_eq!(tree.override_checkpoint(), 0);
        for (idx, border, painted) in originals {
            assert_eq!(tree.layout_style_for_key(idx).unwrap().border, border);
            assert_eq!(tree.nodes[idx].collapsed_border, painted);
        }
        assert_eq!(tree.nodes[row].collapsed_border, row_paint);
    }
}

#[test]
fn border_scope_outer_error_keeps_successful_nested_paint() {
    for panic_body in [false, true] {
        let (mut doc, _, outer, cells) = fixture(&[2, 9], false);
        let nested = append_grid_part(
            &mut doc,
            cells[0],
            "table",
            DisplayValue::Table,
            "display:table;border-collapse:collapse",
        );
        let row = append_grid_part(
            &mut doc,
            nested,
            "tr",
            DisplayValue::TableRow,
            "display:table-row",
        );
        let mut nested_cells = Vec::new();
        for width in [3, 7] {
            nested_cells.push(append_grid_part(
                &mut doc,
                row,
                "td",
                DisplayValue::TableCell,
                &format!("display:table-cell;width:10px;border:{width}px solid black"),
            ));
        }
        doc.mark_in_document_flags();
        let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
        crate::layout::apply_computed_to_style(&mut doc, &computed).unwrap();
        let mut tree = LayoutSession::new(&mut doc);
        let prepared = prepared_state(&mut tree, outer);
        let state = prepared.borders.as_ref().unwrap();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            super::super::border_scopes::with_table_borders(
                &mut tree,
                state,
                |tree| -> Result<(), ()> {
                    super::super::compute_table_layout_checked(
                        tree,
                        nested.into(),
                        column_probe_input(),
                    )
                    .unwrap();
                    tree.layout_style_for_key_mut(cells[0]).unwrap().margin.left =
                        LengthPercentageAuto::length(91.0);
                    if panic_body {
                        panic!("outer table failure");
                    }
                    Err(())
                },
            )
        }));
        if panic_body {
            assert_eq!(
                result.unwrap_err().downcast_ref::<&str>(),
                Some(&"outer table failure")
            );
        } else {
            assert_eq!(result.unwrap(), Err(()));
        }
        assert_eq!(tree.nodes[cells[0]].collapsed_border, None);
        assert_eq!(
            tree.layout_style_for_key(cells[0]).unwrap().border.right,
            LengthPercentage::length(2.0)
        );
        assert_eq!(
            tree.layout_style_for_key(cells[0]).unwrap().margin.left,
            LengthPercentageAuto::length(91.0)
        );
        assert_eq!(
            tree.nodes[nested_cells[0]]
                .collapsed_border
                .unwrap()
                .right
                .width()
                .px(),
            7.0
        );
        assert_eq!(tree.override_checkpoint(), 0);
    }
}

#[test]
fn border_scope_temporary_reset_is_explicit_and_restores_outer_values() {
    use super::super::{
        CellBorderSide,
        border_scopes::{with_table_borders, with_temporary_borders},
    };
    let (mut doc, _, table, cells) = fixture(&[2, 9], false);
    let first = cells[0];
    let mut tree = LayoutSession::new(&mut doc);
    let grid = super::super::cache::table_grid(&mut tree, table).unwrap();
    let authored = super::super::BorderState::authored(&tree, &grid);
    let mut outer = authored.clone();
    let mut used = authored.clone();
    let outer_winner = collapsed_candidate(BorderStyle::Solid, 17.0, CssColor::BLACK);
    let inner_winner = collapsed_candidate(BorderStyle::Double, 9.0, CssColor::BLACK);
    outer.set_side(
        first,
        CellBorderSide::Right,
        outer_winner.border,
        outer_winner.taffy_width,
    );
    used.set_side(
        first,
        CellBorderSide::Right,
        inner_winner.border,
        inner_winner.taffy_width,
    );
    let result: Result<(), ()> = with_table_borders(&mut tree, &outer, |tree| {
        assert_eq!(
            tree.layout_style_for_key(first).unwrap().border.right,
            LengthPercentage::length(17.0)
        );
        with_temporary_borders(tree, &authored, |tree| {
            assert_eq!(
                tree.layout_style_for_key(first).unwrap().border.right,
                LengthPercentage::length(2.0)
            );
            assert_eq!(tree.nodes[first].collapsed_border, None);
            let result: Result<(), ()> = with_temporary_borders(tree, &used, |tree| {
                assert_eq!(
                    tree.layout_style_for_key(first).unwrap().border.right,
                    LengthPercentage::length(9.0)
                );
                assert_eq!(
                    tree.nodes[first]
                        .collapsed_border
                        .unwrap()
                        .right
                        .width()
                        .px(),
                    9.0
                );
                tree.layout_style_for_key_mut(first).unwrap().margin.left =
                    LengthPercentageAuto::length(91.0);
                Err(())
            });
            assert_eq!(result, Err(()));
            assert_eq!(
                tree.layout_style_for_key(first).unwrap().border.right,
                LengthPercentage::length(2.0)
            );
            assert_eq!(tree.nodes[first].collapsed_border, None);
        });
        assert_eq!(
            tree.layout_style_for_key(first).unwrap().border.right,
            LengthPercentage::length(17.0)
        );
        assert_eq!(
            tree.nodes[first]
                .collapsed_border
                .unwrap()
                .right
                .width()
                .px(),
            17.0
        );
        Err(())
    });
    assert_eq!(result, Err(()));
    assert_eq!(
        tree.layout_style_for_key(first).unwrap().border.right,
        LengthPercentage::length(2.0)
    );
    assert_eq!(tree.nodes[first].collapsed_border, None);
    assert_eq!(
        tree.layout_style_for_key(first).unwrap().margin.left,
        LengthPercentageAuto::length(91.0)
    );
    assert_eq!(tree.override_checkpoint(), 0);
}

#[test]
fn border_scope_native_panic_restores_original_paint() {
    fn fail(tree: &mut LayoutSession<'_>, idx: usize) {
        if FIRST_CELL.get() == Some(idx) {
            assert_eq!(
                tree.layout_style_for_key(idx).unwrap().border.right,
                LengthPercentage::length(9.0)
            );
            panic!("native table failure");
        }
    }
    let (mut doc, _, table, cells) = fixture(&[2, 9, 3], true);
    let originals: Vec<_> = cells
        .iter()
        .map(|&idx| {
            (
                idx,
                doc.layout_style_for_key(idx).unwrap().border,
                doc.nodes[idx].collapsed_border,
            )
        })
        .collect();
    let mut tree = LayoutSession::new(&mut doc);
    FIRST_CELL.set(Some(cells[0]));
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        super::cell_scope_tests::with_inspector(fail, || {
            super::super::compute_table_layout_checked(
                &mut tree,
                table.into(),
                column_probe_input(),
            )
        })
    }))
    .unwrap_err();
    FIRST_CELL.set(None);
    assert_eq!(panic.downcast_ref::<&str>(), Some(&"native table failure"));
    for (idx, border, paint) in originals {
        assert_eq!(tree.layout_style_for_key(idx).unwrap().border, border);
        assert_eq!(tree.nodes[idx].collapsed_border, paint);
    }
    assert_eq!(tree.override_checkpoint(), 0);
    let output =
        super::super::compute_table_layout_checked(&mut tree, table.into(), column_probe_input())
            .unwrap();
    assert!(output.size.width > 0.0);
    assert_eq!(
        tree.nodes[cells[0]]
            .collapsed_border
            .unwrap()
            .right
            .width()
            .px(),
        3.0
    );
}

#[test]
fn border_scope_nested_cold_table_keeps_publication() {
    use super::super::border_scopes::with_table_borders;
    let (mut doc, _, outer, cells) = fixture(&[2, 9], false);
    let nested = append_grid_part(
        &mut doc,
        cells[0],
        "table",
        DisplayValue::Table,
        "display:table;border-collapse:collapse",
    );
    let row = append_grid_part(
        &mut doc,
        nested,
        "tr",
        DisplayValue::TableRow,
        "display:table-row",
    );
    doc.append_element(
        Some(row),
        "div",
        Style::default(),
        Some("display:block;width:20px;height:10px"),
    );
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    crate::layout::apply_computed_to_style(&mut doc, &computed).unwrap();
    let mut tree = LayoutSession::with_cascade(&mut doc, &computed);
    tree.prepare_style_views();
    tree.sync_layout_cache_mutations();
    assert!(!tree.layout_dirty);
    assert!(tree.box_style_data.is_ready());
    let synthetic = tree
        .nodes
        .layout_keys()
        .find(|&idx| {
            matches!(
                tree.nodes.box_key(idx),
                Some(crate::node::arena::BoxKey::AnonymousCell(_))
            )
        })
        .expect("orphan child has a prepared synthetic cell");
    let prepared = prepared_state(&mut tree, outer);
    let publication = tree.layout_cache_context.publication_epoch();
    let foundation = tree.box_style_data.stamp();
    super::super::GRID_BUILDS.set(0);
    super::super::BORDER_RESOLUTIONS.set(0);
    with_table_borders(
        &mut tree,
        prepared.borders.as_ref().unwrap(),
        |tree| -> Result<(), ()> {
            let first = super::super::compute_table_layout_checked(
                tree,
                nested.into(),
                column_probe_input(),
            )
            .unwrap();
            let paint = tree.nodes[synthetic].collapsed_border;
            assert!(paint.is_some());
            let second = super::super::compute_table_layout_checked(
                tree,
                nested.into(),
                column_probe_input(),
            )
            .unwrap();
            assert_eq!(first.size, second.size);
            assert_eq!(tree.nodes[synthetic].collapsed_border, paint);
            assert!(first.size.width > 0.0);
            assert!(!tree.layout_dirty);
            assert!(tree.box_style_data.is_ready());
            assert_eq!(tree.layout_cache_context.publication_epoch(), publication);
            assert_eq!(tree.box_style_data.stamp(), foundation);
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(super::super::GRID_BUILDS.get(), 1);
    assert_eq!(super::super::BORDER_RESOLUTIONS.get(), 1);
    assert_eq!(tree.layout_cache_context.publication_epoch(), publication);
    assert_eq!(tree.box_style_data.stamp(), foundation);
    assert_eq!(tree.override_checkpoint(), 0);
}

#[test]
fn border_scope_publications_keep_linear_entries_and_no_history() {
    fn count(_tree: &LayoutSession<'_>, _idx: usize, count: usize) {
        MAX_WIDTH_ENTRIES.set(MAX_WIDTH_ENTRIES.get().max(count));
    }
    thread_local! { static MAX_WIDTH_ENTRIES: Cell<usize> = const { Cell::new(0) }; }
    for (rows, columns) in [(2, 2), (32, 16), (64, 16)] {
        let (mut doc, table, first) = super::cache_tests::fixture(rows, columns);
        let authored = doc.layout_style_for_key(first).unwrap().border;
        let mut tree = LayoutSession::new(&mut doc);
        let prepared = prepared_state(&mut tree, table);
        let state = prepared.borders.as_ref().unwrap();
        assert_eq!(state.entries(&tree).len(), 4 * rows * columns);
        MAX_WIDTH_ENTRIES.set(0);
        for _ in 0..3 {
            crate::layout::style_views::tests::with_setup_inspector(count, || {
                super::super::border_scopes::with_table_borders(
                    &mut tree,
                    state,
                    |tree| -> Result<(), ()> {
                        assert_eq!(tree.override_checkpoint(), 4 * rows * columns);
                        Ok(())
                    },
                )
            })
            .unwrap();
            assert_eq!(tree.override_checkpoint(), 0);
            assert_eq!(tree.layout_style_for_key(first).unwrap().border, authored);
        }
        assert_eq!(MAX_WIDTH_ENTRIES.get(), 4 * rows * columns);
        eprintln!(
            "{rows}x{columns}: {} entries per publication, no retained history",
            MAX_WIDTH_ENTRIES.get()
        );
    }
}

#[test]
fn border_scope_cache_hit_and_forced_miss_keep_geometry_and_paint() {
    for collapsed in [false, true] {
        let (mut doc, _, table, cells) = fixture(&[2, 9, 3], collapsed);
        let mut tree = LayoutSession::new(&mut doc);
        tree.sync_layout_cache_mutations();
        let publication = tree.layout_cache_context.publication_epoch();
        super::super::GRID_BUILDS.set(0);
        super::super::BORDER_RESOLUTIONS.set(0);
        let input = column_probe_input();
        let first =
            super::super::compute_table_layout_checked(&mut tree, table.into(), input).unwrap();
        let expected: Vec<_> = cells
            .iter()
            .map(|&idx| {
                (
                    tree.layout_style_for_key(idx).unwrap().border,
                    tree.nodes[idx].collapsed_border,
                    tree.nodes[idx].unrounded_layout,
                )
            })
            .collect();
        let resolutions = super::super::BORDER_RESOLUTIONS.get();
        assert!(resolutions > 0);
        for force in [false, true] {
            if force {
                tree.context.table_cache.clear();
            }
            let repeated =
                super::super::compute_table_layout_checked(&mut tree, table.into(), input).unwrap();
            assert_eq!(repeated.size, first.size);
            let actual: Vec<_> = cells
                .iter()
                .map(|&idx| {
                    (
                        tree.layout_style_for_key(idx).unwrap().border,
                        tree.nodes[idx].collapsed_border,
                        tree.nodes[idx].unrounded_layout,
                    )
                })
                .collect();
            assert_eq!(actual, expected);
            assert_eq!(super::super::GRID_BUILDS.get(), if force { 2 } else { 1 });
            assert_eq!(
                super::super::BORDER_RESOLUTIONS.get(),
                resolutions * if force { 2 } else { 1 }
            );
            assert_eq!(tree.layout_cache_context.publication_epoch(), publication);
            assert_eq!(tree.override_checkpoint(), 0);
        }
    }
}

#[test]
fn border_scope_empty_table_keeps_natural_and_assigned_widths() {
    for (parent_display, parent_css, assigned) in [
        (DisplayValue::Block, "display:block", false),
        (DisplayValue::Flex, "display:flex", true),
        (DisplayValue::Grid, "display:grid", true),
    ] {
        for collapse in [false, true] {
            for known in [None, Some(8.0), Some(80.0)] {
                let mut doc = Document::new();
                let parent = append_grid_part(&mut doc, 0, "div", parent_display, parent_css);
                let table = append_grid_part(
                    &mut doc,
                    parent,
                    "table",
                    DisplayValue::Table,
                    if collapse {
                        "display:table;border-collapse:collapse;padding:5px;border:2px solid black"
                    } else {
                        "display:table;padding:5px;border:2px solid black"
                    },
                );
                doc.mark_in_document_flags();
                let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
                crate::layout::apply_computed_to_style(&mut doc, &computed).unwrap();
                let mut tree = LayoutSession::new(&mut doc);
                let mut input = column_probe_input();
                input.known_dimensions.width = known;
                let output =
                    super::super::compute_table_layout_checked(&mut tree, table.into(), input)
                        .unwrap();
                let expected = match (assigned, known) {
                    (true, Some(width)) => width,
                    (false, Some(width)) => 14.0_f32.min(width),
                    (_, None) => 14.0,
                };
                assert_eq!(output.size.width, expected);
                assert_eq!(tree.override_checkpoint(), 0);
            }
        }
    }
}
