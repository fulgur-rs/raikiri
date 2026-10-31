use super::*;
use crate::table_reservation_diagnostics::{self as diagnostics, ReservationWork};
use raikiri_layout_style::values::Dimension;

fn placement(row: u16, col_span: u16, row_span: u16) -> CellPlacement {
    CellPlacement {
        node_id: 0,
        row,
        col_start: 0,
        col_span,
        row_span,
        specified_width: Dimension::auto(),
        percent_border_padding: 0.0,
        specified_min_width: Dimension::auto(),
        resolved: None,
        natural_height: 0.0,
        flow_bottom: None,
        baseline: 0.0,
        baseline_from_flow: false,
        row_pass: None,
    }
}

fn prefix_case(count: u16) -> (Document, usize, Vec<usize>, Vec<CellPlacement>) {
    let mut doc = Document::new();
    let table = doc.append_element(Some(0), "table", Some("display:table"));
    let rows = (0..count)
        .map(|_| doc.append_element(Some(table), "tr", Some("display:table-row")))
        .collect();
    let mut cells = vec![placement(0, count, count)];
    cells.extend((1..count).map(|row| placement(row, 1, 1)));
    (doc, table, rows, cells)
}

#[test]
fn optional_reservation_diagnostics_preserve_column_placements() {
    let (doc, table, rows, mut cells) = prefix_case(8);
    assert_eq!(diagnostics::finish(), ReservationWork::default());
    let mut uncounted = cells.clone();
    assert_eq!(
        reserve_columns(&doc, table, &rows, &mut uncounted).unwrap(),
        9
    );
    assert_eq!(diagnostics::finish(), ReservationWork::default());
    diagnostics::begin();
    assert_eq!(reserve_columns(&doc, table, &rows, &mut cells).unwrap(), 9);
    let work = diagnostics::finish();
    assert_eq!(work.passes, 1);
    assert!(work.occupied_slots + work.search_nodes > 0);
    assert!(work.slot_updates + work.update_nodes > 0);
    assert!(format!("{work:?}").contains("passes: 1"));
    for (actual, expected) in cells.iter().zip(uncounted) {
        assert_eq!(
            (actual.col_start, actual.row_span),
            (expected.col_start, expected.row_span)
        );
    }
    assert_eq!(diagnostics::finish(), ReservationWork::default());
}

#[test]
fn occupied_prefix_search_work_grows_with_rows_and_tree_height() {
    for count in [32_u16, 64, 128] {
        let (doc, table, rows, mut cells) = prefix_case(count);
        diagnostics::begin();
        assert_eq!(
            reserve_columns(&doc, table, &rows, &mut cells).unwrap(),
            count + 1
        );
        let work = diagnostics::finish();
        assert_eq!(work.passes, 1);
        assert!(cells.iter().skip(1).all(|cell| cell.col_start == count));
        let height = u64::from(count.ilog2() + 2);
        assert!(
            work.occupied_slots + work.search_nodes <= 4 * u64::from(count) * height,
            "occupied prefixes must not be scanned again for every row: {work:?}"
        );
    }
}

#[test]
fn counted_growth_preserves_reservations_and_reports_copied_nodes() {
    let (doc, table, rows, _) = prefix_case(4);
    let mut cells = vec![
        placement(0, 3, 4),
        placement(0, 5, 4),
        placement(1, 1, 1),
        placement(2, 1, 1),
        placement(3, 1, 1),
    ];
    diagnostics::begin();
    assert_eq!(reserve_columns(&doc, table, &rows, &mut cells).unwrap(), 9);
    let work = diagnostics::finish();
    assert_eq!(work.copied_nodes, 7);
    assert!(work.search_nodes > 0 && work.update_nodes > 0);
    assert_eq!(
        cells.iter().map(|cell| cell.col_start).collect::<Vec<_>>(),
        [0, 3, 8, 8, 8]
    );
}

#[test]
fn single_row_spans_do_not_create_reservation_index_work() {
    let (doc, table, rows, mut cells) = prefix_case(8);
    for cell in &mut cells {
        cell.row_span = 1;
    }
    diagnostics::begin();
    assert_eq!(reserve_columns(&doc, table, &rows, &mut cells).unwrap(), 8);
    assert_eq!(
        diagnostics::finish(),
        ReservationWork {
            passes: 1,
            ..Default::default()
        }
    );
    assert!(cells.iter().skip(1).all(|cell| cell.col_start == 0));
}

