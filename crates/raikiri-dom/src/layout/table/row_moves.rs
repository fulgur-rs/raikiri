//! Table boxes that follow the rows pagination moved to a later page.

use crate::Document;
use raikiri_style::property::{BorderCollapseValue, CaptionSideValue, DisplayValue};
use std::collections::{BTreeSet, HashMap};

/// Brings each table box in line with the rows that pagination moved by the
/// distances in `moved_rows`. A row group, or a table, whose first row moved
/// starts with that row on its new page, so no empty fragment of its top edge
/// stays behind (CSS Fragmentation 3 §4.4: the break before a table's first
/// row is the break before the table). Every part then grows over the rows
/// moved below its end. A table for which `repeats_header` holds keeps its
/// top with its repeated header and is finished with it. A table with cells
/// spanning rows keeps its layout geometry, since a spanning cell's rectangle
/// covers more than its own row's track.
pub(crate) fn follow_moved_rows(
    document: &mut Document,
    moved_rows: &HashMap<usize, f32>,
    repeats_header: impl Fn(usize) -> bool,
) {
    let tables: BTreeSet<usize> = moved_rows
        .keys()
        .filter_map(|&row| table_of(document, row))
        .filter(|&table| {
            !document.table_objects.rows.get(&table).is_some_and(|rows| {
                rows.iter()
                    .flat_map(|row| row.cells.iter().copied())
                    .any(|cell| super::get_rowspan(document, cell) != 1)
            })
        })
        .collect();
    for table in tables {
        let repeated_header = repeats_header(table);
        let groups: Vec<usize> = document.nodes[table]
            .children
            .iter()
            .copied()
            .filter(|&id| {
                !document.nodes[id].is_display_none()
                    && matches!(
                        document.nodes[id].display,
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
                || repeated_header && document.nodes[part].display == DisplayValue::TableHeaderGroup
            {
                continue;
            }
            if let Some(&delta) = moved_rows.get(&first) {
                move_top(document, part, delta);
            }
        }
        // A caption above the grid keeps the table's top edge where it is
        // (an absolutely positioned one is out of the table's flow), and so
        // may content outside the rows, wrapped in an anonymous row.
        let top_caption = document.nodes[table].children.iter().any(|&id| {
            !document.nodes[id].is_display_none()
                && document.nodes[id].display == DisplayValue::TableCaption
                && document.nodes[id].caption_side == CaptionSideValue::Top
                && document.nodes[id].style.position != taffy::Position::Absolute
        });
        let anonymous_rows = groups
            .iter()
            .any(|&group| document.anonymous_table_cells(group).next().is_some());
        if repeated_header {
            continue;
        }
        if !top_caption
            && !anonymous_rows
            && let Some((first, _)) = table_first
            && let Some(&delta) = moved_rows.get(&first)
        {
            move_top(document, table, delta);
        }
        let node = &document.nodes[table];
        let gap = if node.border_collapse == BorderCollapseValue::Collapse {
            0.0
        } else {
            node.border_spacing.vertical.0.max(0.0)
        };
        super::headers::finish_table_boxes(document, table, gap);
    }
}

fn table_of(document: &Document, row: usize) -> Option<usize> {
    document.inclusive_ancestors(row).skip(1).find(|&id| {
        matches!(
            document.nodes[id].display,
            DisplayValue::Table | DisplayValue::InlineTable
        )
    })
}

/// The topmost rendered row among the direct children of `part`.
fn first_row(document: &Document, part: usize) -> Option<usize> {
    document.nodes[part]
        .children
        .iter()
        .copied()
        .filter(|&id| {
            !document.nodes[id].is_display_none()
                && document.nodes[id].display == DisplayValue::TableRow
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
    let layout = &mut document.nodes[id].unrounded_layout;
    layout.location.y += delta;
    layout.size.height = (layout.size.height - delta).max(0.0);
    if let Some(grid) = &mut document.nodes[id].table_grid_box {
        grid.height = (grid.height - delta).max(0.0);
    }
    for child in document.nodes[id].children.clone() {
        document.nodes[child].unrounded_layout.location.y -= delta;
    }
}

fn absolute_y(document: &Document, node: usize) -> f32 {
    let mut y = 0.0;
    let mut current = Some(node);
    while let Some(id) = current {
        y += document.nodes[id].unrounded_layout.location.y;
        current = document.layout_parent_of(id);
    }
    y
}
