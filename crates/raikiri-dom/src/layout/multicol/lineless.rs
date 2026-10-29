//! Column fragmentation of block children that hold no lines.
//!
//! A block box with no in-flow content of its own (for example a sized,
//! empty `div`) has no line boxes to break between, but it is still
//! fragmentable: a box taller than the remaining column continues in the
//! next column (CSS Fragmentation 3, 4.4; CSS Multi-column 1, 7.1). The other
//! strategies place such a box atomically in one column. This one lays the
//! children out as one stream and slices each box at the column boundaries.

use super::*;
use crate::fragment::{FragmentRect, LayoutFragment};
use crate::layout::context::LayoutSession;
use crate::layout::ifc::projection::{ATOMIC_TAGS, REPLACED_BOX_TAGS};
use raikiri_layout_geometry::{Edges, Point};
use raikiri_style::property::BreakInside;

/// The rendered children of `parent` when every one of them is a plain
/// block with no content, and at least one of them has a block size (or, with
/// `out_of_flow`, holds an absolutely positioned box).
///
/// Inline margins, generated content, forced or avoided breaks between
/// boxes, absolutely positioned or floated boxes, scroll containers and
/// replaced elements need their own handling at a break, so containers with
/// them keep the other strategies. A `position: relative` box with insets is
/// sliced like any other: its column location carries no offset, which the
/// painter adds. That holds only for a box with nothing rendered inside it, so
/// one that holds blocks or absolutely positioned boxes keeps the other
/// strategies when it has insets, and so does a box inside a wrapper.
///
/// With `out_of_flow`, a child may also hold absolutely positioned boxes
/// that it is the containing block of; [`layout`] breaks them across the
/// columns as well. Such a child may also hold plain blocks with no
/// content (see [`wrapped_blocks`]), which take its place in the stream.
pub(super) fn supported(
    tree: &LayoutSession<'_>,
    parent: usize,
    out_of_flow: bool,
) -> Option<Vec<usize>> {
    supported_with_wrapped_blocks(tree, parent, out_of_flow, false)
}

/// [`supported`] for an outermost multicol container. Its stream may also
/// hold the blocks of eligible block-only wrappers in their place.
pub(super) fn supported_with_wrappers(
    tree: &LayoutSession<'_>,
    parent: usize,
) -> Option<Vec<usize>> {
    supported_with_wrapped_blocks(tree, parent, true, true)
}

fn supported_with_wrapped_blocks(
    tree: &LayoutSession<'_>,
    parent: usize,
    out_of_flow: bool,
    allow_wrapped_blocks: bool,
) -> Option<Vec<usize>> {
    let container = layout_node(tree, parent);
    if container.ifc.is_some()
        || !inherits_horizontal(tree, parent)
        || container.has_before_or_after_content
    {
        return None;
    }
    let mut children = Vec::new();
    let mut boxes = Vec::new();
    let mut sized_flex = false;
    for child in rendered_elements(tree, parent) {
        // A flex container is sliced whole, with the boxes it holds.
        if let Some(sized) = allow_wrapped_blocks
            .then(|| sliceable_flex_container(tree, child, None))
            .flatten()
        {
            sized_flex |= sized;
            children.push(child);
            continue;
        }
        let node = layout_node(tree, child);
        let has_inset = style_of(tree, child).inset() != Edges::splat(LengthPercentageAuto::auto());
        let plain_block = is_plain_block_ignoring_insets(tree, child);
        let plain_wrapper = allow_wrapped_blocks
            && is_plain_wrapper_ignoring_insets(tree, child)
            && has_rendered_content(tree, child, false);
        if !(plain_block || plain_wrapper)
            || has_inset
                && (!super::insets_are_a_relative_offset(tree, child)
                    || has_rendered_content(tree, child, false))
        {
            return None;
        }
        if has_rendered_content(tree, child, out_of_flow && node.contains_absolute_boxes) {
            let leaves = ((out_of_flow && node.contains_absolute_boxes)
                || (allow_wrapped_blocks && plain_wrapper))
                .then(|| wrapped_blocks(tree, child, allow_wrapped_blocks))
                .flatten()?;
            boxes.extend(leaves.leaf_ids());
        } else {
            boxes.push(child);
        }
        children.push(child);
    }
    let sized = boxes
        .iter()
        .any(|&child| has_block_size(&style_of(tree, child)));
    // An empty containing block has no block size, but the absolutely
    // positioned boxes it holds still break across the columns, past the
    // last one if they start there.
    let holds_absolute_boxes = out_of_flow
        && children.iter().any(|&child| {
            layout_node(tree, child).contains_absolute_boxes
                && !absolute_children(tree, child).is_empty()
        });
    (sized || sized_flex || holds_absolute_boxes).then_some(children)
}

/// Whether `child` is a plain block with a block size of its own, nothing
/// rendered inside it and no block margins: a box that a stream of paragraphs
/// places whole in one column, as it has no lines to break between.
pub(super) fn is_sized_leaf(tree: &LayoutSession<'_>, child: usize) -> bool {
    let style = style_of(tree, child);
    is_plain_block_ignoring_insets(tree, child)
        && !has_rendered_content(tree, child, false)
        && has_block_size(&style)
        && block_margins(&style) == Some((0.0, 0.0))
}

/// Whether `c` is white space that collapses away, as opposed to the other
/// characters Unicode counts as white space, such as a no-break space, which
/// render.
fn is_collapsible_whitespace(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{000c}')
}

/// Whether `style` gives its box a block size of its own to slice: a height, a
/// minimum height, or an aspect ratio, which makes the height follow from the
/// width.
fn has_block_size(style: &LayoutStyleView<'_>) -> bool {
    style_dimension_length(style.size().height).is_some()
        || style_dimension_length(style.min_size().height.into()).is_some_and(|px| px > 0.0)
        || style.aspect_ratio().is_some()
}

/// The rendered element children of `parent`, with those of a `display:
/// contents` child in its place. Text that renders gives a box lines, which
/// the callers refuse, so any other node is collapsible white space.
pub(super) fn rendered_elements<'a>(
    tree: &'a LayoutSession<'_>,
    parent: usize,
) -> impl Iterator<Item = usize> + 'a {
    tree.children_through_contents(parent)
        .into_iter()
        .filter(|&child| {
            let node = layout_node(tree, child);
            node.is_in_document()
                && !node.is_display_none()
                && matches!(node.data, NodeData::Element(_))
        })
}

/// Whether `id` starts at the top of the first column of the multicol
/// container it is a child of, with no line or box before it, which leaves
/// it the whole column height.
pub(super) fn starts_the_outer_columns(tree: &LayoutSession<'_>, id: usize) -> bool {
    let Some(parent) = tree.taffy_parent_of(id) else {
        return false;
    };
    let outer = layout_node(tree, parent);
    let zero = LengthPercentage::length(0.0);
    outer.multicol.is_some()
        && !outer.has_before_or_after_content
        && style_of(tree, parent).padding().top == zero
        && style_of(tree, parent).border().top == zero
        // Text that makes lines gives the container inline content.
        && outer.ifc.is_none()
        && rendered_elements(tree, parent).next() == Some(id)
        && style_of(tree, id).margin().top == LengthPercentageAuto::length(0.0)
}

/// Whether `child` is a block that can be sliced at a column end, leaving
/// aside what it contains.
fn is_plain_block(tree: &LayoutSession<'_>, child: usize) -> bool {
    style_of(tree, child).inset() == Edges::splat(LengthPercentageAuto::auto())
        && is_plain_block_ignoring_insets(tree, child)
}

/// [`is_plain_block`] for a box whose insets the caller accepts.
fn is_plain_block_ignoring_insets(tree: &LayoutSession<'_>, child: usize) -> bool {
    is_plain_block_with_display(tree, child, false, false)
}

fn is_plain_wrapper_ignoring_insets(tree: &LayoutSession<'_>, child: usize) -> bool {
    is_plain_block_with_display(tree, child, true, false)
}

/// [`is_plain_block`] for a box that may hold lines of text, and whose insets
/// the caller accepts.
pub(super) fn is_plain_block_holding_lines(tree: &LayoutSession<'_>, child: usize) -> bool {
    is_plain_block_with_display(tree, child, false, true)
}

fn is_plain_block_with_display(
    tree: &LayoutSession<'_>,
    child: usize,
    allow_flow_root: bool,
    allow_lines: bool,
) -> bool {
    let node = layout_node(tree, child);
    let style = style_of(tree, child);
    let display = node.display();
    !(display != DisplayValue::Block && !(allow_flow_root && display == DisplayValue::FlowRoot)
        // An orthogonal flow is monolithic in the column's block axis.
        || !is_horizontal(node)
        // So is a box with size containment (CSS Contain 2, 3.1).
        || node.size_containment.height
        || node.layout_position() != Position::Relative
            && node.layout_position() != Position::Static
        || style.float().is_floated()
        || style.clear() != Clear::None
        || block_margins(&style).is_none()
        || style.overflow()
            != (Point {
                x: TaffyOverflow::Visible,
                y: TaffyOverflow::Visible,
            })
        || node.tag_name().is_some_and(|tag| {
            ATOMIC_TAGS.contains(&tag)
                || REPLACED_BOX_TAGS.contains(&tag)
                || matches!(tag, "fieldset" | "legend" | "details" | "summary")
        })
        || node.break_before != BreakBetween::Auto
        || node.break_after != BreakBetween::Auto
        || node.has_before_or_after_content
        || node.multicol.is_some()
        || node.ifc.is_some() && !allow_lines
        || is_right_aligned_in_parent(tree, child))
}

/// The most boxes a flex container may hold and still be sliced whole, so
/// that checking a very large one stays cheap.
const MAX_SLICED_FLEX_BOXES: usize = 256;

