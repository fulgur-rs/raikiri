//! Column fragmentation of block children that hold no lines.
//!
//! A block box with no in-flow content of its own (for example a sized,
//! empty `div`) has no line boxes to break between, but it is still
//! fragmentable: a box taller than the remaining column continues in the
//! next column (CSS Fragmentation 3, 4.4; CSS Multi-column 1, 7.1). The other
//! strategies place such a box atomically in one column. This one lays the
//! children out as one stream and slices each box at the column boundaries.

use super::column_line_boxes;
use super::column_lines::{self, LineMoves};
use super::*;
use crate::fragment::{FragmentRect, LayoutFragment, MulticolGroup};
use crate::layout::context::LayoutSession;
use crate::layout::ifc::projection::{ATOMIC_TAGS, REPLACED_BOX_TAGS};
use raikiri_layout_geometry::{Edges, Point};
use raikiri_layout_style::values::ExpandedLengthPercentage;
use raikiri_style::property::BreakInside;
use std::collections::HashMap;

mod paged;
pub(super) use paged::{PagedPiece, paginate};

/// The rendered children of `parent` when every one of them is a plain
/// block with no content, and at least one of them has a block size (or, with
/// `out_of_flow`, holds an absolutely positioned box).
///
/// Inline margins, generated content, forced or avoided breaks between
/// boxes, absolutely positioned or floated boxes and replaced elements need
/// their own handling at a break, so containers with them keep the other
/// strategies. A scroll container that a height, a maximum height or an
/// aspect ratio bounds is no such box: it is monolithic, so the stream places
/// it whole in one column (see [`is_monolithic_scroll_box`]), and so is a
/// column container that the
/// columns around it grow to fit (see [`is_unbroken_column_container`]).
/// A `position: relative` box with insets is sliced like any other: its
/// column location carries no offset, which the painter adds. That holds only
/// for a box with nothing rendered inside it, so one that holds blocks or
/// absolutely positioned boxes keeps the other strategies when it has insets,
/// and so does a box inside a wrapper.
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
    supported_with_wrapped_blocks(tree, parent, None, out_of_flow, false)
}

/// [`supported`] for an outermost multicol container. Its stream may also
/// hold the blocks of eligible block-only wrappers in their place.
pub(super) fn supported_with_wrappers(
    tree: &LayoutSession<'_>,
    parent: usize,
) -> Option<Vec<usize>> {
    supported_with_wrapped_blocks(tree, parent, None, true, true)
}

/// The children of `parent` in `segment` (the boxes between two spanners,
/// or after the last, which form a row of columns of their own) when the
/// stream places them all as boxes of their own: sized plain blocks and
/// boxes that no break goes through, with no wrapper whose boxes would join
/// the stream in its place.
pub(super) fn supported_plain_segment(
    tree: &LayoutSession<'_>,
    parent: usize,
    segment: &[usize],
) -> Option<Vec<usize>> {
    supported_with_wrapped_blocks(tree, parent, Some(segment), false, false)
}

