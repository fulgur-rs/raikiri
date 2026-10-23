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
//! - Fixed layout reads cell widths/minimums from the first row and authored
//!   column constraints. An indefinite table width falls back to auto.
//! - Collapse resolves border conflicts between adjacent cells using style,
//!   width, and top/left tie-breaking before intrinsic track sizing. Row,
//!   row-group, column, and column-group candidates and half-border centering
//!   remain out of scope.
//! - The separate model applies horizontal and vertical `border-spacing`
//!   between cells and at the table edges; collapse ignores those gaps.
//! - Only the first caption receives native table placement; multiple caption
//!   boxes remain tracked by `raikiri-spike-0vv.121`.
//! - Nested tables are depth-capped fail-closed (`MAX_TABLE_NESTING`).

use raikiri_style::ComputedBorder;
use raikiri_style::property::{
    BorderCollapseValue, BorderStyle, CaptionSideValue, DisplayValue, PositionValue, Sides,
    TableLayoutValue, VerticalAlign, WritingMode,
};
use taffy::Rect as StyleRect; // taffy-boundary-lint:ignore: Rect<LengthPercentage> is style input
use taffy::style::{CompactLength, Dimension};
use taffy::{BlockItemStyle, CoreStyle, ExpandedDimension, LengthPercentage, Position};

use crate::document::Document;
use crate::document::ancestors::{AncestorCursor, ParentRelation};
use crate::layout::context::LayoutSession;
use crate::layout::dispatch::content_limits::ContentLimits;
use crate::layout::protocol::{
    AvailableSpace, AxisStaticEdge, AxisStaticPosition, LayoutInput, LayoutOutput, NodeId,
    OofCandidate, OofCandidates, OofPosition, OofPositioningArea, RequestedAxis, RunMode,
    SizingMode,
};
use crate::layout::style_views::ReadRole;
use crate::layout::taffy_adapter::{IntoEdges, MaybeResolve, ResolveOrZero};
use raikiri_layout_geometry::{BoxLayout, Edges, Line, Point, Size};
use raikiri_layout_style::LayoutStyleView;

pub(crate) mod anonymous;
mod border_scopes;
mod border_state;
pub(super) mod cache;
use border_state::BorderState;
mod cell_index;
pub(crate) mod headers;
pub(crate) mod row_moves;
mod rowspan;
use cell_index::CellIndex;

// ---------------------------------------------------------------------------
// Depth cap — fail-closed for nested tables.
// ---------------------------------------------------------------------------

#[cfg(test)]
thread_local! {
    pub(super) static GRID_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static BORDER_RESOLUTIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

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

#[derive(Debug, Clone)]
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
    specified_min_width: Dimension,
    resolved: Option<BoxLayout>,
    natural_height: f32,
    baseline: f32,
    /// The row-height pass's layout, kept when the final layout at the row
    /// height would only repeat it (see [`row_pass_is_final`]).
    row_pass: Option<RowPassLayout>,
}

/// What the final cell placement needs from a reusable row-height pass.
#[derive(Debug, Clone, Copy)]
struct RowPassLayout {
    width: f32,
    scrollable_overflow_rect: Edges<f32>,
}

// ---------------------------------------------------------------------------
// Public entry — called from `dispatch.rs` when `Node::display == Table`.
// ---------------------------------------------------------------------------

impl LayoutSession<'_> {
    /// The style of a table box, row, column, caption, cell or cell child, on
    /// the physical axes the table algorithm works in.
    ///
    /// This reads the box's own values with the scoped overrides applied and
    /// never substitutes the transposed view that a vertical block frame gives
    /// block content. The table algorithm positions these boxes itself, so the
    /// neutral core query is the right role for every one of them.
    fn table_style(&self, idx: usize) -> LayoutStyleView<'_> {
        self.query_style(idx, ReadRole::CoreContainer)
    }
}

/// The authored width and minimum width of a cell: what column sizing reads.
fn cell_specified_widths(doc: &LayoutSession<'_>, cell: usize) -> (Dimension, Dimension) {
    let style = doc.table_style(cell);
    (style.size().width, style.min_size().width.into())
}

fn table_writing_mode(doc: &Document, table_idx: usize) -> WritingMode {
    doc.nodes[doc.ifc_source_owner(table_idx)].used_writing_mode
}

fn caption_minimum_inline_size(doc: &mut LayoutSession<'_>, table: usize, vertical: bool) -> f32 {
    let Some(caption) = doc.nodes[table]
        .children
        .iter()
        .copied()
        .find(|&id| doc.nodes[id].display() == DisplayValue::TableCaption)
        .filter(|&id| doc.nodes[id].taffy_position() != taffy::Position::Absolute)
    else {
        return 0.0;
    };
    let probe = |available_space| LayoutInput {
        run_mode: RunMode::ComputeSize,
        sizing_mode: SizingMode::InherentSize,
        axis: if vertical {
            RequestedAxis::Vertical
        } else {
            RequestedAxis::Horizontal
        },
        known_dimensions: Size::NONE,
        parent_size: Size::NONE,
        available_space: Size {
            width: if vertical {
                AvailableSpace::MaxContent
            } else {
                available_space
            },
            height: if vertical {
                available_space
            } else {
                AvailableSpace::MaxContent
            },
        },
        known_dimensions_are_definite: Size {
            width: false,
            height: false,
        },
        vertical_margins_are_collapsible: Line::FALSE,
    };
    let min_inputs = probe(AvailableSpace::MinContent);
    let max_inputs = probe(AvailableSpace::MaxContent);
    let minimum = doc.compute_child_layout(NodeId::from(caption), min_inputs);
    let maximum = doc.compute_child_layout(NodeId::from(caption), max_inputs);
    let limits = doc.nodes[caption].content_limits;
    let min_content = if vertical {
        minimum.size.height
    } else {
        minimum.size.width
    };
    let max_content = if vertical {
        maximum.size.height
    } else {
        maximum.size.width
    };
    let (min_limit, max_limit, available) = if vertical {
        (
            limits.min.height,
            limits.max.height,
            min_inputs.available_space.height,
        )
    } else {
        (
            limits.min.width,
            limits.max.width,
            min_inputs.available_space.width,
        )
    };
    let context = IntrinsicSizeContext {
        min_content,
        max_content,
        inputs: &min_inputs,
        available_space: available,
        horizontal: !vertical,
    };
    let (minimum, _) = constrain_intrinsic_sizes(doc, caption, min_limit, max_limit, context);
    let margins = doc.table_style(caption).margin();
    let (start, end) = if vertical {
        (margins.top, margins.bottom)
    } else {
        (margins.left, margins.right)
    };
    minimum
        + super::used_style_length_percentage_auto(start, 0.0).unwrap_or(0.0)
        + super::used_style_length_percentage_auto(end, 0.0).unwrap_or(0.0)
}

fn layout_table_caption(
    doc: &mut LayoutSession<'_>,
    table_idx: usize,
    parent_width: f32,
    parent_height: Option<f32>,
) -> Option<(Size<f32>, OofCandidates)> {
    let caption_id = doc.nodes[table_idx]
        .children
        .iter()
        .copied()
        .find(|&child| doc.nodes[child].display() == DisplayValue::TableCaption)?;
    let (size, margin, padding, border) = {
        let style = doc.table_style(caption_id);
        (
            style.size(),
            style.margin(),
            style.padding(),
            style.border(),
        )
    };
    // Let block layout apply authored sizes in their box-sizing domain.
    let vertical = table_writing_mode(doc, table_idx).is_vertical();
    let inline_basis = if vertical {
        parent_height.unwrap_or(0.0)
    } else {
        parent_width
    };
    let known_width = if !vertical && size.width.is_auto() {
        let left =
            super::used_style_length_percentage_auto(margin.left, parent_width).unwrap_or(0.0);
        let right =
            super::used_style_length_percentage_auto(margin.right, parent_width).unwrap_or(0.0);
        Some((parent_width - left - right).max(0.0))
    } else {
        None
    };
    let known_height = if vertical && size.height.is_auto() {
        parent_height.map(|height| {
            let top =
                super::used_style_length_percentage_auto(margin.top, inline_basis).unwrap_or(0.0);
            let bottom = super::used_style_length_percentage_auto(margin.bottom, inline_basis)
                .unwrap_or(0.0);
            (height - top - bottom).max(0.0)
        })
    } else {
        None
    };
    let output = doc.compute_child_layout(
        NodeId::from(caption_id),
        LayoutInput {
            run_mode: RunMode::PerformLayout,
            sizing_mode: SizingMode::InherentSize,
            axis: RequestedAxis::Both,
            known_dimensions: Size {
                width: known_width,
                height: known_height,
            },
            parent_size: Size {
                width: Some(parent_width),
                height: parent_height,
            },
            available_space: Size {
                width: AvailableSpace::Definite(parent_width.max(0.0)),
                height: parent_height
                    .map(AvailableSpace::Definite)
                    .unwrap_or(AvailableSpace::MaxContent),
            },
            known_dimensions_are_definite: Size {
                width: known_width.is_some(),
                height: known_height.is_some(),
            },
            vertical_margins_are_collapsible: Line::FALSE,
        },
    );
    let mut margin_left =
        super::used_style_length_percentage_auto(margin.left, inline_basis).unwrap_or(0.0);
    let mut margin_top =
        super::used_style_length_percentage_auto(margin.top, inline_basis).unwrap_or(0.0);
    let mut margin_right =
        super::used_style_length_percentage_auto(margin.right, inline_basis).unwrap_or(0.0);
    let mut margin_bottom =
        super::used_style_length_percentage_auto(margin.bottom, inline_basis).unwrap_or(0.0);
    let (start, end, start_auto, end_auto, available) = if vertical {
        (
            &mut margin_top,
            &mut margin_bottom,
            margin.top.is_auto(),
            margin.bottom.is_auto(),
            parent_height.unwrap_or(output.size.height) - output.size.height,
        )
    } else {
        (
            &mut margin_left,
            &mut margin_right,
            margin.left.is_auto(),
            margin.right.is_auto(),
            parent_width - output.size.width,
        )
    };
    let remaining = (available - *start - *end).max(0.0);
    match (start_auto, end_auto) {
        (true, true) => {
            *start = remaining / 2.0;
            *end = remaining / 2.0;
        }
        (true, false) => *start = remaining,
        (false, true) => *end = remaining,
        (false, false) => {}
    }
    let layout = BoxLayout {
        order: 0,
        location: Point {
            x: margin_left,
            y: margin_top,
        },
        size: output.size,
        scrollable_overflow_rect: output.scrollable_overflow_rect,
        scrollbar_size: Size::ZERO,
        padding: padding.resolve_or_zero(Some(parent_width), crate::layout::dispatch::resolve_calc),
        border: border.resolve_or_zero(Some(parent_width), crate::layout::dispatch::resolve_calc),
        margin: Edges {
            left: margin_left,
            right: margin_right,
            top: margin_top,
            bottom: margin_bottom,
        },
    };
    doc.store_unrounded_layout(caption_id, &layout);
    Some((
        Size {
            width: output.size.width + margin_left + margin_right,
            height: output.size.height + margin_top + margin_bottom,
        },
        output.oof_candidates,
    ))
}

fn translate_table_caption_candidates(
    doc: &Document,
    table_idx: usize,
    candidates: &mut OofCandidates,
) {
    if candidates.is_empty() {
        return;
    }
    if let Some(caption) = doc.nodes[table_idx]
        .children
        .iter()
        .copied()
        .find(|&child| doc.nodes[child].display() == DisplayValue::TableCaption)
    {
        candidates.translate(doc.nodes[caption].unrounded_layout.location);
    }
}

fn position_table_caption(
    doc: &mut LayoutSession<'_>,
    table_idx: usize,
    grid_size: Size<f32>,
    caption_size: Option<Size<f32>>,
    run_mode: RunMode,
) -> (Size<f32>, Point<f32>) {
    let caption_id = doc.nodes[table_idx]
        .children
        .iter()
        .copied()
        .find(|&child| doc.nodes[child].display() == DisplayValue::TableCaption);
    // Keep the legacy positioned caption layout and ink, but exclude it from flow.
    let in_flow =
        caption_id.is_some_and(|id| doc.nodes[id].taffy_position() != taffy::Position::Absolute);
    let caption_size = if in_flow {
        caption_size.unwrap_or(Size::ZERO)
    } else {
        Size::ZERO
    };
    let mode = table_writing_mode(doc, table_idx);
    let vertical = mode.is_vertical();
    let before =
        caption_id.is_some_and(|id| doc.nodes[id].caption_side != CaptionSideValue::Bottom);
    let right_to_left = matches!(mode, WritingMode::VerticalRl | WritingMode::SidewaysRl);
    let grid_origin = if vertical {
        Point {
            x: if before != right_to_left {
                caption_size.width
            } else {
                0.0
            },
            y: 0.0,
        }
    } else {
        Point {
            x: 0.0,
            y: if before { caption_size.height } else { 0.0 },
        }
    };
    if let Some(id) = caption_id.filter(|_| in_flow) {
        let mut layout = doc.nodes[id].unrounded_layout;
        if vertical {
            layout.location.x += if before == right_to_left {
                grid_size.width
            } else {
                0.0
            };
        } else {
            layout.location.y += if before { 0.0 } else { grid_size.height };
        }
        doc.store_unrounded_layout(id, &layout);
    }
    if run_mode == RunMode::PerformLayout && in_flow {
        doc.store_table_grid_box(table_idx, grid_origin, grid_size);
    }
    (
        if vertical {
            Size {
                width: grid_size.width + caption_size.width,
                height: grid_size.height.max(caption_size.height),
            }
        } else {
            Size {
                width: grid_size.width,
                height: grid_size.height + caption_size.height,
            }
        },
        grid_origin,
    )
}

pub fn compute_table_layout(
    doc: &mut LayoutSession<'_>,
    table_id: NodeId,
    inputs: LayoutInput,
) -> LayoutOutput {
    match compute_table_layout_checked(doc, table_id, inputs) {
        Ok(output) => output,
        Err(error) => {
            doc.context.table_layout_error = Some(error.to_string());
            LayoutOutput::from_outer_size(Size::ZERO)
        }
    }
}

