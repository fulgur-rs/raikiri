//! Place block-flow children while retaining one shared float transaction.

use super::context::BlockContext;
use super::{Metrics, child_inputs, child_style, commit, shares_context};
use crate::Document;
use crate::taffy_impl::resolve_calc;
use taffy::{
    AlignContentKeyword, AlignmentSafety, AvailableSpace, Baselines, BlockItemStyle,
    BoxGenerationMode, Clear, CollapsibleMarginSet, CoreStyle, Float, FloatDirection, LayoutInput,
    LayoutOutput, Line, MaybeResolve, NodeId, Point, Position, Rect, RunMode, Size, Style,
    TraversePartialTree,
};

pub(super) struct Flow {
    pub end: f32,
    pub leading: CollapsibleMarginSet,
    pub trailing: CollapsibleMarginSet,
    pub through: bool,
    pub overflow: Rect<f32>,
    pub baselines: Baselines,
    pub placed: Vec<NodeId>,
    pub positioned: Vec<super::positioned::Candidate>,
}

fn margins(style: &Style, basis: f32) -> Rect<f32> {
    style
        .margin
        .map(|edge| edge.resolve_to_option(basis, resolve_calc).unwrap_or(0.0))
}

fn relative(style: &Style, parent: Size<Option<f32>>) -> Point<f32> {
    if !matches!(style.position, Position::Relative | Position::Sticky) {
        return Point::ZERO;
    }
    let inset = taffy::Rect {
        left: style.inset.left.maybe_resolve(parent.width, resolve_calc),
        right: style.inset.right.maybe_resolve(parent.width, resolve_calc),
        top: style.inset.top.maybe_resolve(parent.height, resolve_calc),
        bottom: style
            .inset
            .bottom
            .maybe_resolve(parent.height, resolve_calc),
    };
    Point {
        x: inset
            .left
            .or(inset.right.map(|right| -right))
            .unwrap_or(0.0),
        y: inset
            .top
            .or(inset.bottom.map(|bottom| -bottom))
            .unwrap_or(0.0),
    }
}

fn overflow(
    flow: &mut Flow,
    style: &Style,
    metrics: Metrics,
    output: &LayoutOutput,
    location: Point<f32>,
) {
    let mut rect = Rect {
        left: location.x,
        right: location.x + output.size.width,
        top: location.y,
        bottom: location.y + output.size.height,
    };
    let child = output.scrollable_overflow_rect;
    if !style.overflow.x.is_scroll_container() && style.overflow.x != taffy::Overflow::Clip {
        let (left, right) = if style.direction == taffy::Direction::Rtl {
            let origin = location.x + output.size.width - metrics.border.right;
            (origin - child.right, origin - child.left)
        } else {
            (
                location.x + metrics.border.left + child.left,
                location.x + metrics.border.left + child.right,
            )
        };
        rect.left = rect.left.min(left);
        rect.right = rect.right.max(right);
    }
    if !style.overflow.y.is_scroll_container() && style.overflow.y != taffy::Overflow::Clip {
        rect.top = rect.top.min(location.y + metrics.border.top + child.top);
        rect.bottom = rect
            .bottom
            .max(location.y + metrics.border.top + child.bottom);
    }
    flow.overflow.left = flow.overflow.left.min(rect.left);
    flow.overflow.right = flow.overflow.right.max(rect.right);
    flow.overflow.top = flow.overflow.top.min(rect.top);
    flow.overflow.bottom = flow.overflow.bottom.max(rect.bottom);
}

