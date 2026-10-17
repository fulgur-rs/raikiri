//! Project-owned block flow using public Taffy primitives.

mod children;
pub(crate) mod context;
mod positioned;
mod sizing;

use crate::layout::context::{LayoutSession, TextGroupScope};
use crate::taffy_impl::resolve_calc;
use context::{BlockContext, BlockFormattingContext};
use shodo::style::TextGroupAlign;
use sizing::Metrics;
use taffy::{
    AvailableSpace, BlockItemStyle, BoxGenerationMode, CoreStyle, Float, LayoutBlockContainer,
    LayoutInput, LayoutOutput, LayoutPartialTree, Line, NodeId, Position, Rect, RequestedAxis,
    RunMode, Size, SizingMode, Style, TraversePartialTree,
};

/// Whether a block child's normal flow shares its parent's float context.
fn shares_context(style: &Style) -> bool {
    style.is_block()
        && !style.is_table()
        && style.float == Float::None
        && style.position != Position::Absolute
        && !style.overflow.x.is_scroll_container()
        && !style.overflow.y.is_scroll_container()
        && !style.contain.establishes_independent_formatting_context()
        && style.align_content.keyword == taffy::AlignContentKeyword::Normal
}

fn child_style(tree: &LayoutSession<'_>, child: NodeId) -> Style {
    let query = tree.get_block_child_style(child);
    let mut style = query.style.clone();
    style.padding = query.padding();
    style.border = query.border();
    style
}

fn child_inputs(parent: Size<Option<f32>>, width: Option<f32>, run_mode: RunMode) -> LayoutInput {
    LayoutInput {
        run_mode,
        sizing_mode: SizingMode::InherentSize,
        axis: RequestedAxis::Both,
        known_dimensions: Size {
            width,
            height: None,
        },
        known_dimensions_are_definite: Size {
            width: true,
            height: true,
        },
        parent_size: parent,
        available_space: Size {
            width: width.map_or(AvailableSpace::MaxContent, AvailableSpace::Definite),
            height: AvailableSpace::MaxContent,
        },
        vertical_margins_are_collapsible: Line::FALSE,
    }
}

fn intrinsic_width(tree: &mut LayoutSession<'_>, node: NodeId, space: AvailableSpace) -> f32 {
    let children: Vec<_> = tree.child_ids(node).collect();
    let mut width = 0.0_f32;
    let mut floats = taffy::FloatIntrinsicWidthCalculator::new(space);
    for child in children {
        let style = child_style(tree, child);
        if style.box_generation_mode() == BoxGenerationMode::None
            || style.position == Position::Absolute
        {
            continue;
        }
        let mut inputs = child_inputs(Size::NONE, None, RunMode::ComputeSize);
        inputs.available_space.width = space;
        let measured = tree.compute_child_layout_with_block_ctx(child, inputs, None);
        let margin = style
            .margin
            .map(|edge| edge.resolve_to_option(0.0, resolve_calc).unwrap_or(0.0));
        let child_width = measured.size.width + margin.horizontal_axis_sum();
        match style.float {
            Float::Left => floats.add_float(child_width, taffy::FloatDirection::Left, style.clear),
            Float::Right => {
                floats.add_float(child_width, taffy::FloatDirection::Right, style.clear)
            }
            Float::None => width = width.max(child_width),
        }
    }
    width.max(floats.result())
}

