//! The visit of a node: whether it paints at all, how many fragments it is
//! painted in, and which kind of node it is.

use super::frames::{PaintFrame, VisitFrame};
use super::paint_walk::PaintWalk;
use anyrender::PaintScene;
use raikiri_dom::{FragmentRect, Node};
use raikiri_traits::NodeKind;

impl<'w, S: PaintScene> PaintWalk<'w, S> {
    /// Visit the node of `visit`: paint it and push the frames of its
    /// children, or skip it and its subtree.
    pub(super) fn visit(&mut self, mut visit: VisitFrame) {
        let document = self.document;
        if let Some(shift) =
            document.repeated_table_header_root_shift(visit.node_id, self.content_origin_y)
        {
            let Some(shift) = shift else {
                return;
            };
            visit.parent_abs_y += shift;
        }
        let Some(node) = document.get_node(visit.node_id) else {
            return;
        };
        if self.push_fragment_visits(&visit, node) {
            return;
        }
        if self.fragmented_nodes.contains(&visit.node_id) && !visit.is_fragment_visit {
            return;
        }
        // Skip template descendants and future inert subtrees consistently.
        // Use an explicit gate independent of any UA CSS display:none rule.
        if !node.is_in_document() {
            return;
        }
        let mut node_layout = node.unrounded_layout;
        if let Some(rect) = visit.fragment_rect {
            node_layout.size.width = rect.width;
            node_layout.size.height = rect.height;
        }
        // Do not paint subtrees of HTML hidden elements (metadata, raw text,
        // or fallback parentheses for ruby).
        // Treat `Node::is_non_rendered_html_element` match arms as the
        // single source of truth for the current set of tags.
        // Author / user CSS can override UA CSS `display: none`, so painting
        // fails closed with a cascade-independent defense-in-depth gate
        // (HTML LS §15.3.1 "Hidden elements":
        // https://html.spec.whatwg.org/multipage/rendering.html#hidden-elements).
        // Namespace checks exclude same-named SVG / MathML elements.
        // <template> is doubly gated by is_in_document().
        if node.is_non_rendered_html_element() {
            return;
        }
        match node.kind() {
            NodeKind::Element => self.visit_element(&visit, node, node_layout),
            NodeKind::Text => self.visit_text(&visit, node, node_layout),
            // The walk starts at the document element. A Document node may
            // have children (unattached `<html>`), but they are not handled
            // yet. Skip the subtree defensively.
            //
            // NodeKind is #[non_exhaustive]: `Comment` / `ProcessingInstruction` /
            // `DocumentFragment` arrives here (not paintable). In practice,
            // mark_in_document_flags clears IS_IN_DOCUMENT for Comment/PI,
            // so the earlier is_in_document() gate should filter those
            // before this walker reaches them. Keep this kind gate as a
            // second defense: if both gates fail, skip the whole subtree.
            // Apply the same behavior to future CDATA / DocumentType kinds.
            // Retain the defensive fallback for unknown node kinds.
            _ => {}
        }
    }

    /// Push a visit of `node` for each fragment that layout broke it into,
    /// and say whether it was broken.
    fn push_fragment_visits(&mut self, visit: &VisitFrame, node: &Node) -> bool {
        let VisitFrame {
            node_id,
            parent_abs_x,
            parent_abs_y,
            parent_font_size,
            shift_y,
            transform_x,
            transform_y,
            paint_offset_y,
            fragment_clip_height,
            column_clip_height,
            fragment_id,
            fragment_origin_x,
            fragment_origin_y,
            fragment_rect: _,
            fragment_clip: _,
            fragmentainer: _,
            is_fragment_visit: _,
            inside_fixed,
            inside_fixed_containing_block,
            ref decorations,
        } = *visit;
        let layout_fragments = self.layout_fragments;
        let Some(fragment_ids) = self
            .fragments_by_parent_and_node
            .get(&(fragment_id, node_id))
        else {
            return false;
        };
        for &current_fragment_id in fragment_ids.iter().rev() {
            let fragment = layout_fragments[current_fragment_id];
            let (node_abs_x, node_abs_y) = fragment.paint_origin(
                (parent_abs_x, parent_abs_y),
                (
                    node.unrounded_layout.location.x,
                    node.unrounded_layout.location.y,
                ),
                (fragment_origin_x, fragment_origin_y),
            );
            let fragment_clip = fragment.fragmentainer_clip.map(|clip| FragmentRect {
                x: clip.x - fragment.rect.x,
                y: clip.y - fragment.rect.y,
                width: clip.width,
                height: clip.height,
            });
            self.stack.push(PaintFrame::Visit(VisitFrame {
                node_id,
                parent_abs_x: node_abs_x - node.unrounded_layout.location.x,
                parent_abs_y: node_abs_y - node.unrounded_layout.location.y,
                parent_font_size,
                shift_y,
                transform_x,
                transform_y,
                paint_offset_y,
                fragment_clip_height,
                column_clip_height,
                fragment_id: Some(current_fragment_id),
                fragment_origin_x: node_abs_x,
                fragment_origin_y: node_abs_y,
                fragmentainer: Some(fragment.fragmentainer),
                fragment_rect: Some(fragment.rect),
                fragment_clip,
                is_fragment_visit: true,
                inside_fixed,
                inside_fixed_containing_block,
                decorations: decorations.clone(),
            }));
        }
        true
    }
}
