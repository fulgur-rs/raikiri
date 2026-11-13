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
    AlignContent, AlignContentKeyword, AlignItemsKeyword, AlignmentSafety, BoxGenerationMode,
    Clear, Direction, ExpandedDimension, Float, MaybeResolve, Overflow, Position,
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
    /// The rendered legend of a fieldset, which stays over the border when
    /// `align-content` moves the other children; [`Flow::add_legend`] adds
    /// its overflow and candidates after that.
    pub legend: Option<super::legend::PlacedLegend>,
}

impl Flow {
    /// Add the overflow and the absolutely positioned descendants of the
    /// rendered legend, which come first in document order.
    pub(super) fn add_legend(&mut self) {
        if let Some(mut legend) = self.legend.take() {
            self.overflow = self.overflow.union(legend.overflow);
            let mut rest = self.oof_candidates.take();
            self.oof_candidates = legend.oof_candidates.take();
            self.oof_candidates.append(&mut rest);
        }
    }
}

fn measure_in_flow_child(
    tree: &mut LayoutSession<'_>,
    parent: NodeId,
    child: NodeId,
    inputs: LayoutInput,
    context: Option<&mut BlockContext>,
) -> LayoutOutput {
    let measure = |tree: &mut LayoutSession<'_>| {
        tree.compute_child_layout_with_block_ctx(child, inputs, context)
    };
    if inputs.run_mode == RunMode::ComputeSize
        && (tree.context.baseline_measure_owners.last() == Some(&usize::from(parent))
            || crate::layout::ifc::outside_marker::height(
                tree,
                tree.style_views.cascade(),
                usize::from(parent),
            )
            .is_some())
    {
        // Block flow collects its baseline from an in-flow child's output,
        // just as native flex/grid do through their placement callbacks.
        tree.with_baseline_measure(usize::from(child), measure)
    } else {
        measure(tree)
    }
}

/// Where a block-level box is placed along the inline axis of its block
/// container by `justify-self` (CSS Box Alignment 3 §6.1): `None` for
/// `normal` and `stretch`, which leave it to its margins, else the start, the
/// center or the end of the container, and whether an overflowing box falls
/// back to the start (`safe`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum InlinePosition {
    Start,
    Center,
    End,
}

/// The position `justify-self` gives the in-flow child `style` in the block
/// container `container`, with `auto` taking the container's
/// `justify-items`. Baseline alignment has no shared baseline in a block
/// container, so it falls back to the start, as `self-start` and `self-end`
/// do to the start or the end of the child's own direction.
// spec: https://www.w3.org/TR/css-align-3/#justify-block
pub(super) fn justify_position(style: &Style, container: &Style) -> Option<(InlinePosition, bool)> {
    position_of(
        style.justify_self.unwrap_or(container.justify_items),
        style.direction,
        container.direction,
    )
}

/// The position `alignment` gives a box of `direction` in a block container
/// of `container_direction` (see [`justify_position`]).
fn position_of(
    alignment: raikiri_layout_style::values::AlignItems,
    direction: Direction,
    container_direction: Direction,
) -> Option<(InlinePosition, bool)> {
    let alignment = alignment.resolve_self_relative(direction, container_direction, true);
    let position = match alignment.keyword {
        AlignItemsKeyword::Normal | AlignItemsKeyword::Stretch => return None,
        AlignItemsKeyword::Center => InlinePosition::Center,
        AlignItemsKeyword::End | AlignItemsKeyword::FlexEnd => InlinePosition::End,
        _ => InlinePosition::Start,
    };
    Some((position, alignment.safety == AlignmentSafety::Safe))
}

impl InlinePosition {
    /// The offset from the left of a room `free` wider than the box that
    /// puts the box at this position, where a `safe` alignment that
    /// overflows falls back to the start.
    fn offset(self, safe: bool, free: f32, rtl: bool) -> f32 {
        let position = if safe && free < 0.0 {
            Self::Start
        } else {
            self
        };
        match position {
            Self::Center => free / 2.0,
            Self::Start if rtl => free,
            Self::End if !rtl => free,
            _ => 0.0,
        }
    }
}

