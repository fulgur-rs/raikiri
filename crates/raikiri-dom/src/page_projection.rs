//! Stored page placements and the final views borrowed from them.

pub(crate) mod fragment;
pub(crate) mod records;

use crate::{Document, Fragment, PageContentInsets, PageMargins, PageSlice};
use raikiri_style::CascadeResult;
use raikiri_traits::{NodeId, PageBox, PaintRect};
use records::{
    PageFragment, PageFragmentEvent, PageFragmentInsets, PageFragmentOrientation,
    PageFragmentPageGeometry, PageFragmentRect,
};

#[derive(Debug, Default, Clone)]
pub(crate) struct PageProjection {
    pages: Vec<PageFragment>,
    links: Vec<Vec<(NodeId, String, Vec<PaintRect>)>>,
}

impl PageProjection {
    pub(crate) fn clear(&mut self) {
        self.pages.clear();
        self.links.clear();
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
        let pages = crate::layout::page_fragments_from_slices_with_page_geometry(
            self,
            cascade,
            fallback_page_box,
            slices,
            &geometries,
        );
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
        self.page_projection = PageProjection { pages, links };
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
                page.items
                    .iter()
                    .map(move |item| Fragment::new(item, page.content_box))
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
