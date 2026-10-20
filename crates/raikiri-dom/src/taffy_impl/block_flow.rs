//! Project-owned block flow using public Taffy primitives.

mod children;
pub(crate) mod context;
mod positioned;
mod sizing;

pub(crate) use positioned::aligned_static_position;

use crate::LayoutStyle as Style;
use crate::layout::context::{LayoutSession, TextGroupScope};
use crate::taffy_impl::{measure_against_parent_height, resolve_calc};
use context::{BlockContext, BlockFormattingContext};
use shodo::style::TextGroupAlign;
pub(crate) use sizing::Metrics;
use taffy::{
    AvailableSpace, BlockItemStyle, BoxGenerationMode, CoreStyle, Float, LayoutBlockContainer,
    LayoutInput, LayoutOutput, LayoutPartialTree, Line, MaybeMath, NodeId, Position, Rect,
    RequestedAxis, ResolveOrZero, RunMode, Size, SizingMode, TraversePartialTree,
};

/// How far block flow moves a child of a left-to-right container to the
/// right for its `position: relative` insets, when the container's content
/// box is `width` wide. A sticky box is moved alike, but its offset is not
/// one that a painter applies again, so it has none here.
pub(crate) fn relative_inline_offset(style: &Style, width: f32) -> f32 {
    if style.position != Position::Relative {
        return 0.0;
    }
    children::relative(
        style,
        Size {
            width: Some(width),
            height: None,
        },
        taffy::Direction::Ltr,
    )
    .x
}

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

/// Whether a spanner lies below `node` in blocks that share its formatting
/// context, so that it spans the columns of the multicol container around
/// `node` all the same.
fn contains_spanner(tree: &LayoutSession<'_>, node: NodeId) -> bool {
    tree.child_ids(node).any(|child| {
        let style = child_style(tree, child);
        shares_context(&style)
            && (crate::layout::is_spanner(tree, usize::from(child))
                || contains_spanner(tree, child))
    })
}

/// The width of the margin box of `child` under `space`, measured as a whole.
fn outer_width(
    tree: &mut LayoutSession<'_>,
    child: NodeId,
    margin: f32,
    space: AvailableSpace,
    percent_height: Option<f32>,
) -> f32 {
    let parent_size = Size {
        width: None,
        height: percent_height,
    };
    let mut inputs = child_inputs(parent_size, None, RunMode::ComputeSize);
    inputs.available_space.width = space;
    let measured =
        measure_against_parent_height(tree, usize::from(child), percent_height, |tree| {
            tree.compute_child_layout_with_block_ctx(child, inputs, None)
        });
    measured.size.width + margin
}

/// The width of the content of `node` under `space`, as `(columns, spanners)`:
/// of what flows in the columns of a multicol container, and of what spans
/// them. Any other container has no spanners. `multicol` tells that `node` is
/// a multicol container or a block below one that shares its columns.
/// `percent_height` is the definite height of its content box, against which
/// a child's percentage height resolves while the child's width is measured.
fn intrinsic_widths(
    tree: &mut LayoutSession<'_>,
    node: NodeId,
    multicol: bool,
    space: AvailableSpace,
    percent_height: Option<f32>,
) -> (f32, f32) {
    let children: Vec<_> = tree.child_ids(node).collect();
    let mut width = 0.0_f32;
    let mut spanning = 0.0_f32;
    let mut floats = taffy::FloatIntrinsicWidthCalculator::new(space);
    for child in children {
        let style = child_style(tree, child);
        if style.box_generation_mode() == BoxGenerationMode::None
            || style.position == Position::Absolute
        {
            continue;
        }
        let margin = style
            .margin
            .map(|edge| edge.resolve_to_option(0.0, resolve_calc).unwrap_or(0.0));
        let spans = multicol && style.float == Float::None;
        if spans
            && shares_context(&style)
            && !crate::layout::is_spanner(tree, usize::from(child))
            && contains_spanner(tree, child)
        {
            // The box holds both columns and spanners; its own edges are
            // around each of them. A width of its own fixes what it adds to
            // the columns, and leaves the spanners below it as they are.
            let edges = margin.horizontal_axis_sum()
                + style
                    .padding
                    .resolve_or_zero(None, resolve_calc)
                    .horizontal_axis_sum()
                + style
                    .border
                    .resolve_or_zero(None, resolve_calc)
                    .horizontal_axis_sum();
            let (inner, spanners) = intrinsic_widths(tree, child, true, space, percent_height);
            let columns = if style.size.width.is_auto() {
                inner + edges
            } else {
                outer_width(
                    tree,
                    child,
                    margin.horizontal_axis_sum(),
                    space,
                    percent_height,
                )
            };
            width = width.max(columns);
            spanning = spanning.max(spanners + edges);
            continue;
        }
        let child_width = outer_width(
            tree,
            child,
            margin.horizontal_axis_sum(),
            space,
            percent_height,
        );
        match style.float {
            Float::Left => floats.add_float(child_width, taffy::FloatDirection::Left, style.clear),
            Float::Right => {
                floats.add_float(child_width, taffy::FloatDirection::Right, style.clear)
            }
            Float::None if spans && crate::layout::is_spanner(tree, usize::from(child)) => {
                spanning = spanning.max(child_width);
            }
            Float::None => width = width.max(child_width),
        }
    }
    (width.max(floats.result()), spanning)
}

