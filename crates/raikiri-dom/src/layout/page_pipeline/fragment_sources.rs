//! The boxes and lines of a document as the sources of page fragments: the
//! pages each source lands on, how the fragments of a source are numbered, and
//! which sources clip their overflow.

use super::{
    CascadeResult, Document, FlowAxis, LayoutError, NodeId, NodeKind, OverflowClipSource,
    PageFragment, PageFragmentInsets, PageFragmentItem, PageFragmentKind, PageFragmentLineRange,
    PageFragmentRect, PaintInsets, PaintRect, ProjectionWork, flow_page_overlap,
    line_ranges::LineRanges,
};
use raikiri_traits::PaintClip;
use std::collections::{BTreeMap, HashMap, HashSet};

/// A box, or the lines of a text, at its position in the shared flow space of
/// the pages, before the pages it reaches cut it into fragments.
#[derive(Clone)]
pub(super) struct PageFragmentSource {
    pub(super) node_id: NodeId,
    pub(super) node_kind: NodeKind,
    pub(super) is_img: bool,
    pub(super) abs_x: f32,
    pub(super) abs_y: f32,
    pub(super) width: f32,
    pub(super) height: f32,
    pub(super) content_insets: Option<PageFragmentInsets>,
    pub(super) line_metrics: Option<Vec<(f32, f32)>>,
    /// The lines of `line_metrics` stack along x, from the block-start
    /// edge of their paragraph.
    pub(super) lines_along_x: bool,
    pub(super) line_start_offset: u32,
    pub(super) is_repeat: bool,
    pub(super) fragmentainer: u32,
    pub(super) fragment_clip: Option<PaintClip>,
    pub(super) overflow_chain: Option<usize>,
    pub(super) own_overflow_source: Option<usize>,
    /// How far down the painter moves the box by the relative offsets that
    /// layout leaves to it (see [`super::painter_relative_shift_y`]). The
    /// box is drawn on the pages it reaches where it is painted.
    pub(super) painted_shift: f32,
}

impl PageFragmentSource {
    /// This source with its geometry in the flow space of `axis`.
    fn in_flow_space(&self, axis: FlowAxis) -> Self {
        let (abs_x, abs_y, width, height) =
            axis.to_flow((self.abs_x, self.abs_y, self.width, self.height));
        let along_flow = self.lines_along_x == matches!(axis, FlowAxis::X { .. });
        Self {
            abs_x,
            abs_y,
            // The painter moves boxes down the page, which is the flow only
            // when it runs along y.
            painted_shift: if matches!(axis, FlowAxis::Y) {
                self.painted_shift
            } else {
                0.0
            },
            width,
            height,
            content_insets: self.content_insets.map(|insets| axis.sides_to_flow(insets)),
            // Lines that stack across the flow all share the source's
            // extent along it.
            line_metrics: self.line_metrics.as_ref().map(|metrics| {
                if along_flow {
                    metrics.clone()
                } else {
                    vec![(0.0, height); metrics.len()]
                }
            }),
            fragment_clip: self.fragment_clip.map(|clip| {
                let mut clip = clip;
                let rect = clip.rect;
                let (x, y, width, height) = axis.to_flow((rect.x, rect.y, rect.width, rect.height));
                clip.rect = PaintRect::new(x, y, width, height);
                if matches!(axis, FlowAxis::X { .. }) {
                    std::mem::swap(&mut clip.clip_x, &mut clip.clip_y);
                }
                clip
            }),
            ..self.clone()
        }
    }

    /// What kind of fragment the source makes.
    fn kind(&self) -> PageFragmentKind {
        match self.node_kind {
            NodeKind::Text => PageFragmentKind::Text,
            NodeKind::Element if self.is_img => PageFragmentKind::Replaced,
            _ => PageFragmentKind::Box,
        }
    }

    /// The lines of the source when it lands on a page whole, as a repeated
    /// source does.
    fn all_lines(&self) -> Option<PageFragmentLineRange> {
        self.line_metrics.as_deref().and_then(|metrics| {
            (!metrics.is_empty()).then(|| {
                PageFragmentLineRange::new(0, u32::try_from(metrics.len()).unwrap_or(u32::MAX))
            })
        })
    }
}

