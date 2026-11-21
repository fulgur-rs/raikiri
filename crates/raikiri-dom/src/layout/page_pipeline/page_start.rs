//! Whether a box begins its page: nothing renders on the page above it but the
//! margins, borders and padding of the plain blocks around it that it begins.

use super::sibling_adjacency::{FlowBox, flow_box};
use super::{
    CascadeResult, DisplayValue, Document, IfcBoxKind, NodeKind, current_abs_y,
    has_unusual_coordinates, is_floating_box_for_pagination, is_plain_block_box,
    layout_relative_offset_y,
};
use std::collections::HashMap;

/// How far apart two positions may be and still be the same edge.
const EDGE_TOLERANCE: f32 = 0.001;

/// Layout child positions reused within one pagination pass by the checks of
/// whether a box begins its page: a column piece that travels with the content
/// before it, and a forced break at the start of a page. The pass moves boxes
/// without changing child lists; every query still reads their current
/// positions, sizes and box classifications.
#[derive(Default)]
pub(super) struct ChildPositions {
    positions: HashMap<usize, HashMap<usize, usize>>,
    forced_first_children: HashMap<usize, Option<usize>>,
    #[cfg(test)]
    pub(super) indexed_children: usize,
}

impl ChildPositions {
    /// The first-child holder for a new forced boundary, without caching
    /// positions or margins that pagination can change.
    pub(super) fn forced_holder(
        &mut self,
        document: &Document,
        cascade: &CascadeResult,
        body_id: usize,
        node_id: usize,
    ) -> Option<(usize, f32)> {
        let (holder, lead) = super::leading_holder_with(
            document,
            cascade,
            body_id,
            node_id,
            true,
            |document, parent, child| {
                self.first_forced_child(document, cascade, parent) != Some(child)
            },
        );
        (holder != node_id
            && lead <= super::leading_space_between(document, cascade, node_id, holder) + 0.01)
            .then_some((holder, lead))
    }

    /// The child of `parent` that a forced break before it would carry to
    /// `parent`: its first child that renders in the flow, unless generated
    /// before content comes first.
    fn first_forced_child(
        &mut self,
        document: &Document,
        cascade: &CascadeResult,
        parent: usize,
    ) -> Option<usize> {
        *self.forced_first_children.entry(parent).or_insert_with(|| {
            // Generated before content is not an arena child. Even a
            // zero-height generated box precedes the forced boundary.
            if super::renders_before_pseudo(cascade, parent) {
                return None;
            }
            // An IFC keeps its block tokens outside the box child
            // list. Its source children still give the leading order.
            let node = &document.nodes[parent];
            let children = if node.layout_children().is_empty()
                && document
                    .ifc_layout_node(parent)
                    .is_some_and(|node| node.ifc.is_some())
            {
                &node.children
            } else {
                node.layout_children()
            };
            children.iter().copied().find(|&sibling| {
                super::sibling_renders(document, sibling)
                    && !(document.nodes[sibling].kind() == NodeKind::Element
                        && matches!(
                            document.nodes[sibling].position(),
                            super::PositionValue::Absolute | super::PositionValue::Fixed
                        ))
            })
        })
    }

    /// The box that `node_id` begins and that has a forced break before it
    /// which `node_id` holds (see [`Self::forced_holder`]): the break then
    /// sits before `node_id` too, between it and the box before it.
    pub(super) fn held_forced_child_break(
        &mut self,
        document: &Document,
        cascade: &CascadeResult,
        body_id: usize,
        node_id: usize,
    ) -> Option<usize> {
        let mut parent = node_id;
        while let Some(child) = self.first_forced_child(document, cascade, parent) {
            if super::forces_page_break(document, cascade, child, true) {
                return self
                    .forced_holder(document, cascade, body_id, child)
                    .filter(|&(holder, _)| holder == node_id)
                    .map(|_| child);
            }
            if document.nodes[child].kind() != NodeKind::Element
                || !(is_plain_block_box(document, child)
                    || document.nodes[child].display() == DisplayValue::Contents)
            {
                return None;
            }
            parent = child;
        }
        None
    }

