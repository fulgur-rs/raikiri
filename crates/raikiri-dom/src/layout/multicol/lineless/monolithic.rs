//! Monolithic boxes inside a flex container that the column stream slices
//! whole (see [`super::sliceable_flex_container`]).
//!
//! A box with size containment in the block axis is monolithic content (CSS
//! Fragmentation 3, 4.4): a column end never cuts it. One that starts below
//! the top of a column and does not fit in what is left of it moves to the
//! top of the next column, and the space it leaves stays with the boxes
//! around it: the boxes that follow it in its flow move down with it, and
//! the boxes that hold it grow by as much (CSS Flexbox 1, 12). A break
//! before the first box of a block or of a flex line is one before the
//! block or the line, so a box that starts its parent moves the parent, or
//! the whole line, instead. The values follow Chromium.
//!
//! Boxes that avoid breaks move the same way (CSS Fragmentation 3, 4.4): a
//! box with `break-inside: avoid` moves whole like a monolithic box where a
//! column holds it, and a break that `break-before` or `break-after` avoids
//! moves to the break before the box above it, when that box does not start
//! its column. A break between two boxes of a flow truncates the margins
//! and the gap between them, so the box after it starts at the top of the
//! next column (CSS Fragmentation 3, 5.2; CSS Box Alignment 3, 8.1).

use super::*;
use raikiri_layout_style::values::{AlignItemsKeyword, BoxSizing};

/// Whether `value` avoids a break in columns before or after a box.
pub(super) fn avoids_break(value: BreakBetween) -> bool {
    matches!(value, BreakBetween::Avoid | BreakBetween::AvoidColumn)
}

/// Whether `value` avoids a break in columns inside a box.
pub(super) fn avoids_break_inside(value: BreakInside) -> bool {
    matches!(value, BreakInside::Avoid | BreakInside::AvoidColumn)
}

/// Whether a break after `id` is avoided: by its own `break-after`, or by
/// that of the last box in flow inside it, which propagates to it (CSS
/// Fragmentation 3, 3.1). The boxes are ones the stream slices (see
/// [`passes_breaks_on`]).
fn avoids_break_after(tree: &LayoutSession<'_>, id: usize) -> bool {
    avoids_break(layout_node(tree, id).break_after)
        || passes_breaks_on(tree, id)
            && in_flow_elements(tree, id)
                .last()
                .is_some_and(|&last| avoids_break_after(tree, last))
}

/// Whether a break before `id` is avoided, by its own `break-before` or by
/// that of the first box in flow inside it.
fn avoids_break_before(tree: &LayoutSession<'_>, id: usize) -> bool {
    avoids_break(layout_node(tree, id).break_before)
        || passes_breaks_on(tree, id)
            && in_flow_elements(tree, id)
                .first()
                .is_some_and(|&first| avoids_break_before(tree, first))
}

/// Whether `id` is a box that a column end should not cut: a monolithic box
/// or one that avoids breaks inside it.
fn is_kept_whole(tree: &LayoutSession<'_>, id: usize) -> bool {
    is_monolithic_leaf(tree, id) || avoids_break_inside(layout_node(tree, id).break_inside)
}

/// Whether a box in flow below `id` is one that a column end should not cut,
/// or avoids a break before or after it, which only moves that keep it
/// whole honour.
pub(super) fn holds_kept_boxes(tree: &LayoutSession<'_>, id: usize) -> bool {
    in_flow_elements(tree, id).into_iter().any(|child| {
        let node = layout_node(tree, child);
        is_kept_whole(tree, child)
            || avoids_break(node.break_before)
            || avoids_break(node.break_after)
            || holds_kept_boxes(tree, child)
    })
}

/// Whether `id` is a single-line column flex container that stacks its
/// items in the order of the tree with space between some of them that a
/// break truncates: the row gap, or the block-end margin of an item.
fn has_truncated_space(tree: &LayoutSession<'_>, id: usize) -> bool {
    let style = style_of(tree, id);
    let items = in_flow_elements(tree, id);
    layout_node(tree, id).display() == DisplayValue::Flex
        && !style.flex_direction_is_row()
        && !style.flex_wraps()
        && !style.flex_direction_is_reversed()
        && items.len() > 1
        && (!is_zero(style.gap().height)
            || items.iter().any(|&item| {
                block_margins(&style_of(tree, item)).is_none_or(|(_, bottom)| bottom != 0.0)
            }))
}

fn is_length(gap: LengthPercentage) -> bool {
    matches!(gap.expand(), ExpandedLengthPercentage::Length(_))
}