/// The pages that have a flow range, in page order.
pub(super) struct FlowSlots {
    /// Each page's slot among all the pages, with the start and the end of
    /// its range.
    ranges: Vec<(usize, f32, f32)>,
    /// Whether both ends of the ranges never decrease, so that the pages of a
    /// source can be found by binary search instead of testing every page.
    ordered: bool,
}

impl FlowSlots {
    pub(super) fn new(pages: &[PageFragment]) -> Self {
        let ranges: Vec<(usize, f32, f32)> = pages
            .iter()
            .enumerate()
            .filter_map(|(page_slot, page)| {
                page.flow_range.map(|(start, end)| (page_slot, start, end))
            })
            .collect();
        let ordered = ranges
            .windows(2)
            .all(|pair| pair[0].1 <= pair[1].1 && pair[0].2 <= pair[1].2);
        Self { ranges, ordered }
    }

    /// The page slots whose flow range starts at `origin`, all of one page:
    /// the last page that starts there, as an empty page that comes before
    /// it starts there too.
    fn starting_at(&self, origin: f32, pages: &[PageFragment]) -> Vec<(usize, f32)> {
        let near = |start: f32| (start - origin).abs() <= 0.001;
        let candidates: Vec<(usize, f32)> = if self.ordered {
            let first = self
                .ranges
                .partition_point(|&(_, start, _)| start < origin - 0.001);
            self.ranges[first..]
                .iter()
                .take_while(|&&(_, start, _)| near(start))
                .map(|&(slot, start, _)| (slot, start))
                .collect()
        } else {
            self.ranges
                .iter()
                .filter(|&&(_, start, _)| near(start))
                .map(|&(slot, start, _)| (slot, start))
                .collect()
        };
        let Some(page_index) = candidates.last().map(|&(slot, _)| pages[slot].page_index) else {
            return Vec::new();
        };
        candidates
            .into_iter()
            .filter(|&(slot, _)| pages[slot].page_index == page_index)
            .collect()
    }

    /// The pages `source` can intersect. A repeated source lands on every
    /// page. Any other source only intersects the pages whose flow range
    /// overlaps it, which form one run of the ranges when they are ordered.
    pub(super) fn reached_by(&self, source: &PageFragmentSource) -> &[(usize, f32, f32)] {
        if source.is_repeat || !self.ordered {
            &self.ranges[..]
        } else if source.height > 0.0 {
            let top = source.abs_y + source.painted_shift;
            let bottom = top + source.height;
            let first = self.ranges.partition_point(|&(_, _, end)| end <= top);
            let last =
                first + self.ranges[first..].partition_point(|&(_, start, _)| start < bottom);
            &self.ranges[first..last]
        } else {
            let top = source.abs_y + source.painted_shift;
            let first = self.ranges.partition_point(|&(_, _, end)| end < top);
            let last = first + self.ranges[first..].partition_point(|&(_, start, _)| start <= top);
            &self.ranges[first..last]
        }
    }
}

/// Where one fragment of a source lies on a page.
struct FragmentPlacement {
    /// The page's slot among all the pages.
    page_slot: usize,
    /// Where the fragment starts along the flow, from the top of the page.
    y: f32,
    height: f32,
    line_range: Option<PageFragmentLineRange>,
    /// Where the whole box starts along the flow, from the top of the page.
    box_y: f32,
}

/// Puts each of `sources` on the pages it reaches, as an item of each.
pub(super) fn place_sources(
    document: &Document,
    sources: &[PageFragmentSource],
    pages: &mut [PageFragment],
    flow_axis: FlowAxis,
    work: &mut ProjectionWork<'_, '_>,
) -> Result<(), LayoutError> {
    let slots = FlowSlots::new(pages);
    for source in sources {
        work.check()?;
        let source = &source.in_flow_space(flow_axis);
        let header_placements = document
            .table_objects
            .headers
            .placements_of(source.node_id.0 as usize);
        let is_table_header = header_placements.is_some();
        let is_table_footer = is_table_header
            && document
                .table_objects
                .headers
                .is_footer_source(source.node_id.0 as usize);
        let copy_bands = document
            .table_objects
            .headers
            .copy_bands(document, source.node_id.0 as usize);
        let placements =
            fragment_placements(source, header_placements, copy_bands, pages, &slots, work)?;
        let fragment_count = placements.len() as u32;
        for (fragment_index, placement) in placements.into_iter().enumerate() {
            if let Some(page) = pages.get_mut(placement.page_slot) {
                let mut item = fragment_item(
                    source,
                    &placement,
                    page,
                    flow_axis,
                    is_table_header,
                    (fragment_index as u32, fragment_count),
                );
                item.is_table_footer_repeat = is_table_footer;
                page.items.push(item);
            }
        }
    }
    Ok(())
}

