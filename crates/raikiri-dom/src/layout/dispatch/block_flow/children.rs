//! Place block-flow children while retaining one shared float transaction.

use super::context::{BlockContext, BoxProbe};
use super::{Metrics, child_inputs, child_style, commit, shares_context, shares_parent_context};
use crate::layout::context::LayoutSession;
use crate::layout::dispatch::content_limits;
use crate::layout::protocol::{
    AvailableSpace, AxisStaticEdge, AxisStaticPosition, Baselines, CollapsibleMarginSet,
    FloatDirection, LayoutInput, LayoutOutput, NodeId, OofCandidates, OofPosition, RunMode,
};
use crate::layout::taffy_adapter::algorithms::compute_block_align_content_offset;
use crate::node::FormattingContext;
use raikiri_layout_geometry::{Edges, Line, Point, Size};
use raikiri_layout_style::BoxStyle as Style;
use raikiri_layout_style::values::{
    AlignContent, AlignContentKeyword, AlignmentSafety, BoxGenerationMode, Clear, Direction,
    ExpandedDimension, Float, MaybeResolve, Overflow, Position,
};
use raikiri_style::property::{DisplayValue, TextAlign, TextAlignLast};

pub(super) struct Flow {
    pub end: f32,
    pub leading: CollapsibleMarginSet,
    pub trailing: CollapsibleMarginSet,
    pub through: bool,
    pub overflow: Edges<f32>,
    pub float_overflow: Edges<f32>,
    pub baselines: Baselines,
    pub placed: Vec<NodeId>,
    pub oof_candidates: OofCandidates,
}

