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

/// One line's piece of the box of a non-atomic inline element: its
/// background, borders and outline on that line.
///
/// Coordinates and edge ownership come from the same line pieces the native
/// painter uses.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct LineBox<'a> {
    /// The element's box on the page, whose node and style the piece paints.
    pub fragment: crate::Fragment<'a>,
    /// Paragraph line the piece is on.
    pub line: crate::TextLineId,
    /// Border box of the piece in page-local CSS pixels, in the coordinates
    /// of the paragraph's lines, like [`GeneratedBox::rect`].
    pub rect: PaintRect,
    /// The piece contains the inline start edge of the element's box.
    pub has_start_edge: bool,
    /// The piece contains the inline end edge of the element's box.
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
/// Page, paragraph root and inline element: the line pieces of the inline
/// elements a translucent `::first-line` splits.
pub(super) type PageInlineBoxes = BTreeMap<(usize, usize), Vec<BoxPiece>>;
pub(super) type InlineBoxes = BTreeMap<u32, PageInlineBoxes>;

pub(super) fn prepare(
    document: &Document,
    cascade: &CascadeResult,
    roots: &[ProjectedTextRoot],
    pages: &[PageFragment],
    paragraphs: &crate::layout::ParagraphCache,
    work: &mut crate::layout::ProjectionWork<'_, '_>,
) -> Result<(GeneratedBoxes, InlineBoxes), raikiri_traits::LayoutError> {
    let mut prepared = Vec::new();
    let context = RunContext::new(document, roots, pages);
    let mut eligible = HashMap::new();
    for root in roots {
        work.check()?;
        // Only paragraphs that laid out a generated box, or an inline
        // element split by a translucent `::first-line`, are visited.
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
    let mut inline = InlineBoxes::new();
    for (root, paint_shift, pieces) in &prepared {
        let source = document.ifc_source_owner(root.node);
        let layout_node = document.ifc_layout_node(root.node);
        let offsets = layout_node.map_or(&[][..], |node| node.ifc_relative_offsets());
        // Fixed-position roots and repeated table headers can land on any page.
        let repeated = root.is_repeat || document.table_objects.headers.owner(source).is_some();
        // Each page with the pieces it can hold. A piece in the flow goes only
        // to the pages around its own line, so a box split over many lines
        // and pages is not checked against every page of its paragraph.
        let candidates: Vec<(&PageFragment, &[_])> = if repeated {
            pages.iter().map(|page| (page, *pieces)).collect()
        } else {
            let mut candidates = Vec::new();
            for (index, &(_, _, _, offset, _, top, height)) in pieces.iter().enumerate() {
                // The arithmetic of `line_center_on_page`, so rounding at
                // large coordinates cannot prune the owning page.
                let top = root.y + offset.1 + top;
                let center = (top + (top + height)) * 0.5;
                if !center.is_finite() {
                    continue;
                }
                for page in flow_pages.around(center, center) {
                    work.charge(1)?;
                    candidates.push((page, &pieces[index..=index]));
                }
            }
            candidates
        };
        for (page, pieces) in candidates {
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
            for &(owner, kind, piece, offset, shift, top, height) in pieces {
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
                // A pseudo-element moves by its own relative offset and with
                // its relatively positioned inline ancestors, as its text does.
                // A typographic pseudo-element moves with the inline element
                // its letters come from, as the native painter places it.
                let source = layout_node
                    .and_then(|node| {
                        node.ifc_typographic_fragment(
                            piece.node,
                            piece.source_container,
                            piece.source_owner,
                        )
                    })
                    .map_or(piece.node, |(_, owner)| owner);
                let relative = crate::cumulative_offset(document, root.node, offsets, source);
                let pieces = match kind {
                    Some(kind) => result
                        .entry(page.page_index)
                        .or_default()
                        .entry((root.node, owner, kind))
                        .or_default(),
                    None => inline
                        .entry(page.page_index)
                        .or_default()
                        .entry((root.node, owner))
                        .or_default(),
                };
                pieces.push(BoxPiece {
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
    Ok((result, inline))
}