/// The fragments `source` makes, in page order. A repeated header box has a
/// fragment on each page it was placed on; any other source has one on each
/// page it reaches.
fn fragment_placements(
    source: &PageFragmentSource,
    header_placements: Option<impl Iterator<Item = (u32, f32, f32)>>,
    copy_bands: &[(f32, f32, bool)],
    pages: &[PageFragment],
    slots: &FlowSlots,
    work: &mut ProjectionWork<'_, '_>,
) -> Result<Vec<FragmentPlacement>, LayoutError> {
    let repeat_line_range = source.all_lines();
    let mut placements = Vec::new();
    let reached: &[(usize, f32, f32)] = match header_placements {
        Some(header_placements) => {
            // Repeated header boxes visit only their own pages, and every
            // copy is charged so header size times page count stays bounded.
            for (page_index, origin, shift) in header_placements {
                let start = pages.partition_point(|page| page.page_index < page_index);
                let mut header_pages: Vec<(usize, f32)> = pages
                    .iter()
                    .enumerate()
                    .skip(start)
                    .take_while(|(_, page)| page.page_index == page_index)
                    .filter_map(|(page_slot, page)| {
                        page.flow_range
                            .filter(|&(page_start, _)| (page_start - origin).abs() <= 0.001)
                            .map(|(page_start, _)| (page_slot, page_start))
                    })
                    .collect();
                // The table layout numbers the pages it places a header on
                // from the first page it laid out. A relayout projects its
                // pages under the numbers of the pages they stand for, so
                // there the page is the one that begins where the header's
                // page did, as for painting the header.
                if header_pages.is_empty() {
                    header_pages = slots.starting_at(origin, pages);
                }
                for (page_slot, page_start) in header_pages {
                    work.charge(1)?;
                    let y = source.abs_y + shift - page_start;
                    placements.push(FragmentPlacement {
                        page_slot,
                        y,
                        height: source.height,
                        line_range: repeat_line_range,
                        box_y: y,
                    });
                }
            }
            &[]
        }
        None => slots.reached_by(source),
    };
    let mut line_ranges = None;
    for &(page_slot, page_start, page_end) in reached {
        if source.is_repeat {
            placements.push(FragmentPlacement {
                page_slot,
                y: source.abs_y,
                height: source.height,
                line_range: repeat_line_range,
                box_y: source.abs_y,
            });
            continue;
        }
        // The box lies where the painter draws it when the page is chosen,
        // and keeps its layout position on that page: the painter moves it
        // by `shift` again.
        let shift = source.painted_shift;
        let painted_y = source.abs_y + shift;
        let (intersects, fragment_y, fragment_height) = if source.height > 0.0 {
            let continues = (
                painted_y < page_start - 0.001,
                painted_y + source.height > page_end + 0.001,
            );
            match flow_page_overlap(painted_y, source.height, page_start, page_end)
                .and_then(|(y, height)| below_copies(copy_bands, continues, page_start, y, height))
            {
                Some((y, height)) => (true, y - shift, height),
                None => (false, 0.0, 0.0),
            }
        } else {
            (
                painted_y >= page_start && painted_y <= page_end,
                (painted_y - page_start).max(0.0) - shift,
                0.0,
            )
        };
        if intersects {
            let line_range = source.line_metrics.as_deref().and_then(|metrics| {
                line_ranges
                    .get_or_insert_with(|| LineRanges::new(metrics, painted_y))
                    .range(page_start, page_end)
            });
            placements.push(FragmentPlacement {
                page_slot,
                y: fragment_y,
                height: fragment_height,
                line_range,
                box_y: source.abs_y - page_start,
            });
        }
    }
    Ok(placements)
}

