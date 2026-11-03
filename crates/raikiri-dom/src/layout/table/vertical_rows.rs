//! Physical block-axis tracks of a vertical table.

use super::{
    CellPlacement, RowGroup, TableGrid, distribute_extra, grow_tracks, row_track_gaps,
    spanned_size, track_origins,
};

/// Natural block-axis tracks of a vertical table, before removing rows.
pub(super) fn natural_widths(grid: &TableGrid, cell_widths: &[f32], spacing: f32) -> Vec<f32> {
    let mut rows = vec![0.0f32; grid.rows.len()];
    let mut spanning = Vec::new();
    for (cell, &width) in grid.cells.iter().zip(cell_widths) {
        let start = usize::from(cell.row).min(rows.len());
        let end = (start + usize::from(cell.row_span)).min(rows.len());
        if end - start > 1 {
            spanning.push((start..end, width));
        } else if end > start {
            rows[start] = rows[start].max(width);
        }
    }
    spanning.sort_by_key(|(span, _)| (span.len(), span.start));
    for (span, width) in spanning {
        let current = spanned_size(&rows[span.clone()], spacing);
        if width > current {
            grow_tracks(&mut rows[span], width - current);
        }
    }
    rows
}

/// A table's physical rows before and after visibility collapse.
pub(super) struct VerticalRows {
    natural: Vec<f32>,
    pub(super) widths: Vec<f32>,
    natural_gaps: Vec<f32>,
    gaps: Vec<f32>,
}

/// The widths and separated gaps used by one cell layout pass.
#[derive(Clone, Copy)]
pub(super) struct RowTrackView<'a> {
    pub(super) widths: &'a [f32],
    gaps: &'a [f32],
    hidden: Option<&'a [bool]>,
}

impl RowTrackView<'_> {
    pub(super) fn span_width(self, start: usize, count: usize) -> f32 {
        let start = start.min(self.widths.len());
        let end = start.saturating_add(count).min(self.widths.len());
        self.widths[start..end].iter().sum::<f32>()
            + self.gaps[start..end.saturating_sub(1).max(start)]
                .iter()
                .sum::<f32>()
    }

    pub(super) fn cell_width(self, cell: &CellPlacement) -> f32 {
        let start = usize::from(cell.row);
        if self.hidden.and_then(|hidden| hidden.get(start)) == Some(&true) {
            0.0
        } else {
            self.span_width(start, usize::from(cell.row_span))
        }
    }
}

impl VerticalRows {
    pub(super) fn new(
        mut natural: Vec<f32>,
        groups: &[RowGroup],
        hidden: &[bool],
        spacing: f32,
        target: f32,
    ) -> Self {
        // Authored room is shared before collapsed rows give up their share.
        distribute_extra(&mut natural, None, target);
        let widths = natural
            .iter()
            .zip(hidden)
            .map(|(&width, &hidden)| if hidden { 0.0 } else { width })
            .collect();
        Self {
            natural_gaps: vec![spacing; natural.len()],
            natural,
            widths,
            gaps: row_track_gaps(groups, hidden, spacing),
        }
    }

    pub(super) fn natural(&self) -> RowTrackView<'_> {
        RowTrackView {
            widths: &self.natural,
            gaps: &self.natural_gaps,
            hidden: None,
        }
    }

    pub(super) fn visible<'a>(&'a self, hidden: &'a [bool]) -> RowTrackView<'a> {
        RowTrackView {
            widths: &self.widths,
            gaps: &self.gaps,
            hidden: Some(hidden),
        }
    }

    pub(super) fn outer_width(&self, spacing: f32) -> f32 {
        self.widths.iter().sum::<f32>() + self.gaps.iter().sum::<f32>() + spacing
    }

    pub(super) fn origins(&self, origin: f32, reverse: bool, extent: f32) -> Vec<f32> {
        let overlaps: Vec<_> = self.gaps.iter().map(|gap| -gap).collect();
        let mut origins = track_origins(&self.widths, &overlaps, origin);
        if reverse {
            for (start, width) in origins.iter_mut().zip(&self.widths) {
                *start = origin + extent - (*start - origin) - *width;
            }
        }
        origins
    }
}
