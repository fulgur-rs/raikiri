//! Minimum physical inline extents of vertical table columns.

use super::{
    AvailableSpace, BoxSizing, CellPlacement, LayoutInput, LayoutSession, Line, NodeId,
    RequestedAxis, ResolveOrZero, RowTrackView, RunMode, Size, SizingMode, TableGrid, grow_tracks,
    resolve_dimension, spanned_size, table_writing_mode,
};

/// Include the inline numeric minimum without changing the row pass's records.
pub(super) fn preferred_height(doc: &LayoutSession<'_>, cell: &CellPlacement, width: f32) -> f32 {
    cell.measured_height
        .max(numeric_minimum_height(doc, cell.node_id, width).unwrap_or(0.0))
}

/// Minimum tracks, using local preferences before each spanning cell settles.
pub(super) fn minimum_tracks(
    doc: &mut LayoutSession<'_>,
    grid: &TableGrid,
    rows: RowTrackView<'_>,
    spacing: f32,
) -> Vec<f32> {
    let count = usize::from(grid.n_cols);
    let mut minimum = vec![0.0f32; count];
    let mut preferred = vec![0.0f32; count];
    let mut spans = Vec::new();
    for cell in &grid.cells {
        let start = usize::from(cell.col_start).min(count);
        let end = start.saturating_add(usize::from(cell.col_span)).min(count);
        let width = rows.cell_width(cell);
        let min = minimum_height(doc, cell.node_id, width);
        let pref = preferred_height(doc, cell, width).max(min);
        if end - start > 1 {
            spans.push((start..end, min, pref));
        } else if end > start {
            minimum[start] = minimum[start].max(min);
            preferred[start] = preferred[start].max(pref);
        }
    }
    spans.sort_by_key(|(span, _, _)| (span.len(), span.start));
    for (span, min, pref) in spans {
        let current = spanned_size(&minimum[span.clone()], spacing);
        if min > current {
            // A spanning minimum first uses the room toward the preferences
            // of earlier cells, before this span adds its own preference.
            let target = min - spacing * (span.len() - 1) as f32;
            let mut fitted = preferred[span.clone()].to_vec();
            let total: f32 = fitted.iter().sum();
            if target > total {
                grow_tracks(&mut fitted, target - total);
            } else {
                fit(&mut fitted, &minimum[span.clone()], target);
            }
            minimum[span.clone()].copy_from_slice(&fitted);
        }
        for (preferred, minimum) in preferred[span.clone()]
            .iter_mut()
            .zip(&minimum[span.clone()])
        {
            *preferred = preferred.max(*minimum);
        }
        let current = spanned_size(&preferred[span.clone()], spacing);
        if pref > current {
            grow_tracks(&mut preferred[span], pref - current);
        }
    }
    minimum
}

/// Interpolate preferred tracks toward their minima for a smaller target.
pub(super) fn fit(tracks: &mut [f32], minimum: &[f32], target: f32) {
    let floor: f32 = minimum.iter().sum();
    let flexible: f32 = tracks
        .iter()
        .zip(minimum)
        .map(|(track, min)| (track - min).max(0.0))
        .sum();
    let factor = if flexible > 0.0 {
        (target - floor).max(0.0) / flexible
    } else {
        0.0
    };
    for (track, min) in tracks.iter_mut().zip(minimum) {
        *track = min + (*track - min).max(0.0) * factor;
    }
}

/// Measure content without the preferred physical height of the cell.
fn minimum_height(doc: &mut LayoutSession<'_>, cell: usize, width: f32) -> f32 {
    let vertical = table_writing_mode(doc, cell).is_vertical();
    let key = doc.nodes.box_key(cell).expect("cell has a box key");
    let entry = raikiri_layout_style::OwnedPropertyOverride::length(
        raikiri_layout_style::LengthProperty::Height,
        raikiri_style::ComputedLengthPercentageOrAuto::Auto,
        false,
    )
    .expect("physical cell height");
    let measured = doc.with_overrides(vec![(key, entry)], |doc| {
        doc.compute_child_layout(
            NodeId::from(cell),
            LayoutInput {
                run_mode: RunMode::ComputeSize,
                sizing_mode: SizingMode::InherentSize,
                axis: RequestedAxis::Vertical,
                known_dimensions: Size {
                    width: Some(width),
                    height: None,
                },
                parent_size: Size {
                    width: Some(width),
                    height: None,
                },
                available_space: Size {
                    width: AvailableSpace::Definite(width),
                    height: if vertical {
                        AvailableSpace::MinContent
                    } else {
                        AvailableSpace::MaxContent
                    },
                },
                known_dimensions_are_definite: Size {
                    width: true,
                    height: false,
                },
                vertical_margins_are_collapsible: Line::FALSE,
            },
        )
        .size
        .height
    });
    measured.max(numeric_minimum_height(doc, cell, width).unwrap_or(0.0))
}

/// A resolvable numeric minimum in the same border-box domain as the probe.
fn numeric_minimum_height(doc: &LayoutSession<'_>, cell: usize, width: f32) -> Option<f32> {
    let style = doc.table_style(cell);
    let minimum = resolve_dimension(style.vertical_table_cell_min_height().into(), None)?;
    let padding = style.padding().resolve_or_zero(Some(width));
    let border = style.border().resolve_or_zero(Some(width));
    let edges = padding.top + padding.bottom + border.top + border.bottom;
    let outer = if style.box_sizing() == BoxSizing::ContentBox {
        minimum + edges
    } else {
        minimum.max(edges)
    };
    Some(outer)
}

#[cfg(test)]
mod tests;
