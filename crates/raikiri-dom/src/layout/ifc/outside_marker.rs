//! Additional leading contributed by an outside image marker.

use crate::Document;
use raikiri_style::CascadeResult;

/// The replaced marker's ascent, when it participates in horizontal flow.
pub(crate) fn height(doc: &Document, cascade: &CascadeResult, owner: usize) -> Option<f32> {
    if !crate::generated_content::marker_is_enabled(cascade, owner)
        || crate::generated_content::inside_marker_in_flow(cascade, owner)
        || cascade.computed[owner].writing_mode.is_vertical()
        || doc
            .nodes
            .get(owner)
            .is_some_and(|node| node.multicol.is_some())
    {
        return None;
    }
    doc.list_marker_image_size(owner).map(|size| size.height)
}

/// Chromium places extra leading before the first line or baseline-bearing
/// block, retaining its internal vertical alignment and even negative descent.
pub(crate) fn leading(height: Option<f32>, baseline: f32) -> f32 {
    height.map_or(0.0, |height| (height - baseline).max(0.0))
}

/// The outside marker changes advance and content placement, while Chromium
/// fragments using the line's original fit height. Overflow is page-clipped.
pub(crate) fn fit_height(line: &shodo::Line) -> f32 {
    line.block_size() - line.block_start_leading()
}

// Accepted translations index both the first line's fit coordinates and
// the receiving box's normal float geometry. Cached layouts retain them.
#[derive(Clone, Debug)]
struct ChildLeading {
    child: usize,
    source: Option<(usize, usize)>,
    gap: f32,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct LeadingStore {
    owners: std::collections::HashMap<usize, ChildLeading>,
    lines: std::collections::HashMap<(usize, usize), (f32, usize)>,
    boxes: std::collections::HashMap<usize, (f32, usize)>,
}

impl LeadingStore {
    pub(crate) fn clear(&mut self, owner: usize) {
        if let Some(applied) = self.owners.remove(&owner) {
            if let Some(source) = applied.source {
                let value = self.lines.get_mut(&source).expect("recorded marker line");
                value.0 -= applied.gap;
                value.1 -= 1;
                if value.1 == 0 {
                    self.lines.remove(&source);
                }
            }
            let value = self
                .boxes
                .get_mut(&applied.child)
                .expect("recorded marker box");
            value.0 -= applied.gap;
            value.1 -= 1;
            if value.1 == 0 {
                self.boxes.remove(&applied.child);
            }
        }
    }

    fn record(&mut self, owner: usize, child: usize, gap: f32, source: Option<(usize, usize)>) {
        self.clear(owner);
        if gap > 0.0 {
            self.owners
                .insert(owner, ChildLeading { child, source, gap });
            if let Some(source) = source {
                let value = self.lines.entry(source).or_default();
                value.0 += gap;
                value.1 += 1;
            }
            let value = self.boxes.entry(child).or_default();
            value.0 += gap;
            value.1 += 1;
        }
    }
}

pub(crate) fn record_child(
    doc: &mut Document,
    owner: usize,
    child: usize,
    gap: f32,
    source: Option<(usize, usize)>,
) {
    doc.outside_marker_leading.record(owner, child, gap, source);
}

/// Translation of a child box excluded from its escaped floats' normal geometry.
pub(crate) fn box_leading(doc: &Document, child: usize) -> f32 {
    doc.outside_marker_leading
        .boxes
        .get(&child)
        .map_or(0.0, |value| value.0)
}

pub(crate) fn fit_leading(doc: &Document, root: usize, line: usize) -> f32 {
    doc.outside_marker_leading
        .lines
        .get(&(root, line))
        .map_or(0.0, |value| value.0)
}

#[cfg(test)]
mod tests;