fn place_inline_axis(
    style: &Style,
    direction: Direction,
    container_width: f32,
    insets: [f32; 2],
    (area_start, area_width): (f32, f32),
    used_width: f32,
    margin: &mut Edges<f32>,
) -> f32 {
    let free = area_width - used_width;
    let (left_auto, right_auto) = (style.margin.left.is_auto(), style.margin.right.is_auto());
    let (auto_left, auto_right) = match (left_auto, right_auto) {
        (true, true) => (free.max(0.0) / 2.0, free.max(0.0) / 2.0),
        (true, false) => (free.max(0.0), 0.0),
        (false, true) => (0.0, free.max(0.0)),
        _ => (0.0, 0.0),
    };
    let offset = if direction == Direction::Rtl && (free < 0.0 || !left_auto && !right_auto) {
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

fn margins(style: &Style, basis: f32) -> Edges<f32> {
    style
        .margin
        .map(|edge| edge.resolve(Some(basis)).unwrap_or(0.0))
}

fn supports_inline_static_position(tree: &LayoutSession<'_>, parent: NodeId) -> bool {
    let index = usize::from(parent);
    let node = &tree.nodes[index];
    node.ifc.is_none()
        && node.multicol.is_none()
        && !crate::layout::dispatch::used_writing_mode(tree, index).is_vertical()
        && node.formatting_context == FormattingContext::Block
        && matches!(
            node.display(),
            DisplayValue::Block
                | DisplayValue::InlineBlock
                | DisplayValue::FlowRoot
                | DisplayValue::ListItem
                | DisplayValue::TableCell
                | DisplayValue::TableCaption
        )
}

pub(super) fn is_non_atomic_inline(tree: &LayoutSession<'_>, child: NodeId) -> bool {
    use crate::layout::ifc::projection::{ATOMIC_TAGS, REPLACED_BOX_TAGS};

    let node = &tree.nodes[usize::from(child)];
    let tag = node.tag_name().unwrap_or("");
    node.display() == DisplayValue::Inline
        && !node.is_inline_svg_root()
        && !ATOMIC_TAGS.contains(&tag)
        && !REPLACED_BOX_TAGS.contains(&tag)
}

fn static_inline_share(
    text_align: TextAlign,
    text_align_last: TextAlignLast,
    direction: Direction,
) -> f32 {
    let start = if direction == Direction::Rtl {
        1.0
    } else {
        0.0
    };
    let end = 1.0 - start;
    match text_align_last {
        TextAlignLast::Start => start,
        TextAlignLast::End => end,
        TextAlignLast::Left => 0.0,
        TextAlignLast::Right => 1.0,
        TextAlignLast::Center | TextAlignLast::Justify => 0.5,
        TextAlignLast::Auto | TextAlignLast::MatchParent => {
            match text_align {
                TextAlign::Start => start,
                TextAlign::End => end,
                TextAlign::Left => 0.0,
                TextAlign::Right => 1.0,
                TextAlign::Center | TextAlign::InternalCenter => 0.5,
                // `justify-all` also justifies the last line. A hypothetical
                // line without expandable content uses the specified center
                // fallback for justification.
                TextAlign::JustifyAll => 0.5,
                // A lone hypothetical line has no text to justify, so it
                // falls back to the line's logical start edge when only the
                // ordinary `justify` value applies.
                TextAlign::Justify | TextAlign::MatchParent | TextAlign::Inherit => start,
                _ => start, // cov:ignore: non-exhaustive TextAlign may add future variants
            }
        }
        _ => start, // cov:ignore: non-exhaustive TextAlignLast may add future variants
    }
}

struct StaticLine<'context, 'owner> {
    from: f32,
    context: &'context BlockContext<'owner>,
    direction: Direction,
    text_align: TextAlign,
    text_align_last: TextAlignLast,
}

fn inline_static_position(
    tree: &mut LayoutSession<'_>,
    child: NodeId,
    style: &Style,
    parent_size: Size<Option<f32>>,
    line: StaticLine<'_, '_>,
) -> Point<AxisStaticPosition> {
    let mut after = None;
    loop {
        let slot = line
            .context
            .find_content_slot(line.from, Clear::None, after);
        // CSS 2.1 §10.3.7 permits estimating the hypothetical box's probable
        // position. Non-atomic inline candidates use a zero-width point.
        let child_margins = margins(style, parent_size.width.unwrap_or(slot.width));
        let hypothetical_margin_width = child_margins.horizontal_axis_sum();
        let hypothetical_width = if is_non_atomic_inline(tree, child) {
            0.0
        } else {
            let mut input = child_inputs(parent_size, None, RunMode::ComputeSize);
            input.available_space.width =
                AvailableSpace::Definite((slot.width - hypothetical_margin_width).max(0.0));
            let measured = tree.compute_child_layout_with_block_ctx(child, input, None);
            measured.size.width + hypothetical_margin_width
        };

        let does_not_fit = slot.width <= 0.0 || hypothetical_width > slot.width + 0.001;
        if does_not_fit && let Some(segment) = slot.segment_id {
            after = Some(segment);
            continue;
        }

        let start = slot.x
            + (slot.width - hypothetical_width).max(0.0)
                * static_inline_share(line.text_align, line.text_align_last, line.direction);
        let (anchor, edge) = if line.direction == Direction::Rtl {
            (start + hypothetical_width, AxisStaticEdge::End)
        } else {
            (start, AxisStaticEdge::Start)
        };
        return Point {
            x: AxisStaticPosition::from_edge(anchor, edge),
            y: AxisStaticPosition::from_edge(slot.y, AxisStaticEdge::Start),
        };
    }
}

pub(super) fn relative(
    style: &Style,
    parent: Size<Option<f32>>,
    direction: Direction,
) -> Point<f32> {
    if !matches!(style.position, Position::Relative | Position::Sticky) {
        return Point::ZERO;
    }
    let inset = Edges {
        left: style.inset.left.maybe_resolve(parent.width),
        right: style.inset.right.maybe_resolve(parent.width),
        top: style.inset.top.maybe_resolve(parent.height),
        bottom: style.inset.bottom.maybe_resolve(parent.height),
    };
    Point {
        x: if direction == Direction::Rtl {
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
) -> Edges<f32> {
    let mut rect = Edges {
        left: location.x,
        right: location.x + output.size.width,
        top: location.y,
        bottom: location.y + output.size.height,
    };
    let child = output.scrollable_overflow_rect;
    let contains_overflow = style.contain.contains_scrollable_overflow()
        || style.overflow.x.is_scroll_container()
        || style.overflow.y.is_scroll_container();
    if !contains_overflow && style.overflow.x == Overflow::Visible {
        let (left, right) = if style.direction == Direction::Rtl {
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
    if !contains_overflow && style.overflow.y == Overflow::Visible {
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
    // A margin at an edge of this box that its parent's margins cannot absorb
    // stays inside it: it takes room off a child that stretches to the height
    // of the content box. A margin that would escape takes none.
    let holds_margin = Line {
        start: !collapse_start,
        end: !(shares_context(container)
            && inputs.vertical_margins_are_collapsible.end
            && metrics.inset.bottom == 0.0),
    };
    let mut flow = Flow {
        end: metrics.inset.top,
        leading: CollapsibleMarginSet::ZERO,
        trailing: CollapsibleMarginSet::ZERO,
        through: true,
        overflow: Edges::ZERO,
        float_overflow: Edges::ZERO,
        baselines: Baselines::NONE,
        placed: Vec::new(),
        oof_candidates: OofCandidates::new(),
    };
    let children: Vec<_> = tree.child_ids(node).collect();
    let mut pending = CollapsibleMarginSet::ZERO;
    let mut leading = true;
    if !collapse_start {
        ctx.commit_strut();
    }
    for (order, child) in children.into_iter().enumerate() {
        let mut style = child_style(tree, child);
        if style.box_generation_mode() == BoxGenerationMode::None {
            if inputs.run_mode == RunMode::PerformLayout {
                tree.compute_child_layout_with_block_ctx(child, LayoutInput::HIDDEN, None);
            }
            continue;
        }
        let mut margin = margins(&style, content_width);
        if style.position == Position::Absolute {
            if inputs.run_mode == RunMode::PerformLayout {
                let static_y = flow.end + pending.resolve();
                let child_index = usize::from(child);
                let node_inline = tree.nodes[child_index].display().is_inline_level();
                let resolve_inline = node_inline && supports_inline_static_position(tree, node);
                let static_position = if resolve_inline {
                    let (text_align, text_align_last) = {
                        let parent_node = &tree.nodes[usize::from(node)];
                        (
                            parent_node.static_position_text_align,
                            parent_node.static_position_text_align_last,
                        )
                    };
                    inline_static_position(
                        tree,
                        child,
                        &style,
                        parent,
                        StaticLine {
                            from: static_y,
                            context: ctx,
                            direction: container.direction,
                            text_align,
                            text_align_last,
                        },
                    )
                } else {
                    let inline_area = Line {
                        start: metrics.inset.left,
                        end: width - metrics.inset.right,
                    };
                    super::positioned::aligned_static_position(
                        container,
                        &style,
                        container.direction,
                        inline_area,
                        static_y,
                    )
                };
                super::positioned::push_candidate(
                    &mut flow.oof_candidates,
                    child,
                    order,
                    if tree.nodes[usize::from(child)].fixed_position
                        || tree.nodes[usize::from(child)].position()
                            == &raikiri_style::property::PositionValue::Fixed
                        || style.position == Position::Fixed
                    {
                        OofPosition::Fixed
                    } else {
                        OofPosition::Absolute
                    },
                    static_position,
                );
            }
            continue;
        }
        let mut child_input = child_inputs(parent, None, inputs.run_mode);
        // A keyword `min-width` or `max-width` of the child limits the width
        // this container gives it; the child's lines are laid out in a
        // context prepared for that width.
        let mut room = child_input;
        room.available_space.width =
            AvailableSpace::Definite((content_width - margin.horizontal_axis_sum()).max(0.0));
        crate::layout::dispatch::content_limits::resolve_width_in_style(
            tree, child, &mut style, &room,
        );
        // `height: stretch` fills the content box of a parent whose height is
        // definite, less the margins that do not escape it (CSS Sizing 4
        // §5.1). A float's margins never escape. Without a definite room it
        // sizes like `auto`.
        let stretch_room = parent.height.map(|height| {
            let floated = style.float != Float::None;
            let held = |holds: bool, margin: f32| if holds || floated { margin } else { 0.0 };
            let taken =
                held(holds_margin.start, margin.top) + held(holds_margin.end, margin.bottom);
            (height - taken).max(0.0)
        });
        // A box with an aspect ratio carries a `stretch` limit of its height
        // across to its width.
        content_limits::resolve_stretch_height_limits_in_style(
            tree,
            child,
            &mut style,
            stretch_room,
            content_width,
        );
        // The height a `stretch` one gives is known before the box is
        // measured, so an `auto` width follows from it through the aspect
        // ratio as it does from a height the box states, and no bound carried
        // across the ratio overrides it.
        let stretched_height = stretch_room
            .filter(|_| matches!(style.size.height.expand(), ExpandedDimension::Stretch));
        child_input.known_dimensions.height = stretched_height;
        // The width `stretch` fills is the room the parent offers.
        let mut metrics_input = child_input;
        metrics_input.available_space.width = room.available_space.width;
        let child_metrics = Metrics::resolve(
            &style,
            tree.nodes[usize::from(child)].svg_width_follows_available,
            tree.nodes[usize::from(child)].ratio_in_content_box,
            metrics_input,
        );
        // A size that only follows from the aspect ratio is left for the child
        // to resolve, since its content may raise it.
        child_input.known_dimensions.height = match stretched_height {
            Some(height) => Some(child_metrics.clamp_height(height)),
            None => child_metrics
                .preferred
                .height
                .filter(|_| !child_metrics.ratio_height),
        };
        if style.float != Float::None {
            child_input.known_dimensions.width = child_metrics
                .preferred
                .width
                .filter(|_| !child_metrics.ratio_width);
            child_input.available_space.width =
                AvailableSpace::Definite((content_width - margin.horizontal_axis_sum()).max(0.0));
            if child_input.known_dimensions.width.is_none() {
                // Taffy reads a `min-content` or `max-content` width of a flex
                // or grid container in the room it is offered, and a paragraph
                // root shrinks to fit that room; the width the keyword stands
                // for is what the box measures at.
                let along = match style.size.width.expand() {
                    ExpandedDimension::MinContent => Some(AvailableSpace::MinContent),
                    ExpandedDimension::MaxContent => Some(AvailableSpace::MaxContent),
                    _ => None,
                };
                if let Some(along) = along.filter(|_| {
                    let node = &tree.nodes[usize::from(child)];
                    node.display().is_flex_or_grid_container() || node.is_ifc_root()
                }) {
                    let mut measure = child_input;
                    measure.run_mode = RunMode::ComputeSize;
                    measure.available_space.width = along;
                    let width = tree
                        .compute_child_layout_with_block_ctx(child, measure, None)
                        .size
                        .width;
                    child_input.known_dimensions.width = Some(child_metrics.clamp_width(width));
                }
            }
            let output = tree.compute_child_layout_with_block_ctx(child, child_input, None);
            if tree
                .forced_break_clearance
                .binary_search(&usize::from(child))
                .is_ok()
            {
                ctx.record_page_start_float_floor(style.clear);
            }
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
            if inputs.run_mode == RunMode::PerformLayout && !output.oof_candidates.is_empty() {
                let mut surfaced = output.oof_candidates.clone();
                surfaced.translate(location);
                flow.oof_candidates.append(&mut surfaced);
            }
            if inputs.run_mode == RunMode::PerformLayout {
                commit(tree, child, order, child_metrics, &output, location, margin);
                flow.placed.push(child);
            }
            continue;
        }
        let same = shares_parent_context(tree, node, child, &style);
        // A `stretch` height limit of the child leaves out the margins this
        // box holds in.
        let held_margins = content_limits::has_stretch_height_limit(tree, usize::from(child));
        if held_margins {
            tree.context
                .held_margins
                .push(content_limits::HeldMargins::new(
                    usize::from(child),
                    holds_margin,
                ));
        }
        // The box after the float that starts a page lines up beside that
        // float, but below the floats of the pages before it.
        let floor = tree
            .forced_break_float_floor
            .binary_search(&usize::from(child))
            .is_ok()
            .then(|| ctx.page_start_float_floor())
            .flatten();
        // An `auto` width fills the room, but a table, a compressible
        // replaced box and a button take their own size then; `stretch` fills
        // it for them too (CSS Sizing 4 §5.1).
        let stretches = match style.size.width.expand() {
            ExpandedDimension::Stretch => true,
            ExpandedDimension::Auto => {
                !style.item_is_table
                    && !style.is_compressible_replaced()
                    && !tree.nodes[usize::from(child)].is_html_button()
            }
            _ => false,
        };
        let natural_stretch = content_width - margin.horizontal_axis_sum();
        // A `min-width` or `max-width` of `stretch` fills the room the child
        // finds, which beside a float is the room the float leaves.
        let stretch_widths = content_limits::stretch_widths(tree, usize::from(child));
        // `Width`: the border-box width the child gets where an `auto` width
        // resolves to `stretch`. `Height`: its border-box height at `width`.
        let mut measure = |probe: BoxProbe| match probe {
            BoxProbe::Width { stretch } => {
                let width = 'width: {
                    if let Some(preferred) = child_metrics.preferred.width
                        && !child_metrics.ratio_width
                    {
                        break 'width preferred;
                    }
                    if stretches && !child_metrics.ratio_width {
                        break 'width child_metrics.clamp_width(stretch.max(0.0));
                    }
                    let mut measure = child_input;
                    measure.run_mode = RunMode::ComputeSize;
                    measure.available_space.width = match style.size.width.expand() {
                        ExpandedDimension::MinContent => AvailableSpace::MinContent,
                        ExpandedDimension::MaxContent => AvailableSpace::MaxContent,
                        _ => AvailableSpace::Definite(stretch.max(0.0)),
                    };
                    tree.compute_child_layout_with_block_ctx(child, measure, None)
                        .size
                        .width
                };
                if stretch_widths.any() {
                    stretch_widths.clamp(
                        width,
                        (child_metrics.min.width, child_metrics.max.width),
                        stretch.max(0.0),
                    )
                } else {
                    width
                }
            }
            BoxProbe::Height { width } => {
                let mut measure = child_input;
                measure.run_mode = RunMode::ComputeSize;
                measure.known_dimensions.width = Some(width);
                measure.available_space.width = AvailableSpace::Definite(width);
                measure.vertical_margins_are_collapsible = Line::FALSE;
                tree.compute_child_layout_with_block_ctx(child, measure, None)
                    .size
                    .height
            }
        };
        let mut child_width = measure(BoxProbe::Width {
            stretch: natural_stretch,
        });
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
            let forced = joins
                && (adjoining_clearance || floor.is_some_and(|floor| floor > hypothetical + 0.001));
            // Moving below this point separates the box from the floats.
            let threshold = if adjoining { flow.end } else { hypothetical };
            let search = if forced {
                ctx.cleared_threshold(style.clear)
                    .into_iter()
                    .chain(floor)
                    .reduce(f32::max)
                    .unwrap_or(hypothetical)
            } else if adjoining {
                flow.end
            } else {
                hypothetical
            };
            let search = floor.map_or(search, |floor| search.max(floor));
            if forced || ctx.has_active_floats(search) {
                let placed = ctx.find_box_slot(
                    search,
                    [margin.left, margin.right],
                    container.direction,
                    style.clear,
                    &mut measure,
                );
                let slot = placed.slot;
                child_width = placed.width;
                area_start = slot.x;
                area_width = slot.stretch_width;
                separated = forced || slot.y > threshold + 0.001;
                slot_y = Some(if joins && !separated {
                    flow.end
                } else {
                    slot.y
                });
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
            .into_iter()
            .chain(floor)
            .fold(f32::NEG_INFINITY, f32::max);
        let saved = ctx.snapshot();
        let mut final_pass = resolved && joins && style.clear == Clear::None && floor.is_none();
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
            let clearance = (style.clear != Clear::None || floor.is_some())
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
        if held_margins {
            tree.context.held_margins.pop();
        }
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
        if inputs.run_mode == RunMode::PerformLayout && !output.oof_candidates.is_empty() {
            let mut surfaced = output.oof_candidates.clone();
            surfaced.translate(location);
            flow.oof_candidates.append(&mut surfaced);
        }
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
    // Margins that collapsed through every child have joined the top margin of
    // this box, so they lie outside of it and leave no room below the content.
    let collapsed_into_start = leading && collapse_start;
    if !(collapse_end || collapsed_into_start) {
        flow.end += pending.resolve();
    }
    flow
}

pub(super) fn align(tree: &mut LayoutSession<'_>, style: &Style, flow: &mut Flow, free: f32) {
    if style.align_content.keyword == AlignContentKeyword::Normal {
        return;
    }
    // Without an overflow-position keyword, a block container keeps overflowing
    // content at its start edge, as `safe` does: content pushed past the start
    // edge could not be scrolled to. Only `unsafe` aligns it as written.
    let align_content = AlignContent {
        safety: match style.align_content.safety {
            AlignmentSafety::Default => AlignmentSafety::Safe,
            safety => safety,
        },
        ..style.align_content
    };
    let offset = compute_block_align_content_offset(align_content, free);
    if offset == 0.0 {
        return;
    }
    flow.baselines.first = flow.baselines.first.map(|baseline| baseline + offset);
    flow.baselines.last = flow.baselines.last.map(|baseline| baseline + offset);
    for overflow in [&mut flow.overflow, &mut flow.float_overflow] {
        if *overflow != Edges::ZERO {
            overflow.top += offset;
            overflow.bottom += offset;
        }
    }
    flow.oof_candidates.translate(Point { x: 0.0, y: offset });
    for &node in &flow.placed {
        let mut layout = tree.nodes[usize::from(node)].layout;
        layout.location.y += offset;
        tree.set_unrounded_layout(node, &layout);
    }
}