/// The min-content width of the content of `node`, which a multicol container
/// counts across its columns.
fn min_content_width(
    tree: &mut LayoutSession<'_>,
    node: NodeId,
    percent_height: Option<f32>,
) -> f32 {
    let multicol = tree.nodes[usize::from(node)]
        .multicol
        .filter(|style| style.horizontal);
    let (columns, spanners) = intrinsic_widths(
        tree,
        node,
        multicol.is_some(),
        AvailableSpace::MinContent,
        percent_height,
    );
    match multicol {
        Some(multicol) => {
            multicol
                .intrinsic_inline_sizes((columns, columns), (spanners, spanners))
                .0
        }
        None => columns,
    }
}

pub(crate) fn compute_block_flow(
    tree: &mut LayoutSession<'_>,
    node: NodeId,
    inputs: LayoutInput,
    context: Option<&mut BlockContext<'_>>,
) -> LayoutOutput {
    let style = tree.get_block_container_style(node).clone();
    let replaced = crate::paint_rules::is_replaced_element(tree, usize::from(node));
    let content_box_ratio = tree.nodes[usize::from(node)].ratio_in_content_box;
    let metrics = Metrics::resolve(&style, replaced, content_box_ratio, inputs);
    let width = if let Some(width) = metrics.preferred.width {
        if metrics.ratio_width {
            let minimum = min_content_width(tree, node, metrics.percent_height)
                + metrics.inset.horizontal_axis_sum();
            // The width the content asks for is capped by the maximum the
            // box states, not by one carried over from its height, which
            // the automatic minimum overrides.
            width.max(minimum.maybe_clamp(None, metrics.own_max.width))
        } else {
            width
        }
    } else {
        let multicol = tree.nodes[usize::from(node)]
            .multicol
            .filter(|style| style.horizontal);
        let (maximum_columns, maximum_spanners) = intrinsic_widths(
            tree,
            node,
            multicol.is_some(),
            AvailableSpace::MaxContent,
            metrics.percent_height,
        );
        let (minimum_columns, minimum_spanners) = intrinsic_widths(
            tree,
            node,
            multicol.is_some(),
            AvailableSpace::MinContent,
            metrics.percent_height,
        );
        // The columns of a multicol container repeat the width of its content.
        let (minimum, maximum) = match multicol {
            Some(multicol) => multicol.intrinsic_inline_sizes(
                (minimum_columns, maximum_columns),
                (minimum_spanners, maximum_spanners),
            ),
            None => (minimum_columns, maximum_columns),
        };
        let maximum = maximum + metrics.inset.horizontal_axis_sum();
        let minimum = minimum + metrics.inset.horizontal_axis_sum();
        let fit = |limit: f32| maximum.min(limit).max(minimum);
        let width = match style.size.width.expand() {
            taffy::ExpandedDimension::MinContent => minimum,
            taffy::ExpandedDimension::MaxContent => maximum,
            // `fit-content(<length-percentage>)` takes the room its limit
            // makes, which `with_fit_content_room` offers before this runs.
            _ => match inputs.available_space.width {
                AvailableSpace::MaxContent => maximum,
                AvailableSpace::MinContent => minimum,
                AvailableSpace::Definite(limit) => fit(limit),
            },
        };
        metrics.clamp_width(width)
    };
    // A width the content settled, with no height to go by, gives the height
    // through the aspect ratio.
    let metrics = if style.aspect_ratio.is_some()
        && metrics.preferred.width.is_none()
        && metrics.preferred.height.is_none()
    {
        let mut sized = inputs;
        sized.known_dimensions.width = Some(width);
        Metrics::resolve(&style, replaced, content_box_ratio, sized)
    } else {
        metrics
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
        TextGroupScope::Probing | TextGroupScope::Skipping
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
    let outer = std::mem::replace(&mut tree.context.text_group, TextGroupScope::Probing);
    tree.context.text_group_shortest = None;
    let saved = shared.as_deref().map(BlockContext::snapshot);
    run(tree, shared.as_deref_mut());
    if let (Some(ctx), Some(saved)) = (shared.as_deref_mut(), &saved) {
        ctx.restore(saved);
    }
    let padding = tree.context.text_group_shortest.take().unwrap_or(0.0);
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
    let height = match metrics.preferred.height {
        // The content raises the height up to the maximum the box states,
        // not one carried over from its width.
        Some(height) if metrics.ratio_height => height
            .max((natural_end + metrics.inset.bottom).maybe_clamp(None, metrics.own_max.height)),
        Some(height) => height,
        None => metrics.clamp_height(natural_end + metrics.inset.bottom),
    };
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
        oof_candidates: flow.oof_candidates,
        oof_positioning_area: (inputs.run_mode == RunMode::PerformLayout)
            .then(|| positioned::positioning_area(metrics, size)),
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
