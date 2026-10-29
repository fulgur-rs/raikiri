use super::super::BORDER_RESOLUTIONS;
use super::*;
use crate::layout::test_support::TABLE_CELL_LOOKUP_STEPS;

fn cold_layout(rows: usize, columns: usize, span: usize) -> (usize, usize) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block;margin:0"));
    let table = doc.append_element(Some(body), "table", Some("display:table;border-collapse:collapse;table-layout:fixed;width:600px;border:3px solid red"));
    for _ in 0..rows {
        let row = doc.append_element(Some(table), "tr", Some("display:table-row"));
        for _ in 0..columns {
            let cell = doc.append_element(
                Some(row),
                "td",
                Some("display:table-cell;height:1px;border:2px solid blue"),
            );
            doc.set_element_attribute(cell, "colspan", span.to_string())
                .unwrap();
        }
    }
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    super::super::boundaries::WORK.set([0; 5]);
    TABLE_CELL_LOOKUP_STEPS.set(0);
    BORDER_RESOLUTIONS.set(0);
    crate::layout::layout_single_page(&mut doc, &computed, PageBox::A4).unwrap();
    assert!(doc.nodes[table].layout.size.width.is_finite());
    eprintln!(
        "boundary work [interval visits, merge steps, emitted segments, retained bytes, builds]={:?}",
        super::super::boundaries::WORK.get()
    );
    (TABLE_CELL_LOOKUP_STEPS.get(), BORDER_RESOLUTIONS.get())
}

#[test]
fn cold_sparse_colspan_borders_do_not_visit_virtual_cell_area() {
    let mut excessive = Vec::new();
    for rows in [8, 32] {
        for span in [16, 1000] {
            let (lookups, resolutions) = cold_layout(rows, 1, span);
            eprintln!(
                "cold rows={rows}, columns={span}, cells={rows}: lookups={lookups}, resolutions={resolutions}"
            );
            assert!(resolutions > 0);
            let work = super::super::boundaries::WORK.get();
            assert!(work[0] <= 128 * rows);
            assert!(work[2] <= 4 * rows);
            assert_eq!(work[4], 1);
            if lookups > 128 * rows + 8 * (rows + span) {
                excessive.push((rows, span, lookups));
            }
        }
    }
    assert!(
        excessive.is_empty(),
        "excessive cold lookup work: {excessive:?}"
    );
}

#[test]
fn cold_dense_border_control_has_bounded_lookup_work() {
    for (rows, columns) in [(8, 8), (32, 16)] {
        let (lookups, resolutions) = cold_layout(rows, columns, 1);
        eprintln!("cold dense {rows}x{columns}: lookups={lookups}, resolutions={resolutions}");
        assert!(resolutions > 0);
        assert!(lookups <= 256 * rows * columns);
    }
}

fn random(seed: &mut u64, limit: usize) -> usize {
    *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
    ((*seed >> 32) as usize) % limit
}