fn compute_table_layout_checked(
    doc: &mut LayoutSession<'_>,
    table_id: NodeId,
    inputs: LayoutInput,
) -> Result<LayoutOutput, raikiri_traits::LayoutError> {
    // Depth cap: fail-closed as block fallback.
    let _depth_guard = match depth_enter() {
        Some(_prev) => DepthGuard,
        None => {
            // Exceeded nesting — render as block with available width, zero height fallback.
            // We still need to run block layout for children to avoid leaving them uninitialized?
            // For fail-closed, return outer size based on known_dimensions or zero.
            let w = inputs.known_dimensions.width.unwrap_or(0.0);
            let h = inputs.known_dimensions.height.unwrap_or(0.0);
            return Ok(LayoutOutput::from_outer_size(Size {
                width: w,
                height: h,
            }));
        }
    };

    let table_idx = usize::from(table_id);
    if inputs.run_mode == RunMode::PerformLayout {
        doc.nodes[table_idx].table_grid_box = None;
        doc.nodes[table_idx].table_first_baseline = None;
    }
    let abspos_table = doc.nodes[table_idx].taffy_position() == taffy::Position::Absolute;
    let parent_is_flex_or_grid = doc
        .taffy_parent_of(table_idx)
        .is_some_and(|parent| doc.nodes[parent].formatting_context.is_flex_or_grid());
    let grid = cache::table_grid(doc, table_idx)?;
    let table_layout = doc.nodes[table_idx].table_layout;
    let collapse = doc.nodes[table_idx].border_collapse == BorderCollapseValue::Collapse;
    let vertical_writing = table_writing_mode(doc, table_idx).is_vertical();
    let (table_padding, table_border_widths) = {
        let table_style = doc.table_style(table_idx);
        (table_style.padding(), table_style.border())
    };
    let pad = resolve_table_rect(&table_padding, inputs.parent_size);
    let table_border = resolve_table_rect(&table_border_widths, inputs.parent_size);
    let prepared = cache::prepared_borders(
        doc,
        table_idx,
        &grid,
        inputs.parent_size.width,
        table_border,
    );
    let layout = |doc: &mut LayoutSession<'_>| {
        let mut grid = grid;
        let mut oof_candidates = OofCandidates::new();
        let cache::PreparedBorders {
            collapsed_rows,
            collapsed_cols,
            columns_in_place,
            collapsed,
            ..
        } = prepared.as_ref();
        let columns_collapse = collapsed_cols.contains(&true);
        // CAPMIN constrains the grid's inline axis before its tracks are assigned;
        // growing only the wrapper would leave the cells and grid paint too small.
        let caption_inline_minimum = caption_minimum_inline_size(doc, table_idx, vertical_writing);
        let caption_minimum = if vertical_writing {
            0.0
        } else {
            caption_inline_minimum
        };

        // Known outer dimensions (from taffy's compute_root_layout known_dimensions or style.size)
        // The table's outer size caller may have imposed via `apply_page_content_box_to_body` / block layout.
        // For tables, style.size is already injected via bridge; inputs.known_dimensions carries it.
        // Also honour the node at table_idx's style.size if known_dimensions is None (similar to prototype).
        // taffy's Dimension::maybe_resolve not directly available; use helper below.
        let table_size = doc.table_style(table_idx).size();
        let effective_known = Size {
            width: inputs.known_dimensions.width.or(resolve_dimension(
                table_size.width,
                inputs.parent_size.width,
            )),
            height: inputs.known_dimensions.height.or(resolve_dimension(
                table_size.height,
                inputs.parent_size.height,
            )),
        };

        let border_spacing = if collapse {
            (0.0, 0.0)
        } else {
            (
                doc.nodes[table_idx].border_spacing.horizontal.0.max(0.0),
                doc.nodes[table_idx].border_spacing.vertical.0.max(0.0),
            )
        };

        let padding_border_size = Size {
            width: pad.left + pad.right + collapsed.outer_left + collapsed.outer_right,
            height: pad.top + pad.bottom + collapsed.outer_top + collapsed.outer_bottom,
        };

        if grid.n_cols == 0 || grid.rows.is_empty() {
            // Empty tables shrink-wrap like the main path below: a specified
            // width wins; otherwise the container width is only a cap over the
            // padding/border extents (never stretch-to-fill).
            let specified_w = resolve_dimension(table_size.width, inputs.parent_size.width);
            // A flex/grid parent supplies the table item's used main size after
            // flexing/tracking. It must override the table's percentage width
            // (which is only the item's hypothetical basis); otherwise a
            // shrinking table item snaps back to its pre-flex percentage width.
            let mut width = if parent_is_flex_or_grid {
                effective_known
                    .width
                    .or(specified_w)
                    .unwrap_or(padding_border_size.width)
            } else {
                specified_w.unwrap_or_else(|| {
                    let natural = padding_border_size.width;
                    match effective_known.width {
                        Some(container) => f32_max_compat(natural.min(container), 0.0),
                        None => natural,
                    }
                })
            };
            let specified_h = resolve_dimension(table_size.height, inputs.parent_size.height);
            let mut height = specified_h.unwrap_or_else(|| {
                effective_known
                    .height
                    .unwrap_or(0.0)
                    .max(padding_border_size.height)
            });
            if vertical_writing {
                height = height.max(caption_inline_minimum);
            }
            let (caption_size, mut caption_candidates) = layout_table_caption(
                doc,
                table_idx,
                if specified_w.is_some() || effective_known.width.is_some() {
                    width
                } else {
                    inputs.parent_size.width.unwrap_or(width).max(width)
                },
                if vertical_writing {
                    Some(height)
                } else {
                    inputs.parent_size.height
                },
            )
            .map(|(size, candidates)| (Some(size), candidates))
            .unwrap_or_else(|| (None, OofCandidates::new()));
            if !vertical_writing && let Some(size) = caption_size {
                width = width.max(size.width);
            }
            let (wrapper_size, _) = position_table_caption(
                doc,
                table_idx,
                Size { width, height },
                caption_size,
                inputs.run_mode,
            );
            translate_table_caption_candidates(doc, table_idx, &mut caption_candidates);
            return Ok(table_layout_output(
                doc,
                table_idx,
                wrapper_size,
                caption_candidates,
                inputs.run_mode,
            ));
        }

        // Overlap totals up front: tracks are sized in pre-overlap space
        // (origins net the overlaps out afterwards), so the avail/target the
        // distributors aim at must add the overlaps back — otherwise a stretched
        // table leaves an `overlap`-wide slack at its end edge. Separate model:
        // overlaps are zero.
        let overlap_w: f32 = collapsed.col_overlaps.iter().sum();
        let overlap_h: f32 = collapsed.row_overlaps.iter().sum();
        // Separated borders (CSS 2.2 §17.6.1): `border-spacing` sits between
        // adjoining cells and between the outer cells and the table's padding
        // edge, so n tracks take n + 1 gaps. The spacing belongs to the table's
        // content box, so it is part of the extents the tracks share but not of
        // `padding_border_size`. Collapse zeroes `border_spacing`.
        // A row whose overlapping cells all hide their empty boxes has zero
        // height and only one spacing side (CSS 2.2 section 17.6.1.1).
        let hidden_rows: Vec<bool> = hidden_empty_rows(doc, &grid)
            .into_iter()
            .zip(collapsed_rows)
            .map(|(hidden, collapsed)| hidden || *collapsed)
            .collect();
        let row_gaps: Vec<_> = hidden_rows
            .iter()
            .map(|hidden| if *hidden { 0.0 } else { border_spacing.1 })
            .collect();
        let shown_columns = collapsed_cols
            .iter()
            .filter(|collapsed| !**collapsed)
            .count();
        let spacing = Size {
            // With every column collapsed nothing remains to space apart.
            width: if shown_columns == 0 {
                0.0
            } else {
                border_spacing.0 * (shown_columns as f32 + 1.0)
            },
            // With every row collapsed nothing remains to space apart.
            height: if collapsed_rows.iter().all(|collapsed| *collapsed) {
                0.0
            } else {
                border_spacing.1
                    * (hidden_rows.iter().filter(|hidden| !**hidden).count() as f32 + 1.0)
            },
        };
        let distrib_insets = Size {
            width: padding_border_size.width - overlap_w + spacing.width,
            height: padding_border_size.height - overlap_h + spacing.height,
        };

        // Specified (non-auto) table width, resolved against the containing
        // block. Auto-width tables shrink-wrap their content (CSS 2.1 §17.5.2.2)
        // and must NOT stretch to fill the containing block: `effective_known`
        // carries the container-imposed width (800 for top-level blocks), which
        // is a cap for auto tables, the target only for specified widths.
        let specified_width = resolve_dimension(table_size.width, inputs.parent_size.width);
        // In fixed mode an unresolvable specified width (auto, or % of an
        // indefinite container) falls back to the auto algorithm below
        // (CSS 2.1 §17.5.2.1: a fixed-layout table with `width: auto` uses the
        // automatic layout algorithm; §17.5.2: such a table does not fill its
        // containing block).
        // Auto and fixed+auto columns use the specified width, or the used width
        // assigned by a flex/grid parent. Otherwise, with `known_dimensions.width = None` the
        // resolver takes its cap branch (preferred size capped by the definite
        // container, §17.5.2.2) instead of stretch-to-fill. A specified width
        // keeps the previous basis (`effective_known`, the taffy-resolved outer
        // width — subtracting insets recovers the content box).
        let wrapper_limits = doc.nodes[table_idx].content_limits;
        let (wrapper_minimum, wrapper_maximum, constrained_available_space) = if wrapper_limits
            .min
            .width
            .is_auto()
            && wrapper_limits.max.width.is_auto()
        {
            (None, None, inputs.available_space.width)
        } else {
            let intrinsic_grid = if columns_collapse {
                without_collapsed_columns(&grid, collapsed_cols, false)
            } else {
                grid.clone()
            };
            let intrinsic_inputs = LayoutInput {
                known_dimensions: Size {
                    width: None,
                    height: inputs.known_dimensions.height,
                },
                ..inputs
            };
            let measure_intrinsic_grid = |doc: &mut LayoutSession<'_>, available_space| {
                let probe = LayoutInput {
                    available_space: Size {
                        width: available_space,
                        height: intrinsic_inputs.available_space.height,
                    },
                    ..intrinsic_inputs
                };
                resolve_column_widths(
                    doc,
                    &intrinsic_grid,
                    probe,
                    Size {
                        width: distrib_insets.width,
                        height: 0.0,
                    },
                    border_spacing.0,
                )
                .iter()
                .sum::<f32>()
                    + distrib_insets.width
            };
            let grid_minimum = measured_in_place(doc, columns_in_place.as_ref(), |doc| {
                measure_intrinsic_grid(doc, AvailableSpace::MinContent)
            });
            let grid_maximum = measured_in_place(doc, columns_in_place.as_ref(), |doc| {
                measure_intrinsic_grid(doc, AvailableSpace::MaxContent)
            });
            let intrinsic_minimum = grid_minimum.max(caption_minimum);
            let intrinsic_maximum = grid_maximum.max(caption_minimum).max(intrinsic_minimum);
            let wrapper_context = IntrinsicSizeContext {
                min_content: intrinsic_minimum,
                max_content: intrinsic_maximum,
                inputs: &inputs,
                available_space: inputs.available_space.width,
                horizontal: true,
            };
            let wrapper_minimum =
                resolve_intrinsic_limit(doc, table_idx, wrapper_limits.min.width, wrapper_context)
                    .map(|minimum| minimum.max(intrinsic_minimum));
            let authored_wrapper_maximum =
                resolve_intrinsic_limit(doc, table_idx, wrapper_limits.max.width, wrapper_context);
            let wrapper_maximum = authored_wrapper_maximum
                .map(|maximum| maximum.min(intrinsic_maximum).max(intrinsic_minimum));
            let wrapper_maximum = if wrapper_minimum
                .zip(authored_wrapper_maximum)
                .is_some_and(|(minimum, maximum)| minimum > maximum)
            {
                None
            } else {
                wrapper_maximum
            };
            let constrained_available_space = match inputs.available_space.width {
                AvailableSpace::MinContent => {
                    let target = apply_wrapper_width_bounds(
                        intrinsic_minimum,
                        wrapper_minimum,
                        wrapper_maximum,
                    );
                    if target != intrinsic_minimum {
                        AvailableSpace::Definite(target)
                    } else {
                        inputs.available_space.width
                    }
                }
                AvailableSpace::MaxContent => {
                    let target = apply_wrapper_width_bounds(
                        intrinsic_maximum,
                        wrapper_minimum,
                        wrapper_maximum,
                    );
                    if target != intrinsic_maximum {
                        AvailableSpace::Definite(target)
                    } else {
                        inputs.available_space.width
                    }
                }
                AvailableSpace::Definite(room) => {
                    let target = apply_wrapper_width_bounds(room, wrapper_minimum, wrapper_maximum);
                    if target != room {
                        AvailableSpace::Definite(target)
                    } else {
                        inputs.available_space.width
                    }
                }
            };
            (
                wrapper_minimum,
                wrapper_maximum,
                constrained_available_space,
            )
        };
        let inputs_for_columns = LayoutInput {
            known_dimensions: Size {
                width: match (specified_width, parent_is_flex_or_grid) {
                    (Some(_), _) | (_, true) => effective_known.width.map(|width| {
                        apply_wrapper_width_bounds(
                            width.max(caption_minimum),
                            wrapper_minimum,
                            wrapper_maximum,
                        )
                    }),
                    (None, false) => None,
                },
                height: effective_known.height,
            },
            available_space: Size {
                width: constrained_available_space,
                height: inputs.available_space.height,
            },
            ..inputs
        };
        let pass = ColumnPass {
            table_layout,
            specified_width,
            caption_minimum,
            wrapper_minimum,
            wrapper_maximum,
            inputs: inputs_for_columns,
            border_spacing_x: border_spacing.0,
        };
        // A collapsed column leaves the table, but its cells still count toward
        // the row heights, which keep the widths the columns had before the
        // collapse ("col visibility:collapse doesn't change table height").
        let natural_insets_width = pad.left
            + pad.right
            + columns_in_place.as_ref().map_or_else(
                || collapsed.inline_border_width(),
                |columns| columns.border_width,
            )
            + border_spacing.0 * (grid.n_cols as f32 + 1.0);
        let natural_widths = columns_collapse.then(|| {
            measured_in_place(doc, columns_in_place.as_ref(), |doc| {
                resolve_table_columns(doc, &mut grid, &pass, natural_insets_width)
            })
        });
        let mut column_widths = if columns_collapse {
            let mut remaining = without_collapsed_columns(&grid, collapsed_cols, false);
            let mut widths =
                resolve_table_columns(doc, &mut remaining, &pass, distrib_insets.width).into_iter();
            collapsed_cols
                .iter()
                .map(|collapsed| {
                    if *collapsed {
                        0.0
                    } else {
                        widths.next().unwrap_or(0.0)
                    }
                })
                .collect()
        } else {
            resolve_table_columns(doc, &mut grid, &pass, distrib_insets.width)
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
        let (authored_min, authored_max, box_sizing) = {
            let st = doc.table_style(table_idx);
            (st.min_size(), st.max_size(), st.box_sizing())
        };
        let specified_height = resolve_dimension(table_size.height, inputs.parent_size.height);
        let min_w = resolve_dimension(authored_min.width.into(), inputs.parent_size.width);
        let max_w = resolve_dimension(authored_max.width.into(), inputs.parent_size.width);
        let min_h = resolve_dimension(authored_min.height.into(), inputs.parent_size.height);
        let max_h = resolve_dimension(authored_max.height.into(), inputs.parent_size.height);
        let is_content_box = box_sizing == taffy::BoxSizing::ContentBox;
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
        let minimum_table_width = [min_w_outer, wrapper_minimum, Some(caption_minimum)]
            .into_iter()
            .flatten()
            .reduce(f32::max);
        let grow_columns = |widths: &mut [f32], insets_width: f32, hidden: Option<&[bool]>| {
            if let Some(minimum) = minimum_table_width {
                distribute_extra(widths, hidden, f32_max_compat(minimum - insets_width, 0.0));
            }
        };
        grow_columns(
            &mut column_widths,
            distrib_insets.width,
            columns_collapse.then_some(&collapsed_cols[..]),
        );
        let natural_widths = natural_widths.map(|mut widths| {
            grow_columns(&mut widths, natural_insets_width, None);
            widths
        });

        // Row heights
        let height_widths = natural_widths.as_deref().unwrap_or(&column_widths);
        let mut row_heights = measured_in_place(doc, columns_in_place.as_ref(), |doc| {
            resolve_row_heights(doc, &mut grid, height_widths, border_spacing)
        })?;
        if let Some(columns) = columns_in_place {
            columns.measure_changed_cells(doc, &mut grid, height_widths, border_spacing)?;
        }
        for (height, hidden) in row_heights.iter_mut().zip(&hidden_rows) {
            if *hidden {
                *height = 0.0;
            }
        }
        // An authored definite table height also establishes the containing block
        // for percentage-sized children of a single-row cell. Fragmentation can
        // impose a definite height on a single-row table without doing so; keep
        // that free space in the wrapper rather than manufacturing a cell
        // fragment in that case.
        if effective_known.height.is_some() && (grid.rows.len() > 1 || specified_height.is_some()) {
            let known_h = effective_known.height.unwrap(); // cov:ignore: exercised by ignored exact table-fragmentation WPT.
            let target = f32_max_compat(known_h - distrib_insets.height, 0.0);
            distribute_extra(&mut row_heights, Some(&hidden_rows), target);
        }

        // Extra min-height grows rows (same path as a definite height);
        // max-height only clamps the box (content overflows visibly). The outer
        // domain keeps content-box correct: `min_outer - distrib_insets` =
        // `min_content + overlap`, the track sum that yields `min_content`.
        if let Some(mn) = min_h_outer {
            let target = f32_max_compat(mn - distrib_insets.height, 0.0);
            distribute_extra(&mut row_heights, Some(&hidden_rows), target);
        }
        if vertical_writing {
            // Vertical columns stack the same row track along the physical
            // inline axis. Share CAPMIN across those stacks after removing the
            // grid's own edges and the intervening inline spacing.
            let columns = grid.n_cols as f32;
            let edges = padding_border_size.height + border_spacing.1 * (columns + 1.0);
            let target = (caption_inline_minimum - edges).max(0.0) / columns;
            distribute_extra(&mut row_heights, Some(&hidden_rows), target);
        }

        // Content extents net of collapsed-line overlaps, plus the separated
        // spacing (one of the two is always zero).
        let content_width: f32 = column_widths.iter().sum::<f32>() - overlap_w + spacing.width;
        let content_height: f32 = row_heights.iter().sum::<f32>() - overlap_h + spacing.height;
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
        // Auto-width tables (auto layout, and fixed+auto fallback per CSS 2.1
        // §17.5.2.1) size to content (shrink-wrap, §17.5.2.2); only a specified
        // width keeps the fill basis.
        let table_width_basis = if abspos_table && specified_width.is_none() {
            // An auto-width absolutely positioned table uses the containing block's
            // available width for this table-layout path.  Taffy's shrink-to-fit
            // probe may report the intrinsic cell width as `known_dimensions.width`,
            // but CSS Tables 3 caps the available size at (and in this case fills)
            // the containing block; retaining the probe would leave a narrow strip
            // missing from the table background.
            inputs.parent_size.width.or(effective_known.width)
        } else if parent_is_flex_or_grid {
            // Flex/grid supplies a used item width even when the table's own
            // width is auto. Keep that width instead of shrink-wrapping again.
            effective_known.width
        } else {
            match specified_width {
                Some(_) => effective_known.width,
                None => None,
            }
        };
        // Vertical tables stack each column's inline track along the physical
        // height, so the physical spacing applies across that stack: the
        // vertical gap between and around the stacked tracks, the horizontal gap
        // on both sides of the single physical column.
        let vertical_content_height = row_heights.iter().sum::<f32>() * grid.n_cols as f32
            + border_spacing.1 * (grid.n_cols as f32 + 1.0);
        let vertical_content_width =
            column_widths.iter().sum::<f32>() - overlap_w + border_spacing.0 * 2.0;
        // CSS 2.1 §17.5.2: the used table width is at least the columns plus
        // the spacing and borders, even when the specified width is smaller.
        let min_table_width = content_width + padding_border_size.width;
        let final_size = Size {
            width: if vertical_writing {
                effective_known
                    .width
                    .unwrap_or(vertical_content_width + padding_border_size.width)
            } else {
                table_width_basis
                    .map(|w| {
                        let width = apply_wrapper_width_bounds(w, wrapper_minimum, wrapper_maximum);
                        clamp_min_max(
                            f32_max_compat(width, min_table_width),
                            min_w_outer,
                            max_w_outer,
                        )
                    })
                    .unwrap_or_else(|| {
                        clamp_min_max(
                            f32_max_compat(
                                apply_wrapper_width_bounds(
                                    content_width + padding_border_size.width,
                                    wrapper_minimum,
                                    wrapper_maximum,
                                ),
                                min_table_width,
                            ),
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
                    .map(|h| {
                        // Taffy reports authored definite table heights in the
                        // outer box-sizing domain. Auto tables retain the legacy
                        // known-height normalization used by fragmentation.
                        // CSS 2.2 §17.5.3: a specified height is a minimum; the
                        // table still holds its rows and the spacing.
                        let outer_height = if specified_height.is_some() {
                            f32_max_compat(h, content_height + padding_border_size.height)
                        } else {
                            (h - padding_border_size.height).max(0.0) // cov:ignore: exercised by ignored exact table-fragmentation WPT.
                        };
                        clamp_min_max(outer_height, min_h_outer, max_h_outer)
                    })
                    .unwrap_or_else(|| {
                        clamp_min_max(
                            content_height + padding_border_size.height,
                            min_h_outer,
                            max_h_outer,
                        )
                    })
            },
        };

        let (caption_size, mut caption_candidates) = layout_table_caption(
            doc,
            table_idx,
            final_size.width,
            if vertical_writing {
                Some(final_size.height)
            } else {
                inputs.parent_size.height
            },
        )
        .map(|(size, candidates)| (Some(size), candidates))
        .unwrap_or_else(|| (None, OofCandidates::new()));
        let (wrapper_size, grid_origin) =
            position_table_caption(doc, table_idx, final_size, caption_size, inputs.run_mode);
        translate_table_caption_candidates(doc, table_idx, &mut caption_candidates);
        oof_candidates.append(&mut caption_candidates);
        if inputs.run_mode == RunMode::ComputeSize {
            return Ok(LayoutOutput::from_outer_size(wrapper_size));
        }

        // Position cells: origin includes padding + collapsed outer border.
        let x_origin = pad.left
            + collapsed.outer_left
            + if shown_columns == 0 {
                0.0
            } else {
                border_spacing.0
            }
            + grid_origin.x;
        let y_origin = pad.top + collapsed.outer_top + border_spacing.1 + grid_origin.y;
        // A separated gap is a negative overlap between adjoining tracks.
        // The gap after a column sits before the next column that shows, and
        // none sits before the first one.
        let mut seen_shown = false;
        let col_gaps: Vec<f32> = (0..collapsed_cols.len())
            .map(|column| {
                seen_shown |= !collapsed_cols[column];
                let next_shown = collapsed_cols.get(column + 1).is_some_and(|next| !next);
                if seen_shown && next_shown {
                    border_spacing.0
                } else {
                    0.0
                }
            })
            .collect();
        let col_steps = if collapse {
            collapsed.col_overlaps.clone()
        } else {
            col_gaps
                .iter()
                .take(usize::from(grid.n_cols).saturating_sub(1))
                .map(|gap| -*gap)
                .collect()
        };
        let row_steps = if collapse {
            collapsed.row_overlaps.clone()
        } else {
            row_gaps.iter().map(|gap| -*gap).collect()
        };
        let col_origins = track_origins(&column_widths, &col_steps, x_origin);
        let row_origins = track_origins(&row_heights, &row_steps, y_origin);
        place_cells(
            doc,
            &mut grid,
            CellPlacementTracks {
                column_widths: &column_widths,
                column_gaps: &col_gaps,
                collapsed_columns: collapsed_cols,
                row_heights: &row_heights,
                row_gaps: &row_gaps,
                collapsed_rows,
                column_origins: &col_origins,
                row_origins: &row_origins,
                row_spacing: border_spacing.1,
                vertical_writing,
            },
            &mut oof_candidates,
        )?; // cov:ignore: failure is defensive; resolve_row_heights already validated these ranges and track_origins supplies every column origin.
        // The first row the table still shows carries its baseline.
        let baseline_row = collapsed_rows
            .iter()
            .position(|collapsed| !collapsed)
            .unwrap_or(0);
        let first_row_baseline = grid
            .cells
            .iter()
            .filter(|cell| {
                first_shown_row(cell, collapsed_rows) == baseline_row
                    && !hidden_rows[baseline_row]
                    && cell_baseline_aligned(
                        doc.ifc_layout_node(cell.node_id)
                            .expect("table cell layout view")
                            .table_vertical_align,
                    )
            })
            .map(|cell| cell.baseline)
            .reduce(f32::max)
            .unwrap_or_else(|| {
                grid.cells
                    .iter()
                    .filter(|cell| {
                        first_shown_row(cell, collapsed_rows) == baseline_row
                            && !hidden_rows[baseline_row]
                    })
                    .map(|cell| {
                        let layout = doc.table_layout_node_mut(cell.node_id).unrounded_layout;
                        layout.size.height - layout.padding.bottom - layout.border.bottom
                    })
                    .fold(0.0, f32::max)
            });
        // With every row collapsed the table has no first row, so it has no
        // first baseline, like an empty table (CSS Tables 3 §3.10.2).
        doc.nodes[table_idx].table_first_baseline = collapsed_rows
            .iter()
            .any(|collapsed| !collapsed)
            .then(|| row_origins[baseline_row] + first_row_baseline);
        if vertical_writing {
            reposition_cells_for_vertical_writing(
                doc,
                &mut grid,
                &row_heights,
                (final_size.width - padding_border_size.width - border_spacing.0 * 2.0).max(0.0),
                Point {
                    x: x_origin,
                    y: y_origin,
                },
                border_spacing.1,
            );
        }
        let projected = projects_source_parts(doc, table_idx, &grid);
        let part_claimants = table_part_claimants(doc, table_idx, &oof_candidates);
        let parts = (projected || !part_claimants.is_empty()).then(|| {
            source_part_boxes(
                doc,
                table_idx,
                &grid,
                &column_widths,
                &row_heights,
                (&col_origins, &row_origins),
                vertical_writing,
            )
        });
        if let Some(parts) = &parts {
            dispatch_table_part_oof(doc, &part_claimants, parts, projected, &mut oof_candidates);
        }
        position_source_table_parts(doc, table_idx, &grid, parts.filter(|_| projected));
        Ok(table_layout_output(
            doc,
            table_idx,
            wrapper_size,
            oof_candidates,
            inputs.run_mode,
        ))
    };
    if let Some(borders) = &prepared.borders {
        border_scopes::with_table_borders(doc, borders, layout)
    } else {
        layout(doc)
    }
}

fn table_layout_output(
    doc: &LayoutSession<'_>,
    table: usize,
    size: Size<f32>,
    mut candidates: OofCandidates,
    run_mode: RunMode,
) -> LayoutOutput {
    let mut output = LayoutOutput::from_outer_size(size);
    if run_mode != RunMode::PerformLayout {
        return output;
    }
    append_direct_table_oof_candidates(doc, table, &mut candidates);
    if candidates.is_empty() {
        return output;
    }
    let style = doc.table_style(table);
    let border = style
        .border()
        .resolve_or_zero(Some(size.width), crate::layout::dispatch::resolve_calc);
    let overflow = style.overflow();
    let gutter_x = if overflow.y.is_scroll_container() {
        style.scrollbar_width()
    } else {
        0.0
    };
    let gutter_y = if overflow.x.is_scroll_container() {
        style.scrollbar_width()
    } else {
        0.0
    };
    let rtl_gutter = if style.direction() == taffy::Direction::Rtl {
        gutter_x
    } else {
        0.0
    };
    output.oof_candidates = candidates;
    output.oof_positioning_area = Some(OofPositioningArea {
        size: Size {
            width: (size.width - border.left - border.right - gutter_x).max(0.0),
            height: (size.height - border.top - border.bottom - gutter_y).max(0.0),
        },
        offset: Point {
            x: border.left + rtl_gutter,
            y: border.top,
        },
    });
    output
}

fn append_direct_table_oof_candidates(
    doc: &Document,
    table: usize,
    candidates: &mut OofCandidates,
) {
    for (order, &child) in doc.nodes[table].children.iter().enumerate() {
        if !doc.nodes[child].is_in_document()
            || doc.nodes[child].taffy_position() != Position::Absolute
            // The table layout has always placed absolute captions at its
            // local origin; preserve that legacy geometry while dispatching
            // fixed captions against the page area.
            || doc.nodes[child].display() == DisplayValue::TableCaption
                && doc.nodes[child].position() == &PositionValue::Absolute
        {
            continue;
        }
        let position = if doc.nodes[child].position() == &PositionValue::Fixed {
            OofPosition::Fixed
        } else {
            OofPosition::Absolute
        };
        candidates.push(OofCandidate {
            node: NodeId::from(child),
            order: order as u32,
            position,
            static_position: Point {
                x: AxisStaticPosition::from_edge(0.0, AxisStaticEdge::Start),
                y: AxisStaticPosition::from_edge(0.0, AxisStaticEdge::Start),
            },
        });
    }
}

/// Whether the source rows and groups of `table` get parent-relative boxes.
///
/// Pagination needs parent-relative source boxes even when all cells are
/// explicit. Single-page callers retain their table-coordinate grid. The
/// parts of an anonymous table are DOM children of its parent, which places
/// them through their own boxes.
fn projects_source_parts(doc: &LayoutSession<'_>, table: usize, grid: &TableGrid) -> bool {
    doc.table_objects.project_explicit_parts
        || doc.is_anonymous_table(table)
        || grid
            .cells
            .iter()
            .any(|cell| doc.is_anonymous_cell(cell.node_id))
}

/// The rows and row groups of `table` that are the containing block of an
/// out-of-flow box in `candidates`, innermost first.
///
/// Taffy lays a table's cells out itself, so a row or row group that is a
/// containing block through `position`, a transform or a filter never sees
/// the boxes whose containing block it is.
fn table_part_claimants(
    doc: &LayoutSession<'_>,
    table: usize,
    candidates: &OofCandidates,
) -> Vec<usize> {
    let boundary = doc.table_parts_boundary(table);
    let mut seen = std::collections::HashSet::new();
    let mut claimants: Vec<usize> = Vec::new();
    for candidate in candidates.iter() {
        for id in doc
            .ancestors(usize::from(candidate.node))
            .take_while(|&id| id != boundary)
        {
            if is_source_table_part(doc, id)
                && seen.insert(id)
                && doc
                    .query_style(id, ReadRole::CoreContainer)
                    .is_containing_block()
                    != taffy::ContainingBlockClaims::NONE
            {
                claimants.push(id);
            }
        }
    }
    claimants.sort_by_cached_key(|&id| std::cmp::Reverse(doc.ancestors(id).count()));
    claimants
}

/// Lay out the candidates whose containing block is one of `claimants`,
/// rows and row groups, against the padding box that part has in `parts`.
/// What a part does not claim stays in `candidates`.
///
/// The candidates and the boxes laid out are in table coordinates until the
/// parts are projected, and relative to the part after that.
fn dispatch_table_part_oof(
    doc: &mut LayoutSession<'_>,
    claimants: &[usize],
    parts: &std::collections::HashMap<usize, BoxLayout>,
    projected: bool,
    candidates: &mut OofCandidates,
) {
    if claimants.is_empty() {
        return;
    }
    let mut pending = candidates.take();
    for &part in claimants {
        // A row of a nested table is in the claimants of the outer table
        // when it does not claim a box that passes through it, and the outer
        // table has no box for it.
        let Some(rect) = parts.get(&part).copied() else {
            continue;
        };
        let mut inside = OofCandidates::new();
        let mut outside = OofCandidates::new();
        for candidate in pending.iter() {
            if doc.is_ancestor_or_self(part, usize::from(candidate.node)) {
                inside.push(candidate);
            } else {
                outside.push(candidate);
            }
        }
        pending = outside;
        // The location of the part's box in the frame its children are
        // placed relative to.
        let frame = if projected {
            rect.location
        } else {
            Point::ZERO
        };
        inside.translate(Point {
            x: -frame.x,
            y: -frame.y,
        });
        let area = OofPositioningArea {
            size: rect.size,
            offset: Point {
                x: rect.location.x - frame.x,
                y: rect.location.y - frame.y,
            },
        };
        let mut result = crate::layout::dispatch::layout_container_candidates(
            doc,
            NodeId::from(part),
            &inside,
            area,
        );
        result.unclaimed.translate(frame);
        pending.append(&mut result.unclaimed);
    }
    *candidates = pending;
}

/// Give source rows and groups used boxes, while cell placement remains
/// measured in table coordinates until the final parent-relative projection.
/// `parts` are the boxes of the rows and row groups in table coordinates, or
/// `None` where the table keeps them unprojected.
fn position_source_table_parts(
    doc: &mut LayoutSession<'_>,
    table: usize,
    grid: &TableGrid,
    parts: Option<std::collections::HashMap<usize, BoxLayout>>,
) {
    let Some(parts) = parts else {
        return;
    };
    let boundary = doc.table_parts_boundary(table);
    // A part the grid does not place, such as an empty row group, must not
    // keep a box from an earlier pass as its table-relative box.
    for part in doc
        .anonymous_table_parts(table)
        .unwrap_or_default()
        .to_vec()
    {
        doc.nodes[part].unrounded_layout = BoxLayout::new();
    }
    if doc.nodes[table].border_collapse != BorderCollapseValue::Collapse {
        for &id in parts.keys() {
            doc.table_objects
                .part_background_cells
                .insert(id, Vec::new());
        }
        // Visit only the rows occupied by each cell and their group ancestors.
        // Scanning the whole cell grid for every row would be quadratic.
        for cell in &grid.cells {
            if doc
                .ifc_layout_node(cell.node_id)
                .expect("table box layout view")
                .hides_empty_table_cell
            {
                continue;
            }
            let layout = cell
                .resolved
                .expect("cells were positioned before source parts");
            let mut listed = std::collections::HashSet::new();
            let end_row = (usize::from(cell.row) + usize::from(cell.row_span)).min(grid.rows.len());
            for &marker in &grid.rows[usize::from(cell.row)..end_row] {
                let mut source =
                    AncestorCursor::new(Some(doc.ifc_source_owner(marker)), ParentRelation::Source);
                while let Some(id) = source.current() {
                    if id == boundary {
                        break;
                    }
                    if let Some(part) = parts.get(&id)
                        && listed.insert(id)
                    {
                        let left = part.location.x.max(layout.location.x);
                        let top = part.location.y.max(layout.location.y);
                        let right = (part.location.x + part.size.width)
                            .min(layout.location.x + layout.size.width);
                        let bottom = (part.location.y + part.size.height)
                            .min(layout.location.y + layout.size.height);
                        if right > left && bottom > top {
                            doc.table_objects
                                .part_background_cells
                                .get_mut(&id)
                                .expect("table parts were initialized")
                                .push(raikiri_traits::PaintRect::new(
                                    left - part.location.x,
                                    top - part.location.y,
                                    right - left,
                                    bottom - top,
                                ));
                        }
                    }
                    source.advance(doc);
                }
            }
        }
    }
    let origin_of = |owner: usize| {
        doc.inclusive_ancestors(owner)
            .take_while(|&id| id != boundary)
            .find_map(|id| parts.get(&id))
            .map_or(Point::ZERO, |part| part.location)
    };
    let mut projected = Vec::new();
    for (&id, &part) in &parts {
        let parent = doc.parent_of(id).map_or(Point::ZERO, origin_of);
        let mut layout = part;
        layout.location.x -= parent.x;
        layout.location.y -= parent.y;
        projected.push((id, layout));
    }
    let mut offsets = Vec::new();
    for cell in &grid.cells {
        let owner = doc.ifc_source_owner(cell.node_id);
        let parent = if doc.is_anonymous_cell(cell.node_id) {
            Some(owner)
        } else {
            doc.parent_of(owner)
        };
        offsets.push((cell.node_id, parent.map_or(Point::ZERO, origin_of)));
    }
    for (id, layout) in projected {
        doc.table_objects.materialized_parts.insert(id);
        doc.store_unrounded_layout(id, &layout);
    }
    for (key, offset) in offsets {
        let node = doc.table_layout_node_mut(key);
        node.unrounded_layout.location.x -= offset.x;
        node.unrounded_layout.location.y -= offset.y;
        if doc.is_anonymous_cell(key) {
            let root = doc
                .ifc_layout_node(key)
                .expect("anonymous cell layout view");
            let children = if root.is_ifc_root() {
                root.ifc_boxes()
            } else {
                root.children.clone()
            };
            for child in children {
                doc.nodes[child].unrounded_layout.location.x -= offset.x;
                doc.nodes[child].unrounded_layout.location.y -= offset.y;
            }
        }
    }
    doc.place_anonymous_table_parts(table);
}

/// The boxes of the rows and row groups of `table` in table coordinates:
/// each part spans the rows of the cells it holds.
fn source_part_boxes(
    doc: &Document,
    table: usize,
    grid: &TableGrid,
    columns: &[f32],
    rows: &[f32],
    origins: (&[f32], &[f32]),
    vertical: bool,
) -> std::collections::HashMap<usize, BoxLayout> {
    let boundary = doc.table_parts_boundary(table);
    let mut parts: std::collections::HashMap<usize, BoxLayout> = Default::default();
    let (col_x, row_y) = origins;
    let width = col_x
        .get(columns.len().saturating_sub(1))
        .zip(columns.last())
        .map_or(0.0, |(x, width)| {
            x + width - col_x.first().copied().unwrap_or(0.0)
        });
    let mut vertical_rows: std::collections::HashMap<usize, BoxLayout> = Default::default();
    if vertical {
        for cell in &grid.cells {
            let layout = cell
                .resolved
                .expect("cells were positioned before source parts");
            vertical_rows
                .entry(usize::from(cell.row))
                .and_modify(|row| extend_part_bounds(row, &layout))
                .or_insert(layout);
        }
    }
    for (index, &marker) in grid.rows.iter().enumerate() {
        let mut rect = BoxLayout::new();
        rect.location = Point {
            x: col_x.first().copied().unwrap_or(0.0),
            y: row_y[index],
        };
        rect.size = Size {
            width,
            height: rows[index],
        };
        if let Some(layout) = vertical_rows.get(&index) {
            rect = *layout;
        }
        for id in doc
            .inclusive_ancestors(doc.ifc_source_owner(marker))
            .take_while(|&id| id != boundary)
        {
            if is_source_table_part(doc, id) {
                parts
                    .entry(id)
                    .and_modify(|part| extend_part_bounds(part, &rect))
                    .or_insert(rect);
            }
        }
    }
    parts
}

fn is_source_table_part(doc: &Document, id: usize) -> bool {
    matches!(
        doc.nodes[id].display(),
        DisplayValue::TableRow
            | DisplayValue::TableRowGroup
            | DisplayValue::TableHeaderGroup
            | DisplayValue::TableFooterGroup
    )
}

fn extend_part_bounds(part: &mut BoxLayout, other: &BoxLayout) {
    let right = (part.location.x + part.size.width).max(other.location.x + other.size.width);
    let bottom = (part.location.y + part.size.height).max(other.location.y + other.size.height);
    part.location.x = part.location.x.min(other.location.x);
    part.location.y = part.location.y.min(other.location.y);
    part.size = Size {
        width: right - part.location.x,
        height: bottom - part.location.y,
    };
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

fn table_column_error() -> raikiri_traits::LayoutError {
    raikiri_traits::LayoutError::Internal {
        message: "table column grid exceeds supported bounds".to_owned(),
    }
}

fn next_table_column(column: u16, span: u16) -> Result<u16, raikiri_traits::LayoutError> {
    column.checked_add(span).ok_or_else(table_column_error)
}

fn cell_column_range(
    cell: &CellPlacement,
    columns: usize,
) -> Result<std::ops::Range<usize>, raikiri_traits::LayoutError> {
    let start = usize::from(cell.col_start);
    let end = start + usize::from(cell.col_span);
    if start >= end || end > columns {
        return Err(table_column_error());
    }
    Ok(start..end)
}

fn build_table_grid(
    doc: &LayoutSession<'_>,
    table_idx: usize,
) -> Result<TableGrid, raikiri_traits::LayoutError> {
    #[cfg(test)]
    GRID_BUILDS.with(|count| count.set(count.get() + 1));
    let mut rows: Vec<usize> = Vec::new();
    let mut cells: Vec<CellPlacement> = Vec::new();
    let mut n_cols: u16 = 0;
    if let Some(projected_rows) = doc.table_objects.rows.get(&table_idx) {
        for projected in projected_rows {
            rows.push(projected.marker);
            let row = u16::try_from(rows.len() - 1).map_err(|_| table_column_error())?;
            let mut col = 0;
            for &node_id in &projected.cells {
                let anonymous = doc.is_anonymous_cell(node_id);
                let col_span = if anonymous {
                    1
                } else {
                    get_colspan(doc, node_id)
                };
                let row_span = if anonymous {
                    1
                } else {
                    get_rowspan(doc, node_id)
                };
                let next_col = next_table_column(col, col_span)?;
                let (specified_width, specified_min_width) = cell_specified_widths(doc, node_id);
                cells.push(CellPlacement {
                    node_id,
                    row,
                    col_start: col,
                    col_span,
                    row_span,
                    specified_width,
                    specified_min_width,
                    resolved: None,
                    natural_height: 0.0,
                    baseline: 0.0,
                    row_pass: None,
                });
                col = next_col;
            }
            n_cols = n_cols.max(col);
        }
    } else {
        collect_rows(doc, table_idx, &mut rows, &mut cells, &mut n_cols)?;
    }
    n_cols = n_cols.max(rowspan::reserve_columns(doc, table_idx, &rows, &mut cells)?);
    let col_widths = collect_col_widths(doc, table_idx)?;
    n_cols = n_cols.max(col_widths.len() as u16);
    Ok(TableGrid {
        n_cols,
        rows,
        cells,
        col_widths,
    })
}

/// Authored sizing of one `<col>`: width + min/max-width (lengths only;
/// percentages and calc-with-percentage are indefinite at column-measure
/// time and handled by the caller).
#[derive(Debug, Clone, Copy)]
struct ColSizing {
    width: Dimension,
    min_width: Dimension,
    max_width: Dimension,
}

/// Authored `<col>` sizing in grid order, one entry per column.
///
/// The columns come from [`each_column_run`], so `span` (HTML §4.9.8, max
/// 1000) expands to that many entries and `display: contents` wrappers are
/// looked through.
fn collect_col_widths(
    doc: &LayoutSession<'_>,
    table_idx: usize,
) -> Result<Vec<ColSizing>, raikiri_traits::LayoutError> {
    let mut out = Vec::new();
    let mut too_many = false;
    each_column_run(doc, table_idx, |sizing_node, _, span| {
        let style = doc.table_style(sizing_node);
        let sizing = ColSizing {
            width: style.size().width,
            min_width: style.min_size().width.into(),
            max_width: style.max_size().width.into(),
        };
        if span > usize::from(u16::MAX) - out.len() {
            too_many = true;
        } else {
            out.extend(std::iter::repeat_n(sizing, span));
        }
    });
    if too_many {
        return Err(table_column_error());
    }
    Ok(out)
}

/// Columns removed by `visibility: collapse` on the `<col>` or its
/// `<colgroup>` (CSS Tables 3, "Visibility-collapse rendering"), one entry
/// per grid column.
fn collapsed_columns(doc: &Document, table_idx: usize, n_cols: u16) -> Vec<bool> {
    let mut out = Vec::new();
    // `collect_col_widths` has already rejected a column count past `u16`.
    each_column_run(doc, table_idx, |_, collapsed, span| {
        out.extend(std::iter::repeat_n(collapsed, span));
    });
    out.resize(usize::from(n_cols), false);
    out
}

/// Walks the table's `<col>` and `<colgroup>` children in grid order and
/// calls `each` with the node that sizes a run of columns, whether the run
/// is collapsed, and its length. `span` (HTML §4.9.8, max 1000) sets the
/// length; a `<colgroup>` without `<col>` children stands for its own `span`
/// columns and sizes them itself. Direct `<col>` children (anonymous
/// colgroup) are included in DOM order. A `display: contents` element makes
/// no box, so its children take its place, both under the table and under a
/// `<colgroup>`.
fn each_column_run(doc: &Document, table_idx: usize, mut each: impl FnMut(usize, bool, usize)) {
    fn col_span(doc: &Document, node_id: usize) -> usize {
        doc.ifc_layout_node(node_id)
            .expect("table box layout view")
            .attribute("span")
            .and_then(|value| value.parse::<usize>().ok())
            .map_or(1, |v| v.clamp(1, 1000))
    }
    // The children of `parent` that take part in the table, with each
    // `display: contents` child replaced by its own children. Elements that
    // are never rendered (`script`, `datalist`, ...) drop out together with
    // their subtrees, as they do from the table's row projection.
    // spec: https://www.w3.org/TR/css-display-3/#valdef-display-contents
    fn box_children(doc: &Document, parent: usize, out: &mut Vec<usize>) {
        // An explicit stack keeps deeply nested wrappers off the call stack.
        let mut pending: Vec<usize> = doc.nodes[parent].children.iter().rev().copied().collect();
        while let Some(child_id) = pending.pop() {
            if !doc.nodes[child_id].is_in_document()
                || doc.nodes[child_id].is_non_rendered_html_element()
            {
                continue;
            }
            if doc.nodes[child_id].display() == DisplayValue::Contents {
                pending.extend(doc.nodes[child_id].children.iter().rev());
            } else {
                out.push(child_id);
            }
        }
    }
    let mut table_children = Vec::new();
    box_children(doc, table_idx, &mut table_children);
    let mut group_children = Vec::new();
    for child_id in table_children {
        match doc.nodes[child_id].display() {
            DisplayValue::TableColumnGroup => {
                let group_collapsed = doc.nodes[child_id].collapses_table_columns;
                let mut has_columns = false;
                group_children.clear();
                box_children(doc, child_id, &mut group_children);
                for &col_id in &group_children {
                    match doc.nodes[col_id].display() {
                        DisplayValue::TableColumn => {}
                        // A `<col>` that generates no box still ends the
                        // group's own `span`.
                        DisplayValue::None => {
                            has_columns = true;
                            continue;
                        }
                        _ => continue,
                    }
                    has_columns = true;
                    each(
                        col_id,
                        group_collapsed || doc.nodes[col_id].collapses_table_columns,
                        col_span(doc, col_id),
                    );
                }
                if !has_columns {
                    each(child_id, group_collapsed, col_span(doc, child_id));
                }
            }
            DisplayValue::TableColumn => {
                each(
                    child_id,
                    doc.nodes[child_id].collapses_table_columns,
                    col_span(doc, child_id),
                );
            }
            _ => {}
        }
    }
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
    doc: &LayoutSession<'_>,
    container_idx: usize,
    rows: &mut Vec<usize>,
    cells: &mut Vec<CellPlacement>,
    n_cols: &mut u16,
) -> Result<(), raikiri_traits::LayoutError> {
    collect_rows_inner(doc, container_idx, rows, cells, n_cols, true)
}

fn collect_rows_inner(
    doc: &LayoutSession<'_>,
    container_idx: usize,
    rows: &mut Vec<usize>,
    cells: &mut Vec<CellPlacement>,
    n_cols: &mut u16,
    reorder_sections: bool,
) -> Result<(), raikiri_traits::LayoutError> {
    let mut pending: Vec<usize> = Vec::new();

    let len = doc.nodes[container_idx].children.len();
    // At table level, float the first header group to the top and sink
    // the first footer group to the bottom (stable otherwise). Nested
    // groups keep DOM order.
    let order: Vec<usize> = if reorder_sections {
        let mut idxs: Vec<usize> = (0..len).collect();
        let displays: Vec<DisplayValue> = idxs
            .iter()
            .map(|&i| doc.nodes[doc.nodes[container_idx].children[i]].display())
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
        let disp = doc.nodes[child_id].display();
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
            flush_pending(doc, &mut pending, rows, cells, n_cols)?;
            collect_cells_in_row(doc, child_id, rows, cells, n_cols)?;
        } else if is_cell {
            pending.push(child_id);
        } else if is_row_group || is_contents {
            // Anonymous row-group: recurse. But flush pending first (cells cannot cross row-group boundary)
            flush_pending(doc, &mut pending, rows, cells, n_cols)?;
            collect_rows_inner(doc, child_id, rows, cells, n_cols, false)?;
        } else {
            // Non-table descendent (e.g., caption, div inside table). For initial implementation, skip but flush pending.
            flush_pending(doc, &mut pending, rows, cells, n_cols)?;
            // If this is a caption etc., ignore for grid. If it's inside table but not row/cell,
            // recursion not needed - those children are not part of table grid.
            // However if it's a plain div wrapping rows (anomalous), recurse to find rows inside.
            // Heuristic: recurse if it has children that could be rows/cells.
            if !doc.nodes[child_id].children.is_empty() {
                // Only recurse if container might hold rows; avoid diving into block content inside cells.
                // We treat any block container directly under table that contains row-like descendants as group.
                // Simple: recurse
                collect_rows_inner(doc, child_id, rows, cells, n_cols, false)?;
            }
        }
    }
    flush_pending(doc, &mut pending, rows, cells, n_cols)
}

fn flush_pending(
    doc: &LayoutSession<'_>,
    pending: &mut Vec<usize>,
    rows: &mut Vec<usize>,
    cells: &mut Vec<CellPlacement>,
    n_cols: &mut u16,
) -> Result<(), raikiri_traits::LayoutError> {
    if pending.is_empty() {
        return Ok(());
    }
    // Anonymous row: borrow first cell's id as row marker (not used for layout beyond count)
    push_row(doc, pending[0], pending.drain(..), rows, cells, n_cols)
}

fn collect_cells_in_row(
    doc: &LayoutSession<'_>,
    row_id: usize,
    rows: &mut Vec<usize>,
    cells: &mut Vec<CellPlacement>,
    n_cols: &mut u16,
) -> Result<(), raikiri_traits::LayoutError> {
    // Only collect cells; non-cell children are anonymous? Per spec, non-cell child of a row
    // is wrapped in anonymous cell — for initial implementation we skip non-cells inside row.
    let row_cells = doc.nodes[row_id]
        .children
        .iter()
        .copied()
        .filter(|&cell_id| {
            doc.nodes[cell_id].is_in_document()
                && doc.nodes[cell_id].display() == DisplayValue::TableCell
        });
    push_row(doc, row_id, row_cells, rows, cells, n_cols)
}

/// Append row `row_id` and place `row_cells` in it left to right, each
/// starting where the one before it ends.
fn push_row(
    doc: &LayoutSession<'_>,
    row_id: usize,
    row_cells: impl IntoIterator<Item = usize>,
    rows: &mut Vec<usize>,
    cells: &mut Vec<CellPlacement>,
    n_cols: &mut u16,
) -> Result<(), raikiri_traits::LayoutError> {
    rows.push(row_id);
    let row_ix = (rows.len() - 1) as u16;
    let mut col: u16 = 0;
    for cell_id in row_cells {
        let col_span = get_colspan(doc, cell_id);
        let next_col = next_table_column(col, col_span)?;
        let row_span = get_rowspan(doc, cell_id);
        let (specified_width, specified_min_width) = cell_specified_widths(doc, cell_id);
        cells.push(CellPlacement {
            node_id: cell_id,
            row: row_ix,
            col_start: col,
            col_span,
            row_span,
            specified_width,
            specified_min_width,
            resolved: None,
            natural_height: 0.0,
            baseline: 0.0,
            row_pass: None,
        });
        col = next_col;
    }
    *n_cols = (*n_cols).max(col);
    Ok(())
}

/// Cell `colspan` per WHATWG HTML 4.9.12.1 ("Forming a table", "Cells" step):
/// a missing attribute, a parse failure, or a zero value defaults to 1,
/// and values greater than 1000 clamp to 1000 (mirroring the `colSpan`
/// IDL `ReflectRange=(1, 1000)`).
///
/// Note: the full "rules for parsing non-negative integers" tolerance
/// (surrounding whitespace, trailing junk) is intentionally not implemented;
/// the value must parse as a plain unsigned integer, anything else falls
/// back to 1.
fn get_colspan(doc: &Document, node_id: usize) -> u16 {
    // Parse wide (u32) so huge values (e.g. "100000", which overflows u16)
    // clamp to 1000 instead of failing to parse and falling back to 1.
    doc.ifc_layout_node(node_id)
        .expect("table box layout view")
        .attribute("colspan")
        .and_then(|value| value.parse::<u32>().ok())
        .map_or(1, |v| v.clamp(1, 1000) as u16)
}

fn get_rowspan(doc: &Document, node_id: usize) -> u16 {
    doc.ifc_layout_node(node_id)
        .expect("table box layout view")
        .attribute("rowspan")
        .and_then(|value| {
            let digits = value.strip_prefix('+').unwrap_or(value);
            if digits.is_empty() {
                return None;
            }
            // Saturate any digit count, but still reject invalid suffixes.
            digits.bytes().try_fold(0_u16, |span, digit| {
                digit.is_ascii_digit().then(|| {
                    span.saturating_mul(10)
                        .saturating_add(u16::from(digit - b'0'))
                })
            })
        })
        .map_or(1, |v| {
            // HTML §4.9.9: rowspan=0 spans to the end of the table section.
            // The reservation pass clamps this sentinel to the source row
            // group before layout consumes the grid.
            if v == 0 { u16::MAX } else { v.min(65534) }
        })
}

// ---------------------------------------------------------------------------
// Measure cells
// ---------------------------------------------------------------------------

fn measure_cell_inline(doc: &mut LayoutSession<'_>, cell_id: usize, axis: AvailableSpace) -> f32 {
    // Only the width is read. A cell whose content is one horizontal
    // paragraph without boxes of its own gets it from the paragraph's
    // intrinsic widths, without breaking it into lines; the size cached for
    // that probe would carry no height, so the cell's cache is restored.
    let width_only = is_plain_paragraph_cell(doc, cell_id);
    let saved_cache = width_only.then(|| doc.clone_layout_cache(cell_id));
    doc.context.ifc_width_probe = width_only;
    // Use a fresh measure: indefinite parent size so that % widths resolve to auto (spec §2.3).
    // Height available is MAX_CONTENT (intrinsic).
    let output = doc.compute_child_layout(
        NodeId::from(cell_id),
        LayoutInput {
            run_mode: RunMode::ComputeSize,
            sizing_mode: SizingMode::InherentSize,
            axis: RequestedAxis::Horizontal,
            known_dimensions: Size::NONE,
            parent_size: Size::NONE,
            available_space: Size {
                width: axis,
                height: AvailableSpace::MAX_CONTENT,
            },
            known_dimensions_are_definite: Size {
                width: true,
                height: true,
            },
            vertical_margins_are_collapsible: Line::FALSE,
        },
    );
    doc.context.ifc_width_probe = false;
    if let Some(cache) = saved_cache {
        doc.restore_layout_cache(cell_id, cache);
    }
    output.size.width
}

#[derive(Clone, Copy)]
struct IntrinsicSizeContext<'a> {
    min_content: f32,
    max_content: f32,
    inputs: &'a LayoutInput,
    available_space: AvailableSpace,
    horizontal: bool,
}

/// Resolve a cell's intrinsic width limits against the min/max-content border-box
/// measurements that the table column algorithm already computed.
fn resolve_intrinsic_limit(
    doc: &LayoutSession<'_>,
    node_id: usize,
    limit: Dimension,
    context: IntrinsicSizeContext<'_>,
) -> Option<f32> {
    let expanded = limit.expand();
    match expanded {
        ExpandedDimension::MinContent => Some(context.min_content),
        ExpandedDimension::MaxContent => Some(context.max_content),
        ExpandedDimension::FitContent
        | ExpandedDimension::FitContentPx(_)
        | ExpandedDimension::FitContentPercent(_)
        | ExpandedDimension::FitContentCalc(_) => {
            let basis = if context.horizontal {
                context.inputs.parent_size.width
            } else {
                context.inputs.parent_size.height
            };
            let fit_room = crate::layout::dispatch::fit_content_room(
                doc,
                node_id,
                expanded,
                basis,
                context.horizontal,
            )
            .or_else(|| match context.available_space {
                AvailableSpace::MinContent => Some(context.min_content),
                AvailableSpace::MaxContent => Some(context.max_content),
                AvailableSpace::Definite(room) => Some(room.max(0.0)),
            })?;
            Some(context.max_content.min(fit_room).max(context.min_content))
        }
        _ => None,
    }
}

fn constrain_intrinsic_sizes(
    doc: &LayoutSession<'_>,
    node_id: usize,
    min_limit: Dimension,
    max_limit: Dimension,
    context: IntrinsicSizeContext<'_>,
) -> (f32, f32) {
    let min = resolve_intrinsic_limit(doc, node_id, min_limit, context)
        .map_or(context.min_content, |limit| context.min_content.max(limit));
    let max = resolve_intrinsic_limit(doc, node_id, max_limit, context)
        .map_or(context.max_content, |limit| context.max_content.min(limit));
    (min, max.max(min))
}

/// The table's column track room, before adding a spanning cell's internal gaps.
fn table_column_available_space(inputs: &LayoutInput, insets_width: f32) -> AvailableSpace {
    if let Some(width) = inputs.known_dimensions.width {
        return AvailableSpace::Definite((width - insets_width).max(0.0));
    }
    match inputs.available_space.width {
        AvailableSpace::Definite(width) => {
            AvailableSpace::Definite((width - insets_width).max(0.0))
        }
        intrinsic => intrinsic,
    }
}

fn constrain_cell_intrinsic_widths(
    doc: &LayoutSession<'_>,
    cell_id: usize,
    limits: ContentLimits,
    min_content: f32,
    max_content: f32,
    inputs: &LayoutInput,
    available_space: AvailableSpace,
) -> (f32, f32) {
    let context = IntrinsicSizeContext {
        min_content,
        max_content,
        inputs,
        available_space,
        horizontal: true,
    };
    constrain_intrinsic_sizes(doc, cell_id, limits.min.width, limits.max.width, context)
}

/// What the column pass needs besides the grid and the room the tracks share.
struct ColumnPass {
    table_layout: TableLayoutValue,
    specified_width: Option<f32>,
    caption_minimum: f32,
    wrapper_minimum: Option<f32>,
    wrapper_maximum: Option<f32>,
    inputs: LayoutInput,
    border_spacing_x: f32,
}

fn apply_wrapper_width_bounds(width: f32, minimum: Option<f32>, maximum: Option<f32>) -> f32 {
    let width = minimum.map_or(width, |minimum| width.max(minimum));
    maximum.map_or(width, |maximum| width.min(maximum))
}

/// Column widths for `grid` by the automatic or fixed algorithm (CSS 2.1
/// §17.5.2). `insets_width` is the table width the tracks do not share:
/// padding, borders and spacing net of collapsed-line overlaps.
fn resolve_table_columns(
    doc: &mut LayoutSession<'_>,
    grid: &mut TableGrid,
    pass: &ColumnPass,
    insets_width: f32,
) -> Vec<f32> {
    let insets = Size {
        width: insets_width,
        height: 0.0,
    };
    if pass.table_layout != TableLayoutValue::Fixed {
        return resolve_column_widths(doc, grid, pass.inputs, insets, pass.border_spacing_x);
    }
    // Fixed with an indefinite (auto/percent-of-indefinite) table width
    // has no basis for the §17.5.2.1 distribution — fall back to auto.
    // Crucially the container width is NOT a substitute basis: a
    // fixed+auto table shrink-wraps like an auto table (WPT
    // table_grid_size_col_colspan), it does not fill its container.
    let avail = pass.specified_width.map(|width| {
        let width = apply_wrapper_width_bounds(
            width.max(pass.caption_minimum),
            pass.wrapper_minimum,
            pass.wrapper_maximum,
        );
        f32_max_compat(width - insets_width, 0.0)
    });
    let Some(avail) = avail else {
        return resolve_column_widths(doc, grid, pass.inputs, insets, pass.border_spacing_x);
    };
    for cell in &mut grid.cells {
        if cell.row != 0 {
            continue;
        }
        let style = doc.table_style(cell.node_id);
        if let Some(minimum) = style
            .min_size()
            .width
            .maybe_resolve(Some(avail), crate::layout::dispatch::resolve_calc)
        {
            let padding = style
                .padding()
                .resolve_or_zero(Some(avail), crate::layout::dispatch::resolve_calc);
            let border = style
                .border()
                .resolve_or_zero(Some(avail), crate::layout::dispatch::resolve_calc);
            let cell_insets = padding.left + padding.right + border.left + border.right;
            let outer = if style.box_sizing() == taffy::BoxSizing::ContentBox {
                minimum + cell_insets
            } else {
                minimum.max(cell_insets)
            };
            let gaps = pass.border_spacing_x * usize::from(cell.col_span).saturating_sub(1) as f32;
            cell.specified_min_width = Dimension::length((outer - gaps).max(0.0));
        }
    }
    resolve_fixed_column_widths(grid, avail)
}

// ---------------------------------------------------------------------------
// Column sizing — ported from Blitz L3 (table layout design)
// ---------------------------------------------------------------------------

fn resolve_column_widths(
    doc: &mut LayoutSession<'_>,
    grid: &TableGrid,
    inputs: LayoutInput,
    padding_border_size: Size<f32>,
    spacing: f32,
) -> Vec<f32> {
    #[cfg(test)]
    tests::COLUMN_RESOLUTIONS.with(|count| count.set(count.get() + 1));
    let n = grid.n_cols as usize;
    let column_space = table_column_available_space(&inputs, padding_border_size.width);
    // (B) Measure each cell
    let mut cell_min = vec![0.0f32; grid.cells.len()];
    let mut cell_max = vec![0.0f32; grid.cells.len()];
    for (i, cell) in grid.cells.iter().enumerate() {
        let min_content = measure_cell_inline(doc, cell.node_id, AvailableSpace::MIN_CONTENT);
        let max_content = measure_cell_inline(doc, cell.node_id, AvailableSpace::MAX_CONTENT);
        let cell_space = match column_space {
            AvailableSpace::Definite(room) => AvailableSpace::Definite(
                room + spacing * usize::from(cell.col_span).saturating_sub(1) as f32,
            ),
            intrinsic => intrinsic,
        };
        let (min, max) = constrain_cell_intrinsic_widths(
            doc,
            cell.node_id,
            doc.nodes[cell.node_id].content_limits,
            min_content,
            max_content,
            &inputs,
            cell_space,
        );
        cell_min[i] = min;
        cell_max[i] = max;
    }

    // (C.1) Aggregate colspan==1
    let mut col_min = vec![0.0f32; n];
    let mut col_max = vec![0.0f32; n];
    let mut col_pct = vec![0.0f32; n];
    let mut authored_length = vec![false; n];
    for (i, cell) in grid.cells.iter().enumerate() {
        #[cfg(test)]
        tests::AUTHORED_LENGTH_CELL_VISITS.with(|count| count.set(count.get() + 1));
        let c = cell.col_start as usize;
        if cell.col_span == 1 && c < n {
            authored_length[c] |= cell.specified_width.tag() == CompactLength::LENGTH_TAG;
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
        // The spanned columns' separated gaps already give the cell room.
        let gaps = spacing * (e - s - 1) as f32;
        let cur_min: f32 = col_min_full[s..e].iter().sum::<f32>() + gaps;
        if cell_min[i] > cur_min {
            let add = (cell_min[i] - cur_min) / cnt;
            for column in &targets {
                col_min_full[*column] += add;
            }
        }
        let cur_max: f32 = col_max_full[s..e].iter().sum::<f32>() + gaps;
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
            col_len[c] = resolve_dimension(cell.specified_width, None);
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
        let min_w = resolve_dimension(sz.min_width, None);
        let max_w = resolve_dimension(sz.max_width, None);
        // Base width: definite length always; percent only when occupied.
        let base = resolve_dimension(sz.width, None);
        if base.is_none() && sz.width.tag() == CompactLength::PERCENT_TAG && occupied[c] {
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
                    distribute_columns_with_authored(
                        avail,
                        &col_min_full,
                        &col_max_full,
                        &col_pct,
                        &authored_length,
                    )
                }
            }
        },
    }
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

/// Rows removed by `visibility: collapse` on the row or its row group (CSS
/// Tables 3, "Visibility-collapse rendering"). An anonymous row is marked by
/// its first cell and collapses only with the row group around it.
fn collapsed_rows(doc: &Document, grid: &TableGrid) -> Vec<bool> {
    grid.rows
        .iter()
        .map(|&row_id| {
            let row = &doc.nodes[row_id];
            // `display: contents` wrappers are transparent to the grid, so
            // the row group may sit further up.
            let group = doc
                .ancestors(row_id)
                .find(|&id| doc.nodes[id].display() != DisplayValue::Contents);
            let in_collapsed_group =
                group.is_some_and(|group| doc.nodes[group].collapses_table_rows);
            match row.display() {
                DisplayValue::TableRow => row.collapses_table_rows || in_collapsed_group,
                DisplayValue::TableCell => in_collapsed_group,
                _ => false,
            }
        })
        .collect()
}

/// The grid as it shows once `collapsed` rows are removed: later rows move
/// up, and each cell keeps only the rows of its span that remain.
fn without_collapsed_rows(grid: &TableGrid, collapsed: &[bool]) -> TableGrid {
    let mut shown_before = Vec::with_capacity(collapsed.len() + 1);
    let mut shown = 0u16;
    for &row_collapsed in collapsed {
        shown_before.push(shown);
        shown += u16::from(!row_collapsed);
    }
    shown_before.push(shown);
    let cells = grid
        .cells
        .iter()
        .filter_map(|cell| {
            let start = cell.row as usize;
            let end = (start + cell.row_span as usize).min(collapsed.len());
            let row_span = shown_before[end] - shown_before[start];
            (row_span > 0).then_some(CellPlacement {
                row: shown_before[start],
                row_span,
                ..*cell
            })
        })
        .collect();
    TableGrid {
        n_cols: grid.n_cols,
        rows: grid
            .rows
            .iter()
            .zip(collapsed)
            .filter(|(_, collapsed)| !**collapsed)
            .map(|(&row, _)| row)
            .collect(),
        cells,
        col_widths: grid.col_widths.clone(),
    }
}

/// The grid as it shows once `collapsed` columns are removed: later columns
/// move left. With `keep_crossing`, each cell keeps only the columns of its
/// span that remain; without it, a cell spanning a collapsed column is left
/// out, so its content cannot widen the columns that stay.
fn without_collapsed_columns(
    grid: &TableGrid,
    collapsed: &[bool],
    keep_crossing: bool,
) -> TableGrid {
    let mut shown_before = Vec::with_capacity(collapsed.len() + 1);
    let mut shown = 0u16;
    for &column_collapsed in collapsed {
        shown_before.push(shown);
        shown += u16::from(!column_collapsed);
    }
    shown_before.push(shown);
    let cells = grid
        .cells
        .iter()
        .filter_map(|cell| {
            let start = cell.col_start as usize;
            let end = (start + cell.col_span as usize).min(collapsed.len());
            let col_span = shown_before[end] - shown_before[start];
            let crosses = usize::from(col_span) != end - start;
            (col_span > 0 && (keep_crossing || !crosses)).then_some(CellPlacement {
                col_start: shown_before[start],
                col_span,
                ..*cell
            })
        })
        .collect();
    TableGrid {
        n_cols: shown,
        rows: grid.rows.clone(),
        cells,
        col_widths: grid
            .col_widths
            .iter()
            .zip(collapsed)
            .filter(|(_, collapsed)| !**collapsed)
            .map(|(&sizing, _)| sizing)
            .collect(),
    }
}

/// The first row a cell shows in once collapsed rows are removed, or the
/// row it starts in when its whole span is collapsed.
fn first_shown_row(cell: &CellPlacement, collapsed: &[bool]) -> usize {
    let start = cell.row as usize;
    let end = (start + cell.row_span as usize).min(collapsed.len());
    (start..end).find(|&row| !collapsed[row]).unwrap_or(start)
}

fn hidden_empty_rows(doc: &Document, grid: &TableGrid) -> Vec<bool> {
    let mut rows = vec![None; grid.rows.len()];
    for cell in &grid.cells {
        let start = cell.row as usize;
        let end = (start + cell.row_span as usize).min(rows.len());
        for row in &mut rows[start..end] {
            *row = Some(
                row.unwrap_or(true)
                    && doc
                        .ifc_layout_node(cell.node_id)
                        .expect("table cell layout view")
                        .hides_empty_table_cell,
            );
        }
    }
    rows.into_iter().map(|row| row.unwrap_or(false)).collect()
}

/// Grow `sizes` (row heights or column widths) until they sum to `target`,
/// skipping the tracks `hidden` marks: each grows in proportion to its
/// current size, or by an equal share when all are zero. Tracks are sized in
/// pre-overlap space, so the caller passes the outer-domain target
/// (`min_outer - distrib_insets` = `min_content + overlap`).
fn distribute_extra(sizes: &mut [f32], hidden: Option<&[bool]>, target: f32) {
    let is_hidden = |index: usize| hidden.and_then(|hidden| hidden.get(index)) == Some(&true);
    let n = (0..sizes.len()).filter(|&index| !is_hidden(index)).count();
    if n == 0 {
        return;
    }
    let current: f32 = sizes.iter().sum();
    if current >= target {
        return;
    }
    let extra = target - current;
    for (index, size) in sizes.iter_mut().enumerate() {
        if is_hidden(index) {
            continue;
        }
        *size += if current > 0.0 {
            extra * (*size) / current
        } else {
            extra / n as f32
        };
    }
}

// ---------------------------------------------------------------------------
// Row heights
// ---------------------------------------------------------------------------

fn cell_baseline_aligned(align: VerticalAlign) -> bool {
    !matches!(
        align,
        VerticalAlign::Top | VerticalAlign::Middle | VerticalAlign::Bottom
    )
}

// Baselines are relative to the border edge; a cell without a line uses
// its bottom content edge in the caller (CSS 2.2 section 17.5.3).
fn first_cell_baseline(doc: &LayoutSession<'_>, root: usize, content_top: f32) -> Option<f32> {
    let node = doc.ifc_layout_node(root).expect("table cell layout view");
    if matches!(
        node.display(),
        DisplayValue::Table | DisplayValue::InlineTable
    ) && !node.is_ifc_root()
    {
        // Captions are outside the grid and cannot supply its first row baseline.
        return node.table_first_baseline;
    }

    if let Some(lines) = node.ifc.as_ref().and_then(|ifc| ifc.lines.as_ref())
        && let Some(baseline) = super::ifc::flow::first_baseline(lines)
    {
        // A block before line zero can supply the first qualifying line or
        // table row. Later blocks must not displace the root's first line.
        for &(child, line) in &lines.block_line_starts {
            if line != 0 {
                continue;
            }
            let child_node = doc.ifc_layout_node(child).unwrap();
            let top =
                child_node.unrounded_layout.padding.top + child_node.unrounded_layout.border.top;
            let first = child_node
                .table_first_baseline
                .or_else(|| first_cell_baseline(doc, child, top));
            if let Some(first) = first {
                return Some(child_node.unrounded_layout.location.y + first);
            }
        }
        return Some(content_top + baseline);
    }
    let mut stack = Vec::new();
    for &child in node.children.iter().rev() {
        stack.push((child, doc.nodes[child].unrounded_layout.location.y));
    }
    while let Some((id, y)) = stack.pop() {
        let node = doc.ifc_layout_node(id).unwrap();
        if !node.is_in_document()
            || node.is_display_none()
            || node.taffy_position() == taffy::Position::Absolute
            || doc.table_style(id).float() != taffy::Float::None
        {
            continue;
        }
        if matches!(
            node.display(),
            DisplayValue::Table | DisplayValue::InlineTable
        ) && !node.is_ifc_root()
        {
            if let Some(baseline) = node.table_first_baseline {
                return Some(y + baseline);
            }
            continue;
        }
        if let Some(lines) = node.ifc.as_ref().and_then(|ifc| ifc.lines.as_ref())
            && let Some(baseline) = super::ifc::flow::first_baseline(lines)
        {
            return Some(
                y + node.unrounded_layout.padding.top + node.unrounded_layout.border.top + baseline,
            );
        }
        for &child in node.children.iter().rev() {
            stack.push((child, y + doc.nodes[child].unrounded_layout.location.y));
        }
    }
    None
}

fn resolve_row_heights(
    doc: &mut LayoutSession<'_>,
    grid: &mut TableGrid,
    column_widths: &[f32],
    spacing: (f32, f32),
) -> Result<Vec<f32>, raikiri_traits::LayoutError> {
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
        let h = doc.table_style(row_id).size().height;
        if let Some(height) = resolve_dimension(h, None) {
            row_heights[r] = f32_max_compat(row_heights[r], height);
        }
    }
    for cell in &mut grid.cells {
        let columns = cell_column_range(cell, column_widths.len())?;
        let cell_width = spanned_size(&column_widths[columns], spacing.0);
        let h = measure_cell(doc, cell, cell_width);
        let rs = cell.row_span as usize;
        if rs == 1 {
            row_heights[cell.row as usize] = f32_max_compat(row_heights[cell.row as usize], h);
        } else {
            let start = cell.row as usize;
            let end_row = (start + rs).min(row_heights.len());
            let cnt = (end_row - start) as f32;
            if cnt > 0.0 {
                // The first spanned row must contain the cell's baseline
                // before the remaining span height is distributed (CSS 2.2 17.5.3).
                if cell_baseline_aligned(
                    doc.ifc_layout_node(cell.node_id)
                        .expect("table cell layout view")
                        .table_vertical_align,
                ) {
                    row_heights[start] = row_heights[start].max(cell.baseline);
                }
                // Need to ensure span can accommodate h: if sum current < h, distribute deficit
                let cur = spanned_size(&row_heights[start..end_row], spacing.1);
                if h > cur {
                    let add = (h - cur) / cnt;
                    for slot in row_heights[start..end_row].iter_mut() {
                        *slot += add;
                    }
                }
            }
        }
    }
    // Baseline-aligned cells can require more height than any individual
    // cell: the maximum ascent and descent need not belong to the same cell.
    let mut row_baselines = vec![0.0f32; grid.rows.len()];
    for cell in &grid.cells {
        if cell_baseline_aligned(
            doc.ifc_layout_node(cell.node_id)
                .expect("table cell layout view")
                .table_vertical_align,
        ) {
            let row = cell.row as usize;
            row_baselines[row] = row_baselines[row].max(cell.baseline);
        }
    }
    for cell in &grid.cells {
        if cell.row_span == 1
            && cell_baseline_aligned(
                doc.ifc_layout_node(cell.node_id)
                    .expect("table cell layout view")
                    .table_vertical_align,
            )
        {
            let row = cell.row as usize;
            row_heights[row] =
                row_heights[row].max(row_baselines[row] + cell.natural_height - cell.baseline);
        }
    }
    Ok(row_heights)
}

