//! Positioned glyph runs of the paragraphs on a page, for a painter that
//! draws text itself.

use super::records::{PageFragment, ProjectedTextRoot};
use crate::generated_content::generated_origin;
use crate::layout::{PositionedLine, PositionedLines, PositionedRun, line_center_on_page};
use crate::{Document, relative_offset};
use raikiri_style::property::{CssColor, DisplayValue, PositionValue, TextShadowColor};
use raikiri_style::resolve::ComputedLengthPercentageOrAuto;
use raikiri_style::{CascadeResult, ComputedValues, PseudoElem};
use raikiri_traits::{NodeId, NodeKind};
use shodo::geometry::WritingMode;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::ops::Range;
use std::sync::Arc;

#[derive(Clone)]
pub(super) struct MarkerText {
    shaped: Arc<crate::StandaloneText>,
    offset_x: f32,
    color: CssColor,
    shadows: Vec<TextShadow>,
    first_page: Option<u32>,
}

impl MarkerText {
    /// Number of stable line identities in the prepared standalone marker.
    pub(super) fn line_count(&self) -> usize {
        self.shaped.lines().len()
    }
}

impl fmt::Debug for MarkerText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MarkerText")
            .field("lines", &self.shaped.lines().len())
            .field("offset_x", &self.offset_x)
            .field("color", &self.color)
            .finish()
    }
}

pub(super) fn prepare_markers(
    document: &Document,
    cascade: &CascadeResult,
    roots: &[ProjectedTextRoot],
    pages: &[PageFragment],
) -> Result<BTreeMap<usize, MarkerText>, raikiri_traits::LayoutError> {
    let owners: Vec<_> = roots
        .iter()
        .filter_map(|root| {
            generated_origin(root.node)
                .filter(|(_, pseudo)| *pseudo == PseudoElem::Marker)
                .map(|(owner, _)| owner)
        })
        .collect();
    let mut markers = BTreeMap::new();
    if owners.is_empty() {
        return Ok(markers);
    }
    let snapshots = crate::counter_snapshots(document, cascade).map_err(|error| {
        raikiri_traits::LayoutError::CounterSnapshotLimitExceeded {
            limit: error.limit,
            actual: error.actual,
        }
    })?;
    let mut first_pages = BTreeMap::new();
    for page in pages {
        for item in &page.items {
            if item.fragment_index == 0 {
                first_pages
                    .entry(item.node_id.0 as usize)
                    .or_insert(page.page_index);
            }
        }
    }
    for owner in owners {
        let node = &document.nodes[owner];
        let layout = node.unrounded_layout;
        // Only a usable image replaces the list-style-type text fallback.
        if document.list_marker_image(owner).is_some() {
            continue;
        }
        if !node.children.is_empty() && (layout.size.width <= 0.0 || layout.size.height <= 0.0) {
            continue;
        }
        let Some((style, content)) =
            crate::generated_content::markers::marker_render_info_with_snapshots(
                document, cascade, owner, &snapshots,
            )
        else {
            continue;
        };
        if style.visibility == raikiri_style::property::Visibility::Hidden {
            continue;
        }
        if let Some((shaped, offset_x)) =
            document.shape_list_marker_text(&content, style, owner, layout.padding.left)
        {
            markers.insert(
                owner,
                MarkerText {
                    shaped: Arc::new(shaped),
                    offset_x,
                    color: style.color,
                    shadows: text_shadows(style),
                    first_page: first_pages.get(&owner).copied(),
                },
            );
        }
    }
    Ok(markers)
}

/// Identifies one line of a paragraph, independent of its paint coordinates.
///
/// Runs with different colors or fonts on the same line share this identity.
/// Coincident lines, including lines with zero line height, remain distinct.
/// The identity is scoped to the laid-out document and remains the same for
/// a repeated paragraph on different pages.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct TextLineId {
    /// The paragraph's inline formatting context root, or a synthetic key
    /// for a standalone list marker. Synthetic keys are not DOM nodes.
    pub root: NodeId,
    /// Zero-based line index in that paragraph, before page slicing.
    pub index: usize,
}

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
    /// The line containing this run, for painting decorations per line.
    pub line: TextLineId,
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
    /// Used decoration segments, including lines propagated from ancestors.
    /// Generated text and ellipses have no decoration segments in this version.
    pub decorations: Vec<crate::DecorationLine>,
    /// Used `text-shadow` list of the run's text (CSS Text Decoration 3 §4),
    /// in declaration order: the first shadow is painted on top, and every
    /// shadow is painted below the run's glyphs. Empty for `none` and for
    /// page-margin box text.
    pub shadows: Vec<TextShadow>,
}

