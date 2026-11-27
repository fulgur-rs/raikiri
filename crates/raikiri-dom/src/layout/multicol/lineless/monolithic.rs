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
use raikiri_layout_style::values::AlignItemsKeyword;

/// Whether `id`, a box inside a flex container the stream slices, is a
/// monolithic box that the stream moves whole: a block with size containment
/// in the block axis and nothing rendered inside it, in flow, with block
/// margins of a length and no forced break of its own.
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
        && !has_rendered_content(tree, id, false)
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
    /// A row flex container, with its flex lines in order, each holding the
    /// indices of its items and whether each one stretches to the line.
    Row(Vec<Vec<(usize, bool)>>),
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
    /// The shape of `container`, a flex container the stream slices whole,
    /// when it holds monolithic boxes; `Ok(None)` when it holds none, and
    /// `Err` when it holds them inside boxes whose growth is not modelled
    /// here: a grid, a box with a block size of its own, a multi-line or
    /// reordered column flex container, a flex line aligned other than at
    /// its start or by stretching, or absolutely positioned boxes.
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
                DisplayValue::Flex if !style.flex_lines_are_reversed() => Kind::Row(Vec::new()),
                _ => return Err(()),
            }
        };
        let descends = matches!(kind, Kind::Flow | Kind::Row(_));
        self.nodes.push(ShapeNode {
            id,
            parent,
            kind,
            grows,
            children: Vec::new(),
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
        if let Kind::Row(lines) = &mut self.nodes[index].kind {
            let align_items = style_of(tree, id).align_items();
            let mut grouped = Vec::new();
            for line in flex_lines(tree, id, children.iter().map(|&(child, _)| child).collect()) {
                let mut items = Vec::with_capacity(line.len());
                for item in line {
                    let style = style_of(tree, item);
                    let margin = style.margin();
                    let stretched = match style.align_self().unwrap_or(align_items).keyword {
                        AlignItemsKeyword::Normal | AlignItemsKeyword::Stretch => {
                            style.size().height.is_auto()
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
            *lines = grouped;
        }
        self.nodes[index].children = children.into_iter().map(|(_, index)| index).collect();
        Ok(index)
    }

    /// Where the monolithic boxes move when the border box of the container
    /// starts `start` below the top of a column, in columns `column_height`
    /// tall, and how the boxes around them grow. `height` is the block size
    /// of the container, whose own node does not have it yet. `None` when a
    /// monolithic
    /// box is taller than a column, which overflows it, or when the
    /// container itself would have to move to the next column.
    /// The block size of the tallest monolithic box, which a column must be
    /// at least as tall as to hold it.
    pub(super) fn tallest(&self, tree: &LayoutSession<'_>) -> f32 {
        self.nodes
            .iter()
            .filter(|node| matches!(node.kind, Kind::Monolithic))
            .map(|node| tree.nodes[node.id].layout.size.height)
            .fold(0.0, f32::max)
    }

    pub(super) fn push(
        &self,
        tree: &LayoutSession<'_>,
        start: f32,
        column_height: f32,
        height: f32,
    ) -> Option<Pushes> {
        let mut state = State {
            shape: self,
            tree,
            moved: vec![0.0; self.nodes.len()],
            grown: vec![0.0; self.nodes.len()],
            growth: 0.0,
        };
        let column_of = |position: f32| {
            let column = (position / column_height).floor();
            // A box that ends a column within rounding starts the next one.
            if position - column * column_height > column_height - EPSILON {
                column + 1.0
            } else {
                column
            }
        };
        for index in 0..self.nodes.len() {
            if !matches!(self.nodes[index].kind, Kind::Monolithic) {
                continue;
            }
            let size = state.height(index);
            if size > column_height + EPSILON {
                return None;
            }
            let position = start + state.top(index);
            let column = column_of(position);
            let offset = position - column * column_height;
            if offset <= EPSILON || offset + size <= column_height + EPSILON {
                continue;
            }
            // A break before the first box of a block or of a flex line is
            // a break before the block or the line, as far up as that holds
            // and the box does not start its column.
            let mut target = index;
            let mut current = index;
            while let Some(parent) = self.nodes[current].parent {
                if !self.starts(parent, current) {
                    break;
                }
                let above = start + state.top(parent);
                if column_of(above) != column || above - column * column_height <= EPSILON {
                    break;
                }
                if self.nodes[parent].parent.is_none() {
                    // The container itself would move.
                    return None;
                }
                target = parent;
                current = parent;
            }
            let distance = column_height - (start + state.top(target) - column * column_height);
            state.move_down(target, distance);
        }
        // What a box with a block size of its own no longer holds overflows
        // it, which the pieces of the container show only inside it.
        let end = height + state.growth;
        if (0..self.nodes.len()).any(|index| {
            matches!(self.nodes[index].kind, Kind::Monolithic)
                && state.top(index) + state.height(index) > end + EPSILON
        }) {
            return None;
        }
        let mut changes = Vec::new();
        for (index, node) in self.nodes.iter().enumerate().skip(1) {
            if state.moved[index] != 0.0 || state.grown[index] != 0.0 {
                changes.push((node.id, state.moved[index], state.grown[index]));
            }
        }
        Some(Pushes {
            changes,
            growth: state.growth,
        })
    }

    /// Whether `child` is the first box inside `parent`, or in its first flex
    /// line.
    fn starts(&self, parent: usize, child: usize) -> bool {
        match &self.nodes[parent].kind {
            Kind::Row(lines) => lines
                .first()
                .is_some_and(|line| line.iter().any(|&(item, _)| item == child)),
            _ => self.nodes[parent].children.first() == Some(&child),
        }
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
            Kind::Row(lines) => {
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

    /// Move `index` down by `distance`, with what follows it in the flow of
    /// its parent; in a flex line, the whole line moves.
    fn move_down(&mut self, index: usize, distance: f32) {
        let parent = self.shape.nodes[index]
            .parent
            .expect("the container never moves");
        let moving = self.following(parent, index, true);
        self.shift(moving, distance);
        self.content_grew(parent, distance);
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
            Kind::Row(lines) => {
                let line = lines
                    .iter()
                    .find(|items| items.iter().any(|&(item, _)| item == index))
                    .cloned()
                    .unwrap_or_default();
                let stretched = line
                    .iter()
                    .any(|&(item, stretched)| item == index && stretched);
                let line_end = line
                    .iter()
                    .map(|&(item, _)| self.end(item))
                    .fold(f32::NEG_INFINITY, f32::max);
                // A stretched item is as tall as its line, or as its content
                // when that is taller; the content has grown already.
                let item_end = if stretched {
                    self.offset(index) + self.content_end(index) + self.layout(index).margin.bottom
                } else {
                    self.end(index) + distance
                };
                let grown_line = (item_end - line_end).max(0.0);
                if !stretched {
                    self.grown[index] += distance;
                }
                for &(item, stretched) in &line {
                    if stretched {
                        self.grown[item] += grown_line;
                    }
                }
                if grown_line > 0.0 {
                    let following = self.following(parent, index, false);
                    self.shift(following, grown_line);
                    self.content_grew(parent, grown_line);
                }
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
