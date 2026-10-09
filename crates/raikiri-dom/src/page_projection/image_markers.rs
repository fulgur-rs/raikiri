//! Prepared marker placements shared by page paint events and raster consumers.

use super::records::{PageFragment, ProjectedTextRoot};
use crate::generated_content::{computed_for_id, generated_origin};
use crate::{Document, PositionedLines, PositionedMarker};
use raikiri_style::{CascadeResult, PseudoElem, property::Visibility};
use raikiri_traits::{NodeId, PaintRect};
use std::collections::BTreeMap;

pub(super) fn prepare(
    document: &Document,
    cascade: &CascadeResult,
    roots: &[ProjectedTextRoot],
    pages: &[PageFragment],
) -> BTreeMap<u32, BTreeMap<NodeId, PaintRect>> {
    let mut placements = BTreeMap::new();
    // Only IFC roots themselves can own an atomic image marker. Extract their
    // source placements once; ordinary paragraphs need no additional glyph walk.
    let roots: Vec<_> = roots
        .iter()
        .filter_map(|root| match generated_origin(root.node) {
            Some((owner, PseudoElem::Marker)) if document.list_marker_image(owner).is_some() => {
                Some((*root, None))
            }
            None if crate::generated_content::inside_marker_in_flow(cascade, root.node)
                && document.list_marker_image(root.node).is_some() =>
            {
                let lines = PositionedLines::new(document, cascade, root.node, None)?;
                let markers: Vec<_> = lines
                    .lines()
                    .flat_map(|line| {
                        let top = line.offset.1 + line.line.block_offset();
                        let height = line.line.block_size();
                        line.markers.into_iter().map(move |marker| {
                            (
                                top,
                                height,
                                PositionedMarker {
                                    owner: marker.owner,
                                    rect: PaintRect::new(
                                        line.offset.0 + marker.rect.x,
                                        line.offset.1 + marker.rect.y,
                                        marker.rect.width,
                                        marker.rect.height,
                                    ),
                                },
                            )
                        })
                    })
                    .collect();
                Some((*root, Some(markers)))
            }
            _ => None,
        })
        .collect();
    if roots.is_empty() {
        return placements;
    }
    let mut first_pages = BTreeMap::new();
    for page in pages {
        for item in &page.items {
            if item.fragment_index == 0 {
                first_pages.entry(item.node_id).or_insert(page.page_index);
            }
        }
    }
    for page in pages {
        let mut markers = BTreeMap::new();
        for (root, inline_markers) in &roots {
            let mut root = *root;
            let source = generated_origin(root.node)
                .map_or_else(|| document.ifc_source_owner(root.node), |(owner, _)| owner);
            if let Some(shift) = document
                .table_objects
                .headers
                .shift(source, page.content_origin_y)
            {
                let Some(shift) = shift else { continue };
                root.y += shift - page.content_origin_y;
                root.is_repeat = true;
            }
            let x = page.content_box.x + root.x;
            let y = page.content_box.y + root.y
                - if root.is_repeat {
                    0.0
                } else {
                    page.content_origin_y
                };
            if let Some((owner, PseudoElem::Marker)) = generated_origin(root.node) {
                let id = NodeId::new(owner as u64);
                if !root.is_repeat && first_pages.get(&id) != Some(&page.page_index) {
                    continue;
                }
                let Some(size) = document.list_marker_image_size(owner) else {
                    continue;
                };
                let Some(style) = computed_for_id(cascade, root.node)
                    .filter(|style| style.visibility == Visibility::Visible)
                else {
                    continue;
                };
                let node = &document.nodes[owner];
                let layout = node.unrounded_layout;
                if !node.children.is_empty()
                    && (layout.size.width <= 0.0 || layout.size.height <= 0.0)
                {
                    continue;
                }
                markers.insert(
                    id,
                    document.standalone_marker_image_rect(
                        style,
                        owner,
                        (x, y),
                        layout.padding.left,
                        size,
                    ),
                );
            } else if let Some(inline_markers) = inline_markers {
                for &(top, height, marker) in inline_markers {
                    if !root.is_repeat {
                        let Some((start, end)) = page.flow_range else {
                            continue;
                        };
                        if !crate::layout::line_center_on_page(
                            root.y + top,
                            root.y + top + height,
                            start,
                            end,
                        ) {
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
        placements.insert(page.page_index, markers);
    }
    placements
}
