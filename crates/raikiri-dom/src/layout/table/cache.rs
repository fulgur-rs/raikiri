use std::collections::HashMap;
use std::rc::Rc;

use super::*;

// Layout sessions own these entries. Bridging computed styles and changing
// pagination axes clear them before prepared styles or anonymous keys change.
// Border preparation depends on authored border inputs, collapse/visibility,
// writing mode, topology, parent width and the table border. Temporary size
// probes do not change those inputs. External temporary table-border changes
// must clear this cache; publishing resolved borders is an output, not an input.
#[derive(Debug, Default)]
pub(in crate::layout) struct TableCache {
    publication_epoch: Option<crate::layout::cache_context::CacheEpoch>,
    grids: HashMap<usize, TableGrid>,
    borders: HashMap<
        usize,
        (
            BorderKey,
            crate::layout::cache_context::CacheEpoch,
            Rc<PreparedBorders>,
        ),
    >,
}

impl TableCache {
    fn synchronize(&mut self, epoch: crate::layout::cache_context::CacheEpoch) {
        if self.publication_epoch != Some(epoch) {
            self.clear();
            self.publication_epoch = Some(epoch);
        }
    }

    pub(in crate::layout) fn clear(&mut self) {
        self.grids.clear();
        self.borders.clear();
    }
}

#[derive(Debug, PartialEq)]
struct BorderKey {
    parent_width: Option<u32>,
    table_border: [u32; 4],
}

#[derive(Debug)]
pub(super) struct PreparedBorders {
    pub(super) collapsed_rows: Vec<bool>,
    pub(super) collapsed_cols: Vec<bool>,
    pub(super) columns_in_place: Option<ColumnsInPlace>,
    pub(super) collapsed: CollapsedLines,
    pub(super) borders: Option<BorderState>,
}

pub(super) fn table_grid(
    doc: &mut LayoutSession<'_>,
    table_idx: usize,
) -> Result<TableGrid, raikiri_traits::LayoutError> {
    doc.sync_layout_cache_mutations();
    doc.context
        .table_cache
        .synchronize(doc.document.layout_cache_context.publication_epoch());
    let Some(template) = doc.context.table_cache.grids.get(&table_idx) else {
        let grid = build_table_grid(doc, table_idx)?;
        doc.context
            .table_cache
            .grids
            .insert(table_idx, grid.clone());
        return Ok(grid);
    };
    let mut grid = TableGrid {
        n_cols: template.n_cols,
        rows: template.rows.clone(),
        cells: template.cells.clone(),
        col_widths: collect_col_widths(doc, table_idx)?,
    };
    // Intrinsic-size probes temporarily override retained sizes. Keep their
    // current values while sharing only the immutable placement topology.
    for cell in &mut grid.cells {
        (cell.specified_width, cell.specified_min_width) = cell_specified_widths(doc, cell.node_id);
    }
    Ok(grid)
}

