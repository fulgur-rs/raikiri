//! Stored page placements and the final views borrowed from them.

pub(crate) mod continuation;
mod flow_pages;
pub(crate) mod fragment;
pub(crate) mod generated_boxes;
pub(crate) mod generated_images;
mod image_markers;
pub(crate) mod paint_order;
pub(crate) mod paint_transform;
pub(crate) mod records;
pub(crate) mod text_runs;

use crate::{Document, Fragment, OverflowClip, PageContentInsets, PageMargins, PageSlice};
use raikiri_style::CascadeResult;
use raikiri_traits::{NodeId, PageBox, PaintRect, PaintTransform};
use records::{
    OverflowClipSource, PageFragment, PageFragmentEvent, PageFragmentInsets,
    PageFragmentOrientation, PageFragmentPageGeometry, PageFragmentRect,
    PlacementOverflowClipSource, ProjectedTextRoot,
};
use std::collections::{BTreeMap, HashMap, HashSet};

/// One link of a page: the owning `<a>`, its target, its quads, and the
/// placement each quad comes from.
type PageLink = (NodeId, String, Vec<PaintRect>, Vec<NodeId>);

#[derive(Debug, Default, Clone)]
pub(crate) struct PageProjection {
    pages: Vec<PageFragment>,
    links: Vec<Vec<PageLink>>,
    /// Paragraphs and standalone markers, in document order.
    text_roots: Vec<ProjectedTextRoot>,
    /// For each page fragment, the indices of the text roots that can draw
    /// on it, so a page's runs do not walk every paragraph of the document.
    text_roots_by_page: Vec<Vec<usize>>,
    /// Column-overlap facts about `text_roots`, shared by every page.
    run_context: text_runs::RunContext,
    /// Elements that open an opacity group, which the paint order lists on
    /// every page it visits them.
    opacity_layers: Vec<usize>,
    /// Standalone marker text shaped once, shared by every page placement.
    markers: BTreeMap<usize, text_runs::MarkerText>,
    image_markers: BTreeMap<u32, BTreeMap<NodeId, PaintRect>>,
    generated_images: generated_images::GeneratedImages,
    generated_boxes: generated_boxes::GeneratedBoxes,
    /// The line pieces of the inline elements split by a translucent
    /// `::first-line`, by page.
    inline_boxes: generated_boxes::InlineBoxes,
    /// The `::first-letter` opacity groups of the paragraphs, built when
    /// first read.
    letter_groups: paint_order::LetterGroupCache,
    column_rules: crate::column_rules::PageColumnRules,
    /// Source clip geometry shared across pages rather than copied per page.
    overflow_clips: BTreeMap<NodeId, OverflowClipSource>,
    /// Geometry shared by column placements, independently of the page count.
    placement_overflow_clips: Vec<PlacementOverflowClipSource>,
    /// Layout boxes of the elements whose transforms do more than translate.
    transform_boxes: paint_transform::TransformBoxes,
}

