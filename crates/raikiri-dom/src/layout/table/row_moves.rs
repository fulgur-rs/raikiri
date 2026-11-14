//! Table boxes that follow the rows pagination moved to a later page.

use crate::Document;
use raikiri_layout_style::values::Position;
use raikiri_style::property::{BorderCollapseValue, CaptionSideValue, DisplayValue};
use std::collections::{BTreeSet, HashMap};

#[cfg(test)]
thread_local! {
    /// The moved rows [`follow_moved_rows`] was given, summed over its
    /// calls, and the calls of [`has_row_spanning_cell`].
    pub(crate) static ROW_MOVE_WORK: std::cell::Cell<(usize, usize)> =
        const { std::cell::Cell::new((0, 0)) };
}

/// Whether a cell of `table` spans more than one row.
pub(crate) fn has_row_spanning_cell(document: &Document, table: usize) -> bool {
    #[cfg(test)]
    ROW_MOVE_WORK.with(|work| {
        let (rows, checks) = work.get();
        work.set((rows, checks + 1));
    });
    document.table_objects.rows.get(&table).is_some_and(|rows| {
        rows.iter()
            .flat_map(|row| row.cells.iter().copied())
            .any(|cell| super::get_rowspan(document, cell) != 1)
    })
}

/// Records the collapsed border lines that pagination split by moving the
/// rows in `moved_rows` to a later page.
///
/// A row moves by more than the row before it only when it opens a page.
/// The line between the two is then split at the break: the half above it
/// stays with the cells that end at the earlier row, and the half below
/// moves with the cells of the later row (CSS Fragmentation 3 §5.4 and
/// CSS 2.1 §17.6.2, which centers the line between the rows).
pub(crate) fn mark_split_lines(document: &mut Document, moved_rows: &HashMap<usize, f32>) {
    let tables: BTreeSet<usize> = moved_rows
        .keys()
        .filter_map(|&row| document.table_of_row(row))
        .filter(|&table| {
            document
                .ifc_layout_node(table)
                .is_some_and(|node| node.border_collapse == BorderCollapseValue::Collapse)
        })
        .collect();
    let mut split_lines = std::mem::take(&mut document.table_objects.split_lines);
    for (table, rows) in tables
        .iter()
        .filter_map(|&table| Some((table, document.table_objects.rows.get(&table)?)))
    {
        // The cells whose rows end at each row. A cell spans no further than
        // the end of its row group, as in layout, and `rowspan="0"` reaches
        // that end.
        let markers: Vec<usize> = rows.iter().map(|row| row.marker).collect();
        let group_ends = super::rowspan::group_ends(document, table, &markers);
        let mut ending: Vec<Vec<usize>> = vec![Vec::new(); rows.len()];
        for (index, row) in rows.iter().enumerate() {
            for &cell in &row.cells {
                let span = usize::from(super::get_rowspan(document, cell));
                let end = span
                    .checked_sub(1)
                    .map_or(usize::MAX, |more| index + more)
                    .min(group_ends[index] - 1);
                ending[end].push(cell);
            }
        }
        let delta = |index: usize| moved_rows.get(&rows[index].marker).copied().unwrap_or(0.0);
        for index in 1..rows.len() {
            if delta(index) > delta(index - 1) + 0.001 {
                for &cell in &rows[index].cells {
                    split_lines.entry(cell).or_default().0 = true;
                }
                for &cell in &ending[index - 1] {
                    split_lines.entry(cell).or_default().1 = true;
                }
            }
        }
    }
    document.table_objects.split_lines = split_lines;
}