/// The position `justify-self` gives the in-flow block `child` in the block
/// container that lays it out, read from the style views, and whether its
/// margins are auto (see [`justify_position`]).
fn justify_position_of(
    tree: &LayoutSession<'_>,
    child: usize,
) -> Option<((InlinePosition, bool), bool)> {
    use crate::layout::style_views::ReadRole;
    let parent = tree.taffy_parent_of(child)?;
    let style = tree.query_style(child, ReadRole::BlockItem);
    let container = tree.query_style(parent, ReadRole::BlockContainer);
    // `justify-items` does not apply to an inline box, so `auto` is `normal`
    // for a block inside one, whatever its containing block says.
    let alignment = match style.bridge_justify_self() {
        Some(alignment) => alignment,
        None if inline_parent(tree, child) => return None,
        None => container.justify_items(),
    };
    let position = position_of(alignment, style.direction(), container.direction())?;
    let margin = style.margin();
    Some((position, margin.left.is_auto() || margin.right.is_auto()))
}

/// Whether the parent box of `child`, past `display: contents`, is an inline
/// box.
fn inline_parent(tree: &LayoutSession<'_>, child: usize) -> bool {
    std::iter::successors(tree.parent_of(child), |&parent| tree.parent_of(parent))
        .find(|&parent| tree.nodes[parent].display() != DisplayValue::Contents)
        .is_some_and(|parent| tree.nodes[parent].display() == DisplayValue::Inline)
}

/// Whether `justify-self` places the in-flow block `child` at a position,
/// which gives an `auto` width the fit-content width, even where an auto
/// margin then places the box.
pub(crate) fn justify_fits_content(tree: &LayoutSession<'_>, child: usize) -> bool {
    justify_position_of(tree, child).is_some()
}

/// Where `justify-self` puts the in-flow block `child` in a room `free` wider
/// than its margin box, as an offset from the left of that room; `None` when
/// its alignment leaves it to its margins, as `normal`, `stretch` and an
/// auto margin do.
pub(crate) fn justified_offset(
    tree: &LayoutSession<'_>,
    child: usize,
    rtl: bool,
    free: f32,
) -> Option<f32> {
    match justify_position_of(tree, child)? {
        (_, true) => None,
        ((position, safe), false) => Some(position.offset(safe, free, rtl)),
    }
}

/// Whether `justify-self` (or the container's `justify-items` for `auto`)
/// asks the in-flow child `style` to stretch explicitly, which fills the room
/// for a replaced box that `normal` leaves at its natural width.
fn justify_stretches(style: &Style, container: &Style) -> bool {
    style
        .justify_self
        .unwrap_or(container.justify_items)
        .keyword
        == AlignItemsKeyword::Stretch
}