/// Whether `child` is a flex container that the stream slices as one box:
/// it and the boxes inside it hold no lines, so there is nowhere to break
/// between lines, and nothing that breaks on its own terms. The container is
/// laid out whole, and each column shows the part of it that falls in the
/// column (CSS Flexbox 1, 12). `Some(sized)` says whether any of them has a
/// block size of its own to slice.
///
/// `block_size` is the block size of the container in this layout, once the
/// boxes it holds are laid out. Their geometry then tells whether a box
/// reaches past the container. The check that settles which strategy takes
/// the container runs before they are laid out, with the geometry of an
/// earlier layout or none, so it passes `None` and judges the structure only.
///
/// Boxes that need handling at a break of their own keep the other
/// strategies: forced or avoided breaks, absolutely positioned or floated
/// boxes whose containing block is not inside the container, scroll
/// containers, replaced elements, boxes that paint past their own box (a
/// transform, shadow, outline or filter) or as a whole group, boxes whose
/// block edges repeat at each break, containers that are not block or flex,
/// and containers whose content overflows their block start or end.
fn sliceable_flex_container(
    tree: &LayoutSession<'_>,
    child: usize,
    block_size: Option<f32>,
) -> Option<bool> {
    let node = layout_node(tree, child);
    let style = style_of(tree, child);
    if node.display() != DisplayValue::Flex
        || style.inset() != Edges::splat(LengthPercentageAuto::auto())
        || style.float().is_floated()
        || style.clear() != Clear::None
        || block_margins(&style).is_none()
        || is_right_aligned_in_parent(tree, child)
    {
        return None;
    }
    // Boxes that reach past the block end of the container, absolutely
    // positioned or not, would continue in columns the container's own pieces
    // do not reach, and those that start above its block start would be cut
    // off by the clip of its first piece.
    let bounds = match block_size {
        Some(height) => {
            let top = tree.layout_origin_of(child)?.y;
            Some((top, top + height))
        }
        None => None,
    };
    let mut boxes = 0usize;
    let mut sized = false;
    // Whether a box on the way down is the containing block of the absolutely
    // positioned boxes below it, so that they are cut with the container.
    let mut pending = vec![(child, false)];
    while let Some((id, contained)) = pending.pop() {
        boxes += 1;
        if boxes > MAX_SLICED_FLEX_BOXES || !is_sliceable_flex_box(tree, id, contained) {
            return None;
        }
        if let Some((top, bottom)) = bounds {
            let start = tree.layout_origin_of(id)?.y;
            if start < top - EPSILON || start + tree.nodes[id].layout.size.height > bottom + EPSILON
            {
                return None;
            }
        }
        let contained = contained || layout_node(tree, id).contains_absolute_boxes;
        let style = style_of(tree, id);
        sized |= has_block_size(&style);
        // Text that renders gives a box lines, and a flex container makes an
        // anonymous item of text that is more than white space that collapses,
        // a no-break space included.
        for held in tree.children_through_contents(id) {
            let held_node = layout_node(tree, held);
            if !held_node.is_in_document() || held_node.is_display_none() {
                continue;
            }
            match &held_node.data {
                NodeData::Element(_) => pending.push((held, contained)),
                NodeData::Text(text)
                    if text.text_content.chars().all(is_collapsible_whitespace) => {}
                _ => return None,
            }
        }
    }
    Some(sized)
}

/// Whether `id`, a flex container or a box inside one, can be sliced with
/// the rest of the container (see [`sliceable_flex_container`]).
fn is_sliceable_flex_box(tree: &LayoutSession<'_>, id: usize, contained: bool) -> bool {
    let node = layout_node(tree, id);
    let style = style_of(tree, id);
    matches!(
        node.display(),
        DisplayValue::Block | DisplayValue::FlowRoot | DisplayValue::Flex
    ) && is_horizontal(node)
        && !node.size_containment.height
        && (matches!(
            node.layout_position(),
            Position::Relative | Position::Static
        ) || contained && is_absolute_box(node))
        && !style.float().is_floated()
        && style.overflow()
            == (Point {
                x: TaffyOverflow::Visible,
                y: TaffyOverflow::Visible,
            })
        && !node.tag_name().is_some_and(|tag| {
            ATOMIC_TAGS.contains(&tag)
                || REPLACED_BOX_TAGS.contains(&tag)
                || matches!(tag, "fieldset" | "legend" | "details" | "summary")
        })
        && node.break_before == BreakBetween::Auto
        && node.break_after == BreakBetween::Auto
        && node.break_inside == BreakInside::Auto
        && !node.has_before_or_after_content
        && !node.column_span_all
        && node.multicol.is_none()
        && node.ifc.is_none()
        // The painter draws a transform, a shadow, an outline or a filter
        // inside the clip of the piece, past the overflow that layout reports.
        && !node.paints_whole_box_effects
        // The pieces are cut from one whole box, so block edges that repeat at
        // each break have neither a place to be drawn nor space reserved.
        && !(node.clones_box_decorations && has_block_edge_decorations(&style))
}

/// A block-only wrapper tree whose leaves can be placed in the column stream.
struct WrappedBlockTree {
    node: usize,
    children: Vec<WrappedBlockChild>,
    leaf_count: usize,
    layout: Option<WrapperLayout>,
}

enum WrappedBlockChild {
    Leaf(usize),
    Wrapper(Box<WrappedBlockTree>),
}

impl WrappedBlockTree {
    fn leaf_ids(&self) -> Vec<usize> {
        let mut leaves = Vec::with_capacity(self.leaf_count);
        self.append_leaf_ids(&mut leaves);
        leaves
    }

    fn append_leaf_ids(&self, leaves: &mut Vec<usize>) {
        for child in &self.children {
            match child {
                WrappedBlockChild::Leaf(leaf) => leaves.push(*leaf),
                WrappedBlockChild::Wrapper(wrapper) => wrapper.append_leaf_ids(leaves),
            }
        }
    }

    fn append_leaf_adjustments(
        &self,
        leading: f32,
        trailing: f32,
        adjustments: &mut Vec<(usize, f32, f32)>,
    ) -> Option<()> {
        let layout = self.layout?;
        let last = self.children.len().checked_sub(1)?;
        for (index, child) in self.children.iter().enumerate() {
            let leading = if index == 0 {
                leading + layout.stream_leading
            } else {
                0.0
            };
            let trailing = if index == last {
                trailing + layout.stream_trailing
            } else {
                0.0
            };
            match child {
                WrappedBlockChild::Leaf(leaf) => adjustments.push((*leaf, leading, trailing)),
                WrappedBlockChild::Wrapper(wrapper) => {
                    wrapper.append_leaf_adjustments(leading, trailing, adjustments)?;
                }
            }
        }
        Some(())
    }
}

// Keep candidate discovery bounded so very deep wrapper trees fall back safely.
const MAX_WRAPPED_BLOCK_DEPTH: usize = 32;

fn wrapped_blocks(
    tree: &LayoutSession<'_>,
    wrapper: usize,
    allow_sliced_wrappers: bool,
) -> Option<WrappedBlockTree> {
    wrapped_block_tree(tree, wrapper, allow_sliced_wrappers, true, true, 0)
}

fn wrapped_block_tree(
    tree: &LayoutSession<'_>,
    wrapper: usize,
    allow_sliced_wrappers: bool,
    is_root: bool,
    in_multicol_context: bool,
    depth: usize,
) -> Option<WrappedBlockTree> {
    if depth >= MAX_WRAPPED_BLOCK_DEPTH {
        return None;
    }
    let node = layout_node(tree, wrapper);
    let style = style_of(tree, wrapper);
    // The blocks a flex container holds are not stacked.
    if !matches!(node.display(), DisplayValue::Block | DisplayValue::FlowRoot) {
        return None;
    }
    if in_multicol_context && node.column_span_all {
        return None;
    }
    let children_in_multicol_context =
        in_multicol_context && node.display() != DisplayValue::FlowRoot;
    let has_edges = has_block_edge_decorations(&style);
    if node.display() == DisplayValue::FlowRoot && !allow_sliced_wrappers
        || has_edges
            && (!allow_sliced_wrappers || node.clones_box_decorations || !is_root)
        || node.paints_whole_box_effects
        || matches!(
            node.break_inside,
            BreakInside::Avoid | BreakInside::AvoidColumn
        )
        || style.size().height != Dimension::auto()
        // A minimum, even a percentage one, can hold the wrapper open past
        // its blocks.
        || {
            let minimum: Dimension = style.min_size().height.into();
            minimum != Dimension::auto() && style_dimension_length(minimum) != Some(0.0)
        }
        || style.max_size().height != LengthPercentageAuto::auto()
        || style.aspect_ratio().is_some()
        || style.contain() != Contain::NONE
    {
        return None;
    }
    let mut children = Vec::new();
    let mut leaf_count = 0usize;
    for child in rendered_elements(tree, wrapper) {
        if is_absolute_box(&tree.nodes[child]) {
            if !is_root {
                return None;
            }
            continue;
        }
        if is_plain_block(tree, child) && !has_rendered_content(tree, child, false) {
            // A spanner leaves the columns, so it is not a block of the stream.
            if children_in_multicol_context && tree.nodes[child].column_span_all {
                return None;
            }
            children.push(WrappedBlockChild::Leaf(child));
            leaf_count += 1;
        } else {
            if !allow_sliced_wrappers
                || style_of(tree, child).inset() != Edges::splat(LengthPercentageAuto::auto())
                || !is_plain_wrapper_ignoring_insets(tree, child)
            {
                return None;
            }
            let nested = wrapped_block_tree(
                tree,
                child,
                true,
                false,
                children_in_multicol_context,
                depth + 1,
            )?;
            leaf_count = leaf_count.checked_add(nested.leaf_count)?;
            children.push(WrappedBlockChild::Wrapper(Box::new(nested)));
        }
    }
    (leaf_count > 0).then_some(WrappedBlockTree {
        node: wrapper,
        children,
        leaf_count,
        layout: None,
    })
}

