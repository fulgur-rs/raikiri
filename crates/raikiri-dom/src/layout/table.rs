//! Native table layout — CSS Tables 3 auto + fixed layout, separate + collapse borders.
//!
//! Ported from `table layout design` (Blitz L3 column distribution) and adapted to
//! raikiri's `Document` arena + `DisplayValue`.
//!
//! Pipeline (per `docs/superpowers/specs/2026-07-10-taffy-table-layout-design.md`
//! and the table layout design):
//!
//! 1. Build `TableGrid` (rows, cells with col/row spans) via `build_table_grid`.
//! 2. Column sizing — `table-layout` branch:
//!    - `auto`: batch-measure each cell's min/max-content inline size with
//!      indefinite parent width, then pure column sizing
//!      (`resolve_column_widths` / `distribute_columns`) — no taffy recursion.
//!    - `fixed`: content is not measured; column widths come from the table
//!      width plus first-row specified widths
//!      (`resolve_fixed_column_widths`, CSS 2.1 §17.5.2.1).
//! 3. Batch-layout each row's cells with resolved column widths to determine row heights.
//! 4. Position each cell at (col-origin, row-origin) and write final layouts —
//!    `border-collapse: collapse` overlaps adjoining cell boxes by the
//!    collapsed line width (`compute_collapsed_lines`, CSS 2.1 §17.6.2).
//!
//! Scope limits (documented, not silent):
//! - Fixed layout reads specified widths from **first-row cells only** —
//!   `col` / `colgroup` `width` is ignored (the grid does not track column
//!   boxes yet). Fixed with an indefinite table width falls back to auto.
//! - Collapse uses a **simplified max-width conflict resolution**: each grid
//!   line takes the max of the adjoining border widths. Style-priority
//!   resolution (`hidden` > `double` > …) and half-border centering are out
//!   of scope — adjoining boxes overlap by the pairwise min so the visible
//!   line equals the max.
//! - `border-spacing` (separate model gaps) is not implemented — separate
//!   cells abut exactly.
//! - Nested tables are depth-capped fail-closed (`MAX_TABLE_NESTING`).

use raikiri_style::property::{BorderCollapseValue, DisplayValue, TableLayoutValue, WritingMode};
use taffy::style::{CompactLength, Dimension};
use taffy::style_helpers::{TaffyMaxContent, TaffyMinContent};
use taffy::tree::{RunMode, SizingMode};
use taffy::{
    AvailableSpace, Layout as TaffyLayout, LayoutInput, LayoutOutput, LayoutPartialTree, NodeId,
    Point, Rect, Size,
};

use crate::document::Document;

// ---------------------------------------------------------------------------
// Depth cap — fail-closed for nested tables.
// ---------------------------------------------------------------------------

const MAX_TABLE_NESTING: u32 = 8;

