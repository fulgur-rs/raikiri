//! The offsets that generated text puts between the children of an element.

use super::element_box::ElementBox;
use super::paint_walk::PaintWalk;
use super::{generated_flow_height, generated_pseudo_text_advance, generated_pseudo_text_height};
use crate::text;
use anyrender::PaintScene;
use raikiri_style::property::PositionValue;
use raikiri_traits::NodeKind;

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
}