fn synthetic_grid(
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

fn coordinate_pairs(
    grid: &super::super::TableGrid,
    index: &super::super::CellIndex,
    saturating: bool,
) -> Vec<(bool, usize, usize, usize, usize)> {
    let mut pairs = Vec::new();
    for boundary in 1..usize::from(grid.n_cols) {
        for row in 0..grid.rows.len() {
            let a = index.at(row as u16, (boundary - 1) as u16);
            let b = index.at(row as u16, boundary as u16);
            if let (Some(a), Some(b)) = (a, b) {
                let (left, right) = (&grid.cells[a], &grid.cells[b]);
                if usize::from(left.col_start + left.col_span) == boundary
                    && usize::from(right.col_start) == boundary
                {
                    pairs.push((false, boundary, row, a, b));
                }
            }
        }
    }
    for boundary in 1..grid.rows.len() {
        for col in 0..grid.n_cols {
            if let (Some(a), Some(b)) = (
                index.at((boundary - 1) as u16, col),
                index.at(boundary as u16, col),
            ) {
                let (above, below) = (&grid.cells[a], &grid.cells[b]);
                let end = if saturating {
                    usize::from(above.row.saturating_add(above.row_span))
                } else {
                    usize::from(above.row) + usize::from(above.row_span)
                };
                if end == boundary && usize::from(below.row) == boundary {
                    pairs.push((true, boundary, usize::from(col), a, b));
                }
            }
        }
    }
    pairs
}

fn segment_pairs(
    grid: &super::super::TableGrid,
    index: &super::super::CellIndex,
    saturating: bool,
) -> Vec<(bool, usize, usize, usize, usize)> {
    let segments = index.boundary_segments(grid).unwrap();
    let mut pairs = Vec::new();
    for segment in &segments.vertical {
        let (a, b, boundary) = (segment.leading, segment.trailing, segment.boundary);
        if boundary == 0 || boundary == usize::from(grid.n_cols) {
            continue;
        }
        if usize::from(grid.cells[a].col_start + grid.cells[a].col_span) == boundary
            && usize::from(grid.cells[b].col_start) == boundary
        {
            for offset in segment.start..segment.end {
                pairs.push((false, boundary, offset, a, b));
            }
        }
    }
    for segment in &segments.horizontal {
        let (a, b, boundary) = (segment.leading, segment.trailing, segment.boundary);
        let above = &grid.cells[a];
        let end = if saturating {
            usize::from(above.row.saturating_add(above.row_span))
        } else {
            usize::from(above.row) + usize::from(above.row_span)
        };
        if end == boundary && usize::from(grid.cells[b].row) == boundary {
            for offset in segment.start..segment.end {
                pairs.push((true, boundary, offset, a, b));
            }
        }
    }
    pairs
}

#[test]
fn sparse_visibility_and_order_match_coordinate_oracle() {
    let mut seed = 979;
    for case in 0..1000 {
        let rows = random(&mut seed, 10);
        let columns = random(&mut seed, 10) as u16;
        let cells = (0..random(&mut seed, 35))
            .map(|node| {
                make_cell(
                    node,
                    random(&mut seed, 12) as u16,
                    random(&mut seed, 12) as u16,
                    random(&mut seed, 6) as u16,
                    random(&mut seed, 6) as u16,
                    Dimension::auto(),
                )
            })
            .collect();
        let grid = synthetic_grid(rows, columns, cells);
        let index = super::super::CellIndex::new(&grid);
        for row in 0..rows {
            let runs = index.row_runs(row, 32768).unwrap();
            for column in 0..columns {
                let actual = runs
                    .iter()
                    .find(|run| run.start <= column && column < run.end)
                    .map(|run| run.cell);
                assert_eq!(
                    actual,
                    index.at(row as u16, column),
                    "case={case} row={row} col={column}"
                );
            }
        }
        for saturating in [false, true] {
            assert_eq!(
                segment_pairs(&grid, &index, saturating),
                coordinate_pairs(&grid, &index, saturating),
                "case={case}"
            );
        }
    }
}

fn line_bits(lines: &super::super::CollapsedLines) -> Vec<u32> {
    lines
        .col_overlaps
        .iter()
        .chain(&lines.row_overlaps)
        .copied()
        .chain([
            lines.outer_left,
            lines.outer_right,
            lines.outer_top,
            lines.outer_bottom,
        ])
        .map(f32::to_bits)
        .collect()
}

#[test]
fn sparse_full_borders_and_lines_match_old_path() {
    let mut seed = 8979;
    for case in 0..120 {
        let rows = random(&mut seed, 8);
        let columns = random(&mut seed, 8) as u16;
        let mut doc = Document::new();
        let cells = (0..random(&mut seed, 25))
            .map(|i| {
                let style = ["solid", "dashed", "hidden", "none"][i % 4];
                let color = ["red", "blue", "green"][i % 3];
                let node = doc.append_element(
                    Some(0),
                    "td",
                    Some(format!(
                        "display:table-cell;border:{}px {style} {color}",
                        i % 3
                    )),
                );
                make_cell(
                    node,
                    random(&mut seed, 10) as u16,
                    random(&mut seed, 10) as u16,
                    random(&mut seed, 5) as u16,
                    random(&mut seed, 5) as u16,
                    Dimension::auto(),
                )
            })
            .collect();
        doc.mark_in_document_flags();
        let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
        crate::layout::apply_computed_to_style(&mut doc, &computed).unwrap();
        let grid = synthetic_grid(rows, columns, cells);
        with_layout_session(&mut doc, |session| {
            let sparse = super::super::CellIndex::new(&grid);
            let old = super::super::CellIndex::new(&grid);
            old.disable_sparse();
            let mut expected = super::super::BorderState::retained(session, &grid);
            let mut actual = expected.clone();
            super::super::resolve_collapsed_cell_borders(session, &mut expected, &grid, &old);
            super::super::resolve_collapsed_cell_borders(session, &mut actual, &grid, &sparse);
            assert_eq!(actual.cells, expected.cells, "case={case}");
            let outer = Edges {
                left: 1.0,
                right: 2.0,
                top: 3.0,
                bottom: 4.0,
            };
            let expected_lines = super::super::compute_collapsed_lines(
                session, &expected, &grid, &old, &outer, None,
            );
            let actual_lines = super::super::compute_collapsed_lines(
                session, &actual, &grid, &sparse, &outer, None,
            );
            assert_eq!(
                line_bits(&actual_lines),
                line_bits(&expected_lines),
                "case={case}"
            );
            // Width arithmetic preserves NaN payloads and signed zero bitwise.
            for (i, cell) in actual.cells.iter_mut().enumerate() {
                let widths = [f32::from_bits(0x7fc00001), -0.0, 0.0, 2.0];
                cell.widths = Edges {
                    left: LengthPercentage::length(widths[i % 4]),
                    right: LengthPercentage::length(widths[(i + 1) % 4]),
                    top: LengthPercentage::length(widths[(i + 2) % 4]),
                    bottom: LengthPercentage::length(widths[(i + 3) % 4]),
                };
            }
            let mut expected_nonfinite = actual.clone();
            super::super::resolve_collapsed_cell_borders(
                session,
                &mut expected_nonfinite,
                &grid,
                &old,
            );
            super::super::resolve_collapsed_cell_borders(session, &mut actual, &grid, &sparse);
            for (actual, expected) in actual.cells.iter().zip(&expected_nonfinite.cells) {
                assert_eq!(actual.paint, expected.paint, "nonfinite paint case={case}");
                let bits = |widths: Edges<LengthPercentage>| {
                    [widths.left, widths.right, widths.top, widths.bottom]
                        .map(|width| super::super::resolve_length(width, None).to_bits())
                };
                assert_eq!(
                    bits(actual.widths),
                    bits(expected.widths),
                    "nonfinite widths case={case}"
                );
            }
            let outer = Edges {
                left: -0.0,
                right: f32::from_bits(0x7fc00002),
                top: 0.0,
                bottom: -0.0,
            };
            let expected_lines =
                super::super::compute_collapsed_lines(session, &actual, &grid, &old, &outer, None);
            let actual_lines = super::super::compute_collapsed_lines(
                session, &actual, &grid, &sparse, &outer, None,
            );
            assert_eq!(
                line_bits(&actual_lines),
                line_bits(&expected_lines),
                "nonfinite case={case}"
            );
        });
    }
}

#[test]
fn sparse_limits_and_extreme_row_eligibility_preserve_old_domain() {
    for rows in [65535, 65536, 65537] {
        let grid = synthetic_grid(rows, 1, vec![]);
        let index = super::super::CellIndex::new(&grid);
        assert_eq!(index.boundary_segments(&grid).is_some(), rows <= 65536);
    }
    // An earlier bottom-row placement exposes an overflowing earlier-row span.
    let grid = synthetic_grid(
        65536,
        1,
        vec![
            make_cell(0, 65535, 0, 1, 1, Dimension::auto()),
            make_cell(1, 65534, 0, 1, 3, Dimension::auto()),
        ],
    );
    let index = super::super::CellIndex::new(&grid);
    assert_eq!(
        segment_pairs(&grid, &index, true),
        coordinate_pairs(&grid, &index, true)
    );
    assert_eq!(
        segment_pairs(&grid, &index, false),
        coordinate_pairs(&grid, &index, false)
    );
    assert_ne!(
        segment_pairs(&grid, &index, true),
        segment_pairs(&grid, &index, false)
    );
    let grid = synthetic_grid(
        8,
        8,
        (0..8)
            .map(|row| make_cell(row, row as u16, 0, 8, 1, Dimension::auto()))
            .collect(),
    );
    let index = super::super::CellIndex::new(&grid);
    assert!(super::super::boundaries::BoundarySegments::new_bounded(&grid, &index, 1).is_none());
    assert!(index.row_runs(0, 0).is_none());
}

#[test]
fn adversarial_segment_payload_is_bounded_and_failed_build_is_memoized() {
    let mut cells: Vec<_> = (0..500)
        .map(|column| make_cell(column, 0, column as u16, 1, 1000, Dimension::auto()))
        .collect();
    cells.extend((0..1000).map(|row| make_cell(500 + row, row as u16, 0, 0, 1, Dimension::auto())));
    let grid = synthetic_grid(1000, 1000, cells);
    let index = super::super::CellIndex::new(&grid);
    super::super::boundaries::WORK.set([0; 5]);
    assert!(index.boundary_segments(&grid).is_none());
    let first = super::super::boundaries::WORK.get();
    assert_eq!(first[4], 1);
    assert!(
        first[2]
            <= 2 * (2 * 1024 * 1024 / std::mem::size_of::<super::super::boundaries::Segment>())
    );
    assert!(index.boundary_segments(&grid).is_none());
    assert_eq!(super::super::boundaries::WORK.get(), first);
    eprintln!("adversarial capped work={first:?}");
}

#[test]
fn cold_single_cell_rowspan_work_is_independent_of_virtual_rows() {
    for rows in [16, 1000] {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Some("display:block"));
        let body = doc.append_element(Some(html), "body", Some("display:block;margin:0"));
        let table = doc.append_element(
            Some(body),
            "table",
            Some("display:table;border-collapse:collapse;table-layout:fixed;width:600px"),
        );
        for row in 0..rows {
            let tr = doc.append_element(Some(table), "tr", Some("display:table-row"));
            if row == 0 {
                let cell = doc.append_element(
                    Some(tr),
                    "td",
                    Some("display:table-cell;height:10px;border:2px solid blue"),
                );
                doc.set_element_attribute(cell, "rowspan", rows.to_string())
                    .unwrap();
                doc.set_element_attribute(cell, "colspan", "1000").unwrap();
            }
        }
        doc.mark_in_document_flags();
        let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
        super::super::boundaries::WORK.set([0; 5]);
        TABLE_CELL_LOOKUP_STEPS.set(0);
        BORDER_RESOLUTIONS.set(0);
        crate::layout::layout_single_page(&mut doc, &computed, PageBox::A4).unwrap();
        let work = super::super::boundaries::WORK.get();
        assert_eq!(TABLE_CELL_LOOKUP_STEPS.get(), 0);
        assert_eq!(BORDER_RESOLUTIONS.get(), 1);
        assert!(work[0] <= 16);
        assert_eq!(work[2], 2);
        assert_eq!(work[4], 1);
        eprintln!("cold single cell rows={rows}, columns=1000: work={work:?}");
    }
}