pub(super) fn prepared_borders(
    doc: &mut LayoutSession<'_>,
    table_idx: usize,
    grid: &TableGrid,
    parent_width: Option<f32>,
    table_border: Rect<f32>,
) -> Rc<PreparedBorders> {
    doc.sync_layout_cache_mutations();
    doc.context
        .table_cache
        .synchronize(doc.document.layout_cache_context.publication_epoch());
    let epoch = doc.document.layout_cache_context.publication_epoch();
    let key = BorderKey {
        parent_width: parent_width.map(f32::to_bits),
        table_border: [
            table_border.left,
            table_border.right,
            table_border.top,
            table_border.bottom,
        ]
        .map(f32::to_bits),
    };
    if let Some((cached_key, cached_epoch, prepared)) =
        doc.context.table_cache.borders.get(&table_idx)
        && *cached_key == key
        && *cached_epoch == epoch
    {
        return Rc::clone(prepared);
    }
    let collapse = doc.nodes[table_idx].border_collapse == BorderCollapseValue::Collapse;
    let mut borders = collapse.then(|| BorderState::authored(doc, grid));
    let vertical_writing = table_writing_mode(doc, table_idx).is_vertical();
    let cell_index =
        (collapse && !grid.rows.is_empty() && grid.n_cols > 0).then(|| CellIndex::new(grid));
    let collapsed_rows = collapsed_rows(doc, grid);
    // A vertical table stacks its columns along the block axis, which the
    // collapse rules here do not model.
    let collapsed_cols = if vertical_writing {
        vec![false; usize::from(grid.n_cols)]
    } else {
        collapsed_columns(doc, table_idx, grid.n_cols)
    };
    let columns_collapse = collapsed_cols.contains(&true);
    // Collapsed rows and columns take no part in border conflict resolution:
    // the tracks around them, and the table edge, resolve against each other.
    let shown = (cell_index.is_some() && (columns_collapse || collapsed_rows.contains(&true)))
        .then(|| {
            without_collapsed_columns(
                &without_collapsed_rows(grid, &collapsed_rows),
                &collapsed_cols,
                true,
            )
        });
    let shown_index = shown
        .as_ref()
        .filter(|shown| !shown.rows.is_empty() && shown.n_cols > 0)
        .map(CellIndex::new);
    // Container metrics, split so collapse can substitute collapsed outer
    // borders for the table's own border widths (CSS 2.1 §17.6.2 — the table
    // border participates in conflict resolution with edge cells).
    // The columns that collapse still set the widths the rows are measured at,
    // so the borders around them are read before they are left out.
    let borders_in_place = shown
        .as_ref()
        .filter(|_| cell_index.is_some() && columns_collapse)
        .map(|shown| {
            border_width_with_columns_in_place(
                doc,
                borders.as_mut().expect("collapsed border state"),
                grid,
                &collapsed_rows,
                shown,
                table_idx,
                (&table_border, parent_width),
            )
        });
    if let Some(index) = cell_index.as_ref() {
        let (resolved, index) = match (shown.as_ref(), shown_index.as_ref()) {
            (Some(shown), Some(shown_index)) => (Some(shown), shown_index),
            (Some(_), None) => (None, index),
            (None, _) => (Some(grid), index),
        };
        if let Some(resolved) = resolved {
            resolve_collapsed_borders(
                doc,
                borders.as_mut().expect("collapsed border state"),
                resolved,
                index,
                table_idx,
            );
        }
    }
    let columns_in_place =
        shown
            .as_ref()
            .zip(borders_in_place)
            .map(|(shown, (border_width, in_place))| {
                ColumnsInPlace::new(
                    shown,
                    border_width,
                    in_place,
                    borders.as_ref().expect("collapsed border state").clone(),
                )
                .with_row_overlaps(
                    doc,
                    grid,
                    &collapsed_rows,
                    &table_border,
                    parent_width,
                )
            });
    // Collapsed grid lines (collapse only; separate keeps zero overlaps so
    // the positioning path below is shared).
    // Collapsed rows keep their measured contribution to the columns and to
    // spanning cells, then lose their height and spacing (CSS Tables 3,
    // "Visibility-collapse rendering"). Their lines come from the rows that
    // remain: the rows around a collapsed run meet at their resolved line,
    // and a run at either end of the table leaves no line behind.
    let kept_rows: Vec<usize> = (0..collapsed_rows.len())
        .filter(|&row| !collapsed_rows[row])
        .collect();
    let collapsed = match (cell_index.as_ref(), shown.as_ref(), shown_index.as_ref()) {
        (None, ..) => CollapsedLines::separate(&table_border),
        (Some(index), None, _) => compute_collapsed_lines(
            doc,
            borders.as_ref().expect("collapsed border state"),
            grid,
            index,
            &table_border,
            parent_width,
        ),
        (Some(_), Some(shown), Some(shown_index)) => {
            let lines = compute_collapsed_lines(
                doc,
                borders.as_ref().expect("collapsed border state"),
                shown,
                shown_index,
                &table_border,
                parent_width,
            );
            let kept_cols: Vec<usize> = (0..collapsed_cols.len())
                .filter(|&column| !collapsed_cols[column])
                .collect();
            let mut row_overlaps = vec![0.0; grid.rows.len() - 1];
            for (line, rows) in lines.row_overlaps.iter().zip(kept_rows.windows(2)) {
                row_overlaps[rows[0]] = *line;
            }
            if let Some(columns) = columns_in_place.as_ref() {
                columns.raise_row_overlaps(&kept_rows, &mut row_overlaps);
            }
            let mut col_overlaps = vec![0.0; usize::from(grid.n_cols) - 1];
            for (line, columns) in lines.col_overlaps.iter().zip(kept_cols.windows(2)) {
                col_overlaps[columns[0]] = *line;
            }
            CollapsedLines {
                row_overlaps,
                col_overlaps,
                ..lines
            }
        }
        // Nothing shows, so no cell border meets the table's own.
        (Some(_), Some(_), None) => {
            let mut row_overlaps = vec![0.0; grid.rows.len() - 1];
            if let Some(columns) = columns_in_place.as_ref() {
                columns.raise_row_overlaps(&kept_rows, &mut row_overlaps);
            }
            CollapsedLines {
                col_overlaps: vec![0.0; usize::from(grid.n_cols) - 1],
                row_overlaps,
                outer_left: table_border.left,
                outer_right: table_border.right,
                outer_top: table_border.top,
                outer_bottom: table_border.bottom,
            }
        }
    };
    let prepared = Rc::new(PreparedBorders {
        collapsed_rows,
        collapsed_cols,
        columns_in_place,
        collapsed,
        borders,
    });
    doc.context
        .table_cache
        .borders
        .insert(table_idx, (key, epoch, Rc::clone(&prepared)));
    prepared
}