/// Lay `cell` out at `cell_width` with no definite height, and record what the
/// rows and the final placement need of it: the lines it can keep, its height
/// and its baseline. Returns the height it asks of its rows.
fn measure_cell(doc: &mut LayoutSession<'_>, cell: &mut CellPlacement, cell_width: f32) -> f32 {
    // Cell height floors the row rather than stretching the content used
    // for alignment. Preserve ordinary block border and margin sizing.
    let authored_height = doc.table_style(cell.node_id).size().height;
    let measure = |doc: &mut LayoutSession<'_>| {
        #[cfg(test)]
        tests::cell_scope_tests::before_measure(doc, cell.node_id);
        doc.compute_child_layout(
            NodeId::from(cell.node_id),
            LayoutInput {
                run_mode: RunMode::PerformLayout,
                sizing_mode: SizingMode::InherentSize,
                axis: RequestedAxis::Horizontal,
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
                known_dimensions_are_definite: Size {
                    width: true,
                    height: true,
                },
                vertical_margins_are_collapsible: Line::FALSE,
            },
        )
    };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if authored_height.is_auto() {
            measure(doc)
        } else {
            let key = doc.nodes.box_key(cell.node_id).expect("cell has a box key");
            let entry = raikiri_layout_style::OwnedPropertyOverride::length(
                raikiri_layout_style::LengthProperty::Height,
                raikiri_style::ComputedLengthPercentageOrAuto::Auto,
                false,
            )
            .expect("physical cell height");
            doc.with_overrides(vec![(key, entry)], |doc| {
                doc.table_layout_node_mut(cell.node_id).cache.clear();
                measure(doc)
            })
        }
    }));
    if !authored_height.is_auto() {
        doc.table_layout_node_mut(cell.node_id).cache.clear();
    }
    let output = match result {
        Ok(output) => output,
        Err(payload) => std::panic::resume_unwind(payload),
    };
    cell.row_pass = (authored_height.is_auto() && row_pass_is_final(doc, cell.node_id)).then_some(
        RowPassLayout {
            width: cell_width,
            scrollable_overflow_rect: output.scrollable_overflow_rect,
        },
    );
    // Distribute rowspan height across spanned rows (simple: equally)
    cell.natural_height = output.size.height;
    let style = doc.table_style(cell.node_id);
    let padding = style
        .padding()
        .resolve_or_zero(Some(cell_width), crate::layout::dispatch::resolve_calc);
    let border = style
        .border()
        .resolve_or_zero(Some(cell_width), crate::layout::dispatch::resolve_calc);
    let authored_height = resolve_dimension(style.size().height, None).map_or(0.0, |height| {
        if style.box_sizing() == taffy::BoxSizing::ContentBox {
            height + padding.top + padding.bottom + border.top + border.bottom
        } else {
            height
        }
    });
    let h = output.size.height.max(authored_height);
    let baseline = first_cell_baseline(doc, cell.node_id, padding.top + border.top);
    cell.baseline =
        baseline.unwrap_or((output.size.height - padding.bottom - border.bottom).max(0.0));
    h
}