    /// Whether the page that starts at `page_start` holds nothing above
    /// `node_id` but the leading edges of the plain blocks around it: the box
    /// is the first thing in each of them, and whatever comes before the
    /// outermost of them ends on an earlier page.
    ///
    /// A forced break before such a box describes the boundary that already
    /// opened the page. A page does not break between the start of a block and
    /// its first child (CSS Break 3 §4.1), so the break is carried to the block
    /// that the box begins and happens before that block's padding and border,
    /// which stay with the box on the new page.
    pub(super) fn begins_page_below_leading_edges(
        &mut self,
        document: &Document,
        cascade: &CascadeResult,
        body_id: usize,
        node_id: usize,
        page_start: f32,
    ) -> bool {
        self.begins_page_below(document, cascade, body_id, node_id, page_start, None)
    }

    /// Whether the page that starts at `page_start` holds nothing above
    /// `node_id` but the leading edges of the plain blocks around it, as
    /// [`Self::begins_page_below_leading_edges`] tells, where empty boxes at
    /// the very start of the page count as nothing either, and a flex or grid
    /// container starts with its only item when nothing of the container
    /// lies above it, as does a grid whose items stack in rows that size to
    /// their content with its first item. Moving such a box to the next page
    /// gains it no room: an
    /// unforced break there would leave nothing on this page but those empty
    /// boxes and the edges, so the page breaks between the box's lines
    /// instead.
    pub(super) fn gains_nothing_from_next_page(
        &mut self,
        document: &super::super::context::LayoutSession<'_>,
        cascade: &CascadeResult,
        body_id: usize,
        node_id: usize,
        page_start: f32,
    ) -> bool {
        let stacked = |grid, item| super::stacks_item_alone_in_its_rows(document, grid, item);
        self.begins_page_below(
            document,
            cascade,
            body_id,
            node_id,
            page_start,
            Some(&stacked),
        )
    }

    fn begins_page_below(
        &mut self,
        document: &Document,
        cascade: &CascadeResult,
        body_id: usize,
        node_id: usize,
        page_start: f32,
        past_empty_starts: Option<&dyn Fn(usize, usize) -> bool>,
    ) -> bool {
        let positions = &mut self.positions;
        #[cfg(test)]
        let indexed_children = &mut self.indexed_children;
        begins_page_with_positions(
            document,
            cascade,
            body_id,
            node_id,
            page_start,
            past_empty_starts,
            |parent, child| {
                positions
                    .entry(parent)
                    .or_insert_with(|| {
                        let children = document.nodes[parent].layout_children();
                        #[cfg(test)]
                        {
                            *indexed_children += children.len();
                            super::tests::PARENT_CHILD_STEPS
                                .with(|count| count.set(count.get() + children.len()));
                        }
                        children
                            .iter()
                            .enumerate()
                            .map(|(index, &id)| (id, index))
                            .collect()
                    })
                    .get(&child)
                    .copied()
            },
        )
    }
}

/// What an element has before one of its children.
enum Before {
    /// No box and no text renders before the child.
    Nothing,
    /// The nearest box before the child spans these positions.
    Ends { top: f32, bottom: f32 },
    /// Text, a float or generated content precedes the child.
    Other,
}