#[test]
fn repeated_pair_after_another_pair_and_overflowing_row_end_keep_full_borders() {
    for extreme in [false, true] {
        let mut doc = Document::new();
        let ids: Vec<_> = ["red", "blue", "green"]
            .into_iter()
            .map(|color| {
                doc.append_element(
                    Some(0),
                    "td",
                    Some(format!("display:table-cell;border:2px solid {color}")),
                )
            })
            .collect();
        let grid = if extreme {
            synthetic_grid(
                65536,
                1,
                vec![
                    make_cell(ids[0], 65535, 0, 1, 1, Dimension::auto()),
                    make_cell(ids[1], 65534, 0, 1, 3, Dimension::auto()),
                ],
            )
        } else {
            synthetic_grid(
                3,
                2,
                vec![
                    make_cell(ids[0], 1, 1, 1, 1, Dimension::auto()),
                    make_cell(ids[1], 0, 1, 1, 3, Dimension::auto()),
                    make_cell(ids[2], 0, 0, 1, 3, Dimension::auto()),
                ],
            )
        };
        doc.mark_in_document_flags();
        let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
        crate::layout::apply_computed_to_style(&mut doc, &computed).unwrap();
        with_layout_session(&mut doc, |session| {
            let sparse = super::super::CellIndex::new(&grid);
            let old = super::super::CellIndex::new(&grid);
            old.disable_sparse();
            let mut expected = super::super::BorderState::retained(session, &grid);
            let mut actual = expected.clone();
            super::super::resolve_collapsed_cell_borders(session, &mut expected, &grid, &old);
            super::super::resolve_collapsed_cell_borders(session, &mut actual, &grid, &sparse);
            assert_eq!(actual.cells, expected.cells);
            let outer = Edges {
                left: -0.0,
                right: 0.0,
                top: 1.0,
                bottom: 2.0,
            };
            let expected = super::super::compute_collapsed_lines(
                session, &expected, &grid, &old, &outer, None,
            );
            let actual = super::super::compute_collapsed_lines(
                session, &actual, &grid, &sparse, &outer, None,
            );
            assert_eq!(line_bits(&actual), line_bits(&expected));
        });
    }
}