fn is_zero(gap: LengthPercentage) -> bool {
    matches!(gap.expand(), ExpandedLengthPercentage::Length(px) if px == 0.0)
}

/// Whether the layout of `id` changes at a break in a way [`Shape`]
/// models: it holds boxes kept whole (see [`holds_kept_boxes`]), or space
/// that a break truncates (see [`has_truncated_space`]).
fn holds_moves(tree: &LayoutSession<'_>, id: usize) -> bool {
    has_truncated_space(tree, id)
        || in_flow_elements(tree, id).into_iter().any(|child| {
            let node = layout_node(tree, child);
            is_kept_whole(tree, child)
                || avoids_break(node.break_before)
                || avoids_break(node.break_after)
                || holds_moves(tree, child)
        })
}

/// Whether `id`, a box inside a flex container the stream slices, is a
/// monolithic box that the stream moves whole: a block with size containment
/// in the block axis and nothing rendered inside it, in flow, with block
/// margins of a length, no forced break of its own, and nothing painted past
/// its box or repeated at a break, which the pieces of the container would
/// cut (see [`is_sliceable_flex_box`]).
pub(super) fn is_monolithic_leaf(tree: &LayoutSession<'_>, id: usize) -> bool {
    let node = layout_node(tree, id);
    let style = style_of(tree, id);
    node.size_containment.height
        && matches!(node.display(), DisplayValue::Block | DisplayValue::FlowRoot)
        && is_horizontal(node)
        && node.layout_position() == Position::Static
        && !style.float().is_floated()
        && block_margins(&style).is_some()
        && (!sets_break_in_columns(node.break_before) || avoids_break(node.break_before))
        && (!sets_break_in_columns(node.break_after) || avoids_break(node.break_after))
        && !node.has_before_or_after_content
        && !node.paints_whole_box_effects
        && !(node.clones_box_decorations && has_block_edge_decorations(&style))
        && !node.tag_name().is_some_and(|tag| {
            ATOMIC_TAGS.contains(&tag)
                || REPLACED_BOX_TAGS.contains(&tag)
                || matches!(tag, "fieldset" | "legend" | "details" | "summary")
        })
        && !has_rendered_content(tree, id, false)
}

/// Whether the boxes inside `container`, a flex container the stream slices,
/// that move at breaks sit in boxes whose moves [`Shape`] models (see
/// [`Shape::build`]), and no column is shorter than a monolithic one. A box taller
/// than its column overflows it, and the fragment of the container around
/// it grows to hold it, which the pieces of the stream do not model: that
/// needs a content height of the column container at least as tall as the
/// tallest box, or, with no height of its own, columns that balance with
/// nothing to limit them, which grow to the tallest box.
pub(super) fn supports(tree: &LayoutSession<'_>, container: usize) -> bool {
    let shape = match Shape::build(tree, container) {
        Ok(Some(shape)) => shape,
        Ok(None) => return true,
        Err(()) => return false,
    };
    if !shape
        .nodes
        .iter()
        .any(|node| matches!(node.kind, Kind::Monolithic))
    {
        return true;
    }
    let room = column_owner(tree, container).and_then(|owner| {
        let style = style_of(tree, owner);
        let Some(height) = style_dimension_length(style.size().height) else {
            return (style.size().height.is_auto() && balances_without_limit(tree, owner))
                .then_some(f32::INFINITY);
        };
        let edges = block_edges(tree, &style);
        Some(if style.box_sizing() == BoxSizing::BorderBox {
            height - edges
        } else {
            height
        })
    });
    let Some(room) = room else {
        return false;
    };
    shape.nodes.iter().all(|node| {
        !matches!(node.kind, Kind::Monolithic)
            || shape
                .monolithic_height(tree, node)
                .is_some_and(|height| height <= room + EPSILON)
    })
}

/// The sum of the block-axis padding and border of a box, with percentages
/// counting as zero, which leaves more room.
fn block_edges(tree: &LayoutSession<'_>, style: &LayoutStyleView<'_>) -> f32 {
    let inset = |value| super::super::multicol_resolve_inset(tree, value, 0.0);
    let (padding, border) = (style.padding(), style.border());
    inset(padding.top) + inset(padding.bottom) + inset(border.top) + inset(border.bottom)
}