fn apply_wrapped_layout(
    tree: &LayoutSession<'_>,
    wrapped: &mut WrappedBlockTree,
    items: &mut [Item],
    first_item: usize,
    column_width: f32,
) -> Option<WrapperLayout> {
    let mut child_layouts = Vec::with_capacity(wrapped.children.len());
    let mut item_index = first_item;
    for child in &mut wrapped.children {
        match child {
            WrappedBlockChild::Leaf(_) => {
                let item = items.get(item_index)?;
                child_layouts.push(WrapperLayout {
                    margin_top: item.margin_top,
                    margin_bottom: item.margin_bottom,
                    ..WrapperLayout::default()
                });
                item_index += 1;
            }
            WrappedBlockChild::Wrapper(nested) => {
                child_layouts.push(apply_wrapped_layout(
                    tree,
                    nested,
                    items,
                    item_index,
                    column_width,
                )?);
                item_index += nested.leaf_count;
            }
        }
    }
    let first = *child_layouts.first()?;
    let last = *child_layouts.last()?;
    let layout = wrapper_layout(tree, wrapped.node, first, last, column_width);
    let last_item = item_index.checked_sub(1)?;
    if layout.collapses_margins {
        items[first_item].margin_top = layout.margin_top;
        items[last_item].margin_bottom = layout.margin_bottom;
    } else {
        items[first_item].height += layout.stream_leading;
        items[first_item].margin_top = layout.margin_top;
        items[last_item].height += layout.stream_trailing;
        items[last_item].edges += layout.stream_trailing;
        items[last_item].margin_bottom = layout.margin_bottom;
    }
    wrapped.layout = Some(layout);
    Some(layout)
}

/// Whether the bridged CSS writing mode of `id` is horizontal.
fn inherits_horizontal(tree: &LayoutSession<'_>, id: usize) -> bool {
    is_horizontal(&tree.nodes[id])
}

fn is_horizontal(node: &crate::Node) -> bool {
    !node.used_writing_mode.is_vertical()
}

/// The block-start and block-end margins of a child, when they are
/// non-negative lengths and its inline margins are zero.
fn block_margins(style: &LayoutStyleView<'_>) -> Option<(f32, f32)> {
    let zero = LengthPercentageAuto::length(0.0);
    let margin = style.margin();
    if margin.left != zero || margin.right != zero {
        return None;
    }
    let length = |value: LengthPercentageAuto| {
        style_dimension_length(value.into()).filter(|px| px.is_finite() && *px >= 0.0)
    };
    Some((length(margin.top)?, length(margin.bottom)?))
}

pub(super) fn has_block_edge_decorations(style: &LayoutStyleView<'_>) -> bool {
    let zero = LengthPercentage::length(0.0);
    let (padding, border) = (style.padding(), style.border());
    padding.top != zero || padding.bottom != zero || border.top != zero || border.bottom != zero
}

#[derive(Clone, Copy, Default)]
struct WrapperLayout {
    leading: f32,
    trailing: f32,
    stream_leading: f32,
    stream_trailing: f32,
    margin_top: f32,
    margin_bottom: f32,
    collapses_margins: bool,
}

/// Compute a wrapper's internal edges and margins from its first and last
/// child boundaries.
fn wrapper_layout(
    tree: &LayoutSession<'_>,
    wrapper: usize,
    first: WrapperLayout,
    last: WrapperLayout,
    column_width: f32,
) -> WrapperLayout {
    let node = layout_node(tree, wrapper);
    let style = style_of(tree, wrapper);
    let (padding, border) = (style.padding(), style.border());
    let (margin_top, margin_bottom) = block_margins(&style).unwrap_or_default();
    let top_inset = super::multicol_resolve_inset(tree, padding.top, column_width)
        + super::multicol_resolve_inset(tree, border.top, column_width);
    let bottom_inset = super::multicol_resolve_inset(tree, padding.bottom, column_width)
        + super::multicol_resolve_inset(tree, border.bottom, column_width);
    let collapses_margins =
        node.display() == DisplayValue::Block && !has_block_edge_decorations(&style);
    if collapses_margins {
        WrapperLayout {
            leading: first.leading,
            trailing: last.trailing,
            stream_leading: 0.0,
            stream_trailing: 0.0,
            margin_top: margin_top.max(first.margin_top),
            margin_bottom: margin_bottom.max(last.margin_bottom),
            collapses_margins: true,
        }
    } else {
        let stream_leading = first.margin_top + top_inset;
        let stream_trailing = last.margin_bottom + bottom_inset;
        WrapperLayout {
            leading: stream_leading + first.leading,
            trailing: last.trailing + stream_trailing,
            stream_leading,
            stream_trailing,
            margin_top,
            margin_bottom,
            collapses_margins: false,
        }
    }
}

/// Whether `id` has content of its own, other than the absolutely
/// positioned children that `skip_absolute` leaves out.
fn has_rendered_content(tree: &LayoutSession<'_>, id: usize, skip_absolute: bool) -> bool {
    tree.children_through_contents(id).into_iter().any(|child| {
        let node = layout_node(tree, child);
        node.is_in_document()
            && !node.is_display_none()
            && !(skip_absolute && is_absolute_box(node))
            && match &node.data {
                NodeData::Text(text) => !text.text_content.trim().is_empty(),
                _ => true,
            }
    })
}

/// Whether `node` is an absolutely positioned element. A fixed box has the
/// viewport, not its parent, as its containing block.
fn is_absolute_box(node: &crate::Node) -> bool {
    matches!(node.data, NodeData::Element(_))
        && node.layout_position() == Position::Absolute
        && !node.fixed_position
}

/// The absolutely positioned children of `child` that are rendered.
fn absolute_children(tree: &LayoutSession<'_>, child: usize) -> Vec<usize> {
    tree.children_through_contents(child)
        .into_iter()
        .filter(|&id| {
            let node = layout_node(tree, id);
            node.is_in_document() && !node.is_display_none() && is_absolute_box(node)
        })
        .collect()
}

/// One column's share of an absolutely positioned box, which paints the
/// whole box moved into the column and clipped to it.
struct AbsolutePiece {
    node: usize,
    column: usize,
    /// The piece of the containing block it is placed in.
    parent_piece: usize,
    rect: FragmentRect,
    clip: FragmentRect,
}

/// The pieces of the absolutely positioned boxes of each child, in the
/// coordinates of the child's pieces. A box breaks at the column ends of
/// the flow its containing block is part of (CSS Position 3, 3.3.4), and a
/// box that reaches past the last column continues in overflow columns,
/// unless `last_column` names a last column with no block-end limit, which
/// holds the rest of the box. The last piece of the containing block holds
/// the box's pieces in the columns after it. `None` when a box starts above
/// the columns or its containing block, or when its pieces would not fit in
/// `budget` fragments.
#[allow(clippy::too_many_arguments)]
fn absolute_pieces(
    tree: &LayoutSession<'_>,
    children: &[usize],
    sliced_flex: &[bool],
    placements: &[Vec<Piece>],
    context: FragmentationContext,
    column_height: f32,
    last_column: Option<usize>,
    budget: usize,
) -> Option<Vec<Vec<AbsolutePiece>>> {
    let column_of = |offset: f32| {
        let column = (offset / column_height).floor() as usize;
        last_column.map_or(column, |last| column.min(last))
    };
    let mut count = 0usize;
    let mut all = Vec::with_capacity(children.len());
    for ((&child, &flex), pieces) in children.iter().zip(sliced_flex).zip(placements) {
        let mut own = Vec::new();
        let first = pieces.first()?;
        let last = pieces.last()?;
        // A flex container is sliced whole, with the absolutely positioned
        // boxes it holds.
        if flex {
            all.push(own);
            continue;
        }
        // The containing block's block offset in the columns laid end to end.
        let flow_top = first.column as f32 * column_height + first.top;
        for node in absolute_children(tree, child) {
            let layout = tree.nodes[node].layout;
            let start = flow_top + layout.location.y;
            // Descendants that overflow the box are cut with it. The overflow
            // rect is measured from the padding box, so shift it by the
            // border to compare it with the border box.
            let extent = layout
                .size
                .height
                .max(layout.border.top + layout.scrollable_overflow_rect.bottom);
            // Descendants that overflow above the box start its pieces in an
            // earlier column.
            let painted_start =
                start + (layout.border.top + layout.scrollable_overflow_rect.top).min(0.0);
            if !painted_start.is_finite() || !extent.is_finite() || painted_start < 0.0 {
                return None;
            }
            let end = start + extent;
            let first_column = column_of(painted_start);
            // The column holding the box's block end, which a box that ends
            // exactly at a column end does not reach past.
            let end_column = column_of((end - EPSILON).max(painted_start)).max(first_column);
            count = count.checked_add(end_column - first_column + 1)?;
            if count > budget {
                return None;
            }
            for column in first_column..=end_column {
                let parent_piece = match pieces.iter().position(|piece| piece.column == column) {
                    Some(index) => index,
                    // The last piece of the containing block holds the
                    // box's pieces in the columns after its own.
                    None if column > last.column => pieces.len() - 1,
                    None => return None,
                };
                let parent = &pieces[parent_piece];
                let dx = context.column_offset_x(column) - context.column_offset_x(parent.column);
                own.push(AbsolutePiece {
                    node,
                    column,
                    parent_piece,
                    rect: FragmentRect {
                        x: dx + layout.location.x,
                        y: start - column as f32 * column_height - parent.top,
                        width: layout.size.width,
                        height: layout.size.height,
                    },
                    clip: FragmentRect {
                        x: dx,
                        y: -parent.top,
                        width: (layout.location.x
                            + layout
                                .size
                                .width
                                .max(layout.border.left + layout.scrollable_overflow_rect.right))
                        .max(context.column_width),
                        height: if Some(column) == last_column {
                            (end - column as f32 * column_height).max(column_height)
                        } else {
                            column_height
                        },
                    },
                });
            }
        }
        all.push(own);
    }
    Some(all)
}

