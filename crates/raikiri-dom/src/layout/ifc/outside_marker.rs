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

// Only the accepted first line records a parent's gap. Other lines keep
// their full advance; cached parent layouts retain this sparse record.
#[derive(Clone, Debug, Default)]
pub(crate) struct LeadingStore {
    owners: std::collections::HashMap<usize, ((usize, usize), f32)>,
    lines: std::collections::HashMap<(usize, usize), (f32, usize)>,
}

impl LeadingStore {
    pub(crate) fn clear(&mut self, owner: usize) {
        if let Some((source, gap)) = self.owners.remove(&owner) {
            let value = self.lines.get_mut(&source).expect("recorded marker line");
            value.0 -= gap;
            value.1 -= 1;
            if value.1 == 0 {
                self.lines.remove(&source);
            }
        }
    }

    fn record(&mut self, owner: usize, source: (usize, usize), gap: f32) {
        self.clear(owner);
        if gap > 0.0 {
            self.owners.insert(owner, (source, gap));
            let value = self.lines.entry(source).or_default();
            value.0 += gap;
            value.1 += 1;
        }
    }
}

pub(crate) fn record_child(
    doc: &mut Document,
    owner: usize,
    gap: f32,
    source: Option<(usize, usize)>,
) {
    if let Some(source) = source {
        doc.outside_marker_leading.record(owner, source, gap);
    }
}

pub(crate) fn fit_leading(doc: &Document, root: usize, line: usize) -> f32 {
    doc.outside_marker_leading
        .lines
        .get(&(root, line))
        .map_or(0.0, |value| value.0)
}

#[cfg(test)]
mod tests;
