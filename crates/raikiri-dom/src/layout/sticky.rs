//! Offsets of sticky boxes (CSS Position 3 §3.4).
//!
//! A sticky box is laid out in normal flow like a static one. Its offset
//! depends on where layout put it: the box moves only as far as it must to
//! keep its insets from the edges of its sticky view rectangle, and never
//! out of its containing block. A document is printed at its initial scroll
//! position, so the view rectangle is the content box of the nearest scroll
//! container around the box, or the page area when there is none. Chromium
//! measures all of it on the unfragmented flow, before the pages cut it.
//!
//! The offsets are found once layout is done, and the painter applies them
//! as it applies the relative offsets layout leaves to it.

use super::box_tree::BoxTree;
use super::context::LayoutSession;
use super::page::used_computed_length_percentage_or_auto;
use crate::document::Document;
use raikiri_style::property::{DisplayValue, PositionValue};
use raikiri_style::{CascadeResult, ComputedLengthPercentageOrAuto};
use std::collections::HashMap;

/// A rectangle by its edges.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Area {
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
}

impl Area {
    fn translated(self, (dx, dy): (f32, f32)) -> Self {
        Self {
            left: self.left + dx,
            top: self.top + dy,
            right: self.right + dx,
            bottom: self.bottom + dy,
        }
    }
}

/// Find the offset of every sticky box laid out in `document` and keep them
/// for the painter, replacing those of an earlier layout.
pub(super) fn resolve_offsets(document: &mut LayoutSession<'_>, cascade: &CascadeResult) {
    document.sticky_offsets.clear();
    let mut sticky: Vec<(usize, usize)> = (0..document.nodes.len())
        .filter(|&id| is_sticky_box(document, id))
        .map(|id| (document.ancestors(id).count(), id))
        .collect();
    if sticky.is_empty() {
        return;
    }
    // A box is measured where the sticky boxes around it moved it, so those
    // are resolved first.
    sticky.sort_unstable();
    let mut offsets = HashMap::with_capacity(sticky.len());
    for (_, id) in sticky {
        if let Some(offset) = offset_of(document, cascade, &offsets, id)
            && offset != (0.0, 0.0)
        {
            offsets.insert(id, offset);
        }
    }
    document.sticky_offsets = offsets;
}

/// Whether `id` is a sticky element that generates a box other than a
/// non-atomic inline, outside any multi-column container. The text of such
/// an inline is placed from the line it sits on, which carries relative
/// offsets only; the columns place their boxes after the flow is laid out.
/// Either keeps its place.
fn is_sticky_box(document: &Document, id: usize) -> bool {
    let node = &document.nodes[id];
    node.kind() == raikiri_traits::NodeKind::Element
        && *node.position() == PositionValue::Sticky
        && !matches!(node.display(), DisplayValue::Contents | DisplayValue::None)
        && !super::relative::is_non_atomic_inline(document, BoxTree::for_layout(document), id)
        && !document
            .ancestors(id)
            .any(|ancestor| document.nodes[ancestor].is_multicol_container())
}

/// How far the sticky boxes among the inclusive ancestors of `id` moved it.
/// The painter places a fixed box in the page area apart from the offsets
/// around it, so those above such a box do not move what it holds.
fn moved_by(
    document: &Document,
    cascade: &CascadeResult,
    offsets: &HashMap<usize, (f32, f32)>,
    id: usize,
) -> (f32, f32) {
    document
        .inclusive_ancestors(id)
        .take_while(|&ancestor| !super::fixed::is_page_fixed(document, cascade, ancestor))
        .filter_map(|ancestor| offsets.get(&ancestor))
        .fold((0.0, 0.0), |(x, y), &(dx, dy)| (x + dx, y + dy))
}

/// The border box of `id` where layout put it, and its content box.
fn boxes_of(document: &Document, id: usize) -> Option<(Area, Area)> {
    let origin = document.layout_origin_of(id)?;
    let layout = &document.ifc_layout_node(id)?.layout;
    let border_box = Area {
        left: origin.x,
        top: origin.y,
        right: origin.x + layout.size.width,
        bottom: origin.y + layout.size.height,
    };
    let left = border_box.left + layout.border.left + layout.padding.left;
    let top = border_box.top + layout.border.top + layout.padding.top;
    let content_box = Area {
        left,
        top,
        right: left + layout.content_box_width(),
        bottom: top + layout.content_box_height(),
    };
    Some((border_box, content_box))
}