/// One column's share of a child box.
#[derive(Clone, Debug, PartialEq)]
struct Piece {
    column: usize,
    /// Block offset of the piece from the top of its column.
    top: f32,
    height: f32,
}

/// A child box in the stream: its block size, its block margins, whether
/// it may break, and whether cutting it at a column end is refused.
#[derive(Clone, Copy)]
struct Item {
    height: f32,
    margin_top: f32,
    margin_bottom: f32,
    splittable: bool,
    whole: bool,
    /// The block-start and block-end border and padding of a box whose
    /// pieces slice them: it is cut where it is taller than a column, but
    /// not inside these insets.
    slice: Option<(f32, f32)>,
    /// The block-end margin, border and padding that follow the content of
    /// the box and are counted in `height`: a break between the content and
    /// them leaves them alone in the next column.
    edges: f32,
}

pub(super) const EPSILON: f32 = 0.001;

/// Why a placement stopped before the end of the stream.
#[derive(Debug, PartialEq)]
enum Stop {
    /// More pieces or columns were needed than allowed.
    Exhausted,
    /// A box that must stay whole reached a column end.
    Cut,
}

/// The pieces of every child, and where the stream ends.
struct Placement {
    pieces: Vec<Vec<Piece>>,
    /// Block offset in the last column after the last child's block-end
    /// margin, which stays inside the multicol container.
    end: f32,
}

/// Fill columns of `column_height` with `items` in order. A splittable box
/// continues at the top of the next column; a box that avoids breaks moves
/// to the next column when it does not fit below earlier content, overflows
/// a column it starts at the top of, and breaks anyway when it is taller than
/// a whole column (CSS Fragmentation 3, 4.4). A box that avoids breaks and
/// must stay whole overflows instead.
/// Adjoining sibling margins collapse, and a margin at a break is truncated
/// (CSS Fragmentation 3, 5.2). Stops with [`Stop::Exhausted`] once more
/// than `max_pieces` pieces or skipped columns would be needed, so a tall box
/// in short columns cannot allocate or loop past the fragment budget, and
/// with [`Stop::Cut`] as soon as a box that must stay whole would be cut.
fn place(items: &[Item], column_height: f32, max_pieces: usize) -> Result<Placement, Stop> {
    place_within(items, column_height, max_pieces, usize::MAX, true)
}

/// [`place`], also stopping once the stream needs more than `max_columns`
/// columns. Boxes that must stay whole are cut like any other unless
/// `keep_whole` is set.
fn place_within(
    items: &[Item],
    column_height: f32,
    max_pieces: usize,
    max_columns: usize,
    keep_whole: bool,
) -> Result<Placement, Stop> {
    place_columns(
        items,
        &|_| column_height,
        max_pieces,
        max_columns,
        keep_whole,
    )
}

/// [`place_within`] with a block size of its own for each column.
fn place_columns(
    items: &[Item],
    height_of: &dyn Fn(usize) -> f32,
    max_pieces: usize,
    max_columns: usize,
    keep_whole: bool,
) -> Result<Placement, Stop> {
    let mut column = 0usize;
    let mut cursor = 0.0_f32;
    let mut pending_margin = 0.0_f32;
    let mut piece_count = 0usize;
    let mut placements = Vec::with_capacity(items.len());
    for item in items {
        // A box with no block size, borders or padding is self-collapsing:
        // its own margins join the margins around it (CSS 2, 8.3.1).
        if item.height <= 0.0 {
            piece_count += 1;
            if piece_count > max_pieces {
                return Err(Stop::Exhausted);
            }
            placements.push(vec![Piece {
                column,
                top: (cursor + pending_margin.max(item.margin_top)).min(height_of(column)),
                height: 0.0,
            }]);
            pending_margin = pending_margin.max(item.margin_top).max(item.margin_bottom);
            continue;
        }
        let mut pieces = Vec::new();
        let mut gap = pending_margin.max(item.margin_top);
        // A box that must stay whole moves to the next column like a box
        // that avoids breaks, rather than being cut, when it fits there: in
        // the rest of this column, or in a whole new one if content precedes
        // it here or the new one is taller.
        let taller_next = cursor <= 0.0 && height_of(column + 1) > height_of(column) + EPSILON;
        let moves_whole = item.whole
            && (cursor > 0.0 || taller_next)
            && item.height <= height_of(column + 1) + EPSILON;
        let splittable = item.splittable
            && !(item.whole
                && (cursor + gap + item.height <= height_of(column) + EPSILON || moves_whole));
        // A box that slices its decorations starts where its block-start
        // border and padding fit with some content.
        let leading = item.slice.map_or(0.0, |(leading, _)| leading);
        let fits_after_gap = if splittable {
            cursor + gap + leading < height_of(column) - EPSILON
        } else {
            cursor + gap + item.height <= height_of(column) + EPSILON
        };
        // A gap that fills the column leaves no room for a box that avoids
        // breaks either, so it starts the next column with the gap truncated.
        let gap_fills_column = !splittable && gap >= height_of(column) - EPSILON;
        if (cursor > 0.0 || gap_fills_column || moves_whole)
            && !fits_after_gap
            && (item.height > 0.0 || !splittable)
        {
            column += 1;
            if column >= max_columns {
                return Err(Stop::Exhausted);
            }
            cursor = 0.0;
            gap = 0.0;
        }
        cursor += gap;
        pending_margin = item.margin_bottom;
        // A box that avoids breaks but is taller than a whole column breaks
        // anyway (CSS Fragmentation 3, 4.4), unless painting cannot slice it.
        let breaks_anyway = !splittable
            && (!item.whole || item.slice.is_some())
            && item.height > height_of(column) + EPSILON;
        if splittable || breaks_anyway {
            let mut left = item.height;
            loop {
                let room = height_of(column) - cursor;
                // A break between a box and the block-end border and padding
                // that follow it leaves them alone at the top of the next
                // column, a break that browsers avoid for one before the box
                // (CSS Break 3, 4.1): the box moves whole when something
                // precedes it in this column and the next one holds it.
                if cursor > 0.0
                    && item.edges > 0.0
                    && left > room + EPSILON
                    && left - room <= item.edges + EPSILON
                    && item.height <= height_of(column + 1) + EPSILON
                {
                    column += 1;
                    if column >= max_columns {
                        return Err(Stop::Exhausted);
                    }
                    cursor = 0.0;
                    continue;
                }
                // A break is not inside the block-end border and padding: the
                // box ends in the next column with them, whole. At the top of
                // a column that is taller than what is left, the next column
                // offers no more room, so the border and padding are cut like
                // any other box.
                let room = match item.slice {
                    Some((_, trailing))
                        if left > room + EPSILON
                            && left - room < trailing - EPSILON
                            && (cursor > 0.0 || left - trailing > EPSILON) =>
                    {
                        (left - trailing).min(room)
                    }
                    _ => room,
                };
                // A gap that fills the column leaves no room for the box,
                // which then starts in the next column.
                if room < -EPSILON || (room <= EPSILON && left > EPSILON) {
                    // Columns too short for any of the box at their top
                    // would cut it at every one of them.
                    if keep_whole && item.whole && item.slice.is_none() && cursor <= 0.0 {
                        return Err(Stop::Cut);
                    }
                    piece_count += 1;
                    if piece_count > max_pieces {
                        return Err(Stop::Exhausted);
                    }
                    column += 1;
                    if column >= max_columns {
                        return Err(Stop::Exhausted);
                    }
                    cursor = 0.0;
                    continue;
                }
                piece_count += 1;
                if piece_count > max_pieces {
                    return Err(Stop::Exhausted);
                }
                if left <= room + EPSILON {
                    pieces.push(Piece {
                        column,
                        top: cursor,
                        height: left,
                    });
                    cursor += left;
                    break;
                }
                if keep_whole && item.whole && item.slice.is_none() {
                    return Err(Stop::Cut);
                }
                pieces.push(Piece {
                    column,
                    top: cursor,
                    height: room,
                });
                left -= room;
                column += 1;
                if column >= max_columns {
                    return Err(Stop::Exhausted);
                }
                cursor = 0.0;
            }
        } else {
            piece_count += 1;
            if piece_count > max_pieces {
                return Err(Stop::Exhausted);
            }
            pieces.push(Piece {
                column,
                top: cursor,
                height: item.height,
            });
            cursor += item.height;
        }
        placements.push(pieces);
    }
    Ok(Placement {
        pieces: placements,
        end: cursor + pending_margin,
    })
}

impl Placement {
    fn columns_used(&self) -> usize {
        self.pieces
            .iter()
            .flatten()
            .map(|piece| piece.column + 1)
            .max()
            .unwrap_or(0)
    }

    /// The tallest column from column `first` on, counting the trailing
    /// margin of the stream.
    fn extent_after(&self, first: usize) -> f32 {
        self.pieces
            .iter()
            .flatten()
            .filter(|piece| piece.column >= first)
            .map(|piece| piece.top + piece.height)
            .fold(self.end, f32::max)
    }
}

/// The block size of the stream in one column of unlimited height, with
/// adjoining margins collapsed.
fn stacked_height(items: &[Item], max_pieces: usize) -> Option<f32> {
    place(items, f32::INFINITY, max_pieces)
        .ok()
        .map(|placement| placement.end)
}

/// The smallest column height that fits `items` in `column_count` columns.
/// Content that may break anywhere shares the columns equally; boxes that
/// avoid breaks can need taller columns (CSS Multi-column 1, 7.1). The
/// search ignores the fragment budget, which the final placement enforces,
/// so running out of fragments is not mistaken for columns that are too
/// short. A placement needs at least one piece per column, so columns past
/// `max_pieces` already exceed the budget and bound each probe.
fn balanced_height(items: &[Item], column_count: usize, max_pieces: usize) -> f32 {
    let max_columns = column_count.min(max_pieces.saturating_add(1));
    // One unbounded column takes a piece per box, which the unlimited
    // budget always allows.
    let high = stacked_height(items, usize::MAX).unwrap_or(0.0);
    balanced_height_after(items, 0, &|_| 0.0, column_count, max_columns, high, false)
}

