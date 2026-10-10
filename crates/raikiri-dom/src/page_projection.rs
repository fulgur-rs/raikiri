//! Stored page placements and the final views borrowed from them.

pub(crate) mod continuation;
pub(crate) mod fragment;
pub(crate) mod generated_boxes;
mod image_markers;
pub(crate) mod paint_order;
pub(crate) mod records;
pub(crate) mod text_runs;

use crate::{Document, Fragment, OverflowClip, PageContentInsets, PageMargins, PageSlice};
use raikiri_style::CascadeResult;
use raikiri_traits::{NodeId, PageBox, PaintRect};
use records::{
    OverflowClipSource, PageFragment, PageFragmentEvent, PageFragmentInsets,
    PageFragmentOrientation, PageFragmentPageGeometry, PageFragmentRect,
    PlacementOverflowClipSource, ProjectedTextRoot,
};
use std::collections::{BTreeMap, HashSet};

#[derive(Debug, Default, Clone)]
pub(crate) struct PageProjection {
    pages: Vec<PageFragment>,
    links: Vec<Vec<(NodeId, String, Vec<PaintRect>)>>,
    /// Paragraphs and standalone markers, in document order.
    text_roots: Vec<ProjectedTextRoot>,
    /// For each page fragment, the indices of the text roots that can draw
    /// on it, so a page's runs do not walk every paragraph of the document.
    text_roots_by_page: Vec<Vec<usize>>,
    /// Column-overlap facts about `text_roots`, shared by every page.
    run_context: text_runs::RunContext,
    /// Standalone marker text shaped once, shared by every page placement.
    markers: BTreeMap<usize, text_runs::MarkerText>,
    image_markers: BTreeMap<u32, BTreeMap<NodeId, PaintRect>>,
    generated_boxes: generated_boxes::GeneratedBoxes,
    column_rules: crate::column_rules::PageColumnRules,
    /// Source clip geometry shared across pages rather than copied per page.
    overflow_clips: BTreeMap<NodeId, OverflowClipSource>,
    /// Geometry shared by column placements, independently of the page count.
    placement_overflow_clips: Vec<PlacementOverflowClipSource>,
}

impl PageProjection {
    pub(crate) fn clear(&mut self) {
        self.pages.clear();
        self.links.clear();
        self.text_roots.clear();
        self.text_roots_by_page.clear();
        self.run_context = text_runs::RunContext::default();
        self.markers.clear();
        self.image_markers.clear();
        self.generated_boxes.clear();
        self.column_rules.clear();
        self.overflow_clips.clear();
        self.placement_overflow_clips.clear();
    }

    /// Position of page `page_index` in [`Self::pages`]. Pages are projected
    /// in page order, so the position the index implies is tried before a
    /// search; walking every page then costs linear time, not quadratic.
    fn page_position(&self, page_index: u32) -> Option<usize> {
        let first = self.pages.first()?.page_index;
        if let Some(guess) = page_index.checked_sub(first).map(|offset| offset as usize)
            && self
                .pages
                .get(guess)
                .is_some_and(|page| page.page_index == page_index)
        {
            return Some(guess);
        }
        self.pages
            .iter()
            .position(|page| page.page_index == page_index)
    }

    fn page(&self, page_index: u32) -> Option<&PageFragment> {
        self.page_position(page_index)
            .map(|index| &self.pages[index])
    }
}

impl Document {
    /// Prepared marker URL and rectangle belonging to one page and list item.
    #[doc(hidden)]
    pub fn page_marker_image(
        &self,
        page_index: u32,
        owner: NodeId,
    ) -> Option<(&url::Url, PaintRect)> {
        let rect = *self
            .page_projection
            .image_markers
            .get(&page_index)?
            .get(&owner)?;
        Some((
            self.list_marker_image_url(usize::try_from(owner.0).ok()?)?,
            rect,
        ))
    }

    /// Replace page placements after pagination has finished.
    #[doc(hidden)]
    pub fn project_pages(
        &mut self,
        cascade: &CascadeResult,
        fallback_page_box: PageBox,
        slices: &[PageSlice],
        geometries: &[(PageBox, PageMargins, PageContentInsets)],
    ) -> Result<(), raikiri_traits::LayoutError> {
        self.project_pages_with_control(
            cascade,
            fallback_page_box,
            slices,
            geometries,
            &crate::PageLayoutControl::default(),
        )
    }

