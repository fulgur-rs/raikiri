//! What an element hands to its children: the origin they are placed from,
//! the offsets that generated text gives them, and the frames that paint
//! them and the lines of its own paragraph.

use super::element_box::ElementBox;
use super::frames::{AnonymousParagraphFrame, LayerClose, LinesFrame, PaintFrame, VisitFrame};
use super::paint_walk::PaintWalk;
use super::{
    box_intersects_page, first_text_descendant, fixed_background_viewport, generated_flow_height,
    generated_pseudo_text_advance, generated_pseudo_text_height, ifc_paint_extent,
};
use crate::text;
use anyrender::PaintScene;
use kurbo::Affine;
use raikiri_dom::Node;
use raikiri_style::property::{ColumnCountValue, DisplayValue, PositionValue, ZIndexValue};
use raikiri_traits::NodeKind;

/// Where the children of an element are placed from, and what they inherit
/// of its visit.
pub(super) struct ChildOrigin<'a> {
    pub(super) parent_x: f32,
    pub(super) parent_y: f32,
    pub(super) font_size: f32,
    pub(super) shift_y: f32,
    pub(super) transform_x: f32,
    pub(super) transform_y: f32,
    /// How far the relative offsets that paint applies to the element and
    /// its ancestors move `parent_y` from where layout placed it.
    pub(super) paint_offset_y: f32,
    pub(super) inside_fixed: bool,
    pub(super) inside_fixed_containing_block: bool,
    pub(super) fragment_id: Option<usize>,
    pub(super) fragment_origin_x: f32,
    pub(super) fragment_origin_y: f32,
    pub(super) fragmentainer: Option<usize>,
    pub(super) decorations: &'a text::DecorationContext,
}

impl<'a> ChildOrigin<'a> {
    /// Where the children of `element`, visited as `visit`, are placed from.
    /// `decorations` is what they inherit of the text decorations.
    pub(super) fn new(
        element: &ElementBox<'_>,
        visit: &VisitFrame,
        decorations: &'a text::DecorationContext,
    ) -> Self {
        // Relative insets are already present in `layout.location`
        // except where the placement of the box applies them by hand, as
        // `pos_dx` and `pos_dy`.
        let parent_x = element.abs_x + element.pos_dx + element.fixed_dx;
        let parent_y = element.abs_y + element.pos_dy + element.fixed_dy;
        let paint_offset_y = visit.paint_offset_y + element.pos_dy + element.fixed_dy;
        // The descendants of a box that is placed in a column are
        // placed from the origin of its fragment, which stands where
        // layout left it. The offset that paint adds to the box moves
        // them as well, as it does the descendants that follow the
        // parent chain.
        let (fragment_origin_x, fragment_origin_y) = if element.placed_in_column {
            (
                visit.fragment_origin_x + element.pos_dx,
                visit.fragment_origin_y + element.pos_dy,
            )
        } else {
            (visit.fragment_origin_x, visit.fragment_origin_y)
        };
        Self {
            parent_x,
            parent_y,
            font_size: element.cv.font_size.px(),
            shift_y: element.child_shift_y,
            transform_x: element.child_transform_x,
            transform_y: element.child_transform_y,
            paint_offset_y,
            // A paragraph inside a fixed box, or a fixed box that is a
            // paragraph itself, repeats on every page and is not clipped
            // like flowed text.
            inside_fixed: visit.inside_fixed || element.fixed_in_viewport,
            inside_fixed_containing_block: visit.inside_fixed_containing_block
                || element.establishes_fixed_containing_block,
            fragment_id: visit.fragment_id,
            fragment_origin_x,
            fragment_origin_y,
            fragmentainer: visit.fragmentainer,
            decorations,
        }
    }
}