/// One used `text-shadow` of a [`PositionedGlyphRun`].
///
/// The shadow is the run's glyphs filled with [`Self::color`], moved by
/// [`Self::offset`] and blurred as for `box-shadow` (CSS Backgrounds 3 §7.1):
/// a Gaussian blur with a standard deviation of half the blur radius.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct TextShadow {
    /// Horizontal and vertical offset in CSS px, with y growing downwards.
    pub offset: (f32, f32),
    /// Blur radius in CSS px; zero draws a sharp shadow.
    pub blur_radius: f32,
    /// Shadow color, with `currentcolor` resolved to the text color.
    pub color: CssColor,
}

/// The used `text-shadow` list of text styled by `style`.
pub(crate) fn text_shadows(style: &ComputedValues) -> Vec<TextShadow> {
    style
        .text_shadow
        .iter()
        .map(|shadow| TextShadow {
            offset: (shadow.offset_x.px(), shadow.offset_y.px()),
            blur_radius: shadow.blur_radius.px().max(0.0),
            color: match shadow.color {
                TextShadowColor::Resolved(color) => color,
                _ => style.color,
            },
        })
        .collect()
}

impl PositionedGlyphRun<'_> {
    /// Whether this run belongs to a list marker painted separately from
    /// the item's inline content. Such markers use a separate line identity
    /// and are painted before the item's own overflow clip, while remaining
    /// inside its ancestor clips and opacity group.
    pub fn is_standalone_marker(&self) -> bool {
        matches!(self.source, RunSource::Generated(_, GeneratedKind::Marker))
            && generated_origin(self.line.root.0 as usize)
                .is_some_and(|(_, pseudo)| pseudo == PseudoElem::Marker)
    }
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
    /// The generated content of a page-margin box (CSS Paged Media 3 §4.2).
    MarginBox(raikiri_style::PageMarginBoxSlot),
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
    OverlappingColumnLines,
    UnrootedColumnFragments,
    RelativeInlineOffset,
    AncestorOffset,
    FixedPlacement,
}

