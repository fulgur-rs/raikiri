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

#[cfg(test)]
mod tests;

/// Monotone expiry lower bounds, restricted to range raises and first-free queries.
/// A leaf's effective expiry is the maximum on its root-to-leaf path. A search
/// only descends past bounds at most its row, so it never needs to push them.
#[derive(Default)]
struct ColumnReservations {
    lower_bounds: Vec<usize>,
}

impl ColumnReservations {
    fn capacity(&self) -> usize {
        self.lower_bounds.len() / 2
    }

    fn grow<const COUNT: bool>(&mut self, end: usize) {
        let old_capacity = self.capacity();
        if end <= old_capacity {
            return;
        }
        // Reservation ends were checked as u16 column boundaries by the caller.
        let capacity = end.next_power_of_two();
        let old = std::mem::replace(&mut self.lower_bounds, vec![0; 2 * capacity]);
        if old_capacity == 0 {
            return;
        }
        // Move the old root into the leftmost subtree of the same width.
        // Copy entire levels so every existing ancestor bound stays in place.
        let mut old_start = 1;
        let mut new_start = capacity / old_capacity;
        while old_start < old.len() {
            self.lower_bounds[new_start..new_start + old_start]
                .copy_from_slice(&old[old_start..2 * old_start]);
            old_start *= 2;
            new_start *= 2;
        }
        #[cfg(any(test, feature = "table-reservation-diagnostics"))]
        if COUNT {
            crate::table_reservation_diagnostics::record(|work| {
                work.copied_nodes += (old.len() - 1) as u64;
            });
        }
    }

    fn reserve_range<const COUNT: bool>(&mut self, start: usize, end: usize, expiry: usize) {
        if start >= end {
            return;
        }
        self.grow::<COUNT>(end);
        self.raise::<COUNT>(1, 0, self.capacity(), start, end, expiry);
    }

    fn raise<const COUNT: bool>(
        &mut self,
        node: usize,
        low: usize,
        high: usize,
        start: usize,
        end: usize,
        expiry: usize,
    ) {
        #[cfg(any(test, feature = "table-reservation-diagnostics"))]
        if COUNT {
            crate::table_reservation_diagnostics::record(|work| work.update_nodes += 1);
        }
        if end <= low || high <= start || self.lower_bounds[node] >= expiry {
            return;
        }
        if start <= low && high <= end {
            self.lower_bounds[node] = expiry;
            return;
        }
        let middle = low + (high - low) / 2;
        self.raise::<COUNT>(2 * node, low, middle, start, end, expiry);
        self.raise::<COUNT>(2 * node + 1, middle, high, start, end, expiry);
        // Keep the inherited lower bound even when a stale child is smaller.
        self.lower_bounds[node] = self.lower_bounds[node]
            .max(self.lower_bounds[2 * node].min(self.lower_bounds[2 * node + 1]));
    }

    fn first_free<const COUNT: bool>(&self, start: usize, row: usize) -> usize {
        let capacity = self.capacity();
        if start >= capacity {
            return start;
        }
        self.find::<COUNT>(1, 0, capacity, start, row)
            .unwrap_or(capacity)
    }

    fn find<const COUNT: bool>(
        &self,
        node: usize,
        low: usize,
        high: usize,
        start: usize,
        row: usize,
    ) -> Option<usize> {
        #[cfg(any(test, feature = "table-reservation-diagnostics"))]
        if COUNT {
            crate::table_reservation_diagnostics::record(|work| work.search_nodes += 1);
        }
        if high <= start || self.lower_bounds[node] > row {
            return None;
        }
        if high - low == 1 {
            return Some(low);
        }
        let middle = low + (high - low) / 2;
        self.find::<COUNT>(2 * node, low, middle, start, row)
            .or_else(|| self.find::<COUNT>(2 * node + 1, middle, high, start, row))
    }
}

fn row_group(doc: &Document, table: usize, marker: usize) -> usize {
    doc.inclusive_ancestors(doc.ifc_source_owner(marker))
        .take_while(|&source| source != table)
        .find(|&source| {
            matches!(
                doc.nodes[source].display(),
                DisplayValue::TableRowGroup
                    | DisplayValue::TableHeaderGroup
                    | DisplayValue::TableFooterGroup
            )
        })
        .unwrap_or(table)
}

pub(super) fn reserve_columns(
    doc: &Document,
    table: usize,
    rows: &[usize],
    cells: &mut [CellPlacement],
) -> Result<u16, LayoutError> {
    #[cfg(any(test, feature = "table-reservation-diagnostics"))]
    if crate::table_reservation_diagnostics::enabled() {
        return reserve_columns_counted::<true>(doc, table, rows, cells);
    }
    reserve_columns_counted::<false>(doc, table, rows, cells)
}

fn reserve_columns_counted<const COUNT: bool>(
    doc: &Document,
    table: usize,
    rows: &[usize],
    cells: &mut [CellPlacement],
) -> Result<u16, LayoutError> {
    #[cfg(any(test, feature = "table-reservation-diagnostics"))]
    if COUNT {
        crate::table_reservation_diagnostics::record(|work| work.passes += 1);
    }
    let groups: Vec<_> = rows.iter().map(|&row| row_group(doc, table, row)).collect();
    let mut group_ends = vec![rows.len(); rows.len()];
    let mut end = rows.len();
    for row in (0..rows.len()).rev() {
        if row + 1 < rows.len() && groups[row] != groups[row + 1] {
            end = row + 1;
        }
        group_ends[row] = end;
    }

    let mut reservations = ColumnReservations::default();
    let mut current_row = None;
    let mut column = 0_u16;
    let mut column_count = 0_u16;
    // Grid cells arrive in row order; the cursor only advances within a row.
    debug_assert!(cells.windows(2).all(|pair| pair[0].row <= pair[1].row));
    for cell in cells {
        let row = usize::from(cell.row);
        if current_row != Some(row) {
            current_row = Some(row);
            column = 0;
        }
        // Checked exclusive ends leave the terminal u16 boundary unreserved.
        column = u16::try_from(reservations.first_free::<COUNT>(usize::from(column), row))
            .expect("the terminal column boundary remains free");
        let next = next_table_column(column, cell.col_span)?;
        cell.col_start = column;
        cell.row_span = usize::from(cell.row_span).min(group_ends[row] - row) as u16;
        // Single-row reservations expire before the next cursor reset. Omitting
        // them keeps ordinary tables allocation-free here and preserves older
        // overlapping reservations, which only range raises may extend.
        if cell.row_span > 1 {
            reservations.reserve_range::<COUNT>(
                usize::from(column),
                usize::from(next),
                row + usize::from(cell.row_span),
            );
        }
        column_count = column_count.max(next);
        column = next;
    }
    Ok(column_count)
}
