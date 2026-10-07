//! Positioned glyph runs of the paragraphs on a page, for a painter that
//! draws text itself.

use super::records::{PageFragment, ProjectedTextRoot};
use crate::generated_content::generated_origin;
use crate::layout::{PositionedLine, PositionedLines, PositionedRun, line_center_on_page};
use crate::{Document, relative_offset};
use raikiri_style::property::{CssColor, DisplayValue, PositionValue};
use raikiri_style::resolve::ComputedLengthPercentageOrAuto;
use raikiri_style::{CascadeResult, ComputedValues, PseudoElem};
use raikiri_traits::{NodeId, NodeKind};
use shodo::geometry::WritingMode;
use std::collections::HashSet;
use std::fmt;
use std::ops::Range;
use std::sync::Arc;

/// One run of glyphs in one font and color, positioned on the page.
///
/// Positions are in CSS px with the origin at the top-left of the page box and
/// y growing downwards. Glyph `i` is drawn at
/// `origin + (advance[0] + … + advance[i - 1], 0) + (x_offset[i], y_offset[i])`,
/// where the sums run over [`Self::glyphs`] in order. Glyphs are listed left to
/// right, so a right-to-left run lists its last logical glyph first.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct PositionedGlyphRun<'a> {
    /// What the text of the run comes from.
    pub source: RunSource,
    /// The font face.
    pub font: FontRef,
    /// Font size in CSS px.
    pub font_size: f32,
    /// Variation axis settings the run was shaped with.
    pub variations: Vec<FontVariation>,
    /// Normalized variation coordinates in the face's axis order, as
    /// F2Dot14 bits.
    pub normalized_coords: Vec<i16>,
    /// Synthetic bold and oblique applied to the face.
    pub synthesis: Synthesis,
    /// Pen position of the first glyph on the baseline.
    pub origin: (f32, f32),
    /// Inline size of the run.
    pub advance: f32,
    /// Ascent of the font at the run's size, above the baseline.
    pub ascent: f32,
    /// Descent of the font at the run's size, as a positive distance below
    /// the baseline.
    pub descent: f32,
    /// The run's part of the processed text: after white-space collapsing
    /// and text transformation, as it was shaped.
    pub text: &'a str,
    /// The glyphs, left to right.
    pub glyphs: Vec<Glyph>,
    /// Text color (used value).
    pub color: CssColor,
}

/// One glyph of a [`PositionedGlyphRun`].
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Glyph {
    /// Glyph id in the run's font.
    pub id: u32,
    /// Advance to the next glyph's pen position.
    pub advance: f32,
    /// Horizontal displacement from the pen position.
    pub x_offset: f32,
    /// Vertical displacement from the pen position, positive downwards.
    pub y_offset: f32,
    /// UTF-8 byte range of [`PositionedGlyphRun::text`] the glyph shows.
    /// Glyphs of one cluster share a range; a ligature covers every
    /// character it joins.
    pub text_range: Range<usize>,
}

/// What the text of a run comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RunSource {
    /// A text node.
    Text(NodeId),
    /// The generated content of a pseudo-element, by its originating element.
    Generated(NodeId, GeneratedKind),
    /// The `text-overflow` ellipsis of a block container, by that container
    /// (CSS Overflow 3 §5.1). Its text is `…`, or periods when the font has
    /// no ellipsis character.
    Ellipsis(NodeId),
}

/// The pseudo-element generated text comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum GeneratedKind {
    /// `::before`.
    Before,
    /// `::after`.
    After,
    /// `::marker`.
    Marker,
}

/// A font face and its data.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct FontRef {
    /// Identifies the face.
    pub id: FontId,
    /// The font file.
    pub data: FontBlob,
    /// Index of the face in a font collection file, 0 otherwise.
    pub index: u32,
}

/// Identifies a font face. Equal for runs in the same face of the same font
/// collection, and unchanged for as long as that collection is in use, so it
/// can key a painter's font cache together with the run's variations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FontId {
    blob: u64,
    index: u32,
}

/// The bytes of a font file, shared without copying.
#[derive(Clone)]
pub struct FontBlob(Arc<dyn AsRef<[u8]> + Send + Sync>);

impl FontBlob {
    /// The font file.
    pub fn as_bytes(&self) -> &[u8] {
        (*self.0).as_ref()
    }

