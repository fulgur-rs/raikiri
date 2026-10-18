//! The visit of a node: where its box lies on the page, what it paints of
//! itself, and the frames that paint its children.

use super::frames::{PaintFrame, VisitFrame};
use super::page_geometry::PageGeometry;
use super::paint_walk::PaintWalk;
use super::{box_intersects_page, fixed_background_viewport};
use anyrender::PaintScene;
use raikiri_dom::FragmentRect;
use raikiri_traits::NodeKind;

impl<'w, S: PaintScene> PaintWalk<'w, S> {
    /// Visit the node of `visit`: paint it and push the frames of its
    /// children, or skip it and its subtree.
    pub(super) fn visit(&mut self, mut visit: VisitFrame) {
        let (document, cascade, page_box, active_page_name) = (
            self.document,
            self.cascade,
            self.page_box,
            self.active_page_name,
        );
        let (image_cache, pixel_source, custom_highlights) =
            (self.image_cache, self.pixel_source, self.custom_highlights);
        let (content_origin_y, root_transform, layout_fragments) = (
            self.content_origin_y,
            self.root_transform,
            self.layout_fragments,
        );
        let geometry = self.geometry;
        let PageGeometry {
            margins: _,
            insets: _,
            content_width: _,
            content_height: _,
            page_area,
            fixed_rtl: _,
            fixed_content_width: _,
            pages_follow_x_axis: _,
            page_top,
            page_bottom,
            page_offset_x,
            page_offset_y,
            fixed_flow_shift_x: _,
            fixed_origin_y: _,
            page_clip_transform,
            page_content_clip: _,
            flow_band: _,
            page_lines,
        } = geometry;
        let named_page_matches = |node_id: usize| {
            raikiri_dom::paint_rules::named_page_matches(
                document,
                cascade,
                node_id,
                active_page_name,
            )
        };
        let text_page_clip =
            |inside_fixed: bool| geometry.text_page_clip(cascade, page_box, inside_fixed);
        if let Some(shift) =
            document.repeated_table_header_root_shift(visit.node_id, content_origin_y)
        {
            let Some(shift) = shift else {
                return;
            };
            visit.parent_abs_y += shift;
        }
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
            fragmentainer,
            fragment_rect,
            fragment_clip: _,
            is_fragment_visit,
            inside_fixed,
            inside_fixed_containing_block,
            ref decorations,
        } = visit;
        let Some(node) = document.get_node(node_id) else {
            return;
        };
        if let Some(fragment_ids) = self
            .fragments_by_parent_and_node
            .get(&(fragment_id, node_id))
        {
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
            return;
        }
        if self.fragmented_nodes.contains(&node_id) && !is_fragment_visit {
            return;
        }
        // Skip template descendants and future inert subtrees consistently.
        // Use an explicit gate independent of any UA CSS display:none rule.
        if !node.is_in_document() {
            return;
        }
        let mut node_layout = node.unrounded_layout;
        if let Some(rect) = fragment_rect {
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
            NodeKind::Element => {
                self.visit_element(&visit, node, node_layout);
            }
            NodeKind::Text => {
                let scene = &mut *self.scene;
                let trace = &mut self.trace;
                let warnings = &mut *self.warnings;
                let layout = node_layout;
                let abs_x = parent_abs_x + layout.location.x;
                let abs_y = parent_abs_y + layout.location.y;
                if named_page_matches(node_id)
                    && (inside_fixed
                        || box_intersects_page(abs_y, layout.size.height, page_top, page_bottom))
                {
                    let text_clip = text_page_clip(inside_fixed);
                    if let Some(clip) = &text_clip {
                        scene.scene.push_clip_layer(page_clip_transform, clip);
                    }
                    if node.is_ifc_root() {
                        // A text node laid out as an anonymous flex or grid
                        // item is a paragraph of its own: its lines start at
                        // its own box, which has no edges.
                        if let Some(t) = trace.as_deref_mut() {
                            t.push(crate::PaintTraceEvent::Text(node_id));
                        }
                        let paint_transform = scene.transform;
                        crate::ifc_text::draw_ifc_lines_with_resources(
                            image_cache,
                            scene,
                            document,
                            cascade,
                            node_id,
                            crate::ifc_text::IfcPosition {
                                x: abs_x + page_offset_x + transform_x,
                                y: abs_y + page_offset_y + transform_y,
                                shift_y,
                            },
                            decorations,
                            fragmentainer,
                            custom_highlights,
                            pixel_source,
                            warnings,
                            page_box,
                            page_lines
                                .filter(|_| !inside_fixed)
                                .map(|lines| lines.translated(transform_y + paint_offset_y)),
                            paint_transform,
                            fixed_background_viewport(
                                page_area,
                                root_transform,
                                inside_fixed_containing_block,
                            ),
                        );
                    }
                    // Any other text node lies outside every paragraph and
                    // has no lines to draw.
                    if text_clip.is_some() {
                        scene.pop_layer();
                    }
                }
            }
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
}
