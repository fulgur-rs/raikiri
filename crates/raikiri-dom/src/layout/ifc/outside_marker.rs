//! Additional leading contributed by an outside marker.

use crate::Document;
use raikiri_style::property::{ContentComponent, ListStyleType};
use raikiri_style::{CascadeResult, ComputedValues, PseudoElem, StyleNodeId};
use shodo::font::FontCollection;
use shodo::limits::Limits;

/// How far the outside marker of `owner` reaches above the baseline of the
/// first line, when it participates in horizontal flow: the ascent of an
/// image, or the room the marker's text takes above its baseline with the
/// marker's own font and `line-height`. The fonts are the document's.
pub(crate) fn height(doc: &Document, cascade: &CascadeResult, owner: usize) -> Option<f32> {
    height_in(
        doc,
        cascade,
        doc.ifc.as_ref().map(|state| (&state.fonts, &state.limits)),
        owner,
    )
}

/// [`height`] with the marker's text measured in `shaping`, the fonts and
/// limits of the document.
pub(crate) fn height_in(
    doc: &Document,
    cascade: &CascadeResult,
    shaping: Option<(&FontCollection, &Limits)>,
    owner: usize,
) -> Option<f32> {
    if !takes_part(doc, cascade, owner) {
        return None;
    }
    if let Some(size) = doc.list_marker_image_size(owner) {
        return Some(size.height);
    }
    let fallback;
    let marker = match cascade
        .pseudo
        .get(&(StyleNodeId::new(owner as u64), PseudoElem::Marker))
    {
        Some(marker) => &**marker,
        None => {
            fallback = ComputedValues::inherit_marker_from(&cascade.computed[owner]);
            &fallback
        }
    };
    let sample = sample_text(marker)?;
    let (fonts, limits) = shaping?;
    let inline = super::style::inline_style(marker, owner, fonts).ok()?;
    super::standalone::line_ascent(fonts, limits, &sample, &inline)
}

/// Whether `owner` has an outside marker in horizontal flow whose leading
/// this module places.
fn takes_part(doc: &Document, cascade: &CascadeResult, owner: usize) -> bool {
    crate::generated_content::marker_is_enabled(cascade, owner)
        && !crate::generated_content::inside_marker_in_flow(cascade, owner)
        && !cascade.computed[owner].writing_mode.is_vertical()
        && doc
            .nodes
            .get(owner)
            .is_none_or(|node| node.multicol.is_none())
}

/// Text that stands for what the marker with the values `marker` shows, so
/// that its glyphs pick the fonts that set the marker's line: its literal
/// strings, its bullet, or a digit for a counter. `None` when it shows
/// nothing, as an empty string or `list-style-type: none` do, since an empty
/// line takes no room.
fn sample_text(marker: &ComputedValues) -> Option<String> {
    let sample: String = if marker.content.is_empty() {
        match &marker.list_style_type {
            ListStyleType::None => String::new(),
            ListStyleType::String(text) => text.to_string(),
            other => {
                // `disc`, the initial value, is the only keyword not kept as
                // a name.
                let name = match other {
                    ListStyleType::Named(name) => name.to_ascii_lowercase(),
                    _ => "disc".to_owned(),
                };
                match name.as_str() {
                    "disc" => "\u{2022}",
                    "circle" => "\u{25E6}",
                    "square" => "\u{25AA}",
                    _ => "0",
                }
                .to_owned()
            }
        }
    } else {
        marker
            .content
            .iter()
            .map(|part| match part {
                ContentComponent::Literal(text) => text.as_str(),
                // An image alone still gives the marker a line, and its
                // strut, in the marker's font.
                ContentComponent::Image { .. } => " ",
                _ => "0",
            })
            .collect()
    };
    (!sample.is_empty()).then_some(sample)
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
