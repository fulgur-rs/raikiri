//! Place block-flow children while retaining one shared float transaction.

use super::context::BlockContext;
use super::{Metrics, child_inputs, child_style, commit, shares_context};
use crate::layout::context::LayoutSession;
use crate::taffy_impl::resolve_calc;
use taffy::{
    AvailableSpace, Baselines, BlockItemStyle, BoxGenerationMode, Clear, CollapsibleMarginSet,
    CoreStyle, Float, FloatDirection, LayoutInput, LayoutOutput, Line, MaybeResolve, NodeId, Point,
    Position, Rect, RunMode, Size, Style, TraversePartialTree,
};

pub(super) struct Flow {
    pub end: f32,
    pub leading: CollapsibleMarginSet,
    pub trailing: CollapsibleMarginSet,
    pub through: bool,
    pub overflow: Rect<f32>,
    pub float_overflow: Rect<f32>,
    pub baselines: Baselines,
    pub placed: Vec<NodeId>,
    pub positioned: Vec<super::positioned::Candidate>,
}

fn place_inline_axis(
    style: &Style,
    direction: taffy::Direction,
    container_width: f32,
    insets: [f32; 2],
    (area_start, area_width): (f32, f32),
    used_width: f32,
    margin: &mut Rect<f32>,
) -> f32 {
    let free = area_width - used_width;
    let (left_auto, right_auto) = (style.margin.left.is_auto(), style.margin.right.is_auto());
    let (auto_left, auto_right) = match (left_auto, right_auto) {
        (true, true) => (free.max(0.0) / 2.0, free.max(0.0) / 2.0),
        (true, false) => (free.max(0.0), 0.0),
        (false, true) => (0.0, free.max(0.0)),
        _ => (0.0, 0.0),
    };
    let offset = if direction == taffy::Direction::Rtl && (free < 0.0 || !left_auto && !right_auto)
    {
        free
    } else {
        auto_left
    };
    if left_auto {
        margin.left = auto_left + (area_start - insets[0]).max(0.0);
    }
    if right_auto {
        margin.right =
            auto_right + (container_width - insets[1] - area_start - area_width).max(0.0);
    }
    area_start + offset
}

fn margins(style: &Style, basis: f32) -> Rect<f32> {
    style
        .margin
        .map(|edge| edge.resolve_to_option(basis, resolve_calc).unwrap_or(0.0))
}

pub(super) fn relative(
    style: &Style,
    parent: Size<Option<f32>>,
    direction: taffy::Direction,
) -> Point<f32> {
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
        x: if direction == taffy::Direction::Rtl {
            inset.right.map(|right| -right).or(inset.left)
        } else {
            inset.left.or(inset.right.map(|right| -right))
        }
        .unwrap_or(0.0),
        y: inset
            .top
            .or(inset.bottom.map(|bottom| -bottom))
            .unwrap_or(0.0),
    }
}