/// The sticky view rectangle of `id`: the content box of the nearest scroll
/// container around it, or the page area, and whether it is the page area.
fn view_of(
    document: &LayoutSession<'_>,
    cascade: &CascadeResult,
    offsets: &HashMap<usize, (f32, f32)>,
    id: usize,
) -> Option<(Area, bool)> {
    // A fixed box takes what it holds out of the scroll containers around
    // it, to the page area, unless it is one itself. Chromium prints a box in
    // a fixed box that a transform contains against the page area too.
    let mut scroller = None;
    for ancestor in document.ancestors(id) {
        if cascade.computed.get(ancestor).is_some_and(|computed| {
            super::page_pipeline::scrolls_overflow(document, computed, ancestor)
        }) {
            scroller = Some(ancestor);
            break;
        }
        if *document.nodes[ancestor].position() == PositionValue::Fixed {
            break;
        }
    }
    if let Some(scroller) = scroller {
        let (_, content_box) = boxes_of(document, scroller)?;
        // Its scrollbars cover the far edges of the view.
        let scrollbars = document.ifc_layout_node(scroller)?.layout.scrollbar_size;
        let view = Area {
            right: content_box.right - scrollbars.width,
            bottom: content_box.bottom - scrollbars.height,
            ..content_box
        };
        return Some((
            view.translated(moved_by(document, cascade, offsets, scroller)),
            false,
        ));
    }
    let (width, height) = document.fixed_page_area?;
    let page_area = Area {
        left: 0.0,
        top: 0.0,
        right: width,
        bottom: height,
    };
    Some((page_area, true))
}

/// The offset of the sticky box `id`, given those of the sticky boxes
/// around it; `None` when it has no box to measure.
fn offset_of(
    document: &LayoutSession<'_>,
    cascade: &CascadeResult,
    offsets: &HashMap<usize, (f32, f32)>,
    id: usize,
) -> Option<(f32, f32)> {
    let computed = cascade.computed.get(id)?;
    let around = document.parent_of(id).map_or((0.0, 0.0), |parent| {
        moved_by(document, cascade, offsets, parent)
    });
    let (border_box, _) = boxes_of(document, id)?;
    let sticky_box = border_box.translated(around);
    let block = super::relative::containing_block(document, BoxTree::for_layout(document), id)?;
    let (_, block_content) = boxes_of(document, block.key)?;
    // The box stays inside its containing block with its margins.
    let margin = document.ifc_layout_node(id)?.layout.margin;
    let block_content = block_content.translated(moved_by(document, cascade, offsets, block.key));
    let limits = Area {
        left: block_content.left + margin.left,
        top: block_content.top + margin.top,
        right: block_content.right - margin.right,
        bottom: block_content.bottom - margin.bottom,
    };
    let (view, in_page_area) = view_of(document, cascade, offsets, id)?;
    // Percentages are of the view rectangle.
    let inset = |value: &ComputedLengthPercentageOrAuto, basis: f32| {
        used_computed_length_percentage_or_auto(value, basis)
    };
    let width = view.right - view.left;
    let height = view.bottom - view.top;
    // Chromium pulls a box that does not lie wholly on the first page back
    // onto it with its `bottom` (or, when the pages run along the x axis,
    // its `right`), over content the pages cut there. The painter draws a
    // box on the page the flow cuts it onto, from where such a pull would
    // move it off the page, so that box keeps its place.
    let right = if in_page_area && sticky_box.right > view.right {
        None
    } else {
        inset(&computed.right, width)
    };
    let bottom = if in_page_area && sticky_box.bottom > view.bottom {
        None
    } else {
        inset(&computed.bottom, height)
    };
    let x = axis_offset(
        (sticky_box.left, sticky_box.right),
        (limits.left, limits.right),
        (view.left, view.right),
        (inset(&computed.left, width), right),
    );
    let y = axis_offset(
        (sticky_box.top, sticky_box.bottom),
        (limits.top, limits.bottom),
        (view.top, view.bottom),
        (inset(&computed.top, height), bottom),
    );
    Some((x, y))
}

/// The offset of a sticky box along one axis, from its edges `(start, end)`,
/// the edges its margin box stays within, the edges of its view rectangle
/// and its `(start, end)` insets. As in Chromium, each inset is measured
/// from where layout put the box and the two moves add up; neither moves the
/// box out of its limits, nor further out when it already stands outside.
fn axis_offset(
    (start, end): (f32, f32),
    (limit_start, limit_end): (f32, f32),
    (view_start, view_end): (f32, f32),
    (inset_start, inset_end): (Option<f32>, Option<f32>),
) -> f32 {
    let mut offset = 0.0;
    if let Some(inset) = inset_end {
        let toward_start = (view_end - inset - end).min(0.0);
        let room = (limit_start - start).min(0.0);
        offset += toward_start.max(room);
    }
    if let Some(inset) = inset_start {
        let toward_end = (view_start + inset - start).max(0.0);
        let room = (limit_end - end).max(0.0);
        offset += toward_end.min(room);
    }
    offset
}

#[cfg(test)]
mod tests;