/// The fragment at `y` with `height` on the page that starts at
/// `page_start` in the flow, shortened where the box goes on from the page
/// before or to the page after, as `continues` tells: it then starts below
/// the header copies of `bands` (see [`HeaderRepeats::copy_bands`]) that
/// cover its top, or ends at the footer copies inside it. `None` when the
/// copies cover all of it.
///
/// [`HeaderRepeats::copy_bands`]: crate::layout::table::headers::HeaderRepeats::copy_bands
fn below_copies(
    bands: &[(f32, f32, bool)],
    (from_before, to_after): (bool, bool),
    page_start: f32,
    y: f32,
    height: f32,
) -> Option<(f32, f32)> {
    let mut top = page_start + y;
    let mut bottom = top + height;
    if !from_before && !to_after {
        return Some((y, height));
    }
    // The bands do not overlap, so their ends are sorted as their starts
    // are: only the bands from the first one ending below the top to the
    // last one starting above the bottom can cut the fragment.
    let first = bands.partition_point(|&(_, end, _)| end <= top);
    for &(start, end, footer) in &bands[first..] {
        if start >= bottom {
            break;
        }
        if footer {
            if to_after && start < bottom && end > top {
                bottom = bottom.min(start);
            }
        } else if from_before && start <= top + 0.001 && end > top {
            top = end;
        }
    }
    (bottom > top + 0.001 || height <= 0.001).then_some((top - page_start, (bottom - top).max(0.0)))
}

/// The item of `page` for the fragment of `source` at `placement`, which is
/// fragment `numbering.0` of the `numbering.1` fragments of the source.
fn fragment_item(
    source: &PageFragmentSource,
    placement: &FragmentPlacement,
    page: &PageFragment,
    flow_axis: FlowAxis,
    is_table_header: bool,
    numbering: (u32, u32),
) -> PageFragmentItem {
    let FragmentPlacement {
        y,
        height: fragment_height,
        line_range,
        box_y,
        ..
    } = *placement;
    let page_axis = page.flow_axis;
    let to_page = |rect: (f32, f32, f32, f32)| {
        let (x, y, width, height) = page_axis.to_layout(rect);
        PageFragmentRect::new(x, y, width, height)
    };
    let rect = to_page((source.abs_x, y, source.width, fragment_height));
    // A page cuts boxes only along the flow; across it, the whole box
    // is the placed one.
    let (item_box_y, item_box_height) = match flow_axis {
        FlowAxis::Y => (box_y, source.height),
        FlowAxis::X { .. } => (rect.y, rect.height),
    };
    let mut item = PageFragmentItem::new(
        source.node_id,
        rect,
        source.kind(),
        numbering.0,
        numbering.1,
        source.is_repeat || is_table_header,
    )
    .with_page_index(page.page_index)
    .with_box_extent(item_box_y, item_box_height)
    .with_flow_box_y(box_y)
    .with_content_rect(source.content_insets.map(|insets| {
        to_page((
            source.abs_x + insets.left,
            box_y + insets.top,
            (source.width - insets.left - insets.right).max(0.0),
            (source.height - insets.top - insets.bottom).max(0.0),
        ))
    }));
    item.is_table_header_repeat = is_table_header;
    item.fragmentainer = source.fragmentainer;
    item.overflow_chain = source.overflow_chain;
    item.own_overflow_source = source.own_overflow_source;
    item.fragment_clip = source.fragment_clip.map(|mut clip| {
        let rect = clip.rect;
        let rect = to_page((
            rect.x,
            rect.y + box_y - source.abs_y,
            rect.width,
            rect.height,
        ));
        clip.rect = PaintRect::new(rect.x, rect.y, rect.width, rect.height);
        if matches!(page_axis, FlowAxis::X { .. }) {
            std::mem::swap(&mut clip.clip_x, &mut clip.clip_y);
        }
        clip
    });
    match line_range {
        Some(range) => item.with_line_range(PageFragmentLineRange::new(
            range.start.saturating_add(source.line_start_offset),
            range.end.saturating_add(source.line_start_offset),
        )),
        None => item,
    }
}