/// How a box of the shape lays out the boxes it holds.
#[derive(Debug)]
enum Kind {
    /// A monolithic box (see [`is_monolithic_leaf`]).
    Monolithic,
    /// A box whose layout no break changes, which moves as a whole.
    Leaf,
    /// A block, or a single-line column flex container in the order of the
    /// tree, which stacks its boxes in order along the block axis.
    Flow,
    /// A row flex container (see [`ShapeNode::lines`]).
    Row,
}

#[derive(Debug)]
struct ShapeNode {
    id: usize,
    parent: Option<usize>,
    kind: Kind,
    /// Whether it avoids breaks inside it: it moves whole like a monolithic
    /// box, and where no column holds it, it is cut there as the boxes
    /// inside it allow.
    avoids: bool,
    /// Whether its block size follows its content. A box with a block size
    /// of its own keeps it: what moves inside it overflows it, and the boxes
    /// after it stay where they are.
    grows: bool,
    /// The boxes in flow inside it, in order.
    children: Vec<usize>,
    /// The flex lines of a row flex container, in order, each holding the
    /// indices of its items and whether each one stretches to the line.
    lines: Vec<Vec<(usize, bool)>>,
}

/// The boxes of a sliced flex container on the way to the monolithic boxes
/// it holds.
#[derive(Debug)]
pub(super) struct Shape {
    nodes: Vec<ShapeNode>,
}

/// The changes the breaks inside a container make to its layout: how far
/// each box moves and how much it grows, and how much the container itself
/// grows.
#[derive(Debug, Default)]
pub(super) struct Pushes {
    pub(super) changes: Vec<(usize, f32, f32)>,
    pub(super) growth: f32,
    /// The least amount by which taller columns would keep a box from
    /// moving down, if one moves.
    pub(super) shortage: Option<f32>,
}

/// What a break before a monolithic box moves.
#[derive(Clone, Copy, Debug)]
enum Target {
    /// A box in a block flow, with the boxes after it.
    Flow(usize),
    /// A flex line, with the lines after it, from one of its items: a break
    /// before an item at the start of its line is one between lines.
    Line(usize),
    /// A flex item below the start of its line, alone: the items beside it
    /// stay where they are, and the line grows to hold it.
    Item(usize),
    /// The flex container itself.
    Container,
}

/// What the monolithic boxes of a container do where it starts.
#[derive(Debug)]
pub(super) enum Outcome {
    /// They move as [`Pushes`] says.
    Moves(Pushes),
    /// The first of them does not fit and starts the container, so the
    /// container itself moves to the next column.
    MovesContainer,
    /// They do what is not modelled here: one overflows the container. The
    /// container is sliced as if they were plain boxes.
    Unmodelled,
}

/// Whether a break before or after the first or last box of `container`
/// is avoided, which is a break between the container and the boxes
/// around it that the stream does not model.
fn avoids_breaks_at_ends(tree: &LayoutSession<'_>, container: usize) -> bool {
    let items = in_flow_elements(tree, container);
    passes_breaks_on(tree, container)
        && (items
            .first()
            .is_some_and(|&first| avoids_break_before(tree, first))
            || items
                .last()
                .is_some_and(|&last| avoids_break_after(tree, last)))
}

/// Whether the block size of a box follows its content alone, with no
/// minimum or maximum, `Some(false)` when it has a length of its own and
/// nothing else, and `None` otherwise.
fn grows_with_content(style: &LayoutStyleView<'_>) -> Option<bool> {
    let free = style_dimension_length(style.min_size().height.into()).is_none_or(|px| px <= 0.0)
        && style_dimension_length(style.max_size().height.into()).is_none()
        && style.aspect_ratio().is_none();
    if !free {
        None
    } else if style.size().height.is_auto() {
        Some(true)
    } else {
        style_dimension_length(style.size().height).map(|_| false)
    }
}

impl Shape {
    /// The border-box height of `node`, a monolithic box, when its style
    /// fixes it: a length with no minimum, maximum or aspect ratio, and,
    /// in a column flex container, no flex basis or growth that would change
    /// it. Shrinking only makes it shorter.
    fn monolithic_height(&self, tree: &LayoutSession<'_>, node: &ShapeNode) -> Option<f32> {
        let style = style_of(tree, node.id);
        if grows_with_content(&style) != Some(false) {
            return None;
        }
        let in_column_flex = node.parent.is_some_and(|parent| {
            let parent = self.nodes[parent].id;
            layout_node(tree, parent).display() == DisplayValue::Flex
                && !style_of(tree, parent).flex_direction_is_row()
        });
        if in_column_flex && (style.flex_grow() > 0.0 || style.flex_basis() != Dimension::auto()) {
            return None;
        }
        let height = style_dimension_length(style.size().height)?;
        Some(if style.box_sizing() == BoxSizing::BorderBox {
            height
        } else {
            height + block_edges(tree, &style)
        })
    }

