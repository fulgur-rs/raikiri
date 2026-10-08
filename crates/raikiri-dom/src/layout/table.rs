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
    BorderCollapseValue, BorderStyle, CaptionSideValue, DisplayValue, Sides, TableLayoutValue,
    VerticalAlign, WritingMode,
};
use taffy::style::{CompactLength, Dimension};
use taffy::style_helpers::{TaffyMaxContent, TaffyMinContent};
use taffy::tree::{RunMode, SizingMode};
use taffy::{
    AvailableSpace, Layout as TaffyLayout, LayoutInput, LayoutOutput, LayoutPartialTree,
    LengthPercentage, NodeId, Point, Rect, Size,
};

use crate::document::Document;
use taffy::util::{MaybeResolve, ResolveOrZero};

pub(crate) mod anonymous;

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
    specified_min_width: Dimension,
    resolved: Option<TaffyLayout>,
    natural_height: f32,
    baseline: f32,
}

// ---------------------------------------------------------------------------
// Public entry — called from `taffy_impl.rs` when `Node::display == Table`.
// ---------------------------------------------------------------------------

fn table_writing_mode(doc: &Document, table_idx: usize) -> WritingMode {
    let mut current = Some(doc.ifc_source_owner(table_idx));
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

fn caption_minimum_width(doc: &mut Document, table: usize) -> f32 {
    let Some(caption) = doc.nodes[table]
        .children
        .iter()
        .copied()
        .find(|&id| doc.nodes[id].display == DisplayValue::TableCaption)
        .filter(|&id| doc.nodes[id].style.position != taffy::Position::Absolute)
    else {
        return 0.0;
    };
    let output = doc.compute_child_layout(
        NodeId::from(caption),
        LayoutInput {
            run_mode: RunMode::ComputeSize,
            sizing_mode: SizingMode::InherentSize,
            axis: taffy::tree::RequestedAxis::Horizontal,
            known_dimensions: Size::NONE,
            parent_size: Size::NONE,
            available_space: Size {
                width: AvailableSpace::MinContent,
                height: AvailableSpace::MaxContent,
            },
            known_dimensions_are_definite: Size {
                width: false,
                height: false,
            },
            vertical_margins_are_collapsible: taffy::geometry::Line::FALSE,
        },
    );
    let margins = doc.nodes[caption].style.margin;
    output.size.width
        + super::used_style_length_percentage_auto(margins.left, 0.0).unwrap_or(0.0)
        + super::used_style_length_percentage_auto(margins.right, 0.0).unwrap_or(0.0)
}

fn layout_table_caption(
    doc: &mut Document,
    table_idx: usize,
    parent_width: f32,
    parent_height: Option<f32>,
) -> Option<Size<f32>> {
    let caption_id = doc.nodes[table_idx]
        .children
        .iter()
        .copied()
        .find(|&child| doc.nodes[child].display == DisplayValue::TableCaption)?;
    let style = doc.nodes[caption_id].style.clone();
    // Let block layout apply authored sizes in their box-sizing domain.
    let vertical = is_vertical_writing_mode(table_writing_mode(doc, table_idx));
    let known_width = if !vertical && style.size.width.is_auto() {
        let left = super::used_style_length_percentage_auto(style.margin.left, parent_width)
            .unwrap_or(0.0);
        let right = super::used_style_length_percentage_auto(style.margin.right, parent_width)
            .unwrap_or(0.0);
        Some((parent_width - left - right).max(0.0))
    } else {
        None
    };
    let known_height = if vertical && style.size.height.is_auto() {
        parent_height.map(|height| {
            let top = super::used_style_length_percentage_auto(style.margin.top, parent_width)
                .unwrap_or(0.0);
            let bottom =
                super::used_style_length_percentage_auto(style.margin.bottom, parent_width)
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
            axis: taffy::tree::RequestedAxis::Both,
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
            known_dimensions_are_definite: taffy::geometry::Size {
                width: known_width.is_some(),
                height: known_height.is_some(),
            },
            vertical_margins_are_collapsible: taffy::geometry::Line::FALSE,
        },
    );
    let margin_left =
        super::used_style_length_percentage_auto(style.margin.left, parent_width).unwrap_or(0.0);
    let margin_top =
        super::used_style_length_percentage_auto(style.margin.top, parent_width).unwrap_or(0.0);
    let margin_right =
        super::used_style_length_percentage_auto(style.margin.right, parent_width).unwrap_or(0.0);
    let margin_bottom =
        super::used_style_length_percentage_auto(style.margin.bottom, parent_width).unwrap_or(0.0);
    let layout = TaffyLayout {
        order: 0,
        location: Point {
            x: margin_left,
            y: margin_top,
        },
        size: output.size,
        scrollable_overflow_rect: output.scrollable_overflow_rect,
        scrollbar_size: Size::ZERO,
        padding: style
            .padding
            .resolve_or_zero(Some(parent_width), crate::taffy_impl::resolve_calc),
        border: style
            .border
            .resolve_or_zero(Some(parent_width), crate::taffy_impl::resolve_calc),
        margin: Rect {
            left: margin_left,
            right: margin_right,
            top: margin_top,
            bottom: margin_bottom,
        },
    };
    let sanitized = super::sanitize_taffy_layout(&layout, &mut doc.layout_warnings);
    doc.nodes[caption_id].unrounded_layout = sanitized;
    Some(Size {
        width: output.size.width + margin_left + margin_right,
        height: output.size.height + margin_top + margin_bottom,
    })
}

fn position_table_caption(
    doc: &mut Document,
    table_idx: usize,
    grid_size: Size<f32>,
    caption_size: Option<Size<f32>>,
    run_mode: RunMode,
) -> (Size<f32>, Point<f32>) {
    let caption_id = doc.nodes[table_idx]
        .children
        .iter()
        .copied()
        .find(|&child| doc.nodes[child].display == DisplayValue::TableCaption);
    // Keep the legacy positioned caption layout and ink, but exclude it from flow.
    let in_flow =
        caption_id.is_some_and(|id| doc.nodes[id].style.position != taffy::Position::Absolute);
    let caption_size = if in_flow {
        caption_size.unwrap_or(Size::ZERO)
    } else {
        Size::ZERO
    };
    let mode = table_writing_mode(doc, table_idx);
    let vertical = is_vertical_writing_mode(mode);
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
        doc.nodes[id].unrounded_layout =
            super::sanitize_taffy_layout(&layout, &mut doc.layout_warnings);
    }
    if run_mode == RunMode::PerformLayout && in_flow {
        let grid_layout = super::sanitize_taffy_layout(
            &TaffyLayout {
                location: grid_origin,
                size: grid_size,
                ..TaffyLayout::with_order(0)
            },
            &mut doc.layout_warnings,
        );
        doc.nodes[table_idx].table_grid_box = Some(raikiri_traits::PaintRect::new(
            grid_layout.location.x,
            grid_layout.location.y,
            grid_layout.size.width,
            grid_layout.size.height,
        ));
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
    doc: &mut Document,
    table_id: NodeId,
    inputs: LayoutInput,
) -> LayoutOutput {
    match compute_table_layout_checked(doc, table_id, inputs) {
        Ok(output) => output,
        Err(error) => {
            doc.table_layout_error = Some(error.to_string());
            LayoutOutput::from_outer_size(Size::ZERO)
        }
    }
}

fn compute_table_layout_checked(
    doc: &mut Document,
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
    let abspos_table = doc.nodes[table_idx].style.position == taffy::Position::Absolute;
    let mut grid = build_table_grid(doc, table_idx)?;
    let table_layout = doc.nodes[table_idx].table_layout;
    let collapse = doc.nodes[table_idx].border_collapse == BorderCollapseValue::Collapse;
    if collapse && !grid.rows.is_empty() && grid.n_cols > 0 {
        resolve_collapsed_row_borders(doc, &grid);
        resolve_collapsed_cell_borders(doc, &grid);
        resolve_collapsed_table_edges(doc, &grid, table_idx);
    }
    let vertical_writing = is_vertical_writing_mode(table_writing_mode(doc, table_idx));
    // CAPMIN constrains the grid before columns are assigned; widening only
    // the wrapper would leave its background and containing block too narrow.
    let caption_minimum = if vertical_writing {
        0.0
    } else {
        caption_minimum_width(doc, table_idx)
    };

    // Container metrics, split so collapse can substitute collapsed outer
    // borders for the table's own border widths (CSS 2.1 §17.6.2 — the table
    // border participates in conflict resolution with edge cells).
    let pad = resolve_table_padding(doc, table_idx, inputs.parent_size);
    let table_border = resolve_table_border(doc, table_idx, inputs.parent_size);

    // Known outer dimensions (from taffy's compute_root_layout known_dimensions or style.size)
    // The table's outer size caller may have imposed via `apply_page_content_box_to_body` / block layout.
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
        let mut width = if parent_is_flex_or_grid {
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
                .max(padding_border_size.height)
        });
        let caption_size = layout_table_caption(
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
        );
        if !vertical_writing && let Some(size) = caption_size {
            width = width.max(size.width + padding_border_size.width);
        }
        let (wrapper_size, _) = position_table_caption(
            doc,
            table_idx,
            Size { width, height },
            caption_size,
            inputs.run_mode,
        );
        return Ok(LayoutOutput::from_outer_size(wrapper_size));
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
    let spacing = Size {
        width: border_spacing.0 * (grid.n_cols as f32 + 1.0),
        height: border_spacing.1 * (grid.rows.len() as f32 + 1.0),
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
    let specified_width = resolve_dimension(
        doc.nodes[table_idx].style.size.width,
        inputs.parent_size.width,
    );
    // In fixed mode an unresolvable specified width (auto, or % of an
    // indefinite container) falls back to the auto algorithm below
    // (CSS 2.1 §17.5.2.1: a fixed-layout table with `width: auto` uses the
    // automatic layout algorithm; §17.5.2: such a table does not fill its
    // containing block).
    // Both auto and fixed+auto resolve columns against the SPECIFIED width
    // only (`None` when auto): with `known_dimensions.width = None` the
    // resolver takes its cap branch (preferred size capped by the definite
    // container, §17.5.2.2) instead of stretch-to-fill. A specified width
    // keeps the previous basis (`effective_known`, the taffy-resolved outer
    // width — subtracting insets recovers the content box).
    let inputs_for_columns = LayoutInput {
        known_dimensions: Size {
            width: match specified_width {
                Some(_) => effective_known
                    .width
                    .map(|width| width.max(caption_minimum)),
                None => None,
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
        let avail = specified_width
            .map(|w| f32_max_compat(w.max(caption_minimum) - distrib_insets.width, 0.0));
        match avail {
            Some(avail) => {
                for cell in &mut grid.cells {
                    if cell.row != 0 {
                        continue;
                    }
                    let style = &doc
                        .ifc_layout_node(cell.node_id)
                        .expect("table cell layout view")
                        .style;
                    if let Some(minimum) = style
                        .min_size
                        .width
                        .maybe_resolve(Some(avail), crate::taffy_impl::resolve_calc)
                    {
                        let padding = style
                            .padding
                            .resolve_or_zero(Some(avail), crate::taffy_impl::resolve_calc);
                        let border = style
                            .border
                            .resolve_or_zero(Some(avail), crate::taffy_impl::resolve_calc);
                        let insets = padding.left + padding.right + border.left + border.right;
                        let outer = if style.box_sizing == taffy::BoxSizing::ContentBox {
                            minimum + insets
                        } else {
                            minimum.max(insets)
                        };
                        let gaps =
                            border_spacing.0 * usize::from(cell.col_span).saturating_sub(1) as f32;
                        cell.specified_min_width = Dimension::length((outer - gaps).max(0.0));
                    }
                }
                resolve_fixed_column_widths(&grid, avail)
            }
            None => resolve_column_widths(
                doc,
                &grid,
                inputs_for_columns,
                distrib_insets,
                border_spacing.0,
            ),
        }
    } else {
        resolve_column_widths(
            doc,
            &grid,
            inputs_for_columns,
            distrib_insets,
            border_spacing.0,
        )
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
    let specified_height = resolve_dimension(st.size.height, inputs.parent_size.height);
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
    distribute_extra_width(
        &mut column_widths,
        (caption_minimum - distrib_insets.width).max(0.0),
    );

    // Row heights
    let mut row_heights = resolve_row_heights(doc, &mut grid, &column_widths, border_spacing)?;
    // An authored definite table height also establishes the containing block
    // for percentage-sized children of a single-row cell. Fragmentation can
    // impose a definite height on a single-row table without doing so; keep
    // that free space in the wrapper rather than manufacturing a cell
    // fragment in that case.
    if effective_known.height.is_some() && (grid.rows.len() > 1 || specified_height.is_some()) {
        let known_h = effective_known.height.unwrap(); // cov:ignore: exercised by ignored exact table-fragmentation WPT.
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
                    clamp_min_max(f32_max_compat(w, min_table_width), min_w_outer, max_w_outer)
                })
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

    let caption_size = layout_table_caption(
        doc,
        table_idx,
        final_size.width,
        if vertical_writing {
            Some(final_size.height)
        } else {
            inputs.parent_size.height
        },
    );
    let (wrapper_size, grid_origin) =
        position_table_caption(doc, table_idx, final_size, caption_size, inputs.run_mode);
    if inputs.run_mode == RunMode::ComputeSize {
        return Ok(LayoutOutput::from_outer_size(wrapper_size));
    }

    // Position cells: origin includes padding + collapsed outer border.
    let x_origin = pad.left + collapsed.outer_left + border_spacing.0 + grid_origin.x;
    let y_origin = pad.top + collapsed.outer_top + border_spacing.1 + grid_origin.y;
    // A separated gap is a negative overlap between adjoining tracks.
    let col_steps = if collapse {
        collapsed.col_overlaps.clone()
    } else {
        vec![-border_spacing.0; usize::from(grid.n_cols).saturating_sub(1)]
    };
    let row_steps = if collapse {
        collapsed.row_overlaps.clone()
    } else {
        vec![-border_spacing.1; grid.rows.len().saturating_sub(1)]
    };
    let col_origins = track_origins(&column_widths, &col_steps, x_origin);
    let row_origins = track_origins(&row_heights, &row_steps, y_origin);
    place_cells(
        doc,
        &mut grid,
        &column_widths,
        &row_heights,
        (&col_origins, &row_origins),
        border_spacing,
        vertical_writing,
    )?; // cov:ignore: failure is defensive; resolve_row_heights already validated these ranges and track_origins supplies every column origin.
    let first_row_baseline = grid
        .cells
        .iter()
        .filter(|cell| {
            cell.row == 0
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
                .filter(|cell| cell.row == 0)
                .map(|cell| {
                    let layout = doc.table_layout_node_mut(cell.node_id).unrounded_layout;
                    layout.size.height - layout.padding.bottom - layout.border.bottom
                })
                .fold(0.0, f32::max)
        });
    doc.nodes[table_idx].table_first_baseline = Some(y_origin + first_row_baseline);
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
            table_writing_mode(doc, table_idx),
        );
    }
    Ok(LayoutOutput::from_outer_size(wrapper_size))
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
    doc: &Document,
    table_idx: usize,
) -> Result<TableGrid, raikiri_traits::LayoutError> {
    let mut rows: Vec<usize> = Vec::new();
    let mut cells: Vec<CellPlacement> = Vec::new();
    let mut n_cols: u16 = 0;
    if let Some(projected_rows) = doc.table_objects.rows.get(&table_idx) {
        for projected in projected_rows {
            rows.push(projected.marker);
            let row = u16::try_from(rows.len() - 1).map_err(|_| table_column_error())?;
            let mut col = 0;
            for &node_id in &projected.cells {
                let anonymous = node_id >= doc.nodes.len();
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
                cells.push(CellPlacement {
                    node_id,
                    row,
                    col_start: col,
                    col_span,
                    row_span,
                    specified_width: doc
                        .ifc_layout_node(node_id)
                        .expect("table cell layout view")
                        .style
                        .size
                        .width,
                    specified_min_width: doc
                        .ifc_layout_node(node_id)
                        .expect("table cell layout view")
                        .style
                        .min_size
                        .width
                        .into(),
                    resolved: None,
                    natural_height: 0.0,
                    baseline: 0.0,
                });
                col = next_col;
            }
            n_cols = n_cols.max(col);
        }
    } else {
        collect_rows(doc, table_idx, &mut rows, &mut cells, &mut n_cols)?;
    }
    let col_widths = collect_col_widths(doc, table_idx)?;
    n_cols = n_cols.max(col_widths.len() as u16);
    Ok(TableGrid {
        n_cols,
        rows,
        cells,
        col_widths,
    })
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

fn collect_col_widths(
    doc: &Document,
    table_idx: usize,
) -> Result<Vec<ColSizing>, raikiri_traits::LayoutError> {
    fn col_span(doc: &Document, node_id: usize) -> usize {
        if let crate::node::NodeData::Element(data) = &doc
            .ifc_layout_node(node_id)
            .expect("table box layout view")
            .data
        {
            for a in &data.attributes {
                if a.namespace.is_none()
                    && a.local.as_str() == "span"
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
    fn append_columns(
        out: &mut Vec<ColSizing>,
        sizing: ColSizing,
        span: usize,
    ) -> Result<(), raikiri_traits::LayoutError> {
        if span > usize::from(u16::MAX) - out.len() {
            return Err(table_column_error());
        }
        out.extend(std::iter::repeat_n(sizing, span));
        Ok(())
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
                    append_columns(&mut out, sz, col_span(doc, col_id))?;
                }
            }
            DisplayValue::TableColumn => {
                let sz = sizing_of(doc, child_id);
                append_columns(&mut out, sz, col_span(doc, child_id))?;
            }
            _ => {}
        }
    }
    Ok(out)
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
) -> Result<(), raikiri_traits::LayoutError> {
    collect_rows_inner(doc, container_idx, rows, cells, n_cols, true)
}

fn collect_rows_inner(
    doc: &Document,
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
    doc: &Document,
    pending: &mut Vec<usize>,
    rows: &mut Vec<usize>,
    cells: &mut Vec<CellPlacement>,
    n_cols: &mut u16,
) -> Result<(), raikiri_traits::LayoutError> {
    if pending.is_empty() {
        return Ok(());
    }
    // Anonymous row: borrow first cell's id as row marker (not used for layout beyond count)
    rows.push(pending[0]);
    let row_ix = (rows.len() - 1) as u16;
    let mut col: u16 = 0;
    for &cell_id in pending.iter() {
        let col_span = get_colspan(doc, cell_id);
        let next_col = next_table_column(col, col_span)?;
        let row_span = get_rowspan(doc, cell_id);
        let specified_width = doc.nodes[cell_id].style.size.width;
        cells.push(CellPlacement {
            node_id: cell_id,
            row: row_ix,
            col_start: col,
            col_span,
            row_span,
            specified_width,
            specified_min_width: doc.nodes[cell_id].style.min_size.width.into(),
            resolved: None,
            natural_height: 0.0,
            baseline: 0.0,
        });
        col = next_col;
    }
    *n_cols = (*n_cols).max(col);
    pending.clear();
    Ok(())
}

fn collect_cells_in_row(
    doc: &Document,
    row_id: usize,
    rows: &mut Vec<usize>,
    cells: &mut Vec<CellPlacement>,
    n_cols: &mut u16,
) -> Result<(), raikiri_traits::LayoutError> {
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
        let next_col = next_table_column(col, col_span)?;
        let row_span = get_rowspan(doc, cell_id);
        let specified_width = doc.nodes[cell_id].style.size.width;
        cells.push(CellPlacement {
            node_id: cell_id,
            row: row_ix,
            col_start: col,
            col_span,
            row_span,
            specified_width,
            specified_min_width: doc.nodes[cell_id].style.min_size.width.into(),
            resolved: None,
            natural_height: 0.0,
            baseline: 0.0,
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
    if let crate::node::NodeData::Element(data) = &doc
        .ifc_layout_node(node_id)
        .expect("table box layout view")
        .data
    {
        for a in &data.attributes {
            if a.namespace.is_none()
                && a.local.as_str() == "colspan"
                && let Ok(v) = a.value.parse::<u32>()
            {
                // Parse wide (u32) so huge values (e.g. "100000", which
                // overflows u16) clamp to 1000 instead of failing to parse
                // and falling back to 1.
                return v.clamp(1, 1000) as u16;
            }
        }
    }
    1
}

fn get_rowspan(doc: &Document, node_id: usize) -> u16 {
    if let crate::node::NodeData::Element(data) = &doc
        .ifc_layout_node(node_id)
        .expect("table box layout view")
        .data
    {
        for a in &data.attributes {
            if a.namespace.is_none()
                && a.local.as_str() == "rowspan"
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
    spacing: f32,
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

fn cell_baseline_aligned(align: VerticalAlign) -> bool {
    !matches!(
        align,
        VerticalAlign::Top | VerticalAlign::Middle | VerticalAlign::Bottom
    )
}

// Baselines are relative to the border edge; a cell without a line uses
// its bottom content edge in the caller (CSS 2.2 section 17.5.3).
fn first_cell_baseline(doc: &Document, root: usize, content_top: f32) -> Option<f32> {
    let node = doc.ifc_layout_node(root).expect("table cell layout view");
    if matches!(
        node.display,
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
            let child_node = &doc.nodes[child];
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
        let node = &doc.nodes[id];
        if !node.is_in_document()
            || node.style.display == taffy::Display::None
            || node.style.position == taffy::Position::Absolute
            || node.style.float != taffy::Float::None
        {
            continue;
        }
        if matches!(
            node.display,
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
    doc: &mut Document,
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
        let h = doc
            .ifc_layout_node(row_id)
            .expect("table row layout view")
            .style
            .size
            .height;
        if h.tag() == CompactLength::LENGTH_TAG {
            row_heights[r] = f32_max_compat(row_heights[r], h.value());
        }
    }
    for cell in &mut grid.cells {
        let columns = cell_column_range(cell, column_widths.len())?;
        let cell_width = spanned_size(&column_widths[columns], spacing.0);
        // Cell height floors the row rather than stretching the content used
        // for alignment. Preserve ordinary block border and margin sizing.
        let authored_height = doc
            .ifc_layout_node(cell.node_id)
            .expect("table cell layout view")
            .style
            .size
            .height;
        if !authored_height.is_auto() {
            doc.table_layout_node_mut(cell.node_id).style.size.height = Dimension::auto();
            doc.table_layout_node_mut(cell.node_id).cache.clear();
        }
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
        if !authored_height.is_auto() {
            doc.table_layout_node_mut(cell.node_id).style.size.height = authored_height;
            doc.table_layout_node_mut(cell.node_id).cache.clear();
        }
        // Distribute rowspan height across spanned rows (simple: equally)
        cell.natural_height = output.size.height;
        let style = &doc
            .ifc_layout_node(cell.node_id)
            .expect("table cell layout view")
            .style;
        let padding = style
            .padding
            .resolve_or_zero(Some(cell_width), crate::taffy_impl::resolve_calc);
        let border = style
            .border
            .resolve_or_zero(Some(cell_width), crate::taffy_impl::resolve_calc);
        let authored_height = resolve_dimension(style.size.height, None).map_or(0.0, |height| {
            if style.box_sizing == taffy::BoxSizing::ContentBox {
                height + padding.top + padding.bottom + border.top + border.bottom
            } else {
                height
            }
        });
        let h = output.size.height.max(authored_height);
        let baseline = first_cell_baseline(doc, cell.node_id, padding.top + border.top);
        cell.baseline =
            baseline.unwrap_or((output.size.height - padding.bottom - border.bottom).max(0.0));
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

fn place_cells(
    doc: &mut Document,
    grid: &mut TableGrid,
    column_widths: &[f32],
    row_heights: &[f32],
    origins: (&[f32], &[f32]),
    spacing: (f32, f32),
    vertical_writing: bool,
) -> Result<(), raikiri_traits::LayoutError> {
    let (col_x, row_y) = origins;
    let mut row_baselines = vec![0.0f32; row_heights.len()];
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
    for (order, cell) in grid.cells.iter_mut().enumerate() {
        let columns = cell_column_range(cell, column_widths.len())?;
        let end_row = (cell.row as usize + cell.row_span as usize).min(row_heights.len());
        let cell_x = *col_x.get(columns.start).ok_or_else(table_column_error)?;
        let cell_y = row_y[cell.row as usize];
        // Full spanned widths — origins already net out the collapsed
        // overlaps for *positioning*; shrinking the box by the overlap as
        // well would double-count the absorbed line. Separated borders add
        // the spanned gaps instead, matching the origin differences.
        let cell_width = spanned_size(&column_widths[columns], spacing.0);
        let cell_height = spanned_size(&row_heights[cell.row as usize..end_row], spacing.1);
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
        // A cell laid out by the inline engine has no child layouts: its
        // lines are drawn from its content box, which its own border and
        // padding (as the engine measured them) place inside the cell.
        let (mut padding, border) = {
            let style = &doc
                .ifc_layout_node(cell.node_id)
                .expect("table cell layout view")
                .style;
            (
                style
                    .padding
                    .resolve_or_zero(Some(cell_width), crate::taffy_impl::resolve_calc),
                style
                    .border
                    .resolve_or_zero(Some(cell_width), crate::taffy_impl::resolve_calc),
            )
        };
        let vertical_cell =
            vertical_writing && is_vertical_writing_mode(table_writing_mode(doc, cell.node_id));
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
            VerticalAlign::Top => 0.0,
            VerticalAlign::Middle => extra / 2.0,
            VerticalAlign::Bottom => extra,
            _ if !vertical_cell => (row_baselines[cell.row as usize] - cell.baseline).max(0.0),
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
            if doc.nodes[child].style.position != taffy::Position::Absolute {
                let mut child_layout = doc.nodes[child].unrounded_layout;
                child_layout.location.y += shift;
                doc.nodes[child].unrounded_layout =
                    super::sanitize_taffy_layout(&child_layout, &mut doc.layout_warnings);
            }
        }
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
            padding,
            border,
            margin: Rect::ZERO,
        };
        cell.resolved = Some(layout);
        // Write via Document's layout storage (includes sanitize)
        {
            let sanitized = super::sanitize_taffy_layout(&layout, &mut doc.layout_warnings);
            doc.table_layout_node_mut(cell.node_id).unrounded_layout = sanitized;
        }
        if cell.node_id >= doc.nodes.len() {
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
        let _ = output;
    }
    Ok(())
}

/// Re-map the already measured table grid from horizontal physical axes to
/// vertical writing-mode axes. Vertical cells are re-laid out at their final
/// physical dimensions and aligned on their block axis. Orthogonal horizontal
/// cells retain the normal pass's content layout.
fn reposition_cells_for_vertical_writing(
    doc: &mut Document,
    grid: &mut TableGrid,
    row_heights: &[f32],
    cell_width: f32,
    origin: Point<f32>,
    spacing: f32,
    writing_mode: WritingMode,
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
        if !is_vertical_writing_mode(table_writing_mode(doc, cell.node_id)) {
            // A horizontal cell in a vertical table still aligns content on
            // its own block axis. Preserve the normal pass's vertical shift.
            doc.table_layout_node_mut(cell.node_id).unrounded_layout =
                super::sanitize_taffy_layout(layout, &mut doc.layout_warnings);
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
                axis: taffy::tree::RequestedAxis::Both,
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
                vertical_margins_are_collapsible: taffy::geometry::Line::FALSE,
            },
        );
        let node = doc
            .ifc_layout_node(cell.node_id)
            .expect("table cell layout view");
        let mut padding = node
            .style
            .padding
            .resolve_or_zero(Some(cell_width), crate::taffy_impl::resolve_calc);
        let border = node
            .style
            .border
            .resolve_or_zero(Some(cell_width), crate::taffy_impl::resolve_calc);
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
                .filter(|&&child| doc.nodes[child].style.position != taffy::Position::Absolute)
                .map(|&child| {
                    let child = &doc.nodes[child];
                    // Relative insets move ink after normal flow; they must
                    // not consume the cell's alignment space. Taffy leaves
                    // floated boxes at their normal positions, so their
                    // paint-side offsets are not removed here.
                    let offset = if child.style.float == taffy::Float::None {
                        let inner_width = cell_width
                            - padding.left
                            - padding.right
                            - border.left
                            - border.right
                            - if node.style.overflow.y == taffy::Overflow::Scroll {
                                node.style.scrollbar_width
                            } else {
                                0.0
                            };
                        let left = child
                            .style
                            .inset
                            .left
                            .maybe_resolve(Some(inner_width), crate::taffy_impl::resolve_calc);
                        let right = child
                            .style
                            .inset
                            .right
                            .maybe_resolve(Some(inner_width), crate::taffy_impl::resolve_calc);
                        if node.style.direction == taffy::Direction::Rtl {
                            right.map(|right| -right).or(left).unwrap_or(0.0)
                        } else {
                            left.or(right.map(|right| -right)).unwrap_or(0.0)
                        }
                    } else {
                        0.0
                    };
                    let child = &child.unrounded_layout;
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
        let rtl = matches!(
            writing_mode,
            WritingMode::VerticalRl | WritingMode::SidewaysRl
        );
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
            if doc.nodes[child].style.position != taffy::Position::Absolute {
                let mut layout = doc.nodes[child].unrounded_layout;
                layout.location.x += child_shift;
                doc.nodes[child].unrounded_layout =
                    super::sanitize_taffy_layout(&layout, &mut doc.layout_warnings);
            }
        }
        layout.padding = padding;
        layout.border = border;
        let sanitized = super::sanitize_taffy_layout(layout, &mut doc.layout_warnings);
        doc.table_layout_node_mut(cell.node_id).unrounded_layout = sanitized;
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
    // First-row spans are disjoint. Reserve each other span's minimum
    // proportionally in its auto columns before any span draws from them.
    for cell in &grid.cells {
        if cell.row != 0 || cell.col_span <= 1 {
            continue;
        }
        let start = usize::from(cell.col_start);
        let end = (start + usize::from(cell.col_span)).min(n);
        if start >= end {
            continue; // cov:ignore: defensive range guard; native grid construction validates nonzero spans within its column count.
        }
        let minimum = resolve_dimension(cell.specified_min_width, Some(avail)).unwrap_or(0.0);
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
    for cell in &grid.cells {
        if cell.row != 0 || cell.col_span <= 1 {
            continue;
        }
        let start = usize::from(cell.col_start);
        let end = (start + usize::from(cell.col_span)).min(n);
        if start >= end {
            continue; // cov:ignore: defensive range guard; native grid construction validates nonzero spans within its column count.
        }
        let minimum = resolve_dimension(cell.specified_min_width, Some(avail)).unwrap_or(0.0);
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
    let s = &doc
        .ifc_layout_node(cell_id)
        .expect("table cell layout view")
        .style;
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
    doc: &Document,
    node_id: usize,
    side: CellBorderSide,
) -> Option<CollapsedBorderCandidate> {
    // An earlier resolution step may already have replaced a side.
    let node = &doc.ifc_layout_node(node_id).expect("table box layout view");
    let borders = node
        .collapsed_border
        .as_ref()
        .or(node.computed_border.as_ref())?;
    let (border, taffy_width) = match side {
        CellBorderSide::Top => (
            borders.top,
            doc.ifc_layout_node(node_id)
                .expect("table box layout view")
                .style
                .border
                .top,
        ),
        CellBorderSide::Right => (
            borders.right,
            doc.ifc_layout_node(node_id)
                .expect("table box layout view")
                .style
                .border
                .right,
        ),
        CellBorderSide::Bottom => (
            borders.bottom,
            doc.ifc_layout_node(node_id)
                .expect("table box layout view")
                .style
                .border
                .bottom,
        ),
        CellBorderSide::Left => (
            borders.left,
            doc.ifc_layout_node(node_id)
                .expect("table box layout view")
                .style
                .border
                .left,
        ),
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

/// Give `node_id`'s `side` the used collapsed border `border`: its layout
/// width and the style and color it paints.
fn set_collapsed_side(
    doc: &mut Document,
    node_id: usize,
    side: CellBorderSide,
    border: ComputedBorder,
    taffy_width: LengthPercentage,
) {
    let node = doc.table_layout_node_mut(node_id);
    match side {
        CellBorderSide::Top => node.style.border.top = taffy_width,
        CellBorderSide::Right => node.style.border.right = taffy_width,
        CellBorderSide::Bottom => node.style.border.bottom = taffy_width,
        CellBorderSide::Left => node.style.border.left = taffy_width,
    }
    let Some(computed) = node.computed_border else {
        return; // cov:ignore: callers only reach here after border_candidate found computed borders.
    };
    let painted = node.collapsed_border.get_or_insert(computed);
    *border_side(painted, side) = border;
}

/// Resolve the shared border between two adjacent cells. Both cells take the
/// winner's width, and both paint the winner's style and color, so the later
/// cell no longer paints its losing border over the winning one.
#[cfg(test)]
fn resolve_collapsed_border_pair(
    doc: &mut Document,
    leading_id: usize,
    leading_side: CellBorderSide,
    trailing_id: usize,
    trailing_side: CellBorderSide,
) {
    let (Some(leading), Some(trailing)) = (
        border_candidate(doc, leading_id, leading_side),
        border_candidate(doc, trailing_id, trailing_side),
    ) else {
        return;
    };
    let winner = collapsed_border_winner(leading, trailing);
    set_collapsed_side(
        doc,
        leading_id,
        leading_side,
        winner.border,
        winner.taffy_width,
    );
    set_collapsed_side(
        doc,
        trailing_id,
        trailing_side,
        winner.border,
        winner.taffy_width,
    );
}

/// Resolve every shared border between adjacent cells. Each segment's winner
/// is decided from the cells' borders before any segment is resolved, so a
/// cell spanning several neighbours does not carry one segment's winner into
/// the next. A spanning cell paints the strongest winner along its side;
/// each neighbour paints its own segment's winner.
fn resolve_collapsed_cell_borders(doc: &mut Document, grid: &TableGrid) {
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
                SIDES.map(|side| border_candidate(doc, cell.node_id, side)),
            )
        })
        .collect();
    let resolve = |doc: &mut Document,
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
        keep_stronger_side(doc, leading_id, leading_side, winner);
        keep_stronger_side(doc, trailing_id, trailing_side, winner);
    };

    let column_count = grid.n_cols as usize;
    for boundary in 1..column_count {
        for row in 0..grid.rows.len() {
            let row = row as u16;
            let left = cell_at(grid, row, (boundary - 1) as u16)
                .filter(|cell| cell.col_start + cell.col_span == boundary as u16);
            let right = cell_at(grid, row, boundary as u16)
                .filter(|cell| cell.col_start == boundary as u16);
            if let (Some(left), Some(right)) = (left, right) {
                resolve(
                    doc,
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
            let above = cell_at(grid, boundary - 1, column)
                .filter(|cell| cell.row.saturating_add(cell.row_span) == boundary);
            let below = cell_at(grid, boundary, column).filter(|cell| cell.row == boundary);
            if let (Some(above), Some(below)) = (above, below) {
                resolve(
                    doc,
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
    doc: &mut Document,
    node_id: usize,
    side: CellBorderSide,
    winner: CollapsedBorderCandidate,
) {
    // The pair winner is passed first so it wins ties: a cell's own border
    // must not beat the winner of its segment by equality alone.
    let stronger = match border_candidate(doc, node_id, side) {
        Some(current) => collapsed_border_winner(winner, current),
        None => winner, // cov:ignore: the pair's own candidate came from this side.
    };
    set_collapsed_side(doc, node_id, side, stronger.border, stronger.taffy_width);
}

/// Resolve each cell's sides against the borders of the rows they lie on
/// (CSS 2.1 §17.6.2.1): its top against its first row, its bottom against
/// its last row, and an edge cell's left and right against its row. A cell
/// wins ties over a row. Rows then paint nothing themselves; their borders
/// live on in the cells that took them.
fn resolve_collapsed_row_borders(doc: &mut Document, grid: &TableGrid) {
    let last_col = grid.n_cols;
    for index in 0..grid.cells.len() {
        let cell = &grid.cells[index];
        // An anonymous row is recorded by its first cell, which has no row
        // border of its own to contribute.
        let row_box = |index: usize| {
            grid.rows.get(index).copied().filter(|&row| {
                doc.ifc_layout_node(row)
                    .is_some_and(|node| node.display == DisplayValue::TableRow)
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
                border_candidate(doc, node_id, side),
                border_candidate(doc, row, side),
            ) else {
                continue;
            };
            let winner = collapsed_border_winner(own, row_border);
            set_collapsed_side(doc, node_id, side, winner.border, winner.taffy_width);
        }
    }
    for &row in &grid.rows {
        if doc
            .ifc_layout_node(row)
            .is_none_or(|node| node.display != DisplayValue::TableRow)
        {
            continue;
        }
        if let Some(computed) = doc.nodes[row].computed_border {
            doc.nodes[row].collapsed_border = Some(Sides {
                top: computed.top.without_line(),
                right: computed.right.without_line(),
                bottom: computed.bottom.without_line(),
                left: computed.left.without_line(),
            });
        }
    }
}

/// Resolve each edge cell's outer side against the table's own border on
/// that side (CSS 2.1 §17.6.2.1). A cell wins ties over the table. When the
/// table's border wins, the table paints that edge, so the cell's side takes
/// no width and paints nothing.
fn resolve_collapsed_table_edges(doc: &mut Document, grid: &TableGrid, table_idx: usize) {
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
                border_candidate(doc, node_id, side),
                border_candidate(doc, table_idx, side),
            ) else {
                continue;
            };
            let winner = collapsed_border_winner(own, table);
            if winner.border == own.border && winner.taffy_width == own.taffy_width {
                continue;
            }
            set_collapsed_side(
                doc,
                node_id,
                side,
                own.border.without_line(),
                LengthPercentage::length(0.0),
            );
        }
    }
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
mod tests;
