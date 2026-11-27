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

use super::*;
use raikiri_layout_style::values::{AlignItemsKeyword, BoxSizing};

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
        && !sets_break_in_columns(node.break_before)
        && !sets_break_in_columns(node.break_after)
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

/// Whether the monolithic boxes inside `container`, a flex container the
/// stream slices, sit in boxes whose moves [`Shape`] models (see
/// [`Shape::build`]), and no column is shorter than one of them. A box taller
/// than its column overflows it, and the fragment of the container around
/// it grows to hold it, which the pieces of the stream do not model: that
/// needs a content height of the column container at least as tall as the
/// tallest box, or, with no height of its own, columns that balance with
/// nothing to limit them, which grow to the tallest box.
pub(super) fn supports(tree: &LayoutSession<'_>, container: usize) -> bool {
    let Ok(Some(shape)) = Shape::build(tree, container) else {
        return false;
    };
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
    /// A box that holds no monolithic box, which moves as a whole.
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

/// The changes the monolithic boxes of a container make to its layout: how
/// far each box moves down and how much it grows, and how much the
/// container itself grows.
#[derive(Debug, Default)]
pub(super) struct Pushes {
    pub(super) changes: Vec<(usize, f32, f32)>,
    pub(super) growth: f32,
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

/// Whether `id` holds a monolithic box anywhere in flow below it.
fn holds_monolithic(tree: &LayoutSession<'_>, id: usize) -> bool {
    in_flow_elements(tree, id)
        .into_iter()
        .any(|child| is_monolithic_leaf(tree, child) || holds_monolithic(tree, child))
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
    /// when it holds monolithic boxes; `Ok(None)` when it holds none, and
    /// `Err` when it holds them inside boxes whose growth is not modelled
    /// here: a grid, a box with a minimum or maximum block size, a
    /// multi-line or reversed column flex container, a flex container with a
    /// gap between its lines, a flex line aligned other than at its start or
    /// by stretching, a relative offset, or absolutely positioned boxes.
    pub(super) fn build(tree: &LayoutSession<'_>, container: usize) -> Result<Option<Self>, ()> {
        if !holds_monolithic(tree, container) {
            return Ok(None);
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
        let kind = if parent.is_some() && is_monolithic_leaf(tree, id) {
            Kind::Monolithic
        } else if parent.is_some() && !holds_monolithic(tree, id) {
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
            // A gap at a break is truncated (CSS Box Alignment 3, 8.1),
            // which would move the boxes after it up.
            let no_gap = |gap: LengthPercentage| matches!(gap.expand(), ExpandedLengthPercentage::Length(px) if px == 0.0);
            if node.display() == DisplayValue::Flex && !no_gap(style.gap().height) {
                return Err(());
            }
            match node.display() {
                DisplayValue::Block | DisplayValue::FlowRoot => Kind::Flow,
                DisplayValue::Flex if !style.flex_direction_is_row() => {
                    if style.flex_wraps() || style.flex_direction_is_reversed() {
                        return Err(());
                    }
                    Kind::Flow
                }
                DisplayValue::Flex if !style.flex_lines_are_reversed() => Kind::Row,
                _ => return Err(()),
            }
        };
        let descends = matches!(kind, Kind::Flow | Kind::Row);
        self.nodes.push(ShapeNode {
            id,
            parent,
            kind,
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
        // Each move puts a box at the top of a column, so there are no more
        // moves than boxes; the bound keeps rounding from looping, and what
        // is left unmoved then overflows like a box that is cut.
        for _ in 0..=self.nodes.len() {
            // The box to move for each monolithic box that does not fit, with
            // the top of what moves and the column it is in.
            let mut first: Option<(Target, f32, f32)> = None;
            for index in self.monolithic() {
                let position = start + state.top(index);
                let column = column_of(position);
                let offset = position - column * column_height;
                if offset <= EPSILON || offset + state.height(index) <= column_height + EPSILON {
                    continue;
                }
                // A break before the first box of a block or of a flex line
                // is a break before the block or the line, as far up as that
                // holds and what moves does not start its column.
                let mut target = None;
                for candidate in self.breaks_before(tree, index) {
                    let above = start + state.region_top(candidate);
                    if column_of(above) != column || above - column * column_height <= EPSILON {
                        break;
                    }
                    target = Some(candidate);
                }
                let target = target.unwrap_or(Target::Flow(index));
                if matches!(target, Target::Container) {
                    return Outcome::MovesContainer;
                }
                let top = start + state.region_top(target);
                if first.is_none_or(|(_, earliest, _)| top < earliest - EPSILON) {
                    first = Some((target, top, column));
                }
            }
            let Some((target, top, column)) = first else {
                break;
            };
            state.move_down(target, (column + 1.0) * column_height - top);
        }
        // What a box with a block size of its own no longer holds overflows
        // it, which the pieces of the container show only inside it.
        let end = height + state.growth;
        if self
            .monolithic()
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
        })
    }

    /// The indices of the monolithic boxes, in order.
    fn monolithic(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.nodes.len()).filter(|&index| matches!(self.nodes[index].kind, Kind::Monolithic))
    }

    /// What a break before the monolithic box `index` may move, from the
    /// box itself up: a break before the first box of a block is one before
    /// the block, one before a flex item at the start of its line is one
    /// before the line, and one before the first line of a flex container
    /// is one before the container. An item that a block-start margin keeps
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
                    if self.nodes[parent].children.first() != Some(&current) {
                        break;
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
