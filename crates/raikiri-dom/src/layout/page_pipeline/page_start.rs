//! Whether a box begins its page: nothing renders on the page above it but the
//! margins, borders and padding of the plain blocks around it that it begins.

use super::sibling_adjacency::{FlowBox, flow_box};
use super::{
    CascadeResult, DisplayValue, Document, NodeKind, current_abs_y, has_unusual_coordinates,
    is_floating_box_for_pagination, is_plain_block_box,
};

/// How far apart two positions may be and still be the same edge.
const EDGE_TOLERANCE: f32 = 0.001;

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
/// content. The nearest of the others decides.
fn before_child(
    document: &Document,
    cascade: &CascadeResult,
    parent: usize,
    child: usize,
    page_start: f32,
) -> Before {
    let siblings = document.nodes[parent].layout_children();
    // A child that its parent does not list is held by a box the parent does
    // not show, such as the anonymous box around inline content.
    let Some(index) = siblings.iter().position(|&sibling| sibling == child) else {
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
            FlowBox::Contents => return Before::Other,
            FlowBox::Box => {
                let block_box = document.nodes[sibling].kind() == NodeKind::Element
                    && !document.nodes[sibling].display().is_inline_level()
                    && !has_unusual_coordinates(document, sibling);
                return if block_box {
                    let (top, bottom) = span();
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

/// Whether the page that starts at `page_start` holds nothing above
/// `node_id` but the leading edges of the plain blocks around it: the box is
/// the first thing in each of them, and whatever comes before the outermost
/// of them ends on an earlier page.
///
/// A forced break before such a box describes the boundary that already
/// opened the page. A page does not break between the start of a block and its
/// first child (CSS Break 3 §4.1), so the break is carried to the block that
/// the box begins and happens before that block's padding and border, which
/// stay with the box on the new page.
pub(super) fn begins_page_below_leading_edges(
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
    page_start: f32,
) -> bool {
    let mut child = node_id;
    !has_unusual_coordinates(document, node_id)
        && document
            .layout_ancestors(node_id)
            .find_map(|parent| {
                let verdict = match before_child(document, cascade, parent, child, page_start) {
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
                    // Nothing precedes the outermost block in the document. A
                    // box that is that block itself is the first of the flow,
                    // which no break opens a page for.
                    Before::Nothing if parent == body_id => Some(child != node_id),
                    Before::Nothing if !holds_first_child(document, cascade, parent) => Some(false),
                    Before::Nothing => None,
                };
                child = parent;
                verdict
            })
            .unwrap_or(false)
}
