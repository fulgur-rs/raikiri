//! Stored page placements and the final views borrowed from them.

pub(crate) mod fragment;
pub(crate) mod paint_order;
pub(crate) mod records;
pub(crate) mod text_runs;

use crate::{Document, Fragment, PageContentInsets, PageMargins, PageSlice};
use raikiri_style::CascadeResult;
use raikiri_traits::{NodeId, PageBox, PaintInsets, PaintRect};
use records::{
    PageFragment, PageFragmentEvent, PageFragmentInsets, PageFragmentOrientation,
    PageFragmentPageGeometry, PageFragmentRect, ProjectedTextRoot,
};

#[derive(Debug, Default, Clone)]
pub(crate) struct PageProjection {
    pages: Vec<PageFragment>,
    links: Vec<Vec<(NodeId, String, Vec<PaintRect>)>>,
    /// Paragraphs laid out by the inline engine, in document order.
    text_roots: Vec<ProjectedTextRoot>,
}

impl PageProjection {
    pub(crate) fn clear(&mut self) {
        self.pages.clear();
        self.links.clear();
        self.text_roots.clear();
    }
}

impl Document {
    /// Replace page placements after pagination has finished.
    #[doc(hidden)]
    pub fn project_pages(
        &mut self,
        cascade: &CascadeResult,
        fallback_page_box: PageBox,
        slices: &[PageSlice],
        geometries: &[(PageBox, PageMargins, PageContentInsets)],
    ) {
        let geometries: Vec<_> = slices
            .iter()
            .zip(geometries)
            .map(|(slice, &(page_box, margins, insets))| {
                PageFragmentPageGeometry::new(
                    slice.page_index,
                    page_box,
                    PageFragmentInsets::new(
                        margins.top,
                        margins.right,
                        margins.bottom,
                        margins.left,
                    ),
                    PageFragmentInsets::new(insets.top, insets.right, insets.bottom, insets.left),
                    PageFragmentRect::new(
                        margins.left + insets.left,
                        margins.top + insets.top,
                        margins.content_width(page_box),
                        (margins.content_height(page_box) - insets.top - insets.bottom).max(0.0),
                    ),
                    if page_box.width > page_box.height {
                        PageFragmentOrientation::Landscape
                    } else {
                        PageFragmentOrientation::Portrait
                    },
                )
            })
            .collect();
        let (mut pages, text_roots) =
            crate::layout::project_slices(self, cascade, fallback_page_box, slices, &geometries);
        for page in &mut pages {
            for item in &page.items {
                if item.kind == records::PageFragmentKind::Text {
                    continue;
                }
                let Ok(node_id) = usize::try_from(item.node_id.0) else {
                    continue;
                };
                let Some(node) = self.get_node(node_id) else {
                    continue;
                };
                let border = node.unrounded_layout.border;
                let whole_box = PaintRect::new(
                    page.content_box.x + item.rect.x,
                    page.content_box.y + item.box_y,
                    item.rect.width,
                    item.box_height,
                );
                if let Some(clip) = crate::paint_rules::overflow_clip(
                    self,
                    cascade,
                    node_id,
                    whole_box,
                    PaintInsets::new(border.top, border.right, border.bottom, border.left),
                ) {
                    page.overflow_clips
                        .insert((item.node_id, item.fragment_index), clip);
                }
            }
        }
        let events = crate::layout::page_fragment_events_from_pages(self, &pages);
        let mut links: Vec<Vec<(NodeId, String, Vec<PaintRect>)>> =
            pages.iter().map(|_| Vec::new()).collect();
        for event in events {
            let PageFragmentEvent::Link(link) = event;
            let target = link.link.href.trim();
            if target.is_empty() {
                continue;
            }
            let Some(page) = pages.iter().position(|p| p.page_index == link.page_index) else {
                continue;
            };
            let content_box = pages[page].content_box;
            let quad = PaintRect::new(
                link.rect.x + content_box.x,
                link.rect.y + content_box.y,
                link.rect.width,
                link.rect.height,
            );
            let entries = &mut links[page];
            match entries
                .iter_mut()
                .find(|(owner, t, _)| *owner == link.anchor_node_id && t == target)
            {
                Some((_, _, quads)) => quads.push(quad),
                None => entries.push((link.anchor_node_id, target.to_owned(), vec![quad])),
            }
        }
        self.page_projection = PageProjection {
            pages,
            links,
            text_roots,
        };
    }

    /// Borrow the final placements on one page.
    #[doc(hidden)]
    pub fn page_fragments(&self, page_index: u32) -> impl Iterator<Item = Fragment<'_>> + '_ {
        self.page_projection
            .pages
            .iter()
            .find(|page| page.page_index == page_index)
            .into_iter()
            .flat_map(|page| {
                page.items.iter().map(move |item| {
                    Fragment::new(item, page.content_box).with_overflow_clip(
                        (item.kind != records::PageFragmentKind::Text)
                            .then(|| {
                                page.overflow_clips
                                    .get(&(item.node_id, item.fragment_index))
                            })
                            .flatten(),
                    )
                })
            })
    }

    /// Borrow grouped links in page-box coordinates.
    #[doc(hidden)]
    pub fn page_links(
        &self,
        page_index: u32,
    ) -> impl Iterator<Item = (NodeId, &str, &[PaintRect])> + '_ {
        self.page_projection
            .pages
            .iter()
            .position(|page| page.page_index == page_index)
            .and_then(|index| self.page_projection.links.get(index))
            .into_iter()
            .flatten()
            .map(|(owner, target, quads)| (*owner, target.as_str(), quads.as_slice()))
    }
}

#[cfg(test)]
mod tests;