thread_local! {
    static TABLE_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

fn depth_enter() -> Option<u32> {
    let mut overflow = false;
    let prev = TABLE_DEPTH.with(|c| {
        let v = c.get();
        if v >= MAX_TABLE_NESTING {
            overflow = true;
            v
        } else {
            c.set(v + 1);
            v
        }
    });
    if overflow { None } else { Some(prev) }
}

fn depth_exit() {
    TABLE_DEPTH.with(|c| {
        let v = c.get();
        if v > 0 {
            c.set(v - 1);
        }
    });
}

// ---------------------------------------------------------------------------
// Transient grid
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct TableGrid {
    n_cols: u16,
    rows: Vec<usize>, // node ids
    cells: Vec<CellPlacement>,
    /// Authored sizing of `<col>` elements in grid order (CSS 2.1 §17.5.2:
    /// `col` widths constrain columns even with no cells in them).
    /// `span` attributes are expanded (one entry per spanned column).
    col_widths: Vec<ColSizing>,
}

#[derive(Debug, Clone, Copy)]
struct CellPlacement {
    node_id: usize,
    row: u16,
    col_start: u16,
    col_span: u16,
    row_span: u16,
    specified_width: Dimension,
    resolved: Option<TaffyLayout>,
}

// ---------------------------------------------------------------------------
// Public entry — called from `taffy_impl.rs` when `Node::display == Table`.
// ---------------------------------------------------------------------------

fn table_writing_mode(doc: &Document, table_idx: usize) -> WritingMode {
    let mut current = Some(table_idx);
    while let Some(id) = current {
        if let Some(mode) = doc.nodes[id].authored_writing_mode {
            return mode;
        }
        current = doc.parent_of(id);
    }
    WritingMode::HorizontalTb
}

fn is_vertical_writing_mode(mode: WritingMode) -> bool {
    matches!(
        mode,
        WritingMode::VerticalRl
            | WritingMode::VerticalLr
            | WritingMode::SidewaysRl
            | WritingMode::SidewaysLr
    )
}

pub fn compute_table_layout(
    doc: &mut Document,
    table_id: NodeId,
    inputs: LayoutInput,
) -> LayoutOutput {
    // Depth cap: fail-closed as block fallback.
    let _depth_guard = match depth_enter() {
        Some(_prev) => DepthGuard,
        None => {
            // Exceeded nesting — render as block with available width, zero height fallback.
            // We still need to run block layout for children to avoid leaving them uninitialized?
            // For fail-closed, return outer size based on known_dimensions or zero.
            let w = inputs.known_dimensions.width.unwrap_or(0.0);
            let h = inputs.known_dimensions.height.unwrap_or(0.0);
            return LayoutOutput::from_outer_size(Size {
                width: w,
                height: h,
            });
        }
    };

    let table_idx = usize::from(table_id);
    let mut grid = build_table_grid(doc, table_idx);
    let table_layout = doc.nodes[table_idx].table_layout;
    let collapse = doc.nodes[table_idx].border_collapse == BorderCollapseValue::Collapse;
    let vertical_writing = is_vertical_writing_mode(table_writing_mode(doc, table_idx));

    // Container metrics, split so collapse can substitute collapsed outer
    // borders for the table's own border widths (CSS 2.1 §17.6.2 — the table
    // border participates in conflict resolution with edge cells).
    let pad = resolve_table_padding(doc, table_idx, inputs.parent_size);
    let table_border = resolve_table_border(doc, table_idx, inputs.parent_size);

    // Known outer dimensions (from taffy's compute_root_layout known_dimensions or style.size)
    // The table's outer size caller may have imposed via `apply_page_box_to_body` / block layout.
    // For tables, style.size is already injected via bridge; inputs.known_dimensions carries it.
    // Also honour the node at table_idx's style.size if known_dimensions is None (similar to prototype).
    // taffy's Dimension::maybe_resolve not directly available; use helper below.
    let effective_known = Size {
        width: inputs.known_dimensions.width.or(resolve_dimension(
            doc.nodes[table_idx].style.size.width,
            inputs.parent_size.width,
        )),
        height: inputs.known_dimensions.height.or(resolve_dimension(
            doc.nodes[table_idx].style.size.height,
            inputs.parent_size.height,
        )),
    };

    // Collapsed grid lines (collapse only; separate keeps zero overlaps so
    // the positioning path below is shared).
    let collapsed = if collapse && !grid.rows.is_empty() && grid.n_cols > 0 {
        compute_collapsed_lines(doc, &grid, &table_border, inputs.parent_size.width)
    } else {
        CollapsedLines::separate(&table_border)
    };

    let padding_border_size = Size {
        width: pad.left + pad.right + collapsed.outer_left + collapsed.outer_right,
        height: pad.top + pad.bottom + collapsed.outer_top + collapsed.outer_bottom,
    };

    if grid.n_cols == 0 || grid.rows.is_empty() {
        // A table with ordinary flow content generates anonymous row/cell
        // boxes (CSS 2.1 §17.2.1). This is especially important when a table
        // is used as a flex item: its direct text must still contribute an
        // intrinsic block size even though it is not an explicit table row.
        let direct_content_height = doc.nodes[table_idx]
            .children
            .iter()
            .filter_map(|&child| doc.nodes[child].text_layout().map(|layout| layout.height()))
            .sum::<f32>();
        // Empty tables shrink-wrap like the main path below: a specified
        // width wins; otherwise the container width is only a cap over the
        // padding/border extents (never stretch-to-fill).
        let specified_w = resolve_dimension(
            doc.nodes[table_idx].style.size.width,
            inputs.parent_size.width,
        );
        // A flex/grid parent supplies the table item's used main size after
        // flexing/tracking. It must override the table's percentage width
        // (which is only the item's hypothetical basis); otherwise a
        // shrinking table item snaps back to its pre-flex percentage width.
        let parent_is_flex_or_grid = doc.parent_of(table_idx).is_some_and(|parent| {
            matches!(
                doc.nodes[parent].style.display,
                taffy::Display::Flex | taffy::Display::Grid
            )
        });
        let width = if parent_is_flex_or_grid {
            effective_known.width.or(specified_w).unwrap_or_else(|| {
                let natural = padding_border_size.width;
                match effective_known.width {
                    Some(container) => f32_max_compat(natural.min(container), 0.0),
                    None => natural,
                }
            })
        } else {
            specified_w.unwrap_or_else(|| {
                let natural = padding_border_size.width;
                match effective_known.width {
                    Some(container) => f32_max_compat(natural.min(container), 0.0),
                    None => natural,
                }
            })
        };
        let specified_h = resolve_dimension(
            doc.nodes[table_idx].style.size.height,
            inputs.parent_size.height,
        );
        let height = specified_h.unwrap_or_else(|| {
            effective_known
                .height
                .unwrap_or(0.0)
                .max(direct_content_height + padding_border_size.height)
        });
        return LayoutOutput::from_outer_size(Size { width, height });
    }

    // Overlap totals up front: tracks are sized in pre-overlap space
    // (origins net the overlaps out afterwards), so the avail/target the
    // distributors aim at must add the overlaps back — otherwise a stretched
    // table leaves an `overlap`-wide slack at its end edge. Separate model:
    // overlaps are zero, so this reduces to `padding_border_size`.
    let overlap_w: f32 = collapsed.col_overlaps.iter().sum();
    let overlap_h: f32 = collapsed.row_overlaps.iter().sum();
    let distrib_insets = Size {
        width: padding_border_size.width - overlap_w,
        height: padding_border_size.height - overlap_h,
    };

    // Specified (non-auto) table width, resolved against the containing
    // block. Auto-width tables shrink-wrap their content (CSS 2.1 §17.5.2.2)
    // and must NOT stretch to fill the containing block: `effective_known`
    // carries the container-imposed width (800 for top-level blocks), which
    // is a cap for auto tables, the target only for specified widths.
    let specified_width = resolve_dimension(
        doc.nodes[table_idx].style.size.width,
        inputs.parent_size.width,
    );
    // In fixed mode an unresolvable specified width (auto, or % of an
    // indefinite container) falls back to the auto algorithm below.
    // Auto layout resolves columns against the SPECIFIED width only
    // (`None` when auto): with `known_dimensions.width = None` the resolver
    // takes its cap branch (preferred size capped by the definite
    // container) instead of stretch-to-fill. A specified width keeps the
    // previous basis (`effective_known`, the taffy-resolved outer width —
    // subtracting insets recovers the content box). Fixed layout keeps the
    // previous behavior (container width as distribution basis).
    let inputs_for_columns = LayoutInput {
        known_dimensions: Size {
            width: match table_layout {
                TableLayoutValue::Fixed => effective_known.width,
                _ => match specified_width {
                    Some(_) => effective_known.width,
                    None => None,
                },
            },
            height: effective_known.height,
        },
        ..inputs
    };
    let mut column_widths = if table_layout == TableLayoutValue::Fixed {
        // Fixed with an indefinite (auto/percent-of-indefinite) table width
        // has no basis for the §17.5.2.1 distribution — fall back to auto.
        // Crucially the container width is NOT a substitute basis: a
        // fixed+auto table shrink-wraps like an auto table (WPT
        // table_grid_size_col_colspan), it does not fill its container.
        let avail = specified_width.map(|w| f32_max_compat(w - distrib_insets.width, 0.0));
        match avail {
            Some(avail) => resolve_fixed_column_widths(&grid, avail),
            None => resolve_column_widths(doc, &grid, inputs_for_columns, distrib_insets),
        }
    } else {
        resolve_column_widths(doc, &grid, inputs_for_columns, distrib_insets)
    };

    // min/max authored values + box-sizing (CSS Sizing 3 §3.3/§4/§5, WPT
    // min-height-table-*, min-max-size-table-content-box). Percentages
    // resolve against the parent (containing block); unresolvable/Auto
    // Freeman. With `box-sizing: content-box` (the initial value, and the
    // content-box test's authored value) min/max apply to the content box;
    // with `border-box` they apply to the outer box. The engine clamps and
    // distributes in outer sizes below, so content-box values are lifted by
    // the padding+border extents. Lifting both the natural size and the max
    // by the same insets keeps the max-never-shrinks gate (`mx >= v`,
    // csswg-drafts#5336 / Mozilla bug 1651530) in the same domain, so
    // max-height/max-width keep leaving sub-intrinsic tables at natural
    // size. min still wins over max on direct conflict.
    let st = &doc.nodes[table_idx].style;
    let min_w = resolve_dimension(st.min_size.width.into(), inputs.parent_size.width);
    let max_w = resolve_dimension(st.max_size.width.into(), inputs.parent_size.width);
    let min_h = resolve_dimension(st.min_size.height.into(), inputs.parent_size.height);
    let max_h = resolve_dimension(st.max_size.height.into(), inputs.parent_size.height);
    let is_content_box = st.box_sizing == taffy::BoxSizing::ContentBox;
    let lift_w = if is_content_box {
        padding_border_size.width
    } else {
        0.0
    };
    let lift_h = if is_content_box {
        padding_border_size.height
    } else {
        0.0
    };
    let min_w_outer = min_w.map(|m| m + lift_w);
    let max_w_outer = max_w.map(|m| m + lift_w);
    let min_h_outer = min_h.map(|m| m + lift_h);
    let max_h_outer = max_h.map(|m| m + lift_h);

    // Extra min-width grows columns (mirrors the min-height→rows path
    // below, and the definite-width distribution above). Must run before
    // row heights are measured so cells lay out at final column widths.
    if let Some(mn) = min_w_outer {
        let target = f32_max_compat(mn - distrib_insets.width, 0.0);
        distribute_extra_width(&mut column_widths, target);
    }

    // Row heights
    let mut row_heights = resolve_row_heights(doc, &grid, &column_widths);
    if let Some(known_h) = effective_known.height {
        let target = f32_max_compat(known_h - distrib_insets.height, 0.0);
        distribute_extra_height(&mut row_heights, target);
    }

    // Extra min-height grows rows (same path as a definite height);
    // max-height only clamps the box (content overflows visibly). The outer
    // domain keeps content-box correct: `min_outer - distrib_insets` =
    // `min_content + overlap`, the track sum that yields `min_content`.
    if let Some(mn) = min_h_outer {
        let target = f32_max_compat(mn - distrib_insets.height, 0.0);
        distribute_extra_height(&mut row_heights, target);
    }

    // Content extents net of collapsed-line overlaps (separate: overlaps are
    // zero, so this reduces to the plain sum).
    let content_width: f32 = column_widths.iter().sum::<f32>() - overlap_w;
    let content_height: f32 = row_heights.iter().sum::<f32>() - overlap_h;
    // min grows the box; max NEVER shrinks a table below its intrinsic
    // content size (csswg-drafts#5336 / Mozilla bug 1651530: WPT
    // min-max-size-table-content-box and max-height-table check that
    // max-height/max-width leave sub-intrinsic tables at natural size —
    // only min-* grow). min still wins over max on direct conflict.
    let clamp_min_max = |v: f32, mn: Option<f32>, mx: Option<f32>| -> f32 {
        let mut x = v;
        if let Some(mn) = mn {
            x = f32_max_compat(x, mn);
        }
        if let Some(mx) = mx
            && !(mn.is_some_and(|mn| mn > mx))
            && mx >= v
        {
            x = x.min(mx);
        }
        x
    };
    // Auto tables without a specified width size to content
    // (shrink-wrap); specified widths (and fixed layout) keep the previous
    // fill basis.
    let table_width_basis = match table_layout {
        TableLayoutValue::Fixed => effective_known.width,
        _ => match specified_width {
            Some(_) => effective_known.width,
            None => None,
        },
    };
    let vertical_content_height = row_heights.iter().sum::<f32>() * grid.n_cols as f32;
    let final_size = Size {
        width: if vertical_writing {
            effective_known
                .width
                .unwrap_or(content_width + padding_border_size.width)
        } else {
            table_width_basis
                .map(|w| clamp_min_max(w, min_w_outer, max_w_outer))
                .unwrap_or_else(|| {
                    clamp_min_max(
                        content_width + padding_border_size.width,
                        min_w_outer,
                        max_w_outer,
                    )
                })
        },
        height: if vertical_writing {
            vertical_content_height + padding_border_size.height
        } else {
            effective_known
                .height
                .map(|h| clamp_min_max(h, min_h_outer, max_h_outer))
                .unwrap_or_else(|| {
                    clamp_min_max(
                        content_height + padding_border_size.height,
                        min_h_outer,
                        max_h_outer,
                    )
                })
        },
    };

    if inputs.run_mode == RunMode::ComputeSize {
        return LayoutOutput::from_outer_size(final_size);
    }

    // Position cells: origin includes padding + collapsed outer border.
    let x_origin = pad.left + collapsed.outer_left;
    let y_origin = pad.top + collapsed.outer_top;
    let col_origins = track_origins(&column_widths, &collapsed.col_overlaps, x_origin);
    let row_origins = track_origins(&row_heights, &collapsed.row_overlaps, y_origin);
    place_cells(
        doc,
        &mut grid,
        &column_widths,
        &row_heights,
        &col_origins,
        &row_origins,
    );
    if vertical_writing {
        reposition_cells_for_vertical_writing(
            doc,
            &mut grid,
            &column_widths,
            &row_heights,
            (final_size.width - padding_border_size.width).max(0.0),
            x_origin,
            y_origin,
        );
    }

    LayoutOutput::from_outer_size(final_size)
}

#[inline(always)]
fn f32_max_compat(a: f32, b: f32) -> f32 {
    if a > b { a } else { b }
}

struct DepthGuard;
impl Drop for DepthGuard {
    fn drop(&mut self) {
        depth_exit();
    }
}

// ---------------------------------------------------------------------------
// Grid construction
// ---------------------------------------------------------------------------

fn build_table_grid(doc: &Document, table_idx: usize) -> TableGrid {
    let mut rows: Vec<usize> = Vec::new();
    let mut cells: Vec<CellPlacement> = Vec::new();
    let mut n_cols: u16 = 0;
    collect_rows(doc, table_idx, &mut rows, &mut cells, &mut n_cols);
    let col_widths = collect_col_widths(doc, table_idx);
    n_cols = n_cols.max(col_widths.len().min(u16::MAX as usize) as u16);
    TableGrid {
        n_cols,
        rows,
        cells,
        col_widths,
    }
}

/// Authored `<col>` sizing in grid order.
///
/// Walks the table's direct children (and `colgroup` children) for
/// `display: table-column` elements. `span` (HTML §4.9.8, max 1000)
/// expands to that many entries. Direct `<col>` children (anonymous
/// colgroup) are included in DOM order.
/// Authored sizing of one `<col>`: width + min/max-width (lengths only;
/// percentages and calc-with-percentage are indefinite at column-measure
/// time and handled by the caller).
#[derive(Debug, Clone, Copy)]
struct ColSizing {
    width: Dimension,
    min_width: Dimension,
    max_width: Dimension,
}

fn collect_col_widths(doc: &Document, table_idx: usize) -> Vec<ColSizing> {
    fn col_span(doc: &Document, node_id: usize) -> usize {
        if let crate::node::NodeData::Element(data) = &doc.nodes[node_id].data {
            for a in &data.attributes {
                if a.local.as_str() == "span"
                    && let Ok(v) = a.value.parse::<usize>()
                {
                    return v.clamp(1, 1000);
                }
            }
        }
        1
    }
    fn sizing_of(doc: &Document, col_id: usize) -> ColSizing {
        let st = &doc.nodes[col_id].style;
        ColSizing {
            width: st.size.width,
            min_width: st.min_size.width.into(),
            max_width: st.max_size.width.into(),
        }
    }
    let mut out = Vec::new();
    for &child_id in &doc.nodes[table_idx].children.clone() {
        if !doc.nodes[child_id].is_in_document() {
            continue;
        }
        match doc.nodes[child_id].display {
            DisplayValue::TableColumnGroup => {
                for &col_id in &doc.nodes[child_id].children.clone() {
                    if !doc.nodes[col_id].is_in_document() {
                        continue;
                    }
                    if doc.nodes[col_id].display != DisplayValue::TableColumn {
                        continue;
                    }
                    let sz = sizing_of(doc, col_id);
                    for _ in 0..col_span(doc, col_id) {
                        out.push(sz);
                    }
                }
            }
            DisplayValue::TableColumn => {
                let sz = sizing_of(doc, child_id);
                for _ in 0..col_span(doc, child_id) {
                    out.push(sz);
                }
            }
            _ => {}
        }
    }
    out
}

/// Visual section ordering for a table's direct children.
///
/// CSS 2.1 §17.5 orders the (single) thead before tbody rows and the
/// (single) tfoot after them. With repeated groups (author error, exercised
/// by WPT css/css-tables/row-group-order) engines keep DOM order except the
/// FIRST header group floats to the top and the FIRST footer group sinks to
/// the bottom — matching `HTMLTableElement.tHead`/`tFoot` (first match) and
/// reproducing row-group-order-ref exactly. Nested groups keep DOM order.
fn section_order(order: &mut [usize], displays: &[DisplayValue]) {
    let first_head = order
        .iter()
        .find(|&&i| displays[i] == DisplayValue::TableHeaderGroup)
        .copied();
    let first_foot = order
        .iter()
        .find(|&&i| displays[i] == DisplayValue::TableFooterGroup)
        .copied();
    order.sort_by_key(|&i| {
        if Some(i) == first_head {
            0u8
        } else if Some(i) == first_foot {
            2u8
        } else {
            1u8
        }
    });
}

fn collect_rows(
    doc: &Document,
    container_idx: usize,
    rows: &mut Vec<usize>,
    cells: &mut Vec<CellPlacement>,
    n_cols: &mut u16,
) {
    collect_rows_inner(doc, container_idx, rows, cells, n_cols, true)
}

fn collect_rows_inner(
    doc: &Document,
    container_idx: usize,
    rows: &mut Vec<usize>,
    cells: &mut Vec<CellPlacement>,
    n_cols: &mut u16,
    reorder_sections: bool,
) {
    let mut pending: Vec<usize> = Vec::new();

    let len = doc.nodes[container_idx].children.len();
    // At table level, float the first header group to the top and sink
    // the first footer group to the bottom (stable otherwise). Nested
    // groups keep DOM order.
    let order: Vec<usize> = if reorder_sections {
        let mut idxs: Vec<usize> = (0..len).collect();
        let displays: Vec<DisplayValue> = idxs
            .iter()
            .map(|&i| doc.nodes[doc.nodes[container_idx].children[i]].display)
            .collect();
        section_order(&mut idxs, &displays);
        idxs
    } else {
        (0..len).collect()
    };
    for i in order {
        let child_id = doc.nodes[container_idx].children[i];
        if !doc.nodes[child_id].is_in_document() {
            continue;
        }
        // Character data carries no grid structure: inter-cell whitespace
        // must not flush a pending anonymous row (each whitespace run would
        // otherwise split sibling cells into separate single-cell rows), and
        // text has no children to recurse into. Non-whitespace text directly
        // under table wrappers is dropped either way today (anonymous-cell
        // wrapping is out of scope), so skipping changes nothing for it.
        if !matches!(doc.nodes[child_id].data, crate::node::NodeData::Element(_)) {
            continue;
        }
        let disp = doc.nodes[child_id].display;
        let is_row = disp == DisplayValue::TableRow;
        let is_cell = disp == DisplayValue::TableCell;
        // Treat row-group / header / footer / contents as anonymous group
        let is_row_group = matches!(
            disp,
            DisplayValue::TableRowGroup
                | DisplayValue::TableHeaderGroup
                | DisplayValue::TableFooterGroup
                | DisplayValue::TableColumnGroup
                | DisplayValue::TableColumn
        );
        // `contents` acts as anonymous row-group per Blitz port, but for raikiri we treat similarly
        let is_contents = disp == DisplayValue::Contents;

        if is_row {
            flush_pending(doc, &mut pending, rows, cells, n_cols);
            collect_cells_in_row(doc, child_id, rows, cells, n_cols);
        } else if is_cell {
            pending.push(child_id);
        } else if is_row_group || is_contents {
            // Anonymous row-group: recurse. But flush pending first (cells cannot cross row-group boundary)
            flush_pending(doc, &mut pending, rows, cells, n_cols);
            collect_rows_inner(doc, child_id, rows, cells, n_cols, false);
        } else {
            // Non-table descendent (e.g., caption, div inside table). For initial implementation, skip but flush pending.
            flush_pending(doc, &mut pending, rows, cells, n_cols);
            // If this is a caption etc., ignore for grid. If it's inside table but not row/cell,
            // recursion not needed - those children are not part of table grid.
            // However if it's a plain div wrapping rows (anomalous), recurse to find rows inside.
            // Heuristic: recurse if it has children that could be rows/cells.
            if !doc.nodes[child_id].children.is_empty() {
                // Only recurse if container might hold rows; avoid diving into block content inside cells.
                // We treat any block container directly under table that contains row-like descendants as group.
                // Simple: recurse
                collect_rows_inner(doc, child_id, rows, cells, n_cols, false);
            }
        }
    }
    flush_pending(doc, &mut pending, rows, cells, n_cols);
}

fn flush_pending(
    doc: &Document,
    pending: &mut Vec<usize>,
    rows: &mut Vec<usize>,
    cells: &mut Vec<CellPlacement>,
    n_cols: &mut u16,
) {
    if pending.is_empty() {
        return;
    }
    // Anonymous row: borrow first cell's id as row marker (not used for layout beyond count)
    rows.push(pending[0]);
    let row_ix = (rows.len() - 1) as u16;
    let mut col: u16 = 0;
    for &cell_id in pending.iter() {
        let col_span = get_colspan(doc, cell_id);
        let row_span = get_rowspan(doc, cell_id);
        let specified_width = doc.nodes[cell_id].style.size.width;
        cells.push(CellPlacement {
            node_id: cell_id,
            row: row_ix,
            col_start: col,
            col_span,
            row_span,
            specified_width,
            resolved: None,
        });
        col += col_span;
    }
    *n_cols = (*n_cols).max(col);
    pending.clear();
}

fn collect_cells_in_row(
    doc: &Document,
    row_id: usize,
    rows: &mut Vec<usize>,
    cells: &mut Vec<CellPlacement>,
    n_cols: &mut u16,
) {
    rows.push(row_id);
    let row_ix = (rows.len() - 1) as u16;
    let mut col: u16 = 0;
    for i in 0..doc.nodes[row_id].children.len() {
        let cell_id = doc.nodes[row_id].children[i];
        if !doc.nodes[cell_id].is_in_document() {
            continue;
        }
        // Only collect cells; non-cell children are anonymous? Per spec, non-cell child of a row
        // is wrapped in anonymous cell — for initial implementation we skip non-cells inside row.
        if doc.nodes[cell_id].display != DisplayValue::TableCell {
            continue;
        }
        let col_span = get_colspan(doc, cell_id);
        let row_span = get_rowspan(doc, cell_id);
        let specified_width = doc.nodes[cell_id].style.size.width;
        cells.push(CellPlacement {
            node_id: cell_id,
            row: row_ix,
            col_start: col,
            col_span,
            row_span,
            specified_width,
            resolved: None,
        });
        col += col_span;
    }
    *n_cols = (*n_cols).max(col);
}

fn get_colspan(doc: &Document, node_id: usize) -> u16 {
    if let crate::node::NodeData::Element(data) = &doc.nodes[node_id].data {
        for a in &data.attributes {
            if a.local.as_str() == "colspan"
                && let Ok(v) = a.value.parse::<u16>()
            {
                return v.max(1);
            }
        }
    }
    // Also check html attribute via ElementData retrieval alternative: try parsing int, clamp to at least 1, huge values clamp to reasonable?
    // spec allows up to 1000; we clamp to 1000 for safety.
    1
}

fn get_rowspan(doc: &Document, node_id: usize) -> u16 {
    if let crate::node::NodeData::Element(data) = &doc.nodes[node_id].data {
        for a in &data.attributes {
            if a.local.as_str() == "rowspan"
                && let Ok(v) = a.value.parse::<u16>()
            {
                // HTML §4.9.9: rowspan=0 spans to the end of the table
                // section. The grid is section-flat, so approximate with a
                // saturating span — every consumer clamps to the row count
                // (table end == section end for single-section tables).
                if v == 0 {
                    return u16::MAX;
                }
                return v.clamp(1, 65534);
            }
        }
    }
    1
}

// ---------------------------------------------------------------------------
// Measure cells
// ---------------------------------------------------------------------------

fn measure_cell_inline(doc: &mut Document, cell_id: usize, axis: AvailableSpace) -> f32 {
    // Use a fresh measure: indefinite parent size so that % widths resolve to auto (spec §2.3).
    // Height available is MAX_CONTENT (intrinsic).
    let output = doc.compute_child_layout(
        NodeId::from(cell_id),
        LayoutInput {
            run_mode: RunMode::ComputeSize,
            sizing_mode: SizingMode::InherentSize,
            axis: taffy::tree::RequestedAxis::Horizontal,
            known_dimensions: Size::NONE,
            parent_size: Size::NONE,
            available_space: Size {
                width: axis,
                height: AvailableSpace::MAX_CONTENT,
            },
            known_dimensions_are_definite: taffy::geometry::Size {
                width: true,
                height: true,
            },
            vertical_margins_are_collapsible: taffy::geometry::Line::FALSE,
        },
    );
    output.size.width
}

// ---------------------------------------------------------------------------
// Column sizing — ported from Blitz L3 (table layout design)
// ---------------------------------------------------------------------------

fn resolve_column_widths(
    doc: &mut Document,
    grid: &TableGrid,
    inputs: LayoutInput,
    padding_border_size: Size<f32>,
) -> Vec<f32> {
    let n = grid.n_cols as usize;
    // (B) Measure each cell
    let mut cell_min = vec![0.0f32; grid.cells.len()];
    let mut cell_max = vec![0.0f32; grid.cells.len()];
    for (i, cell) in grid.cells.iter().enumerate() {
        cell_min[i] = measure_cell_inline(doc, cell.node_id, AvailableSpace::MIN_CONTENT);
        cell_max[i] = measure_cell_inline(doc, cell.node_id, AvailableSpace::MAX_CONTENT);
    }

    // (C.1) Aggregate colspan==1
    let mut col_min = vec![0.0f32; n];
    let mut col_max = vec![0.0f32; n];
    let mut col_pct = vec![0.0f32; n];
    for (i, cell) in grid.cells.iter().enumerate() {
        let c = cell.col_start as usize;
        if cell.col_span == 1 && c < n {
            col_min[c] = f32_max_compat(col_min[c], cell_min[i]);
            col_max[c] = f32_max_compat(col_max[c], cell_max[i]);
            if cell.specified_width.tag() == CompactLength::PERCENT_TAG {
                col_pct[c] = f32_max_compat(col_pct[c], cell.specified_width.value());
            }
        }
    }
    let mut col_min_full = col_min.clone();
    let mut col_max_full = col_max.clone();

    // (C.2) colspan>1 excess distribution (smallest span first). A
    // definite single-column width already constrains that track, so a
    // spanning cell's excess is assigned to the remaining tracks whenever
    // possible (CSS Tables track sizing, intrinsic minimum phase).
    let authored_length = (0..n)
        .map(|column| {
            grid.cells.iter().any(|cell| {
                cell.col_span == 1
                    && usize::from(cell.col_start) == column
                    && cell.specified_width.tag() == CompactLength::LENGTH_TAG
            })
        })
        .collect::<Vec<_>>();
    let mut spans: Vec<(usize, u16, u16)> = grid
        .cells
        .iter()
        .enumerate()
        .filter(|(_, c)| c.col_span > 1)
        .map(|(i, c)| (i, c.col_start, c.col_span))
        .collect();
    spans.sort_by_key(|&(_, _, span)| span);
    for (i, col_start, span) in spans {
        let s = col_start as usize;
        let e = (s + span as usize).min(n);
        if s >= e {
            continue;
        }
        let targets: Vec<usize> = (s..e).filter(|&column| !authored_length[column]).collect();
        let targets = if targets.is_empty() {
            (s..e).collect()
        } else {
            targets
        };
        let cnt = targets.len() as f32;
        let cur_min: f32 = col_min_full[s..e].iter().sum();
        if cell_min[i] > cur_min {
            let add = (cell_min[i] - cur_min) / cnt;
            for column in &targets {
                col_min_full[*column] += add;
            }
        }
        let cur_max: f32 = col_max_full[s..e].iter().sum();
        if cell_max[i] > cur_max {
            let add = (cell_max[i] - cur_max) / cnt;
            for column in &targets {
                col_max_full[*column] += add;
            }
        }
        if grid.cells[i].specified_width.tag() == CompactLength::PERCENT_TAG {
            let p = grid.cells[i].specified_width.value();
            let cur_p: f32 = col_pct[s..e].iter().sum();
            if p > cur_p {
                let add = (p - cur_p) / cnt;
                for column in &targets {
                    col_pct[*column] += add;
                }
            }
        }
    }

    // (C.3) Per-column authored length: width:<px>
    let mut col_len: Vec<Option<f32>> = vec![None; n];
    for cell in &grid.cells {
        let c = cell.col_start as usize;
        if cell.col_span == 1
            && c < n
            && cell.specified_width.tag() == CompactLength::LENGTH_TAG
            && col_len[c].is_none()
        {
            col_len[c] = Some(cell.specified_width.value());
        }
    }
    for c in 0..n {
        if let Some(l) = col_len[c] {
            col_min[c] = f32_max_compat(col_min[c], l);
            col_max[c] = f32_max_compat(col_max[c], l);
            col_min_full[c] = f32_max_compat(col_min_full[c], l);
            col_max_full[c] = f32_max_compat(col_max_full[c], l);
        }
    }
    // (C.3b) `<col>` authored sizing (css-tables-3 missing-cells-fixup /
    // outer-max-content, WPT col-definite-*): definite lengths floor (and
    // cap via max-width) even empty columns; percentages only constrain
    // occupied columns (indefinite for missing cells); percent/calc
    // min/max are indefinite and ignored. CSS Sizing 3 §4/§5: min > max
    // resolves to min (max ignored).
    let occupied = {
        let mut occ = vec![false; n];
        for cell in grid.cells.iter() {
            let s = cell.col_start as usize;
            let e = (s + cell.col_span as usize).min(n);
            for slot in occ.iter_mut().take(e).skip(s) {
                *slot = true;
            }
        }
        occ
    };
    for (c, sz) in grid.col_widths.iter().enumerate() {
        if c >= n {
            break;
        }
        let is_len = |d: Dimension| d.tag() == CompactLength::LENGTH_TAG;
        let min_w = is_len(sz.min_width).then(|| sz.min_width.value());
        let max_w = is_len(sz.max_width).then(|| sz.max_width.value());
        // Base width: definite length always; percent only when occupied.
        let mut base: Option<f32> = None;
        if is_len(sz.width) {
            base = Some(sz.width.value());
        } else if sz.width.tag() == CompactLength::PERCENT_TAG && occupied[c] {
            col_pct[c] = f32_max_compat(col_pct[c], sz.width.value());
        }
        // Clamp base by min/max (min wins over max).
        let mut floor = min_w;
        if let (Some(mn), Some(mx)) = (min_w, max_w)
            && mn > mx
        {
            floor = Some(mn);
        }
        if let Some(b) = base {
            let mut v = b;
            if let Some(mn) = min_w {
                v = f32_max_compat(v, mn);
            }
            if let Some(mx) = max_w
                && !(min_w.is_some_and(|mn| mn > mx))
            {
                v = v.min(mx);
            }
            floor = Some(floor.map_or(v, |f| f32_max_compat(f, v)));
        }
        if let Some(f) = floor {
            col_min[c] = f32_max_compat(col_min[c], f);
            col_max[c] = f32_max_compat(col_max[c], f);
            col_min_full[c] = f32_max_compat(col_min_full[c], f);
            col_max_full[c] = f32_max_compat(col_max_full[c], f);
        }
    }
    for i in 0..n {
        col_max[i] = f32_max_compat(col_max[i], col_min[i]);
        col_max_full[i] = f32_max_compat(col_max_full[i], col_min_full[i]);
    }

    // (C.4) Distribute - auto only (fixed deferred to Wave4)
    let insets = padding_border_size.width;
    match inputs.known_dimensions.width {
        Some(w) => {
            let avail = f32_max_compat(w - insets, 0.0);
            distribute_columns_with_authored(
                avail,
                &col_min_full,
                &col_max_full,
                &col_pct,
                &authored_length,
            )
        }
        None => match inputs.available_space.width {
            AvailableSpace::MinContent => col_min_full,
            AvailableSpace::MaxContent => col_max_full,
            AvailableSpace::Definite(avail_w) => {
                let avail = f32_max_compat(avail_w - insets, 0.0);
                if col_max_full.iter().sum::<f32>() <= avail {
                    col_max_full
                } else {
                    distribute_columns(avail, &col_min, &col_max, &col_pct)
                }
            }
        },
    }
}

fn distribute_columns(avail: f32, min: &[f32], max: &[f32], pct: &[f32]) -> Vec<f32> {
    let n = min.len();
    let psum: f32 = pct.iter().sum();
    let scale = if psum > 1.0 { 1.0 / psum } else { 1.0 };
    let mut w = vec![0.0f32; n];
    for i in 0..n {
        w[i] = if pct[i] > 0.0 {
            f32_max_compat(pct[i] * scale * avail, min[i])
        } else {
            min[i]
        };
    }
    let assigned: f32 = w.iter().sum();
    if assigned + 0.01 < avail {
        let mut remaining = avail - assigned;
        let grow: Vec<f32> = (0..n)
            .map(|i| {
                if pct[i] == 0.0 {
                    f32_max_compat(max[i] - w[i], 0.0)
                } else {
                    0.0
                }
            })
            .collect();
        let gsum: f32 = grow.iter().sum();
        if gsum > 0.0 {
            let take = if remaining < gsum { remaining } else { gsum };
            for i in 0..n {
                w[i] += take * grow[i] / gsum;
            }
            remaining -= take;
        }
        if remaining > 0.0 {
            let np: Vec<usize> = (0..n).filter(|&i| pct[i] == 0.0).collect();
            let targets = if np.is_empty() { (0..n).collect() } else { np };
            let share = remaining / targets.len() as f32;
            for i in targets {
                w[i] += share;
            }
        }
    } else if assigned > avail + 0.01 {
        let excess = assigned - avail;
        let shrink: Vec<f32> = (0..n).map(|i| f32_max_compat(w[i] - min[i], 0.0)).collect();
        let ssum: f32 = shrink.iter().sum();
        if ssum > 0.0 {
            let take = if excess < ssum { excess } else { ssum };
            for i in 0..n {
                w[i] -= take * shrink[i] / ssum;
            }
        }
    }
    w
}

fn distribute_columns_with_authored(
    avail: f32,
    min: &[f32],
    max: &[f32],
    pct: &[f32],
    authored_length: &[bool],
) -> Vec<f32> {
    let n = min.len();
    let psum: f32 = pct.iter().sum();
    let scale = if psum > 1.0 { 1.0 / psum } else { 1.0 };
    let mut w = vec![0.0f32; n];
    for i in 0..n {
        w[i] = if pct[i] > 0.0 {
            f32_max_compat(pct[i] * scale * avail, min[i])
        } else {
            min[i]
        };
    }
    let assigned: f32 = w.iter().sum();
    if assigned + 0.01 < avail {
        let mut remaining = avail - assigned;
        let grow: Vec<f32> = (0..n)
            .map(|i| {
                if pct[i] == 0.0 {
                    f32_max_compat(max[i] - w[i], 0.0)
                } else {
                    0.0
                }
            })
            .collect();
        let gsum: f32 = grow.iter().sum();
        if gsum > 0.0 {
            let take = if remaining < gsum { remaining } else { gsum };
            for i in 0..n {
                w[i] += take * grow[i] / gsum;
            }
            remaining -= take;
        }
        if remaining > 0.0 {
            let np: Vec<usize> = (0..n)
                .filter(|&i| pct[i] == 0.0 && !authored_length[i])
                .collect();
            let targets = if np.is_empty() {
                let all_auto: Vec<usize> = (0..n).filter(|&i| !authored_length[i]).collect();
                if all_auto.is_empty() {
                    (0..n).collect()
                } else {
                    all_auto
                }
            } else {
                np
            };
            let share = remaining / targets.len() as f32;
            for i in targets {
                w[i] += share;
            }
        }
    } else if assigned > avail + 0.01 {
        let excess = assigned - avail;
        let shrink: Vec<f32> = (0..n).map(|i| f32_max_compat(w[i] - min[i], 0.0)).collect();
        let ssum: f32 = shrink.iter().sum();
        if ssum > 0.0 {
            let take = if excess < ssum { excess } else { ssum };
            for i in 0..n {
                w[i] -= take * shrink[i] / ssum;
            }
        } // cov:ignore: llvm-cov reports the covered shrink branch's closing brace as uncovered.
    } // cov:ignore: llvm-cov reports the covered shrink branch's closing brace as uncovered.
    w
}

fn distribute_extra_height(row_heights: &mut [f32], target: f32) {
    let n = row_heights.len();
    if n == 0 {
        return;
    }
    let current: f32 = row_heights.iter().sum();
    if current >= target {
        return;
    }
    let extra = target - current;
    if current > 0.0 {
        for h in row_heights.iter_mut() {
            *h += extra * (*h) / current;
        }
    } else {
        let share = extra / n as f32;
        for h in row_heights.iter_mut() {
            *h += share;
        }
    }
}

/// Extra min-width grows columns (mirrors [`distribute_extra_height`] for
/// rows; same proportional-share rule). Tracks are sized in pre-overlap
/// space, so the caller passes the outer-domain target
/// (`min_outer - distrib_insets.width` = `min_content + overlap`).
fn distribute_extra_width(column_widths: &mut [f32], target: f32) {
    let n = column_widths.len();
    if n == 0 {
        return;
    }
    let current: f32 = column_widths.iter().sum();
    if current >= target {
        return;
    }
    let extra = target - current;
    if current > 0.0 {
        for w in column_widths.iter_mut() {
            *w += extra * (*w) / current;
        }
    } else {
        let share = extra / n as f32;
        for w in column_widths.iter_mut() {
            *w += share;
        }
    }
}

// ---------------------------------------------------------------------------
// Row heights
// ---------------------------------------------------------------------------

fn resolve_row_heights(doc: &mut Document, grid: &TableGrid, column_widths: &[f32]) -> Vec<f32> {
    let mut row_heights = vec![0.0f32; grid.rows.len()];
    // Authored `height` on rows floors the row (CSS 2.1 §17.5.3; lengths
    // only — percentages need the table height, indefinite at this stage).
    // Anonymous-row markers reuse the first cell's id; flooring by a cell's
    // own height there is consistent with its content measure.
    // Vertical writing modes are NOT adjusted here: in vertical-rl the row's
    // block axis is horizontal, which this horizontal-centric engine does not
    // model (css/css-tables/paint/col-paint-vrl-rtl.html covers it and needs
    // full vertical-table support plus transform:rotate on its reference).
    for (r, &row_id) in grid.rows.iter().enumerate() {
        let h = doc.nodes[row_id].style.size.height;
        if h.tag() == CompactLength::LENGTH_TAG {
            row_heights[r] = f32_max_compat(row_heights[r], h.value());
        }
    }
    for cell in &grid.cells {
        let end = (cell.col_start as usize + cell.col_span as usize).min(column_widths.len());
        let cell_width: f32 = column_widths[cell.col_start as usize..end].iter().sum();
        let output = doc.compute_child_layout(
            NodeId::from(cell.node_id),
            LayoutInput {
                run_mode: RunMode::PerformLayout,
                sizing_mode: SizingMode::InherentSize,
                axis: taffy::tree::RequestedAxis::Horizontal,
                known_dimensions: Size {
                    width: Some(cell_width),
                    height: None,
                },
                parent_size: Size {
                    width: Some(cell_width),
                    height: None,
                },
                available_space: Size {
                    width: AvailableSpace::Definite(cell_width),
                    height: AvailableSpace::MAX_CONTENT,
                },
                known_dimensions_are_definite: taffy::geometry::Size {
                    width: true,
                    height: true,
                },
                vertical_margins_are_collapsible: taffy::geometry::Line::FALSE,
            },
        );
        // Distribute rowspan height across spanned rows (simple: equally)
        let h = output.size.height;
        let rs = cell.row_span as usize;
        if rs == 1 {
            row_heights[cell.row as usize] = f32_max_compat(row_heights[cell.row as usize], h);
        } else {
            let start = cell.row as usize;
            let end_row = (start + rs).min(row_heights.len());
            let cnt = (end_row - start) as f32;
            if cnt > 0.0 {
                // Need to ensure span can accommodate h: if sum current < h, distribute deficit
                let cur: f32 = row_heights[start..end_row].iter().sum();
                if h > cur {
                    let add = (h - cur) / cnt;
                    for slot in row_heights[start..end_row].iter_mut() {
                        *slot += add;
                    }
                }
            }
        }
    }
    // Ensure every row has at least min (empty rows get 0 -> keep 0)
    row_heights
}

// ---------------------------------------------------------------------------
// Positioning
// ---------------------------------------------------------------------------

/// Cumulative track origins from sizes minus collapsed-line overlaps.
///
/// `origins.len() == sizes.len() + 1`, `origins[0] == origin`, and
/// `origins[i + 1] - origins[i] == sizes[i] - overlaps[i]` where
/// `overlaps[i]` is the collapsed line between track `i` and `i + 1`
/// (zero for the separate model). A cell spanning tracks `i..j` therefore
/// covers `origins[j] - origins[i]` — interior collapsed lines are absorbed
/// into the spanning cell rather than double-counted.
fn track_origins(sizes: &[f32], overlaps: &[f32], origin: f32) -> Vec<f32> {
    let mut out = vec![0.0f32; sizes.len() + 1];
    out[0] = origin;
    for i in 0..sizes.len() {
        let ov = overlaps.get(i).copied().unwrap_or(0.0);
        out[i + 1] = out[i] + sizes[i] - ov;
    }
    out
}

fn place_cells(
    doc: &mut Document,
    grid: &mut TableGrid,
    column_widths: &[f32],
    row_heights: &[f32],
    col_x: &[f32],
    row_y: &[f32],
) {
    for (order, cell) in grid.cells.iter_mut().enumerate() {
        let end_col = (cell.col_start as usize + cell.col_span as usize).min(column_widths.len());
        let end_row = (cell.row as usize + cell.row_span as usize).min(row_heights.len());
        let cell_x = col_x[cell.col_start as usize];
        let cell_y = row_y[cell.row as usize];
        // Full spanned widths — origins already net out the collapsed
        // overlaps for *positioning*; shrinking the box by the overlap as
        // well would double-count the absorbed line (separate model:
        // origins differences equal these sums, so this is a no-op there).
        let cell_width: f32 = column_widths[cell.col_start as usize..end_col].iter().sum();
        let cell_height: f32 = row_heights[cell.row as usize..end_row].iter().sum();
        // Final layout for cell contents with definite size
        let output = doc.compute_child_layout(
            NodeId::from(cell.node_id),
            LayoutInput {
                run_mode: RunMode::PerformLayout,
                sizing_mode: SizingMode::InherentSize,
                axis: taffy::tree::RequestedAxis::Horizontal,
                known_dimensions: Size {
                    width: Some(cell_width),
                    height: Some(cell_height),
                },
                parent_size: Size {
                    width: Some(cell_width),
                    height: Some(cell_height),
                },
                available_space: Size {
                    width: AvailableSpace::Definite(cell_width),
                    height: AvailableSpace::Definite(cell_height),
                },
                known_dimensions_are_definite: taffy::geometry::Size {
                    width: true,
                    height: true,
                },
                vertical_margins_are_collapsible: taffy::geometry::Line::FALSE,
            },
        );
        let layout = TaffyLayout {
            order: order as u32,
            location: Point {
                x: cell_x,
                y: cell_y,
            },
            size: Size {
                width: cell_width,
                height: cell_height,
            },
            scrollable_overflow_rect: output.scrollable_overflow_rect,
            scrollbar_size: Size::ZERO,
            padding: Rect::ZERO,
            border: Rect::ZERO,
            margin: Rect::ZERO,
        };
        cell.resolved = Some(layout);
        // Write via Document's layout storage (includes sanitize)
        {
            let sanitized = super::sanitize_taffy_layout(&layout, &mut doc.layout_warnings);
            doc.nodes[cell.node_id].unrounded_layout = sanitized;
        }
        let _ = output;
    }
}

/// Re-map the already measured table grid from horizontal physical axes to
/// vertical writing-mode axes. The normal placement pass remains authoritative
/// for child sizing; this pass changes only the cell track positions and the
/// vertical span used by the table grid.
fn reposition_cells_for_vertical_writing(
    doc: &mut Document,
    grid: &mut TableGrid,
    column_widths: &[f32],
    row_heights: &[f32],
    cell_width: f32,
    x_origin: f32,
    y_origin: f32,
) {
    let vertical_track = row_heights.iter().sum::<f32>();
    if !vertical_track.is_finite() || vertical_track <= 0.0 {
        return; // cov:ignore: defensive empty/invalid vertical track fallback.
    }
    for cell in &mut grid.cells {
        let col_start = cell.col_start as usize;
        let col_end = col_start
            .saturating_add(cell.col_span as usize)
            .min(column_widths.len());
        let col_span = col_end.saturating_sub(col_start).max(1);
        let y = y_origin + vertical_track * col_start as f32;
        let height = vertical_track * col_span as f32;
        let Some(layout) = cell.resolved.as_mut() else {
            continue; // cov:ignore: defensive unresolved-cell fallback.
        };
        layout.location = Point { x: x_origin, y };
        layout.size.width = cell_width;
        layout.size.height = height;
        let sanitized = super::sanitize_taffy_layout(layout, &mut doc.layout_warnings);
        doc.nodes[cell.node_id].unrounded_layout = sanitized;
    }
}

// ---------------------------------------------------------------------------
// Helpers for style resolution
// ---------------------------------------------------------------------------

fn resolve_dimension(d: Dimension, basis: Option<f32>) -> Option<f32> {
    match d.tag() {
        x if x == CompactLength::LENGTH_TAG => Some(d.value()),
        x if x == CompactLength::PERCENT_TAG => basis.map(|b| b * d.value()),
        x if x == CompactLength::AUTO_TAG => None,
        _ => None,
    }
}

fn resolve_length(lp: taffy::style::LengthPercentage, basis: Option<f32>) -> f32 {
    // LengthPercentage in style.padding / border — Dimension-like but wrapped
    let raw = lp.into_raw();
    match raw.tag() {
        x if x == CompactLength::LENGTH_TAG => raw.value(),
        x if x == CompactLength::PERCENT_TAG => basis.map(|b| b * raw.value()).unwrap_or(0.0),
        _ => 0.0,
    }
}

fn resolve_table_padding(
    doc: &Document,
    table_idx: usize,
    parent_size: Size<Option<f32>>,
) -> Rect<f32> {
    let s = &doc.nodes[table_idx].style;
    let w_basis = parent_size.width;
    Rect {
        left: resolve_length(s.padding.left, w_basis),
        right: resolve_length(s.padding.right, w_basis),
        top: resolve_length(s.padding.top, w_basis),
        bottom: resolve_length(s.padding.bottom, w_basis),
    }
}

fn resolve_table_border(
    doc: &Document,
    table_idx: usize,
    parent_size: Size<Option<f32>>,
) -> Rect<f32> {
    let s = &doc.nodes[table_idx].style;
    let w_basis = parent_size.width;
    Rect {
        left: resolve_length(s.border.left, w_basis),
        right: resolve_length(s.border.right, w_basis),
        top: resolve_length(s.border.top, w_basis),
        bottom: resolve_length(s.border.bottom, w_basis),
    }
}

// ---------------------------------------------------------------------------
// Fixed layout (CSS 2.1 §17.5.2.1)
// ---------------------------------------------------------------------------

/// Fixed column sizing: no content measurement.
///
/// `avail` is the table width net of padding + outer (collapsed) borders.
/// Per §17.5.2.1, a first-row cell with `colspan == 1` and a specified
/// `width` (length or percentage-of-table-width) fixes its column; all
/// remaining columns divide the leftover equally. Specified widths in
/// non-first rows, `colspan > 1` first-row widths, and `col`-element widths
/// are ignored (module-doc scope limit). Leftover underflow clamps unfixed
/// columns to zero (the table overflows rather than shrinking fixed
/// columns).
fn resolve_fixed_column_widths(grid: &TableGrid, avail: f32) -> Vec<f32> {
    let n = grid.n_cols as usize;
    let mut fixed: Vec<Option<f32>> = vec![None; n];
    for cell in &grid.cells {
        if cell.row != 0 || cell.col_span != 1 {
            continue;
        }
        let c = cell.col_start as usize;
        if c < n && fixed[c].is_none() {
            fixed[c] = match cell.specified_width.tag() {
                x if x == CompactLength::LENGTH_TAG => Some(cell.specified_width.value()),
                x if x == CompactLength::PERCENT_TAG => Some(cell.specified_width.value() * avail),
                _ => None,
            };
        }
    }
    // `<col>` widths fix columns with no first-row cell width (CSS 2.1 §17.5.2.1).
    // Percentages resolve against the same avail basis as cell percentages.
    // Length min/max clamp the fallback (same min-wins rule as auto path).
    for (c, sz) in grid.col_widths.iter().enumerate() {
        if c >= n || fixed[c].is_some() {
            continue;
        }
        let is_len = |d: Dimension| d.tag() == CompactLength::LENGTH_TAG;
        let mut v: Option<f32> = None;
        if is_len(sz.width) {
            v = Some(sz.width.value());
        } else if sz.width.tag() == CompactLength::PERCENT_TAG {
            v = Some(sz.width.value() * avail);
        }
        if v.is_none() && is_len(sz.min_width) {
            v = Some(sz.min_width.value());
        }
        if let Some(mut x) = v {
            if is_len(sz.min_width) {
                x = f32_max_compat(x, sz.min_width.value());
            }
            if is_len(sz.max_width)
                && !(is_len(sz.min_width) && sz.min_width.value() > sz.max_width.value())
            {
                x = x.min(sz.max_width.value());
            }
            fixed[c] = Some(x);
        }
    }
    let fixed_sum: f32 = fixed.iter().filter_map(|v| *v).sum();
    let unfixed = fixed.iter().filter(|v| v.is_none()).count();
    let share = if unfixed > 0 {
        f32_max_compat((avail - fixed_sum) / unfixed as f32, 0.0)
    } else {
        0.0
    };
    fixed.iter().map(|v| v.unwrap_or(share)).collect()
}

// ---------------------------------------------------------------------------
// Collapsed borders (CSS 2.1 §17.6.2, simplified max-width resolution)
// ---------------------------------------------------------------------------

/// Collapsed-line metrics for one table.
///
/// - `col_overlaps[i]` — width absorbed between column `i` and `i + 1`
///   (length `n_cols - 1`; empty for ≤1 column).
/// - `row_overlaps[j]` — same between row `j` and `j + 1`.
/// - `outer_*` — collapsed outer border on each side (max of the table's
///   own border and the edge cells' adjoining borders).
///
/// Separate model: all overlaps zero and outers equal the table's own
/// border widths ([`CollapsedLines::separate`]).
#[derive(Debug)]
struct CollapsedLines {
    col_overlaps: Vec<f32>,
    row_overlaps: Vec<f32>,
    outer_left: f32,
    outer_right: f32,
    outer_top: f32,
    outer_bottom: f32,
}

impl CollapsedLines {
    fn separate(table_border: &Rect<f32>) -> Self {
        Self {
            col_overlaps: Vec::new(),
            row_overlaps: Vec::new(),
            outer_left: table_border.left,
            outer_right: table_border.right,
            outer_top: table_border.top,
            outer_bottom: table_border.bottom,
        }
    }
}

/// Border-box side widths of one cell's `style.border`.
fn cell_border_sides(doc: &Document, cell_id: usize, basis: Option<f32>) -> Rect<f32> {
    let s = &doc.nodes[cell_id].style;
    Rect {
        left: resolve_length(s.border.left, basis),
        right: resolve_length(s.border.right, basis),
        top: resolve_length(s.border.top, basis),
        bottom: resolve_length(s.border.bottom, basis),
    }
}

/// The cell covering grid position (`row`, `col`), following spans.
fn cell_at(grid: &TableGrid, row: u16, col: u16) -> Option<&CellPlacement> {
    grid.cells.iter().find(|c| {
        row >= c.row
            && row < c.row + c.row_span
            && col >= c.col_start
            && col < c.col_start + c.col_span
    })
}

fn compute_collapsed_lines(
    doc: &Document,
    grid: &TableGrid,
    table_border: &Rect<f32>,
    basis: Option<f32>,
) -> CollapsedLines {
    let n = grid.n_cols as usize;
    let m = grid.rows.len();

    // Per-cell border cache (row-major over grid.cells).
    let borders: Vec<Rect<f32>> = grid
        .cells
        .iter()
        .map(|c| cell_border_sides(doc, c.node_id, basis))
        .collect();
    let border_of = |cell: &CellPlacement| -> Rect<f32> {
        let pos = grid
            .cells
            .iter()
            .position(|c| c.node_id == cell.node_id)
            .expect("cell must be in grid");
        borders[pos]
    };

    // Vertical boundaries 0..=n. Boundary i separates column i-1 and i;
    // the collapsed line is the max of adjoining borders, and the overlap
    // (absorbed width) is the row-wise max of the pairwise min.
    let mut col_overlaps = vec![0.0f32; n.saturating_sub(1)];
    let mut outer_left = table_border.left;
    let mut outer_right = table_border.right;
    for i in 0..=n {
        if i == 0 {
            for r in 0..m {
                if let Some(c) = cell_at(grid, r as u16, 0)
                    && c.col_start == 0
                {
                    outer_left = f32_max_compat(outer_left, border_of(c).left);
                }
            }
        } else if i == n {
            for r in 0..m {
                if let Some(c) = cell_at(grid, r as u16, (n as u16).saturating_sub(1))
                    && c.col_start + c.col_span == n as u16
                {
                    outer_right = f32_max_compat(outer_right, border_of(c).right);
                }
            }
        } else {
            let mut ov = 0.0f32;
            for r in 0..m {
                let left = cell_at(grid, r as u16, (i as u16).saturating_sub(1))
                    .filter(|c| c.col_start + c.col_span == i as u16);
                let right = cell_at(grid, r as u16, i as u16).filter(|c| c.col_start == i as u16);
                if let (Some(a), Some(b)) = (left, right) {
                    let (ra, lb) = (border_of(a).right, border_of(b).left);
                    ov = f32_max_compat(ov, ra.min(lb));
                }
            }
            col_overlaps[i - 1] = ov;
        }
    }

    // Horizontal boundaries 0..=m, symmetric.
    let mut row_overlaps = vec![0.0f32; m.saturating_sub(1)];
    let mut outer_top = table_border.top;
    let mut outer_bottom = table_border.bottom;
    for j in 0..=m {
        if j == 0 {
            for c in grid.cells.iter().filter(|c| c.row == 0) {
                outer_top = f32_max_compat(outer_top, border_of(c).top);
            }
        } else if j == m {
            for c in grid
                .cells
                .iter()
                .filter(|c| c.row as usize + c.row_span as usize == m)
            {
                outer_bottom = f32_max_compat(outer_bottom, border_of(c).bottom);
            }
        } else {
            let mut ov = 0.0f32;
            for i in 0..n {
                let above = cell_at(grid, (j as u16).saturating_sub(1), i as u16)
                    .filter(|c| c.row as usize + c.row_span as usize == j);
                let below = cell_at(grid, j as u16, i as u16).filter(|c| c.row as usize == j);
                if let (Some(a), Some(b)) = (above, below) {
                    let (ba, tb) = (border_of(a).bottom, border_of(b).top);
                    ov = f32_max_compat(ov, ba.min(tb));
                }
            }
            row_overlaps[j - 1] = ov;
        }
    }

    CollapsedLines {
        col_overlaps,
        row_overlaps,
        outer_left,
        outer_right,
        outer_top,
        outer_bottom,
    }
}

#[cfg(test)]
mod tests {
    use crate::document::Document;
    use parley::FontContext;
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;
    use taffy::Style;

    fn build_simple_2x2() -> Document {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let table = doc.append_element(
            Some(body),
            "table",
            Style::default(),
            Some("display: table"),
        );
        let tr1 = doc.append_element(
            Some(table),
            "tr",
            Style::default(),
            Some("display: table-row"),
        );
        let td1 = doc.append_element(
            Some(tr1),
            "td",
            Style::default(),
            Some("display: table-cell"),
        );
        let td2 = doc.append_element(
            Some(tr1),
            "td",
            Style::default(),
            Some("display: table-cell"),
        );
        let _t1 = doc.append_text(td1, "cell1");
        let _t2 = doc.append_text(td2, "cell2");
        let tr2 = doc.append_element(
            Some(table),
            "tr",
            Style::default(),
            Some("display: table-row"),
        );
        let td3 = doc.append_element(
            Some(tr2),
            "td",
            Style::default(),
            Some("display: table-cell"),
        );
        let td4 = doc.append_element(
            Some(tr2),
            "td",
            Style::default(),
            Some("display: table-cell"),
        );
        let _t3 = doc.append_text(td3, "cell3");
        let _t4 = doc.append_text(td4, "cell4");
        doc
    }

    #[test]
    fn table_simple_2x2_columns_and_rows() {
        let mut doc = build_simple_2x2();
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade");
        crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new())
            .expect("layout");
        let td_idxs: Vec<usize> = doc
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.tag_name() == Some("td"))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(td_idxs.len(), 4);
        let l0 = doc.nodes[td_idxs[0]].unrounded_layout;
        let l1 = doc.nodes[td_idxs[1]].unrounded_layout;
        let l2 = doc.nodes[td_idxs[2]].unrounded_layout;
        assert!(
            (l1.location.x - l0.location.x).abs() > 1.0,
            "columns must separate: {} vs {}",
            l1.location.x,
            l0.location.x
        );
        assert!(
            (l2.location.x - l0.location.x).abs() < 1.0,
            "first column x must align vertically"
        );
        assert!(
            (l2.location.y - l0.location.y).abs() > 1.0,
            "rows must separate"
        );
        // table outer size should cover at least sum of two columns
        let table_idx = doc
            .nodes
            .iter()
            .position(|n| n.tag_name() == Some("table"))
            .unwrap();
        let tl = doc.nodes[table_idx].unrounded_layout;
        let expected_w = l0.size.width + l1.size.width;
        assert!(
            tl.size.width >= expected_w - 1.0,
            "table width {} should >= content {}",
            tl.size.width,
            expected_w
        );
    }

    #[test]
    fn table_vertical_writing_mode_maps_columns_to_inline_axis() {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let table = doc.append_element(
            Some(body),
            "table",
            Style::default(),
            Some("display: table; writing-mode: vertical-lr"),
        );
        let row = doc.append_element(
            Some(table),
            "tr",
            Style::default(),
            Some("display: table-row"),
        );
        let first = doc.append_element(
            Some(row),
            "td",
            Style::default(),
            Some("display: table-cell"),
        );
        let second = doc.append_element(
            Some(row),
            "td",
            Style::default(),
            Some("display: table-cell"),
        );
        let _first_child = doc.append_element(
            Some(first),
            "div",
            Style::default(),
            Some("width: 50px; height: 100px"),
        );
        let _second_child = doc.append_element(
            Some(second),
            "div",
            Style::default(),
            Some("width: 50px; height: 100px"),
        );
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade");
        crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new())
            .expect("layout");

        let first_layout = doc.nodes[first].unrounded_layout;
        let second_layout = doc.nodes[second].unrounded_layout;
        let table_layout = doc.nodes[table].unrounded_layout;
        assert!(
            (first_layout.location.x - second_layout.location.x).abs() < 0.5,
            "vertical columns share the block-axis origin: {:?} vs {:?}", // cov:ignore: assertion diagnostic is evaluated only on failure.
            first_layout.location,
            second_layout.location
        );
        assert!(
            second_layout.location.y - first_layout.location.y >= 99.5,
            "vertical columns advance along the inline axis: {:?} vs {:?}", // cov:ignore: assertion diagnostic is evaluated only on failure.
            first_layout.location,
            second_layout.location
        );
        assert!(
            table_layout.size.height >= first_layout.size.height + second_layout.size.height - 0.5,
            "vertical table height covers its inline tracks: {:?} vs {:?}", // cov:ignore: assertion diagnostic is evaluated only on failure.
            table_layout.size,
            (first_layout.size, second_layout.size) // cov:ignore: assertion diagnostic is evaluated only on failure.
        );
    }

    #[test]
    fn table_colspan_wide_equals_sum_of_spanned_columns() {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let table = doc.append_element(
            Some(body),
            "table",
            Style::default(),
            Some("display: table"),
        );
        let tr1 = doc.append_element(
            Some(table),
            "tr",
            Style::default(),
            Some("display: table-row"),
        );
        let td_wide = doc.append_element(
            Some(tr1),
            "td",
            Style::default(),
            Some("display: table-cell"),
        );
        doc.set_element_attributes(td_wide, vec![("colspan".into(), "2".into())]);
        let _t = doc.append_text(td_wide, "wide");
        let tr2 = doc.append_element(
            Some(table),
            "tr",
            Style::default(),
            Some("display: table-row"),
        );
        let td_a = doc.append_element(
            Some(tr2),
            "td",
            Style::default(),
            Some("display: table-cell"),
        );
        let td_b = doc.append_element(
            Some(tr2),
            "td",
            Style::default(),
            Some("display: table-cell"),
        );
        let _ta = doc.append_text(td_a, "a");
        let _tb = doc.append_text(td_b, "b");
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).unwrap();
        crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).unwrap();
        let wide = doc.nodes[td_wide].unrounded_layout;
        let a = doc.nodes[td_a].unrounded_layout;
        let b = doc.nodes[td_b].unrounded_layout;
        assert!(
            (wide.size.width - (a.size.width + b.size.width)).abs() < 2.0,
            "wide {} vs a+b {} {}",
            wide.size.width,
            a.size.width + b.size.width,
            (wide.size.width - (a.size.width + b.size.width)).abs()
        );
    }

    #[test]
    fn table_rowspan_distributes_height() {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let table = doc.append_element(
            Some(body),
            "table",
            Style::default(),
            Some("display: table"),
        );
        let tr1 = doc.append_element(
            Some(table),
            "tr",
            Style::default(),
            Some("display: table-row"),
        );
        let td_span = doc.append_element(
            Some(tr1),
            "td",
            Style::default(),
            Some("display: table-cell"),
        );
        doc.set_element_attributes(td_span, vec![("rowspan".into(), "2".into())]);
        let _t = doc.append_text(td_span, "span");
        let td1 = doc.append_element(
            Some(tr1),
            "td",
            Style::default(),
            Some("display: table-cell"),
        );
        let _t1 = doc.append_text(td1, "a");
        let tr2 = doc.append_element(
            Some(table),
            "tr",
            Style::default(),
            Some("display: table-row"),
        );
        let td2 = doc.append_element(
            Some(tr2),
            "td",
            Style::default(),
            Some("display: table-cell"),
        );
        let _t2 = doc.append_text(td2, "b");
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).unwrap();
        crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).unwrap();
        // rowspan cell should span at least as tall as two rows combined
        let span = doc.nodes[td_span].unrounded_layout;
        let a = doc.nodes[td1].unrounded_layout;
        let b = doc.nodes[td2].unrounded_layout;
        assert!(
            span.size.height >= a.size.height - 0.5,
            "rowspan height {} should >= single row {}",
            span.size.height,
            a.size.height
        );
        assert!(
            (span.size.height - (a.size.height + b.size.height)).abs() < 5.0
                || span.size.height >= b.size.height,
            "rowspan approx"
        );
    }

    #[test]
    fn nested_table_depth_cap_does_not_panic() {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let mut parent = body;
        for _ in 0..10 {
            let table = doc.append_element(
                Some(parent),
                "table",
                Style::default(),
                Some("display: table"),
            );
            let tr = doc.append_element(
                Some(table),
                "tr",
                Style::default(),
                Some("display: table-row"),
            );
            let td = doc.append_element(
                Some(tr),
                "td",
                Style::default(),
                Some("display: table-cell"),
            );
            let _t = doc.append_text(td, "x");
            parent = td;
        }
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).unwrap();
        let res = crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new());
        assert!(res.is_ok(), "nested table should not panic");
    }

    #[test]
    fn table_with_tbody_anonymization() {
        // Ensure html5ever tbody auto-insertion not required: manually use tbody group
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let table = doc.append_element(
            Some(body),
            "table",
            Style::default(),
            Some("display: table"),
        );
        let tbody = doc.append_element(
            Some(table),
            "tbody",
            Style::default(),
            Some("display: table-row-group"),
        );
        let tr = doc.append_element(
            Some(tbody),
            "tr",
            Style::default(),
            Some("display: table-row"),
        );
        let td = doc.append_element(
            Some(tr),
            "td",
            Style::default(),
            Some("display: table-cell"),
        );
        let _t = doc.append_text(td, "inside tbody");
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).unwrap();
        crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).unwrap();
        let td_l = doc.nodes[td].unrounded_layout;
        assert!(
            td_l.size.width > 1.0 && td_l.size.height > 1.0,
            "tbody wrapped cell should layout"
        );
    }

    #[test]
    fn table_parse_path_via_raikiri_html_uses_ua_display() {
        // Simulate parse path UA injection: inject minimal UA CSS manually.
        use raikiri_style::Origin;
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        // No inline display: rely on UA
        let table = doc.append_element(Some(body), "table", Style::default(), None::<&str>);
        let tr = doc.append_element(Some(table), "tr", Style::default(), None::<&str>);
        let td1 = doc.append_element(Some(tr), "td", Style::default(), None::<&str>);
        let td2 = doc.append_element(Some(tr), "td", Style::default(), None::<&str>);
        let _t1 = doc.append_text(td1, "hi");
        let _t2 = doc.append_text(td2, "bye");
        doc.mark_in_document_flags();
        let mut rules = build_rule_tree(&doc);
        rules.add_stylesheet(raikiri_html::ua::MINIMAL_UA_CSS, Origin::UserAgent);
        let cr = cascade(&doc, &rules).unwrap();
        let table_idx = doc
            .nodes
            .iter()
            .position(|n| n.tag_name() == Some("table"))
            .expect("table");
        assert_eq!(
            cr.computed[table_idx].display,
            raikiri_style::property::DisplayValue::Table,
            "UA should set table display"
        );
        crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).unwrap();
        let tds: Vec<usize> = doc
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.tag_name() == Some("td"))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(tds.len(), 2);
        let l0 = doc.nodes[tds[0]].unrounded_layout;
        let l1 = doc.nodes[tds[1]].unrounded_layout;
        assert!(
            (l1.location.x - l0.location.x).abs() > 1.0,
            "parsed table columns must separate"
        );
    }

    #[test]
    fn fixed_layout_honors_first_row_widths_and_ignores_content() {
        // CSS 2.1 §17.5.2.1: table 400px, first-row cell 100px fixed →
        // columns 100 / 300 regardless of the long content in column 2.
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let table = doc.append_element(
            Some(body),
            "table",
            Style::default(),
            Some("display: table; table-layout: fixed; width: 400px"),
        );
        let tr = doc.append_element(
            Some(table),
            "tr",
            Style::default(),
            Some("display: table-row"),
        );
        let td1 = doc.append_element(
            Some(tr),
            "td",
            Style::default(),
            Some("display: table-cell; width: 100px"),
        );
        let td2 = doc.append_element(
            Some(tr),
            "td",
            Style::default(),
            Some("display: table-cell"),
        );
        let _t1 = doc.append_text(td1, "x");
        let _t2 = doc.append_text(
            td2,
            "a very long content string that must not widen its column under fixed layout",
        );
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).unwrap();
        let table_idx = doc
            .nodes
            .iter()
            .position(|n| n.tag_name() == Some("table"))
            .unwrap();
        assert_eq!(
            cr.computed[table_idx].table_layout,
            raikiri_style::property::TableLayoutValue::Fixed
        );
        crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).unwrap();
        let l0 = doc.nodes[td1].unrounded_layout;
        let l1 = doc.nodes[td2].unrounded_layout;
        assert!(
            (l0.size.width - 100.0).abs() < 2.0,
            "fixed first column should be 100px, got {}",
            l0.size.width
        );
        assert!(
            (l1.size.width - 300.0).abs() < 2.0,
            "fixed second column should take the 300px remainder, got {}",
            l1.size.width
        );
    }

    #[test]
    fn fixed_layout_ignores_non_first_row_widths() {
        // CSS 2.1 §17.5.2.1: only the first row sets widths — a 300px width
        // on a second-row cell must not move the columns off 200/200.
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let table = doc.append_element(
            Some(body),
            "table",
            Style::default(),
            Some("display: table; table-layout: fixed; width: 400px"),
        );
        let tr1 = doc.append_element(
            Some(table),
            "tr",
            Style::default(),
            Some("display: table-row"),
        );
        let td1 = doc.append_element(
            Some(tr1),
            "td",
            Style::default(),
            Some("display: table-cell"),
        );
        let td2 = doc.append_element(
            Some(tr1),
            "td",
            Style::default(),
            Some("display: table-cell"),
        );
        let _a = doc.append_text(td1, "a");
        let _b = doc.append_text(td2, "b");
        let tr2 = doc.append_element(
            Some(table),
            "tr",
            Style::default(),
            Some("display: table-row"),
        );
        let td3 = doc.append_element(
            Some(tr2),
            "td",
            Style::default(),
            Some("display: table-cell; width: 300px"),
        );
        let td4 = doc.append_element(
            Some(tr2),
            "td",
            Style::default(),
            Some("display: table-cell"),
        );
        let _c = doc.append_text(td3, "c");
        let _d = doc.append_text(td4, "d");
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).unwrap();
        crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).unwrap();
        let l0 = doc.nodes[td1].unrounded_layout;
        let l1 = doc.nodes[td2].unrounded_layout;
        assert!(
            (l0.size.width - 200.0).abs() < 2.0 && (l1.size.width - 200.0).abs() < 2.0,
            "unfixed columns should split 400px equally, got {} and {}",
            l0.size.width,
            l1.size.width
        );
    }

    #[test]
    fn collapse_overlaps_adjoining_borders_by_pairwise_min() {
        // CSS 2.1 §17.6.2 (simplified max-width resolution): two 4px-bordered
        // cells collapse the interior line to max(4, 4) = 4 by overlapping
        // the boxes by min(4, 4) = 4.
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let table = doc.append_element(
            Some(body),
            "table",
            Style::default(),
            Some("display: table; border-collapse: collapse"),
        );
        let tr = doc.append_element(
            Some(table),
            "tr",
            Style::default(),
            Some("display: table-row"),
        );
        let td1 = doc.append_element(
            Some(tr),
            "td",
            Style::default(),
            Some("display: table-cell; border: 4px solid black"),
        );
        let td2 = doc.append_element(
            Some(tr),
            "td",
            Style::default(),
            Some("display: table-cell; border: 4px solid black"),
        );
        let _t1 = doc.append_text(td1, "a");
        let _t2 = doc.append_text(td2, "b");
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).unwrap();
        let table_idx = doc
            .nodes
            .iter()
            .position(|n| n.tag_name() == Some("table"))
            .unwrap();
        assert_eq!(
            cr.computed[table_idx].border_collapse,
            raikiri_style::property::BorderCollapseValue::Collapse
        );
        crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).unwrap();

        let l0 = doc.nodes[td1].unrounded_layout;
        let l1 = doc.nodes[td2].unrounded_layout;
        let overlap = (l0.location.x + l0.size.width) - l1.location.x;
        assert!(
            (overlap - 4.0).abs() < 1.0,
            "collapsed interior line should overlap by 4px, got {overlap}"
        );
        let tl = doc.nodes[table_idx].unrounded_layout;
        // Right-flush invariant: the last cell's far edge plus the 4px
        // collapsed outer border (no padding, no table border) meets the
        // table's right edge — the interior overlap must not leave slack.
        assert!(
            (tl.size.width - (l1.location.x + l1.size.width + 4.0)).abs() < 2.0,
            "table right edge should flush with last cell + outer collapsed border, got {}",
            tl.size.width
        );
    }

    #[test]
    fn separate_cells_abut_without_overlap() {
        // Counterpart check: without `border-collapse: collapse` the same
        // bordered cells abut exactly (no absorbed line).
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let table = doc.append_element(
            Some(body),
            "table",
            Style::default(),
            Some("display: table"),
        );
        let tr = doc.append_element(
            Some(table),
            "tr",
            Style::default(),
            Some("display: table-row"),
        );
        let td1 = doc.append_element(
            Some(tr),
            "td",
            Style::default(),
            Some("display: table-cell; border: 4px solid black"),
        );
        let td2 = doc.append_element(
            Some(tr),
            "td",
            Style::default(),
            Some("display: table-cell; border: 4px solid black"),
        );
        let _t1 = doc.append_text(td1, "a");
        let _t2 = doc.append_text(td2, "b");
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).unwrap();
        crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).unwrap();
        let l0 = doc.nodes[td1].unrounded_layout;
        let l1 = doc.nodes[td2].unrounded_layout;
        let overlap = (l0.location.x + l0.size.width) - l1.location.x;
        assert!(
            overlap.abs() < 1.0,
            "separate cells should abut exactly, got overlap {overlap}"
        );
    }

    #[test]
    fn table_min_size_respects_box_sizing() {
        // WPT css/css-tables/min-max-size-table-content-box (CSS Sizing 3
        // §3.3, csswg-drafts#5336 / Mozilla bug 1651530): with
        // `box-sizing: content-box`, min-width/min-height apply to the
        // content box — a 50px min with 3px border + 5px padding (16 total
        // insets) grows the content to 50 (outer 66). `border-box` keeps
        // the outer clamp (outer 50, content 34).
        for (box_sizing, expect_outer, expect_cell) in
            [("content-box", 66.0, 50.0), ("border-box", 50.0, 34.0)]
        {
            let mut doc = Document::new();
            let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
            let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
            let table_style = format!(
                "display: table; box-sizing: {box_sizing}; border: 3px solid black; \
                 padding: 5px; min-width: 50px; min-height: 50px"
            );
            let table = doc.append_element(
                Some(body),
                "table",
                Style::default(),
                Some(table_style.as_str()),
            );
            let tr = doc.append_element(
                Some(table),
                "tr",
                Style::default(),
                Some("display: table-row"),
            );
            let td = doc.append_element(
                Some(tr),
                "td",
                Style::default(),
                Some("display: table-cell"),
            );
            doc.mark_in_document_flags();
            let rules = build_rule_tree(&doc);
            let cr = cascade(&doc, &rules).unwrap();
            crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new())
                .unwrap();
            let tl = doc.nodes[table].unrounded_layout;
            let cl = doc.nodes[td].unrounded_layout;
            assert!(
                (tl.size.width - expect_outer).abs() < 1.5,
                "{box_sizing}: table outer width should be {expect_outer}, got {}",
                tl.size.width
            );
            assert!(
                (tl.size.height - expect_outer).abs() < 1.5,
                "{box_sizing}: table outer height should be {expect_outer}, got {}",
                tl.size.height
            );
            assert!(
                (cl.size.width - expect_cell).abs() < 1.5,
                "{box_sizing}: cell width should be {expect_cell}, got {}",
                cl.size.width
            );
            assert!(
                (cl.size.height - expect_cell).abs() < 1.5,
                "{box_sizing}: cell height should be {expect_cell}, got {}",
                cl.size.height
            );
        }
    }

    #[test]
    fn distribute_columns_preserves_authored_tracks_for_spanning_excess() {
        let widths = super::distribute_columns_with_authored(
            110.0,
            &[5.0, 95.0, 0.0, 5.0],
            &[5.0, 95.0, 0.0, 5.0],
            &[0.0, 0.0, 0.0, 0.0],
            &[true, false, false, true],
        );
        assert!((widths[0] - 5.0).abs() < 0.01);
        assert!((widths[3] - 5.0).abs() < 0.01);
        assert!((widths.iter().sum::<f32>() - 110.0).abs() < 0.01);
    }

    #[test]
    fn distribute_columns_falls_back_to_all_tracks_when_all_are_authored() {
        let widths = super::distribute_columns_with_authored(
            12.0,
            &[5.0, 5.0],
            &[5.0, 5.0],
            &[0.0, 0.0],
            &[true, true],
        );
        assert_eq!(widths, [6.0, 6.0]);
    }

    #[test]
    fn distribute_columns_covers_growth_and_shrink_phases() {
        let grown =
            super::distribute_columns_with_authored(10.0, &[1.0], &[10.0], &[0.0], &[false]);
        assert_eq!(grown, [10.0]);

        let percentage =
            super::distribute_columns_with_authored(10.0, &[0.0], &[0.0], &[0.5], &[false]);
        assert_eq!(percentage, [10.0]);

        let shrunk = super::distribute_columns_with_authored(
            4.0,
            &[0.0, 5.0],
            &[10.0, 5.0],
            &[1.0, 0.0],
            &[false, false],
        );
        assert_eq!(shrunk, [0.0, 5.0]);
    }

    #[test]
    fn table_colspan_percent_distribution_uses_definite_table_width() {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let table = doc.append_element(
            Some(body),
            "table",
            Style::default(),
            Some("display: table; width: 110px"),
        );
        let tr1 = doc.append_element(
            Some(table),
            "tr",
            Style::default(),
            Some("display: table-row"),
        );
        let wide = doc.append_element(
            Some(tr1),
            "td",
            Style::default(),
            Some("display: table-cell; width: 50%"),
        );
        doc.set_element_attributes(wide, vec![("colspan".into(), "2".into())]);
        let tr2 = doc.append_element(
            Some(table),
            "tr",
            Style::default(),
            Some("display: table-row"),
        );
        let left = doc.append_element(
            Some(tr2),
            "td",
            Style::default(),
            Some("display: table-cell; width: 5px"),
        );
        let right = doc.append_element(
            Some(tr2),
            "td",
            Style::default(),
            Some("display: table-cell; width: 5px"),
        );
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).unwrap();
        crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).unwrap();
        let wide_layout = doc.nodes[wide].unrounded_layout;
        let left_layout = doc.nodes[left].unrounded_layout;
        let right_layout = doc.nodes[right].unrounded_layout;
        assert!((wide_layout.size.width - 110.0).abs() < 1.0);
        assert!((left_layout.size.width - 55.0).abs() < 1.0);
        assert!((right_layout.size.width - 55.0).abs() < 1.0);
    }
}
