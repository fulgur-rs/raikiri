//! The pages of the table rows the flow is inside.
//!
//! The cells of a row sit side by side, so each of them starts on the page
//! its row starts on (CSS 2.1 §17.5), however far the cells before it in the
//! source ran on. The flow walks the cells one after another: when it enters
//! a later cell of a row it goes back to the row's page, and when it leaves
//! the row it goes on from the last page any of its cells reached.

use super::Document;
use raikiri_style::property::DisplayValue;

/// The rows the flow is in, outermost first, each with the cell it is in.
#[derive(Default)]
pub(super) struct CellRows(Vec<CellRow>);

struct CellRow {
    /// The source parent of the cells: the row, or the table or row group
    /// whose boxes an anonymous row holds.
    parent: usize,
    /// The child of `parent` that is the cell or that the cell wraps.
    entry: usize,
    /// The cell, which is the layout key of an anonymous cell when one wraps
    /// `entry`.
    cell: usize,
    /// The page the row starts on.
    start_page: u32,
    /// The last page that the cells left so far reach, and the rows before
    /// it.
    end_page: u32,
}

impl CellRows {
    /// Enters the rows and cells that hold `node_id`, leaving the ones that
    /// do not, with the flow on `page`; returns the page the flow is on then.
    /// `row_page` gives the page a row box starts on: a row can start on a
    /// page before the one the rows above it reach, beside a cell that spans
    /// both (CSS 2.1 §17.2.1, `rowspan`).
    pub(super) fn enter(
        &mut self,
        document: &Document,
        node_id: usize,
        mut page: u32,
        row_page: impl Fn(usize) -> u32,
    ) -> u32 {
        let mut chain = Vec::new();
        let mut child = node_id;
        for parent in document.ancestors(node_id) {
            let cell = if document.nodes[child].display() == DisplayValue::TableCell {
                Some(child)
            } else {
                document.anonymous_cell_of(child)
            };
            if let Some(cell) = cell {
                chain.push((parent, child, cell));
            }
            child = parent;
        }
        let depth = chain.len();
        for (depth, (parent, entry, cell)) in chain.into_iter().rev().enumerate() {
            match self.0.get(depth) {
                Some(open) if open.parent == parent && same_row(document, open.entry, entry) => {
                    if open.cell != cell {
                        page = self.leave(depth + 1, page);
                        let open = &mut self.0[depth];
                        open.end_page = open.end_page.max(page);
                        open.entry = entry;
                        open.cell = cell;
                        page = open.start_page;
                    }
                }
                _ => {
                    let end_page = self.leave(depth, page);
                    page = if document.nodes[parent].display() == DisplayValue::TableRow {
                        row_page(parent).min(end_page)
                    } else {
                        end_page
                    };
                    self.0.push(CellRow {
                        parent,
                        entry,
                        cell,
                        start_page: page,
                        end_page,
                    });
                }
            }
        }
        self.leave(depth, page)
    }

    /// Leaves the rows from `depth` in: the flow goes on from the last page
    /// their cells reach.
    fn leave(&mut self, depth: usize, page: u32) -> u32 {
        self.0
            .drain(depth.min(self.0.len())..)
            .fold(page, |page, row| page.max(row.end_page))
    }
}

/// Whether the children `earlier` and `later` of one parent are cells, or
/// boxes wrapped in cells, of the same row: no row, row group or caption
/// comes between them, as one would close the anonymous row they share
/// (CSS 2.1 §17.2.1).
fn same_row(document: &Document, earlier: usize, later: usize) -> bool {
    let parent = document.parent_of(later);
    if parent.is_some_and(|parent| document.nodes[parent].display() == DisplayValue::TableRow) {
        return true;
    }
    parent
        .into_iter()
        .flat_map(|parent| &document.nodes[parent].children)
        .skip_while(|&&child| child != earlier)
        .take_while(|&&child| child != later)
        .all(|&child| {
            !matches!(
                document.nodes[child].display(),
                DisplayValue::TableRow
                    | DisplayValue::TableRowGroup
                    | DisplayValue::TableHeaderGroup
                    | DisplayValue::TableFooterGroup
                    | DisplayValue::TableCaption
            )
        })
}