// ---------------------------------------------------------------------------
// Positioning
// ---------------------------------------------------------------------------

/// The size a cell spanning `tracks` covers in the separated borders
/// model: the tracks plus the `spacing` gaps between them.
fn spanned_size(tracks: &[f32], spacing: f32) -> f32 {
    tracks.iter().sum::<f32>() + spacing * tracks.len().saturating_sub(1) as f32
}

/// Cumulative track origins from sizes minus collapsed-line overlaps.
///
/// `origins.len() == sizes.len() + 1`, `origins[0] == origin`, and
/// `origins[i + 1] - origins[i] == sizes[i] - overlaps[i]` where
/// `overlaps[i]` is the collapsed line between track `i` and `i + 1`.
/// The separated borders model passes the negated `border-spacing` instead,
/// so adjoining tracks move apart by the gap. In the collapsing model a
/// cell spanning tracks `i..j` covers `origins[j] - origins[i]` — interior
/// collapsed lines are absorbed into the spanning cell rather than
/// double-counted.
fn track_origins(sizes: &[f32], overlaps: &[f32], origin: f32) -> Vec<f32> {
    let mut out = vec![0.0f32; sizes.len() + 1];
    out[0] = origin;
    for i in 0..sizes.len() {
        let ov = overlaps.get(i).copied().unwrap_or(0.0);
        out[i + 1] = out[i] + sizes[i] - ov;
    }
    out
}