/// [`balanced_height`] for the `column_count` columns that follow the first
/// `fixed` columns, whose heights `fixed_height` gives, with the content
/// placed in at most `max_columns` columns after those. Boxes that must stay
/// whole are not cut when `keep_whole` is set. `high` bounds the search
/// from above and is the answer when nothing shorter fits.
fn balanced_height_after(
    items: &[Item],
    fixed: usize,
    fixed_height: &dyn Fn(usize) -> f32,
    column_count: usize,
    max_columns: usize,
    high: f32,
    keep_whole: bool,
) -> f32 {
    let limit = fixed.saturating_add(max_columns);
    let probe = |height: f32| {
        let height_of = |column: usize| {
            if column < fixed {
                fixed_height(column)
            } else {
                height
            }
        };
        place_columns(items, &height_of, usize::MAX, limit, keep_whole).ok()
    };
    // Every column holds its content, and the last one the trailing margin.
    let fits = |height: f32| {
        probe(height).is_some_and(|placement| {
            placement.columns_used() <= limit && placement.extent_after(fixed) <= height + EPSILON
        })
    };
    // Margins collapse and are truncated at breaks, so only the boxes
    // themselves bound the height from below: the content left after the
    // fixed columns, shared equally, and the tallest box that avoids breaks
    // and fits within `high` (a taller one breaks anyway).
    let reference = probe(high);
    let before: f32 = reference.as_ref().map_or(0.0, |placement| {
        placement
            .pieces
            .iter()
            .flatten()
            .filter(|piece| piece.column < fixed)
            .map(|piece| piece.height)
            .sum()
    });
    let content: f32 = items.iter().map(|item| item.height).sum();
    // Boxes the fixed columns already hold do not bound the rest.
    let left = |index: usize| {
        reference.as_ref().is_none_or(|placement| {
            placement.pieces[index]
                .iter()
                .any(|piece| piece.column >= fixed)
        })
    };
    let tallest = items
        .iter()
        .enumerate()
        .filter(|&(index, item)| !item.splittable && item.height <= high + EPSILON && left(index))
        .map(|(_, item)| item.height)
        .fold(0.0_f32, f32::max);
    let mut low = ((content - before) / column_count as f32).max(tallest);
    if low > 0.0 && low <= high && fits(low) {
        return low;
    }
    let mut high = high.max(low);
    // Each probe places the children once; the bound keeps the search linear
    // in the number of children.
    for _ in 0..24 {
        let middle = (low + high) / 2.0;
        if fits(middle) {
            high = middle;
        } else {
            low = middle;
        }
    }
    // The search stops near the answer; the content extent of the columns
    // it found is the exact height when that still fits.
    let extent = probe(high).map_or(0.0, |placement| placement.extent_after(fixed));
    if extent > 0.0 && fits(extent) {
        extent
    } else {
        high
    }
}

/// The pieces that `balance-all` probes may place while balancing the rows
/// of one nested container.
const BALANCE_ALL_WORK: usize = 1 << 22;

/// Balance the columns of each of the first `rows` rows of `column_count`
/// columns for `column-fill: balance-all`: each row gets the shortest column
/// height that still holds the content its columns hold at the heights in
/// `heights`, one per row (CSS Multi-column 1, 8.1). Columns past the last
/// row take that row's height. Rows are balanced in order, so the content
/// left for later rows does not change. Rows after the probes have placed
/// `budget` pieces keep their heights.
fn balance_rows(
    items: &[Item],
    column_count: usize,
    rows: usize,
    heights: &mut [f32],
    max_pieces: usize,
    budget: usize,
) {
    let last = heights.len().saturating_sub(1);
    let row_of = |column: usize| (column / column_count).min(last);
    // Pieces placed so far by the probes below.
    let work = std::cell::Cell::new(0usize);
    // The block size of the pieces in the columns before `end`.
    let held = |heights: &[f32], end: usize| {
        place_columns(
            items,
            &|column| heights[row_of(column)],
            max_pieces,
            usize::MAX,
            true,
        )
        .ok()
        .map(|placement| {
            let pieces: usize = placement.pieces.iter().map(Vec::len).sum();
            work.set(work.get().saturating_add(pieces.max(1)));
            let height: f32 = placement
                .pieces
                .iter()
                .flatten()
                .filter(|piece| piece.column < end)
                .map(|piece| piece.height)
                .sum();
            (placement, height)
        })
    };
    // The heights each probe tries: the committed heights, with the row being
    // balanced at the height under test.
    let mut trial = heights.to_vec();
    for row in 0..rows.min(last) {
        // Every probe places the whole stream, so a container with many rows
        // stops balancing once the probes have placed `budget` pieces; the
        // remaining rows keep their full height, as with `balance`.
        if work.get() > budget {
            break;
        }
        let end = column_count * (row + 1);
        // A stream that does not fit leaves the row as it is; the caller's
        // own placement then rejects it.
        let target = held(heights, end).map_or(f32::INFINITY, |(_, height)| height);
        let full = heights[row];
        let fits = |trial: &mut Vec<f32>, height: f32| {
            trial[row] = height;
            held(trial, end).is_some_and(|(_, height)| height >= target - EPSILON)
        };
        // The row's content shared equally is the answer when nothing in it
        // avoids breaks taller than that share.
        let before = held(heights, end - column_count).map_or(0.0, |(_, height)| height);
        let share = (target - before) / column_count as f32;
        if share > 0.0 && share <= full && fits(&mut trial, share) {
            heights[row] = share;
            continue;
        }
        let mut low = 0.0_f32;
        let mut high = full;
        for _ in 0..24 {
            let middle = (low + high) / 2.0;
            if fits(&mut trial, middle) {
                high = middle;
            } else {
                low = middle;
            }
        }
        // The content extent of the row's columns is the exact height when
        // it still holds the row's content.
        trial[row] = high;
        let extent = held(&trial, end).map_or(0.0, |(placement, _)| {
            placement
                .pieces
                .iter()
                .flatten()
                .filter(|piece| piece.column >= end - column_count && piece.column < end)
                .map(|piece| piece.top + piece.height)
                .fold(0.0_f32, f32::max)
        });
        heights[row] = Some(extent)
            .filter(|&extent| extent > 0.0 && extent <= full && fits(&mut trial, extent))
            .unwrap_or(high);
        trial[row] = heights[row];
    }
}

fn make_wrapped_leaf_pieces(
    wrapped: &WrappedBlockTree,
    stream_pieces: &[Vec<Piece>],
) -> Option<Vec<Vec<Piece>>> {
    let mut adjustments = Vec::with_capacity(wrapped.leaf_count);
    wrapped.append_leaf_adjustments(0.0, 0.0, &mut adjustments)?;
    if adjustments.len() != stream_pieces.len() {
        return None;
    }
    Some(
        stream_pieces
            .iter()
            .zip(adjustments)
            .map(|(pieces, (_, leading, trailing))| trim_wrapped_pieces(pieces, leading, trailing))
            .collect(),
    )
}

fn wrapper_pieces_for_tree(
    wrapped: &WrappedBlockTree,
    leaf_pieces: &[Vec<Piece>],
    column_height: f32,
) -> Option<Vec<Piece>> {
    let layout = wrapped.layout?;
    let first = leaf_pieces
        .iter()
        .flat_map(|pieces| pieces.iter())
        .find(|piece| piece.height > 0.0)?;
    let last = leaf_pieces
        .iter()
        .rev()
        .flat_map(|pieces| pieces.iter().rev())
        .find(|piece| piece.height > 0.0)?;
    wrapper_pieces(first, last, layout.leading, layout.trailing, column_height)
}

fn wrapper_pieces(
    first: &Piece,
    last: &Piece,
    leading: f32,
    trailing: f32,
    column_height: f32,
) -> Option<Vec<Piece>> {
    let start = first.column as f32 * column_height + first.top - leading;
    let end = last.column as f32 * column_height + last.top + last.height + trailing;
    if !start.is_finite() || !end.is_finite() || start < 0.0 || end < start {
        return None;
    }
    let first_column = (start / column_height).floor() as usize;
    let last_column = if end - start <= EPSILON {
        first_column
    } else {
        ((end - EPSILON).max(start) / column_height).floor() as usize
    };
    Some(
        (first_column..=last_column)
            .map(|column| {
                let top = if column == first_column {
                    start - column as f32 * column_height
                } else {
                    0.0
                };
                let bottom = if column == last_column {
                    end - column as f32 * column_height
                } else {
                    column_height
                };
                Piece {
                    column,
                    top,
                    height: (bottom - top).max(0.0),
                }
            })
            .collect(),
    )
}

fn count_wrapped_fragments(
    wrapped: &WrappedBlockTree,
    leaf_pieces: &[Vec<Piece>],
    column_height: f32,
) -> Option<usize> {
    let mut count = wrapper_pieces_for_tree(wrapped, leaf_pieces, column_height)?.len();
    let mut offset = 0usize;
    for child in &wrapped.children {
        match child {
            WrappedBlockChild::Leaf(_) => offset += 1,
            WrappedBlockChild::Wrapper(nested) => {
                let end = offset.checked_add(nested.leaf_count)?;
                count = count.checked_add(count_wrapped_fragments(
                    nested,
                    leaf_pieces.get(offset..end)?,
                    column_height,
                )?)?;
                offset = end;
            }
        }
    }
    (offset == leaf_pieces.len()).then_some(count)
}