/// A relayout resumed at a page start lays the content before it out
/// without the elements wholly before it, so a box crossing the start
/// begins lower than in the earlier layout. It keeps the top the earlier
/// layout gave it; its end, after the start, is the same in both. A box
/// with no border or padding above the start begins at the start itself.
pub(super) fn restore_crossing_tops(
    document: &Document,
    sources: &mut [PageFragmentSource],
    pages: &[PageFragment],
) {
    if let Some(earlier) = &document.earlier_fragments
        && let Some((page_start, _)) = pages.first().and_then(|page| page.flow_range)
    {
        for source in sources {
            if let Some(crossing) = earlier.get(&source.node_id)
                && !source.is_repeat
                && source.abs_y <= page_start + 0.01
            {
                let bottom = source.abs_y + source.height;
                source.abs_y = page_start + crossing.top;
                source.height = (bottom - source.abs_y).max(0.0);
            }
        }
    }
}

/// A relayout resumed at a page start continues the fragments the boxes
/// crossing that start had before it. The body begins on the relayout's
/// first page, which the relayout drops; when it did not cross the start
/// in the earlier layout, it has no fragment after it.
pub(super) fn drop_uncrossed_body(document: &Document, pages: &mut [PageFragment], body_id: usize) {
    if let Some(earlier) = &document.earlier_fragments
        && let Some(first) = pages.first_mut()
    {
        let body = NodeId::new(body_id as u64);
        first.items.retain(|item| {
            item.node_id != body || item.flow_box_y >= -0.01 || earlier.contains_key(&body)
        });
    }
}

/// Numbers the fragments of every node through the pages, in page order, and
/// gives each the number of fragments its node has. A node resumed from an
/// earlier layout goes on from the fragments it had there, and one that goes
/// on in a later layout counts those too.
pub(super) fn number_fragments(document: &Document, pages: &mut [PageFragment]) {
    let earlier = document.earlier_fragments.as_ref();
    let before = |node: &NodeId| {
        earlier
            .and_then(|earlier| earlier.get(node))
            .map(|crossing| crossing.earlier_fragments)
    };

    // A source can have several column pieces on the same physical page.
    // Count all committed pieces together so edge ownership remains stable.
    let mut fragment_counts = HashMap::<NodeId, u32>::new();
    for item in pages.iter().flat_map(|page| &page.items) {
        *fragment_counts.entry(item.node_id).or_insert_with(|| {
            before(&item.node_id).unwrap_or(0)
                + document
                    .later_fragments
                    .get(&item.node_id)
                    .copied()
                    .unwrap_or(0)
        }) += 1;
    }
    let mut fragment_indices = HashMap::<NodeId, u32>::new();
    for item in pages.iter_mut().flat_map(|page| &mut page.items) {
        let index = fragment_indices
            .entry(item.node_id)
            .or_insert_with(|| before(&item.node_id).unwrap_or(0));
        item.fragment_index = *index;
        item.fragment_count = fragment_counts[&item.node_id];
        *index += 1;
    }
}

/// Retains each clipping source once, other than those of the nodes in
/// `repeated_placement_nodes`, which clip per placement. Page-local clips are
/// resolved on access, so a deep open-axis chain cannot multiply storage by
/// the page count.
pub(super) fn overflow_clip_sources(
    document: &Document,
    cascade: &CascadeResult,
    sources: &[PageFragmentSource],
    repeated_placement_nodes: &HashSet<usize>,
) -> BTreeMap<NodeId, OverflowClipSource> {
    sources
        .iter()
        .filter(|source| !repeated_placement_nodes.contains(&(source.node_id.0 as usize)))
        .filter_map(|source| {
            let id = source.node_id.0 as usize;
            let border = document.nodes[id].layout.border;
            let padding = document.nodes[id].layout.padding;
            let border_box =
                PaintRect::new(source.abs_x, source.abs_y, source.width, source.height);
            let geometry = crate::paint_rules::overflow_clip_geometry(
                document,
                cascade,
                id,
                border_box,
                PaintInsets::new(border.top, border.right, border.bottom, border.left),
                PaintInsets::new(padding.top, padding.right, padding.bottom, padding.left),
            )?;
            Some((
                source.node_id,
                OverflowClipSource {
                    border_box,
                    geometry,
                    is_repeat: source.is_repeat,
                },
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests;
