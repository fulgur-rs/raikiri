//! Prepared marker placements shared by page paint events and raster consumers.

use super::flow_pages::FlowPages;
use super::line_ownership::LineOwnership;
use super::records::{PageFragment, ProjectedTextRoot};
use crate::generated_content::{computed_for_id, generated_origin};
use crate::{Document, PositionedMarker};
use raikiri_style::{CascadeResult, PseudoElem, property::Visibility};
use raikiri_traits::{NodeId, PaintRect};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default)]
pub(super) struct PreparedMarkers {
    pub(super) pages: BTreeMap<u32, BTreeMap<NodeId, PaintRect>>,
    pub(super) anchors: BTreeMap<usize, ImageAnchor>,
}

impl PreparedMarkers {
    pub(super) fn clear(&mut self) {
        self.pages.clear();
        self.anchors.clear();
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ImageAnchor {
    pub(super) offset_y: f32,
    pub(super) page_origin: Option<f32>,
}

enum MarkerRoot {
    Standalone {
        owner: usize,
        size: raikiri_traits::ImageRasterSize,
        offset_y: f32,
        page: Option<u32>,
    },
    Inline(Vec<(f32, f32, PositionedMarker)>),
}

pub(super) fn prepare(
    document: &Document,
    cascade: &CascadeResult,
    roots: &[ProjectedTextRoot],
    pages: &[PageFragment],
    paragraphs: &crate::layout::ParagraphCache,
    work: &mut crate::layout::ProjectionWork<'_, '_>,
) -> Result<PreparedMarkers, raikiri_traits::LayoutError> {
    let mut placements = PreparedMarkers::default();
    // Only IFC roots themselves can own an atomic image marker. Extract their
    // source placements once; ordinary paragraphs need no additional glyph walk.
    let mut prepared = Vec::new();
    for root in roots {
        work.check()?;
        match generated_origin(root.node) {
            Some((owner, PseudoElem::Marker)) if document.list_marker_image(owner).is_some() => {
                let size = document
                    .list_marker_image_size(owner)
                    .expect("prepared marker pixels retain their raster size");
                work.charge(1)?;
                prepared.push((
                    *root,
                    MarkerRoot::Standalone {
                        owner,
                        size,
                        offset_y: 0.0,
                        page: None,
                    },
                ));
            }
            None if crate::generated_content::inside_marker_in_flow(cascade, root.node)
                && document.list_marker_image(root.node).is_some() =>
            {
                let paragraph = paragraphs
                    .get(&root.node)
                    .expect("projected inline image markers retain their paragraph");
                let markers = paragraph.markers(root.fragmentainer);
                work.charge(markers.len().saturating_add(1))?;
                prepared.push((*root, MarkerRoot::Inline(markers.to_vec())));
            }
            _ => {}
        }
    }
    if prepared.is_empty() {
        return Ok(placements);
    }
    // The painter moves a marker with its item's text.
    let context = super::text_runs::RunContext::new(document, roots, pages);
    let mut roots = Vec::with_capacity(prepared.len());
    for (root, kind) in prepared {
        // The placement walks the marker owner's ancestors.
        let owner = generated_origin(root.node)
            .map_or_else(|| document.ifc_source_owner(root.node), |(owner, _)| owner);
        work.charge(document.inclusive_ancestors(owner).count())?;
        let shift = super::text_runs::placement(document, cascade, &context, root.node)
            .unwrap_or((0.0, 0.0));
        roots.push((root, shift, kind));
    }
    let mut first_pages = BTreeMap::new();
    for page in pages {
        for item in &page.items {
            if item.fragment_index == 0 {
                first_pages
                    .entry(item.node_id)
                    .or_insert((page.page_index, page.content_origin_y));
            }
        }
    }
    let flow_pages = FlowPages::new(pages);
    let vertical_flow = pages
        .iter()
        .all(|page| page.flow_axis == super::records::FlowAxis::Y);
    let mut counts = BTreeMap::<usize, usize>::new();
    for (_, _, kind) in &roots {
        if let MarkerRoot::Standalone { owner, .. } = kind {
            *counts.entry(*owner).or_default() += 1;
        }
    }
    let mut first_lines = BTreeMap::new();
    for (root, _, kind) in &mut roots {
        let MarkerRoot::Standalone {
            owner,
            size,
            offset_y,
            page,
        } = kind
        else {
            continue;
        };
        let line = if let Some(line) = first_lines.get(owner) {
            *line
        } else {
            // Reserve work for the first-line search once per owner,
            // rather than walking the item's subtree on each page.
            let mut pending = vec![*owner];
            while let Some(node) = pending.pop() {
                work.charge(1)?;
                if let Some(node) = document.ifc_layout_node(node) {
                    pending.extend(node.layout_children());
                }
            }
            // The shared first-line metrics describe the horizontal
            // baseline. Keep physical placement for vertical items and
            // flows that paginate along the other axis.
            let line = (vertical_flow && !document.nodes[*owner].used_writing_mode.is_vertical())
                .then(|| super::text_runs::first_line(document, cascade, *owner))
                .flatten()
                .filter(|line| {
                    line.horizontal && line.baseline.is_finite() && line.center.is_finite()
                });
            first_lines.insert(*owner, line);
            line
        };
        *offset_y = line.map_or(0.0, |line| {
            let top = line.baseline - size.height;
            // An oversized outside marker stays above the column flow's
            // text, but not above the item's content-box start.
            if document.nodes[*owner].multicol.is_some() {
                let layout = document.nodes[*owner].layout;
                top.max(layout.border.top + layout.padding.top)
            } else {
                top
            }
        });
        let first = first_pages.get(&NodeId::new(*owner as u64)).copied();
        let selected = super::text_runs::marker_page(
            line.filter(|_| counts[owner] == 1)
                .map(|line| root.y + line.center),
            vertical_flow.then_some(&flow_pages),
            first,
        );
        *page = selected.map(|(index, _)| index);
        placements.anchors.insert(
            *owner,
            ImageAnchor {
                offset_y: *offset_y,
                page_origin: selected.map(|(_, origin)| origin),
            },
        );
    }
    let mut line_ownership = LineOwnership::new(pages);
    let flow_axis = pages
        .first()
        .map_or_else(Default::default, |page| page.flow_axis);
    let mut per_page: Vec<BTreeMap<NodeId, PaintRect>> = vec![BTreeMap::new(); pages.len()];
    for (root, paint_shift, kind) in &roots {
        work.check()?;
        let source = generated_origin(root.node)
            .map_or_else(|| document.ifc_source_owner(root.node), |(owner, _)| owner);
        // Fixed-position roots and repeated table headers can land on any
        // page; any other marker lands only on the pages its owner's first
        // fragment or its lines are on.
        let repeated = root.is_repeat || document.table_objects.headers.owner(source).is_some();
        let candidates: Box<dyn Iterator<Item = (usize, &PageFragment)>> = if repeated {
            Box::new(pages.iter().enumerate())
        } else {
            match kind {
                MarkerRoot::Standalone { page, .. } => {
                    let Some(first_page) = *page else {
                        continue;
                    };
                    let start = pages.partition_point(|page| page.page_index < first_page);
                    Box::new(
                        pages
                            .iter()
                            .enumerate()
                            .skip(start)
                            .take_while(move |(_, page)| page.page_index == first_page),
                    )
                }
                MarkerRoot::Inline(inline_markers) => {
                    // A line's extent holds its center, and a line taller
                    // than its page reaches the pages along all of it.
                    // Without a finite extent the range stays empty, and so
                    // does the candidate list.
                    let (low, high) = inline_markers
                        .iter()
                        .map(|&(top, height, _)| {
                            // The arithmetic of `PageFragment::owns_line`, so
                            // rounding at large coordinates cannot prune an
                            // owning page.
                            let top = root.y + top;
                            let (top, bottom) = flow_axis.line_extent(root, top, top + height);
                            (top.min(bottom), top.max(bottom))
                        })
                        .filter(|(low, high)| low.is_finite() && high.is_finite())
                        .fold(
                            (f32::INFINITY, f32::NEG_INFINITY),
                            |(first, last), (low, high)| (first.min(low), last.max(high)),
                        );
                    Box::new(flow_pages.around_slots(low, high))
                }
            }
        };
        for (page_slot, page) in candidates {
            work.charge(1)?;
            let markers = &mut per_page[page_slot];
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
            match kind {
                MarkerRoot::Standalone {
                    owner,
                    size,
                    offset_y,
                    page: marker_page,
                } => {
                    let owner = *owner;
                    let id = NodeId::new(owner as u64);
                    if !root.is_repeat && *marker_page != Some(page.page_index) {
                        continue;
                    }
                    let Some(style) = computed_for_id(cascade, root.node)
                        .filter(|style| style.visibility == Visibility::Visible)
                    else {
                        continue;
                    };
                    let node = &document.nodes[owner];
                    let layout = node.layout;
                    if !node.children.is_empty()
                        && (layout.size.width <= 0.0 || layout.size.height <= 0.0)
                    {
                        continue;
                    }
                    markers.insert(
                        id,
                        document.standalone_marker_image_rect(
                            style,
                            (x, y + offset_y),
                            layout.padding.left,
                            *size,
                        ),
                    );
                }
                MarkerRoot::Inline(inline_markers) => {
                    for &(top, height, marker) in inline_markers {
                        work.charge(1)?;
                        if !root.is_repeat {
                            let (top, bottom) = page.flow_axis.line_extent(
                                &root,
                                root.y + top,
                                root.y + top + height,
                            );
                            if !line_ownership.owns(page, top, bottom) {
                                continue;
                            }
                        }
                        markers.insert(
                            NodeId::new(marker.owner as u64),
                            PaintRect::new(
                                x + marker.rect.x,
                                y + marker.rect.y,
                                marker.rect.width,
                                marker.rect.height,
                            ),
                        );
                    }
                }
            }
        }
    }
    for (page, markers) in pages.iter().zip(per_page) {
        placements.pages.insert(page.page_index, markers);
    }
    Ok(placements)
}

#[cfg(test)]
mod tests;