/// Brings each table box in line with the rows that pagination moved by the
/// distances in `moved_rows`. A row group, or a table, whose first row moved
/// starts with that row on its new page, so no empty fragment of its top edge
/// stays behind (CSS Fragmentation 3 §4.4: the break before a table's first
/// row is the break before the table). Every part then grows over the rows
/// moved below its end. A table for which `repeats_header` holds keeps its
/// top with its repeated header and is finished with it. A table with cells
/// spanning rows keeps its layout geometry, since a spanning cell's rectangle
/// covers more than its own row's track.
///
/// A caption above the grid that pagination moved to a later page, with the
/// distance in `moved_captions`, takes the table's top edge along: the table
/// starts on the page it opens, as it does with its first row when no caption
/// comes before it.
pub(crate) fn follow_moved_rows(
    document: &mut Document,
    moved_rows: &HashMap<usize, f32>,
    moved_captions: &HashMap<usize, f32>,
    repeats_header: impl Fn(usize) -> bool,
) {
    // Collect the tables first: many moved rows share a table, and checking
    // a table for spanning cells walks all of its cells.
    let tables: BTreeSet<usize> = moved_rows
        .keys()
        .filter_map(|&row| document.table_of_row(row))
        .chain(
            moved_captions
                .keys()
                .filter_map(|&caption| document.layout_parent_of(caption)),
        )
        .collect();
    #[cfg(test)]
    ROW_MOVE_WORK.with(|work| {
        let (rows, checks) = work.get();
        work.set((rows + moved_rows.len() + moved_captions.len(), checks));
    });
    for table in tables {
        if has_row_spanning_cell(document, table) {
            continue;
        }
        let repeated_header = repeats_header(table);
        let groups: Vec<usize> = document.nodes[table]
            .children
            .iter()
            .copied()
            .filter(|&id| {
                !document.nodes[id].is_display_none()
                    && matches!(
                        document.nodes[id].display(),
                        DisplayValue::TableRowGroup
                            | DisplayValue::TableHeaderGroup
                            | DisplayValue::TableFooterGroup
                    )
            })
            .collect();
        let mut table_first: Option<(usize, f32)> = None;
        for &part in groups.iter().chain(std::iter::once(&table)) {
            let Some(first) = first_row(document, part) else {
                continue;
            };
            let y = absolute_y(document, first);
            if table_first.is_none_or(|(_, first_y)| y < first_y) {
                table_first = Some((first, y));
            }
            // The repeated header is placed with its copies.
            if part == table
                || repeated_header
                    && document.nodes[part].display() == DisplayValue::TableHeaderGroup
            {
                continue;
            }
            if let Some(&delta) = moved_rows.get(&first) {
                move_top(document, part, delta);
            }
        }
        // A caption above the grid, one inside a `display: contents` child of
        // the table included, keeps the table's top edge where it is (an
        // absolutely positioned one is out of the table's flow), and so may
        // content outside the rows, wrapped in an anonymous row.
        let first_top_caption = document
            .children_through_contents(table)
            .iter()
            .copied()
            .find(|&id| {
                !document.nodes[id].is_display_none()
                    && document.nodes[id].display() == DisplayValue::TableCaption
                    && document.nodes[id].caption_side == CaptionSideValue::Top
                    && document.nodes[id].layout_position() != Position::Absolute
            });
        let anonymous_rows = groups
            .iter()
            .any(|&group| document.anonymous_table_cells(group).next().is_some());
        if repeated_header {
            continue;
        }
        // The first caption above the grid is the first box of the table: when
        // it moved to a later page, the table's top edge goes with it.
        if let Some(&delta) = first_top_caption.and_then(|caption| moved_captions.get(&caption)) {
            move_top(document, table, delta);
        } else if first_top_caption.is_none()
            && !anonymous_rows
            // An anonymous table has no box of its own to draw: its row groups
            // and rows, which stand in the table's parent, are all that moves.
            && !document.is_anonymous_table(table)
            && let Some((first, _)) = table_first
            && let Some(&delta) = moved_rows.get(&first)
        {
            move_top(document, table, delta);
        }
        let node = document.ifc_layout_node(table).unwrap();
        let gap = if node.border_collapse == BorderCollapseValue::Collapse {
            0.0
        } else {
            node.border_spacing.vertical.0.max(0.0)
        };
        super::headers::finish_table_boxes(document, table, gap);
    }
}

/// The topmost rendered row among the direct children of `part`.
fn first_row(document: &Document, part: usize) -> Option<usize> {
    document.nodes[part]
        .children
        .iter()
        .copied()
        .filter(|&id| {
            !document.nodes[id].is_display_none()
                && document.nodes[id].display() == DisplayValue::TableRow
        })
        .min_by(|&a, &b| absolute_y(document, a).total_cmp(&absolute_y(document, b)))
}

/// Moves the top edge of `id` down by `delta`, keeping its bottom edge and
/// the page positions of its children. A box that also holds content outside
/// its rows, wrapped in anonymous cells, keeps its geometry.
fn move_top(document: &mut Document, id: usize, delta: f32) {
    if !delta.is_finite() || delta <= 0.0 || document.anonymous_table_cells(id).next().is_some() {
        return;
    }
    // A box shorter than the move ends up empty above its first row, and
    // grows over its rows again when the table is finished.
    let layout = &mut document.nodes[id].layout;
    layout.location.y += delta;
    layout.size.height = (layout.size.height - delta).max(0.0);
    if let Some(grid) = &mut document.nodes[id].table_grid_box {
        grid.height = (grid.height - delta).max(0.0);
    }
    for child in document.nodes[id].children.clone() {
        document.nodes[child].layout.location.y -= delta;
    }
}

fn absolute_y(document: &Document, node: usize) -> f32 {
    document
        .inclusive_layout_ancestors(node)
        .fold(0.0, |y, id| y + document.nodes[id].layout.location.y)
}