    /// The shape of `container`, a flex container the stream slices whole,
    /// when breaks change its layout (see [`holds_moves`]); `Ok(None)` when
    /// they do not, and `Err` when what moves sits inside boxes whose growth
    /// is not modelled here: a grid, a box with a minimum or maximum block
    /// size, a multi-line or reversed column flex container, a row flex
    /// container with a gap between its lines or with items that avoid
    /// breaks, a flex line aligned other than at its start or by
    /// stretching, a relative offset, or absolutely positioned boxes, or
    /// when a break avoided before its first box or after its last falls
    /// outside it.
    pub(super) fn build(tree: &LayoutSession<'_>, container: usize) -> Result<Option<Self>, ()> {
        if !holds_moves(tree, container) {
            return Ok(None);
        }
        if holds_kept_boxes(tree, container) && avoids_breaks_at_ends(tree, container) {
            return Err(());
        }
        let mut shape = Shape { nodes: Vec::new() };
        shape.add(tree, container, None)?;
        Ok(Some(shape))
    }

    fn add(
        &mut self,
        tree: &LayoutSession<'_>,
        id: usize,
        parent: Option<usize>,
    ) -> Result<usize, ()> {
        let index = self.nodes.len();
        let mut grows = false;
        // Positions in flow decide the breaks, which a relative offset does
        // not move.
        if parent.is_some()
            && style_of(tree, id).inset() != Edges::splat(LengthPercentageAuto::auto())
        {
            return Err(());
        }
        let avoids = parent.is_some() && avoids_break_inside(layout_node(tree, id).break_inside);
        let kind = if parent.is_some() && is_monolithic_leaf(tree, id) {
            Kind::Monolithic
        } else if parent.is_some() && !holds_moves(tree, id) {
            Kind::Leaf
        } else {
            let node = layout_node(tree, id);
            let style = style_of(tree, id);
            if node.contains_absolute_boxes
                || rendered_elements(tree, id).count() != in_flow_elements(tree, id).len()
            {
                return Err(());
            }
            grows = grows_with_content(&style).ok_or(())?;
            match node.display() {
                DisplayValue::Block | DisplayValue::FlowRoot => Kind::Flow,
                DisplayValue::Flex if !style.flex_direction_is_row() => {
                    // A percentage gap resolves against a block size that
                    // the style fixes, not against one its content grows.
                    if style.flex_wraps()
                        || style.flex_direction_is_reversed()
                        || (grows && !is_length(style.gap().height))
                    {
                        return Err(());
                    }
                    Kind::Flow
                }
                // A gap between flex lines at a break is truncated (CSS Box
                // Alignment 3, 8.1), which would move the lines after it up.
                // A break between flex lines that an item avoids is not
                // modelled here.
                DisplayValue::Flex
                    if !style.flex_lines_are_reversed()
                        && is_zero(style.gap().height)
                        && !in_flow_elements(tree, id).into_iter().any(|item| {
                            let item = layout_node(tree, item);
                            avoids_break(item.break_before) || avoids_break(item.break_after)
                        }) =>
                {
                    Kind::Row
                }
                _ => return Err(()),
            }
        };
        let descends = matches!(kind, Kind::Flow | Kind::Row);
        self.nodes.push(ShapeNode {
            id,
            parent,
            kind,
            avoids,
            grows,
            children: Vec::new(),
            lines: Vec::new(),
        });
        if !descends {
            return Ok(index);
        }
        // A flex container lays out its items in `order` (CSS Flexbox 1,
        // 5.4), which keeps the order of the tree among equal values.
        let mut items = in_flow_elements(tree, id);
        if layout_node(tree, id).display() == DisplayValue::Flex {
            let cascade = tree.style_views.cascade();
            items.sort_by_key(|&item| cascade.computed[item].order);
        }
        let mut children = Vec::new();
        for child in items {
            children.push((child, self.add(tree, child, Some(index))?));
        }
        if matches!(self.nodes[index].kind, Kind::Row) {
            let align_items = style_of(tree, id).align_items();
            let mut grouped = Vec::new();
            for line in flex_lines(tree, id, children.iter().map(|&(child, _)| child).collect()) {
                let mut items = Vec::with_capacity(line.len());
                for item in line {
                    let style = style_of(tree, item);
                    let margin = style.margin();
                    let stretched = match style.align_self().unwrap_or(align_items).keyword {
                        AlignItemsKeyword::Normal | AlignItemsKeyword::Stretch => {
                            grows_with_content(&style) == Some(true)
                                && !margin.top.is_auto()
                                && !margin.bottom.is_auto()
                        }
                        AlignItemsKeyword::Start
                        | AlignItemsKeyword::FlexStart
                        | AlignItemsKeyword::SelfStart => false,
                        _ => return Err(()),
                    };
                    // An item that the line does not stretch stays at its
                    // start only without an automatic block-start margin.
                    if !stretched && margin.top.is_auto() {
                        return Err(());
                    }
                    let index = children
                        .iter()
                        .find(|&&(child, _)| child == item)
                        .map(|&(_, index)| index)
                        .ok_or(())?;
                    items.push((index, stretched));
                }
                grouped.push(items);
            }
            self.nodes[index].lines = grouped;
        }
        self.nodes[index].children = children.into_iter().map(|(_, index)| index).collect();
        Ok(index)
    }