/// Whether a cell's row-height layout is already its final content layout.
///
/// A horizontal inline formatting context with no boxes of its own (floats,
/// atomic inlines, positioned children) breaks its lines at its width alone:
/// the definite row height only changes the box, which the placement sets
/// directly. Block content can depend on the height (percentage heights), so
/// it is laid out again.
fn row_pass_is_final(doc: &Document, cell_id: usize) -> bool {
    is_plain_paragraph_cell(doc, cell_id)
        && doc
            .ifc_layout_node(cell_id)
            .and_then(|node| node.ifc.as_ref())
            .is_some_and(|root| root.lines.is_some())
}

/// Whether a cell's content is one horizontal paragraph with no boxes of its
/// own, whose lines depend on nothing but the cell's width.
fn is_plain_paragraph_cell(doc: &Document, cell_id: usize) -> bool {
    let node = doc
        .ifc_layout_node(cell_id)
        .expect("table cell layout view");
    node.is_ifc_root()
        && node.multicol.is_none()
        && node.ifc.as_ref().is_some_and(|root| {
            root.writing_mode == shodo::geometry::WritingMode::HorizontalTb
                && root.boxes.is_empty()
                && root.generated_blocks.is_empty()
        })
}

struct CellPlacementTracks<'a> {
    column_widths: &'a [f32],
    column_gaps: &'a [f32],
    collapsed_columns: &'a [bool],
    row_heights: &'a [f32],
    row_gaps: &'a [f32],
    collapsed_rows: &'a [bool],
    column_origins: &'a [f32],
    row_origins: &'a [f32],
    row_spacing: f32,
    vertical_writing: bool,
}