pub(crate) fn compute_block_flow(
    tree: &mut LayoutSession<'_>,
    node: NodeId,
    inputs: LayoutInput,
    context: Option<&mut BlockContext<'_>>,
) -> LayoutOutput {
    let style = tree.get_block_container_style(node).clone();
    let metrics = Metrics::resolve(&style, inputs);
    let width = if let Some(width) = metrics.preferred.width {
        width
    } else {
        let maximum = intrinsic_width(tree, node, AvailableSpace::MaxContent)
            + metrics.inset.horizontal_axis_sum();
        let minimum = intrinsic_width(tree, node, AvailableSpace::MinContent)
            + metrics.inset.horizontal_axis_sum();
        let fit = |limit: f32| maximum.min(limit).max(minimum);
        let width = match style.size.width.expand() {
            taffy::ExpandedDimension::MinContent => minimum,
            taffy::ExpandedDimension::MaxContent => maximum,
            taffy::ExpandedDimension::FitContentPx(limit) => {
                fit(limit + metrics.inset.horizontal_axis_sum())
            }
            taffy::ExpandedDimension::FitContentPercent(limit) => {
                inputs.parent_size.width.map_or(maximum, |basis| {
                    fit(limit * basis + metrics.inset.horizontal_axis_sum())
                })
            }
            taffy::ExpandedDimension::FitContentCalc(handle) => {
                inputs.parent_size.width.map_or(maximum, |basis| {
                    fit(resolve_calc(handle, basis) + metrics.inset.horizontal_axis_sum())
                })
            }
            _ => match inputs.available_space.width {
                AvailableSpace::MaxContent => maximum,
                AvailableSpace::MinContent => minimum,
                AvailableSpace::Definite(limit) => fit(limit),
            },
        };
        metrics.clamp_width(width)
    };
    let align = tree.nodes[usize::from(node)].text_group_align;
    let mut shared = context.filter(|_| shares_context(&style));
    let run = |tree: &mut LayoutSession<'_>, shared: Option<&mut BlockContext<'_>>| {
        if let Some(ctx) = shared {
            return layout_context(tree, node, inputs, &style, metrics, width, ctx);
        }
        let mut owner = BlockFormattingContext::new();
        layout_context(
            tree,
            node,
            inputs,
            &style,
            metrics,
            width,
            &mut owner.root_block_context(),
        )
    };
    if align == TextGroupAlign::None {
        return run(tree, shared);
    }
    if matches!(
        tree.context.text_group,
        TextGroupScope::Probing { .. } | TextGroupScope::Skipping
    ) {
        // Its lines are not part of the group being measured, so it is laid
        // out once, unpadded, with nothing below it joining anything.
        let outer = std::mem::replace(&mut tree.context.text_group, TextGroupScope::Skipping);
        let output = run(tree, shared);
        tree.context.text_group = outer;
        return output;
    }
    // The box roots a `text-group-align` group: the lines under it are laid
    // out once to find the space the shortest leaves unfilled, then again
    // with every line padded by it. The measuring pass leaves the block
    // context as it found it.
    let outer = std::mem::replace(
        &mut tree.context.text_group,
        TextGroupScope::Probing { shortest: None },
    );
    let saved = shared.as_deref().map(BlockContext::snapshot);
    run(tree, shared.as_deref_mut());
    if let (Some(ctx), Some(saved)) = (shared.as_deref_mut(), &saved) {
        ctx.restore(saved);
    }
    let padding = match tree.context.text_group {
        TextGroupScope::Probing { shortest, .. } => shortest.unwrap_or(0.0),
        _ => 0.0, // cov:ignore: the probe leaves its own scope in place.
    };
    tree.context.text_group = TextGroupScope::Applying { align, padding };
    let output = run(tree, shared);
    tree.context.text_group = outer;
    output
}