/// How the children of an element take part in the paint of its layer.
pub(super) struct ChildPlan {
    /// The `fragment_clip_height` of the visit of the element.
    pub(super) fragment_clip_height: Option<f32>,
    /// The height the columns of the element, or of the container it sits
    /// in as a `display: contents` element, clip its children to.
    pub(super) multicol_clip_height: Option<f32>,
    /// How far the generated text before the element pushes its direct text
    /// along the line.
    pub(super) before_advance: f32,
    /// Whether the inline content of the element waits for the end of its
    /// layer.
    pub(super) defers: bool,
    /// Whether the lines of the element are held for the boxes beside them.
    pub(super) hold_lines: bool,
    /// Whether the paragraph of the element has content positioned below its
    /// blocks.
    pub(super) negative_content: bool,
    /// How the layer of the element ends when it merges its inline content
    /// into the enclosing layer, inside its overflow clip.
    pub(super) merge: Option<LayerClose>,
    /// Whether the frame of the `::after` content of the element was pushed.
    pub(super) after_pushed: bool,
}

/// The frames that paint the children of an element, each list last child
/// first: the children in place, those below the lines of the paragraph,
/// and the blocks the paragraph paints before its lines.
pub(super) struct ChildFrames<'w> {
    pub(super) children: Vec<PaintFrame<'w>>,
    pub(super) below_lines: Vec<PaintFrame<'w>>,
    pub(super) blocks: Vec<PaintFrame<'w>>,
}

/// How far the generated inline text before each child pushes it along the
/// line, as `(child, offset)` pairs, from the `(child, is_text, advance)`
/// triples of the children in source order.
///
/// A child that carries generated text starts where the ones before it end.
/// Each text child between two such children adds one collapsed space to the
/// line.
fn inline_flow_offsets(
    child_generated_advances: &[(usize, bool, f32)],
    collapsed_space: f32,
) -> Vec<(usize, f32)> {
    let mut flow_advance = 0.0;
    let mut saw_generated_inline = false;
    let mut offsets = Vec::new();
    for (index, (child, is_text, advance)) in child_generated_advances.iter().enumerate() {
        if *is_text {
            let has_following_generated_inline = child_generated_advances
                .iter()
                .skip(index + 1)
                .any(|(_, _, following)| *following > 0.0);
            if saw_generated_inline && has_following_generated_inline {
                flow_advance += collapsed_space;
            }
        } else if *advance > 0.0 {
            offsets.push((*child, flow_advance));
            flow_advance += *advance;
            saw_generated_inline = true;
        }
    }
    offsets
}

/// The shifts that generated text puts on the children of an element, as
/// `(child, shift)` pairs.
pub(super) struct GeneratedOffsets {
    /// How far the generated text before a child pushes it along the line.
    pub(super) inline: Vec<(usize, f32)>,
    /// How far the generated text above a child pushes it down.
    pub(super) vertical: Vec<(usize, f32)>,
}

impl<'w, S: PaintScene> PaintWalk<'w, S> {
    /// The shifts that the generated text of `children` puts on them: the
    /// inline offsets of the children that carry generated text, and the
    /// vertical offsets of the children of a block container.
    pub(super) fn generated_offsets(
        &mut self,
        element: &ElementBox<'w>,
        children: &[usize],
    ) -> GeneratedOffsets {
        GeneratedOffsets {
            inline: self.inline_generated_offsets(element, children),
            vertical: self.vertical_generated_offsets(element, children),
        }
    }

    /// How far the generated text before each of `children` pushes it along
    /// the line, as `(child, offset)` pairs; empty when no child carries
    /// generated text.
    fn inline_generated_offsets(
        &self,
        element: &ElementBox<'w>,
        children: &[usize],
    ) -> Vec<(usize, f32)> {
        let child_generated_advances = self.child_generated_advances(children);
        if child_generated_advances
            .iter()
            .any(|(_, _, advance)| *advance > 0.0)
        {
            let family = element
                .cv
                .font_family
                .first()
                .cloned()
                .unwrap_or_else(|| raikiri_style::FontFamilyName::generic("serif"));
            let collapsed_space = text::measure_margin_text_advance(
                self.document,
                " ",
                element.cv.font_size.px(),
                std::slice::from_ref(&family),
            );
            inline_flow_offsets(&child_generated_advances, collapsed_space)
        } else {
            Vec::new()
        }
    }