#[allow(clippy::too_many_arguments)]
pub(super) fn layout(
    tree: &mut Document,
    node: NodeId,
    inputs: LayoutInput,
    container: &Style,
    metrics: Metrics,
    width: f32,
    ctx: &mut BlockContext<'_>,
    collapse_start: bool,
    collapse_end: bool,
) -> Flow {
    let content_width = (width - metrics.inset.horizontal_axis_sum()).max(0.0);
    let parent = Size {
        width: Some(content_width),
        height: metrics.percent_height,
    };
    let mut flow = Flow {
        end: metrics.inset.top,
        leading: CollapsibleMarginSet::ZERO,
        trailing: CollapsibleMarginSet::ZERO,
        through: true,
        overflow: Rect::ZERO,
        baselines: Baselines::NONE,
        placed: Vec::new(),
        positioned: Vec::new(),
    };
    let children: Vec<_> = tree.child_ids(node).collect();
    let mut pending = CollapsibleMarginSet::ZERO;
    let mut leading = true;
    if !collapse_start {
        ctx.commit_strut();
    }
    for (order, child) in children.into_iter().enumerate() {
        let style = child_style(tree, child);
        if style.box_generation_mode() == BoxGenerationMode::None {
            if inputs.run_mode == RunMode::PerformLayout {
                tree.compute_child_layout_with_block_ctx(child, LayoutInput::HIDDEN, None);
            }
            continue;
        }
        let mut margin = margins(&style, content_width);
        if style.position == Position::Absolute {
            flow.positioned.push(super::positioned::Candidate {
                node: child,
                order,
                static_position: Point {
                    x: metrics.inset.left,
                    y: flow.end + pending.resolve(),
                },
            });
            continue;
        }
        let mut child_input = child_inputs(parent, None, inputs.run_mode);
        let child_metrics = Metrics::resolve(&style, child_input);
        child_input.known_dimensions.height = child_metrics.preferred.height;
        if style.float != Float::None {
            child_input.known_dimensions.width = child_metrics.preferred.width;
            child_input.available_space.width =
                AvailableSpace::Definite((content_width - margin.horizontal_axis_sum()).max(0.0));
            let output = tree.compute_child_layout_with_block_ctx(child, child_input, None);
            let position = ctx.place_floated_box(
                output.size + margin.sum_axes(),
                if leading && collapse_start {
                    flow.end
                } else {
                    flow.end + pending.resolve()
                },
                if style.float == Float::Left {
                    FloatDirection::Left
                } else {
                    FloatDirection::Right
                },
                style.clear,
                leading && collapse_start,
            );
            let location = Point {
                x: position.x + margin.left,
                y: position.y + margin.top,
            };
            overflow(&mut flow, &style, child_metrics, &output, location);
            if inputs.run_mode == RunMode::PerformLayout {
                commit(tree, child, order, child_metrics, &output, location, margin);
            }
            continue;
        }
        let same = shares_context(&style);
        let mut child_width = child_metrics.preferred.width;
        let natural_stretch = content_width - margin.horizontal_axis_sum();
        if child_width.is_none()
            && matches!(
                style.size.width.expand(),
                taffy::ExpandedDimension::Auto | taffy::ExpandedDimension::Stretch
            )
            && !style.is_table()
            && !style.is_compressible_replaced()
        {
            child_width = Some(child_metrics.clamp_width(natural_stretch.max(0.0)));
        }
        let mut left = metrics.inset.left + margin.left;
        let mut slot_y = None;
        if !same && ctx.has_active_floats(flow.end + pending.resolve()) {
            let mut after = None;
            loop {
                let slot = ctx.find_bfc_slot(
                    flow.end + pending.collapse_with_margin(margin.top).resolve(),
                    [margin.left, margin.right],
                    container.direction,
                    style.clear,
                    after,
                );
                let proposed = child_metrics
                    .preferred
                    .width
                    .unwrap_or_else(|| child_metrics.clamp_width(slot.stretch_width.max(0.0)));
                if proposed <= slot.border_width + 0.001 || slot.segment_id.is_none() {
                    child_width = Some(proposed);
                    left = slot.x;
                    slot_y = Some(slot.y);
                    break;
                }
                after = slot.segment_id;
            }
        } else if let Some(child_width) = child_width {
            let free = (content_width - margin.horizontal_axis_sum() - child_width).max(0.0);
            match (style.margin.left.is_auto(), style.margin.right.is_auto()) {
                (true, true) => {
                    margin.left = free / 2.0;
                    margin.right = free / 2.0;
                }
                (true, false) => margin.left = free,
                (false, true) => margin.right = free,
                (false, false) if container.direction == taffy::Direction::Rtl => {
                    margin.left = content_width - margin.right - child_width
                }
                _ => {}
            }
            left = metrics.inset.left + margin.left;
        }
        child_input.known_dimensions.width = child_width;
        child_input.available_space.width =
            child_width.map_or(AvailableSpace::MaxContent, AvailableSpace::Definite);
        child_input.vertical_margins_are_collapsible = if same { Line::TRUE } else { Line::FALSE };
        let resolved = ctx.origin_resolved();
        // Clearance refers to preceding floats. Descendant floats created
        // by a speculative child must never push that same child downward.
        let clear_y = ctx
            .cleared_threshold(style.clear)
            .unwrap_or(f32::NEG_INFINITY);
        let adjoining_clearance = ctx.has_adjoining_float(style.clear);
        let saved = ctx.snapshot();
        let joins = leading && collapse_start;
        let mut final_pass = resolved && joins && style.clear == Clear::None;
        let mut guess = slot_y
            .unwrap_or(if joins {
                flow.end
            } else {
                flow.end + pending.collapse_with_margin(margin.top).resolve()
            })
            .max(clear_y);
        let (output, top, top_set, bottom_set, clearance, floated_end, adjoining) = loop {
            let (output, floated_end, adjoining) = if same {
                let right_inset = width - left - child_width.unwrap_or(0.0);
                let mut sub = ctx.sub_context_for_collapsing_child(
                    guess,
                    [left, right_inset],
                    flow.end,
                    pending.collapse_with_margin(margin.top),
                    joins,
                );
                sub.set_origin_resolved(final_pass);
                let output =
                    tree.compute_child_layout_with_block_ctx(child, child_input, Some(&mut sub));
                (
                    output,
                    sub.floated_content_height_contribution(),
                    sub.top_adjoining_floats(),
                )
            } else {
                (
                    tree.compute_child_layout_with_block_ctx(child, child_input, None),
                    f32::NEG_INFINITY,
                    [false; 2],
                )
            };
            let top_set = output.top_margin.collapse_with_margin(margin.top);
            let bottom_set = output.bottom_margin.collapse_with_margin(margin.bottom);
            let hypothetical = if joins && resolved {
                ctx.hypothetical_top_position(pending.collapse_with_set(top_set))
                    .unwrap_or(flow.end)
            } else {
                flow.end + pending.collapse_with_set(top_set).resolve()
            };
            let clearance = style.clear != Clear::None
                && ((joins && adjoining_clearance) || hypothetical < clear_y);
            let top = if let Some(slot_y) = slot_y {
                slot_y
            } else if clearance {
                clear_y
            } else if joins {
                flow.end
            } else {
                flow.end + pending.collapse_with_set(top_set).resolve()
            };
            if resolved && same && ctx.has_floats() && (!final_pass || (guess - top).abs() > 0.001)
            {
                ctx.restore(&saved);
                guess = top;
                final_pass = true;
                continue;
            }
            break (
                output,
                top,
                top_set,
                bottom_set,
                clearance,
                floated_end,
                adjoining,
            );
        };
        ctx.add_child_floated_content_height_contribution(guess + floated_end);
        ctx.merge_adjoining_floats(adjoining);
        if joins && !clearance {
            flow.leading = flow
                .leading
                .collapse_with_set(pending)
                .collapse_with_set(top_set);
        }
        let offset = relative(&style, parent);
        let location = Point {
            x: left + offset.x,
            y: top + offset.y,
        };
        overflow(&mut flow, &style, child_metrics, &output, location);
        if let Some(baseline) = output.baselines.first {
            flow.baselines.first.get_or_insert(top + baseline);
        }
        flow.baselines.last = output
            .baselines
            .last
            .or(output.baselines.first)
            .map(|baseline| top + baseline)
            .or(flow.baselines.last);
        if inputs.run_mode == RunMode::PerformLayout {
            commit(tree, child, order, child_metrics, &output, location, margin);
            flow.placed.push(child);
        }
        if output.margins_can_collapse_through && !clearance {
            pending = pending
                .collapse_with_set(top_set)
                .collapse_with_set(bottom_set);
            if joins {
                flow.leading = flow.leading.collapse_with_set(bottom_set);
            }
        } else {
            flow.end = top + output.size.height;
            pending = bottom_set;
            leading = false;
            flow.through = false;
            ctx.commit_strut();
        }
    }
    flow.trailing = pending;
    if !collapse_end {
        flow.end += pending.resolve();
    }
    flow
}

pub(super) fn align(tree: &mut Document, style: &Style, placed: &[NodeId], free: f32) {
    let align = style.align_content;
    let free = if align.safety == AlignmentSafety::Safe {
        free.max(0.0)
    } else {
        free
    };
    let offset = match align.keyword {
        AlignContentKeyword::End | AlignContentKeyword::FlexEnd => free,
        AlignContentKeyword::Center => free / 2.0,
        _ => 0.0,
    };
    if offset == 0.0 {
        return;
    }
    for &node in placed {
        let mut layout = tree.nodes[usize::from(node)].unrounded_layout;
        layout.location.y += offset;
        taffy::LayoutPartialTree::set_unrounded_layout(tree, node, &layout);
    }
}
