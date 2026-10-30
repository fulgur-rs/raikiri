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
        specified_min_width: Dimension::auto(),
        resolved: None,
        natural_height: 0.0,
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
