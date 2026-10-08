//! Column reservations for cells that span materialized table rows.
//!
//! HTML's table model advances past slots already covered in the current row.
//! Keep that reservation pass shared by native and prepared table grids. The
//! grid retains its existing explicit-row approximation: spans end at the
//! source row group, without creating implicit rows beyond its source rows.

use super::{CellPlacement, next_table_column};
use crate::Document;
use raikiri_style::property::DisplayValue;
use raikiri_traits::LayoutError;

fn row_group(doc: &Document, table: usize, marker: usize) -> usize {
    let mut source = doc.ifc_source_owner(marker);
    while source != table {
        if matches!(
            doc.nodes[source].display,
            DisplayValue::TableRowGroup
                | DisplayValue::TableHeaderGroup
                | DisplayValue::TableFooterGroup
        ) {
            return source;
        }
        let Some(parent) = doc.parent_of(source) else {
            break;
        };
        source = parent;
    }
    table
}

pub(super) fn reserve_columns(
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