/// What the children of `parent` render before `child` on the page that starts
/// at `page_start`. White space, boxes that generate nothing, out-of-flow
/// boxes, and floats that start the page or lie on an earlier one are not
/// content, and a `display: contents` element counts as its children. (Its
/// generated text is inline content of the parent, which then holds its
/// blocks in boxes it does not list.) An empty inline box lays out a phantom
/// line (CSS 2.1 §9.4.2), whose anonymous block is a box of no height. With
/// `past_empty_starts`, neither are boxes of no height at the page start,
/// when the child itself starts at the page start. The nearest of the others
/// decides.
#[allow(clippy::too_many_arguments)]
fn before_child(
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    parent: usize,
    page_start: f32,
    child: usize,
    child_index: Option<usize>,
    past_empty_starts: bool,
) -> Before {
    let siblings = document.nodes[parent].layout_children();
    // A child that its parent does not list is held by a box the parent does
    // not show, such as the anonymous box around inline content, or sits
    // among the lines of the paragraph the parent lays out.
    let Some(index) = child_index else {
        return before_block_among_lines(document, cascade, body_id, parent, child)
            .unwrap_or(Before::Other);
    };
    // Empty boxes count as nothing only while the child starts right at the
    // page start: a margin they leave above it is room a break there would
    // truncate.
    let skip_empty = past_empty_starts
        && (current_abs_y(document, siblings[index]) - page_start).abs() <= EDGE_TOLERANCE;
    // The siblings, last first, with `display: contents` elements replaced
    // by their children: a stack of reverse iterators, as they can nest
    // without limit, which reads no further back than the nearest box that
    // decides.
    let mut levels = vec![siblings[..index].iter().rev()];
    // The top of the nearest phantom line: its anonymous block is a box of
    // no height, unless inline content before it shares the line.
    let mut phantom_top = None;
    while let Some(level) = levels.last_mut() {
        let Some(&sibling) = level.next() else {
            levels.pop();
            continue;
        };
        let node = &document.nodes[sibling];
        // Only boxes have a position to read.
        let span = || {
            let top = current_abs_y(document, sibling);
            (top, top + node.layout.size.height)
        };
        match flow_box(document, cascade, sibling) {
            FlowBox::Skip => {
                if is_floating_box_for_pagination(document, cascade, sibling) {
                    let (top, bottom) = span();
                    if bottom > page_start + EDGE_TOLERANCE
                        && (top - page_start).abs() > EDGE_TOLERANCE
                    {
                        return Before::Other;
                    }
                }
            }
            FlowBox::Phantom => {
                // A relative offset moves the inline box but not its line.
                let top = current_abs_y(document, sibling)
                    - layout_relative_offset_y(document, cascade, body_id, sibling);
                if !(skip_empty && (top - page_start).abs() <= EDGE_TOLERANCE) {
                    phantom_top = phantom_top.or(Some(top));
                }
            }
            FlowBox::Contents => levels.push(node.children.iter().rev()),
            FlowBox::Box => {
                let block_box = node.kind() == NodeKind::Element
                    && !node.display().is_inline_level()
                    && !has_unusual_coordinates(document, sibling);
                if !block_box {
                    return Before::Other;
                }
                if let Some(top) = phantom_top {
                    return Before::Ends { top, bottom: top };
                }
                let (top, bottom) = span();
                if skip_empty
                    && (top - page_start).abs() <= EDGE_TOLERANCE
                    && (bottom - page_start).abs() <= EDGE_TOLERANCE
                {
                    continue;
                }
                return Before::Ends { top, bottom };
            }
        }
    }
    phantom_top.map_or(Before::Nothing, |top| Before::Ends { top, bottom: top })
}

/// What the paragraph `root` lays out before its block child `block`, which
/// sits among the paragraph's lines rather than in a box the root lists: the
/// block right before it when no line comes between them, else the last line
/// before it, at their places in the flow. `None` when the block is not one of
/// the paragraph's, or when floats, generated blocks or column fragments of
/// the paragraph may lie between, which this does not follow.
fn before_block_among_lines(
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    root: usize,
    block: usize,
) -> Option<Before> {
    let ifc = document.ifc_layout_node(root)?.ifc.as_ref()?;
    let lines = ifc.lines.as_ref()?;
    let starts = &lines.block_line_starts;
    let mut entries = starts
        .iter()
        .enumerate()
        .filter(|(_, (child, _))| *child == block);
    let (index, &(_, after)) = entries.next()?;
    let plain = entries.next().is_none()
        && ifc.multicol_fragments.is_none()
        && lines.generated_block_starts.is_empty()
        && ifc.boxes.iter().all(|b| b.kind != IfcBoxKind::Float)
        && !generates_content_within(document, root);
    plain.then_some(())?;
    let flow_y = |id: usize| {
        current_abs_y(document, id) - layout_relative_offset_y(document, cascade, body_id, id)
    };
    if let Some(&(previous, previous_after)) = index.checked_sub(1).map(|i| &starts[i])
        && previous_after == after
    {
        let block_box =
            is_plain_block_box(document, previous) && !has_unusual_coordinates(document, previous);
        block_box.then_some(())?;
        let top = flow_y(previous);
        let bottom = top + document.nodes[previous].layout.size.height;
        return Some(Before::Ends { top, bottom });
    }
    let Some(line) = after.checked_sub(1) else {
        return Some(Before::Nothing);
    };
    let (top, bottom) = *super::ifc_root_line_bounds(document, root)?.get(line)?;
    let layout = &document.nodes[root].layout;
    let content_top = flow_y(root) + layout.border.top + layout.padding.top;
    Some(Before::Ends {
        top: content_top + top,
        bottom: content_top + bottom,
    })
}

