use super::*;
use crate::layout::test_support::TABLE_CELL_LOOKUP_STEPS;

use super::super::cell_index::CellIndex;

fn grid(
    rows: usize,
    columns: u16,
    cells: Vec<super::super::CellPlacement>,
) -> super::super::TableGrid {
    super::super::TableGrid {
        n_cols: columns,
        rows: vec![0; rows],
        cells,
        col_widths: vec![],
    }
}

fn placement(row: u16, column: u16, row_span: u16, col_span: u16) -> super::super::CellPlacement {
    make_cell(0, row, column, col_span, row_span, Dimension::auto())
}

fn assert_matches_linear_search(grid: &super::super::TableGrid) {
    let index = CellIndex::new(grid);
    for row in 0..grid.rows.len() {
        let row = u16::try_from(row).unwrap();
        for col in 0..grid.n_cols {
            let expected = grid.cells.iter().position(|cell| {
                usize::from(row) >= usize::from(cell.row)
                    && usize::from(row) < usize::from(cell.row) + usize::from(cell.row_span)
                    && usize::from(col) >= usize::from(cell.col_start)
                    && usize::from(col) < usize::from(cell.col_start) + usize::from(cell.col_span)
            });
            assert_eq!(index.at(row, col), expected, "position ({row}, {col})");
        }
    }
}

#[test]
fn overlapping_spans_keep_first_placement_across_tree_levels() {
    let grid = grid(
        8,
        12,
        vec![
            placement(3, 4, 1, 2),
            placement(0, 2, 8, 8),
            placement(0, 0, 8, 12),
            placement(1, 1, 5, 10),
        ],
    );
    let index = CellIndex::new(&grid);
    assert_eq!(index.at(3, 4), Some(0));
    assert_eq!(index.at(3, 6), Some(1));
    assert_eq!(index.at(4, 4), Some(1));
    assert_eq!(index.at(3, 10), Some(2));
    assert_matches_linear_search(&grid);
}

#[test]
fn sparse_intervals_preserve_holes_and_half_open_edges() {
    let grid = grid(
        7,
        11,
        vec![
            placement(1, 2, 3, 2),
            placement(2, 4, 1, 3),
            placement(4, 2, 2, 2),
            placement(6, 9, 1, 2),
        ],
    );
    let index = CellIndex::new(&grid);
    assert_eq!(index.at(0, 2), None);
    assert_eq!(index.at(1, 4), None);
    assert_eq!(index.at(2, 4), Some(1));
    assert_eq!(index.at(4, 2), Some(2));
    assert_eq!(index.at(6, 8), None);
    assert_eq!(index.at(6, 11), None);
    assert_eq!(index.at(7, 2), None);
    assert_matches_linear_search(&grid);
}

#[test]
fn mixed_overlapping_intervals_match_linear_search() {
    let cells = (0..80)
        .map(|i| {
            placement(
                (i * 7 % 9) as u16,
                (i * 11 % 17) as u16,
                (i % 5 + 1) as u16,
                (i % 7 + 1) as u16,
            )
        })
        .collect();
    assert_matches_linear_search(&grid(9, 17, cells));
}

#[test]
fn maximum_sparse_span_storage_depends_on_rows_and_placements() {
    let grid = grid(
        usize::from(u16::MAX),
        u16::MAX,
        vec![placement(0, 0, u16::MAX, u16::MAX)],
    );
    let index = CellIndex::new(&grid);
    assert!(index.storage_entries() <= 16);
    assert_eq!(index.at(0, 0), Some(0));
    assert_eq!(index.at(u16::MAX - 1, u16::MAX - 1), Some(0));
    assert_eq!(index.at(u16::MAX, 0), None);
    assert_eq!(index.at(0, u16::MAX), None);
}

#[test]
fn indexed_search_preserves_overlaps_and_holes_after_a_large_prefix() {
    let mut cells: Vec<_> = (0..128).map(|column| placement(0, column, 1, 1)).collect();
    cells.push(placement(3, 129, 1, 2));
    cells.push(placement(0, 128, 8, 4));
    let grid = grid(8, 132, cells);
    let index = CellIndex::new(&grid);
    assert_eq!(index.at(3, 129), Some(128));
    assert_eq!(index.at(3, 128), Some(129));
    assert_eq!(index.at(4, 129), Some(129));
    assert_eq!(index.at(3, 131), Some(129));
    assert_eq!(index.at(3, 127), None);
    assert_matches_linear_search(&grid);
}

