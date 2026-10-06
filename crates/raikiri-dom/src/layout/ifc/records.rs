//! The layout of the inline elements of a paragraph: the bounding box of
//! each element's border boxes on its lines.

use super::boxes::commit_child_layout;
use super::flow::FlowGeometry;
use super::inline_boxes::{BoxRect, InlineBoxPiece, forced_break_boxes, inline_box_pieces};
use super::root::IfcRoot;
use crate::Document;
use raikiri_traits::NodeKind;
use std::collections::HashMap;

fn union(a: BoxRect, b: BoxRect) -> BoxRect {
    let x0 = a.x.min(b.x);
    let y0 = a.y.min(b.y);
    let x1 = (a.x + a.width).max(b.x + b.width);
    let y1 = (a.y + a.height).max(b.y + b.height);
    BoxRect {
        x: x0,
        y: y0,
        width: x1 - x0,
        height: y1 - y0,
    }
}

/// Every in-document element below `root` that is not one of the paragraph's
/// boxes (nor inside one), in document order.
fn inline_elements(tree: &Document, root: usize, ifc: &IfcRoot) -> Vec<usize> {
    let mut out = Vec::new();
    let mut stack: Vec<usize> = tree.nodes[root].children.iter().rev().copied().collect();
    while let Some(id) = stack.pop() {
        let node = &tree.nodes[id];
        if !node.is_in_document()
            || node.kind() != NodeKind::Element
            || ifc.boxes.iter().any(|b| b.node == id)
        {
            continue;
        }
        out.push(id);
        stack.extend(node.children.iter().rev().copied());
    }
    out
}

/// Write the layout of every inline element of the paragraph rooted at `root`
/// from the pieces of its lines: the bounding box of the element's border
/// boxes, located relative to its DOM parent (or, for a child of the root, to
/// the root's border box). Elements without a piece get an empty layout.
///
/// The layout serves the readers that accumulate locations along the DOM
/// parents; painting reads the pieces of each line instead.
pub(crate) fn record_inline_boxes(
    tree: &mut Document,
    root: usize,
    ifc: &IfcRoot,
    lines: &[shodo::Line],
    geometry: &FlowGeometry,
) {
    let pieces = inline_box_pieces(lines, ifc.writing_mode, geometry.content_size);
    // Bounding box of each element's border boxes, in content-box coordinates.
    let mut boxes: HashMap<usize, BoxRect> = HashMap::new();
    for InlineBoxPiece {
        node, border_box, ..
    } in &pieces
    {
        boxes
            .entry(*node)
            .and_modify(|current| *current = union(*current, *border_box))
            .or_insert(*border_box);
    }
    // A `<br>` has no box on its line: it is placed where the line it ends is.
    for (node, rect) in forced_break_boxes(lines, ifc.writing_mode, geometry.content_size) {
        boxes.entry(node).or_insert(rect);
    }
    for element in inline_elements(tree, root, ifc) {
        let rect = boxes.get(&element).copied();
        // The nearest ancestor with a box of its own: another inline element
        // of the paragraph, or the root.
        let mut parent = tree.parent_of(element);
        while let Some(id) = parent {
            if id == root || boxes.contains_key(&id) {
                break;
            }
            parent = tree.parent_of(id);
        }
        let origin = match parent.and_then(|id| boxes.get(&id)) {
            Some(parent_box) => (parent_box.x, parent_box.y),
            // A child of the root sits in the root's content box, which starts
            // inside the root's border box by its left and top border and
            // padding (`FlowGeometry::edges` is the physical left and right).
            None => (-geometry.edges.0, -geometry.top_edge),
        };
        let output = taffy::LayoutOutput::from_outer_size(taffy::Size {
            width: rect.map_or(0.0, |r| r.width),
            height: rect.map_or(0.0, |r| r.height),
        });
        // A relative offset moves the element and, through this location,
        // everything inside it; a child does not add its parent's offset.
        let own = ifc
            .offsets
            .iter()
            .find(|(id, _)| *id == element)
            .map_or((0.0, 0.0), |(_, offset)| *offset);
        let location = taffy::Point {
            x: rect.map_or(0.0, |r| r.x - origin.0) + own.0,
            y: rect.map_or(0.0, |r| r.y - origin.1) + own.1,
        };
        commit_child_layout(
            tree,
            element,
            &output,
            location,
            geometry.content_size.width,
        );
    }
}

#[cfg(test)]
mod tests;