fn layout_context(
    tree: &mut LayoutSession<'_>,
    node: NodeId,
    inputs: LayoutInput,
    style: &Style,
    metrics: Metrics,
    width: f32,
    ctx: &mut BlockContext<'_>,
) -> LayoutOutput {
    let collapse_start = shares_context(style)
        && inputs.vertical_margins_are_collapsible.start
        && metrics.inset.top == 0.0;
    let collapse_end = shares_context(style)
        && inputs.vertical_margins_are_collapsible.end
        && metrics.inset.bottom == 0.0
        && style.size.height.is_auto()
        && metrics.preferred.height.is_none();
    if ctx.is_bfc_root() {
        ctx.set_width(width);
        ctx.set_origin_resolved(!collapse_start);
    }
    ctx.apply_content_box_inset([metrics.inset.left, metrics.inset.right]);
    let mut flow = children::layout(
        tree,
        node,
        inputs,
        style,
        metrics,
        width,
        ctx,
        collapse_start,
        collapse_end,
    );
    let mut natural_end = flow.end;
    if ctx.is_bfc_root() {
        natural_end = natural_end.max(ctx.floated_content_height_contribution());
    }
    let height = metrics
        .preferred
        .height
        .unwrap_or_else(|| metrics.clamp_height(natural_end + metrics.inset.bottom));
    // When `min-height` raises the used height above the content, the last
    // child's bottom margin no longer adjoins the bottom edge. As in
    // browsers, it then neither escapes nor adds to the height that
    // `min-height` fixed.
    let minimum_separates = collapse_end
        && metrics
            .min
            .height
            .is_some_and(|min| min > 0.0 && min > natural_end + metrics.inset.bottom);
    let size = Size { width, height };
    children::align(
        tree,
        style,
        &mut flow,
        height - metrics.inset.bottom - natural_end,
    );
    let positioned_overflow = if inputs.run_mode == RunMode::PerformLayout {
        positioned::layout(tree, node, &flow.positioned, metrics, size)
    } else {
        Rect::ZERO
    };
    let mut overflow = flow.overflow.union(flow.float_overflow);
    if style.direction == taffy::Direction::Rtl {
        let origin = width - metrics.border.right;
        overflow = Rect {
            left: (origin - overflow.right).min(0.0),
            right: (origin - overflow.left).max(0.0),
            top: overflow.top - metrics.border.top,
            bottom: overflow.bottom - metrics.border.top,
        };
    } else {
        overflow.left -= metrics.border.left;
        overflow.right -= metrics.border.left;
        overflow.top -= metrics.border.top;
        overflow.bottom -= metrics.border.top;
    }
    overflow.left = overflow.left.min(0.0);
    overflow.top = overflow.top.min(0.0);
    overflow.right = overflow.right.max(0.0);
    overflow.bottom = overflow.bottom.max(0.0);
    overflow = overflow.union(positioned_overflow);
    if style.overflow.x.is_scroll_container() || style.overflow.y.is_scroll_container() {
        let end_padding = if style.direction == taffy::Direction::Rtl {
            metrics.padding.left
        } else {
            metrics.padding.right
        };
        overflow.right = (overflow.right + end_padding)
            .max(width - metrics.border.horizontal_axis_sum() - metrics.scrollbar.width);
        overflow.bottom = (overflow.bottom + metrics.padding.bottom)
            .max(height - metrics.border.vertical_axis_sum() - metrics.scrollbar.height);
    }
    LayoutOutput {
        oof_candidates: taffy::OofCandidates::NONE,
        oof_positioning_area: None,
        size,
        scrollable_overflow_rect: overflow,
        baselines: flow.baselines,
        top_margin: if collapse_start {
            flow.leading
        } else {
            taffy::CollapsibleMarginSet::ZERO
        },
        bottom_margin: if collapse_end && !minimum_separates {
            flow.trailing
        } else {
            taffy::CollapsibleMarginSet::ZERO
        },
        margins_can_collapse_through: collapse_start
            && collapse_end
            && flow.through
            && height == 0.0
            && !metrics.min.height.is_some_and(|min| min > 0.0),
    }
}

fn commit(
    tree: &mut LayoutSession<'_>,
    node: NodeId,
    order: usize,
    metrics: Metrics,
    output: &LayoutOutput,
    location: taffy::Point<f32>,
    margin: Rect<f32>,
) {
    tree.set_unrounded_layout(
        node,
        &taffy::Layout {
            order: order as u32,
            location,
            size: output.size,
            scrollable_overflow_rect: output.scrollable_overflow_rect,
            scrollbar_size: metrics.scrollbar,
            border: metrics.border,
            padding: metrics.padding,
            margin,
        },
    );
}

#[cfg(test)]
mod tests;