#[test]
fn highest_representable_row_can_own_a_cell() {
    let grid = grid(
        usize::from(u16::MAX) + 1,
        u16::MAX,
        vec![placement(u16::MAX, u16::MAX - 1, 1, 1)],
    );
    let index = CellIndex::new(&grid);
    assert_eq!(index.storage_entries(), 1);
    assert_eq!(index.at(u16::MAX, u16::MAX - 1), Some(0));
    assert_eq!(index.at(u16::MAX - 1, u16::MAX - 1), None);
}

#[test]
fn earliest_spanning_cell_has_constant_lookup_work() {
    for rows in [8, 128, 4096, usize::from(u16::MAX)] {
        let grid = grid(
            rows,
            u16::MAX,
            vec![
                placement(0, 0, rows as u16, u16::MAX),
                placement(1, 1, 1, 1),
            ],
        );
        let index = CellIndex::new(&grid);
        TABLE_CELL_LOOKUP_STEPS.with(|steps| steps.set(0));
        for (row, col) in [(0, 0), (1, 1), (rows as u16 - 1, u16::MAX - 1)] {
            assert_eq!(index.at(row, col), Some(0));
        }
        let steps = TABLE_CELL_LOOKUP_STEPS.with(|steps| steps.get());
        eprintln!("{rows} rows: {steps} earliest-span lookup steps");
        assert!(steps <= 3);
    }
}

#[test]
fn later_spanning_cells_keep_lookup_work_bounded_by_their_position() {
    for rows in [8, 128, 4096, usize::from(u16::MAX)] {
        for earlier in [1, 2, 8, 32, 128] {
            let mut cells: Vec<_> = (0..earlier)
                .map(|column| placement(0, column as u16, 1, 1))
                .collect();
            cells.push(placement(1, 1, rows as u16 - 2, u16::MAX - 1));
            cells.push(placement(1, 1, 1, 1));
            let grid = grid(rows, u16::MAX, cells);
            let index = CellIndex::new(&grid);
            TABLE_CELL_LOOKUP_STEPS.with(|steps| steps.set(0));
            for (row, col) in [(1, 1), (rows as u16 - 2, u16::MAX - 1), (1, 2)] {
                assert_eq!(index.at(row, col), Some(earlier));
            }
            let steps = TABLE_CELL_LOOKUP_STEPS.with(|steps| steps.get());
            eprintln!("{rows} rows, {earlier} earlier cells: {steps} lookup steps");
            assert!(steps <= 6 * (earlier + 1));
        }
    }
}

#[test]
fn single_spanning_cell_and_holes_have_constant_lookup_work() {
    for rows in [8, 128, 4096, usize::from(u16::MAX)] {
        let grid = grid(rows, 8, vec![placement(1, 1, rows as u16 - 2, 3)]);
        let index = CellIndex::new(&grid);
        TABLE_CELL_LOOKUP_STEPS.with(|steps| steps.set(0));
        assert_eq!(index.at(1, 1), Some(0));
        assert_eq!(index.at(0, 1), None);
        assert_eq!(index.at(rows as u16 - 1, 1), None);
        assert_eq!(index.at(2, 4), None);
        let steps = TABLE_CELL_LOOKUP_STEPS.with(|steps| steps.get());
        eprintln!("{rows} rows: {steps} single-cell/hole lookup steps");
        assert!(steps <= 4);
    }
}

#[test]
fn empty_and_clipped_ranges_are_safe() {
    for grid in [grid(0, 3, vec![]), grid(3, 0, vec![])] {
        let index = CellIndex::new(&grid);
        assert_eq!(index.at(0, 0), None);
        assert_eq!(index.storage_entries(), 0);
    }
    assert_matches_linear_search(&grid(
        4,
        5,
        vec![
            placement(0, 0, 0, 3),
            placement(0, 0, 3, 0),
            placement(9, 0, 1, 1),
            placement(0, 9, 1, 1),
            placement(2, 3, u16::MAX, u16::MAX),
        ],
    ));
    assert_matches_linear_search(&grid(
        4,
        5,
        vec![placement(2, 3, u16::MAX, u16::MAX), placement(0, 0, 4, 5)],
    ));
}