fn push_wrapped_children(
    tree: &mut LayoutSession<'_>,
    wrapped: &WrappedBlockTree,
    leaf_pieces: &[Vec<Piece>],
    parent_pieces: &[Piece],
    parent_records: &[usize],
    context: FragmentationContext,
    column_height: f32,
) -> Option<()> {
    let mut offset = 0usize;
    for child in &wrapped.children {
        match child {
            WrappedBlockChild::Leaf(leaf) => {
                push_wrapped_block(
                    tree,
                    *leaf,
                    leaf_pieces.get(offset)?,
                    parent_pieces,
                    parent_records,
                    context,
                    column_height,
                )?;
                offset += 1;
            }
            WrappedBlockChild::Wrapper(nested) => {
                let end = offset.checked_add(nested.leaf_count)?;
                let nested_leaf_pieces = leaf_pieces.get(offset..end)?;
                let pieces = wrapper_pieces_for_tree(nested, nested_leaf_pieces, column_height)?;
                let records = push_wrapped_wrapper(
                    tree,
                    nested.node,
                    &pieces,
                    parent_pieces,
                    parent_records,
                    context,
                    column_height,
                )?;
                push_wrapped_children(
                    tree,
                    nested,
                    nested_leaf_pieces,
                    &pieces,
                    &records,
                    context,
                    column_height,
                )?;
                offset = end;
            }
        }
    }
    (offset == leaf_pieces.len()).then_some(())
}

fn push_wrapped_wrapper(
    tree: &mut LayoutSession<'_>,
    wrapper: usize,
    pieces: &[Piece],
    parents: &[Piece],
    parent_records: &[usize],
    context: FragmentationContext,
    column_height: f32,
) -> Option<Vec<usize>> {
    let first = pieces.first()?;
    let parent_first = parents.first()?;
    let mut layout = tree.nodes[wrapper].layout;
    let x = layout.location.x;
    let size = layout.size;
    layout.location = Point {
        x: x + context.column_offset_x(first.column) - context.column_offset_x(parent_first.column),
        y: first.top - parent_first.top,
    };
    tree.set_unrounded_layout(NodeId::from(wrapper), &layout);
    let mut records = Vec::with_capacity(pieces.len());
    for (index, piece) in pieces.iter().enumerate() {
        // Wrapper parents come from wrapper_pieces: one contiguous column range.
        let parent_index = piece.column.checked_sub(parent_first.column)?;
        let parent = parents.get(parent_index).filter(|parent| {
            #[cfg(test)]
            tests::record_parent_comparison(0);
            parent.column == piece.column
        })?;
        let x_offset =
            context.column_offset_x(piece.column) - context.column_offset_x(parent.column);
        let record = tree.fragment_tree.try_push(LayoutFragment {
            node_id: wrapper,
            parent: Some(parent_records[parent_index]),
            fragmentainer: context.column_index + piece.column,
            rect: FragmentRect {
                x: x + x_offset,
                y: piece.top - parent.top,
                width: size.width,
                height: piece.height,
            },
            fragmentainer_clip: (pieces.len() > 1).then_some(FragmentRect {
                x: x_offset,
                y: -parent.top,
                width: (x + size.width).max(context.column_width),
                height: column_height,
            }),
            fragment_index: index,
            fragment_count: pieces.len(),
            line_start: None,
            line_end: None,
        })?;
        records.push(record);
    }
    Some(records)
}

/// Remove the wrapper's leading and trailing insets from a leaf's stream
/// pieces so the leaf fragments keep their own border-box geometry.
fn trim_wrapped_pieces(pieces: &[Piece], mut leading: f32, mut trailing: f32) -> Vec<Piece> {
    let mut pieces = pieces.to_vec();
    for piece in &mut pieces {
        let trim = leading.min(piece.height);
        piece.top += trim;
        piece.height -= trim;
        leading -= trim;
    }
    for piece in pieces.iter_mut().rev() {
        let trim = trailing.min(piece.height);
        piece.height -= trim;
        trailing -= trim;
    }
    pieces
}

/// Record the pieces `pieces` of `leaf`, a block a wrapper holds, under the
/// wrapper's pieces `parents` and their fragment `records`.
fn push_wrapped_block(
    tree: &mut LayoutSession<'_>,
    leaf: usize,
    pieces: &[Piece],
    parents: &[Piece],
    records: &[usize],
    context: FragmentationContext,
    column_height: f32,
) -> Option<()> {
    let first = pieces.first()?;
    let wrapper_first = parents.first()?;
    let mut layout = tree.nodes[leaf].layout;
    let x = layout.location.x;
    let size = layout.size;
    layout.location = Point {
        x: x + context.column_offset_x(first.column)
            - context.column_offset_x(wrapper_first.column),
        y: first.top - wrapper_first.top,
    };
    tree.set_unrounded_layout(NodeId::from(leaf), &layout);
    let sliced = pieces.len() > 1 && slices_decorations(tree, leaf);
    let mut taken = 0.0f32;
    for (index, piece) in pieces.iter().enumerate() {
        // Wrapper parents come from wrapper_pieces: one contiguous column range.
        let parent_index = piece.column.checked_sub(wrapper_first.column)?;
        let parent = parents.get(parent_index).filter(|parent| {
            #[cfg(test)]
            tests::record_parent_comparison(1);
            parent.column == piece.column
        })?;
        tree.fragment_tree.try_push(LayoutFragment {
            node_id: leaf,
            parent: Some(records[parent_index]),
            fragmentainer: context.column_index + piece.column,
            rect: sliced_rect(
                FragmentRect {
                    x,
                    y: piece.top - parent.top,
                    width: size.width,
                    height: piece.height,
                },
                sliced.then_some((taken, size.height)),
            ),
            // A box that stays in one column is not cut.
            fragmentainer_clip: (pieces.len() > 1).then_some(FragmentRect {
                x: 0.0,
                y: -parent.top,
                width: (x + size.width).max(context.column_width),
                height: column_height,
            }),
            fragment_index: index,
            fragment_count: pieces.len(),
            line_start: None,
            line_end: None,
        })?;
        taken += piece.height;
    }
    Some(())
}

/// The block size of the box each piece of a wrapper shows, which is what the
/// pieces after it are moved up by. A piece that ends where a block moved to
/// the next column leaves room at the bottom of its column, which is no part
/// of the box, and the margin between two blocks that a column break cuts
/// still is. `stream_pieces` are those of the wrapper's blocks as the stream
/// placed them, with the wrapper's padding and border that fall in them, which
/// do fill the column they reach the end of; `items` are the blocks' stream
/// items.
fn consumed_rows(pieces: &[Piece], stream_pieces: &[Vec<Piece>], items: &[Item]) -> Vec<f32> {
    /// The blocks that start and end in a column.
    struct ColumnBlocks {
        first: usize,
        first_top: f32,
        last: usize,
        end: f32,
    }
    let mut columns = std::collections::HashMap::<usize, ColumnBlocks>::new();
    for (block, block_pieces) in stream_pieces.iter().enumerate() {
        for piece in block_pieces.iter().filter(|piece| piece.height > 0.0) {
            let end = piece.top + piece.height;
            let blocks = columns.entry(piece.column).or_insert(ColumnBlocks {
                first: block,
                first_top: piece.top,
                last: block,
                end,
            });
            if end >= blocks.end {
                blocks.last = block;
                blocks.end = end;
            }
        }
    }
    let last = pieces.len().saturating_sub(1);
    pieces
        .iter()
        .enumerate()
        .map(|(index, piece)| {
            let Some(blocks) = columns.get(&piece.column).filter(|_| index < last) else {
                return piece.height;
            };
            let shown = (blocks.end - piece.top).clamp(0.0, piece.height);
            // The next column starts with a block after this column's last
            // one, with only blocks with no block size between them. The
            // margins around them collapse into one, which the break cuts.
            let cut_margin = columns
                .get(&pieces[index + 1].column)
                .filter(|next| {
                    next.first > blocks.last
                        && next.first_top <= EPSILON
                        && items[blocks.last + 1..next.first]
                            .iter()
                            .all(|item| item.height <= 0.0)
                })
                .map_or(0.0, |next| {
                    items[blocks.last + 1..next.first].iter().fold(
                        items[blocks.last]
                            .margin_bottom
                            .max(items[next.first].margin_top),
                        |gap, item| gap.max(item.margin_top).max(item.margin_bottom),
                    )
                });
            shown + cut_margin
        })
        .collect()
}

/// The rect of a piece of a box that slices its decorations: the whole box,
/// of block size `whole.1`, moved up by the `whole.0` the earlier pieces took.
/// The clip of the column then cuts it to the piece. `rect` is the piece's own.
fn sliced_rect(rect: FragmentRect, whole: Option<(f32, f32)>) -> FragmentRect {
    match whole {
        Some((taken, height)) => FragmentRect {
            y: rect.y - taken,
            height,
            ..rect
        },
        None => rect,
    }
}

/// Whether a box cut at a column end slices its block-axis borders and
/// padding (CSS Fragmentation 3, 5.4): its pieces paint the whole box, moved
/// up by what the earlier pieces took and clipped to the column, so no border
/// shows where it breaks. Effects sized from the whole box, and decorations
/// that repeat at each break, are not sliced.
fn slices_decorations(tree: &LayoutSession<'_>, idx: usize) -> bool {
    let node = layout_node(tree, idx);
    has_block_edge_decorations(&style_of(tree, idx))
        && !node.clones_box_decorations
        && !node.paints_whole_box_effects
}

/// Where the block-axis border and padding of a box whose pieces slice them
/// are measured from.
#[derive(Clone, Copy)]
enum Insets {
    /// Resolved against this inline size, that of the column a child of the
    /// container sits in.
    Against(f32),
    /// Taken from the layout that placed the box, which resolved them against
    /// the containing block the box has inside a wrapper.
    Laid,
}