fn assert_matches_legacy(doc: &Document, table: usize, rows: &[usize], cells: &[CellPlacement]) {
    let mut old = cells.to_vec();
    let mut new = cells.to_vec();
    let expected =
        legacy_reserve_columns(doc, table, rows, &mut old).map_err(|error| format!("{error:?}"));
    let actual = reserve_columns(doc, table, rows, &mut new).map_err(|error| format!("{error:?}"));
    assert_eq!(actual, expected);
    for (actual, expected) in new.iter().zip(old) {
        assert_eq!(
            (
                actual.row,
                actual.col_start,
                actual.col_span,
                actual.row_span
            ),
            (
                expected.row,
                expected.col_start,
                expected.col_span,
                expected.row_span
            )
        );
    }
}

#[test]
fn reservation_placements_match_legacy_with_holes_expiry_and_overlaps() {
    let (doc, table, rows, _) = prefix_case(6);
    let cases = [
        vec![
            placement(0, 1, 4),
            placement(0, 1, 1),
            placement(0, 1, 3),
            placement(1, 3, 1),
            placement(2, 1, 1),
            placement(2, 1, 1),
            placement(3, 2, 1),
            placement(4, 1, 1),
        ],
        vec![
            placement(0, 1, 2),
            placement(0, 1, 1),
            placement(0, 1, 6),
            placement(1, 3, 2),
            placement(2, 1, 1),
            placement(2, 1, 1),
            placement(3, 2, 1),
            placement(4, 1, 1),
        ],
    ];
    for cells in cases {
        assert_matches_legacy(&doc, table, &rows, &cells);
    }
    for initial in [1_u64, 77, 1223] {
        let mut seed = initial;
        for _ in 0..32 {
            let mut cells = Vec::new();
            for row in 0..6 {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                for _ in 0..1 + seed % 3 {
                    seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                    let span = 1 + (seed % 5) as u16;
                    seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                    cells.push(placement(row, span, 1 + (seed % 9) as u16));
                }
            }
            assert_matches_legacy(&doc, table, &rows, &cells);
        }
    }
}

#[test]
fn reservation_bounds_match_legacy_at_65535_columns() {
    let (doc, table, rows, _) = prefix_case(2);
    for width in [65534_u16, 65535] {
        let mut cells = Vec::new();
        let mut remaining = width;
        while remaining > 0 {
            let span = remaining.min(1000);
            cells.push(placement(0, span, 2));
            remaining -= span;
        }
        cells.push(placement(1, 1, 1));
        assert_matches_legacy(&doc, table, &rows, &cells);
        let result = reserve_columns(&doc, table, &rows, &mut cells);
        if width == 65534 {
            assert_eq!(result.unwrap(), 65535);
            assert_eq!(cells.last().unwrap().col_start, 65534);
        } else {
            assert!(result.is_err());
        }
    }
}

#[test]
fn zero_rowspan_sentinel_stops_at_the_source_row_group_end() {
    let mut doc = Document::new();
    let table = doc.append_element(Some(0), "table", Some("display:table"));
    let mut rows = Vec::new();
    for _ in 0..2 {
        let group = doc.append_element(Some(table), "tbody", Some("display:table-row-group"));
        doc.set_display(group, DisplayValue::TableRowGroup);
        for _ in 0..2 {
            rows.push(doc.append_element(Some(group), "tr", Some("display:table-row")));
        }
    }
    let cell = doc.append_element(Some(rows[0]), "td", None::<&str>);
    doc.set_element_attribute(cell, "rowspan", "0").unwrap();
    let mut cells = vec![
        placement(0, 2, super::super::get_rowspan(&doc, cell)),
        placement(1, 1, 1),
        placement(2, 1, 1),
        placement(3, 1, 1),
    ];
    assert_matches_legacy(&doc, table, &rows, &cells);
    assert_eq!(reserve_columns(&doc, table, &rows, &mut cells).unwrap(), 3);
    assert_eq!(cells[0].row_span, 2);
    assert_eq!(
        cells.iter().map(|cell| cell.col_start).collect::<Vec<_>>(),
        [0, 2, 0, 0]
    );
}