    /// The shared font file, to hand to another library without copying.
    pub fn to_arc(&self) -> Arc<dyn AsRef<[u8]> + Send + Sync> {
        Arc::clone(&self.0)
    }
}

impl fmt::Debug for FontBlob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FontBlob")
            .field("len", &self.as_bytes().len())
            .finish()
    }
}

/// An OpenType tag such as a variation axis name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Tag(pub [u8; 4]);

/// One variation axis setting.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct FontVariation {
    /// Axis tag, such as `wght`.
    pub tag: Tag,
    /// Axis value in user units.
    pub value: f32,
}

/// Synthetic styling applied when the face lacks the requested weight or
/// style.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct Synthesis {
    /// Draw the outlines bolder.
    pub embolden: bool,
    /// Slant the outlines by this angle in degrees.
    pub skew: Option<f32>,
}

/// Why the runs of a paragraph are not reported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TextRunOmission {
    VerticalWritingMode,
    MulticolLines,
    RelativeInlineOffset,
    AncestorOffset,
    FixedPlacement,
}

impl TextRunOmission {
    fn describe(self) -> &'static str {
        match self {
            Self::VerticalWritingMode => "its writing mode is vertical",
            Self::MulticolLines => "it is laid out in the columns of a multicol container",
            Self::RelativeInlineOffset => "it has relatively positioned inline elements",
            Self::AncestorOffset => {
                "it or an ancestor is moved by a transform or relative positioning"
            }
            Self::FixedPlacement => {
                "it is inside a fixed box not placed by `top` and `left` lengths"
            }
        }
    }
}

/// What every paragraph of one document shares when its runs are placed.
struct RunContext {
    /// Nodes placed through the fragment tree of a multicol container.
    fragmented: HashSet<usize>,
}

impl RunContext {
    fn new(document: &Document) -> Self {
        Self {
            fragmented: document
                .layout_fragments()
                .iter()
                .map(|fragment| fragment.node_id)
                .collect(),
        }
    }
}

/// What about `cv`'s box moves the paragraphs inside it at paint time, if
/// anything does.
fn box_omission(cv: &ComputedValues) -> Option<TextRunOmission> {
    // An inline-level box's relative offset is already part of its layout
    // location.
    let inline_level = matches!(
        cv.display,
        DisplayValue::Inline
            | DisplayValue::InlineBlock
            | DisplayValue::InlineFlex
            | DisplayValue::InlineGrid
            | DisplayValue::InlineTable
    );
    let relative = cv.position == PositionValue::Relative
        && !inline_level
        && relative_offset(cv) != Some((0.0, 0.0));
    if !cv.transform.is_empty() || relative {
        return Some(TextRunOmission::AncestorOffset);
    }
    // The painter places a fixed box on each page from its `top` and `left`
    // lengths, and from other insets differently than layout.
    let length = |inset| matches!(inset, ComputedLengthPercentageOrAuto::Px(_));
    (cv.position == PositionValue::Fixed && !(length(cv.left) && length(cv.top)))
        .then_some(TextRunOmission::FixedPlacement)
}

/// Whether the runs of `root` can be positioned from its layout alone, or
/// what moves them at paint time that the runs do not model yet.
fn omission(
    document: &Document,
    cascade: &CascadeResult,
    context: &RunContext,
    root: usize,
) -> Option<TextRunOmission> {
    let node = document.get_node(root)?;
    if node
        .ifc_writing_mode()
        .is_some_and(|mode| mode != WritingMode::HorizontalTb)
    {
        return Some(TextRunOmission::VerticalWritingMode);
    }
    if node
        .ifc_relative_offsets()
        .iter()
        .any(|(_, offset)| *offset != (0.0, 0.0))
    {
        return Some(TextRunOmission::RelativeInlineOffset);
    }
    let mut current = Some(root);
    while let Some(id) = current {
        // The painter places a fragmented box, and everything inside it, from
        // its column's fragment rather than from its layout.
        if context.fragmented.contains(&id) {
            return Some(TextRunOmission::MulticolLines);
        }
        if let Some(reason) = cascade.computed.get(id).and_then(box_omission) {
            return Some(reason);
        }
        current = document.parent_of(id);
    }
    node.ifc_multicol_fragments()
        .map(|_| TextRunOmission::MulticolLines)
}