    /// For each of `children`, whether it is a text node and how far the
    /// generated text of its `::before` and `::after` advances along the line.
    ///
    /// The current Taffy bridge treats inline children as zero-sized block
    /// items. Keep that layout authority, but provide the measured paint-time
    /// inline advance for generated pseudo text so a sequence such as
    /// `span::before { content: counter(c) }` remains in source order. This is
    /// deliberately limited to siblings that actually carry generated content;
    /// ordinary inline layout remains unchanged until a full IFC lands.
    fn child_generated_advances(&self, children: &[usize]) -> Vec<(usize, bool, f32)> {
        let (document, cascade, counter_snapshots) =
            (self.document, self.cascade, self.counter_snapshots);
        children
            .iter()
            .map(|&child| {
                let is_text = document
                    .get_node(child)
                    .is_some_and(|child_node| child_node.kind() == NodeKind::Text);
                let advance = if !is_text
                    && cascade
                        .computed
                        .get(child)
                        .is_some_and(|child_cv| child_cv.display.is_inline_flow())
                {
                    generated_pseudo_text_advance(
                        document,
                        cascade,
                        child,
                        raikiri_style::PseudoElem::Before,
                        counter_snapshots,
                    ) + generated_pseudo_text_advance(
                        document,
                        cascade,
                        child,
                        raikiri_style::PseudoElem::After,
                        counter_snapshots,
                    )
                } else {
                    0.0
                };
                (child, is_text, advance)
            })
            .collect()
    }

    /// How far the generated text above each of `children` pushes it down, as
    /// `(child, offset)` pairs; empty unless `element` is a block container.
    fn vertical_generated_offsets(
        &mut self,
        element: &ElementBox<'w>,
        children: &[usize],
    ) -> Vec<(usize, f32)> {
        let (document, cascade, counter_snapshots) =
            (self.document, self.cascade, self.counter_snapshots);
        let (cv, node_id) = (element.cv, element.node_id);
        if cv.display.is_inline_flow() {
            return Vec::new();
        }
        let mut flow_extra = generated_pseudo_text_height(
            document,
            cascade,
            node_id,
            raikiri_style::PseudoElem::Before,
            counter_snapshots,
        );
        let mut offsets = Vec::new();
        for &child in children {
            offsets.push((child, flow_extra));
            if let Some(child_node) = document.get_node(child) {
                let child_cv = &cascade.computed[child];
                let child_is_normal_flow = matches!(
                    child_cv.position,
                    PositionValue::Static | PositionValue::Relative | PositionValue::Sticky
                );
                if child_node.kind() != NodeKind::Text
                    && !child_cv.display.is_inline_flow()
                    && child_is_normal_flow
                {
                    let required_height = generated_flow_height(
                        document,
                        cascade,
                        child,
                        counter_snapshots,
                        &mut self.generated_flow_heights,
                    );
                    flow_extra +=
                        (required_height - child_node.unrounded_layout.size.height).max(0.0);
                }
            }
        }
        offsets
    }