fn compare_tree_with_slots(tree: &ColumnReservations, slots: &[usize]) {
    for row in 0..=20 {
        for start in 0..slots.len() + 3 {
            let expected = (start..slots.len())
                .find(|&column| slots[column] <= row)
                .unwrap_or(start.max(slots.len()));
            assert_eq!(
                tree.first_free::<false>(start, row),
                expected,
                "start={start} row={row} slots={slots:?}"
            );
        }
    }
}

fn raise_and_compare(
    tree: &mut ColumnReservations,
    slots: &mut Vec<usize>,
    start: usize,
    end: usize,
    expiry: usize,
) {
    tree.reserve_range::<false>(start, end, expiry);
    slots.resize(slots.len().max(end), 0);
    for slot in &mut slots[start..end] {
        *slot = (*slot).max(expiry);
    }
    compare_tree_with_slots(tree, slots);
}

#[test]
fn lower_bounds_survive_partial_updates_and_capacity_growth() {
    let mut tree = ColumnReservations::default();
    let mut slots = Vec::new();
    compare_tree_with_slots(&tree, &slots);
    for (start, end, expiry) in [
        (0, 4, 12),
        (1, 3, 3),
        (8, 11, 7),
        (2, 10, 9),
        (0, 32, 20),
        (3, 7, 2),
        (64, 65, 4),
        (1, 70, 16),
    ] {
        raise_and_compare(&mut tree, &mut slots, start, end, expiry);
    }
}

#[test]
fn lower_bound_tree_matches_slot_oracle_for_deterministic_overlaps() {
    for width in [7, 31, 127] {
        for initial in [1_u64, 7919] {
            let mut tree = ColumnReservations::default();
            let mut slots = Vec::new();
            let mut seed = initial;
            for _ in 0..96 {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                let end = 1 + seed as usize % width;
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                let start = seed as usize % end;
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                let expiry = seed as usize % 21;
                raise_and_compare(&mut tree, &mut slots, start, end, expiry);
            }
        }
    }
}

#[test]
fn lower_bound_tree_handles_empty_updates_and_column_limit() {
    let mut tree = ColumnReservations::default();
    tree.reserve_range::<false>(0, 0, 8);
    assert_eq!(tree.capacity(), 0);
    tree.reserve_range::<false>(0, 65535, 10);
    for start in [0, 1, 1000, 65534, 65535, 65536] {
        assert_eq!(tree.first_free::<false>(start, 9), start.max(65535));
        assert_eq!(tree.first_free::<false>(start, 10), start);
    }
}

fn legacy_reserve_columns(
    doc: &Document,
    table: usize,
    rows: &[usize],
    cells: &mut [CellPlacement],
) -> Result<u16, LayoutError> {
    let groups: Vec<_> = rows.iter().map(|&row| row_group(doc, table, row)).collect();
    let mut group_ends = vec![rows.len(); rows.len()];
    let mut end = rows.len();
    for row in (0..rows.len()).rev() {
        if row + 1 < rows.len() && groups[row] != groups[row + 1] {
            end = row + 1;
        }
        group_ends[row] = end;
    }

    // Each slot stores the first row after the last cell covering that slot.
    let mut occupied_until: Vec<usize> = Vec::new();
    let mut current_row = None;
    let mut column = 0_u16;
    let mut column_count = 0_u16;
    for cell in cells {
        let row = usize::from(cell.row);
        if current_row != Some(row) {
            current_row = Some(row);
            column = 0;
        }
        while occupied_until
            .get(usize::from(column))
            .is_some_and(|&end| end > row)
        {
            column = next_table_column(column, 1)?;
        }
        let next = next_table_column(column, cell.col_span)?;
        cell.col_start = column;
        cell.row_span = usize::from(cell.row_span).min(group_ends[row] - row) as u16;
        occupied_until.resize(occupied_until.len().max(usize::from(next)), 0);
        for slot in &mut occupied_until[usize::from(column)..usize::from(next)] {
            // Invalid overlapping spans must not erase an older reservation.
            *slot = (*slot).max(row + usize::from(cell.row_span));
        }
        column_count = column_count.max(next);
        column = next;
    }
    Ok(column_count)
}