/// Whether `root` or an element inside it generates `::before` or `::after`
/// content, which the paragraph lays out without a line or box of its own
/// that the lines record.
fn generates_content_within(document: &Document, root: usize) -> bool {
    let mut pending = vec![root];
    while let Some(id) = pending.pop() {
        let node = &document.nodes[id];
        if node.has_before_or_after_content {
            return true;
        }
        pending.extend(node.children.iter().copied());
    }
    false
}

/// Whether a page does not break between the start of `parent` and its first
/// child: a plain block of the normal flow, or a `display: contents` element
/// that is no box.
fn holds_first_child(document: &Document, cascade: &CascadeResult, parent: usize) -> bool {
    let node = &document.nodes[parent];
    node.display() == DisplayValue::Contents
        || (is_plain_block_box(document, parent)
            && !node.is_multicol_container()
            && !is_floating_box_for_pagination(document, cascade, parent)
            && !has_unusual_coordinates(document, parent))
}

/// Whether `child` is the only in-flow item of the flex or grid container
/// `parent`, or an item that `stacked` tells is alone in its grid rows, of a
/// box of the normal flow with no padding or border above the item, and
/// starts at the top of the container. A page that starts with the container
/// then starts with the item, and gains it no room by breaking before it. Padding or border
/// above the item is content the page can break after, unlike that of a
/// plain block, which goes with its first child.
fn starts_as_sole_item(
    document: &Document,
    cascade: &CascadeResult,
    parent: usize,
    child: usize,
    stacked: &dyn Fn(usize, usize) -> bool,
) -> bool {
    let node = &document.nodes[parent];
    let display = node.display();
    display.is_flex_or_grid_container()
        && !is_floating_box_for_pagination(document, cascade, parent)
        && !has_unusual_coordinates(document, parent)
        && node.layout.border.top + node.layout.padding.top <= EDGE_TOLERANCE
        && (node.layout_children().iter().all(|&sibling| {
            sibling == child
                || !super::sibling_renders(document, sibling)
                || matches!(
                    document.nodes[sibling].position(),
                    super::PositionValue::Absolute | super::PositionValue::Fixed
                )
        }) || stacked(parent, child))
        && (current_abs_y(document, child) - current_abs_y(document, parent)).abs()
            <= EDGE_TOLERANCE
}

fn begins_page_with_positions(
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
    page_start: f32,
    past_empty_starts: Option<&dyn Fn(usize, usize) -> bool>,
    mut child_position: impl FnMut(usize, usize) -> Option<usize>,
) -> bool {
    let mut child = node_id;
    !has_unusual_coordinates(document, node_id)
        && document
            .layout_ancestors(node_id)
            .find_map(|parent| {
                let verdict = match before_child(
                    document,
                    cascade,
                    body_id,
                    parent,
                    page_start,
                    child,
                    child_position(parent, child),
                    past_empty_starts.is_some(),
                ) {
                    // The box lies on an earlier page. An empty one at the
                    // page start is on this page, so the break comes after it.
                    // A box with a sibling of its own before it is left to
                    // the margin box test: only a wrapper carries the break.
                    Before::Ends { top, bottom } => Some(
                        child != node_id
                            && top < page_start - EDGE_TOLERANCE
                            && bottom <= page_start + EDGE_TOLERANCE,
                    ),
                    Before::Other => Some(false),
                    // Nothing precedes the outermost block in the document,
                    // or with `past_empty_starts` only empty boxes at the page
                    // start do. A box that is that block itself is the first
                    // of the flow, which no break opens a page for.
                    Before::Nothing if parent == body_id => Some(child != node_id),
                    Before::Nothing
                        if !(holds_first_child(document, cascade, parent)
                            || past_empty_starts.is_some_and(|stacked| {
                                starts_as_sole_item(document, cascade, parent, child, stacked)
                            })) =>
                    {
                        Some(false)
                    }
                    Before::Nothing => None,
                };
                child = parent;
                verdict
            })
            .unwrap_or(false)
}