fn overflow(
    style: &Style,
    metrics: Metrics,
    output: &LayoutOutput,
    location: Point<f32>,
) -> Rect<f32> {
    let mut rect = Rect {
        left: location.x,
        right: location.x + output.size.width,
        top: location.y,
        bottom: location.y + output.size.height,
    };
    let child = output.scrollable_overflow_rect;
    let contains_overflow = style.contain.contains_scrollable_overflow()
        || style.overflow.x.is_scroll_container()
        || style.overflow.y.is_scroll_container();
    if !contains_overflow && style.overflow.x == taffy::Overflow::Visible {
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
    if !contains_overflow && style.overflow.y == taffy::Overflow::Visible {
        rect.top = rect.top.min(location.y + metrics.border.top + child.top);
        rect.bottom = rect
            .bottom
            .max(location.y + metrics.border.top + child.bottom);
    }
    rect
}

#[allow(clippy::too_many_arguments)]
pub(super) fn layout(
    tree: &mut LayoutSession<'_>,
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
        float_overflow: Rect::ZERO,
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
            flow.float_overflow =
                flow.float_overflow
                    .union(overflow(&style, child_metrics, &output, location));
            if inputs.run_mode == RunMode::PerformLayout {
                commit(tree, child, order, child_metrics, &output, location, margin);
            }
            continue;
        }
        let same = shares_context(&style);
        let stretches = matches!(
            style.size.width.expand(),
            taffy::ExpandedDimension::Auto | taffy::ExpandedDimension::Stretch
        ) && !style.is_table()
            && !style.is_compressible_replaced();
        let natural_stretch = content_width - margin.horizontal_axis_sum();
        let mut measure_width = |limit: f32| {
            if let Some(preferred) = child_metrics.preferred.width {
                return preferred;
            }
            if stretches {
                return child_metrics.clamp_width(limit.max(0.0));
            }
            let mut measure = child_input;
            measure.run_mode = RunMode::ComputeSize;
            measure.available_space.width = match style.size.width.expand() {
                taffy::ExpandedDimension::MinContent => AvailableSpace::MinContent,
                taffy::ExpandedDimension::MaxContent => AvailableSpace::MaxContent,
                _ => AvailableSpace::Definite(limit.max(0.0)),
            };
            tree.compute_child_layout_with_block_ctx(child, measure, None)
                .size
                .width
        };
        let mut child_width = measure_width(natural_stretch);
        let mut area_start = metrics.inset.left + margin.left;
        let mut area_width = natural_stretch;
        let mut slot_y = None;
        // Whether a box establishing a formatting context moved down past
        // floats. Like clearance, that separates its top margin from the
        // margins of its parent.
        let mut separated = false;
        let joins = leading && collapse_start;
        let adjoining_clearance = ctx.has_adjoining_float(style.clear);
        if !same {
            let margins = pending.collapse_with_margin(margin.top);
            let hypothetical = if joins {
                ctx.hypothetical_top_position(margins)
                    .unwrap_or(flow.end + margins.resolve())
            } else {
                flow.end + margins.resolve()
            };
            // A float adjoining the unresolved strut moves with it while the
            // margins escape, so the box is first tried beside that float
            // without its margin. A float the box clears forces clearance.
            let adjoining = joins && ctx.has_adjoining_float(Clear::Both);
            let forced = joins && adjoining_clearance;
            // Moving below this point separates the box from the floats.
            let threshold = if adjoining { flow.end } else { hypothetical };
            let search = if forced {
                ctx.cleared_threshold(style.clear).unwrap_or(hypothetical)
            } else if adjoining {
                flow.end
            } else {
                hypothetical
            };
            if forced || ctx.has_active_floats(search) {
                let mut after = None;
                loop {
                    let slot = ctx.find_bfc_slot(
                        search,
                        [margin.left, margin.right],
                        container.direction,
                        style.clear,
                        after,
                    );
                    let proposed = measure_width(slot.stretch_width);
                    if proposed <= slot.border_width + 0.001 || slot.segment_id.is_none() {
                        child_width = proposed;
                        area_start = slot.x;
                        area_width = slot.stretch_width;
                        separated = forced || slot.y > threshold + 0.001;
                        slot_y = Some(if joins && !separated {
                            flow.end
                        } else {
                            slot.y
                        });
                        break;
                    }
                    after = slot.segment_id;
                }
            }
        }
        let left = place_inline_axis(
            &style,
            container.direction,
            width,
            [metrics.inset.left, metrics.inset.right],
            (area_start, area_width),
            child_width,
            &mut margin,
        );
        child_input.known_dimensions.width = Some(child_width);
        child_input.available_space.width = AvailableSpace::Definite(child_width);
        child_input.vertical_margins_are_collapsible = if same { Line::TRUE } else { Line::FALSE };
        let resolved = ctx.origin_resolved();
        // Clearance refers to preceding floats. Descendant floats created
        // by a speculative child must never push that same child downward.
        let clear_y = ctx
            .cleared_threshold(style.clear)
            .unwrap_or(f32::NEG_INFINITY);
        let saved = ctx.snapshot();
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
                let right_inset = width - left - child_width;
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
            } else if clearance && joins && !resolved && !adjoining_clearance {
                // A provisional origin leaves out part of the margins that
                // escape above this block, so an earlier float's edge sits
                // that much higher in its final coordinates. A float on the
                // same strut moves with the block instead.
                let escaped = ctx
                    .hypothetical_top_position(pending)
                    .map_or(pending.resolve(), |y| y - flow.end);
                clear_y - escaped
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
        if joins && separated {
            flow.leading = flow.leading.collapse_with_set(pending);
        } else if joins && !clearance {
            flow.leading = flow
                .leading
                .collapse_with_set(pending)
                .collapse_with_set(top_set);
        }
        // A specialized child can return a used width different from the
        // width supplied by this parent. Position its final border box.
        let left = place_inline_axis(
            &style,
            container.direction,
            width,
            [metrics.inset.left, metrics.inset.right],
            (area_start, area_width),
            output.size.width,
            &mut margin,
        );
        let offset = relative(&style, parent, container.direction);
        let location = Point {
            x: left + offset.x,
            y: top + offset.y,
        };
        flow.overflow = flow
            .overflow
            .union(overflow(&style, child_metrics, &output, location));
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

pub(super) fn align(tree: &mut LayoutSession<'_>, style: &Style, flow: &mut Flow, free: f32) {
    if style.align_content.keyword == taffy::AlignContentKeyword::Normal {
        return;
    }
    let offset = taffy::compute_block_align_content_offset(style.align_content, free);
    if offset == 0.0 {
        return;
    }
    flow.baselines.first = flow.baselines.first.map(|baseline| baseline + offset);
    flow.baselines.last = flow.baselines.last.map(|baseline| baseline + offset);
    if flow.overflow != Rect::ZERO {
        flow.overflow.top += offset;
        flow.overflow.bottom += offset;
    }
    for candidate in &mut flow.positioned {
        candidate.static_position.y += offset;
    }
    for &node in &flow.placed {
        let mut layout = tree.nodes[usize::from(node)].unrounded_layout;
        layout.location.y += offset;
        taffy::LayoutPartialTree::set_unrounded_layout(tree, node, &layout);
    }
}