fn place_cells(
    doc: &mut LayoutSession<'_>,
    grid: &mut TableGrid,
    tracks: CellPlacementTracks<'_>,
    oof_candidates: &mut OofCandidates,
) -> Result<(), raikiri_traits::LayoutError> {
    let CellPlacementTracks {
        column_widths,
        column_gaps: col_gaps,
        collapsed_columns: collapsed_cols,
        row_heights,
        row_gaps,
        collapsed_rows,
        column_origins: col_x,
        row_origins: row_y,
        row_spacing,
        vertical_writing,
    } = tracks;
    // A cell moved past collapsed rows aligns with the row it now starts in.
    let mut row_baselines = vec![0.0f32; row_heights.len()];
    for cell in &grid.cells {
        if cell_baseline_aligned(
            doc.ifc_layout_node(cell.node_id)
                .expect("table cell layout view")
                .table_vertical_align,
        ) {
            let row = first_shown_row(cell, collapsed_rows);
            row_baselines[row] = row_baselines[row].max(cell.baseline);
        }
    }
    for (order, cell) in grid.cells.iter_mut().enumerate() {
        let columns = cell_column_range(cell, column_widths.len())?;
        let span_end = (cell.row as usize + cell.row_span as usize).min(row_heights.len());
        // A cell spanning collapsed rows shrinks to the rows that remain and
        // clips its contents; with none left it has no block extent (CSS
        // Tables 3, "Visibility-collapse cell rendering").
        let mut kept = (cell.row as usize..span_end).filter(|&row| !collapsed_rows[row]);
        let first_kept = kept.next();
        let (start_row, end_row) = match first_kept {
            Some(first) => (first, kept.next_back().unwrap_or(first) + 1),
            None => (cell.row as usize, cell.row as usize),
        };
        let spans_collapsed_row = collapsed_rows[cell.row as usize..span_end]
            .iter()
            .any(|collapsed| *collapsed);
        // The same holds across columns: the cell keeps the columns that
        // remain, and none of its width when they all collapse.
        let mut shown_columns = columns.clone().filter(|&column| !collapsed_cols[column]);
        let first_shown_column = shown_columns.next();
        let (start_col, end_col) = match first_shown_column {
            Some(first) => (first, shown_columns.next_back().unwrap_or(first) + 1),
            None => (columns.start, columns.start),
        };
        let spans_collapsed_column = collapsed_cols[columns.clone()]
            .iter()
            .any(|collapsed| *collapsed);
        doc.nodes[cell.node_id].clips_collapsed_table_span =
            spans_collapsed_row || spans_collapsed_column;
        let cell_x = *col_x.get(start_col).ok_or_else(table_column_error)?;
        let cell_y = row_y[start_row];
        // Full spanned widths — origins already net out the collapsed
        // overlaps for *positioning*; shrinking the box by the overlap as
        // well would double-count the absorbed line. Separated borders add
        // the spanned gaps instead, matching the origin differences.
        let cell_width = column_widths[start_col..end_col].iter().sum::<f32>()
            + col_gaps[start_col..end_col.saturating_sub(1).max(start_col)]
                .iter()
                .sum::<f32>();
        let cell_height = row_heights[start_row..end_row].iter().sum::<f32>()
            + row_gaps[start_row..end_row.saturating_sub(1).max(start_row)]
                .iter()
                .filter(|gap| **gap != 0.0)
                .count() as f32
                * row_spacing;
        // Final layout for cell contents with definite size. A cell whose
        // content does not depend on its height keeps its row-pass lines.
        let reused = cell
            .row_pass
            .filter(|pass| pass.width == cell_width)
            .map(|pass| pass.scrollable_overflow_rect);
        let (scrollable_overflow_rect, mut child_candidates) = if let Some(rect) = reused {
            (rect, OofCandidates::new())
        } else {
            let output = doc.compute_child_layout(
                NodeId::from(cell.node_id),
                LayoutInput {
                    run_mode: RunMode::PerformLayout,
                    sizing_mode: SizingMode::InherentSize,
                    axis: RequestedAxis::Horizontal,
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
                    known_dimensions_are_definite: Size {
                        width: true,
                        height: true,
                    },
                    vertical_margins_are_collapsible: Line::FALSE,
                },
            );
            (output.scrollable_overflow_rect, output.oof_candidates)
        };
        child_candidates.translate(Point {
            x: cell_x,
            y: cell_y,
        });
        oof_candidates.append(&mut child_candidates);
        // A cell laid out by the inline engine has no child layouts: its
        // lines are drawn from its content box, which its own border and
        // padding (as the engine measured them) place inside the cell.
        let (mut padding, mut border) = {
            let style = doc.table_style(cell.node_id);
            (
                style
                    .padding()
                    .resolve_or_zero(Some(cell_width), crate::layout::dispatch::resolve_calc),
                style
                    .border()
                    .resolve_or_zero(Some(cell_width), crate::layout::dispatch::resolve_calc),
            )
        };
        let vertical_cell = vertical_writing && table_writing_mode(doc, cell.node_id).is_vertical();
        let collapsed_empty = first_kept.is_none()
            || cell_height == 0.0
                && doc
                    .ifc_layout_node(cell.node_id)
                    .expect("table cell layout view")
                    .hides_empty_table_cell;
        if collapsed_empty {
            // Hidden empty rows have no block extent, including the cell's
            // otherwise retained padding/border. Keep the zero content box
            // valid without resetting its positioned out-of-flow subtree.
            padding.top = 0.0;
            padding.bottom = 0.0;
            border.top = 0.0;
            border.bottom = 0.0;
        }
        if first_shown_column.is_none() {
            // Likewise a cell in collapsed columns has no inline extent.
            padding.left = 0.0;
            padding.right = 0.0;
            border.left = 0.0;
            border.right = 0.0;
        }
        let extra = if vertical_cell {
            0.0
        } else {
            (cell_height - cell.natural_height).max(0.0)
        };
        let shift = match doc
            .ifc_layout_node(cell.node_id)
            .expect("table cell layout view")
            .table_vertical_align
        {
            _ if collapsed_empty => 0.0,
            VerticalAlign::Top => 0.0,
            VerticalAlign::Middle => extra / 2.0,
            VerticalAlign::Bottom => extra,
            _ if !vertical_cell => (row_baselines[start_row] - cell.baseline).max(0.0),
            _ => 0.0,
        };
        padding.top += shift;
        padding.bottom += (extra - shift).max(0.0);
        for child in doc
            .ifc_layout_node(cell.node_id)
            .expect("table cell layout view")
            .children
            .clone()
        {
            if doc.nodes[child].taffy_position() != taffy::Position::Absolute {
                let mut child_layout = doc.nodes[child].unrounded_layout;
                child_layout.location.y += shift;
                doc.store_unrounded_layout(child, &child_layout);
            }
        }
        let layout = BoxLayout {
            order: order as u32,
            location: Point {
                x: cell_x,
                y: cell_y,
            },
            size: Size {
                width: cell_width,
                height: cell_height,
            },
            scrollable_overflow_rect,
            scrollbar_size: Size::ZERO,
            padding,
            border,
            margin: Edges::ZERO,
        };
        cell.resolved = Some(layout);
        doc.store_unrounded_layout(cell.node_id, &layout);
        if doc.is_anonymous_cell(cell.node_id) {
            let root = doc
                .ifc_layout_node(cell.node_id)
                .expect("anonymous cell layout view");
            let children = if root.is_ifc_root() {
                root.ifc_boxes()
            } else {
                root.children.clone()
            };
            for child in children {
                doc.nodes[child].unrounded_layout.location.x += cell_x;
                doc.nodes[child].unrounded_layout.location.y += cell_y;
            }
        }
    }
    Ok(())
}

