//! The visit of a text node: the lines of a paragraph that is a text node of
//! its own.

use super::frames::VisitFrame;
use super::paint_walk::PaintWalk;
use super::{box_intersects_page, fixed_background_viewport};
use anyrender::PaintScene;
use raikiri_dom::Node;

impl<'w, S: PaintScene> PaintWalk<'w, S> {
    /// Paint the lines of the text node `node` of `visit`, laid out as
    /// `layout`, when it is a paragraph that belongs on this page.
    pub(super) fn visit_text(&mut self, visit: &VisitFrame, node: &'w Node, layout: taffy::Layout) {
        let VisitFrame {
            node_id,
            parent_abs_x,
            parent_abs_y,
            shift_y,
            transform_x,
            transform_y,
            paint_offset_y,
            fragmentainer,
            inside_fixed,
            inside_fixed_containing_block,
            ref decorations,
            ..
        } = *visit;
        let (document, cascade, page_box) = (self.document, self.cascade, self.page_box);
        let (image_cache, pixel_source, custom_highlights) =
            (self.image_cache, self.pixel_source, self.custom_highlights);
        let geometry = self.geometry;
        let scene = &mut *self.scene;
        let trace = &mut self.trace;
        let warnings = &mut *self.warnings;
        let abs_x = parent_abs_x + layout.location.x;
        let abs_y = parent_abs_y + layout.location.y;
        if raikiri_dom::paint_rules::named_page_matches(
            document,
            cascade,
            node_id,
            self.active_page_name,
        ) && (inside_fixed
            || box_intersects_page(
                abs_y,
                layout.size.height,
                geometry.page_top,
                geometry.page_bottom,
            ))
        {
            let text_clip = geometry.text_page_clip(cascade, page_box, inside_fixed);
            if let Some(clip) = &text_clip {
                scene
                    .scene
                    .push_clip_layer(geometry.page_clip_transform, clip);
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
                        x: abs_x + geometry.page_offset_x + transform_x,
                        y: abs_y + geometry.page_offset_y + transform_y,
                        shift_y,
                    },
                    decorations,
                    fragmentainer,
                    custom_highlights,
                    pixel_source,
                    warnings,
                    page_box,
                    geometry
                        .page_lines
                        .filter(|_| !inside_fixed)
                        .map(|lines| lines.translated(transform_y + paint_offset_y)),
                    paint_transform,
                    fixed_background_viewport(
                        geometry.page_area,
                        self.root_transform,
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
}
