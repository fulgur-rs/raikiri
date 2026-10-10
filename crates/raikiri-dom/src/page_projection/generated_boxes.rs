//! Page ownership of decorations from generated inline box pieces.

use super::records::{PageFragment, ProjectedTextRoot};
use super::text_runs::{RunContext, omission};
use crate::{Document, GeneratedKind};
use raikiri_style::{CascadeResult, ComputedValues, PseudoElem};
use raikiri_traits::{NodeId, PaintRect};
use std::collections::{BTreeMap, HashMap, HashSet};

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
    /// Border box in page-local CSS pixels.
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
    pub line: usize,
    pub rect: PaintRect,
    pub has_start_edge: bool,
    pub has_end_edge: bool,
}

/// Page, paragraph root, originating element, and whether this is `::after`.
pub(super) type PageGeneratedBoxes = BTreeMap<(usize, usize, bool), Vec<BoxPiece>>;
pub(super) type GeneratedBoxes = BTreeMap<u32, PageGeneratedBoxes>;

pub(super) fn prepare(
    document: &Document,
    cascade: &CascadeResult,
    roots: &[ProjectedTextRoot],
    pages: &[PageFragment],
    paragraphs: &crate::layout::ParagraphCache,
    work: &mut crate::layout::ProjectionWork<'_, '_>,
) -> Result<GeneratedBoxes, raikiri_traits::LayoutError> {
    let candidates: HashSet<_> = cascade
        .pseudo
        .keys()
        .filter_map(|&(owner, pseudo)| {
            if !matches!(pseudo, PseudoElem::Before | PseudoElem::After) {
                return None;
            }
            let owner = usize::try_from(owner.0).ok()?;
            if document
                .get_node(owner)
                .is_some_and(|node| node.is_ifc_root())
            {
                Some(owner)
            } else {
                document.ifc_root_of(owner)
            }
        })
        .collect();
    let mut prepared = Vec::new();
    let context = RunContext::new(document, roots);
    let mut eligible = HashMap::new();
    for root in roots {
        work.check()?;
        if !candidates.contains(&root.node) {
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
            entry.insert(omission(document, cascade, &context, root.node).is_none());
        }
        if !eligible[&root.node] {
            continue;
        }
        let pieces = paragraphs
            .get(&root.node)
            .map_or(&[][..], |paragraph| paragraph.generated(root.fragmentainer));
        work.charge(pieces.len().saturating_add(1))?;
        prepared.push((*root, pieces));
    }
    // Pages with a flow range, ordered by their start, so a paragraph that
    // is not a repeated header visits only the pages its lines can land on
    // instead of every page.
    let mut by_flow: Vec<_> = pages
        .iter()
        .filter_map(|page| page.flow_range.map(|(start, end)| (start, end, page)))
        .collect();
    by_flow.sort_by(|left, right| left.0.total_cmp(&right.0));
    let ends_ordered = by_flow.windows(2).all(|pair| pair[0].1 <= pair[1].1);
    let mut result = GeneratedBoxes::new();
    for (root, pieces) in &prepared {
        let source = document.ifc_source_owner(root.node);
        let repeated = document.table_objects.headers.owner(source).is_some();
        let candidates: Box<dyn Iterator<Item = &PageFragment>> = if repeated {
            Box::new(pages.iter())
        } else {
            let mut centers = pieces
                .iter()
                .map(|&(_, _, _, offset, _, top, height)| root.y + offset.1 + top + height * 0.5)
                .filter(|center| center.is_finite());
            let Some(first) = centers.next() else {
                continue;
            };
            let (low, high) = centers.fold((first, first), |(low, high), center| {
                (low.min(center), high.max(center))
            });
            // The slack keeps the pruning conservative; the exact test below
            // still decides which page owns each line.
            let last = by_flow.partition_point(|&(start, _, _)| start - 1.0 <= high);
            let first = if ends_ordered {
                by_flow[..last].partition_point(|&(_, end, _)| end + 1.0 <= low)
            } else {
                0
            };
            Box::new(by_flow[first..last].iter().map(|&(_, _, page)| page))
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
            for &(owner, after, piece, offset, shift, top, height) in *pieces {
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
                result
                    .entry(page.page_index)
                    .or_default()
                    .entry((root.node, owner, after))
                    .or_default()
                    .push(BoxPiece {
                        line: piece.line,
                        rect: PaintRect::new(
                            page.content_box.x + root.x + offset.0 + rect.x,
                            page.content_box.y + root.y + offset.1 + rect.y
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