/// The raikiri-owned form of a shodo variation axis setting.
fn font_variation(variation: &shodo::style::FontVariation) -> FontVariation {
    FontVariation {
        tag: Tag(variation.tag),
        value: variation.value,
    }
}

/// The source of a run painted for `owner`, or `None` for an element that
/// supplied text of its own (such as the zero-width space of `<wbr>`).
fn run_source(document: &Document, owner: usize) -> Option<RunSource> {
    if let Some((element, pseudo)) = generated_origin(owner) {
        let kind = match pseudo {
            PseudoElem::After => GeneratedKind::After,
            PseudoElem::Marker => GeneratedKind::Marker,
            PseudoElem::Before | PseudoElem::FirstLine => GeneratedKind::Before,
            // Retained typographic boxes are projected with their original
            // text owner; unsupported box pseudos never supply text sources.
            PseudoElem::FirstLetter | PseudoElem::Backdrop | PseudoElem::FileSelectorButton => {
                return None;
            }
        };
        return Some(RunSource::Generated(NodeId::new(element as u64), kind));
    }
    (document.get_node(owner)?.kind() == NodeKind::Text)
        .then(|| RunSource::Text(NodeId::new(owner as u64)))
}

/// The text of an ellipsis run of `glyphs` glyphs, with each glyph's range.
/// The run holds no text of the line: it is one `…` glyph, or one period per
/// glyph when the font has no ellipsis character (`periods`, or a run of
/// more than one glyph).
fn ellipsis_text(glyphs: usize, periods: bool) -> (&'static str, Vec<Range<usize>>) {
    if glyphs == 1 && !periods {
        // The ellipsis character is three bytes long.
        return ("\u{2026}", std::iter::once(0..3).collect());
    }
    let count = glyphs.clamp(1, 3);
    let ranges = (0..glyphs).map(|index| {
        let index = index.min(count - 1);
        index..index + 1
    });
    (&"..."[..count], ranges.collect())
}

/// Byte ranges of `run_text` (the run's slice, starting at `run_start` in the
/// line's text) for glyphs with the given cluster starts, in logical order.
/// A cluster ends where the next cluster of the run starts.
fn glyph_text_ranges(clusters: &[u32], run_start: usize, run_end: usize) -> Vec<Range<usize>> {
    let mut starts: Vec<u32> = clusters.to_vec();
    starts.sort_unstable();
    starts.dedup();
    let len = run_end.saturating_sub(run_start);
    clusters
        .iter()
        .map(|&cluster| {
            let next = starts.partition_point(|&start| start <= cluster);
            let end = starts.get(next).map_or(run_end, |&start| start as usize);
            let start = (cluster as usize).saturating_sub(run_start).min(len);
            start..end.saturating_sub(run_start).clamp(start, len)
        })
        .collect()
}