impl TextRunOmission {
    fn describe(self) -> &'static str {
        match self {
            Self::VerticalWritingMode => "its writing mode is vertical",
            Self::OverlappingColumnLines => {
                "the same paragraph line has overlapping column placements"
            }
            Self::UnrootedColumnFragments => "its column fragments have no rooted placement chain",
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

/// Paragraphs whose original line identities do not uniquely select a column placement.
#[derive(Debug, Default, Clone)]
pub(super) struct RunContext {
    overlapping: HashSet<usize>,
}

impl RunContext {
    pub(super) fn new(document: &Document, roots: &[ProjectedTextRoot]) -> Self {
        let mut placements = HashMap::<usize, HashMap<Option<usize>, usize>>::new();
        for root in roots {
            *placements
                .entry(root.node)
                .or_default()
                .entry(root.fragmentainer)
                .or_default() += 1;
        }
        let mut overlapping = HashSet::new();
        for (root, columns) in placements {
            if columns.values().sum::<usize>() < 2 {
                continue;
            }
            let Some(node) = document.ifc_layout_node(root) else {
                overlapping.insert(generated_origin(root).map_or(root, |(owner, _)| owner));
                continue;
            };
            let Some(ranges) = node.ifc_multicol_fragments() else {
                overlapping.insert(root);
                continue;
            };
            if columns.contains_key(&None) || columns.values().any(|&count| count > 1) {
                overlapping.insert(root);
                continue;
            }
            let mut intervals: Vec<_> = ranges
                .iter()
                .filter(|range| columns.contains_key(&Some(range.fragmentainer)))
                .map(|range| (range.line_start, range.line_end))
                .collect();
            intervals.sort_unstable();
            if intervals.windows(2).any(|pair| pair[0].1 > pair[1].0) {
                overlapping.insert(root);
            }
        }
        Self { overlapping }
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
pub(super) fn omission(
    document: &Document,
    cascade: &CascadeResult,
    context: &RunContext,
    root: usize,
) -> Option<TextRunOmission> {
    let root = generated_origin(root).map_or(root, |(owner, _)| owner);
    let node = document.ifc_layout_node(root)?;
    if context.overlapping.contains(&root) {
        return Some(TextRunOmission::OverlappingColumnLines);
    }
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
    let mut current = Some(document.ifc_source_owner(root));
    while let Some(id) = current {
        if let Some(reason) = cascade.computed.get(id).and_then(box_omission) {
            return Some(reason);
        }
        current = document.parent_of(id);
    }
    None
}

fn marker_runs<'a>(
    document: &'a Document,
    page: &PageFragment,
    root: &ProjectedTextRoot,
    owner: usize,
    out: &mut Vec<PositionedGlyphRun<'a>>,
) {
    let Some(marker) = document.page_projection.markers.get(&owner) else {
        return;
    };
    // A marker belongs to the first principal fragment, including when the
    // item is empty; it is never duplicated on the item's continuation pages.
    if !root.is_repeat && marker.first_page != Some(page.page_index) {
        return;
    }
    let x = page.content_box.x + root.x + marker.offset_x;
    let y = page.content_box.y + root.y
        - if root.is_repeat {
            0.0
        } else {
            page.content_origin_y
        };
    standalone_runs(
        &marker.shaped,
        (x, y),
        marker.color,
        &marker.shadows,
        NodeId::new(root.node as u64),
        RunSource::Generated(NodeId::new(owner as u64), GeneratedKind::Marker),
        out,
        None,
    );
}

/// Glyph runs of standalone text whose container's top-left is at `origin`,
/// in the way the built-in painter draws it.
///
/// With `starts`, also push the UTF-8 byte offset in the shaped text at
/// which each pushed run's [`PositionedGlyphRun::text`] begins.
#[allow(clippy::too_many_arguments)]
pub(crate) fn standalone_runs<'a>(
    text: &'a crate::StandaloneText,
    origin: (f32, f32),
    color: CssColor,
    shadows: &[TextShadow],
    line_root: NodeId,
    source: RunSource,
    out: &mut Vec<PositionedGlyphRun<'a>>,
    mut starts: Option<&mut Vec<usize>>,
) {
    let (x, y) = origin;
    for (index, line) in text.lines().iter().enumerate() {
        let converter = shodo::geometry::PhysicalConverter::new(
            line.writing_mode(),
            line.used_direction(),
            text.container(),
        );
        for fragment in line.fragments() {
            let shodo::Fragment::GlyphRun(run) = fragment else {
                continue;
            };
            let Some(font) = run.font_data() else {
                continue; // cov:ignore: plain marker glyph runs shaped with the document's font set always have a face
            };
            let font_index = font.index;
            let (bytes, blob) = font.data.into_raw_parts();
            let text_range = run.text_range();
            let shaped: Vec<_> = run.glyphs().collect();
            let clusters: Vec<_> = shaped.iter().map(|glyph| glyph.cluster).collect();
            let ranges = glyph_text_ranges(&clusters, text_range.start, text_range.end);
            let mut placed: Vec<_> = shaped
                .iter()
                .enumerate()
                .zip(ranges)
                .filter_map(|((i, glyph), range)| {
                    let (gx, gy) = run.glyph_origin(i)?;
                    let (gx, gy) =
                        converter.point(gx + text.hang_shift(index), gy + line.block_offset());
                    Some((x + gx, y + gy, glyph, range))
                })
                .collect();
            if run.bidi_level() % 2 == 1 {
                placed.reverse();
            }
            let Some(first) = placed.first() else {
                continue; // cov:ignore: enumerated marker glyphs always have origins; empty glyph runs are a defensive engine edge
            };
            let origin = (first.0, y + line.block_offset() + run.baseline());
            let mut pen = origin.0;
            let glyphs = placed
                .into_iter()
                .map(|(gx, gy, shaped, text_range)| {
                    let glyph = Glyph {
                        id: shaped.id,
                        advance: shaped.advance,
                        x_offset: gx - pen,
                        y_offset: gy - origin.1,
                        text_range,
                    };
                    pen += shaped.advance;
                    glyph
                })
                .collect();
            let metrics = run.metrics();
            if let Some(starts) = starts.as_deref_mut() {
                starts.push(text_range.start);
            }
            out.push(PositionedGlyphRun {
                line: TextLineId {
                    root: line_root,
                    index,
                },
                source,
                font: FontRef {
                    id: FontId {
                        blob,
                        index: font_index,
                    },
                    data: FontBlob(bytes),
                    index: font_index,
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
                text: line.text().get(text_range).unwrap_or_default(),
                glyphs,
                color,
                decorations: Vec::new(),
                shadows: shadows.to_vec(),
            });
        }
    }
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
    line_id: TextLineId,
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
        line: line_id,
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
        decorations: Vec::new(),
        shadows: text_shadows(positioned.style),
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
    let positioned = PositionedLines::new(document, cascade, root.node, root.fragmentainer)?;
    // A repeated paragraph sits at the same place on every page; any other
    // is placed in its page's slice of the flow.
    let x = page.content_box.x + root.x;
    let y = page.content_box.y + root.y - flow_range.map_or(0.0, |(start, _)| start);
    let decoration_context = crate::text_decoration::context_for_root(document, cascade, root.node);
    for (index, line) in positioned.all_lines().iter().enumerate() {
        let Some(offset) = positioned.line_offset(index) else {
            continue;
        };
        if let Some((start, end)) = flow_range {
            let top = root.y + offset.1 + line.block_offset();
            if !line_center_on_page(top, top + line.block_size(), start, end) {
                continue;
            }
        }
        let Some(line) = positioned.line_at(index) else {
            continue;
        };
        let line_origin = (x + line.offset.0, y + line.offset.1);
        let decorations = crate::text_decoration::positioned_line_decorations(
            document,
            cascade,
            root.node,
            &decoration_context,
            &line,
            line_origin,
        );
        for (run, lines) in line.runs.iter().zip(decorations) {
            if let Some(mut run) = glyph_run(
                document,
                &line,
                run,
                line_origin,
                TextLineId {
                    root: NodeId::new(root.node as u64),
                    index: line.index,
                },
            ) {
                if matches!(run.source, RunSource::Text(_)) {
                    run.decorations = lines;
                }
                out.push(run);
            } // cov:ignore: current positioned body runs have a source, style, font and nonempty glyphs; future element-owned runs may be omitted.
        }
    }
    Some(())
}

/// The node whose table-header repeat moves the runs of text root `node`.
fn text_source(document: &Document, node: usize) -> usize {
    generated_origin(node).map_or_else(|| document.ifc_source_owner(node), |(owner, _)| owner)
}

/// The lowest and highest block-axis line center of `root` in shared flow
/// coordinates, over the lines [`root_runs`] can draw. `None` when no line
/// has a finite center.
fn line_center_span(
    document: &Document,
    cascade: &CascadeResult,
    root: &ProjectedTextRoot,
) -> Option<(f32, f32)> {
    let positioned = PositionedLines::new(document, cascade, root.node, root.fragmentainer)?;
    let mut span: Option<(f32, f32)> = None;
    for (index, line) in positioned.all_lines().iter().enumerate() {
        let Some(offset) = positioned.line_offset(index) else {
            continue;
        };
        // The same arithmetic as `root_runs` and `line_center_on_page`.
        let top = root.y + offset.1 + line.block_offset();
        let center = (top + (top + line.block_size())) * 0.5;
        if !center.is_finite() {
            continue;
        }
        span = Some(span.map_or((center, center), |(low, high)| {
            (low.min(center), high.max(center))
        }));
    }
    span
}

/// For each page fragment, by position in `pages`, the indices into `roots`
/// of the text roots that can draw on it, in document order.
///
/// A root is left out only when [`Document::page_text_runs`] would draw
/// nothing of it on that page: a paragraph none of whose line centers falls
/// in the page's flow slice, or a marker on a page other than its first.
/// Repeated roots, and roots that a repeated table header may move, are kept
/// on every page and filtered per page as before.
pub(super) fn roots_by_page(
    document: &Document,
    cascade: &CascadeResult,
    roots: &[ProjectedTextRoot],
    markers: &BTreeMap<usize, MarkerText>,
    pages: &[PageFragment],
) -> Vec<Vec<usize>> {
    let mut by_page = vec![Vec::new(); pages.len()];
    // Flow slices in increasing order allow a binary search for the pages a
    // paragraph spans; otherwise every page is checked.
    let ordered = pages.iter().all(|page| page.flow_range.is_some())
        && pages.windows(2).all(|pair| {
            let (Some(first), Some(second)) = (pair[0].flow_range, pair[1].flow_range) else {
                return false; // cov:ignore: every slice was checked above
            };
            first.0 <= second.0 && first.1 <= second.1
        });
    for (index, root) in roots.iter().enumerate() {
        let moved_by_header = document
            .table_objects
            .headers
            .owner(text_source(document, root.node))
            .is_some();
        if root.is_repeat || moved_by_header {
            for page in &mut by_page {
                page.push(index);
            }
            continue;
        }
        if let Some((owner, PseudoElem::Marker)) = generated_origin(root.node) {
            let first_page = markers.get(&owner).and_then(|marker| marker.first_page);
            for (position, page) in pages.iter().enumerate() {
                if first_page == Some(page.page_index) {
                    by_page[position].push(index);
                }
            }
            continue;
        }
        let Some((low, high)) = line_center_span(document, cascade, root) else {
            continue;
        };
        // `line_center_on_page` keeps a center in `[start - 0.001, end - 0.001)`.
        let reaches = |range: (f32, f32)| high >= range.0 - 0.001 && low < range.1 - 0.001;
        let first = if ordered {
            pages.partition_point(|page| page.flow_range.is_some_and(|(_, end)| end - 0.001 <= low))
        } else {
            0
        };
        for (position, page) in pages.iter().enumerate().skip(first) {
            let Some(range) = page.flow_range else {
                continue;
            };
            if reaches(range) {
                by_page[position].push(index);
            } else if ordered && range.0 - 0.001 > high {
                break;
            }
        }
    }
    by_page
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
        let context = &projection.run_context;
        let pages = projection.pages.iter().enumerate();
        for (position, page) in pages.filter(|(_, page)| page.page_index == page_index) {
            let indices = projection.text_roots_by_page.get(position);
            for &index in indices.into_iter().flatten() {
                let mut root = projection.text_roots[index];
                let source = text_source(self, root.node);
                if let Some(shift) = self
                    .table_objects
                    .headers
                    .shift(source, page.content_origin_y)
                {
                    let Some(shift) = shift else {
                        continue;
                    };
                    root.y += shift - page.content_origin_y;
                    root.is_repeat = true;
                }
                if let Some((owner, PseudoElem::Marker)) = generated_origin(root.node) {
                    if omission(self, cascade, context, root.node).is_none() {
                        marker_runs(self, page, &root, owner, &mut runs);
                    }
                } else {
                    root_runs(self, cascade, page, context, &root, &mut runs);
                }
            }
        }
        runs
    }

    /// The paragraphs whose runs [`Document::page_text_runs`] leaves out, each
    /// with the reason.
    #[doc(hidden)]
    pub fn omitted_text_run_roots(&self, cascade: &CascadeResult) -> Vec<(NodeId, &'static str)> {
        let context = RunContext::new(self, &self.page_projection.text_roots);
        let mut seen = HashSet::new();
        let placed: HashSet<_> = self
            .page_projection
            .text_roots
            .iter()
            .map(|root| root.node)
            .collect();
        // A malformed nested fragment chain can make an entire paragraph
        // unreachable. Preserve its omission diagnostic without fabricating
        // a paint placement or publishing guessed glyph coordinates.
        let unrooted = self
            .layout_fragments()
            .iter()
            .filter(|fragment| {
                !placed.contains(&fragment.node_id)
                    && self
                        .get_node(fragment.node_id)
                        .is_some_and(|node| node.is_ifc_root())
            })
            .map(|fragment| {
                (
                    NodeId::new(self.ifc_source_owner(fragment.node_id) as u64),
                    TextRunOmission::UnrootedColumnFragments.describe(),
                )
            });
        self.page_projection
            .text_roots
            .iter()
            .filter_map(|root| {
                if let Some((owner, PseudoElem::Marker)) = generated_origin(root.node)
                    && !self.page_projection.markers.contains_key(&owner)
                {
                    return None;
                }
                let reason = omission(self, cascade, &context, root.node)?;
                Some((
                    NodeId::new(
                        generated_origin(root.node)
                            .map_or_else(|| self.ifc_source_owner(root.node), |(owner, _)| owner)
                            as u64,
                    ),
                    reason.describe(),
                ))
            })
            .chain(unrooted)
            .filter(|entry| seen.insert(*entry))
            .collect()
    }
}

#[cfg(test)]
mod tests;
