//! Page ownership of the images of `::before` and `::after` content.

use super::flow_pages::FlowPages;
use super::records::{PageFragment, ProjectedTextRoot};
use crate::{Document, GeneratedKind};
use raikiri_style::{CascadeResult, PseudoElem};
use raikiri_traits::{NodeId, PaintRect};
use std::collections::BTreeMap;

/// One image of `::before` or `::after` content: the anonymous inline
/// replaced element an `<image>` of the `content` list stands for (CSS
/// Content 3 §2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct GeneratedImageId {
    /// Originating DOM element.
    pub owner: NodeId,
    /// [`GeneratedKind::Before`] or [`GeneratedKind::After`].
    pub kind: GeneratedKind,
    /// Index of the image among the images of the `content` list.
    pub index: usize,
}

impl GeneratedImageId {
    pub(crate) fn new(owner: usize, after: bool, index: usize) -> Self {
        Self {
            owner: NodeId::new(owner as u64),
            kind: if after {
                GeneratedKind::After
            } else {
                GeneratedKind::Before
            },
            index,
        }
    }

    /// The element, whether the pseudo-element is `::after`, and the index.
    pub(crate) fn key(self) -> Option<(usize, bool, usize)> {
        let after = match self.kind {
            GeneratedKind::Before => false,
            GeneratedKind::After => true,
            GeneratedKind::Marker | GeneratedKind::FirstLetter => return None,
        };
        Some((usize::try_from(self.owner.0).ok()?, after, self.index))
    }

    pub(crate) fn pseudo(self) -> PseudoElem {
        match self.kind {
            GeneratedKind::After => PseudoElem::After,
            _ => PseudoElem::Before,
        }
    }
}

/// Where one content image is drawn on a page.
#[derive(Debug, Clone, Copy)]
pub(super) struct ImagePlacement {
    /// Paragraph root whose line holds the image.
    pub root: usize,
    /// Index of that line in the paragraph.
    pub line: usize,
    /// Image rectangle in page-local CSS pixels, in the coordinates of the
    /// paragraph's lines.
    pub rect: PaintRect,
    /// Offset of the image in its line's processed text, when known.
    pub text_offset: Option<usize>,
}

/// Element, whether the pseudo-element is `::after`, and image index.
pub(super) type PageGeneratedImages = BTreeMap<(usize, bool, usize), ImagePlacement>;
pub(super) type GeneratedImages = BTreeMap<u32, PageGeneratedImages>;

pub(super) fn prepare(
    document: &Document,
    cascade: &CascadeResult,
    roots: &[ProjectedTextRoot],
    pages: &[PageFragment],
    paragraphs: &crate::layout::ParagraphCache,
    work: &mut crate::layout::ProjectionWork<'_, '_>,
) -> Result<GeneratedImages, raikiri_traits::LayoutError> {
    let mut placements = GeneratedImages::new();
    // Only paragraphs that laid out a content image are visited.
    let mut prepared = Vec::new();
    for root in roots {
        work.check()?;
        let has_images = document
            .ifc_layout_node(root.node)
            .and_then(|node| node.ifc.as_ref())
            .is_some_and(|ifc| !ifc.generated_atomics.is_empty());
        if !has_images || crate::generated_content::generated_origin(root.node).is_some() {
            continue;
        }
        let Some(paragraph) = paragraphs.get(&root.node) else {
            continue; // cov:ignore: projected paragraphs retain their cache entry
        };
        let images = paragraph.content_images(root.fragmentainer);
        work.charge(images.len().saturating_add(1))?;
        if !images.is_empty() {
            prepared.push((*root, images));
        }
    }
    if prepared.is_empty() {
        return Ok(placements);
    }
    // The painter moves an image with its paragraph's text.
    let context = super::text_runs::RunContext::new(document, roots, pages);
    let mut roots = Vec::with_capacity(prepared.len());
    for (root, images) in prepared {
        work.charge(
            document
                .inclusive_ancestors(document.ifc_source_owner(root.node))
                .count(),
        )?;
        let shift = super::text_runs::placement(document, cascade, &context, root.node)
            .unwrap_or((0.0, 0.0));
        roots.push((root, shift, images));
    }
    let flow_pages = FlowPages::new(pages);
    let flow_axis = pages
        .first()
        .map_or_else(Default::default, |page| page.flow_axis);
    for (root, paint_shift, prepared) in &roots {
        work.check()?;
        let source = document.ifc_source_owner(root.node);
        let offsets = document
            .ifc_layout_node(root.node)
            .map_or(&[][..], |node| node.ifc_relative_offsets());
        // Fixed-position roots and repeated table headers can land on any
        // page; a normal-flow root only on the pages its image lines reach.
        let repeated = root.is_repeat || document.table_objects.headers.owner(source).is_some();
        let candidates: Box<dyn Iterator<Item = &PageFragment>> = if repeated {
            Box::new(pages.iter())
        } else {
            // Without a finite line center the range stays empty, and so
            // does the candidate list.
            let (low, high) = prepared
                .iter()
                .map(|&(top, height, _, _)| {
                    // The arithmetic of `line_center_on_page`, so rounding at
                    // large coordinates cannot prune the owning page.
                    let top = root.y + top;
                    let (top, bottom) = flow_axis.line_extent(root, top, top + height);
                    (top + bottom) * 0.5
                })
                .filter(|center| center.is_finite())
                .fold((f32::INFINITY, f32::NEG_INFINITY), |(low, high), center| {
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
            let (dx, dy) = page.flow_shift(root.is_repeat);
            let x = page.content_box.x + root.x + paint_shift.0 + dx;
            let y = page.content_box.y + root.y + paint_shift.1 + dy;
            for &(top, height, line, image) in *prepared {
                work.charge(1)?;
                if !root.is_repeat {
                    // A page without flow geometry cannot own a normal-flow line.
                    let (start, end) = page.flow_range.unwrap_or((0.0, 0.0));
                    let (top, bottom) =
                        page.flow_axis
                            .line_extent(&root, root.y + top, root.y + top + height);
                    if !crate::layout::line_center_on_page(top, bottom, start, end) {
                        continue;
                    }
                }
                // A pseudo-element moves by its own relative offset and with
                // its relatively positioned inline ancestors, as its text does.
                let relative = crate::cumulative_offset(
                    document,
                    root.node,
                    offsets,
                    crate::generated_content::generated_node_id(image.owner, image.pseudo),
                );
                placements.entry(page.page_index).or_default().insert(
                    (image.owner, image.pseudo == PseudoElem::After, image.index),
                    ImagePlacement {
                        root: root.node,
                        line,
                        rect: PaintRect::new(
                            x + relative.0 + image.rect.x,
                            y + relative.1 + image.rect.y,
                            image.rect.width,
                            image.rect.height,
                        ),
                        text_offset: image.text_offset,
                    },
                );
            }
        }
    }
    Ok(placements)
}