/// Build the public run for one positioned run, with the line's origin on the
/// page at `(x, y)`.
fn glyph_run<'a>(
    document: &Document,
    line: &PositionedLine<'a>,
    positioned: &PositionedRun<'a>,
    (x, y): (f32, f32),
) -> Option<PositionedGlyphRun<'a>> {
    let run = &positioned.run;
    let ellipsis = run.is_ellipsis();
    let source = if ellipsis {
        RunSource::Ellipsis(NodeId::new(positioned.owner as u64))
    } else {
        run_source(document, positioned.owner)?
    };
    let color = positioned.style.color;
    let font = run.font_data()?;
    let index = font.index;
    let (bytes, blob) = font.data.into_raw_parts();
    let text_range = run.text_range();
    let shaped: Vec<shodo::Glyph> = run.glyphs().collect();
    let (text, ranges) = if ellipsis {
        // The periods of a fallback ellipsis are split into one run per font.
        let runs = line
            .line
            .fragments()
            .filter(|fragment| {
                matches!(fragment, shodo::Fragment::GlyphRun(other) if other.is_ellipsis())
            })
            .count();
        ellipsis_text(shaped.len(), runs > 1)
    } else {
        let clusters: Vec<u32> = shaped.iter().map(|glyph| glyph.cluster).collect();
        (
            line.line.text().get(text_range.clone()).unwrap_or_default(),
            glyph_text_ranges(&clusters, text_range.start, text_range.end),
        )
    };
    let (dx, dy) = (x + positioned.offset.0, y + positioned.offset.1);
    let mut placed: Vec<(f32, f32, &shodo::Glyph, Range<usize>)> = positioned
        .glyphs
        .iter()
        .zip(&shaped)
        .zip(ranges)
        .map(|((glyph, shaped), range)| (dx + glyph.x, dy + glyph.y, shaped, range))
        .collect();
    // Glyphs are stored in logical order; a right-to-left run is drawn from
    // its last logical glyph on the left.
    if run.bidi_level() % 2 == 1 {
        placed.reverse();
    }
    let origin = (
        placed.first()?.0,
        dy + line.line.block_offset() + run.baseline(),
    );
    let mut pen = origin.0;
    let glyphs = placed
        .into_iter()
        .map(|(glyph_x, glyph_y, shaped, text_range)| {
            let glyph = Glyph {
                id: shaped.id,
                advance: shaped.advance,
                x_offset: glyph_x - pen,
                y_offset: glyph_y - origin.1,
                text_range,
            };
            pen += shaped.advance;
            glyph
        })
        .collect();
    let metrics = run.metrics();
    Some(PositionedGlyphRun {
        source,
        font: FontRef {
            id: FontId { blob, index },
            data: FontBlob(bytes),
            index,
        },
        font_size: run.font_size(),
        variations: run.variations().iter().map(font_variation).collect(),
        normalized_coords: run
            .normalized_coords()
            .iter()
            .map(|coord| coord.to_bits())
            .collect(),
        synthesis: Synthesis {
            embolden: run.embolden(),
            skew: run.skew(),
        },
        origin,
        advance: run.inline_size(),
        ascent: metrics.ascent,
        descent: metrics.descent,
        text,
        glyphs,
        color,
    })
}

/// Append the runs of `root` that belong to `page`.
fn root_runs<'a>(
    document: &'a Document,
    cascade: &'a CascadeResult,
    page: &PageFragment,
    context: &RunContext,
    root: &ProjectedTextRoot,
    out: &mut Vec<PositionedGlyphRun<'a>>,
) -> Option<()> {
    // A paragraph that is not repeated needs its page's slice of the flow.
    let flow_range = if root.is_repeat {
        None
    } else {
        Some(page.flow_range?)
    };
    if omission(document, cascade, context, root.node).is_some() {
        return None;
    }
    let positioned = PositionedLines::new(document, cascade, root.node, None)?;
    // A repeated paragraph sits at the same place on every page; any other
    // is placed in its page's slice of the flow.
    let x = page.content_box.x + root.x;
    let y = page.content_box.y + root.y - flow_range.map_or(0.0, |(start, _)| start);
    for line in positioned.lines() {
        if let Some((start, end)) = flow_range {
            let top = root.y + line.offset.1 + line.line.block_offset();
            if !line_center_on_page(top, top + line.line.block_size(), start, end) {
                continue;
            }
        }
        let line_origin = (x + line.offset.0, y + line.offset.1);
        out.extend(
            line.runs
                .iter()
                .filter_map(|run| glyph_run(document, &line, run, line_origin)),
        );
    }
    Some(())
}

impl Document {
    /// Positioned glyph runs of the paragraphs on one page, in document
    /// order. `cascade` is the one the document was laid out with.
    #[doc(hidden)]
    pub fn page_text_runs<'a>(
        &'a self,
        cascade: &'a CascadeResult,
        page_index: u32,
    ) -> Vec<PositionedGlyphRun<'a>> {
        let mut runs = Vec::new();
        let projection = &self.page_projection;
        let context = RunContext::new(self);
        let pages = projection.pages.iter();
        for page in pages.filter(|page| page.page_index == page_index) {
            for root in &projection.text_roots {
                root_runs(self, cascade, page, &context, root, &mut runs);
            }
        }
        runs
    }

    /// The paragraphs whose runs [`Document::page_text_runs`] leaves out, each
    /// with the reason.
    #[doc(hidden)]
    pub fn omitted_text_run_roots(&self, cascade: &CascadeResult) -> Vec<(NodeId, &'static str)> {
        let context = RunContext::new(self);
        self.page_projection
            .text_roots
            .iter()
            .filter_map(|root| {
                let reason = omission(self, cascade, &context, root.node)?;
                Some((NodeId::new(root.node as u64), reason.describe()))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