impl PageProjection {
    pub(crate) fn clear(&mut self) {
        self.pages.clear();
        self.links.clear();
        self.text_roots.clear();
        self.text_roots_by_page.clear();
        self.run_context = text_runs::RunContext::default();
        self.opacity_layers.clear();
        self.markers.clear();
        self.image_markers.clear();
        self.generated_images.clear();
        self.generated_boxes.clear();
        self.inline_boxes.clear();
        self.letter_groups.clear();
        self.column_rules.clear();
        self.overflow_clips.clear();
        self.placement_overflow_clips.clear();
        self.transform_boxes.clear();
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

    /// Where an outside text marker is drawn on the page whose flow starts
    /// at `page.0`, as in [`crate::PositionedGlyphRun`]s of the page, where
    /// `page.1` tells that the painter is drawing content repeated on every
    /// page, such as a fixed box: `Some(None)` on any page but the item's
    /// first, unless the item is repeated there, else the top of
    /// the marker's line for a list item whose border edge is painted at
    /// `top` on a page whose content ends at `page_bottom`. Its baseline
    /// meets the item's first line, and it stays on the page when that line
    /// starts on a later one. The line's page is decided where layout put
    /// it, before the translations that paint adds. `None` when layout
    /// prepared no text marker for `owner`.
    #[doc(hidden)]
    pub fn list_marker_text_top(
        &self,
        cascade: &CascadeResult,
        owner: usize,
        page: (f32, bool),
        top: f32,
        page_bottom: f32,
    ) -> Option<Option<f32>> {
        let (content_origin_y, repeated) = page;
        // A marker in a table header repeated on this page belongs to it,
        // as the page's runs place it.
        let repeated = repeated
            || matches!(
                self.table_objects.headers.shift(owner, content_origin_y),
                Some(Some(_))
            );
        self.page_projection.markers.get(&owner).map(|marker| {
            (repeated || marker.is_on_page(content_origin_y)).then(|| {
                let moved = text_runs::run_translation_y(
                    self,
                    cascade,
                    &self.page_projection.run_context,
                    owner,
                );
                marker.line_top(top - moved, page_bottom) + moved
            })
        })
    }

    /// The line holding one image of `::before` or `::after` content placed
    /// on one page, and the UTF-8 offset of the image in that line's
    /// processed text when it is known. The offset orders the image among
    /// the line's glyph runs by [`crate::PositionedGlyphRun::text_offset`],
    /// in logical (reading) order.
    #[doc(hidden)]
    pub fn page_generated_image_text_position(
        &self,
        page_index: u32,
        image: generated_images::GeneratedImageId,
    ) -> Option<(crate::TextLineId, Option<usize>)> {
        let placement = self
            .page_projection
            .generated_images
            .get(&page_index)?
            .get(&image.key()?)?;
        Some((
            crate::TextLineId {
                root: NodeId::new(placement.root as u64),
                index: placement.line,
            },
            placement.text_offset,
        ))
    }

    /// Prepared URL and rectangle of one image of `::before` or `::after`
    /// content placed on one page.
    #[doc(hidden)]
    pub fn page_generated_image(
        &self,
        page_index: u32,
        image: generated_images::GeneratedImageId,
    ) -> Option<(&url::Url, PaintRect)> {
        let key = image.key()?;
        let placement = self
            .page_projection
            .generated_images
            .get(&page_index)?
            .get(&key)?;
        Some((
            self.content_image_url(key.0, image.pseudo(), key.2)?,
            placement.rect,
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
        let generated_images =
            generated_images::prepare(self, cascade, &text_roots, &pages, &paragraphs, &mut work)?;
        let (generated_boxes, inline_boxes) =
            generated_boxes::prepare(self, cascade, &text_roots, &pages, &paragraphs, &mut work)?;
        let column_rules = crate::column_rules::project(self, cascade, &pages, &mut work)?;
        let text_roots_by_page =
            text_runs::roots_by_page(self, cascade, &text_roots, &markers, &pages);
        let run_context = text_runs::RunContext::new(self, &text_roots, &pages);
        let transform_boxes = paint_transform::transform_boxes(self, cascade, &pages);
        let opacity_layers = cascade
            .computed
            .iter()
            .enumerate()
            .filter(|(_, cv)| crate::paint_rules::opacity_layer(cv).is_some())
            .map(|(id, _)| id)
            .collect();
        let events = crate::layout::page_fragment_events_from_pages(self, &pages);
        let mut links: Vec<Vec<PageLink>> = pages.iter().map(|_| Vec::new()).collect();
        let page_positions: HashMap<u32, usize> = pages
            .iter()
            .enumerate()
            .rev()
            .map(|(position, page)| (page.page_index, position))
            .collect();
        // Position of each (anchor, target) group in its page's list.
        let mut groups: HashMap<(usize, NodeId, String), usize> = HashMap::new();
        for event in events {
            let PageFragmentEvent::Link(link) = event;
            let target = link.link.href.trim();
            if target.is_empty() {
                continue;
            }
            let Some(&page) = page_positions.get(&link.page_index) else {
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
            match groups.entry((page, link.anchor_node_id, target.to_owned())) {
                std::collections::hash_map::Entry::Occupied(group) => {
                    let (_, _, quads, placements) = &mut entries[*group.get()];
                    quads.push(quad);
                    placements.push(link.placement_node_id);
                }
                std::collections::hash_map::Entry::Vacant(group) => {
                    group.insert(entries.len());
                    entries.push((
                        link.anchor_node_id,
                        target.to_owned(),
                        vec![quad],
                        vec![link.placement_node_id],
                    ));
                }
            }
        }
        self.page_projection = PageProjection {
            pages,
            links,
            text_roots,
            text_roots_by_page,
            run_context,
            opacity_layers,
            markers,
            image_markers,
            generated_images,
            generated_boxes,
            inline_boxes,
            letter_groups: Default::default(),
            column_rules,
            overflow_clips,
            placement_overflow_clips,
            transform_boxes,
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
            for id in self.inclusive_ancestors(item.node_id.0 as usize) {
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
            .map(|(owner, target, quads, _)| (*owner, target.as_str(), quads.as_slice()))
    }

    /// The quads of [`Self::page_links`] where the built-in painter draws
    /// them, in the same order: each quad's corners (top-left, top-right,
    /// bottom-right, bottom-left of the layout rectangle) mapped through
    /// [`Self::page_paint_transform`] of the placement it comes from. A quad
    /// whose placement is not painted is left out.
    #[doc(hidden)]
    pub fn page_link_paint_quads(
        &self,
        cascade: &CascadeResult,
        page_index: u32,
    ) -> Vec<Vec<[(f32, f32); 4]>> {
        let Some(position) = self.page_projection.page_position(page_index) else {
            return Vec::new();
        };
        let (Some(page), Some(links)) = (
            self.page_projection.pages.get(position),
            self.page_projection.links.get(position),
        ) else {
            return Vec::new(); // cov:ignore: every projected page has a link list
        };
        let mut motion = paint_transform::PageMotion::new(self, cascade, page);
        links
            .iter()
            .map(|(_, _, quads, placements)| {
                quads
                    .iter()
                    .zip(placements)
                    .filter_map(|(quad, placement)| {
                        let transform = motion.transform(usize::try_from(placement.0).ok()?)?;
                        Some(
                            [
                                (quad.x, quad.y),
                                (quad.x + quad.width, quad.y),
                                (quad.x + quad.width, quad.y + quad.height),
                                (quad.x, quad.y + quad.height),
                            ]
                            .map(|corner| transform.apply(corner)),
                        )
                    })
                    .collect()
            })
            .collect()
    }

    /// The transforms the built-in painter draws the placements of `nodes`
    /// on page `page_index` through, in the same order: the
    /// [`crate::PaintEvent::PushTransform`] events around each node's events
    /// in the paint order, composed, or the identity when there are none.
    /// Text takes the transform of the element it is in and, inside columns,
    /// the offsets that move its lines. `None` for a node the painter draws
    /// nothing of because a transform on it or an ancestor is singular, and
    /// for a node out of range. Empty for a page that does not exist.
    ///
    /// Each element's transform is found once, however many nodes share it.
    #[doc(hidden)]
    pub fn page_paint_transforms(
        &self,
        cascade: &CascadeResult,
        page_index: u32,
        nodes: impl IntoIterator<Item = NodeId>,
    ) -> Vec<Option<PaintTransform>> {
        let Some(page) = self.page_projection.page(page_index) else {
            return Vec::new();
        };
        let mut motion = paint_transform::PageMotion::new(self, cascade, page);
        nodes
            .into_iter()
            .map(|node| {
                let id = usize::try_from(node.0).ok()?;
                self.get_node(id)?;
                motion.transform(id)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