    /// The frame that draws the lines of the paragraph that `element` is,
    /// when they paint on this page.
    pub(super) fn paragraph_lines(
        &self,
        element: &ElementBox<'w>,
        origin: &ChildOrigin<'_>,
        children: &[usize],
        paint_transform: Affine,
    ) -> Option<LinesFrame> {
        let ElementBox {
            node,
            node_id,
            layout,
            abs_y,
            ..
        } = *element;
        let (document, cascade) = (self.document, self.cascade);
        let geometry = self.geometry;
        // A paragraph inside a fixed box, or a fixed box that is a
        // paragraph itself, repeats on every page and is not clipped
        // like flowed text, as the text branch below does.
        let ifc_inside_fixed = origin.inside_fixed;
        // The page a text node belongs to is looked up on the text node
        // itself; the block's own page value can differ (an absolutely
        // positioned box named for a page), so ask as the first text
        // child would.
        let ifc_page_probe = if node.is_ifc_root() {
            first_text_descendant(document, node_id).unwrap_or(node_id)
        } else {
            node_id
        };
        if node.is_ifc_root()
            && raikiri_dom::paint_rules::named_page_matches(
                document,
                cascade,
                ifc_page_probe,
                self.active_page_name,
            )
            && (ifc_inside_fixed
                || box_intersects_page(
                    abs_y,
                    ifc_paint_extent(node, &layout),
                    geometry.page_top,
                    geometry.page_bottom,
                ))
        {
            // The block's own background and border were painted
            // above; its lines are drawn at the content-box origin,
            // where a child text node would have been placed. That
            // origin comes from the taffy layout's border and padding
            // (the text child's `location`), not from
            // `used_padding_for_paint`, which places the element's own
            // generated content.
            let content_x = origin.parent_x
                + geometry.page_offset_x
                + origin.transform_x
                + layout.border.left
                + layout.padding.left;
            let content_y = origin.parent_y
                + geometry.page_offset_y
                + origin.transform_y
                + layout.border.top
                + layout.padding.top;
            Some(LinesFrame {
                node_id,
                x: content_x,
                y: content_y,
                shift_y: origin.shift_y,
                translation_y: origin.transform_y + origin.paint_offset_y,
                decorations: origin.decorations.clone(),
                fragmentainer: origin.fragmentainer,
                inside_fixed: ifc_inside_fixed,
                paint_transform,
                fixed_viewport: fixed_background_viewport(
                    geometry.page_area,
                    self.root_transform,
                    origin.inside_fixed_containing_block,
                ),
                // Lines clipped to the page content box hold no
                // boxes, which that clip does not cover.
                hold: !children.is_empty()
                    && node.ifc_lines().is_some()
                    && geometry
                        .text_page_clip(cascade, self.page_box, ifc_inside_fixed)
                        .is_none(),
                boxes_in_groups: children.iter().any(|&child| {
                    !crate::ifc_text::enclosing_inline_opacities(
                        crate::ifc_text::Paragraph {
                            document,
                            cascade,
                            root_id: node_id,
                        },
                        child,
                    )
                    .is_empty()
                }),
            })
        } else {
            None
        }
    }

    /// How the children of `element`, visited as `visit`, take part in the
    /// paint of its layer. `overflow_clip_pushed` is the clip that
    /// [`PaintWalk::open_clips`] opened for the element and the transform it
    /// was pushed in, `hold_lines` whether the lines of the element are held
    /// for the boxes beside them, `before_advance` how far the generated text
    /// before it pushes its direct text, and `after_pushed` whether the frame
    /// of its `::after` content was pushed.
    pub(super) fn child_plan(
        &self,
        element: &ElementBox<'w>,
        visit: &VisitFrame,
        overflow_clip_pushed: Option<(raikiri_traits::PaintClip, Affine)>,
        hold_lines: bool,
        before_advance: f32,
        after_pushed: bool,
    ) -> ChildPlan {
        let (document, cascade) = (self.document, self.cascade);
        let (node, node_id) = (element.node, element.node_id);
        let own_multicol_clip_height = multicol_clip_height(element, visit.column_clip_height);
        // A plain block in a block container leaves its inline
        // content to the end of the layer (CSS 2.1 Appendix E): its
        // lines and the boxes laid out beside them paint after the
        // backgrounds of the blocks that follow it.
        let defers_here = !element.multicol_clip_pushed
            && !element.fragmentainer_clip_pushed
            && visit.fragmentainer.is_none()
            && !visit.is_fragment_visit
            && !element.fixed_in_viewport;
        let defers = defers_here
            && overflow_clip_pushed.is_none()
            && raikiri_dom::paint_rules::defers_inline_content(document, cascade, node_id);
        // A plain block that clips collects its inline content in a
        // layer of its own, which it hands to the enclosing layer
        // inside its clip.
        let merge = overflow_clip_pushed
            .filter(|_| {
                defers_here
                    && self
                        .deferral_scopes
                        .last()
                        .is_some_and(|layer| layer.allowed)
                    && raikiri_dom::paint_rules::merges_clipped_inline_content(
                        document, cascade, node_id,
                    )
            })
            .map(|(clip, transform)| LayerClose::Merge {
                node_id,
                clip,
                transform,
            });
        // With content positioned below the blocks, the lines place
        // the blocks between that content and the rest.
        let negative_content = node.is_ifc_root()
            && raikiri_dom::paint_rules::holds_negative_stack_level(document, cascade, node_id);
        ChildPlan {
            fragment_clip_height: visit.fragment_clip_height,
            multicol_clip_height: own_multicol_clip_height,
            before_advance,
            defers,
            hold_lines,
            negative_content,
            merge,
            after_pushed,
        }
    }