/// Re-map the already measured table grid from horizontal physical axes to
/// vertical writing-mode axes. Vertical cells are re-laid out at their final
/// physical dimensions and aligned on their block axis. Orthogonal horizontal
/// cells retain the normal pass's content layout.
fn reposition_cells_for_vertical_writing(
    doc: &mut LayoutSession<'_>,
    grid: &mut TableGrid,
    row_heights: &[f32],
    cell_width: f32,
    origin: Point<f32>,
    spacing: f32,
) {
    let vertical_track = row_heights.iter().sum::<f32>();
    if !vertical_track.is_finite() || vertical_track <= 0.0 {
        return; // cov:ignore: defensive empty/invalid vertical track fallback.
    }
    for cell in &mut grid.cells {
        let col_start = cell.col_start as usize;
        let col_end = col_start
            .saturating_add(cell.col_span as usize)
            .min(usize::from(grid.n_cols));
        let col_span = col_end.saturating_sub(col_start).max(1);
        let y = origin.y + (vertical_track + spacing) * col_start as f32;
        let height = vertical_track * col_span as f32 + spacing * (col_span - 1) as f32;
        let Some(layout) = cell.resolved.as_mut() else {
            continue; // cov:ignore: defensive unresolved-cell fallback.
        };
        layout.location = Point { x: origin.x, y };
        layout.size.width = cell_width;
        layout.size.height = height;
        let cell_writing_mode = table_writing_mode(doc, cell.node_id);
        if !cell_writing_mode.is_vertical() {
            // A horizontal cell in a vertical table still aligns content on
            // its own block axis. Preserve the normal pass's vertical shift.
            doc.store_unrounded_layout(cell.node_id, layout);
            continue;
        }
        // Re-layout at the final physical dimensions before distributing
        // block-axis free space; the earlier horizontal measurement cannot
        // determine vertical content's occupied block extent.
        doc.compute_child_layout(
            NodeId::from(cell.node_id),
            LayoutInput {
                run_mode: RunMode::PerformLayout,
                sizing_mode: SizingMode::InherentSize,
                axis: RequestedAxis::Both,
                known_dimensions: Size {
                    width: Some(cell_width),
                    height: Some(height),
                },
                parent_size: Size {
                    width: Some(cell_width),
                    height: Some(height),
                },
                available_space: Size {
                    width: AvailableSpace::Definite(cell_width),
                    height: AvailableSpace::Definite(height),
                },
                known_dimensions_are_definite: Size {
                    width: true,
                    height: true,
                },
                vertical_margins_are_collapsible: Line::FALSE,
            },
        );
        let node = doc
            .ifc_layout_node(cell.node_id)
            .expect("table cell layout view");
        let style = doc.table_style(cell.node_id);
        let mut padding = style
            .padding()
            .resolve_or_zero(Some(cell_width), crate::layout::dispatch::resolve_calc);
        let border = style
            .border()
            .resolve_or_zero(Some(cell_width), crate::layout::dispatch::resolve_calc);
        let scrollbar_width = if style.overflow().y == taffy::Overflow::Scroll {
            style.scrollbar_width()
        } else {
            0.0
        };
        let direction = style.direction();
        let lines = node.ifc.as_ref().and_then(|ifc| ifc.lines.as_ref());
        let has_lines = lines.is_some_and(|lines| !lines.lines.is_empty());
        let natural = if has_lines {
            lines.map_or(0.0, |lines| lines.height)
                + padding.left
                + padding.right
                + border.left
                + border.right
        } else {
            node.children
                .iter()
                .filter(|&&child| doc.nodes[child].taffy_position() != taffy::Position::Absolute)
                .map(|&child| {
                    let child_style = doc.table_style(child);
                    // Relative insets move ink after normal flow; they must
                    // not consume the cell's alignment space. Taffy leaves
                    // floated boxes at their normal positions, so their
                    // paint-side offsets are not removed here.
                    let offset = if child_style.float() == taffy::Float::None {
                        let inner_width = cell_width
                            - padding.left
                            - padding.right
                            - border.left
                            - border.right
                            - scrollbar_width;
                        let inset = child_style.inset();
                        let left = inset.left.maybe_resolve(
                            Some(inner_width),
                            crate::layout::dispatch::resolve_calc,
                        );
                        let right = inset.right.maybe_resolve(
                            Some(inner_width),
                            crate::layout::dispatch::resolve_calc,
                        );
                        if direction == taffy::Direction::Rtl {
                            right.map(|right| -right).or(left).unwrap_or(0.0)
                        } else {
                            left.or(right.map(|right| -right)).unwrap_or(0.0)
                        }
                    } else {
                        0.0
                    };
                    let child = &doc.ifc_layout_node(child).unwrap().unrounded_layout;
                    child.location.x - offset + child.size.width + child.margin.right
                })
                .fold(padding.left + border.left, f32::max)
                + padding.right
                + border.right
        };
        let extra = (cell_width - natural).max(0.0);
        let shift = match node.table_vertical_align {
            VerticalAlign::Middle => extra / 2.0,
            VerticalAlign::Bottom => extra,
            _ => 0.0,
        };
        // Baseline-aligned block content retains its normal-flow placement;
        // without an inline baseline, treating it as block-start alignment
        // would move it across the entire physical cell in vertical-rl.
        let rtl = matches!(
            cell_writing_mode,
            WritingMode::VerticalRl | WritingMode::SidewaysRl
        ) && (has_lines || !cell_baseline_aligned(node.table_vertical_align));
        let child_shift = if rtl {
            if has_lines { -shift } else { extra - shift }
        } else {
            shift
        };
        if rtl {
            padding.right += shift;
            padding.left += extra - shift;
        } else {
            padding.left += shift;
            padding.right += extra - shift;
        }
        for child in doc
            .ifc_layout_node(cell.node_id)
            .expect("table cell layout view")
            .children
            .clone()
        {
            if doc.nodes[child].taffy_position() != taffy::Position::Absolute {
                let mut layout = doc.nodes[child].unrounded_layout;
                layout.location.x += child_shift;
                doc.store_unrounded_layout(child, &layout);
            }
        }
        layout.padding = padding;
        layout.border = border;
        doc.store_unrounded_layout(cell.node_id, layout);
    }
}

// ---------------------------------------------------------------------------
// Helpers for style resolution
// ---------------------------------------------------------------------------

fn resolve_dimension(d: Dimension, basis: Option<f32>) -> Option<f32> {
    super::resolve_dimension(d, basis)
}

fn resolve_length(lp: taffy::style::LengthPercentage, basis: Option<f32>) -> f32 {
    super::resolve_length_percentage(lp, basis).unwrap_or(0.0)
}

/// Resolve a table's padding or border widths (`rect`) against the width of
/// its containing block (CSS 2.1 §8.4).
fn resolve_table_rect(
    rect: &StyleRect<taffy::style::LengthPercentage>,
    parent_size: Size<Option<f32>>,
) -> Edges<f32> {
    let w_basis = parent_size.width;
    Edges {
        left: resolve_length(rect.left, w_basis),
        right: resolve_length(rect.right, w_basis),
        top: resolve_length(rect.top, w_basis),
        bottom: resolve_length(rect.bottom, w_basis),
    }
}

// ---------------------------------------------------------------------------
// Fixed layout (CSS 2.1 §17.5.2.1)
// ---------------------------------------------------------------------------

/// The width a first-row cell or a `<col>` gives its column in fixed table
/// layout, against the table width `avail`; `None` leaves the column auto.
fn fixed_column_width(width: Dimension, avail: f32) -> Option<f32> {
    // spec: https://drafts.csswg.org/css-tables-3/#distributing-width-to-columns
    // spec: https://github.com/w3c/csswg-drafts/issues/94
    // A `calc()` that sums a percentage and a length counts as `auto`, even
    // in fixed table layout. The style layer folds a `calc()` with only a
    // percentage or only a length into that percentage or length, so every
    // `calc()` left here is such a sum.
    if width.into_raw().is_plain_calc() {
        return None;
    }
    resolve_dimension(width, Some(avail))
}