/// The stream item of `child`, a box of block size `height`. `slice` says how
/// to measure the decorations when the pieces of a cut box are placed to
/// slice them.
fn item(tree: &LayoutSession<'_>, child: usize, height: f32, slice: Option<Insets>) -> Item {
    let node = layout_node(tree, child);
    let style = style_of(tree, child);
    let (margin_top, margin_bottom) = block_margins(&style).unwrap_or_default();
    let slice = slice
        .filter(|_| slices_decorations(tree, child))
        .map(|insets| match insets {
            Insets::Against(basis) => {
                let inset = |value| super::multicol_resolve_inset(tree, value, basis);
                let (padding, border) = (style.padding(), style.border());
                (
                    inset(padding.top) + inset(border.top),
                    inset(padding.bottom) + inset(border.bottom),
                )
            }
            Insets::Laid => {
                let layout = &tree.nodes[child].layout;
                (
                    layout.padding.top + layout.border.top,
                    layout.padding.bottom + layout.border.bottom,
                )
            }
        });
    Item {
        height,
        margin_top,
        margin_bottom,
        splittable: !matches!(
            node.break_inside,
            BreakInside::Avoid | BreakInside::AvoidColumn
        ),
        // A box with block-axis borders or padding, or with effects sized
        // from the whole box, sits whole in the next column when it fits
        // there. One taller than a column is cut only where its pieces slice
        // the decorations, as a piece that does not would repeat them at each
        // break.
        whole: node.paints_whole_box_effects || has_block_edge_decorations(&style),
        slice,
        edges: slice.map_or(0.0, |(_, trailing)| trailing),
    }
}

/// One piece of a child in a row of a nested container's columns.
#[derive(Debug, PartialEq)]
pub(super) struct RowPiece {
    /// Index of the child in the stream.
    pub(super) entry: usize,
    pub(super) row: usize,
    /// Column within the row; the last row may hold overflow columns.
    pub(super) column: usize,
    /// Block offset of the piece from the row's top.
    pub(super) y: f32,
    pub(super) height: f32,
    /// Index of the piece among the child's pieces, and their count.
    pub(super) index: usize,
    pub(super) count: usize,
}

/// The pieces of a nested container's children in rows of its columns,
/// and the used block size of each row.
#[derive(Debug, PartialEq)]
pub(super) struct Rows {
    pub(super) pieces: Vec<RowPiece>,
    pub(super) heights: Vec<f32>,
}

/// Fill the children (from [`supported`]) of a nested column container,
/// whose own columns are cut again at the end of each outer column, into
/// rows of `column_count` columns: the first row holds `first` of block
/// size and every later row `later` (CSS Fragmentation 3, 2). A container
/// with a definite content height `content` ends with the row that reaches
/// it, which takes the content that is left in overflow columns (CSS
/// Multi-column 1, 8.2); an auto-height one ends with its content, and a
/// balanced one balances its last row. `None` when one row holds everything,
/// when a box that must stay whole would be cut, or past `max_pieces`.
#[allow(clippy::too_many_arguments)]
pub(super) fn fill_rows(
    tree: &LayoutSession<'_>,
    children: &[usize],
    column_count: usize,
    column_fill: ColumnFillValue,
    content: Option<f32>,
    first: f32,
    later: f32,
    max_pieces: usize,
) -> Option<Rows> {
    let items: Vec<Item> = children
        .iter()
        .map(|&child| item(tree, child, tree.nodes[child].layout.size.height, None))
        .collect();
    fill_item_rows(
        &items,
        column_count,
        column_fill,
        content,
        first,
        later,
        max_pieces,
    )
}

/// [`fill_rows`] for the stream `items`.
fn fill_item_rows(
    items: &[Item],
    column_count: usize,
    column_fill: ColumnFillValue,
    content: Option<f32>,
    first: f32,
    later: f32,
    max_pieces: usize,
) -> Option<Rows> {
    if column_count == 0 || !(first > EPSILON && later > EPSILON) || !later.is_finite() {
        return None;
    }
    let row_of = |column: usize, last: usize| (column / column_count).min(last);
    let (placement, heights) = if let Some(content) = content {
        let mut heights = Vec::new();
        let mut left = content;
        while left > EPSILON {
            let height = left.min(if heights.is_empty() { first } else { later });
            heights.push(height);
            left -= height;
            if heights.len() > max_pieces {
                return None;
            }
        }
        if heights.len() < 2 {
            return None;
        }
        let last = heights.len() - 1;
        // `balance-all` balances the columns of every row; a row keeps its
        // share of the container's height.
        let mut columns = heights.clone();
        if column_fill == ColumnFillValue::BalanceAll {
            balance_rows(
                items,
                column_count,
                last,
                &mut columns,
                max_pieces,
                BALANCE_ALL_WORK,
            );
        }
        let height_of = |column: usize| columns[row_of(column, last)];
        let placement = place_columns(items, &height_of, max_pieces, usize::MAX, true).ok()?;
        // Content that fits the last row's columns is balanced in them
        // unless they fill sequentially; the row keeps its height.
        let fixed = column_count * last;
        let placement = if column_fill != ColumnFillValue::Auto
            && placement.columns_used() <= fixed + column_count
        {
            let height = balanced_height_after(
                items,
                fixed,
                &height_of,
                column_count,
                column_count,
                heights[last],
                true,
            );
            let balanced = |column: usize| {
                if column < fixed {
                    height_of(column)
                } else {
                    height
                }
            };
            place_columns(items, &balanced, max_pieces, usize::MAX, true).ok()?
        } else {
            placement
        };
        (placement, heights)
    } else {
        let height_of = |column: usize| if column < column_count { first } else { later };
        let placement = place_columns(items, &height_of, max_pieces, usize::MAX, true).ok()?;
        let last = (placement.columns_used().max(1) - 1) / column_count;
        if last == 0 {
            return None;
        }
        // The last row is as tall as its content, or balanced: the shortest
        // height that keeps the content in that row's columns.
        let mut columns: Vec<f32> = (0..=last)
            .map(|row| if row == 0 { first } else { later })
            .collect();
        if column_fill == ColumnFillValue::BalanceAll {
            balance_rows(
                items,
                column_count,
                last,
                &mut columns,
                max_pieces,
                BALANCE_ALL_WORK,
            );
        }
        let height_of = |column: usize| columns[row_of(column, last)];
        let fixed = column_count * last;
        let placement = if column_fill == ColumnFillValue::Auto {
            placement
        } else {
            let height = balanced_height_after(
                items,
                fixed,
                &height_of,
                column_count,
                column_count,
                later,
                true,
            );
            let balanced = |column: usize| {
                if column < fixed {
                    height_of(column)
                } else {
                    height
                }
            };
            place_columns(items, &balanced, max_pieces, fixed + column_count, true).ok()?
        };
        let mut heights = columns[..last].to_vec();
        heights.push(placement.extent_after(fixed));
        (placement, heights)
    };
    let last = heights.len() - 1;
    let pieces = placement
        .pieces
        .iter()
        .enumerate()
        .flat_map(|(entry, pieces)| {
            let count = pieces.len();
            pieces.iter().enumerate().map(move |(index, piece)| {
                let row = row_of(piece.column, last);
                RowPiece {
                    entry,
                    row,
                    column: piece.column - row * column_count,
                    y: piece.top,
                    height: piece.height,
                    index,
                    count,
                }
            })
        })
        .collect();
    Some(Rows { pieces, heights })
}

