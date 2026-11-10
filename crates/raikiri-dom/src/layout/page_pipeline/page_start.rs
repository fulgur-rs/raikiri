//! Whether a box begins its page: nothing renders on the page above it but the
//! margins, borders and padding of the plain blocks around it that it begins.

use super::sibling_adjacency::{FlowBox, flow_box};
use super::{
    CascadeResult, DisplayValue, Document, NodeKind, current_abs_y, has_unusual_coordinates,
    is_floating_box_for_pagination, is_plain_block_box,
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
                let first = self.forced_first_children.entry(parent).or_insert_with(|| {
                    // Generated before content is not an arena child. Even a
                    // zero-height generated box precedes the forced boundary.
                    let before = cascade.pseudo.get(&(
                        raikiri_style::StyleNodeId::new(parent as u64),
                        raikiri_style::PseudoElem::Before,
                    ));
                    if before.is_some_and(|style| {
                        style.display != DisplayValue::None
                            && !matches!(
                                style.position,
                                super::PositionValue::Absolute | super::PositionValue::Fixed
                            )
                            && !style.content.is_empty()
                            && !style.content.iter().any(|part| {
                                matches!(part, raikiri_style::property::ContentComponent::None)
                            })
                    }) {
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
                });
                *first != Some(child)
            },
        );
        (holder != node_id
            && lead <= super::leading_space_between(document, cascade, node_id, holder) + 0.01)
            .then_some((holder, lead))
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
        self.begins_page_below(document, cascade, body_id, node_id, page_start, false)
    }

    /// Whether the page that starts at `page_start` holds nothing above
    /// `node_id` but the leading edges of the plain blocks around it, as
    /// [`Self::begins_page_below_leading_edges`] tells, where empty boxes at
    /// the very start of the page count as nothing either, and a flex or grid
    /// container starts with its only item when nothing of the container
    /// lies above it. Moving such a box to the next page gains it no room: an
    /// unforced break there would leave nothing on this page but those empty
    /// boxes and the edges, so the page breaks between the box's lines
    /// instead.
    pub(super) fn gains_nothing_from_next_page(
        &mut self,
        document: &Document,
        cascade: &CascadeResult,
        body_id: usize,
        node_id: usize,
        page_start: f32,
    ) -> bool {
        self.begins_page_below(document, cascade, body_id, node_id, page_start, true)
    }

    fn begins_page_below(
        &mut self,
        document: &Document,
        cascade: &CascadeResult,
        body_id: usize,
        node_id: usize,
        page_start: f32,
        past_empty_starts: bool,
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
/// content. With `past_empty_starts`, neither are block boxes of no height at
/// the page start and elements without a box that hold nothing. The nearest of
/// the others decides.
fn before_child(
    document: &Document,
    cascade: &CascadeResult,
    parent: usize,
    page_start: f32,
    child_index: Option<usize>,
    past_empty_starts: bool,
) -> Before {
    let siblings = document.nodes[parent].layout_children();
    // A child that its parent does not list is held by a box the parent does
    // not show, such as the anonymous box around inline content.
    let Some(index) = child_index else {
        return Before::Other;
    };
    for &sibling in siblings[..index].iter().rev() {
        // Only boxes have a position to read.
        let span = || {
            let top = current_abs_y(document, sibling);
            (top, top + document.nodes[sibling].layout.size.height)
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
            // An element without a box that holds nothing that renders is
            // as empty as a box of no height.
            FlowBox::Contents
                if past_empty_starts
                    && document.nodes[sibling]
                        .children
                        .iter()
                        .all(|&child| !super::sibling_renders(document, child)) => {}
            FlowBox::Contents => return Before::Other,
            FlowBox::Box => {
                let block_box = document.nodes[sibling].kind() == NodeKind::Element
                    && !document.nodes[sibling].display().is_inline_level()
                    && !has_unusual_coordinates(document, sibling);
                return if block_box {
                    let (top, bottom) = span();
                    if past_empty_starts
                        && (top - page_start).abs() <= EDGE_TOLERANCE
                        && (bottom - page_start).abs() <= EDGE_TOLERANCE
                    {
                        continue;
                    }
                    Before::Ends { top, bottom }
                } else {
                    Before::Other
                };
            }
        }
    }
    Before::Nothing
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
/// `parent`, a box of the normal flow with no padding or border above the
/// item, and starts at the top of the container. A page that starts with the container then starts with
/// the item, and gains it no room by breaking before it. Padding or border
/// above the item is content the page can break after, unlike that of a
/// plain block, which goes with its first child.
fn starts_as_sole_item(
    document: &Document,
    cascade: &CascadeResult,
    parent: usize,
    child: usize,
) -> bool {
    let node = &document.nodes[parent];
    let display = node.display();
    display.is_flex_or_grid_container()
        && !is_floating_box_for_pagination(document, cascade, parent)
        && !has_unusual_coordinates(document, parent)
        && node.layout.border.top + node.layout.padding.top <= EDGE_TOLERANCE
        && node.layout_children().iter().all(|&sibling| {
            sibling == child
                || !super::sibling_renders(document, sibling)
                || matches!(
                    document.nodes[sibling].position(),
                    super::PositionValue::Absolute | super::PositionValue::Fixed
                )
        })
        && (current_abs_y(document, child) - current_abs_y(document, parent)).abs()
            <= EDGE_TOLERANCE
}

fn begins_page_with_positions(
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
    page_start: f32,
    past_empty_starts: bool,
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
                    parent,
                    page_start,
                    child_position(parent, child),
                    past_empty_starts,
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
                        if !holds_first_child(document, cascade, parent)
                            && !(past_empty_starts
                                && starts_as_sole_item(document, cascade, parent, child)) =>
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