fn supported_with_wrapped_blocks(
    tree: &LayoutSession<'_>,
    parent: usize,
    segment: Option<&[usize]>,
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
    // A scroll container is placed whole at the height layout gives it, so
    // it has a block size to place even when only a maximum bounds it.
    let mut scroll_box = false;
    let candidates: Box<dyn Iterator<Item = usize>> = match segment {
        Some(segment) => Box::new(
            segment
                .iter()
                .copied()
                .filter(|&child| is_rendered_element(tree, child)),
        ),
        None => Box::new(rendered_elements(tree, parent)),
    };
    for child in candidates {
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
        let scrolls = is_monolithic_scroll_box(tree, child);
        let monolithic = scrolls || is_unbroken_column_container(tree, child);
        scroll_box |= scrolls;
        let plain_block = is_plain_block_ignoring_insets(tree, child);
        let plain_wrapper = allow_wrapped_blocks
            && is_plain_wrapper_ignoring_insets(tree, child)
            && has_rendered_content(tree, child, false);
        // The insets of a sticky box are no relative offset: the columns
        // place it where the flow puts it, with no offset.
        let sticky = node.layout_position() == Position::Sticky;
        if !(plain_block || plain_wrapper || monolithic)
            || has_inset
                && !(sticky && monolithic)
                && (!super::insets_are_a_relative_offset(tree, child)
                    || has_rendered_content(tree, child, false))
        {
            return None;
        }
        // What a scroll container holds stays inside it, so it is a box with
        // no content of its own as far as the stream is concerned.
        if !monolithic
            && has_rendered_content(tree, child, out_of_flow && node.contains_absolute_boxes)
        {
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
    let sized = scroll_box
        || boxes
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
    style_of(tree, child).clear() == Clear::None && is_sized_leaf_below_floats(tree, child)
}

/// [`is_sized_leaf`] for a box that may clear the floats before it, for a
/// caller that places it below them.
pub(super) fn is_sized_leaf_below_floats(tree: &LayoutSession<'_>, child: usize) -> bool {
    let style = style_of(tree, child);
    layout_node(tree, child).display() == DisplayValue::Block
        && is_plain_box_but_clearance(tree, child, false, false)
        && !has_rendered_content(tree, child, false)
        && has_block_size(&style)
        && block_margins(&style) == Some((0.0, 0.0))
}

/// Whether `child` is a plain block with nothing inside it, no block size and
/// no padding or border: a box that takes no room, so that only its margins
/// count, collapsing through it with the ones around it. Its block margins may
/// be of either sign, and a percentage of the column width, which the columns
/// resolve before the stream reads them; inline margins would narrow it, so it
/// has none.
pub(super) fn is_margin_only_block(tree: &LayoutSession<'_>, child: usize) -> bool {
    let style = style_of(tree, child);
    let margin = style.margin();
    let zero = LengthPercentageAuto::length(0.0);
    margin.left == zero
        && margin.right == zero
        && !margin.top.is_auto()
        && !margin.bottom.is_auto()
        && is_plain_block_apart_from_margins(tree, child, false, false)
        && !has_rendered_content(tree, child, false)
        && !has_block_size(&style)
        && !has_block_edge_decorations(&style)
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
    flow_children(tree, parent)
        .into_iter()
        .filter(|&child| is_rendered_element(tree, child))
}

/// Whether `child` is an element in the document that renders a box.
fn is_rendered_element(tree: &LayoutSession<'_>, child: usize) -> bool {
    let node = layout_node(tree, child);
    node.is_in_document() && !node.is_display_none() && matches!(node.data, NodeData::Element(_))
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

/// [`is_plain_block_holding_lines`] for a list item, whose marker, if any,
/// sits on its first line.
pub(super) fn is_plain_list_item_holding_lines(tree: &LayoutSession<'_>, child: usize) -> bool {
    layout_node(tree, child).display() == DisplayValue::ListItem
        && block_margins(&style_of(tree, child)).is_some()
        && is_plain_box_apart_from_display(tree, child, true)
}

fn is_plain_block_with_display(
    tree: &LayoutSession<'_>,
    child: usize,
    allow_flow_root: bool,
    allow_lines: bool,
) -> bool {
    block_margins(&style_of(tree, child)).is_some()
        && is_plain_block_apart_from_margins(tree, child, allow_flow_root, allow_lines)
}

/// Whether `child` is a scroll container with nothing that needs handling at
/// a break: a box a column end never cuts, whatever it holds. A scroll
/// container is monolithic content (CSS Fragmentation 3, `monolithic`), so it
/// moves to the next column when it does not fit below what comes before it,
/// and one taller than a column overflows the column it starts at the top of
/// (CSS Fragmentation 3, 4.4). What it holds scrolls inside it, so the stream
/// does not look at it, unless that content could reach out of it: see
/// [`holds_no_fragmenting_box`]. A `position: sticky` box is placed where the
/// flow puts it, with no offset, as the columns place every sticky box.
///
/// The box needs a height, a maximum height or an aspect ratio that bounds
/// it; [`layout`] measures the height it ends up with before it places the
/// box, which is whole even below its maximum, as Chromium shows it on
/// screen. Chromium prints a scroll container in columns cut at the column
/// end, like a box that does not scroll; a box with nothing to bound its
/// height, such as a paragraph that clips its overflow, keeps the other
/// strategies, which break it between its lines as the printed columns do.
///
/// Keep the checks that leave the box out in step with
/// [`is_plain_block_apart_from_margins`].
fn is_monolithic_scroll_box(tree: &LayoutSession<'_>, child: usize) -> bool {
    let node = layout_node(tree, child);
    let style = style_of(tree, child);
    let scrolls = |overflow| !matches!(overflow, TaffyOverflow::Visible | TaffyOverflow::Clip);
    let overflow = style.overflow();
    let bounded = style_dimension_length(style.size().height).is_some()
        || style_dimension_length(style.max_size().height.into()).is_some()
        || style.aspect_ratio().is_some();
    (scrolls(overflow.x) || scrolls(overflow.y))
        && bounded
        && block_margins(&style).is_some()
        && (style.inset() == Edges::splat(LengthPercentageAuto::auto())
            || node.layout_position() == Position::Sticky)
        && matches!(node.display(), DisplayValue::Block | DisplayValue::FlowRoot)
        && is_horizontal(node)
        && !node.size_containment.height
        && matches!(
            node.layout_position(),
            Position::Relative | Position::Static | Position::Sticky
        )
        && !style.float().is_floated()
        && style.clear() == Clear::None
        && !node.tag_name().is_some_and(|tag| {
            ATOMIC_TAGS.contains(&tag)
                || REPLACED_BOX_TAGS.contains(&tag)
                || matches!(tag, "fieldset" | "legend" | "details" | "summary")
        })
        && !sets_break_in_columns(node.break_before)
        && !sets_break_in_columns(node.break_after)
        && node.multicol.is_none()
        && !is_right_aligned_in_parent(tree, child)
        && holds_no_fragmenting_box(tree, child)
}

/// Whether `child` is a column container of plain blocks that the stream
/// places whole in one column, at the height its own columns balance to.
/// Chromium breaks such a container across the outer columns only when its
/// balanced height does not fit them; whole, it lands where Chromium puts it
/// whenever the outer columns balance to at least that height, and the
/// stream keeps the boxes after it in the columns that follow.
///
/// Only the columns of an outermost container that balance with nothing to
/// limit their height grow to fit it, so only such a container holds it
/// whole. Columns that a height or the page fills in turn, and columns nested
/// in other columns, cut it, which their own strategies do.
pub(super) fn is_unbroken_column_container(tree: &LayoutSession<'_>, child: usize) -> bool {
    let node = layout_node(tree, child);
    let style = style_of(tree, child);
    let in_flow = matches!(
        node.layout_position(),
        Position::Relative | Position::Static
    );
    node.multicol.is_some()
        && column_owner(tree, child).is_some_and(|owner| balances_without_limit(tree, owner))
        && block_margins(&style).is_some()
        && style.inset() == Edges::splat(LengthPercentageAuto::auto())
        && matches!(node.display(), DisplayValue::Block | DisplayValue::FlowRoot)
        && is_horizontal(node)
        && in_flow
        && !style.float().is_floated()
        && style.clear() == Clear::None
        && !node.size_containment.height
        && !node.tag_name().is_some_and(|tag| {
            ATOMIC_TAGS.contains(&tag)
                || REPLACED_BOX_TAGS.contains(&tag)
                || matches!(tag, "fieldset" | "legend" | "details" | "summary")
        })
        && style.overflow().x == TaffyOverflow::Visible
        && style.overflow().y == TaffyOverflow::Visible
        && !sets_break_in_columns(node.break_before)
        && !sets_break_in_columns(node.break_after)
        && !is_right_aligned_in_parent(tree, child)
        && supported(tree, child, false).is_some()
}

/// Whether one of `children` is a column container the stream keeps whole
/// (see [`is_unbroken_column_container`]).
pub(super) fn holds_unbroken_column_container(
    tree: &LayoutSession<'_>,
    children: &[usize],
) -> bool {
    children
        .iter()
        .any(|&child| is_unbroken_column_container(tree, child))
}

/// The multicol container whose stream holds `child`: its nearest ancestor
/// that is not `display: contents`, when that ancestor is a column container.
fn column_owner(tree: &LayoutSession<'_>, child: usize) -> Option<usize> {
    let mut owner = tree.nodes[child].parent?;
    while layout_node(tree, owner).display() == DisplayValue::Contents {
        owner = tree.nodes[owner].parent?;
    }
    tree.nodes[owner].multicol.is_some().then_some(owner)
}

/// Whether the columns of `owner` balance with no maximum, aspect ratio or
/// outer column to stop them, so they grow to fit a box that is never cut. A
/// definite height stops them too, which the caller that resolves it checks.
fn balances_without_limit(tree: &LayoutSession<'_>, owner: usize) -> bool {
    let style = style_of(tree, owner);
    tree.nodes[owner]
        .multicol
        .as_ref()
        .is_some_and(|columns| columns.column_fill != ColumnFillValue::Auto)
        && style.max_size().height.is_auto()
        && style.aspect_ratio().is_none()
        && std::iter::successors(tree.nodes[owner].parent, |&id| tree.nodes[id].parent)
            .all(|id| tree.nodes[id].multicol.is_none())
}

/// Whether nothing inside `child` takes part in the fragmentation of the
/// columns around it: no absolutely positioned box has its containing block
/// outside `child`, as it then breaks across the columns with that block, and
/// no box is a column container, which lays itself out as part of the columns
/// it sits in. An absolutely positioned box whose containing block is `child`
/// or a box inside it moves with `child`, whole, like the rest of what it
/// holds.
fn holds_no_fragmenting_box(tree: &LayoutSession<'_>, child: usize) -> bool {
    // Each pending box carries whether a box from `child` down to its parent
    // is the containing block of absolutely positioned boxes.
    let contains = |id: usize| layout_node(tree, id).layout_position() != Position::Static;
    let mut pending: Vec<(usize, bool)> = layout_node(tree, child)
        .children
        .iter()
        .map(|&id| (id, contains(child)))
        .collect();
    while let Some((id, contained)) = pending.pop() {
        let node = layout_node(tree, id);
        // A fixed box's containing block is the viewport, or an ancestor
        // that a transform or filter makes one, so it is left out whatever
        // holds it.
        if node.layout_position() == Position::Absolute
            && (!contained || matches!(node.position(), PositionValue::Fixed))
            || node.multicol.is_some()
        {
            return false;
        }
        let contained = contained || contains(id);
        pending.extend(node.children.iter().map(|&id| (id, contained)));
    }
    true
}

/// [`is_plain_block_with_display`] for a box whose margins the caller has
/// checked. A scroll container is left out here and taken, in some cases, by
/// [`is_monolithic_scroll_box`], which shares most of these checks.
fn is_plain_block_apart_from_margins(
    tree: &LayoutSession<'_>,
    child: usize,
    allow_flow_root: bool,
    allow_lines: bool,
) -> bool {
    let display = layout_node(tree, child).display();
    (display == DisplayValue::Block || allow_flow_root && display == DisplayValue::FlowRoot)
        && is_plain_box_apart_from_display(tree, child, allow_lines)
}

/// [`is_plain_block_apart_from_margins`] for a box of any display, which the
/// caller checks.
pub(super) fn is_plain_box_apart_from_display(
    tree: &LayoutSession<'_>,
    child: usize,
    allow_lines: bool,
) -> bool {
    style_of(tree, child).clear() == Clear::None
        && is_plain_box_but_clearance(tree, child, allow_lines, false)
}

/// [`is_plain_box_apart_from_display`] for a box that holds no lines and may
/// force a column break before or after itself, for a caller that starts a
/// column there.
pub(super) fn is_plain_box_apart_from_display_and_breaks(
    tree: &LayoutSession<'_>,
    child: usize,
) -> bool {
    style_of(tree, child).clear() == Clear::None
        && is_plain_box_but_clearance(tree, child, false, true)
}

/// [`is_plain_box_apart_from_display`] for a box that may clear floats, for
/// a caller that places it below them. With `breaks`, the box may force a
/// column break before or after itself.
fn is_plain_box_but_clearance(
    tree: &LayoutSession<'_>,
    child: usize,
    allow_lines: bool,
    breaks: bool,
) -> bool {
    let node = layout_node(tree, child);
    let style = style_of(tree, child);
    // An orthogonal flow is monolithic in the column's block axis.
    !(!is_horizontal(node)
        // So is a box with size containment (CSS Contain 2, 3.1).
        || node.size_containment.height
        || node.layout_position() != Position::Relative
            && node.layout_position() != Position::Static
        || style.float().is_floated()
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
        || [node.break_before, node.break_after]
            .into_iter()
            .any(|value| sets_break_in_columns(value) && !(breaks && value == BreakBetween::Column))
        || node.has_before_or_after_content
        || node.multicol.is_some()
        || node.ifc.is_some() && !allow_lines
        || is_right_aligned_in_parent(tree, child))
}

/// The most boxes a flex or grid container may hold and still be sliced whole, so
/// that checking a very large one stays cheap.
const MAX_SLICED_FLEX_BOXES: usize = 256;

/// Whether `child` is a flex container that the stream slices as one box:
/// it and the boxes inside it hold no lines, so there is nowhere to break
/// between lines, and nothing that breaks on its own terms. The container is
/// laid out whole, and each column shows the part of it that falls in the
/// column (CSS Flexbox 1, 12). A grid container whose items hold no lines
/// is sliced the same way, as Chromium does when it breaks its rows at a
/// column end. `Some(sized)` says whether any of them has a block size of
/// its own to slice.
///
/// `block_size` is the block size of the container in this layout, once the
/// boxes it holds are laid out. Their geometry then tells whether a box
/// reaches past the container. The check that settles which strategy takes
/// the container runs before they are laid out, with the geometry of an
/// earlier layout or none, so it passes `None` and judges the structure only.
///
/// Boxes that need handling at a break of their own keep the other
/// strategies: avoided breaks, forced breaks other than column breaks at the
/// container or between its items (see [`flex_breaks`]), forced breaks
/// between items of a container that does not grow with them, that holds
/// absolutely positioned boxes, or whose boxes a negative margin pulls
/// across the break, absolutely positioned or floated
/// boxes whose containing block is not inside the container, scroll
/// containers, replaced elements, boxes that paint past their own box (a
/// transform, shadow, outline or filter) or as a whole group, boxes whose
/// block edges repeat at each break, containers that are not block, flex or
/// grid, forced breaks inside a grid container, and containers whose content
/// overflows their block start or end.
fn sliceable_flex_container(
    tree: &LayoutSession<'_>,
    child: usize,
    block_size: Option<f32>,
) -> Option<bool> {
    let style = style_of(tree, child);
    if !is_flex_or_grid(tree, child)
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
    let breaking_items = breaks_between_items(tree, child);
    // Whether a box on the way down is the containing block of the absolutely
    // positioned boxes below it, so that they are cut with the container, and
    // whether a forced column break before or after it would fall between
    // the container and its neighbours or between its flex items.
    let mut pending = vec![(child, false, true, true)];
    // Whether a forced break falls inside the container, and whether it
    // holds absolutely positioned boxes, which would not move with the
    // items that a break moves down.
    let mut inner_breaks = false;
    let mut holds_absolute = false;
    // Whether a negative block margin pulls a box inside the container up
    // over the one before it, which a break between them would leave on the
    // wrong side of the break.
    let mut pulled = false;
    while let Some((id, contained, before, after)) = pending.pop() {
        boxes += 1;
        if boxes > MAX_SLICED_FLEX_BOXES
            || !is_sliceable_flex_box(tree, id, contained, (before, after))
        {
            return None;
        }
        if let Some((top, bottom)) = bounds {
            let start = tree.layout_origin_of(id)?.y;
            if start < top - EPSILON || start + tree.nodes[id].layout.size.height > bottom + EPSILON
            {
                return None;
            }
        }
        let node = layout_node(tree, id);
        inner_breaks |= id != child
            && (node.break_before == BreakBetween::Column
                || node.break_after == BreakBetween::Column);
        let margin = &tree.nodes[id].layout.margin;
        pulled |= id != child && (margin.top < -EPSILON || margin.bottom < -EPSILON);
        holds_absolute |= is_absolute_box(node) && !placed_from_the_top(&style_of(tree, id));
        let contained = contained || node.contains_absolute_boxes;
        let style = style_of(tree, id);
        let grid = node.display() == DisplayValue::Grid;
        // A row gap is truncated where a fragmentation break falls in it
        // (CSS Box Alignment 3, 8.1), which a grid sliced whole would not do.
        if grid
            && !matches!(style.gap().height.expand(), ExpandedLengthPercentage::Length(px) if px == 0.0)
        {
            return None;
        }
        // Rows of a length give a grid container a block size of its own.
        sized |= has_block_size(&style)
            || grid
                && crate::layout::taffy_adapter::RowSizing::have_a_length(
                    &style,
                    !in_flow_elements(tree, id).is_empty(),
                );
        // A break before the first box in flow inside a block, or after the
        // last, is a break before or after the block itself (CSS
        // Fragmentation 3, 3.1), and one before or after a flex item falls
        // between items. The first box of a flex container in the tree need
        // not be the first one laid out, so a break inside a flex item that
        // is itself a flex container does not reach the item.
        let in_flow = in_flow_elements(tree, id);
        let passes = id == child || passes_breaks_on(tree, id);
        let first = in_flow.first().copied().filter(|_| passes);
        let last = in_flow.last().copied().filter(|_| passes);
        let (before, after) = if id == child {
            (breaking_items, breaking_items)
        } else {
            (before, after)
        };
        // Text that renders gives a box lines, and a flex container makes an
        // anonymous item of text that is more than white space that collapses,
        // a no-break space included.
        for held in flow_children(tree, id) {
            let held_node = layout_node(tree, held);
            if !held_node.is_in_document() || held_node.is_display_none() {
                continue;
            }
            match &held_node.data {
                NodeData::Element(_) => pending.push((
                    held,
                    contained,
                    before && (id == child || first == Some(held)),
                    after && (id == child || last == Some(held)),
                )),
                NodeData::Text(text)
                    if text.text_content.chars().all(is_collapsible_whitespace) => {}
                _ => return None,
            }
        }
    }
    // The parts after a break between items move down to the next column,
    // which a container sized apart from its content would not grow to
    // hold, and neither the absolutely positioned boxes it holds, nor the
    // paint of an item that a relative offset moves, nor a box that a
    // negative margin pulls up over the start of its part would follow them.
    let style = style_of(tree, child);
    if inner_breaks
        && (has_block_size(&style)
            || style_dimension_length(style.max_size().height.into()).is_some()
            || holds_absolute
            || pulled
            || in_flow_elements(tree, child).into_iter().any(|item| {
                style_of(tree, item).inset() != Edges::splat(LengthPercentageAuto::auto())
            }))
    {
        return None;
    }
    Some(sized)
}

/// Whether a forced break before the first box in flow inside `id`, or
/// after the last, is one before or after `id`: `id` lays them out in a
/// block flow, or is a single-line flex container that lays them out in
/// the order of the tree. `id` is a box that the stream can slice (see
/// [`is_sliceable_flex_box`]): a block, flex or grid container. A grid
/// container places its items apart from the order of the tree, so a break
/// inside it stays there.
fn passes_breaks_on(tree: &LayoutSession<'_>, id: usize) -> bool {
    let display = layout_node(tree, id).display();
    debug_assert!(matches!(
        display,
        DisplayValue::Block | DisplayValue::FlowRoot | DisplayValue::Flex | DisplayValue::Grid
    ));
    let style = style_of(tree, id);
    let cascade = tree.style_views.cascade();
    display != DisplayValue::Grid
        && (display != DisplayValue::Flex
            || !style.flex_wraps()
                && !style.flex_direction_is_reversed()
                && in_flow_elements(tree, id)
                    .iter()
                    .all(|&item| cascade.computed[item].order == 0))
}

/// Whether an absolutely positioned box with `style` is placed a fixed
/// length below the top of its containing block, with a block size of its
/// own or from its content, so that it stays where it is when the block
/// grows.
fn placed_from_the_top(style: &LayoutStyleView<'_>) -> bool {
    let inset = style.inset();
    let top = inset.top.maybe_resolve(Some(0.0));
    let height = style.size().height;
    top.is_some()
        && top == inset.top.maybe_resolve(Some(1.0))
        && inset.bottom.is_auto()
        && (height.is_auto() || style_dimension_length(height).is_some())
}

/// Whether `id`, a flex or grid container or a box inside one, can be sliced with
/// the rest of the container (see [`sliceable_flex_container`]).
///
/// `breaks` says whether a forced column break before and after `id` is one
/// the stream honours: one between the container and its neighbours, or
/// between flex items (see [`flex_breaks`]).
fn is_sliceable_flex_box(
    tree: &LayoutSession<'_>,
    id: usize,
    contained: bool,
    breaks: (bool, bool),
) -> bool {
    let node = layout_node(tree, id);
    let style = style_of(tree, id);
    matches!(
        node.display(),
        DisplayValue::Block | DisplayValue::FlowRoot | DisplayValue::Flex | DisplayValue::Grid
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
        && (!sets_break_in_columns(node.break_before)
            || breaks.0 && node.break_before == BreakBetween::Column)
        && (!sets_break_in_columns(node.break_after)
            || breaks.1 && node.break_after == BreakBetween::Column)
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

/// Whether forced breaks before and after the items of the flex or grid
/// container `id` fall between them: between the items of a single-line column
/// container, which stack along the block axis, and between the lines of a
/// row container, which take the breaks of their items (CSS Flexbox 1, 12).
/// A multi-line column container breaks each of its lines on its own, and
/// one whose lines stack the other way breaks them in the other order, so
/// both keep the other strategies, and so does a grid container.
fn breaks_between_items(tree: &LayoutSession<'_>, id: usize) -> bool {
    let style = style_of(tree, id);
    if layout_node(tree, id).display() != DisplayValue::Flex {
        false
    } else if style.flex_direction_is_row() {
        !style.flex_lines_are_reversed()
    } else {
        !style.flex_wraps()
    }
}

/// Whether `id` is a flex or grid container, which the stream slices whole
/// when it can (see [`sliceable_flex_container`]).
fn is_flex_or_grid(tree: &LayoutSession<'_>, id: usize) -> bool {
    matches!(
        layout_node(tree, id).display(),
        DisplayValue::Flex | DisplayValue::Grid
    )
}

/// The rendered element children of `id` in flow.
fn in_flow_elements(tree: &LayoutSession<'_>, id: usize) -> Vec<usize> {
    rendered_elements(tree, id)
        .filter(|&child| {
            matches!(
                layout_node(tree, child).layout_position(),
                Position::Relative | Position::Static
            )
        })
        .collect()
}

/// Whether a forced column break falls before `id` (after it, unless
/// `before`): one set on it, or on the first (last) box in flow inside it,
/// which propagates to it (CSS Fragmentation 3, 3.1). Breaks that are not
/// column breaks do not break columns, as in Chromium. A column break below
/// a box that does not pass breaks on (see [`passes_breaks_on`]) keeps the
/// container from the stream, so the walk need not stop there.
fn forced_column_break(tree: &LayoutSession<'_>, mut id: usize, before: bool) -> bool {
    loop {
        let node = layout_node(tree, id);
        let value = if before {
            node.break_before
        } else {
            node.break_after
        };
        if value == BreakBetween::Column {
            return true;
        }
        let children = in_flow_elements(tree, id);
        let next = if before {
            children.first()
        } else {
            children.last()
        };
        match next {
            Some(&child) => id = child,
            None => return false,
        }
    }
}

/// The number of places in the flow of `owner` where a forced column break
/// starts a new column: each place between two block-level boxes in flow, in
/// `owner` or in the blocks inside it, where a break falls after the first or
/// before the second counts once. A break propagates from the first or last
/// box of a block to the block (CSS Fragmentation 3, 3.1). Breaks on inline
/// boxes, and those below a box that lays out apart, such as a nested
/// multicol container, a flex or grid container or a table, are left out.
pub(super) fn forced_column_starts(tree: &LayoutSession<'_>, owner: usize) -> usize {
    let mut count = 0;
    let mut stack = vec![(owner, 0)];
    while let Some((id, depth)) = stack.pop() {
        let children = if id == owner {
            rendered_elements(tree, id)
                .filter(|&child| layout_node(tree, child).layout_position() != Position::Absolute)
                .collect()
        } else {
            block_flow_children(tree, id)
        };
        count += children
            .windows(2)
            .filter(|pair| {
                starts_column(tree, pair[0], false, depth)
                    || starts_column(tree, pair[1], true, depth)
            })
            .count();
        if depth + 1 < MAX_WRAPPED_BLOCK_DEPTH {
            stack.extend(children.into_iter().map(|child| (child, depth + 1)));
        }
    }
    count
}

/// The block-level boxes in flow in `id`, when it is a block container whose
/// breaks are those of the flow it is in; none otherwise.
fn block_flow_children(tree: &LayoutSession<'_>, id: usize) -> Vec<usize> {
    let node = layout_node(tree, id);
    let passes_breaks = matches!(
        node.display(),
        DisplayValue::Block | DisplayValue::FlowRoot | DisplayValue::ListItem
    ) && node.multicol.is_none()
        && !node.is_ifc_root();
    if !passes_breaks {
        return Vec::new();
    }
    rendered_elements(tree, id)
        .filter(|&child| layout_node(tree, child).layout_position() != Position::Absolute)
        .collect()
}

/// Whether a forced column break falls before the block-level box `id`
/// (after it, unless `before`), set on it or on the first (last) box in flow
/// of the blocks it starts (ends) with, down to `MAX_WRAPPED_BLOCK_DEPTH`
/// below the container.
fn starts_column(tree: &LayoutSession<'_>, id: usize, before: bool, depth: usize) -> bool {
    std::iter::successors(Some(id), |&id| {
        let children = block_flow_children(tree, id);
        if before {
            children.first().copied()
        } else {
            children.last().copied()
        }
    })
    .take(MAX_WRAPPED_BLOCK_DEPTH.saturating_sub(depth))
    .any(|id| {
        let node = layout_node(tree, id);
        let value = if before {
            node.break_before
        } else {
            node.break_after
        };
        value == BreakBetween::Column
    })
}

/// The forced column breaks of a flex container that the stream slices
/// whole.
#[derive(Default)]
struct FlexBreaks {
    /// Whether one falls before the container, and whether one falls after.
    before: bool,
    after: bool,
    /// The parts that the breaks between its items cut the container into,
    /// or none when no break falls between them.
    parts: Vec<FlexPart>,
}

/// The part of a flex container between two forced breaks inside it, at
/// block offsets from the top of its border box. The space between the end
/// of one part and the start of the next, a gap or the margins before the
/// break, is truncated (CSS Fragmentation 3, 5.2); the margins after it are
/// kept.
struct FlexPart {
    start: f32,
    end: f32,
    /// The flex items the part holds, which move down with it.
    items: Vec<usize>,
}

/// The flex lines of a row container of `items`, in order: in `order`, each
/// item starts further along the main axis than the one before ends, until
/// one starts back before it, which starts the next line. Gaps and
/// `justify-content` can leave the items of two lines side by side without
/// overlapping, but never send an item of the same line backwards.
fn flex_lines(
    tree: &LayoutSession<'_>,
    container: usize,
    mut items: Vec<usize>,
) -> Vec<Vec<usize>> {
    let style = style_of(tree, container);
    if !style.flex_wraps() {
        return vec![items];
    }
    let cascade = tree.style_views.cascade();
    items.sort_by_key(|&id| cascade.computed[id].order);
    let forward = style.flex_direction_is_reversed() == style.direction().is_rtl();
    let span = |id: usize| {
        let layout = &tree.nodes[id].layout;
        (
            layout.location.x - layout.margin.left,
            layout.location.x + layout.size.width + layout.margin.right,
        )
    };
    let mut lines = Vec::new();
    let mut line: Vec<usize> = Vec::new();
    for id in items {
        let (start, end) = span(id);
        let starts_over = line.last().is_some_and(|&previous| {
            let (previous_start, previous_end) = span(previous);
            if forward {
                start < previous_end - EPSILON
            } else {
                end > previous_start + EPSILON
            }
        });
        if starts_over {
            lines.push(std::mem::take(&mut line));
        }
        line.push(id);
    }
    lines.push(line);
    lines
}

/// The forced column breaks of `child`, a flex or grid container the stream slices
/// whole (see [`sliceable_flex_container`]), laid out with a border box
/// `height` tall. A break before the first line of items is a break before
/// the container, and one after the last a break after it.
/// [`sliceable_flex_container`] leaves the containers whose parts could not
/// move down with their items to the other strategies.
fn flex_breaks(tree: &LayoutSession<'_>, child: usize, height: f32) -> FlexBreaks {
    let node = layout_node(tree, child);
    let style = style_of(tree, child);
    let mut breaks = FlexBreaks {
        before: node.break_before == BreakBetween::Column,
        after: node.break_after == BreakBetween::Column,
        parts: Vec::new(),
    };
    if !breaks_between_items(tree, child) {
        return breaks;
    }
    let items = in_flow_elements(tree, child);
    let lines = if style.flex_direction_is_row() {
        flex_lines(tree, child, items)
    } else {
        items.into_iter().map(|item| vec![item]).collect()
    };
    // Each line with its block extent, from the block-start margin edge
    // that a break before it keeps to the border edge where the margins
    // after it are truncated, and the breaks before and after it.
    let mut lines: Vec<(f32, f32, bool, bool, Vec<usize>)> = lines
        .into_iter()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let layout = |id: usize| &tree.nodes[id].layout;
            let start = line
                .iter()
                .map(|&id| layout(id).location.y - layout(id).margin.top)
                .fold(f32::INFINITY, f32::min);
            let end = line
                .iter()
                .map(|&id| layout(id).location.y + layout(id).size.height)
                .fold(f32::NEG_INFINITY, f32::max);
            let before = line.iter().any(|&id| forced_column_break(tree, id, true));
            let after = line.iter().any(|&id| forced_column_break(tree, id, false));
            (start, end, before, after, line)
        })
        .collect();
    lines.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (Some(first), Some(last)) = (lines.first(), lines.last()) else {
        return breaks;
    };
    breaks.before |= first.2;
    breaks.after |= last.3;
    if !lines.windows(2).any(|pair| pair[0].3 || pair[1].2) {
        return breaks;
    }
    let mut part = FlexPart {
        start: 0.0,
        end: 0.0,
        items: Vec::new(),
    };
    let mut previous_after = false;
    for (start, end, before, after, items) in lines {
        if (previous_after || before) && !part.items.is_empty() {
            let next = FlexPart {
                start,
                end: 0.0,
                items: Vec::new(),
            };
            breaks.parts.push(std::mem::replace(&mut part, next));
        }
        part.end = part.end.max(end);
        part.items.extend(items);
        previous_after = after;
    }
    part.end = height;
    breaks.parts.push(part);
    breaks
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
    let independent_context =
        node.display() == DisplayValue::FlowRoot || node.roots_block_context();
    let children_in_multicol_context = in_multicol_context && !independent_context;
    let has_edges = has_block_edge_decorations(&style);
    // A column container lays its blocks out in columns of its own, not
    // stacked in the stream (see `is_unbroken_column_container`).
    if node.multicol.is_some()
        || independent_context && !allow_sliced_wrappers
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

/// Whether `style` has padding or border on the left or right.
pub(super) fn has_inline_edge_decorations(style: &LayoutStyleView<'_>) -> bool {
    let zero = LengthPercentage::length(0.0);
    let (padding, border) = (style.padding(), style.border());
    padding.left != zero || padding.right != zero || border.left != zero || border.right != zero
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
    flow_children(tree, id).into_iter().any(|child| {
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
    flow_children(tree, child)
        .into_iter()
        .filter(|&id| {
            let node = layout_node(tree, id);
            node.is_in_document() && !node.is_display_none() && is_absolute_box(node)
        })
        .collect()
}

/// The pieces of the absolutely positioned boxes of each child, with the
/// moves of the content of each box whose lines a column end crosses.
type AbsoluteLayout = (Vec<Vec<AbsolutePiece>>, Vec<(usize, LineMoves)>);

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

/// How far the clip of each of `count` pieces of a box reaches left and right,
/// from `start`: the extent it has by itself, widened by the clips of the
/// pieces of absolutely positioned boxes that sit in it. A piece of those with
/// no piece of the box to sit in widens none.
fn piece_clip_extents(
    count: usize,
    start: (f32, f32),
    absolute: &[AbsolutePiece],
) -> Vec<(f32, f32)> {
    #[cfg(test)]
    tests::record_absolute_parent_work(2, absolute.len());
    let mut extents = vec![start; count];
    for piece in absolute {
        if let Some((left, right)) = extents.get_mut(piece.parent_piece) {
            *left = left.min(piece.clip.x);
            *right = right.max(piece.clip.x + piece.clip.width);
        }
    }
    extents
}

/// The index of the first piece that starts in each column of `pieces`.
fn first_piece_per_column(pieces: &[Piece]) -> HashMap<usize, usize> {
    #[cfg(test)]
    tests::record_absolute_parent_work(0, pieces.len());
    let mut first = HashMap::with_capacity(pieces.len());
    for (index, piece) in pieces.iter().enumerate() {
        first.entry(piece.column).or_insert(index);
    }
    first
}

/// The pieces of each child placed by `placements` in columns of
/// `column_height`, with the pieces of the leaves of each wrapper.
type ChildPieces = (Vec<Vec<Piece>>, Vec<Option<Vec<Vec<Piece>>>>);

fn pieces_of_children(
    spans: &[std::ops::Range<usize>],
    wrapped: &[Option<WrappedBlockTree>],
    placements: &Placement,
    column_height: f32,
) -> Option<ChildPieces> {
    let mut child_pieces = Vec::with_capacity(spans.len());
    let mut wrapped_leaf_pieces = Vec::with_capacity(spans.len());
    for (span, wrapped_tree) in spans.iter().zip(wrapped) {
        let pieces = &placements.pieces[span.clone()];
        match wrapped_tree {
            // The parts of a flex container that breaks between its items
            // are one box: the last piece of a part that a forced break ends
            // fills the rest of its column (see [`flex_breaks`]).
            None => {
                let mut whole: Vec<Piece> = Vec::new();
                for part in pieces {
                    if let (Some(last), Some(next)) = (whole.last_mut(), part.first())
                        && next.column > last.column
                        && column_height.is_finite()
                    {
                        last.height = last.height.max(column_height - last.top);
                    }
                    whole.extend(part.iter().cloned());
                }
                child_pieces.push(Some(whole).filter(|whole| !whole.is_empty())?);
                wrapped_leaf_pieces.push(None);
            }
            Some(wrapped_tree) => {
                let leaf_pieces = make_wrapped_leaf_pieces(wrapped_tree, pieces)?;
                let columns = Columns::Uniform(column_height);
                let wrapper = wrapper_pieces_for_tree(wrapped_tree, &leaf_pieces, columns)?;
                child_pieces.push(wrapper);
                wrapped_leaf_pieces.push(Some(leaf_pieces));
            }
        }
    }
    Some((child_pieces, wrapped_leaf_pieces))
}

/// The children of a column container laid end to end as stream items.
#[derive(Clone, Copy)]
struct Stream<'a> {
    children: &'a [usize],
    sliced_flex: &'a [bool],
    items: &'a [Item],
    spans: &'a [std::ops::Range<usize>],
    wrapped: &'a [Option<WrappedBlockTree>],
}

/// The height of balanced columns that also hold the absolutely positioned
/// boxes of the children, from the `balanced` height of the boxes in flow:
/// balancing counts the content out of flow whose containing block is in the
/// columns, as Chromium does (see [`column_line_boxes::out_of_flow_need`]).
/// Each step grows the columns to the height a box that does not fit needs,
/// or by the least that keeps one of the lines a column end crosses in its
/// column, and lays the stream out again, which moves the containing blocks;
/// the search stops after a few steps.
fn out_of_flow_height(
    tree: &LayoutSession<'_>,
    stream: Stream<'_>,
    column_count: usize,
    balanced: f32,
) -> f32 {
    let cascade = tree.style_views.cascade();
    // The positioned boxes the columns hold, for each child.
    let boxes: Vec<Vec<usize>> = stream
        .children
        .iter()
        .zip(stream.sliced_flex)
        .map(|(&child, &flex)| {
            if flex {
                return Vec::new();
            }
            // Only a child that is the containing block of its boxes puts
            // them in the columns.
            // Boxes inside monolithic content stay in it.
            if tree.nodes[child].claims_absolute_oof
                && !column_line_boxes::is_monolithic(tree, cascade, child)
            {
                absolute_children(tree, child)
            } else {
                Vec::new()
            }
        })
        .collect();
    if boxes.iter().all(Vec::is_empty) {
        return balanced;
    }
    let mut height = balanced;
    for step in 0..super::OUT_OF_FLOW_STEPS {
        // Columns with no height yet lay the stream out in one column.
        let place_height = if height > 0.0 { height } else { f32::INFINITY };
        let placement = place(stream.items, place_height, usize::MAX).ok();
        let pieces = placement.and_then(|placement| {
            pieces_of_children(stream.spans, stream.wrapped, &placement, place_height)
        });
        let Some((pieces, _)) = pieces else { break };
        // The least height past this one that one of the boxes that do not
        // fit needs.
        let mut needed = None;
        let firsts = pieces.iter().zip(&boxes);
        for (first, boxes) in firsts.filter_map(|(pieces, boxes)| Some((pieces.first()?, boxes))) {
            let top = if height > 0.0 {
                first.column as f32 * height + first.top
            } else {
                first.top
            };
            for &node in boxes {
                let mut layout = tree.nodes[node].layout;
                let start = top + layout.location.y;
                // Lines that a column end crosses start the next column, and
                // the column could instead grow until the first of them fits.
                let mut shortage = f32::INFINITY;
                if column_count > 1
                    && let Some(moves) = column_lines::line_moves(tree, node, start, height, None)
                {
                    layout = moves.moved(layout);
                    shortage = moves.shortage();
                }
                let need = column_line_boxes::out_of_flow_need(
                    first.top + layout.location.y,
                    first.column,
                    column_line_boxes::out_of_flow_extent(cascade, node, layout),
                    column_line_boxes::is_monolithic(tree, cascade, node),
                    height,
                    column_count,
                );
                if need > height + EPSILON {
                    needed = Some(super::out_of_flow_step(
                        step,
                        needed,
                        need,
                        height + shortage,
                    ));
                }
            }
        }
        let Some(needed) = needed else { break };
        height = needed;
    }
    height
}

/// The pieces of the absolutely positioned boxes of each child, in the
/// coordinates of the child's pieces. A box breaks at the column ends of
/// the flow its containing block is part of (CSS Position 3, 3.3.4), and a
/// box that reaches past the last column continues in overflow columns,
/// unless `last_column` names a last column with no block-end limit, which
/// holds the rest of the box. The last piece of the containing block holds
/// the box's pieces in the columns after it. `None` when a box starts above
/// the columns or its containing block, or when its pieces would not fit in
/// `budget` fragments. With them, how the content of each box moves where a
/// column end crosses its lines (see [`column_lines`]): the pieces are those
/// of the moved box, and the caller moves it once it keeps the layout.
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
) -> Option<AbsoluteLayout> {
    let column_of = |offset: f32| {
        let column = (offset / column_height).floor() as usize;
        last_column.map_or(column, |last| column.min(last))
    };
    let mut count = 0usize;
    let mut all = Vec::with_capacity(children.len());
    let mut line_moves = Vec::new();
    for ((&child, &flex), pieces) in children.iter().zip(sliced_flex).zip(placements) {
        let mut own = Vec::new();
        let first = pieces.first()?;
        let last = pieces.last()?;
        // A flex container is sliced whole, with the absolutely positioned
        // boxes it holds, and monolithic content keeps its boxes inside it.
        if flex || column_line_boxes::is_monolithic(tree, tree.style_views.cascade(), child) {
            all.push(own);
            continue;
        }
        // The containing block's block offset in the columns laid end to end.
        let flow_top = first.column as f32 * column_height + first.top;
        // Which piece starts in each column, read once for a box that has
        // absolutely positioned children.
        let mut piece_of_column: Option<HashMap<usize, usize>> = None;
        for node in absolute_children(tree, child) {
            let mut layout = tree.nodes[node].layout;
            let start = flow_top + layout.location.y;
            // The lines that a column end crosses start the next column.
            if let Some(moves) =
                column_lines::line_moves(tree, node, start, column_height, last_column)
            {
                layout = moves.moved(layout);
                line_moves.push((node, moves));
            }
            // Descendants that overflow the box are cut with it.
            let extent = column_line_boxes::painted_extent(layout);
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
            #[cfg(test)]
            tests::record_absolute_parent_work(1, end_column - first_column + 1);
            let piece_of_column =
                piece_of_column.get_or_insert_with(|| first_piece_per_column(pieces));
            for column in first_column..=end_column {
                let parent_piece = match piece_of_column.get(&column) {
                    Some(&index) => index,
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
    Some((all, line_moves))
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
    /// A box a column end never cuts and that stays in a column only where it
    /// starts it, with no margin before it either: a scroll container (see
    /// [`is_monolithic_scroll_box`]) or a nested column container (see
    /// [`is_unbroken_column_container`]).
    monolithic: bool,
    /// The block-start and block-end border and padding of a box whose
    /// pieces slice them: it is cut where it is taller than a column, but
    /// not inside these insets.
    slice: Option<(f32, f32)>,
    /// The block-end border and padding of the wrappers that end with the
    /// box, counted in `height`: a break between the box and them would
    /// leave them alone in the next column. The box's own block-end border
    /// and padding are not counted: a break that would fall inside them
    /// falls where its content ends, and they go on alone (see `slice`).
    edges: f32,
    /// Whether a forced break comes before the box, which then starts the
    /// next column unless nothing precedes it in its own.
    break_before: bool,
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
    // Whether a box has been placed in the stream: a box with no block size
    // still precedes a forced break, which then starts the next column.
    let mut holds_boxes = false;
    for item in items {
        // A forced break starts the next column, unless nothing precedes the
        // box in this one. The margins before the break are truncated, and
        // the box keeps its own (CSS Fragmentation 3, 5.2).
        if item.break_before && (cursor > 0.0 || holds_boxes) {
            column += 1;
            if column >= max_columns {
                return Err(Stop::Exhausted);
            }
            cursor = 0.0;
            pending_margin = 0.0;
        }
        // Whatever the box leaves in the column, it is content before a
        // break that follows.
        holds_boxes = true;
        // A box with no block size, borders or padding is self-collapsing:
        // its own margins join the margins around it (CSS 2, 8.3.1). A scroll
        // container is not, as it establishes a formatting context, so it is
        // placed like any other monolithic box, between its own margins.
        if item.height <= 0.0 && !item.monolithic {
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
        // A monolithic box that does not fit after its margin moves to the
        // next column, where the margin is truncated: only a box that starts
        // the column with no margin before it may overflow it.
        let margin_pushes_it = item.monolithic && gap > 0.0;
        if (cursor > 0.0 || gap_fills_column || moves_whole || margin_pushes_it)
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
                // The border and padding of the wrappers that end with the
                // box follow its own, and go on with them.
                let room = match item.slice.map(|(_, trailing)| trailing + item.edges) {
                    Some(trailing)
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
/// adjoining margins collapsed. Forced breaks start further columns, and
/// the tallest of them is the block size.
fn stacked_height(items: &[Item], max_pieces: usize) -> Option<f32> {
    place(items, f32::INFINITY, max_pieces)
        .ok()
        .map(|placement| placement.extent_after(0))
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
    let stacked = stacked_height(items, usize::MAX).unwrap_or(0.0);
    // The end of the last column truncates the margin below the last box,
    // so the columns need not hold it. As in Chromium, the columns are
    // still at least as tall as the stream with that margin divided among
    // them.
    let mut trimmed = items.to_vec();
    if let Some(last) = trimmed.last_mut() {
        last.margin_bottom = 0.0;
    }
    let high = stacked_height(&trimmed, usize::MAX).unwrap_or(0.0);
    balanced_height_after(
        &trimmed,
        0,
        &|_| 0.0,
        column_count,
        max_columns,
        high,
        false,
    )
    .max(stacked / column_count.max(1) as f32)
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
    // Forced breaks can start more columns than there are, which no height
    // keeps within them; the overflow columns are then as tall as `high`,
    // the tallest run between breaks.
    if items.iter().any(|item| item.break_before) {
        low = low.min(high);
    }
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
    columns: Columns<'_>,
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
    wrapper_pieces(first, last, layout.leading, layout.trailing, columns)
}

/// The block sizes of the columns a stream fills, one after the other.
#[derive(Clone, Copy)]
enum Columns<'a> {
    /// Columns of one block size.
    Uniform(f32),
    /// Where each column starts in the stream, with the end of the last.
    Starts(&'a [f32]),
}

impl Columns<'_> {
    fn height(self, column: usize) -> f32 {
        match self {
            Self::Uniform(height) => height,
            Self::Starts(starts) => starts
                .get(column + 1)
                .map_or(f32::INFINITY, |end| end - starts[column]),
        }
    }

    /// Where `column` starts in the stream.
    fn start(self, column: usize) -> f32 {
        match self {
            Self::Uniform(height) => column as f32 * height,
            Self::Starts(starts) => starts[column.min(starts.len() - 1)],
        }
    }

    /// The column that holds the stream offset `offset`.
    fn at(self, offset: f32) -> usize {
        match self {
            Self::Uniform(height) => (offset / height).floor() as usize,
            Self::Starts(starts) => starts
                .partition_point(|&start| start <= offset)
                .saturating_sub(1),
        }
    }
}

fn wrapper_pieces(
    first: &Piece,
    last: &Piece,
    leading: f32,
    trailing: f32,
    columns: Columns<'_>,
) -> Option<Vec<Piece>> {
    let start = columns.start(first.column) + first.top - leading;
    let end = columns.start(last.column) + last.top + last.height + trailing;
    if !start.is_finite() || !end.is_finite() || start < 0.0 || end < start {
        return None;
    }
    let first_column = columns.at(start);
    let last_column = if end - start <= EPSILON {
        first_column
    } else {
        columns.at((end - EPSILON).max(start))
    };
    Some(
        (first_column..=last_column)
            .map(|column| {
                let top = if column == first_column {
                    start - columns.start(column)
                } else {
                    0.0
                };
                let bottom = if column == last_column {
                    end - columns.start(column)
                } else {
                    columns.height(column)
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
    let mut count =
        wrapper_pieces_for_tree(wrapped, leaf_pieces, Columns::Uniform(column_height))?.len();
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

/// Record what `wrapped` holds in the pieces `parent_pieces` of the wrapper,
/// whose fragments are `parent_records`. A piece of a wrapper that slices its
/// decorations sits higher than its place in the column by its entry in
/// `parent_shifts`, which what it holds makes up for.
#[allow(clippy::too_many_arguments)]
fn push_wrapped_children(
    tree: &mut LayoutSession<'_>,
    wrapped: &WrappedBlockTree,
    leaf_pieces: &[Vec<Piece>],
    parent_pieces: &[Piece],
    parent_records: &[usize],
    parent_shifts: &[f32],
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
                    (parent_records, parent_shifts),
                    context,
                    column_height,
                )?;
                offset += 1;
            }
            WrappedBlockChild::Wrapper(nested) => {
                let end = offset.checked_add(nested.leaf_count)?;
                let nested_leaf_pieces = leaf_pieces.get(offset..end)?;
                let grid = Columns::Uniform(column_height);
                let pieces = wrapper_pieces_for_tree(nested, nested_leaf_pieces, grid)?;
                let records = push_wrapped_wrapper(
                    tree,
                    nested.node,
                    &pieces,
                    parent_pieces,
                    (parent_records, parent_shifts),
                    context,
                    column_height,
                )?;
                push_wrapped_children(
                    tree,
                    nested,
                    nested_leaf_pieces,
                    &pieces,
                    &records,
                    &[],
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
    (parent_records, parent_shifts): (&[usize], &[f32]),
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
        let shift = parent_shifts.get(parent_index).copied().unwrap_or(0.0);
        let x_offset =
            context.column_offset_x(piece.column) - context.column_offset_x(parent.column);
        let record = tree.fragment_tree.try_push(LayoutFragment {
            node_id: wrapper,
            parent: Some(parent_records[parent_index]),
            fragmentainer: context.column_index + piece.column,
            rect: FragmentRect {
                x: x + x_offset,
                y: piece.top - parent.top + shift,
                width: size.width,
                height: piece.height,
            },
            fragmentainer_clip: (pieces.len() > 1).then_some(
                FragmentRect {
                    x: x_offset,
                    y: shift - parent.top,
                    width: (x + size.width).max(context.column_width),
                    height: column_height,
                }
                .into(),
            ),
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
    (records, parent_shifts): (&[usize], &[f32]),
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
        let shift = parent_shifts.get(parent_index).copied().unwrap_or(0.0);
        tree.fragment_tree.try_push(LayoutFragment {
            node_id: leaf,
            parent: Some(records[parent_index]),
            fragmentainer: context.column_index + piece.column,
            rect: sliced_rect(
                FragmentRect {
                    x,
                    y: piece.top - parent.top + shift,
                    width: size.width,
                    height: piece.height,
                },
                sliced.then_some((taken, size.height)),
            ),
            // A box that stays in one column is not cut.
            fragmentainer_clip: (pieces.len() > 1).then_some(
                FragmentRect {
                    x: 0.0,
                    y: shift - parent.top,
                    width: (x + size.width).max(context.column_width),
                    height: sliced_piece_clip_height(sliced, pieces, index, column_height),
                }
                .into(),
            ),
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

/// The block size, from the top of its column, of the clip of the `index`th
/// of `pieces`, those of a box cut at column ends in columns `column_height`
/// tall. A piece of a box that paints whole in each piece, `sliced`, ends its
/// clip where the piece ends when the piece ends above the column end, as one
/// does when the box's block-end border and padding go on alone, so that the
/// next piece alone paints what follows. The last piece, and every piece of
/// any other box, is clipped at the column end.
fn sliced_piece_clip_height(
    sliced: bool,
    pieces: &[Piece],
    index: usize,
    column_height: f32,
) -> f32 {
    match pieces.get(index) {
        Some(piece) if sliced && index + 1 < pieces.len() => {
            (piece.top + piece.height).min(column_height)
        }
        _ => column_height,
    }
}

/// The rect of a piece of a box that slices its decorations: the whole box,
/// of block size `whole.1`, moved up by the `whole.0` the earlier pieces took.
/// The clip of the column then cuts it to the piece. `rect` is the piece's own.
pub(super) fn sliced_rect(rect: FragmentRect, whole: Option<(f32, f32)>) -> FragmentRect {
    match whole {
        Some((taken, height)) => FragmentRect {
            y: rect.y - taken,
            height,
            ..rect
        },
        None => rect,
    }
}

/// Whether the column ends may cut `idx`, a sized box with nothing inside it,
/// with its pieces painted as [`slices_decorations`] says: a box that avoids
/// breaks, one that is monolithic, one that repeats its decorations at each
/// break, and one with effects sized from the whole box stay whole.
pub(super) fn slices_sized_leaf(tree: &LayoutSession<'_>, idx: usize) -> bool {
    let node = layout_node(tree, idx);
    !matches!(
        node.break_inside,
        BreakInside::Avoid | BreakInside::AvoidColumn
    ) && !node.clones_box_decorations
        && !node.paints_whole_box_effects
        && !is_monolithic_scroll_box(tree, idx)
        && !is_unbroken_column_container(tree, idx)
}

/// Whether a box cut at a column end slices its block-axis borders and
/// padding (CSS Fragmentation 3, 5.4): its pieces paint the whole box, moved
/// up by what the earlier pieces took and clipped to the column, so no border
/// shows where it breaks. Effects sized from the whole box, and decorations
/// that repeat at each break, are not sliced.
pub(super) fn slices_decorations(tree: &LayoutSession<'_>, idx: usize) -> bool {
    let node = layout_node(tree, idx);
    has_block_edge_decorations(&style_of(tree, idx))
        && !node.clones_box_decorations
        && !node.paints_whole_box_effects
        && !is_monolithic_scroll_box(tree, idx)
        && !is_unbroken_column_container(tree, idx)
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
    let monolithic =
        is_monolithic_scroll_box(tree, child) || is_unbroken_column_container(tree, child);
    let splittable = !monolithic
        && !matches!(
            node.break_inside,
            BreakInside::Avoid | BreakInside::AvoidColumn
        );
    Item {
        height,
        margin_top,
        margin_bottom,
        splittable,
        // A box with effects sized from the whole box, or with block-axis
        // borders or padding that its pieces do not slice, sits whole in the
        // next column when it fits there: a piece that does not slice them
        // would repeat them at each break. One taller than a column is cut
        // only where its pieces slice the decorations. A box whose pieces
        // slice them breaks at a column end like any other (CSS
        // Fragmentation 3, 5.4), unless it avoids breaks. A monolithic box
        // is never cut.
        whole: monolithic
            || node.paints_whole_box_effects
            || (has_block_edge_decorations(&style) && (slice.is_none() || !splittable)),
        monolithic,
        slice,
        edges: 0.0,
        break_before: false,
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
/// which bounds columns that fill in turn from below and makes the container
/// taller than balanced columns. `definite` says layout fixed the container's
/// height; otherwise the columns balance, bounded by the content-box
/// `maximum` when there is one. Returns the used border-box height of the
/// container.
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
        let input = super::column_child_layout_input(
            tree,
            child,
            context.column_width,
            definite_height,
            AvailableSpace::MaxContent,
        );
        let output = tree.compute_child_layout(NodeId::from(child), input);
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
            is_flex_or_grid(tree, child)
                && sliceable_flex_container(tree, child, Some(size.height)).is_some()
        })
        .collect();
    if children
        .iter()
        .zip(&sliced_flex)
        .any(|(&child, &flex)| is_flex_or_grid(tree, child) && !flex)
    {
        return None;
    }
    // The forced breaks of each flex container, which the stream honours
    // around it and between its items.
    let flex_breaks = children
        .iter()
        .zip(&sizes)
        .zip(&sliced_flex)
        .map(|((&child, size), &flex)| flex.then(|| flex_breaks(tree, child, size.height)))
        .collect::<Vec<_>>();
    // A wrapper that holds blocks puts them in the stream in its place, so
    // each child has a range of stream items.
    let mut wrapped: Vec<Option<WrappedBlockTree>> = children
        .iter()
        .map(|&child| wrapped_blocks(tree, child, allow_wrapped_blocks))
        .collect();
    let mut items = Vec::new();
    let mut spans = Vec::with_capacity(children.len());
    // Whether a forced break follows the child before.
    let mut break_after = false;
    for (((&child, size), wrapped_tree), breaks) in children
        .iter()
        .zip(&sizes)
        .zip(&mut wrapped)
        .zip(&flex_breaks)
    {
        let first = items.len();
        match wrapped_tree {
            None => {
                let whole = item(
                    tree,
                    child,
                    size.height,
                    Some(Insets::Against(context.column_width)),
                );
                match breaks.as_ref().map(|breaks| breaks.parts.as_slice()) {
                    // Each part of a flex container that breaks between its
                    // items is a box of its own in the stream, with the
                    // container's margins and insets at its ends.
                    Some(parts) if !parts.is_empty() => {
                        let last = parts.len() - 1;
                        items.extend(parts.iter().enumerate().map(|(index, part)| Item {
                            height: part.end - part.start,
                            margin_top: if index == 0 { whole.margin_top } else { 0.0 },
                            margin_bottom: if index == last {
                                whole.margin_bottom
                            } else {
                                0.0
                            },
                            slice: whole.slice.map(|(leading, trailing)| {
                                (
                                    if index == 0 { leading } else { 0.0 },
                                    if index == last { trailing } else { 0.0 },
                                )
                            }),
                            break_before: index > 0,
                            ..whole
                        }));
                    }
                    _ => items.push(whole),
                }
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
        let break_before = break_after || breaks.as_ref().is_some_and(|breaks| breaks.before);
        if let Some(first) = items.get_mut(first) {
            first.break_before |= break_before;
            break_after = false;
        }
        break_after |= breaks.as_ref().is_some_and(|breaks| breaks.after);
        spans.push(first..items.len());
    }
    // One fragment for the container, the rest for the child pieces.
    let max_pieces = tree
        .fragment_tree
        .limit
        .saturating_sub(tree.fragment_tree.fragments.len())
        .saturating_sub(1);
    let stream = Stream {
        children,
        sliced_flex: &sliced_flex,
        items: &items,
        spans: &spans,
        wrapped: &wrapped,
    };
    let column_height = match definite_height {
        // Columns that balance are as tall as their content needs, up to the
        // height of the container (CSS Multi-column 1, 8.1); the container
        // keeps its own height. Content that needs more than the container
        // gives fills it, and continues in overflow columns.
        Some(height) if context.column_fill != ColumnFillValue::Auto => balanced_below(
            out_of_flow_height(
                tree,
                stream,
                context.column_count,
                balanced_height(&items, context.column_count, max_pieces),
            ),
            height,
        )
        .unwrap_or(height),
        Some(height) => height,
        // Forced breaks start further columns, which balance as Chromium
        // balances them.
        None if context.column_fill == ColumnFillValue::Auto
            && !items.iter().any(|item| item.break_before) =>
        {
            // A single unbounded column cuts nothing, so only the number of
            // boxes can run out of fragments.
            let Some(height) = stacked_height(&items, max_pieces) else {
                tree.fragment_tree.limit_exceeded = true;
                return None;
            };
            // The one column holds the positioned boxes too.
            out_of_flow_height(tree, stream, 1, height)
        }
        None => out_of_flow_height(
            tree,
            stream,
            context.column_count,
            balanced_height(&items, context.column_count, max_pieces),
        ),
    };
    let minimum_content = minimum - origin.y - block_end_inset;
    // A maximum height bounds the columns of an auto-height container, though
    // not below its minimum height, which wins over it (CSS 2, 10.7).
    let column_height = match (definite_height, maximum) {
        (None, Some(maximum)) => column_height.min(maximum.max(minimum_content)),
        _ => column_height,
    };
    // Columns that fill in turn are as tall as the container's content box
    // (CSS Multi-column 1, 2), so a minimum height makes them taller.
    // Balanced columns are as tall as their content needs and leave the
    // minimum height to the container, which grows below them.
    let column_height = if context.column_fill == ColumnFillValue::Auto {
        column_height.max(minimum_content)
    } else {
        column_height
    };
    if !column_height.is_finite() || column_height <= 0.0 {
        return None;
    }
    // A nested column container is kept whole only where the columns grow to
    // fit it; one that a maximum keeps them below is cut by the strategies
    // that break it at the outer column end.
    if children.iter().zip(&sizes).any(|(&child, size)| {
        size.height > column_height + EPSILON && is_unbroken_column_container(tree, child)
    }) {
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
    let (child_pieces, wrapped_leaf_pieces) =
        pieces_of_children(&spans, &wrapped, &placements, column_height)?;
    // The pieces of a flex container that breaks between its items fill
    // each column a break ends, so each part starts the next column as far
    // down the container as the pieces before it took: its items move down
    // by the space the break left, and the container grows by as much. The
    // moves apply once the layout is kept.
    let mut moves = Vec::new();
    for (order, breaks) in flex_breaks.iter().enumerate() {
        let Some(breaks) = breaks.as_ref().filter(|breaks| !breaks.parts.is_empty()) else {
            continue;
        };
        let mut pieces = child_pieces[order].iter();
        let mut taken = 0.0f32;
        for (part, placed) in breaks
            .parts
            .iter()
            .zip(&placements.pieces[spans[order].clone()])
        {
            let shift = taken - part.start;
            moves.extend(part.items.iter().map(|&id| (id, shift)));
            taken += pieces
                .by_ref()
                .take(placed.len())
                .map(|piece| piece.height)
                .sum::<f32>();
        }
        sizes[order].height = taken;
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
    let (absolute, line_moves) = absolute_pieces(
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
    // The caller applies the minimum to the height it sets as well; the check
    // against the outer column below needs it here.
    let used_height = if definite_height.is_some() {
        border_box.height
    } else {
        (origin.y + column_height + block_end_inset).max(minimum)
    };
    // Columns sliced here are not cut again at the end of an outer column,
    // so a nested container taller than the outer column is left to the
    // strategies that are.
    if outer_height.is_some_and(|outer| used_height > outer + EPSILON) {
        return None;
    }
    for (id, shift) in moves {
        let mut layout = tree.nodes[id].layout;
        layout.location.y += shift;
        tree.set_unrounded_layout(NodeId::from(id), &layout);
    }
    tree.fragment_tree.retire_previous_roots(parent);
    tree.nodes[parent].multicol_groups.clear();
    tree.nodes[parent].multicol_rows.clear();
    // Columns that fill in turn under an auto height are one column that
    // nothing limits. Pages that cut the container fill them again, page by
    // page, from the group kept here.
    if definite_height.is_none()
        && maximum.is_none()
        && minimum <= origin.y + block_end_inset
        && context.column_fill == ColumnFillValue::Auto
        && outer_height.is_none()
        && tree.context.fragmentation_stack.len() <= 1
        && !tree.context.unpaged_plain_columns
        && absolute.iter().all(Vec::is_empty)
        && !sliced_flex.contains(&true)
    {
        tree.nodes[parent].multicol_groups.push(MulticolGroup {
            context: FragmentationContext {
                origin_x: origin.x,
                origin_y: origin.y,
                ..context
            },
            height: column_height,
            occupied: placements
                .pieces
                .iter()
                .flatten()
                .filter(|piece| piece.height > 0.0)
                .map(|piece| piece.column)
                .collect(),
            slice: None,
            run_start: None,
        });
    }
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
        // `justify-self` places a box in its column.
        let justified = crate::layout::dispatch::block_flow::justified_offset(
            tree,
            child,
            context.rtl,
            context.column_width - size.width,
        )
        .unwrap_or(0.0);
        let mut layout = tree.nodes[child].layout;
        layout.order = order as u32;
        layout.size = size;
        layout.location = Point {
            x: origin.x + context.column_offset_x(first.column) + justified,
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
        // A box that stays in one column is not cut, so has no clip.
        let clip_extents = if pieces.len() > 1 {
            // The clip ends where the column does, which a box `justify-self`
            // moves along leaves nearer to it.
            let wide = size
                .width
                .max(context.column_width - justified)
                .max(held_right);
            piece_clip_extents(pieces.len(), (held_left, wide), absolute)
        } else {
            Vec::new()
        };
        let mut taken = 0.0f32;
        for (index, piece) in pieces.iter().enumerate() {
            shifts.push(if sliced { taken } else { 0.0 });
            let x = origin.x + context.column_offset_x(piece.column) + justified;
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
                    let (left, right) = clip_extents[index];
                    FragmentRect {
                        x: x + left,
                        y: origin.y,
                        width: right - left,
                        height: sliced_piece_clip_height(sliced, pieces, index, column_height),
                    }
                    .into()
                }),
                fragment_index: index,
                fragment_count: pieces.len(),
                line_start: None,
                line_end: None,
            })?;
            // A nested column container is not cut, so the fragments its own
            // layout placed hang under its single piece.
            if pieces.len() == 1 && tree.nodes[child].multicol.is_some() {
                tree.fragment_tree.reparent_roots(child, record);
            }
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
                &shifts,
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
                fragmentainer_clip: Some(
                    FragmentRect {
                        y: piece.clip.y + shift,
                        ..piece.clip
                    }
                    .into(),
                ),
                fragment_index: 0,
                fragment_count: 1,
                line_start: None,
                line_end: None,
            })?;
        }
    }
    // The boxes are placed for good, without the block offset of their
    // insets, and the content of the positioned boxes moves as their pieces
    // were placed.
    for &child in children {
        super::leave_block_offset_to_painter(tree, child);
    }
    for (node, moves) in &line_moves {
        column_lines::apply(tree, *node, moves);
    }
    Some(used_height)
}

#[cfg(test)]
mod tests;