#[test]
fn built_grid_clips_spans_at_row_groups_before_indexing() {
    for prepared in [false, true] {
        let mut doc = Document::new();
        let table = append_grid_part(&mut doc, 0, "table", DisplayValue::Table, "display:table");
        let mut ids = Vec::new();
        for _ in 0..2 {
            let group = append_grid_part(
                &mut doc,
                table,
                "tbody",
                DisplayValue::TableRowGroup,
                "display:table-row-group",
            );
            for row in 0..2 {
                let tr = append_grid_part(
                    &mut doc,
                    group,
                    "tr",
                    DisplayValue::TableRow,
                    "display:table-row",
                );
                let cell = append_grid_part(
                    &mut doc,
                    tr,
                    "td",
                    DisplayValue::TableCell,
                    "display:table-cell",
                );
                if row == 0 {
                    doc.set_element_attribute(cell, "rowspan", "8").unwrap();
                    doc.set_element_attribute(cell, "colspan", "2").unwrap();
                }
                ids.push(cell);
            }
        }
        if prepared {
            doc.mark_in_document_flags();
            let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
            crate::layout::apply_computed_to_style(&mut doc, &computed).unwrap();
        }
        let grid = super::super::build_table_grid(&doc, table).unwrap();
        let index = CellIndex::new(&grid);
        assert_eq!(grid.n_cols, 3);
        assert_eq!(grid.cells[0].row_span, 2);
        assert_eq!(grid.cells[2].row_span, 2);
        for row in 0..4 {
            assert_eq!(
                grid.cells[index.at(row, 0).unwrap()].node_id,
                ids[(row / 2 * 2) as usize]
            );
            assert_eq!(index.at(row, 2), (row % 2 == 1).then_some(row as usize));
        }
        assert_matches_linear_search(&grid);
    }
}

#[test]
fn fixed_and_auto_keep_collapsed_geometry_and_painted_widths() {
    for mode in ["fixed", "auto"] {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let table = append_grid_part(
            &mut doc,
            body,
            "table",
            DisplayValue::Table,
            &format!("display:table;table-layout:{mode};width:80px;border-collapse:collapse"),
        );
        let mut cells = Vec::new();
        for _ in 0..2 {
            let row = append_grid_part(
                &mut doc,
                table,
                "tr",
                DisplayValue::TableRow,
                "display:table-row",
            );
            for _ in 0..2 {
                cells.push(append_grid_part(
                    &mut doc,
                    row,
                    "td",
                    DisplayValue::TableCell,
                    "display:table-cell;height:10px;border:4px solid black",
                ));
            }
        }
        doc.mark_in_document_flags();
        let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
        crate::layout::layout_single_page(&mut doc, &computed, PageBox::A4).unwrap();
        assert_eq!(
            doc.nodes[table].unrounded_layout.size,
            Size {
                width: 80.0,
                height: 40.0
            }
        );
        for (i, cell) in cells.into_iter().enumerate() {
            let layout = doc.nodes[cell].unrounded_layout;
            assert_eq!(
                layout.size,
                Size {
                    width: 38.0,
                    height: 18.0
                }
            );
            assert_eq!(
                layout.location,
                taffy::Point {
                    x: 4.0 + (i % 2) as f32 * 34.0,
                    y: 4.0 + (i / 2) as f32 * 14.0
                }
            );
            let painted = doc.nodes[cell].collapsed_border().unwrap();
            assert_eq!(painted.top.width().px(), 4.0);
            assert_eq!(painted.right.width().px(), 4.0);
            assert_eq!(painted.bottom.width().px(), 4.0);
            assert_eq!(painted.left.width().px(), 4.0);
        }
    }
}

#[test]
fn collapsed_boundary_search_work_grows_with_grid_area() {
    for side in [8u16, 16, 32] {
        let mut doc = Document::new();
        let mut cells = Vec::new();
        for row in 0..side {
            for col in 0..side {
                let id = collapsed_cell(
                    &mut doc,
                    collapsed_candidate(BorderStyle::Solid, 2.0, CssColor::BLACK),
                );
                cells.push(make_cell(id, row, col, 1, 1, Dimension::auto()));
            }
        }
        let grid = super::super::TableGrid {
            n_cols: side,
            rows: vec![0; usize::from(side)],
            cells,
            col_widths: vec![],
        };
        let index = CellIndex::new(&grid);
        TABLE_CELL_LOOKUP_STEPS.with(|steps| steps.set(0));
        super::super::resolve_collapsed_cell_borders(&mut doc, &grid, &index);
        let lines = super::super::compute_collapsed_lines(&doc, &grid, &index, &Rect::ZERO, None);
        let visits = TABLE_CELL_LOOKUP_STEPS.with(|steps| steps.get());
        let area = usize::from(side).pow(2);
        eprintln!("{side}x{side}: {visits} cell lookup steps");
        assert_eq!(lines.col_overlaps, vec![2.0; usize::from(side - 1)]);
        assert_eq!(lines.row_overlaps, vec![2.0; usize::from(side - 1)]);
        assert_eq!(lines.outer_left, 2.0);
        assert_eq!(lines.outer_right, 2.0);
        assert_eq!(lines.outer_top, 2.0);
        assert_eq!(lines.outer_bottom, 2.0);
        // Count both the linear and indexed work in the adaptive search.
        assert!(visits <= area * (2 * side.ilog2() as usize + 4) * 24);
    }
}