    /// The frames that paint `children`, the children of `element`.
    pub(super) fn child_frames(
        &self,
        element: &ElementBox<'w>,
        origin: &ChildOrigin<'_>,
        children: Vec<usize>,
        offsets: &GeneratedOffsets,
        plan: &ChildPlan,
    ) -> ChildFrames<'w> {
        let (document, cascade) = (self.document, self.cascade);
        let (node, node_id) = (element.node, element.node_id);
        // Frames of the children, last child first.
        let mut child_frames = Vec::new();
        let mut below_lines_frames = Vec::new();
        // In-flow blocks inside the paragraph's inline content, whose
        // backgrounds paint before its lines.
        let mut block_frames = Vec::new();
        let blocks_first = node.is_ifc_root() && !children.is_empty();
        for child in children.into_iter().rev() {
            if let Some(cell) = document.anonymous_table_cell(child) {
                child_frames.push(self.anonymous_cell_frame(child, cell, origin, plan));
                continue;
            }
            // A box beside held lines joins the groups of the
            // paragraph, which composite it with the rest of their
            // paint once the lines and boxes are all recorded.
            // A box beside other lines inside translucent inline
            // elements paints inside their opacity. Each box gets
            // layers of its own, apart from the group the elements'
            // text composites in.
            let inline_opacities = if node.is_ifc_root() && !plan.hold_lines {
                crate::ifc_text::enclosing_inline_opacities(
                    crate::ifc_text::Paragraph {
                        document,
                        cascade,
                        root_id: node_id,
                    },
                    child,
                )
            } else {
                Vec::new()
            };
            // A box with a negative stack level paints below the
            // paragraph's lines (CSS 2.1 Appendix E steps 3 and 7).
            let below_lines = node.is_ifc_root()
                && cascade.computed.get(child).is_some_and(|child_cv| {
                    child_cv.position != PositionValue::Static
                        && matches!(child_cv.z_index, ZIndexValue::Integer(level) if level < 0)
                });
            let block = blocks_first
                && raikiri_dom::paint_rules::paints_before_paragraph_lines(
                    document, cascade, node_id, child,
                );
            let frames = if block {
                &mut block_frames
            } else if below_lines {
                &mut below_lines_frames
            } else {
                &mut child_frames
            };
            // A box beside held lines joins the groups of the
            // paragraph, which composite it with the rest of their
            // paint once the lines and boxes are all recorded.
            let captured = (plan.hold_lines && !block) || (block && plan.negative_content);
            if captured {
                frames.push(PaintFrame::EndBox {
                    root: node_id,
                    node_id: child,
                    early: below_lines || block,
                });
            }
            frames.extend(inline_opacities.iter().map(|_| PaintFrame::PopOpacity));
            frames.push(PaintFrame::Visit(
                self.child_visit(child, origin, offsets, plan),
            ));
            frames.extend(
                inline_opacities
                    .iter()
                    .rev()
                    .map(|&(node_id, alpha)| PaintFrame::PushOpacity { node_id, alpha }),
            );
            if captured {
                frames.push(PaintFrame::BeginBox {
                    root: node_id,
                    node_id: child,
                    early: below_lines || block,
                });
            }
        }
        ChildFrames {
            children: child_frames,
            below_lines: below_lines_frames,
            blocks: block_frames,
        }
    }

    /// The frame of the paragraph of `cell`, an anonymous table cell that is
    /// the child `child` of an element.
    fn anonymous_cell_frame(
        &self,
        child: usize,
        cell: &'w Node,
        origin: &ChildOrigin<'_>,
        plan: &ChildPlan,
    ) -> PaintFrame<'w> {
        PaintFrame::AnonymousParagraph(AnonymousParagraphFrame {
            key: child,
            cell,
            x: origin.parent_x + cell.unrounded_layout.location.x,
            y: origin.parent_y + cell.unrounded_layout.location.y,
            shift_y: origin.shift_y,
            transform_x: origin.transform_x,
            transform_y: origin.transform_y,
            paint_offset_y: origin.paint_offset_y,
            fragmentainer: origin.fragmentainer,
            inside_fixed: origin.inside_fixed,
            decorations: origin.decorations.clone(),
            fixed_viewport: fixed_background_viewport(
                self.geometry.page_area,
                self.root_transform,
                origin.inside_fixed_containing_block,
            ),
            defer: plan.defers,
        })
    }

    /// The visit of the child `child` of an element, which generated text of
    /// the element may shift.
    fn child_visit(
        &self,
        child: usize,
        origin: &ChildOrigin<'_>,
        offsets: &GeneratedOffsets,
        plan: &ChildPlan,
    ) -> VisitFrame {
        let (document, cascade) = (self.document, self.cascade);
        let child_fragment_clip_height = self.child_fragment_clip_height(child, plan);
        let is_direct_text = document
            .get_node(child)
            .is_some_and(|node| node.kind() == NodeKind::Text);
        let generated_text_shift = if is_direct_text {
            plan.before_advance
        } else {
            0.0
        };
        let inline_generated_offset = generated_shift(&offsets.inline, child);
        let vertical_generated_offset = generated_shift(&offsets.vertical, child);
        VisitFrame {
            node_id: child,
            parent_abs_x: origin.parent_x + generated_text_shift + inline_generated_offset,
            parent_abs_y: origin.parent_y + vertical_generated_offset,
            parent_font_size: origin.font_size,
            shift_y: origin.shift_y,
            transform_x: origin.transform_x,
            transform_y: origin.transform_y,
            paint_offset_y: origin.paint_offset_y,
            fragment_clip_height: child_fragment_clip_height,
            column_clip_height: (cascade.computed[child].display == DisplayValue::Contents)
                .then_some(plan.multicol_clip_height)
                .flatten(),
            fragment_id: origin.fragment_id,
            fragment_origin_x: origin.fragment_origin_x,
            fragment_origin_y: origin.fragment_origin_y,
            fragmentainer: origin.fragmentainer,
            fragment_rect: None,
            fragment_clip: None,
            is_fragment_visit: false,
            inside_fixed: origin.inside_fixed,
            inside_fixed_containing_block: origin.inside_fixed_containing_block,
            decorations: origin.decorations.clone(),
        }
    }

    /// The height that the visit of the child `child` is clipped to.
    fn child_fragment_clip_height(&self, child: usize, plan: &ChildPlan) -> Option<f32> {
        let (document, cascade) = (self.document, self.cascade);
        match plan.multicol_clip_height {
            Some(clip_height)
                if document.get_node(child).is_some_and(|child_node| {
                    (cascade.computed[child].display.is_grid_container()
                        || matches!(
                            cascade.computed[child].display,
                            DisplayValue::Table | DisplayValue::InlineTable
                        )) && child_node.kind() == NodeKind::Element
                }) =>
            {
                Some(clip_height)
            }
            // A zero-height block at a multicolumn break can own
            // an overflowing parallel flow. It has no fragment
            // area in the column, so its descendants must not
            // paint into the following sibling's fragment. A
            // block that layout broke into column pieces places
            // its descendants in those pieces itself. A
            // `display: contents` element has no box at all, so
            // its children are the ones the columns hold.
            Some(_) // cov:ignore: exercised by ignored exact paged-text WPT.
                if !self.fragmented_nodes.contains(&child) && document.get_node(child).is_some_and(|child_node| { // cov:ignore: exercised by ignored exact paged-text WPT.
                    child_node.kind() == NodeKind::Element // cov:ignore: exercised by ignored exact paged-text WPT.
                        && cascade.computed[child].display != DisplayValue::Contents
                        && child_node.unrounded_layout.size.height <= 0.0 // cov:ignore: exercised by ignored exact paged-text WPT.
                        && !child_node.children.is_empty() // cov:ignore: exercised by ignored exact paged-text WPT.
                }) => // cov:ignore: exercised by ignored exact paged-text WPT.
            {
                Some(0.0) // cov:ignore: exercised by ignored exact paged-text WPT.
            }
            Some(_) => None,
            None => plan.fragment_clip_height,
        }
    }

    /// The children of `element` that are visited, in the order they paint:
    /// the stacking order of positioned siblings, with source order within a
    /// level.
    pub(super) fn paint_children(&self, element: &ElementBox<'w>) -> Vec<usize> {
        let (document, cascade) = (self.document, self.cascade);
        let ElementBox {
            node, node_id, cv, ..
        } = *element;
        // Paint positioned siblings in stacking order while keeping
        // source order for equal stack levels. Flex/grid containers use
        // order-modified document order within each stacking bucket,
        // as required by CSS Display's `order` property behavior.
        // This is intentionally local to the current parent; full
        // nested stacking-context isolation remains outside this
        // minimal painter.
        let mut children =
            if node.is_inline_svg_root() || document.anonymous_table_contents_paint_only(node_id) {
                Vec::new()
            } else if node.is_ifc_root() {
                // The paragraph's own text and inline elements are drawn
                // from the lines; only the boxes laid out beside them are
                // visited like ordinary children.
                node.ifc_boxes()
            } else if let Some(children) = document.anonymous_table_paint_sequence(node_id) {
                children
            } else {
                node.children.clone()
            };
        raikiri_dom::paint_rules::sort_paint_children(&mut children, cv.display, cascade);
        children
    }
}

/// The height that the columns of `element`, or of the container it sits
/// in as a `display: contents` element, clip its children to.
pub(super) fn multicol_clip_height(
    element: &ElementBox<'_>,
    column_clip_height: Option<f32>,
) -> Option<f32> {
    let ElementBox {
        node, cv, layout, ..
    } = *element;
    let own_multicol_clip_height = match cv.column_count {
        ColumnCountValue::Count(count)
            if count > 1 && node.is_multicol_container() && layout.size.height > 0.0 =>
        {
            Some(layout.size.height)
        }
        _ => None,
    };
    // A `display: contents` element has no box, so the columns of
    // the container it sits in hold its children.
    own_multicol_clip_height.or((cv.display == DisplayValue::Contents)
        .then_some(column_clip_height)
        .flatten())
}

/// How far the generated text shifts `child`, from `(child, shift)` pairs; no
/// shift when the child has none.
fn generated_shift(shifts: &[(usize, f32)], child: usize) -> f32 {
    shifts
        .iter()
        .find_map(|(shifted, shift)| (*shifted == child).then_some(*shift))
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests;