#[allow(clippy::too_many_arguments)]
pub(super) fn place_inline_axis(
    style: &Style,
    direction: Direction,
    container_width: f32,
    insets: [f32; 2],
    (area_start, area_width): (f32, f32),
    used_width: f32,
    justify: Option<(InlinePosition, bool)>,
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
    let rtl = direction == Direction::Rtl;
    let offset = match justify {
        // Auto margins take the free space before alignment does.
        Some((position, safe)) if !left_auto && !right_auto => position.offset(safe, free, rtl),
        _ if rtl && (free < 0.0 || !left_auto && !right_auto) => free,
        _ => auto_left,
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

pub(super) fn margins(style: &Style, basis: f32) -> Edges<f32> {
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

pub(super) fn overflow(
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
        let (left, right) = if style.direction == Direction::Rtl
            && output.overflow_origin == crate::layout::protocol::OverflowOrigin::ScrollOrigin
        {
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
    if inputs.run_mode == RunMode::PerformLayout {
        tree.outside_marker_leading.clear(usize::from(node));
    }
    let marker_height = crate::layout::ifc::outside_marker::height(
        tree,
        tree.style_views.cascade(),
        usize::from(node),
    );
    let content_width = (width - metrics.inset.horizontal_axis_sum()).max(0.0);
    let mut parent = Size {
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
        legend: None,
    };
    let children: Vec<_> = tree.child_ids(node).collect();
    // The rendered legend of a fieldset sits over its block-start border, out
    // of the flow of the other children, which start below it.
    let legend = super::legend::rendered_legend(tree, node, container);
    if let Some(legend) = legend {
        let order = children
            .iter()
            .position(|&child| child == legend)
            .unwrap_or(0);
        let placed = super::legend::place(
            tree, node, legend, order, inputs, container, metrics, width, parent,
        );
        flow.end += placed.content_offset;
        // The part of the legend below the border takes room off the height
        // the percentages of the other children resolve against.
        parent.height = parent
            .height
            .map(|height| (height - placed.content_offset).max(0.0));
        flow.legend = Some(placed);
    }
    let mut pending = CollapsibleMarginSet::ZERO;
    let mut pending_has_clearance = false;
    let mut completed_boundary = None;
    let mut leading = true;
    let track_margin_floats = tree.context.float_resumptions.has_margin_flows();
    let mut margin_float_tops = Vec::new();
    if !collapse_start {
        ctx.commit_strut();
    }
    for (order, child) in children.into_iter().enumerate() {
        if Some(child) == legend {
            continue;
        }
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
        // The `100%` width of an `<svg>` that fills the room its parent offers
        // stands in for an `auto` one.
        let width_stands_in = tree.nodes[usize::from(child)].svg_width_follows_available;
        let child_metrics = Metrics::resolve(
            &style,
            width_stands_in,
            tree.nodes[usize::from(child)].ratio_in_content_box,
            metrics_input,
        );
        // A size that only follows from the aspect ratio is left for the child
        // to resolve, since its content may raise it.
        child_input.known_dimensions.height = match stretched_height {
            Some(height) => Some(child_metrics.clamp_height(height)),
            // The height an `<svg>` gets from the width that stands in for its
            // `auto` one is not its own: the room it ends up in decides the
            // width, and the ratio gives the height from that.
            None => child_metrics
                .preferred
                .height
                .filter(|_| !child_metrics.ratio_height && !width_stands_in),
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
                if let Some(top) = tree.context.float_resumptions.float_top(usize::from(child)) {
                    top
                } else if leading && collapse_start {
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
            if track_margin_floats {
                margin_float_tops.push((usize::from(child), position.y));
            }
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
        // Stretch-dependent sizing leaves out the margins this box holds in.
        let held_margins = content_limits::needs_stretch_height_room(tree, usize::from(child));
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
        let margin_floor = tree
            .context
            .float_resumptions
            .margin_float(usize::from(child))
            .and_then(|source| margin_float_tops.iter().rev().find(|(id, _)| *id == source))
            .map(|(_, top)| top + margin.top);
        let floor = tree
            .forced_break_float_floor
            .binary_search(&usize::from(child))
            .is_ok()
            .then(|| ctx.page_start_float_floor())
            .flatten()
            .into_iter()
            .chain(tree.context.float_resumptions.flow_top(usize::from(child)))
            .chain(margin_floor)
            .reduce(f32::max);
        // An `auto` width fills the room, but a table, a compressible
        // replaced box and a button take their own size then; `stretch` fills
        // it for them too (CSS Sizing 4 §5.1).
        // A box that `justify-self` places at a position takes its
        // fit-content width for `auto` instead (CSS Box Alignment 3 §6.1).
        // An explicit `stretch` fills the room for a replaced box too.
        let justify = justify_position(&style, container);
        let auto_stretches = match style.size.width.expand() {
            ExpandedDimension::Stretch => true,
            ExpandedDimension::Auto => {
                !style.item_is_table
                    && !style.is_compressible_replaced()
                    && !tree.nodes[usize::from(child)].is_html_button()
            }
            _ => false,
        };
        let fit_content = auto_stretches
            && justify.is_some()
            && style.size.width.expand() == ExpandedDimension::Auto;
        let stretches = (auto_stretches && !fit_content)
            || (style.item_is_replaced
                && !style.item_is_table
                && style.size.width.expand() == ExpandedDimension::Auto
                && justify_stretches(&style, container));
        let natural_stretch = content_width - margin.horizontal_axis_sum();
        // A `min-width` or `max-width` of `stretch` fills the room the child
        // finds, which beside a float is the room the float leaves.
        let stretch_widths = content_limits::stretch_widths(tree, usize::from(child));
        // `Width`: the border-box width the child gets where an `auto` width
        // resolves to `stretch`. `Height`: its border-box height at `width`.
        // A table of `auto` or `stretch` width is handed its measured room and
        // sizes itself from its columns there. Its columns are set in that
        // room even where one of them is then removed by
        // `visibility: collapse`, so the width left after the removal must not
        // stand in for it, whether the table is measured or laid out.
        let table_uses_slot_room = style.item_is_table
            && matches!(
                style.size.width.expand(),
                ExpandedDimension::Auto | ExpandedDimension::Stretch
            );
        let laid_out_width = |width: f32, room: f32| {
            if table_uses_slot_room {
                child_metrics.clamp_width(room.max(0.0))
            } else if style.item_is_table {
                child_metrics.preferred.width.unwrap_or(width)
            } else {
                width
            }
        };
        let mut content_widths = None;
        let mut measure = |probe: BoxProbe| match probe {
            BoxProbe::Width { stretch } => {
                // The table resolves its own width limits against its grid
                // and caption minimum. Probe that used width before deciding
                // whether its border box fits beside a float.
                let width = 'width: {
                    // A width standing in for `auto` fills the room a float
                    // leaves.
                    if width_stands_in {
                        break 'width child_metrics.clamp_width(stretch.max(0.0));
                    }
                    if !style.item_is_table
                        && let Some(preferred) = child_metrics.preferred.width
                        && !child_metrics.ratio_width
                    {
                        break 'width preferred;
                    }
                    if !style.item_is_table && stretches && !child_metrics.ratio_width {
                        break 'width child_metrics.clamp_width(stretch.max(0.0));
                    }
                    if fit_content {
                        // The fit-content width: the max-content width, but
                        // no wider than the room and no narrower than the
                        // min-content width (CSS Sizing 3 §5.2). Neither
                        // depends on the room, so each slot beside floats
                        // reuses them.
                        let (max_content, min_content) = *content_widths.get_or_insert_with(|| {
                            let mut probe = |space| {
                                let mut measure = child_input;
                                measure.run_mode = RunMode::ComputeSize;
                                measure.available_space.width = space;
                                tree.compute_child_layout_with_block_ctx(child, measure, None)
                                    .size
                                    .width
                            };
                            (
                                probe(AvailableSpace::MaxContent),
                                probe(AvailableSpace::MinContent),
                            )
                        });
                        break 'width child_metrics
                            .clamp_width(max_content.min(stretch.max(0.0)).max(min_content));
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
                if !style.item_is_table && stretch_widths.any() {
                    stretch_widths.clamp(
                        width,
                        (child_metrics.min.width, child_metrics.max.width),
                        stretch.max(0.0),
                    )
                } else {
                    width
                }
            }
            BoxProbe::Height { width, stretch } => {
                let width = laid_out_width(width, stretch);
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
            let hypothetical = if let Some(boundary) = completed_boundary {
                boundary
            } else if joins {
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
                (area_start, area_width) = placed.stretch_room(container.direction);
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
            justify,
            &mut margin,
        );
        let assigned_width = laid_out_width(child_width, area_width);
        child_input.known_dimensions.width = Some(assigned_width);
        child_input.available_space.width = AvailableSpace::Definite(assigned_width);
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
            .unwrap_or(if let Some(boundary) = completed_boundary {
                boundary
            } else if joins {
                flow.end
            } else {
                flow.end + pending.collapse_with_margin(margin.top).resolve()
            })
            .max(clear_y);
        let (output, top, top_set, bottom_set, clearance, used_clearance, floated_end, adjoining) = loop {
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
                let output = measure_in_flow_child(tree, node, child, child_input, Some(&mut sub));
                (
                    output,
                    sub.floated_content_height_contribution(),
                    sub.top_adjoining_floats(),
                )
            } else {
                (
                    measure_in_flow_child(tree, node, child, child_input, None),
                    f32::NEG_INFINITY,
                    [false; 2],
                )
            };
            let top_set = output.top_margin.collapse_with_margin(margin.top);
            let bottom_set = output.bottom_margin.collapse_with_margin(margin.bottom);
            let hypothetical = if let Some(boundary) = completed_boundary {
                boundary
            } else if joins && resolved {
                ctx.hypothetical_top_position(pending.collapse_with_set(top_set))
                    .unwrap_or(flow.end)
            } else {
                flow.end + pending.collapse_with_set(top_set).resolve()
            };
            let completed = tree
                .context
                .float_resumptions
                .completed_at(usize::from(child), output.size.height);
            let used_clearance = (style.clear != Clear::None || floor.is_some())
                && ((joins && adjoining_clearance) || hypothetical < clear_y);
            let clearance = completed.is_some() || used_clearance;
            let top = if let Some(completed) = completed {
                completed
            } else if let Some(slot_y) = slot_y {
                slot_y
            } else if let (Some(boundary), false) = (completed_boundary, clearance) {
                boundary
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
                used_clearance,
                floated_end,
                adjoining,
            );
        };
        // Chromium translates the accepted child after ordinary float and
        // margin placement; feeding marker leading back into it can cycle.
        let marker_gap = if flow.baselines.first.is_none() {
            output.baselines.first.map_or(0.0, |baseline| {
                crate::layout::ifc::outside_marker::leading(marker_height, baseline)
            })
        } else {
            0.0
        };
        let top = top + marker_gap;
        if held_margins {
            tree.context.held_margins.pop();
        }
        if inputs.run_mode == RunMode::PerformLayout
            && style.clear != Clear::None
            && !tree.context.float_resumption_candidates.is_empty()
        {
            tree.context
                .block_clearance
                .insert(usize::from(child), used_clearance);
            let before = ctx.leading_margin_set(pending);
            if used_clearance
                && joins
                && output.margins_can_collapse_through
                && output.size.height == 0.0
                && before.collapse_with_set(top_set) == top_set
            {
                tree.context
                    .clearance_resumption_offsets
                    .insert(usize::from(child), before.resolve());
            } else {
                tree.context
                    .clearance_resumption_offsets
                    .remove(&usize::from(child));
            }
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
            justify,
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
        if inputs.run_mode == RunMode::PerformLayout && marker_gap > 0.0 {
            let source = crate::page_projection::text_runs::first_line(
                tree,
                tree.style_views.cascade(),
                usize::from(child),
            )
            .and_then(|line| line.source);
            crate::layout::ifc::outside_marker::record_child(
                tree,
                usize::from(node),
                usize::from(child),
                marker_gap,
                source,
            );
        }
        if let Some(baseline) = output.baselines.first {
            flow.baselines.first.get_or_insert(top + baseline);
        }
        // A child without a last baseline passes none up, where its first
        // baseline stands in for a missing last one otherwise.
        flow.baselines.last = output
            .baselines
            .last_or_first()
            .map(|baseline| top + baseline)
            .or(flow.baselines.last);
        flow.baselines.no_last = flow.baselines.first.is_some() && flow.baselines.last.is_none();
        if inputs.run_mode == RunMode::PerformLayout {
            commit(tree, child, order, child_metrics, &output, location, margin);
            flow.placed.push(child);
        }
        if let Some(boundary) = tree
            .context
            .float_resumptions
            .completed_at(usize::from(child), output.size.height)
        {
            // This box completed at the preceding fragment's end. Margins
            // adjoining it are consumed before the next content resumes.
            completed_boundary = Some(boundary);
        } else if clearance || !output.margins_can_collapse_through {
            completed_boundary = None;
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
            if clearance && output.margins_can_collapse_through {
                // Clearance separates earlier margins, but this empty box's
                // own margins still collapse with each other and following
                // siblings. Its border stays at the cleared position.
                flow.end -= top_set.resolve();
                pending = top_set.collapse_with_set(bottom_set);
                pending_has_clearance = true;
            } else {
                pending = bottom_set;
                pending_has_clearance = false;
            }
            leading = false;
            flow.through = false;
            ctx.commit_strut();
        }
    }
    // A margin adjoining an empty box with clearance cannot escape through
    // the parent's bottom edge (CSS 2.1 8.3.1).
    flow.trailing = if pending_has_clearance {
        CollapsibleMarginSet::ZERO
    } else {
        pending
    };
    // Margins that collapsed through every child have joined the top margin of
    // this box, so they lie outside of it and leave no room below the content.
    let collapsed_into_start = leading && collapse_start;
    if let Some(boundary) = completed_boundary {
        flow.end = boundary;
    } else if pending_has_clearance || !(collapse_end || collapsed_into_start) {
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