    /// The block size of the tallest monolithic box, which a column must be
    /// at least as tall as to hold it.
    pub(super) fn tallest(&self, tree: &LayoutSession<'_>) -> f32 {
        self.nodes
            .iter()
            .filter(|node| matches!(node.kind, Kind::Monolithic))
            .map(|node| tree.nodes[node.id].layout.size.height)
            .fold(0.0, f32::max)
    }

    /// Where the monolithic boxes move when the border box of the container
    /// starts `start` below the top of a column, in columns `column_height`
    /// tall, and how the boxes around them grow. `height` is the block size
    /// of the container, whose own node does not have it yet.
    ///
    /// The break that moves the highest box is taken first: a box that
    /// starts a flex line moves the line before the boxes inside its items
    /// are placed in their new columns.
    pub(super) fn push(
        &self,
        tree: &LayoutSession<'_>,
        start: f32,
        column_height: f32,
        height: f32,
    ) -> Outcome {
        let mut state = State {
            shape: self,
            tree,
            moved: vec![0.0; self.nodes.len()],
            grown: vec![0.0; self.nodes.len()],
            growth: 0.0,
        };
        // A box that ends a column within rounding starts the next one.
        let column_of = |position: f32| ((position + EPSILON) / column_height).floor();
        // The highest break before `chain[0]` that falls in `column` above
        // its top: a break before the first box of a block or of a flex line
        // is a break before the block or the line, and one that a box
        // avoids moves up to the box before it, as far up as that holds and
        // what moves does not start its column.
        let pick = |state: &State<'_, '_>, chain: Vec<Target>, column: f32| {
            let mut target = None;
            for candidate in chain {
                let above = start + state.region_top(candidate);
                if column_of(above) != column || above - column * column_height <= EPSILON {
                    break;
                }
                target = Some(candidate);
            }
            target
        };
        // Each move puts a box at the top of a column, and nothing above the
        // break it makes moves after it, so there are at most a few moves for
        // each box; the bound keeps rounding from looping, and what is left
        // unmoved then overflows like a box that is cut.
        // The least amount by which taller columns would keep a box from
        // moving down.
        let mut shortage = f32::INFINITY;
        // The boxes whose space before them a break has truncated.
        let mut truncated = vec![false; self.nodes.len()];
        for _ in 0..4 * self.nodes.len() + 4 {
            // The earliest break to make: what moves, how far, where, and
            // how much taller the column would have to be for it not to
            // move.
            let mut first: Option<(Target, f32, f32, f32)> = None;
            let mut consider = |target: Target, distance: f32, at: f32, short: f32| {
                if first.is_none_or(|(_, _, earliest, _)| at < earliest - EPSILON) {
                    first = Some((target, distance, at, short));
                }
            };
            // A box kept whole that a column end would cut moves to the next
            // column with what the break before it moves.
            for index in self.kept_whole() {
                let position = start + state.top(index);
                let column = column_of(position);
                let offset = position - column * column_height;
                // A box whose margin box starts its column stays, as does
                // one that fits. A box that avoids breaks inside it moves
                // even where no column holds it: it is cut either way, and
                // the break before it is not avoided.
                let margin = state.layout(index).margin.top.max(0.0);
                if offset <= EPSILON
                    || (offset - margin).abs() <= EPSILON
                    || offset + state.height(index) <= column_height + EPSILON
                {
                    continue;
                }
                let target = pick(&state, self.breaks_before(tree, index), column)
                    .unwrap_or(Target::Flow(index));
                if matches!(target, Target::Container) {
                    return Outcome::MovesContainer;
                }
                let top = start + state.region_top(target);
                let short = offset + state.height(index) - column_height;
                consider(target, (column + 1.0) * column_height - top, top, short);
            }
            // A column end between two boxes of a flow breaks there. Where
            // the break is avoided, the box before it moves there. Otherwise,
            // in a column flex container, the margin box of the item after it
            // starts the next column, with the block-end margin of the item
            // before and the gap truncated; its own block-start margin is
            // kept, as in Chromium.
            for (gap, prev, next) in self.flow_pairs(tree) {
                let end = start + state.top(prev) + state.height(prev);
                let border = start + state.top(next);
                let top = border - state.layout(next).margin.top.max(0.0);
                // The break falls at the first column end after `prev`, even
                // where the space after it spans more than a column.
                let column = column_of(end - 2.0 * EPSILON) + 1.0;
                let boundary = column * column_height;
                if boundary <= start + EPSILON || boundary > top + EPSILON {
                    continue;
                }
                let earlier = if self.avoids_break_between(tree, prev, next) {
                    pick(&state, self.breaks_before(tree, prev), column - 1.0)
                } else {
                    None
                };
                if matches!(earlier, Some(Target::Container)) {
                    return Outcome::MovesContainer;
                }
                // A box that a move already put at the top of a column
                // starts it, its block-start margin truncated there.
                let starts_column = border - column_of(border) * column_height <= EPSILON;
                if let Some(target) = earlier {
                    // Columns as much taller would keep the break where it is.
                    let above = start + state.region_top(target);
                    consider(target, boundary - above, above, boundary - above);
                } else if let Some(gap) = gap.filter(|_| !starts_column && !truncated[next]) {
                    // Only the gap and the margin are truncated, not the
                    // free space the container distributes.
                    let truncated = (top - boundary).min(gap + state.layout(prev).margin.bottom);
                    if truncated > EPSILON {
                        consider(Target::Flow(next), -truncated, end, f32::INFINITY);
                    }
                }
            }
            let Some((target, distance, _, short)) = first else {
                break;
            };
            // The space before a box is truncated once: what is left of it
            // after the gap and the margin is free space that stays.
            if let (Target::Flow(index), true) = (target, distance < 0.0) {
                truncated[index] = true;
            }
            shortage = shortage.min(short);
            state.move_down(target, distance);
        }
        // What a box with a block size of its own no longer holds overflows
        // it, which the pieces of the container show only inside it.
        let end = height + state.growth;
        if self
            .kept_whole()
            .any(|index| state.top(index) + state.height(index) > end + EPSILON)
        {
            return Outcome::Unmodelled;
        }
        let changes = self
            .nodes
            .iter()
            .enumerate()
            .skip(1)
            .filter(|&(index, _)| state.moved[index] != 0.0 || state.grown[index] != 0.0)
            .map(|(index, node)| (node.id, state.moved[index], state.grown[index]))
            .collect();
        Outcome::Moves(Pushes {
            changes,
            growth: state.growth,
            shortage: shortage.is_finite().then_some(shortage),
        })
    }