    /// Replace page placements with bounded column work and caller cancellation.
    /// Existing placements remain intact if projection fails.
    #[doc(hidden)]
    pub fn project_pages_with_control(
        &mut self,
        cascade: &CascadeResult,
        fallback_page_box: PageBox,
        slices: &[PageSlice],
        geometries: &[(PageBox, PageMargins, PageContentInsets)],
        control: &crate::PageLayoutControl<'_>,
    ) -> Result<(), raikiri_traits::LayoutError> {
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
                        insets.page_area_width(margins, page_box),
                        insets.page_area_height(margins, page_box),
                    ),
                    if page_box.width > page_box.height {
                        PageFragmentOrientation::Landscape
                    } else {
                        PageFragmentOrientation::Portrait
                    },
                )
            })
            .collect();
        let (pages, text_roots, overflow_clips, placement_overflow_clips, paragraphs, remaining) =
            crate::layout::project_slices_with_control(
                self,
                cascade,
                fallback_page_box,
                slices,
                &geometries,
                control,
            )?;
        let markers = text_runs::prepare_markers(self, cascade, &text_roots, &pages)?;
        let mut work = crate::layout::ProjectionWork::with_remaining(
            control,
            self.fragment_tree.limit,
            remaining,
        );
        let image_markers =
            image_markers::prepare(self, cascade, &text_roots, &pages, &paragraphs, &mut work)?;
        let generated_boxes =
            generated_boxes::prepare(self, cascade, &text_roots, &pages, &paragraphs, &mut work)?;
        let column_rules = crate::column_rules::project(self, cascade, &pages, &mut work)?;
        let text_roots_by_page =
            text_runs::roots_by_page(self, cascade, &text_roots, &markers, &pages);
        let run_context = text_runs::RunContext::new(self, &text_roots);
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
            text_roots_by_page,
            run_context,
            markers,
            image_markers,
            generated_boxes,
            column_rules,
            overflow_clips,
            placement_overflow_clips,
        };
        Ok(())
    }

    /// Borrow the final placements on one page.
    #[doc(hidden)]
    pub fn page_fragments(&self, page_index: u32) -> impl Iterator<Item = Fragment<'_>> + '_ {
        self.page_projection
            .page(page_index)
            .into_iter()
            .flat_map(move |page| {
                page.items.iter().map(move |item| {
                    Fragment::new(item, page.content_box).with_overflow_clip(
                        item.own_overflow_source
                            .map(|index| self.placement_overflow_clip_on_page(index, page).clip)
                            .or_else(|| {
                                self.page_projection
                                    .overflow_clips
                                    .get(&item.node_id)
                                    .filter(|_| item.kind != records::PageFragmentKind::Text)
                                    .map(|source| source.on_page(item.node_id, page).clip)
                            }),
                    )
                })
            })
    }

    /// Borrow resolved local clips of a page's placements and their ancestors.
    #[doc(hidden)]
    pub fn page_overflow_clips(&self, page_index: u32) -> impl Iterator<Item = OverflowClip> + '_ {
        self.page_projection
            .page(page_index)
            .into_iter()
            .flat_map(move |page| {
                let mut entries: Vec<_> = self.overflow_clips_on_page(page).into_values().collect();
                let mut used = HashSet::new();
                for item in &page.items {
                    let mut chain = item.own_overflow_source.or(item.overflow_chain);
                    while let Some(index) = chain {
                        if !used.insert(index) {
                            break;
                        }
                        entries.push(self.placement_overflow_clip_on_page(index, page));
                        chain = self.page_projection.placement_overflow_clips[index].parent;
                    }
                }
                entries.sort_by_key(|entry| entry.node);
                entries
            })
    }

    /// Resolve one shared column overflow source on the requested page.
    fn placement_overflow_clip_on_page(&self, index: usize, page: &PageFragment) -> OverflowClip {
        let entry = self.page_projection.placement_overflow_clips[index];
        let mut source = entry.source;
        if let Some(Some(shift)) = self
            .table_objects
            .headers
            .shift(entry.node.0 as usize, page.content_origin_y)
        {
            source.border_box.y += shift;
        }
        source.on_page(entry.node, page)
    }

    /// Materialize only the requested page's ancestors, then release the map.
    fn overflow_clips_on_page(&self, page: &PageFragment) -> BTreeMap<NodeId, OverflowClip> {
        let mut clips = BTreeMap::new();
        let mut visited = HashSet::new();
        for item in &page.items {
            let mut ancestor = Some(item.node_id.0 as usize);
            while let Some(id) = ancestor {
                if !visited.insert(id) {
                    break;
                }
                let node_id = NodeId::new(id as u64);
                if let Some(source) = self.page_projection.overflow_clips.get(&node_id) {
                    let mut source = *source;
                    if let Some(Some(shift)) =
                        self.table_objects.headers.shift(id, page.content_origin_y)
                    {
                        source.border_box.y += shift;
                    }
                    clips.insert(node_id, source.on_page(node_id, page));
                }
                ancestor = self.nodes[id].parent;
            }
        }
        clips
    }

    /// Borrow grouped links in page-box coordinates.
    #[doc(hidden)]
    pub fn page_links(
        &self,
        page_index: u32,
    ) -> impl Iterator<Item = (NodeId, &str, &[PaintRect])> + '_ {
        self.page_projection
            .page_position(page_index)
            .and_then(|index| self.page_projection.links.get(index))
            .into_iter()
            .flatten()
            .map(|(owner, target, quads)| (*owner, target.as_str(), quads.as_slice()))
    }
}

#[cfg(test)]
mod tests;