/// Fixed column sizing: no content measurement.
///
/// `avail` is the table width net of padding + outer (collapsed) borders.
/// Per §17.5.2.1, a first-row cell with `colspan == 1` and a specified
/// `width` (length or percentage-of-table-width) fixes its column; all
/// remaining columns divide the leftover equally. A `<col>` width fixes a
/// column that no such cell fixed. Specified widths in non-first rows and
/// `colspan > 1` first-row widths are ignored. Leftover underflow clamps unfixed
/// columns to zero (the table overflows rather than shrinking fixed
/// columns).
fn resolve_fixed_column_widths(grid: &TableGrid, avail: f32) -> Vec<f32> {
    let n = grid.n_cols as usize;
    let mut fixed: Vec<Option<f32>> = vec![None; n];
    // A specified `<col>` width takes priority over first-row cell widths
    // (CSS 2.1 §17.5.2.1).
    // Percentages resolve against the same avail basis as cell percentages.
    for (c, sz) in grid.col_widths.iter().enumerate() {
        if c >= n {
            continue;
        }
        let minimum = resolve_dimension(sz.min_width, None);
        let maximum = resolve_dimension(sz.max_width, None);
        if let Some(mut x) = fixed_column_width(sz.width, avail) {
            if let Some(minimum) = minimum {
                x = f32_max_compat(x, minimum);
            }
            if let Some(maximum) = maximum
                && !minimum.is_some_and(|minimum| minimum > maximum)
            {
                x = x.min(maximum);
            }
            fixed[c] = Some(x);
        }
    }
    // First-row cells set widths only for columns without a specified `<col>` width.
    for cell in &grid.cells {
        if cell.row != 0 || cell.col_span != 1 {
            continue;
        }
        let c = cell.col_start as usize;
        if c < n && fixed[c].is_none() {
            fixed[c] = fixed_column_width(cell.specified_width, avail);
        }
    }
    // A `<col>` minimum fixes an otherwise unspecified column.
    for (c, sz) in grid.col_widths.iter().enumerate() {
        if c >= n || fixed[c].is_some() {
            continue;
        }
        let minimum = resolve_dimension(sz.min_width, None);
        let maximum = resolve_dimension(sz.max_width, None);
        if let Some(mut x) = minimum {
            if let Some(maximum) = maximum
                && !minimum.is_some_and(|minimum| minimum > maximum)
            {
                x = x.min(maximum);
            }
            fixed[c] = Some(x);
        }
    }
    let mut floors = vec![0.0_f32; n];
    for cell in &grid.cells {
        if cell.row == 0 && cell.col_span == 1 {
            let c = usize::from(cell.col_start);
            if c < n {
                floors[c] = floors[c]
                    .max(resolve_dimension(cell.specified_min_width, Some(avail)).unwrap_or(0.0));
            }
        }
    }
    for (column, floor) in fixed.iter_mut().zip(&floors) {
        if let Some(width) = column {
            *width = width.max(*floor);
        }
    }
    let fixed_sum: f32 = fixed.iter().flatten().sum();
    let mut remaining = (avail - fixed_sum).max(0.0);
    let reclaimable: Vec<_> = fixed.iter().map(Option::is_none).collect();
    let mut automatic: Vec<_> = (0..n).filter(|&c| reclaimable[c]).collect();
    // A minimum does not make an auto column fixed. Allocate the largest
    // floors first, then divide the remaining space among the other columns.
    automatic.sort_by(|&a, &b| floors[b].total_cmp(&floors[a]));
    let mut count = automatic.len();
    for &column in &automatic {
        let share = (remaining / count as f32).max(0.0);
        let width = floors[column].max(share);
        fixed[column] = Some(width);
        remaining = (remaining - width).max(0.0);
        count -= 1;
    }
    let mut widths: Vec<_> = fixed
        .into_iter()
        .map(|width| width.unwrap_or(0.0))
        .collect();
    // The first row's spanning cells, as their columns and minimum width.
    let first_row_spans: Vec<(usize, usize, f32)> = grid
        .cells
        .iter()
        .filter(|cell| cell.row == 0 && cell.col_span > 1)
        .filter_map(|cell| {
            let start = usize::from(cell.col_start);
            let end = (start + usize::from(cell.col_span)).min(n);
            let minimum = resolve_dimension(cell.specified_min_width, Some(avail)).unwrap_or(0.0);
            (start < end).then_some((start, end, minimum))
        })
        .collect();
    // First-row spans are disjoint. Reserve each other span's minimum
    // proportionally in its auto columns before any span draws from them.
    for &(start, end, minimum) in &first_row_spans {
        let fixed_sum: f32 = (start..end)
            .filter(|&c| !reclaimable[c])
            .map(|c| widths[c])
            .sum();
        let floor_sum: f32 = (start..end)
            .filter(|&c| reclaimable[c])
            .map(|c| floors[c])
            .sum();
        let spare: f32 = (start..end)
            .filter(|&c| reclaimable[c])
            .map(|c| (widths[c] - floors[c]).max(0.0))
            .sum();
        let reserve = (minimum - fixed_sum - floor_sum).max(0.0).min(spare);
        if spare > 0.0 {
            for c in start..end {
                if reclaimable[c] {
                    floors[c] += reserve * (widths[c] - floors[c]).max(0.0) / spare;
                }
            }
        }
    }
    for &(start, end, minimum) in &first_row_spans {
        let current: f32 = widths[start..end].iter().sum();
        if minimum > current {
            let deficit = minimum - current;
            let donor_space: f32 = (0..n)
                .filter(|&c| (c < start || c >= end) && reclaimable[c])
                .map(|c| (widths[c] - floors[c]).max(0.0))
                .sum();
            let reclaimed = deficit.min(donor_space);
            if donor_space > 0.0 {
                for c in 0..n {
                    if (c < start || c >= end) && reclaimable[c] {
                        widths[c] -= reclaimed * (widths[c] - floors[c]).max(0.0) / donor_space;
                    }
                }
            }
            for c in start..end {
                widths[c] += deficit / (end - start) as f32;
                // Subsequent spans must not undo this satisfied minimum.
                floors[c] = floors[c].max(widths[c]);
            }
        }
    }
    widths
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
    /// The inline-axis width the collapsed borders take from the table: the
    /// outer left and right borders, less the overlaps between columns.
    fn inline_border_width(&self) -> f32 {
        self.outer_left + self.outer_right - self.col_overlaps.iter().sum::<f32>()
    }

    fn separate(table_border: &Edges<f32>) -> Self {
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

/// The cell covering grid position (`row`, `col`), following spans.
fn cell_at<'a>(
    grid: &'a TableGrid,
    index: &CellIndex,
    row: u16,
    col: u16,
) -> Option<(usize, &'a CellPlacement)> {
    index.at(row, col).map(|cell| (cell, &grid.cells[cell]))
}

#[derive(Clone, Copy)]
enum CellBorderSide {
    Top,
    Right,
    Bottom,
    Left,
}

#[derive(Clone, Copy)]
struct CollapsedBorderCandidate {
    border: ComputedBorder,
    taffy_width: LengthPercentage,
}

fn border_candidate(
    doc: &LayoutSession<'_>,
    node_id: usize,
    side: CellBorderSide,
) -> Option<CollapsedBorderCandidate> {
    // An earlier resolution step may already have replaced a side.
    let node = &doc.ifc_layout_node(node_id).expect("table box layout view");
    let borders = node
        .collapsed_border
        .as_ref()
        .or(node.computed_border.as_ref())?;
    let widths = doc.table_style(node_id).border();
    let (border, taffy_width) = match side {
        CellBorderSide::Top => (borders.top, widths.top),
        CellBorderSide::Right => (borders.right, widths.right),
        CellBorderSide::Bottom => (borders.bottom, widths.bottom),
        CellBorderSide::Left => (borders.left, widths.left),
    };
    Some(CollapsedBorderCandidate {
        border,
        taffy_width,
    })
}

fn border_style_rank(style: BorderStyle) -> u8 {
    match style {
        BorderStyle::Double => 8,
        BorderStyle::Solid => 7,
        BorderStyle::Dashed => 6,
        BorderStyle::Dotted => 5,
        BorderStyle::Ridge => 4,
        BorderStyle::Outset => 3,
        BorderStyle::Groove => 2,
        BorderStyle::Inset => 1,
        BorderStyle::None => 0,
        BorderStyle::Hidden => 9,
        _ => 0, // cov:ignore: BorderStyle is non-exhaustive and has no other current variants.
    }
}

fn collapsed_border_winner(
    leading: CollapsedBorderCandidate,
    trailing: CollapsedBorderCandidate,
) -> CollapsedBorderCandidate {
    let leading_style = leading.border.style();
    let trailing_style = trailing.border.style();
    if leading_style == BorderStyle::Hidden {
        return leading;
    }
    if trailing_style == BorderStyle::Hidden {
        return trailing;
    }
    if leading_style == BorderStyle::None {
        return trailing;
    }
    if trailing_style == BorderStyle::None {
        return leading;
    }

    let width_order = leading
        .border
        .width()
        .px()
        .total_cmp(&trailing.border.width().px());
    if width_order.is_gt() {
        return leading;
    }
    if width_order.is_lt() {
        return trailing;
    }

    if border_style_rank(leading_style) >= border_style_rank(trailing_style) {
        leading
    } else {
        trailing
    }
}

/// The side of `sides` facing `side`.
fn border_side(sides: &mut Sides<ComputedBorder>, side: CellBorderSide) -> &mut ComputedBorder {
    match side {
        CellBorderSide::Top => &mut sides.top,
        CellBorderSide::Right => &mut sides.right,
        CellBorderSide::Bottom => &mut sides.bottom,
        CellBorderSide::Left => &mut sides.left,
    }
}

/// Resolve every shared border between adjacent cells. Each segment's winner
/// is decided from the cells' borders before any segment is resolved, so a
/// cell spanning several neighbours does not carry one segment's winner into
/// the next. A spanning cell paints the strongest winner along its side;
/// each neighbour paints its own segment's winner.
fn resolve_collapsed_cell_borders(
    doc: &LayoutSession<'_>,
    state: &mut BorderState,
    grid: &TableGrid,
    index: &CellIndex,
) {
    const SIDES: [CellBorderSide; 4] = [
        CellBorderSide::Top,
        CellBorderSide::Right,
        CellBorderSide::Bottom,
        CellBorderSide::Left,
    ];
    let before: std::collections::HashMap<usize, [Option<CollapsedBorderCandidate>; 4]> = grid
        .cells
        .iter()
        .map(|cell| {
            (
                cell.node_id,
                SIDES.map(|side| state.candidate(doc, cell.node_id, side)),
            )
        })
        .collect();
    let resolve = |state: &mut BorderState,
                   leading_id: usize,
                   leading_side: CellBorderSide,
                   trailing_id: usize,
                   trailing_side: CellBorderSide| {
        let side_of = |node_id: usize, side: CellBorderSide| {
            before.get(&node_id).and_then(|sides| sides[side as usize])
        };
        let (Some(leading), Some(trailing)) = (
            side_of(leading_id, leading_side),
            side_of(trailing_id, trailing_side),
        ) else {
            return;
        };
        let winner = collapsed_border_winner(leading, trailing);
        keep_stronger_side(doc, state, leading_id, leading_side, winner);
        keep_stronger_side(doc, state, trailing_id, trailing_side, winner);
    };

    let column_count = grid.n_cols as usize;
    for boundary in 1..column_count {
        for row in 0..grid.rows.len() {
            let row = row as u16;
            let left = cell_at(grid, index, row, (boundary - 1) as u16)
                .filter(|(_, cell)| cell.col_start + cell.col_span == boundary as u16);
            let right = cell_at(grid, index, row, boundary as u16)
                .filter(|(_, cell)| cell.col_start == boundary as u16);
            if let (Some((_, left)), Some((_, right))) = (left, right) {
                resolve(
                    state,
                    left.node_id,
                    CellBorderSide::Right,
                    right.node_id,
                    CellBorderSide::Left,
                );
            }
        }
    }

    for boundary in 1..grid.rows.len() {
        for column in 0..column_count {
            let boundary = boundary as u16;
            let column = column as u16;
            let above = cell_at(grid, index, boundary - 1, column)
                .filter(|(_, cell)| cell.row.saturating_add(cell.row_span) == boundary);
            let below =
                cell_at(grid, index, boundary, column).filter(|(_, cell)| cell.row == boundary);
            if let (Some((_, above)), Some((_, below))) = (above, below) {
                resolve(
                    state,
                    above.node_id,
                    CellBorderSide::Bottom,
                    below.node_id,
                    CellBorderSide::Top,
                );
            }
        }
    }
}

/// Give `node_id`'s `side` the stronger of the border it already paints and
/// `winner`.
fn keep_stronger_side(
    doc: &LayoutSession<'_>,
    state: &mut BorderState,
    node_id: usize,
    side: CellBorderSide,
    winner: CollapsedBorderCandidate,
) {
    // The pair winner is passed first so it wins ties: a cell's own border
    // must not beat the winner of its segment by equality alone.
    let stronger = match state.candidate(doc, node_id, side) {
        Some(current) => collapsed_border_winner(winner, current),
        None => winner, // cov:ignore: the pair's own candidate came from this side.
    };
    state.set_side(node_id, side, stronger.border, stronger.taffy_width);
}

/// Resolve each cell's sides against the borders of the rows they lie on
/// (CSS 2.1 §17.6.2.1): its top against its first row, its bottom against
/// its last row, and an edge cell's left and right against its row. A cell
/// wins ties over a row. Rows then paint nothing themselves; their borders
/// live on in the cells that took them.
fn resolve_collapsed_row_borders(
    doc: &LayoutSession<'_>,
    state: &mut BorderState,
    grid: &TableGrid,
) {
    let last_col = grid.n_cols;
    for index in 0..grid.cells.len() {
        let cell = &grid.cells[index];
        // An anonymous row is recorded by its first cell, which has no row
        // border of its own to contribute.
        let row_box = |index: usize| {
            grid.rows.get(index).copied().filter(|&row| {
                doc.ifc_layout_node(row)
                    .is_some_and(|node| node.display() == DisplayValue::TableRow)
            })
        };
        let first_row = row_box(usize::from(cell.row));
        let last_row =
            row_box(usize::from(cell.row.saturating_add(cell.row_span)).saturating_sub(1));
        let node_id = cell.node_id;
        let sides = [
            (first_row, CellBorderSide::Top),
            (last_row, CellBorderSide::Bottom),
            (
                first_row.filter(|_| cell.col_start == 0),
                CellBorderSide::Left,
            ),
            (
                first_row.filter(|_| cell.col_start + cell.col_span == last_col),
                CellBorderSide::Right,
            ),
        ];
        for (row, side) in sides {
            let Some(row) = row else {
                continue;
            };
            let (Some(own), Some(row_border)) = (
                state.candidate(doc, node_id, side),
                state.candidate(doc, row, side),
            ) else {
                continue;
            };
            let winner = collapsed_border_winner(own, row_border);
            state.set_side(node_id, side, winner.border, winner.taffy_width);
        }
    }
    for &row in &grid.rows {
        if doc
            .ifc_layout_node(row)
            .is_none_or(|node| node.display() != DisplayValue::TableRow)
        {
            continue;
        }
        state.suppress_row(doc, row);
    }
}

/// Resolve each edge cell's outer side against the table's own border on
/// that side (CSS 2.1 §17.6.2.1). A cell wins ties over the table. When the
/// table's border wins, the table paints that edge, so the cell's side takes
/// no width and paints nothing.
fn resolve_collapsed_table_edges(
    doc: &LayoutSession<'_>,
    state: &mut BorderState,
    grid: &TableGrid,
    table_idx: usize,
) {
    let last_row = grid.rows.len() as u16;
    let last_col = grid.n_cols;
    for index in 0..grid.cells.len() {
        let cell = &grid.cells[index];
        let (node_id, sides) = (
            cell.node_id,
            [
                (cell.row == 0, CellBorderSide::Top),
                (
                    cell.row.saturating_add(cell.row_span) >= last_row,
                    CellBorderSide::Bottom,
                ),
                (cell.col_start == 0, CellBorderSide::Left),
                (
                    cell.col_start + cell.col_span == last_col,
                    CellBorderSide::Right,
                ),
            ],
        );
        for (on_edge, side) in sides {
            if !on_edge {
                continue;
            }
            let (Some(own), Some(table)) = (
                state.candidate(doc, node_id, side),
                state.candidate(doc, table_idx, side),
            ) else {
                continue;
            };
            let winner = collapsed_border_winner(own, table);
            if winner.border == own.border && winner.taffy_width == own.taffy_width {
                continue;
            }
            state.set_side(
                node_id,
                side,
                own.border.without_line(),
                LengthPercentage::length(0.0),
            );
        }
    }
}

/// Resolve the borders of `grid` (CSS 2.1 §17.6.2.1): rows against their
/// cells, adjoining cells against each other, and edge cells against the table.
fn resolve_collapsed_borders(
    doc: &LayoutSession<'_>,
    state: &mut BorderState,
    grid: &TableGrid,
    index: &CellIndex,
    table_idx: usize,
) {
    #[cfg(test)]
    BORDER_RESOLUTIONS.with(|count| count.set(count.get() + 1));
    resolve_collapsed_row_borders(doc, state, grid);
    resolve_collapsed_cell_borders(doc, state, grid, index);
    resolve_collapsed_table_edges(doc, state, grid, table_idx);
}

/// The two border states of a table with collapsed columns in the collapsed
/// borders model. The columns that collapse still set the widths the rows are
/// measured at, so the rows are measured with the borders every column in
/// place gives the cells, and the cells are laid out with the borders the
/// columns that remain give them.
#[derive(Debug)]
struct ColumnsInPlace {
    /// [`CollapsedLines::inline_border_width`] with every column in place.
    border_width: f32,
    /// [`CollapsedLines::row_overlaps`] of the cells with the borders `in_place`
    /// gives them, between each pair of consecutive rows that are not
    /// collapsed. The rows keep the heights the cells of a collapsed column
    /// gave them, and those cells share their row lines, which the grid that
    /// remains does not overlap.
    row_overlaps: Vec<f32>,
    /// The borders the cells are measured with: the inline sides they have
    /// with every column in place, and for the cells that remain the block
    /// sides the rest of the layout counts, which are those of the grid that
    /// remains.
    in_place: BorderState,
    /// The borders with the collapsed columns left out.
    remaining: BorderState,
}

impl ColumnsInPlace {
    /// `in_place` and `border_width` are what the grid with every column of
    /// `grid` resolved to ([`border_width_with_columns_in_place`]); `shown`
    /// has been resolved since, so the cells now have the borders of the grid
    /// that remains.
    fn new(
        shown: &TableGrid,
        border_width: f32,
        mut in_place: BorderState,
        remaining: BorderState,
    ) -> Self {
        let remains: std::collections::HashSet<_> =
            shown.cells.iter().map(|cell| cell.node_id).collect();
        for cell in &mut in_place.cells {
            if remains.contains(&cell.node_id) {
                let kept = remaining
                    .cell(cell.node_id)
                    .expect("both states retain every original cell");
                cell.widths.top = kept.widths.top;
                cell.widths.bottom = kept.widths.bottom;
            }
        }
        Self {
            border_width,
            row_overlaps: Vec::new(),
            in_place,
            remaining,
        }
    }

    /// Set the overlaps between the rows that are not in `collapsed_rows`
    /// from the borders the cells are measured with: a cell that remains
    /// keeps the block borders of the grid that remains, so it shares with the
    /// cells of a collapsed column only the line it has itself. Overlapping by
    /// the line the cells would share with every column in place would take
    /// more from a row than the cell was measured with.
    fn with_row_overlaps(
        mut self,
        doc: &Document,
        grid: &TableGrid,
        collapsed_rows: &[bool],
        table_border: &Edges<f32>,
        basis: Option<f32>,
    ) -> Self {
        let rows_shown = without_collapsed_rows(grid, collapsed_rows);
        if rows_shown.rows.is_empty() {
            return self;
        }
        let index = CellIndex::new(&rows_shown);
        self.row_overlaps = compute_collapsed_lines(
            doc,
            &self.in_place,
            &rows_shown,
            &index,
            table_border,
            basis,
        )
        .row_overlaps;
        self
    }

    /// Raise the overlap after each row of `row_overlaps` to the line the cells
    /// of the collapsed columns share there, between the rows in `kept_rows`.
    /// The rows keep the heights those cells gave them, so they keep the lines
    /// the cells share as well.
    fn raise_row_overlaps(&self, kept_rows: &[usize], row_overlaps: &mut [f32]) {
        for (line, rows) in self.row_overlaps.iter().zip(kept_rows.windows(2)) {
            row_overlaps[rows[0]] = row_overlaps[rows[0]].max(*line);
        }
    }

    /// Measure again each cell whose borders differ between the two states,
    /// with the borders it keeps: the measure of the rows broke its lines at
    /// borders it no longer has, so neither its lines, its height nor its
    /// baseline are those it is laid out and aligned by. The rows keep the
    /// heights they were measured at.
    fn measure_changed_cells(
        &self,
        doc: &mut LayoutSession<'_>,
        grid: &mut TableGrid,
        column_widths: &[f32],
        spacing: (f32, f32),
    ) -> Result<(), raikiri_traits::LayoutError> {
        let states = self.in_place.cells.iter().zip(&self.remaining.cells);
        for (cell, (in_place, remaining)) in grid.cells.iter_mut().zip(states) {
            if in_place.widths != remaining.widths {
                let columns = cell_column_range(cell, column_widths.len())?;
                measure_cell(doc, cell, spanned_size(&column_widths[columns], spacing.0));
            }
        }
        Ok(())
    }
}

/// Run `measure` with the borders every column in place gives the cells, when
/// columns collapse, and leave the cells with the borders of the columns that
/// remain.
fn measured_in_place<R>(
    doc: &mut LayoutSession<'_>,
    columns: Option<&ColumnsInPlace>,
    measure: impl FnOnce(&mut LayoutSession<'_>) -> R,
) -> R {
    let Some(columns) = columns else {
        return measure(doc);
    };
    border_scopes::with_temporary_borders(doc, &columns.in_place, measure)
}

#[cfg(test)]
fn snapshot_borders(doc: &LayoutSession<'_>, grid: &TableGrid) -> BorderState {
    BorderState::retained(doc, grid)
}

/// The inline-axis width that collapsed borders take from the table while
/// every column of `grid` is in place ([`CollapsedLines::inline_border_width`]
/// of the grid with those columns), and the borders the cells then have.
/// `collapsed_rows` stay out, as they do when the columns are shown.
///
/// Columns that collapse are still measured at the widths they had before,
/// and those widths depend on the borders they had too, so this resolves
/// the grid with the columns in it. Cells that remain in `shown` are returned
/// to their unresolved borders afterwards, for the caller to resolve `shown`;
/// the cells of the collapsed columns keep the borders they resolved to, which
/// they are measured with.
fn border_width_with_columns_in_place(
    doc: &LayoutSession<'_>,
    state: &mut BorderState,
    grid: &TableGrid,
    collapsed_rows: &[bool],
    shown: &TableGrid,
    table_idx: usize,
    metrics: (&Edges<f32>, Option<f32>),
) -> (f32, BorderState) {
    let (table_border, basis) = metrics;
    let rows_shown = without_collapsed_rows(grid, collapsed_rows);
    if rows_shown.rows.is_empty() {
        // Nothing shows, so no cell border meets the table's own.
        state.reset_authored(doc, grid);
        return (table_border.left + table_border.right, state.clone());
    }
    let index = CellIndex::new(&rows_shown);
    state.reset_authored(doc, grid);
    resolve_collapsed_borders(doc, state, &rows_shown, &index, table_idx);
    let lines = compute_collapsed_lines(doc, state, &rows_shown, &index, table_border, basis);
    let in_place = state.clone();
    state.reset_authored(doc, shown);
    (lines.inline_border_width(), in_place)
}

fn compute_collapsed_lines(
    _doc: &Document,
    state: &BorderState,
    grid: &TableGrid,
    index: &CellIndex,
    table_border: &Edges<f32>,
    basis: Option<f32>,
) -> CollapsedLines {
    let n = grid.n_cols as usize;
    let m = grid.rows.len();

    // Per-cell border cache (row-major over grid.cells).
    let borders: Vec<Edges<f32>> = grid
        .cells
        .iter()
        .map(|c| {
            state
                .cell(c.node_id)
                .expect("line cell belongs to border state")
                .widths
                .into_edges()
                .map(|width| resolve_length(width, basis))
        })
        .collect();

    // Vertical boundaries 0..=n. Boundary i separates column i-1 and i;
    // the collapsed line is the max of adjoining borders, and the overlap
    // (absorbed width) is the row-wise max of the pairwise min.
    let mut col_overlaps = vec![0.0f32; n.saturating_sub(1)];
    let mut outer_left = table_border.left;
    let mut outer_right = table_border.right;
    for i in 0..=n {
        if i == 0 {
            for r in 0..m {
                if let Some((pos, c)) = cell_at(grid, index, r as u16, 0)
                    && c.col_start == 0
                {
                    outer_left = f32_max_compat(outer_left, borders[pos].left);
                }
            }
        } else if i == n {
            for r in 0..m {
                if let Some((pos, c)) = cell_at(grid, index, r as u16, (n as u16).saturating_sub(1))
                    && c.col_start + c.col_span == n as u16
                {
                    outer_right = f32_max_compat(outer_right, borders[pos].right);
                }
            }
        } else {
            let mut ov = 0.0f32;
            for r in 0..m {
                let left = cell_at(grid, index, r as u16, (i as u16).saturating_sub(1))
                    .filter(|(_, c)| c.col_start + c.col_span == i as u16);
                let right = cell_at(grid, index, r as u16, i as u16)
                    .filter(|(_, c)| c.col_start == i as u16);
                if let (Some((a, _)), Some((b, _))) = (left, right) {
                    let (ra, lb) = (borders[a].right, borders[b].left);
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
            for (pos, _) in grid.cells.iter().enumerate().filter(|(_, c)| c.row == 0) {
                outer_top = f32_max_compat(outer_top, borders[pos].top);
            }
        } else if j == m {
            for (pos, _) in grid
                .cells
                .iter()
                .enumerate()
                .filter(|(_, c)| c.row as usize + c.row_span as usize == m)
            {
                outer_bottom = f32_max_compat(outer_bottom, borders[pos].bottom);
            }
        } else {
            let mut ov = 0.0f32;
            for i in 0..n {
                let above = cell_at(grid, index, (j as u16).saturating_sub(1), i as u16)
                    .filter(|(_, c)| c.row as usize + c.row_span as usize == j);
                let below =
                    cell_at(grid, index, j as u16, i as u16).filter(|(_, c)| c.row as usize == j);
                if let (Some((a, _)), Some((b, _))) = (above, below) {
                    let (ba, tb) = (borders[a].bottom, borders[b].top);
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
mod tests;
