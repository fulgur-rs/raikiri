//! The pages of the table rows the flow is inside.
//!
//! The cells of a row sit side by side, so each of them starts on the page
//! its row starts on (CSS 2.1 §17.5), however far the cells before it in the
//! source ran on. The flow walks the cells one after another: when it enters
//! a later cell of a row it goes back to the row's page, and when it leaves
//! the row it goes on from the last page any of its cells reached.

use super::{Document, HashMap, cell_row_end};
use raikiri_style::property::DisplayValue;

/// The rows the flow is in, outermost first, each with the cell it is in.
#[derive(Default)]
pub(super) struct CellRows {
    open: Vec<CellRow>,
    /// The page each row box placed in the flow starts on, until the flow
    /// enters the row.
    placed: HashMap<usize, u32>,
    /// Whether the document has a table part at all, once known: a cell, or
    /// a box that wraps boxes in anonymous cells (CSS 2.1 §17.2.1).
    has_cells: Option<bool>,
    /// The cells around the candidate being entered, innermost first.
    chain: Vec<(Row, usize)>,
}

/// A row, by the box that holds it and the index in that box after which
/// the boxes follow it (see [`cell_row_end`]): a row element, or the
/// anonymous row around a run of cells in a table or row group.
type Row = (usize, usize);

struct CellRow {
    row: Row,
    /// The cell the flow is in, which is the layout key of an anonymous cell
    /// when one wraps the boxes it is in.
    cell: usize,
    /// The page the row starts on.
    start_page: u32,
    /// The last page that the rows before it and the cells of it that the
    /// flow left reach.
    end_page: u32,
}

impl CellRows {
    /// Records that the row box `row` starts on `page`.
    pub(super) fn place_row(&mut self, row: usize, page: u32) {
        self.placed.insert(row, page);
    }

    /// Enters the rows and cells that hold `node_id`, leaving the ones that
    /// do not, with the flow on `page`; returns the page the flow is on then.
    /// A row placed in the flow starts on the page of its own top, which can
    /// be before the one the rows above it reach, beside a cell that spans
    /// both (CSS 2.1 §17.2.1, `rowspan`).
    pub(super) fn enter(&mut self, document: &Document, node_id: usize, mut page: u32) -> u32 {
        let has_cells = *self.has_cells.get_or_insert_with(|| {
            document.nodes.iter().any(|node| {
                matches!(
                    node.display(),
                    DisplayValue::Table
                        | DisplayValue::InlineTable
                        | DisplayValue::TableRowGroup
                        | DisplayValue::TableHeaderGroup
                        | DisplayValue::TableFooterGroup
                        | DisplayValue::TableRow
                        | DisplayValue::TableCell
                )
            })
        });
        if !has_cells {
            return page;
        }
        self.chain.clear();
        for id in std::iter::once(node_id).chain(document.ancestors(node_id)) {
            let cell = if document.nodes[id].display() == DisplayValue::TableCell {
                Some(id)
            } else {
                document.anonymous_cell_of(id)
            };
            if let Some((cell, (holder, end, _))) =
                cell.and_then(|cell| Some((cell, cell_row_end(document, id)?)))
            {
                self.chain.push(((holder, end), cell));
            }
        }
        let levels = self.chain.len();
        for level in 0..levels {
            let (row, cell) = self.chain[levels - 1 - level];
            match self.open.get(level) {
                Some(open) if open.row == row => {
                    if open.cell != cell {
                        page = self.leave(level + 1, page);
                        let open = &mut self.open[level];
                        open.end_page = open.end_page.max(page);
                        open.cell = cell;
                        page = open.start_page;
                    }
                }
                _ => {
                    let end_page = self.leave(level, page);
                    page = self
                        .placed
                        .remove(&row.0)
                        .map_or(end_page, |start| start.min(end_page));
                    self.open.push(CellRow {
                        row,
                        cell,
                        start_page: page,
                        end_page,
                    });
                }
            }
        }
        self.leave(levels, page)
    }

    /// Leaves the rows from `level` in: the flow goes on from the last page
    /// they reach.
    fn leave(&mut self, level: usize, page: u32) -> u32 {
        self.open
            .drain(level.min(self.open.len())..)
            .fold(page, |page, row| page.max(row.end_page))
    }
}