    /// The indices of the boxes kept whole, monolithic or avoiding breaks
    /// inside them, in order.
    fn kept_whole(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.nodes.len()).filter(|&index| {
            let node = &self.nodes[index];
            node.avoids || matches!(node.kind, Kind::Monolithic)
        })
    }

    /// The pairs of adjacent boxes in each flow, in order, with the row gap
    /// between them when the flow is that of a flex container, whose breaks
    /// truncate it. A percentage gap resolves against the content box of a
    /// container whose style fixes its block size, the only kind with such
    /// a gap that holds a flow (see `add`).
    fn flow_pairs(&self, tree: &LayoutSession<'_>) -> Vec<(Option<f32>, usize, usize)> {
        self.nodes
            .iter()
            .filter(|node| matches!(node.kind, Kind::Flow))
            .flat_map(|node| {
                let gap = (layout_node(tree, node.id).display() == DisplayValue::Flex).then(|| {
                    let layout = &tree.nodes[node.id].layout;
                    let content = layout.size.height
                        - layout.padding.top
                        - layout.padding.bottom
                        - layout.border.top
                        - layout.border.bottom;
                    let gap = style_of(tree, node.id).gap().height;
                    super::super::multicol_resolve_inset(tree, gap, content.max(0.0))
                });
                node.children
                    .windows(2)
                    .map(move |pair| (gap, pair[0], pair[1]))
            })
            .collect()
    }

    /// Whether a break between `prev` and `next`, adjacent boxes of a flow,
    /// is avoided.
    fn avoids_break_between(&self, tree: &LayoutSession<'_>, prev: usize, next: usize) -> bool {
        avoids_break_after(tree, self.nodes[prev].id)
            || avoids_break_before(tree, self.nodes[next].id)
    }

    /// What a break before the box `index` may move, from the box itself
    /// up: a break before the first box of a block is one before the block,
    /// one before a flex item at the start of its line is one before the
    /// line, and one before the first line of a flex container is one
    /// before the container. A break between two boxes of a flow that
    /// either avoids is replaced by the break before the first of them. An item that a block-start margin keeps
    /// below the start of its line moves alone, its margin truncated, as in
    /// Chromium.
    fn breaks_before(&self, tree: &LayoutSession<'_>, index: usize) -> Vec<Target> {
        let mut chain = Vec::new();
        let mut current = index;
        while let Some(parent) = self.nodes[current].parent {
            match &self.nodes[parent].kind {
                Kind::Row => {
                    let line = self.nodes[parent]
                        .lines
                        .iter()
                        .position(|items| items.iter().any(|&(item, _)| item == current));
                    if tree.nodes[self.nodes[current].id].layout.margin.top > EPSILON {
                        chain.push(Target::Item(current));
                        break;
                    }
                    chain.push(Target::Line(current));
                    if line != Some(0) {
                        break;
                    }
                }
                _ => {
                    chain.push(Target::Flow(current));
                    let siblings = &self.nodes[parent].children;
                    let at = siblings.iter().position(|&child| child == current);
                    if let Some(at) = at.filter(|&at| at > 0) {
                        // A break the boxes avoid moves to the one before the
                        // box above.
                        let prev = siblings[at - 1];
                        if !self.avoids_break_between(tree, prev, current) {
                            break;
                        }
                        current = prev;
                        continue;
                    }
                }
            }
            if self.nodes[parent].parent.is_none() {
                chain.push(Target::Container);
                break;
            }
            current = parent;
        }
        chain
    }
}

