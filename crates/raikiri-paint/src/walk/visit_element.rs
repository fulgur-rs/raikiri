//! The visit of an element: where its box lies on the page, what it paints
//! of itself, and the frames that paint its children.

use super::child_frames::ChildOrigin;
use super::element_box::BoxInputs;
use super::frames::{PaintFrame, VisitFrame};
use super::paint_walk::PaintWalk;
use crate::text;
use anyrender::PaintScene;
use raikiri_dom::Node;
use raikiri_layout_geometry::BoxLayout;
use raikiri_traits::NodeId;

impl<'w, S: PaintScene> PaintWalk<'w, S> {
    /// Paint the element `node` of `visit`, laid out as `layout`, and push
    /// the frames that paint its children.
    pub(super) fn visit_element(&mut self, visit: &VisitFrame, node: &'w Node, layout: BoxLayout) {
        let node_id = visit.node_id;
        let (document, cascade) = (self.document, self.cascade);
        // An atomic inline an ellipsis removed from its line is not
        // painted (CSS Overflow 3 §5.1).
        if node.is_display_none() || node.is_hidden_by_text_overflow() {
            return;
        }
        let cv = &cascade.computed[node_id];
        let Some(element) = self.element_box(BoxInputs::of(visit), node, layout) else {
            return;
        };
        self.open_opacity_group(&element);
        let has_column_rules = document.column_rules(cascade, node_id).next().is_some();
        let mut before_advance = self.paint_box(&element, has_column_rules);
        let children = self.paint_children(&element);
        let offsets = self.generated_offsets(&element, &children);
        let overflow_clip_pushed = self.open_clips(&element);
        if let Some(advance) = self.paint_generated_overlay(&element, has_column_rules) {
            before_advance = advance;
        }
        let after_pushed = self.push_after(&element, before_advance);
        let paint_transform = self.scene.transform;
        // `text-decoration-line` is non-inherited at the computed-value
        // layer, but its originating line is propagated to descendants
        // by CSS Text Decoration. Keep that paint-only context separate
        // from `CascadeResult`'s inheritance result.
        let child_decorations = text::decorations_for_element(
            &visit.decorations,
            NodeId::new(node_id as u64),
            cv,
            element.child_shift_y,
        );
        let origin = ChildOrigin::new(&element, visit, &child_decorations);
        let lines_frame = self.paragraph_lines(&element, &origin, &children, paint_transform);
        let hold_lines = lines_frame.as_ref().is_some_and(|lines| lines.hold);
        let lines_frame = lines_frame.map(PaintFrame::Lines);
        let plan = self.child_plan(
            &element,
            visit,
            overflow_clip_pushed,
            hold_lines,
            before_advance,
            after_pushed,
        );
        // Reverse push makes the lowest stack level paint first.
        let frames = self.child_frames(&element, &origin, children, &offsets, &plan);
        self.place_frames(&element, &plan, frames, lines_frame);
    }
}
