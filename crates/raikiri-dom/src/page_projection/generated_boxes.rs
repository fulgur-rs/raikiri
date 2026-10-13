//! Page ownership of decorations from generated inline box pieces.

use super::flow_pages::FlowPages;
use super::records::{PageFragment, ProjectedTextRoot};
use super::text_runs::{RunContext, placement};
use crate::{Document, GeneratedKind};
use raikiri_style::{CascadeResult, ComputedValues};
use raikiri_traits::{NodeId, PaintRect};
use std::collections::{BTreeMap, HashMap};

/// The decoration of one generated inline box on one line.
///
/// The owner is an originating DOM element, never the inline engine's synthetic
/// node id. Coordinates and edge ownership come from the same line pieces the
/// native painter uses. Text remains in the page's positioned glyph runs.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct GeneratedBox<'a> {
    /// Originating DOM element.
    pub owner: NodeId,
    /// The pseudo-element whose decoration this is.
    pub kind: GeneratedKind,
    /// Paragraph line whose text belongs to this piece.
    pub line: crate::TextLineId,
    /// Border box in page-local CSS pixels, in the coordinates of the
    /// paragraph's lines: like [`crate::PositionedGlyphRun::origin`], it
    /// leaves the paint transforms around it to the enclosing
    /// [`crate::PaintEvent::PushTransform`] events.
    pub rect: PaintRect,
    /// The pseudo-element's computed style.
    pub style: &'a ComputedValues,
    /// Innermost source element whose overflow clips the decoration.
    pub clip_owner: NodeId,
    /// The piece contains the inline start edge of the generated box.
    pub has_start_edge: bool,
    /// The piece contains the inline end edge of the generated box.
    pub has_end_edge: bool,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct BoxPiece {
    /// The inline engine's node id of the box, with the ids that select the
    /// retained style of a typographic pseudo-element.
    pub node: usize,
    pub source_container: Option<usize>,
    pub source_owner: Option<usize>,
    pub line: usize,
    pub rect: PaintRect,
    pub has_start_edge: bool,
    pub has_end_edge: bool,
}

/// Page, paragraph root, originating element, and the pseudo-element.
pub(super) type PageGeneratedBoxes = BTreeMap<(usize, usize, GeneratedKind), Vec<BoxPiece>>;
pub(super) type GeneratedBoxes = BTreeMap<u32, PageGeneratedBoxes>;

pub(super) fn prepare(
    document: &Document,
    cascade: &CascadeResult,
    roots: &[ProjectedTextRoot],
    pages: &[PageFragment],
    paragraphs: &crate::layout::ParagraphCache,
    work: &mut crate::layout::ProjectionWork<'_, '_>,
) -> Result<GeneratedBoxes, raikiri_traits::LayoutError> {
    let mut prepared = Vec::new();
    let context = RunContext::new(document, roots, pages);
    let mut eligible = HashMap::new();
    for root in roots {
        work.check()?;
        // Only paragraphs that laid out a generated box are visited.
        let pieces = paragraphs
            .get(&root.node)
            .map_or(&[][..], |paragraph| paragraph.generated(root.fragmentainer));
        if pieces.is_empty() {
            continue;
        }
        if let std::collections::hash_map::Entry::Vacant(entry) = eligible.entry(root.node) {
            let node = document
                .ifc_layout_node(root.node)
                .expect("generated candidates retain their paragraph");
            work.charge(node.ifc_relative_offsets().len())?;
            let mut ancestor = Some(document.ifc_source_owner(root.node));
            while let Some(node) = ancestor {
                work.charge(1)?;
                ancestor = document.parent_of(node);
            }
            entry.insert(placement(document, cascade, &context, root.node).ok());
        }
        let Some(paint_shift) = eligible[&root.node] else {
            continue;
        };
        work.charge(pieces.len().saturating_add(1))?;
        prepared.push((*root, paint_shift, pieces));
    }
    let flow_pages = FlowPages::new(pages);
    let mut result = GeneratedBoxes::new();
    for (root, paint_shift, pieces) in &prepared {
        let source = document.ifc_source_owner(root.node);
        let offsets = document
            .ifc_layout_node(root.node)
            .map_or(&[][..], |node| node.ifc_relative_offsets());
        // Fixed-position roots and repeated table headers can land on any page.
        let repeated = root.is_repeat || document.table_objects.headers.owner(source).is_some();
        let candidates: Box<dyn Iterator<Item = &PageFragment>> = if repeated {
            Box::new(pages.iter())
        } else {
            let mut centers = pieces
                .iter()
                .map(|&(_, _, _, offset, _, top, height)| {
                    // The arithmetic of `line_center_on_page`, so rounding at
                    // large coordinates cannot prune the owning page.
                    let top = root.y + offset.1 + top;
                    (top + (top + height)) * 0.5
                })
                .filter(|center| center.is_finite());
            let Some(first) = centers.next() else {
                continue;
            };
            let (low, high) = centers.fold((first, first), |(low, high), center| {
                (low.min(center), high.max(center))
            });
            Box::new(flow_pages.around(low, high))
        };
        for page in candidates {
            work.charge(1)?;
            let mut root = *root;
            if let Some(shift) = document
                .table_objects
                .headers
                .shift(source, page.content_origin_y)
            {
                let Some(shift) = shift else { continue };
                root.y += shift - page.content_origin_y;
                root.is_repeat = true;
            }
            let (start, end) = page.flow_range.unwrap_or((0.0, 0.0));
            for &(owner, kind, piece, offset, shift, top, height) in *pieces {
                work.charge(1)?;
                if !root.is_repeat
                    && !crate::layout::line_center_on_page(
                        root.y + offset.1 + top,
                        root.y + offset.1 + top + height,
                        start,
                        end,
                    )
                {
                    continue;
                }
                let rect = piece.border_box;
                // A pseudo-element moves with its relatively positioned
                // inline ancestors, as its text does.
                let relative = crate::cumulative_offset(document, root.node, offsets, owner);
                result
                    .entry(page.page_index)
                    .or_default()
                    .entry((root.node, owner, kind))
                    .or_default()
                    .push(BoxPiece {
                        node: piece.node,
                        source_container: piece.source_container,
                        source_owner: piece.source_owner,
                        line: piece.line,
                        rect: PaintRect::new(
                            page.content_box.x
                                + root.x
                                + paint_shift.0
                                + relative.0
                                + offset.0
                                + rect.x,
                            page.content_box.y
                                + root.y
                                + paint_shift.1
                                + relative.1
                                + offset.1
                                + rect.y
                                - shift
                                - if root.is_repeat { 0.0 } else { start },
                            rect.width,
                            rect.height,
                        ),
                        has_start_edge: piece.has_start_edge,
                        has_end_edge: piece.has_end_edge,
                    });
            }
        }
    }
    Ok(result)
}