#[test]
fn vertical_segment_allowance_is_shared_across_row_slabs() {
    let grid = synthetic_grid(
        2,
        2,
        vec![
            make_cell(1, 0, 0, 2, 1, Dimension::auto()),
            make_cell(2, 1, 0, 2, 1, Dimension::auto()),
        ],
    );
    let index = super::super::CellIndex::new(&grid);
    let complete = super::super::boundaries::BoundarySegments::new_bounded(&grid, &index, 4)
        .expect("four vertical edges fit the allowance");
    assert_eq!(complete.vertical.len(), 4);
    assert_eq!(complete.horizontal.len(), 1);
    assert!(super::super::boundaries::BoundarySegments::new_bounded(&grid, &index, 2).is_none());
}

#[test]
fn horizontal_segment_allowance_rejects_partial_gapped_rows() {
    let grid = synthetic_grid(
        3,
        3,
        (0..3)
            .map(|row| make_cell(row as usize, row, 1, 1, 1, Dimension::auto()))
            .collect(),
    );
    let index = super::super::CellIndex::new(&grid);
    let complete = super::super::boundaries::BoundarySegments::new_bounded(&grid, &index, 2)
        .expect("two horizontal pairs fit the allowance");
    assert!(complete.vertical.is_empty());
    assert_eq!(
        complete
            .horizontal
            .iter()
            .map(|segment| (segment.boundary, segment.leading, segment.trailing))
            .collect::<Vec<_>>(),
        [(1, 0, 1), (2, 1, 2)]
    );
    assert!(super::super::boundaries::BoundarySegments::new_bounded(&grid, &index, 1).is_none());
}

#[test]
fn row_run_allowance_limits_the_union_of_tree_levels() {
    let grid = synthetic_grid(
        2,
        5,
        vec![
            make_cell(1, 0, 1, 1, 2, Dimension::auto()),
            make_cell(2, 0, 3, 1, 1, Dimension::auto()),
        ],
    );
    let index = super::super::CellIndex::new(&grid);
    let complete = index.row_runs(0, 2).expect("both disjoint runs fit");
    assert_eq!(
        complete
            .iter()
            .map(|run| (run.start, run.end, run.cell))
            .collect::<Vec<_>>(),
        [(1, 2, 0), (3, 4, 1)]
    );
    assert_eq!(index.row_runs(1, 1).unwrap().len(), 1);
    assert!(index.row_runs(0, 1).is_none());
}