/// The moves and growth applied so far.
struct State<'a, 't> {
    shape: &'a Shape,
    tree: &'a LayoutSession<'t>,
    moved: Vec<f32>,
    grown: Vec<f32>,
    growth: f32,
}

impl State<'_, '_> {
    fn layout(&self, index: usize) -> &raikiri_layout_geometry::BoxLayout {
        &self.tree.nodes[self.shape.nodes[index].id].layout
    }

    fn height(&self, index: usize) -> f32 {
        self.layout(index).size.height + self.grown[index]
    }

    /// Block offset of the border box of `index` from that of its parent.
    fn offset(&self, index: usize) -> f32 {
        self.layout(index).location.y + self.moved[index]
    }

    /// Block offset of the border box of `index` from that of the container.
    fn top(&self, index: usize) -> f32 {
        let mut top = 0.0;
        let mut current = index;
        while let Some(parent) = self.shape.nodes[current].parent {
            top += self.offset(current);
            current = parent;
        }
        top
    }

    /// Block offset from the border box of the container of the top of what
    /// `target` moves: the border box of a box, whose margin before a break
    /// is truncated, and the block-start margin edge of the highest item of
    /// a flex line, which moves whole.
    fn region_top(&self, target: Target) -> f32 {
        match target {
            Target::Container => 0.0,
            Target::Flow(index) | Target::Item(index) => self.top(index),
            Target::Line(index) => {
                let parent = self.shape.nodes[index].parent.unwrap_or_default();
                self.top(parent)
                    + self
                        .line_of(parent, index)
                        .iter()
                        .map(|&(item, _)| self.offset(item) - self.layout(item).margin.top)
                        .fold(self.offset(index), f32::min)
            }
        }
    }

    /// The items of the flex line of row container `parent` that holds
    /// `item`, with whether each one stretches.
    fn line_of(&self, parent: usize, item: usize) -> Vec<(usize, bool)> {
        self.shape.nodes[parent]
            .lines
            .iter()
            .find(|items| items.iter().any(|&(index, _)| index == item))
            .cloned()
            .unwrap_or_default()
    }

    /// Where the margin box of the line of `item` ends: that of its
    /// lowest item.
    fn line_end(&self, parent: usize, item: usize) -> f32 {
        self.line_of(parent, item)
            .iter()
            .map(|&(index, _)| self.end(index))
            .fold(f32::NEG_INFINITY, f32::max)
    }

