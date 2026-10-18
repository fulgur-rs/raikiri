//! What an element hands to its children: the origin they are placed from,
//! the offsets that generated text gives them, and the frames that paint
//! them and the lines of its own paragraph.

use super::element_box::ElementBox;
use super::frames::{AnonymousParagraphFrame, LinesFrame, PaintFrame, VisitFrame};
use super::paint_walk::PaintWalk;
use super::{
    box_intersects_page, first_text_descendant, fixed_background_viewport, generated_flow_height,
    generated_pseudo_text_advance, generated_pseudo_text_height, ifc_paint_extent,
};
use crate::text;
use anyrender::PaintScene;
use kurbo::Affine;
use raikiri_style::property::{DisplayValue, PositionValue, ZIndexValue};
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
}

/// The frames that paint the children of an element, each list last child
/// first: the children in place, those below the lines of the paragraph,
/// and the blocks the paragraph paints before its lines.
pub(super) struct ChildFrames<'w> {
    pub(super) children: Vec<PaintFrame<'w>>,
    pub(super) below_lines: Vec<PaintFrame<'w>>,
    pub(super) blocks: Vec<PaintFrame<'w>>,
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
        let (document, cascade, counter_snapshots) =
            (self.document, self.cascade, self.counter_snapshots);
        let (cv, node_id) = (element.cv, element.node_id);
        // The current Taffy bridge treats inline children as zero-sized
        // block items.  Keep that layout authority, but provide the
        // measured paint-time inline advance for generated pseudo text
        // so a sequence such as `span::before { content: counter(c) }`
        // remains in source order.  This is deliberately limited to
        // siblings that actually carry generated content; ordinary
        // inline layout remains unchanged until a full IFC lands.
        let child_generated_advances: Vec<(usize, bool, f32)> = children
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
            .collect();
        let inline_offsets = if child_generated_advances
            .iter()
            .any(|(_, _, advance)| *advance > 0.0)
        {
            let family = cv
                .font_family
                .first()
                .cloned()
                .unwrap_or_else(|| raikiri_style::FontFamilyName::generic("serif"));
            let collapsed_space = text::measure_margin_text_advance(
                document,
                " ",
                cv.font_size.px(),
                std::slice::from_ref(&family),
            );
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
        } else {
            Vec::new()
        };
        let vertical_generated_offsets = if !cv.display.is_inline_flow() {
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
                let Some(child_node) = document.get_node(child) else {
                    continue; // cov:ignore: document child ids are arena-valid
                };
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
            offsets
        } else {
            Vec::new()
        };
        GeneratedOffsets {
            inline: inline_offsets,
            vertical: vertical_generated_offsets,
        }
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
        let geometry = self.geometry;
        // Frames of the children, last child first.
        let mut child_frames = Vec::new();
        let mut below_lines_frames = Vec::new();
        // In-flow blocks inside the paragraph's inline content, whose
        // backgrounds paint before its lines.
        let mut block_frames = Vec::new();
        let blocks_first = node.is_ifc_root() && !children.is_empty();
        for child in children.into_iter().rev() {
            if let Some(cell) = document.anonymous_table_cell(child) {
                child_frames.push(PaintFrame::AnonymousParagraph(AnonymousParagraphFrame {
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
                        geometry.page_area,
                        self.root_transform,
                        origin.inside_fixed_containing_block,
                    ),
                    defer: plan.defers,
                }));
                continue;
            }
            let child_fragment_clip_height = match plan.multicol_clip_height {
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
            };
            let is_direct_text = document
                .get_node(child)
                .is_some_and(|node| node.kind() == NodeKind::Text);
            let generated_text_shift = if is_direct_text {
                plan.before_advance
            } else {
                0.0
            };
            let inline_generated_offset = offsets
                .inline
                .iter()
                .find_map(|(offset_child, offset)| (*offset_child == child).then_some(*offset))
                .unwrap_or(0.0);
            let vertical_generated_offset = offsets
                .vertical
                .iter()
                .find_map(|(offset_child, offset)| (*offset_child == child).then_some(*offset))
                .unwrap_or(0.0);
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
            frames.push(PaintFrame::Visit(VisitFrame {
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
            }));
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
}