/// Lay out `children` (from [`supported`]) as one stream cut into columns.
/// `minimum` is the authored minimum border-box height of the container,
/// which also bounds its columns from below. `definite` says layout fixed
/// the container's height; otherwise the columns balance, bounded by the
/// content-box `maximum` when there is one. Returns the used
/// border-box height of the container.
#[allow(clippy::too_many_arguments)]
pub(super) fn layout(
    tree: &mut LayoutSession<'_>,
    parent: usize,
    children: &[usize],
    context: FragmentationContext,
    border_box: Size<f32>,
    origin: Point<f32>,
    block_end_inset: f32,
    minimum: f32,
    definite: bool,
    maximum: Option<f32>,
    outer_height: Option<f32>,
    allow_wrapped_blocks: bool,
) -> Option<f32> {
    // A definite container uses the content height layout gave it, which
    // already reflects min/max constraints and stretching by its parent.
    let definite_height =
        definite.then(|| (border_box.height - origin.y - block_end_inset).max(0.0));
    let mut sizes = Vec::with_capacity(children.len());
    // What each box lays out past itself, for the column width it was laid out
    // at; the overflow its node kept is that of the pass before.
    let mut overflows = Vec::with_capacity(children.len());
    for &child in children {
        tree.nodes[child].cache.clear();
        let output = tree.compute_child_layout(
            NodeId::from(child),
            super::column_child_layout_input(
                tree,
                child,
                context.column_width,
                definite_height,
                AvailableSpace::MaxContent,
            ),
        );
        sizes.push(output.size);
        overflows.push(output.scrollable_overflow_rect);
    }
    // Whether a flex container is sliced was settled before its boxes were
    // laid out, with no geometry to compare with its block end, or with that
    // of an earlier layout. Now that they have it, a container whose boxes
    // reach past it is left to the other strategies.
    let sliced_flex: Vec<bool> = children
        .iter()
        .zip(&sizes)
        .map(|(&child, size)| {
            layout_node(tree, child).display() == DisplayValue::Flex
                && sliceable_flex_container(tree, child, Some(size.height)).is_some()
        })
        .collect();
    if children
        .iter()
        .zip(&sliced_flex)
        .any(|(&child, &flex)| layout_node(tree, child).display() == DisplayValue::Flex && !flex)
    {
        return None;
    }
    // A wrapper that holds blocks puts them in the stream in its place, so
    // each child has a range of stream items.
    let mut wrapped: Vec<Option<WrappedBlockTree>> = children
        .iter()
        .map(|&child| wrapped_blocks(tree, child, allow_wrapped_blocks))
        .collect();
    let mut items = Vec::new();
    let mut spans = Vec::with_capacity(children.len());
    for ((&child, size), wrapped_tree) in children.iter().zip(&sizes).zip(&mut wrapped) {
        let first = items.len();
        match wrapped_tree {
            None => {
                items.push(item(
                    tree,
                    child,
                    size.height,
                    Some(Insets::Against(context.column_width)),
                ));
            }
            Some(wrapped_tree) => {
                let leaves = wrapped_tree.leaf_ids();
                items.extend(leaves.iter().map(|&leaf| {
                    item(
                        tree,
                        leaf,
                        tree.nodes[leaf].layout.size.height,
                        Some(Insets::Laid),
                    )
                }));
                apply_wrapped_layout(tree, wrapped_tree, &mut items, first, context.column_width)?;
            }
        }
        spans.push(first..items.len());
    }
    // One fragment for the container, the rest for the child pieces.
    let max_pieces = tree
        .fragment_tree
        .limit
        .saturating_sub(tree.fragment_tree.fragments.len())
        .saturating_sub(1);
    let column_height = match definite_height {
        Some(height) => height,
        None if context.column_fill == ColumnFillValue::Auto => {
            // A single unbounded column cuts nothing, so only the number of
            // boxes can run out of fragments.
            let Some(height) = stacked_height(&items, max_pieces) else {
                tree.fragment_tree.limit_exceeded = true;
                return None;
            };
            height
        }
        None => balanced_height(&items, context.column_count, max_pieces),
    };
    // A maximum height bounds the columns of an auto-height container.
    let column_height = match (definite_height, maximum) {
        (None, Some(maximum)) => column_height.min(maximum),
        _ => column_height,
    };
    // Columns are as tall as the container's content box (CSS Multi-column
    // 1, 2), so a minimum height makes them taller, whether the authored
    // height is definite or not.
    let column_height = column_height.max(minimum - origin.y - block_end_inset);
    if !column_height.is_finite() || column_height <= 0.0 {
        return None;
    }
    // A box that must stay whole falls back to the other strategies whole
    // when it would be cut, which is not a layout failure; running out of
    // fragments before that is.
    let placements = match place(&items, column_height, max_pieces) {
        Ok(placements) => placements,
        Err(Stop::Cut) => return None,
        Err(Stop::Exhausted) => {
            tree.fragment_tree.limit_exceeded = true;
            return None;
        }
    };
    let mut child_pieces: Vec<Vec<Piece>> = Vec::with_capacity(children.len());
    let mut wrapped_leaf_pieces: Vec<Option<Vec<Vec<Piece>>>> = Vec::with_capacity(children.len());
    for (span, wrapped_tree) in spans.iter().zip(&wrapped) {
        let pieces = &placements.pieces[span.clone()];
        match wrapped_tree {
            None => {
                child_pieces.push(pieces.first()?.clone());
                wrapped_leaf_pieces.push(None);
            }
            Some(wrapped_tree) => {
                let leaf_pieces = make_wrapped_leaf_pieces(wrapped_tree, pieces)?;
                child_pieces.push(wrapper_pieces_for_tree(
                    wrapped_tree,
                    &leaf_pieces,
                    column_height,
                )?);
                wrapped_leaf_pieces.push(Some(leaf_pieces));
            }
        }
    }
    let wrapped_fragments =
        wrapped.iter().zip(&wrapped_leaf_pieces).try_fold(
            0usize,
            |count, (wrapped_tree, leaf_pieces)| match (wrapped_tree, leaf_pieces) {
                (Some(wrapped_tree), Some(leaf_pieces)) => count.checked_add(
                    count_wrapped_fragments(wrapped_tree, leaf_pieces, column_height)?,
                ),
                (None, None) => Some(count),
                _ => None,
            },
        )?;
    let planned = placements
        .pieces
        .iter()
        .map(Vec::len)
        .sum::<usize>()
        .checked_add(wrapped_fragments)?;
    let absolute = absolute_pieces(
        tree,
        children,
        &sliced_flex,
        &child_pieces,
        context,
        column_height,
        // The last column of a container whose height nothing limits grows
        // to hold what is left (CSS Multi-column 1, 8.1).
        (definite_height.is_none() && maximum.is_none())
            .then(|| context.column_count.saturating_sub(1)),
        max_pieces.saturating_sub(planned),
    )?;
    let used_height = if definite_height.is_some() {
        border_box.height
    } else {
        origin.y + column_height + block_end_inset
    };
    // Columns sliced here are not cut again at the end of an outer column,
    // so a nested container taller than the outer column is left to the
    // strategies that are.
    if outer_height.is_some_and(|outer| used_height > outer + EPSILON) {
        return None;
    }
    tree.fragment_tree.retire_previous_roots(parent);
    tree.nodes[parent].multicol_groups.clear();
    tree.nodes[parent].multicol_rows.clear();
    let container = tree.fragment_tree.try_push(LayoutFragment {
        node_id: parent,
        parent: None,
        fragmentainer: context.column_index,
        rect: FragmentRect {
            x: context.origin_x,
            y: context.origin_y,
            width: border_box.width,
            height: used_height,
        },
        fragmentainer_clip: None,
        fragment_index: 0,
        fragment_count: 1,
        line_start: None,
        line_end: None,
    })?;
    let mut previous_column = 0usize;
    for (order, ((&child, pieces), absolute)) in children
        .iter()
        .zip(&child_pieces)
        .zip(&absolute)
        .enumerate()
    {
        let size = sizes[order];
        let first = pieces.first()?;
        // A break between two children is a resumable point of the flow.
        if first.column > previous_column && first.top == 0.0 {
            tree.fragment_tree
                .record_break(crate::fragment::BreakToken {
                    node_id: parent,
                    child_index: order,
                    line_index: 0,
                });
        }
        previous_column = pieces.last()?.column;
        let mut layout = tree.nodes[child].layout;
        layout.order = order as u32;
        layout.size = size;
        layout.location = Point {
            x: origin.x + context.column_offset_x(first.column),
            y: origin.y + first.top,
        };
        tree.set_unrounded_layout(NodeId::from(child), &layout);
        let mut records = Vec::with_capacity(pieces.len());
        // How far each piece of a sliced box is moved up.
        let mut shifts = Vec::with_capacity(pieces.len());
        // A flex container shows its items in each piece, so its pieces are
        // the whole box moved up, like those that slice their decorations.
        let flex = sliced_flex[order];
        let sliced = pieces.len() > 1 && (flex || slices_decorations(tree, child));
        // What a flex container holds paints with it, so the clip of a piece
        // admits what reaches past the box inline, as the clips of the
        // absolutely positioned boxes of other children do.
        let (held_left, held_right) = if flex {
            (
                (layout.border.left + overflows[order].left).min(0.0),
                layout.border.left + overflows[order].right,
            )
        } else {
            (0.0, 0.0)
        };
        let consumed = match &wrapped_leaf_pieces[order] {
            Some(_) if sliced => consumed_rows(
                pieces,
                &placements.pieces[spans[order].clone()],
                &items[spans[order].clone()],
            ),
            _ => pieces.iter().map(|piece| piece.height).collect(),
        };
        let mut taken = 0.0f32;
        for (index, piece) in pieces.iter().enumerate() {
            shifts.push(if sliced { taken } else { 0.0 });
            let x = origin.x + context.column_offset_x(piece.column);
            let record = tree.fragment_tree.try_push(LayoutFragment {
                node_id: child,
                parent: Some(container),
                fragmentainer: context.column_index + piece.column,
                rect: sliced_rect(
                    FragmentRect {
                        x,
                        y: origin.y + piece.top,
                        width: size.width,
                        height: piece.height,
                    },
                    sliced.then_some((taken, size.height)),
                ),
                // A box that stays in one column is not cut. A cut one
                // still shows its absolutely positioned boxes, which have
                // clips of their own, where they reach past it inline.
                fragmentainer_clip: (pieces.len() > 1).then(|| {
                    let (left, right) = absolute
                        .iter()
                        .filter(|absolute| absolute.parent_piece == index)
                        .fold(
                            (
                                held_left,
                                size.width.max(context.column_width).max(held_right),
                            ),
                            |(left, right), absolute| {
                                (
                                    left.min(absolute.clip.x),
                                    right.max(absolute.clip.x + absolute.clip.width),
                                )
                            },
                        );
                    FragmentRect {
                        x: x + left,
                        y: origin.y,
                        width: right - left,
                        height: column_height,
                    }
                }),
                fragment_index: index,
                fragment_count: pieces.len(),
                line_start: None,
                line_end: None,
            })?;
            records.push(record);
            taken += consumed[index];
        }
        if let (Some(wrapped_tree), Some(leaf_pieces)) =
            (&wrapped[order], &wrapped_leaf_pieces[order])
        {
            push_wrapped_children(
                tree,
                wrapped_tree,
                leaf_pieces,
                pieces,
                &records,
                context,
                column_height,
            )?;
        }
        for piece in absolute {
            // The piece was placed against the column, but a piece of a
            // sliced box sits higher by what the pieces before it took.
            let shift = shifts[piece.parent_piece];
            tree.fragment_tree.try_push(LayoutFragment {
                node_id: piece.node,
                parent: Some(records[piece.parent_piece]),
                fragmentainer: context.column_index + piece.column,
                rect: FragmentRect {
                    y: piece.rect.y + shift,
                    ..piece.rect
                },
                fragmentainer_clip: Some(FragmentRect {
                    y: piece.clip.y + shift,
                    ..piece.clip
                }),
                fragment_index: 0,
                fragment_count: 1,
                line_start: None,
                line_end: None,
            })?;
        }
    }
    // The boxes are placed for good, without the block offset of their
    // insets.
    for &child in children {
        super::leave_block_offset_to_painter(tree, child);
    }
    Some(used_height)
}

#[cfg(test)]
mod tests;
