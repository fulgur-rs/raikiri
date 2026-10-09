//! Prepared marker placements shared by page paint events and raster consumers.

use super::records::{PageFragment, ProjectedTextRoot};
use crate::generated_content::{computed_for_id, generated_origin};
use crate::{Document, PositionedMarker};
use raikiri_style::{CascadeResult, PseudoElem, property::Visibility};
use raikiri_traits::{NodeId, PaintRect};
use std::collections::BTreeMap;

enum MarkerRoot {
    Standalone {
        owner: usize,
        size: raikiri_traits::ImageRasterSize,
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
) -> Result<BTreeMap<u32, BTreeMap<NodeId, PaintRect>>, raikiri_traits::LayoutError> {
    let mut placements = BTreeMap::new();
    // Only IFC roots themselves can own an atomic image marker. Extract their
    // source placements once; ordinary paragraphs need no additional glyph walk.
    let mut prepared = Vec::new();
    for root in roots {
        work.check()?;
        match generated_origin(root.node) {
            Some((owner, PseudoElem::Marker)) if document.list_marker_image(owner).is_some() => {
                if let Some(size) = document.list_marker_image_size(owner) {
                    work.charge(1)?;
                    prepared.push((*root, MarkerRoot::Standalone { owner, size }));
                }
            }
            None if crate::generated_content::inside_marker_in_flow(cascade, root.node)
                && document.list_marker_image(root.node).is_some() =>
            {
                if let Some(paragraph) = paragraphs.get(&root.node) {
                    let markers = paragraph.markers(root.fragmentainer);
                    work.charge(markers.len().saturating_add(1))?;
                    prepared.push((*root, MarkerRoot::Inline(markers.to_vec())));
                }
            }
            _ => {}
        }
    }
    let roots = prepared;
    if roots.is_empty() {
        return Ok(placements);
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
        work.check()?;
        let mut markers = BTreeMap::new();
        for (root, kind) in &roots {
            work.charge(1)?;
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
            match kind {
                MarkerRoot::Standalone { owner, size } => {
                    let owner = *owner;
                    let id = NodeId::new(owner as u64);
                    if !root.is_repeat && first_pages.get(&id) != Some(&page.page_index) {
                        continue;
                    }
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
                            *size,
                        ),
                    );
                }
                MarkerRoot::Inline(inline_markers) => {
                    for &(top, height, marker) in inline_markers {
                        work.charge(1)?;
                        if !root.is_repeat {
                            // A page without flow geometry cannot own a normal-flow line.
                            let (start, end) = page.flow_range.unwrap_or((0.0, 0.0));
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
        }
        placements.insert(page.page_index, markers);
    }
    Ok(placements)
}