    /// Lay out again the line of `item` in row container `parent` after its
    /// items moved or grew, from where it ended, `old_end`: it ends where
    /// its lowest item now does, a stretched item at the end of its content,
    /// the items that stretch reach that end, and the lines after it move
    /// down by as much as it grew, which grows the container too.
    fn relayout_line(&mut self, parent: usize, item: usize, old_end: f32) {
        let line = self.line_of(parent, item);
        let natural_end = |state: &Self, index: usize, stretched: bool| {
            if stretched {
                state.offset(index) + state.content_end(index) + state.layout(index).margin.bottom
            } else {
                state.end(index)
            }
        };
        let end = line
            .iter()
            .map(|&(index, stretched)| natural_end(self, index, stretched))
            .fold(old_end, f32::max);
        for &(index, stretched) in &line {
            if stretched {
                self.grown[index] += end - self.end(index);
            }
        }
        let grown = end - old_end;
        if grown > 0.0 {
            let following = self.following(parent, item, false);
            self.shift(following, grown);
            self.content_grew(parent, grown);
        }
    }

    /// Block offset of the block-end margin edge of `index` from the border
    /// box of its parent.
    fn end(&self, index: usize) -> f32 {
        self.offset(index) + self.height(index) + self.layout(index).margin.bottom
    }

    /// Block offset of the block-end content edge plus padding and border of
    /// `index`, from its border box: where its content ends.
    fn content_end(&self, index: usize) -> f32 {
        let layout = self.layout(index);
        let edges = layout.padding.bottom + layout.border.bottom;
        self.shape.nodes[index]
            .children
            .iter()
            .map(|&child| self.end(child))
            .fold(layout.padding.top + layout.border.top, f32::max)
            + edges
    }

    fn shift(&mut self, indices: impl IntoIterator<Item = usize>, distance: f32) {
        for index in indices {
            self.moved[index] += distance;
        }
    }

    /// The boxes after `child` in the flow of `parent`, or the items of the
    /// flex lines after the one holding `child`, with the items of that
    /// line when `line` is set.
    fn following(&self, parent: usize, child: usize, line: bool) -> Vec<usize> {
        let node = &self.shape.nodes[parent];
        match &node.kind {
            Kind::Row => {
                let lines = &node.lines;
                let at = lines
                    .iter()
                    .position(|items| items.iter().any(|&(item, _)| item == child))
                    .unwrap_or(lines.len());
                lines
                    .iter()
                    .enumerate()
                    .filter(|&(index, _)| index > at || line && index == at)
                    .flat_map(|(_, items)| items.iter().map(|&(item, _)| item))
                    .collect()
            }
            _ => {
                let at = node.children.iter().position(|&index| index == child);
                let from = at.map_or(node.children.len(), |at| if line { at } else { at + 1 });
                node.children[from..].to_vec()
            }
        }
    }

    /// Move what `target` names down by `distance`: a box with the boxes
    /// after it in its flow, a flex line with the lines after it, or an
    /// item alone. The caller moves the container itself (see
    /// [`Outcome::MovesContainer`]).
    fn move_down(&mut self, target: Target, distance: f32) {
        if let Target::Item(index) = target {
            let parent = self.shape.nodes[index].parent.unwrap_or_default();
            let old_end = self.line_end(parent, index);
            self.shift([index], distance);
            self.relayout_line(parent, index, old_end);
        } else if let Target::Flow(index) | Target::Line(index) = target {
            let parent = self.shape.nodes[index].parent.unwrap_or_default();
            let moving = self.following(parent, index, true);
            self.shift(moving, distance);
            self.content_grew(parent, distance);
        }
    }

    /// The content of `index` reaches `distance` further down: it grows, and
    /// so do the boxes that hold it.
    fn content_grew(&mut self, index: usize, distance: f32) {
        if !self.shape.nodes[index].grows {
            return;
        }
        let Some(parent) = self.shape.nodes[index].parent else {
            self.growth += distance;
            return;
        };
        match &self.shape.nodes[parent].kind {
            Kind::Row => {
                // A stretched item grows with its line; one that does not
                // stretch grows with its content.
                let old_end = self.line_end(parent, index);
                if !self
                    .line_of(parent, index)
                    .iter()
                    .any(|&(item, stretched)| item == index && stretched)
                {
                    self.grown[index] += distance;
                }
                self.relayout_line(parent, index, old_end);
            }
            _ => {
                self.grown[index] += distance;
                let following = self.following(parent, index, false);
                self.shift(following, distance);
                self.content_grew(parent, distance);
            }
        }
    }
}
