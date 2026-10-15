//! Positioned glyph runs of the paragraphs on a page, for a painter that
//! draws text itself.

use super::records::{PageFragment, ProjectedTextRoot};
use crate::Document;
use crate::generated_content::generated_origin;
use crate::layout::{PositionedLine, PositionedLines, PositionedRun, line_center_on_page};
use raikiri_style::property::{
    ColumnCountValue, CssColor, Direction, PositionValue, TextShadowColor,
};
use raikiri_style::resolve::ComputedLengthPercentageOrAuto;
use raikiri_style::{CascadeResult, ComputedValues, PseudoElem};
use raikiri_traits::{NodeId, NodeKind, PaintTransform};
use shodo::geometry::WritingMode;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::ops::Range;
use std::sync::Arc;

#[derive(Clone)]
pub(super) struct MarkerText {
    shaped: Arc<crate::StandaloneText>,
    offset_x: f32,
    /// Moves the marker's line down to the half-leading of the item's
    /// line height, so its baseline meets the item's first line.
    offset_y: f32,
    color: CssColor,
    shadows: Vec<TextShadow>,
    first_page: Option<u32>,
    /// The flow origin of [`Self::first_page`].
    first_page_origin: Option<f32>,
    /// The middle of the item's first line from its border edge, when the
    /// marker is aligned with one.
    line_center: Option<f32>,
}

impl MarkerText {
    /// Number of stable line identities in the prepared standalone marker.
    pub(super) fn line_count(&self) -> usize {
        self.shaped.lines().len()
    }

    /// Whether the page whose flow starts at `content_origin_y` is the
    /// item's first page, the one page that draws the marker. Unknown when
    /// the item has no fragment on any page.
    pub(super) fn is_on_page(&self, content_origin_y: f32) -> bool {
        self.first_page_origin
            .is_none_or(|origin| (origin - content_origin_y).abs() < 0.001)
    }

    /// The top of the marker's line for an item whose border edge is at
    /// `top` on a page whose content ends at `page_bottom`.
    ///
    /// The marker is drawn only on its item's first page. When the line it
    /// aligns with starts on a later page, the marker stays at the bottom of
    /// this one rather than past its edge.
    pub(super) fn line_top(&self, top: f32, page_bottom: f32) -> f32 {
        let height = self
            .shaped
            .lines()
            .first()
            .map_or(0.0, |line| line.block_size());
        let aligned = top + self.offset_y;
        // The aligned line's page is decided by its middle, as it is for the
        // line itself; without one, by the marker's top.
        let next_page = match self.line_center {
            Some(center) => top + center >= page_bottom - 0.001,
            None => aligned >= page_bottom,
        };
        if next_page {
            (page_bottom - height).max(top)
        } else {
            aligned
        }
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
    let snapshots = document
        .shared_counter_snapshots(cascade)
        .map_err(
            |error| raikiri_traits::LayoutError::CounterSnapshotLimitExceeded {
                limit: error.limit,
                actual: error.actual,
            },
        )?;
    let mut first_pages = BTreeMap::new();
    for page in pages {
        for item in &page.items {
            if item.fragment_index == 0 {
                first_pages
                    .entry(item.node_id.0 as usize)
                    .or_insert((page.page_index, page.content_origin_y));
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
            // CSS Lists 3 §3.1: an outside marker sits on the baseline of
            // its item's first line. When the item lays out its own first
            // line, the two baselines are matched directly, which covers a
            // marker or a first line with other font metrics.
            let marker_baseline = shaped.lines().first().map(|line| {
                line.block_offset() + line.baseline(shodo::geometry::BaselineKind::Alphabetic)
            });
            let item_line = first_line(document, cascade, owner);
            let offset_y = match (item_line.map(|line| line.baseline), marker_baseline) {
                (Some(item), Some(marker)) if (item - marker).is_finite() => item - marker,
                // Otherwise the marker is shaped with `line-height: normal`.
                // An item with another line height centres its first line's
                // content area in that height (CSS Inline 3 §5.1
                // half-leading), so the marker's line moves by half the
                // difference.
                _ => match style.line_height {
                    raikiri_style::ComputedLineHeight::Normal => 0.0,
                    _ => shaped.lines().first().map_or(0.0, |line| {
                        (crate::layout::multicol::line_height_px(style) - line.block_size()) / 2.0
                    }),
                },
            };
            markers.insert(
                owner,
                MarkerText {
                    shaped: Arc::new(shaped),
                    offset_x,
                    offset_y,
                    color: style.color,
                    shadows: text_shadows(style),
                    first_page: first_pages.get(&owner).map(|&(index, _)| index),
                    first_page_origin: first_pages.get(&owner).map(|&(_, origin)| origin),
                    line_center: item_line.map(|line| line.center),
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
///
/// The built-in painter moves the text with the boxes around it, by
/// relative offsets that layout leaves to paint and by `transform`. Inside
/// columns, positions include those moves, unlike [`crate::Fragment::rect`],
/// and a paragraph there under a transform that does more than translate
/// has no runs. Elsewhere, positions are where layout put the text, like
/// fragment rectangles, and [`Self::transform`] maps them to the page.
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
    /// UTF-8 offset of [`Self::text`] in the processed text of its line.
    /// Within one line, runs and the images of generated content (see
    /// [`crate::Document::page_generated_image_text_position`]) follow the logical (reading)
    /// order of these offsets, whatever their visual order.
    pub text_offset: usize,
    /// The glyphs, left to right.
    pub glyphs: Vec<Glyph>,
    /// Maps the run's positions, decorations and shadows to the page, where
    /// the built-in painter draws them. `None` when they are already on the
    /// page. In paint order, this is the product of the enclosing
    /// [`crate::PaintEvent::PushTransform`] events.
    pub transform: Option<PaintTransform>,
    /// Text color (used value).
    pub color: CssColor,
    /// Used decoration segments, including lines propagated from ancestors.
    /// Generated text and ellipses have no decoration segments in this version.
    pub decorations: Vec<crate::DecorationLine>,
    /// Used `text-shadow` list of the run's text (CSS Text Decoration 3 §4),
    /// in declaration order: the first shadow is painted on top, and every
    /// shadow is painted below the run's glyphs and decorations. A shadow
    /// covers the glyphs and [`Self::decorations`] alike, moved by its
    /// offset and filled with its color. Empty for `none`.
    pub shadows: Vec<TextShadow>,
    /// The `::first-letter` box whose letters the run draws, by its
    /// generated node id, and the node those letters come from; `None` for
    /// other text.
    pub(crate) letter_box: Option<(usize, usize)>,
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

/// The pseudo-element generated text or a generated box comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum GeneratedKind {
    /// `::before`.
    Before,
    /// `::after`.
    After,
    /// `::marker`.
    Marker,
    /// `::first-letter`. Only boxes carry it: the letter's text keeps the
    /// source of the text it comes from.
    FirstLetter,
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
    Transform,
    SingularTransform,
    FragmentedTransform,
    ShiftedClip,
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
            Self::Transform => {
                "it or an ancestor inside columns has a transform that does more than translate"
            }
            Self::SingularTransform => {
                "it or an ancestor has a transform that cannot be inverted, so nothing of it is painted"
            }
            Self::FragmentedTransform => {
                "a box split into column fragments translates it by a percentage"
            }
            Self::ShiftedClip => {
                "inside columns, a relative offset or translation moves an overflow or column \
                 clip around it"
            }
            Self::FixedPlacement => {
                "it is inside a fixed box not placed by `top` and `left` lengths, \
                 or inside a box that is transformed, filtered or moved"
            }
        }
    }
}

/// Paragraphs whose original line identities do not uniquely select a column placement.
#[derive(Debug, Default, Clone)]
pub(super) struct RunContext {
    overlapping: HashSet<usize>,
    /// Elements laid out as column fragments, which the painter places from
    /// each fragment.
    pub(super) fragmented: HashSet<usize>,
    /// Elements with a column fragment the painter clips where it draws
    /// that fragment.
    pub(super) fragment_clipped: HashSet<usize>,
    /// The body, where the painter starts.
    pub(super) body: Option<usize>,
    /// Whether the page areas differ between pages. Layout places fixed
    /// boxes against the first page area, while the painter places them
    /// from their `right` and `bottom` insets against each page's.
    page_areas_vary: bool,
}

impl RunContext {
    pub(super) fn new(
        document: &Document,
        roots: &[ProjectedTextRoot],
        pages: &[PageFragment],
    ) -> Self {
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
        let body = crate::paint_rules::find_paint_root(document);
        let fragments = document.layout_fragments();
        let fragmented = fragments.iter().map(|fragment| fragment.node_id).collect();
        let fragment_clipped = fragments
            .iter()
            .filter(|fragment| fragment.fragmentainer_clip.is_some())
            .map(|fragment| fragment.node_id)
            .collect();
        Self {
            overlapping,
            fragmented,
            fragment_clipped,
            body,
            page_areas_vary: page_areas_vary(document, pages),
        }
    }
}

/// The block offset that layout applies to a box in normal flow for its
/// `position: relative` or `position: sticky` insets, which layout treats
/// alike, resolved against the containing block as layout resolves them.
fn layout_relative_offset_y(
    document: &Document,
    cascade: &CascadeResult,
    cv: &ComputedValues,
    id: usize,
) -> f32 {
    if !matches!(cv.position, PositionValue::Relative | PositionValue::Sticky) {
        return 0.0;
    }
    // Layout takes `top` over `-bottom` in every writing mode.
    crate::layout::used_relative_insets(
        document,
        cascade,
        cv,
        id,
        crate::layout::Precedence::LeftTop,
    )
    .1
    .unwrap_or(0.0)
}

/// How the built-in painter moves one element and its subtree from their
/// layout location.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct BoxShift {
    /// The relative offset that layout leaves to paint, plus the
    /// translation of a `transform` that only translates.
    pub(super) offset: (f32, f32),
    /// The element's `transform` does more than translate.
    pub(super) general: bool,
}

/// How far the built-in painter moves element `id` and its subtree from
/// their layout location: by a relative offset that layout leaves to paint,
/// and by its `transform`.
pub(super) fn box_paint_shift(
    document: &Document,
    cascade: &CascadeResult,
    body: Option<usize>,
    id: usize,
    cv: &ComputedValues,
    fragmented: bool,
) -> Result<BoxShift, TextRunOmission> {
    let Some(node) = document.get_node(id) else {
        return Ok(BoxShift {
            offset: (0.0, 0.0),
            general: false,
        }); // cov:ignore: ancestors of a laid-out paragraph are arena nodes
    };
    // Layout already applies the relative offset of boxes in normal flow,
    // flex and grid, and of boxes on lines. The body, floats and table parts
    // and column fragments are placed without it, and the painter adds it, as
    // it adds the block offset that manual column placement leaves out.
    let painted_inset =
        crate::layout::placed_without_relative_offset(document, cascade, body, id, cv)
            || (fragmented && !cv.display.is_inline_level());
    let (mut dx, mut dy) = if painted_inset {
        crate::layout::used_relative_offset(document, cascade, cv, id)
    } else if node.needs_relative_block_paint_offset() {
        (
            0.0,
            crate::layout::used_relative_offset(document, cascade, cv, id).1,
        )
    } else {
        (0.0, 0.0)
    };
    if !crate::layout::is_transformable(document, id, cv) || cv.transform.is_empty() {
        return Ok(BoxShift {
            offset: (dx, dy),
            general: false,
        });
    }
    if !super::paint_transform::translation_only(cv) {
        // The painter turns the box about its origin; see
        // `paint_transform::element_transform`.
        return Ok(BoxShift {
            offset: (dx, dy),
            general: true,
        });
    }
    // Percentages resolve against the border box (CSS Transforms 1 §6).
    let size = node.unrounded_layout.size;
    // A fragment's percentages resolve against that fragment, which the
    // runs of a paragraph do not know.
    if fragmented
        && cv
            .transform
            .iter()
            .any(|function| function.translates_by_percent())
    {
        return Err(TextRunOmission::FragmentedTransform);
    }
    for function in cv.transform.iter() {
        let (x, y) = function
            .translation(f64::from(size.width), f64::from(size.height))
            .unwrap_or_default();
        dx += x as f32;
        dy += y as f32;
    }
    Ok(BoxShift {
        offset: (dx, dy),
        general: false,
    })
}

/// How many columns a multicol container lays its content out in; 1 for
/// any other box.
fn used_columns(document: &Document, id: usize) -> usize {
    let Some(node) = document.get_node(id) else {
        return 1; // cov:ignore: ancestors of a laid-out paragraph are arena nodes
    };
    let Some(style) = node.multicol else {
        return 1;
    };
    let content_width = node.unrounded_layout.content_box_width();
    crate::fragment::FragmentationContext::resolve(content_width, None, style)
        .map_or(1, |context| context.column_count)
}

/// Whether the page area of some page differs from the one layout placed
/// fixed boxes against.
fn page_areas_vary(document: &Document, pages: &[PageFragment]) -> bool {
    let (laid_out_width, laid_out_height) = document.fixed_page_area;
    pages.iter().any(|page| {
        let (margins, insets) = (page.margins, page.content_insets);
        let width = page.page_box.width - margins.left - margins.right - insets.left - insets.right;
        let height =
            page.page_box.height - margins.top - margins.bottom - insets.top - insets.bottom;
        (width - laid_out_width).abs() > 1e-3 || (height - laid_out_height).abs() > 1e-3
    })
}

/// Whether the painter places `cv`'s box on each page differently than
/// layout does.
fn fixed_omission(
    cv: &ComputedValues,
    page_areas_vary: bool,
    rtl: bool,
) -> Option<TextRunOmission> {
    // The painter places a fixed box on each page from its `left` length, or
    // its `right` length when `left` is auto, and likewise from `top` or
    // `bottom`, as layout does against the page area. It places a box from
    // other insets differently than layout, and one from `right` or
    // `bottom` against each page's area, which layout cannot follow when
    // the page areas differ. A right-to-left page area places a box with
    // both horizontal insets from `right`.
    let length = |inset| matches!(inset, ComputedLengthPercentageOrAuto::Px(_));
    let placed = |start, end, from_end: bool| {
        (length(start) && !from_end)
            || (!page_areas_vary
                && (from_end || matches!(start, ComputedLengthPercentageOrAuto::Auto))
                && length(end))
    };
    let horizontal_from_end = rtl && length(cv.left) && length(cv.right);
    (cv.position == PositionValue::Fixed
        && !(placed(cv.left, cv.right, horizontal_from_end) && placed(cv.top, cv.bottom, false)))
    .then_some(TextRunOmission::FixedPlacement)
}

/// Whether a column of element `id` clips its content: the element is a
/// multi-column container with more than one column, or a column fragment
/// the painter clips where it draws it.
pub(super) fn column_clip_owner(
    document: &Document,
    cascade: &CascadeResult,
    context: &RunContext,
    id: usize,
) -> bool {
    context.fragment_clipped.contains(&id)
        || cascade.computed.get(id).is_some_and(
            |cv| matches!(cv.column_count, ColumnCountValue::Count(count) if count > 1),
        )
        || used_columns(document, id) > 1
}

/// The boxes from the source of text root `root` up to the body, innermost
/// first, each with whether a box above it clips it to a column.
fn paint_chain(
    document: &Document,
    cascade: &CascadeResult,
    context: &RunContext,
    root: usize,
) -> Vec<(usize, bool)> {
    let root = generated_origin(root).map_or(root, |(owner, _)| owner);
    let mut chain = Vec::new();
    let mut current = Some(document.ifc_source_owner(root));
    while let Some(id) = current {
        chain.push((id, false));
        // The painter starts at the body; the boxes above it do not move
        // its content.
        if Some(id) == context.body {
            break;
        }
        current = document.parent_of(id);
    }
    let mut in_columns = false;
    for (id, below) in chain.iter_mut().rev() {
        *below = in_columns;
        in_columns |= column_clip_owner(document, cascade, context, *id);
    }
    chain
}

/// How far the built-in painter moves the lines of text root `root` from
/// their layout location inside columns, or what moves them that the runs
/// do not model.
///
/// Outside columns, the paint order lists the moves of the boxes around the
/// lines as transforms, and the runs carry them in
/// [`PositionedGlyphRun::transform`] (see [`run_transform`]). Inside
/// columns, the clips of the columns stay at their layout location, so the
/// offsets of the boxes there move the lines themselves.
pub(super) fn paint_shift(
    document: &Document,
    cascade: &CascadeResult,
    context: &RunContext,
    root: usize,
) -> Result<(f32, f32), TextRunOmission> {
    // The page area takes the direction of the root element.
    let rtl = context
        .body
        .and_then(|body| document.parent_of(body))
        .and_then(|html| cascade.computed.get(html))
        .is_some_and(|cv| cv.direction == Direction::Rtl);
    let (mut dx, mut dy) = (0.0, 0.0);
    // Whether a clip was among the boxes passed so far: the painter moves
    // that clip with the boxes above it, and the clips of the paint order
    // inside columns stay at their layout location.
    let mut clipped = false;
    let mut fixed = false;
    for (id, in_columns) in paint_chain(document, cascade, context, root) {
        let Some(cv) = cascade.computed.get(id) else {
            continue; // cov:ignore: the cascade has a value for every arena node
        };
        if fixed {
            // spec: https://www.w3.org/TR/css-transforms-1/#containing-block-for-all-descendants
            // An ancestor that is the containing block of fixed boxes makes
            // the fixed box local to it, while the projection repeats it on
            // every page. Any other ancestor neither moves the fixed box in
            // layout, which places it against the page area, nor in the
            // painter, which places it from its insets.
            if crate::layout::establishes_fixed_containing_block(document, id, cv) {
                return Err(TextRunOmission::FixedPlacement);
            }
            continue;
        }
        if let Some(reason) = fixed_omission(cv, context.page_areas_vary, rtl) {
            return Err(reason);
        }
        let fragmented = context.fragmented.contains(&id);
        let shift = box_paint_shift(document, cascade, context.body, id, cv, fragmented)?;
        if shift.general && super::paint_transform::is_singular(cv) {
            return Err(TextRunOmission::SingularTransform);
        }
        if in_columns {
            if shift.general {
                return Err(TextRunOmission::Transform);
            }
            clipped |= column_clip_owner(document, cascade, context, id)
                || crate::paint_rules::clips_element_overflow(document, cascade, id);
            if clipped && shift.offset != (0.0, 0.0) {
                return Err(TextRunOmission::ShiftedClip);
            }
            dx += shift.offset.0;
            dy += shift.offset.1;
        }
        fixed = cv.position == PositionValue::Fixed;
    }
    Ok((dx, dy))
}

/// How far the painter moves the boxes around text root `root` down the
/// page outside columns by offsets that only translate: the part of
/// [`run_transform`] that the built-in painter adds to a box's position
/// rather than to its transform.
pub(super) fn run_translation_y(
    document: &Document,
    cascade: &CascadeResult,
    context: &RunContext,
    root: usize,
) -> f32 {
    let mut dy = 0.0;
    for (id, in_columns) in paint_chain(document, cascade, context, root)
        .into_iter()
        .rev()
    {
        if in_columns {
            break;
        }
        let Some(cv) = cascade.computed.get(id) else {
            continue; // cov:ignore: the cascade has a value for every arena node
        };
        if cv.position == PositionValue::Fixed {
            dy = 0.0;
        }
        let fragmented = context.fragmented.contains(&id);
        if let Ok(shift) = box_paint_shift(document, cascade, context.body, id, cv, fragmented)
            && !shift.general
        {
            dy += shift.offset.1;
        }
    }
    dy
}

/// The transform from the coordinates the runs of text root `root` are
/// placed in on `page` to the page: the moves of the boxes around them
/// outside columns, outermost first, as the paint order lists them.
pub(super) fn run_transform(
    document: &Document,
    cascade: &CascadeResult,
    context: &RunContext,
    page: &PageFragment,
    root: usize,
) -> Option<PaintTransform> {
    let mut transform = PaintTransform::IDENTITY;
    let mut ancestors = (0.0, 0.0);
    for (id, in_columns) in paint_chain(document, cascade, context, root)
        .into_iter()
        .rev()
    {
        if in_columns {
            break;
        }
        let Some(cv) = cascade.computed.get(id) else {
            continue; // cov:ignore: the cascade has a value for every arena node
        };
        if cv.position == PositionValue::Fixed {
            // The painter places a fixed box from its insets, whatever moves
            // its ancestors; one that an ancestor contains is omitted
            // first.
            transform = PaintTransform::IDENTITY;
            ancestors = (0.0, 0.0);
        }
        let fragmented = context.fragmented.contains(&id);
        let Ok(shift) = box_paint_shift(document, cascade, context.body, id, cv, fragmented) else {
            continue; // cov:ignore: the runs of a box the painter cannot place are omitted first
        };
        if let Some(own) = super::paint_transform::element_transform(
            document,
            cascade,
            page,
            id,
            shift.offset,
            shift.general,
            ancestors,
        ) {
            transform = transform * own;
        }
        ancestors.0 += shift.offset.0;
        ancestors.1 += shift.offset.1;
    }
    (transform != PaintTransform::IDENTITY).then_some(transform)
}

/// How far the painter moves the runs of `root` from their layout
/// location, or why the runs cannot be positioned.
pub(super) fn placement(
    document: &Document,
    cascade: &CascadeResult,
    context: &RunContext,
    root: usize,
) -> Result<(f32, f32), TextRunOmission> {
    let root = generated_origin(root).map_or(root, |(owner, _)| owner);
    if let Some(node) = document.ifc_layout_node(root) {
        if context.overlapping.contains(&root) {
            return Err(TextRunOmission::OverlappingColumnLines);
        }
        if node
            .ifc_writing_mode()
            .is_some_and(|mode| mode != WritingMode::HorizontalTb)
        {
            return Err(TextRunOmission::VerticalWritingMode);
        }
    }
    paint_shift(document, cascade, context, root)
}

/// Why the runs of `root` cannot be positioned, if they cannot.
pub(super) fn omission(
    document: &Document,
    cascade: &CascadeResult,
    context: &RunContext,
    root: usize,
) -> Option<TextRunOmission> {
    placement(document, cascade, context, root).err()
}

/// `owner`'s first formatted line, from its border edge, where pagination
/// draws it.
#[derive(Clone, Copy, Debug)]
struct FirstLine {
    baseline: f32,
    /// The middle of the line box, which decides its page. A table row has
    /// no line box of its own, so its baseline stands in.
    center: f32,
}

/// Finds `owner`'s first formatted line.
///
/// The first line may belong to the first in-flow block child, and so on
/// down (CSS Pseudo 4 §2.1.1); if that child has no line, neither has the
/// item. Floats, absolutely positioned boxes and the marker itself are
/// passed over, and a relative offset moves only the box it applies to. A
/// flex container orders its items itself, so its first line is the first
/// baseline its layout recorded; a grid container takes the line of an item
/// in its first row.
fn first_line(document: &Document, cascade: &CascadeResult, owner: usize) -> Option<FirstLine> {
    // `float` does not apply to flex items (CSS Flexbox 1 §3), so a flex
    // container's items are checked without it.
    let laid_out = |id: usize| {
        document.nodes.get(id).is_some_and(|node| {
            node.is_in_document()
                && node.kind() != NodeKind::Text
                && node.style.display != taffy::Display::None
                && node.style.position != taffy::Position::Absolute
                && !generated_origin(id).is_some_and(|(_, pseudo)| pseudo == PseudoElem::Marker)
        })
    };
    let in_flow = |id: usize| {
        laid_out(id)
            && document
                .nodes
                .get(id)
                .is_some_and(|node| node.style.float == taffy::Float::None)
    };
    let (mut id, mut y) = (owner, 0.0);
    loop {
        let node = document.nodes.get(id)?;
        if matches!(
            node.display,
            raikiri_style::property::DisplayValue::Table
                | raikiri_style::property::DisplayValue::InlineTable
        ) && !node.is_ifc_root()
        {
            return node.table_first_baseline.map(|baseline| FirstLine {
                baseline: y + baseline,
                center: y + baseline,
            });
        }
        // A text node keeps Taffy's default display, which is not its own;
        // its lines are found below.
        let is_text = node.kind() == NodeKind::Text;
        if !is_text && node.style.display == taffy::Display::Grid {
            let line = grid_first_line(document, cascade, id)?;
            return Some(FirstLine {
                baseline: y + line.baseline,
                center: y + line.center,
            });
        }
        if !is_text && node.style.display == taffy::Display::Flex {
            // A flex container orders its items itself, and its layout
            // records the first baseline they give it. The middle of that
            // line comes from the first item, in layout order, whose first
            // line has the baseline; when none matches, the baseline stands
            // in.
            let baseline = node.flex_first_baseline?;
            let center = node
                .layout_children()
                .iter()
                .filter(|&&child| laid_out(child))
                .find_map(|&child| {
                    let line = first_line(document, cascade, child)?;
                    let top = document.nodes[child].unrounded_layout.location.y
                        - layout_applied_relative_offset_y(document, cascade, child);
                    ((top + line.baseline - baseline).abs() < 0.01).then_some(top + line.center)
                })
                .unwrap_or(baseline);
            return Some(FirstLine {
                baseline: y + baseline,
                center: y + center,
            });
        }
        let child = match node.ifc.as_ref().and_then(|root| root.lines.as_ref()) {
            Some(lines) if !lines.lines.is_empty() => {
                // A block placed before line zero holds the first line.
                let before = lines
                    .block_line_starts
                    .iter()
                    .find(|&&(child, line)| line == 0 && in_flow(child));
                match before {
                    Some(&(child, _)) => child,
                    None => {
                        // The lines start inside the border and padding,
                        // moved down by any pagination shift.
                        let layout = node.unrounded_layout;
                        let top = y + layout.border.top + layout.padding.top + lines.shift_of(0);
                        return lines.lines.first().map(|line| FirstLine {
                            baseline: top
                                + line.block_offset()
                                + line.baseline(shodo::geometry::BaselineKind::Alphabetic),
                            center: top + line.block_offset() + line.block_size() / 2.0,
                        });
                    }
                }
            }
            _ => first_block_child(document, id, &in_flow)?,
        };
        // A relative offset moves the child, not the item's marker, so the
        // offset that layout already applied is taken back out.
        y += document.nodes[child].unrounded_layout.location.y
            - layout_applied_relative_offset_y(document, cascade, child);
        id = child;
    }
}

/// A grid container's first line, from its border edge (CSS Grid 1 §11.5).
///
/// Among the items in the first row that holds any, in grid order (by
/// column, then in order-modified document order), the first one that takes
/// part in baseline alignment gives the line, or else the first one. An item
/// whose percentage height depends on the rows it spans takes no part (CSS
/// Grid 1 §6.4), as in layout. An
/// item with no line of its own gives the bottom of its border box (CSS Box
/// Alignment 3 §9.1). Text directly in the grid is an anonymous item of its
/// own.
fn grid_first_line(document: &Document, cascade: &CascadeResult, grid: usize) -> Option<FirstLine> {
    let node = document.nodes.get(grid)?;
    let is_item = |id: usize| {
        document.nodes.get(id).is_some_and(|item| {
            item.is_in_document()
                && item.style.display != taffy::Display::None
                && item.style.position != taffy::Position::Absolute
                && !generated_origin(id).is_some_and(|(_, pseudo)| pseudo == PseudoElem::Marker)
        })
    };
    let first_row = node
        .grid_item_row_starts
        .iter()
        .filter(|&&(item, _)| is_item(item))
        .map(|&(_, row)| row)
        .min()?;
    let column_of = |child: usize| {
        node.grid_item_column_starts
            .iter()
            .find(|&&(item, _)| item == child)
            .map_or(0, |&(_, column)| column)
    };
    // The recorded items follow the children Taffy laid out: in
    // order-modified document order, with `display: contents` flattened.
    let mut first_row_items = node
        .grid_item_row_starts
        .iter()
        .filter(|&&(item, row)| row == first_row && is_item(item))
        .map(|&(item, _)| item)
        .collect::<Vec<_>>();
    // A stable sort keeps the order-modified document order within a column.
    first_row_items.sort_by_key(|&child| column_of(child));
    let item = first_row_items
        .iter()
        .copied()
        .find(|&child| {
            aligns_by_baseline(&node.style, &document.nodes[child].style)
                && !node.grid_items_without_baseline.contains(&child)
        })
        .or_else(|| first_row_items.first().copied())?;
    // A relative offset moves the item's box, not the line the grid
    // takes its baseline from.
    let item_node = &document.nodes[item];
    let item_layout = item_node.unrounded_layout;
    let top = item_layout.location.y - layout_applied_relative_offset_y(document, cascade, item);
    let bottom = item_layout.size.height;
    // Layout containment suppresses the item's own baseline, so one is
    // synthesized (CSS Contain 2 §3.2).
    let layout_contained = cascade.computed.get(item).is_some_and(|computed| {
        computed
            .contain
            .contains(raikiri_style::property::Contain::LAYOUT)
    });
    let line = if layout_contained {
        None
    } else {
        first_line(document, cascade, item)
    };
    Some(match line {
        Some(mut line) => {
            // A scroll container's baseline, and the line that decides its
            // page, stay within its border box (CSS Box Alignment 3 §9.1).
            if item_node.style.overflow.y.is_scroll_container() {
                line.baseline = line.baseline.clamp(0.0, bottom);
                line.center = line.center.clamp(0.0, bottom);
            }
            FirstLine {
                baseline: top + line.baseline,
                center: top + line.center,
            }
        }
        None => FirstLine {
            baseline: top + bottom,
            center: top + bottom,
        },
    })
}

/// Whether a grid item takes part in baseline alignment in the block axis:
/// its `align-self` resolves to `baseline` and neither block-axis margin is
/// `auto` (CSS Box Alignment 3 §9.3).
fn aligns_by_baseline(container: &taffy::Style, item: &taffy::Style) -> bool {
    let align = item.align_self.or(container.align_items);
    align.is_some_and(|align| align.keyword() == taffy::AlignItemsKeyword::Baseline)
        && !item.margin.top.is_auto()
        && !item.margin.bottom.is_auto()
}

/// The vertical relative offset that layout already applied to `id`'s
/// location; the painter applies the others.
fn layout_applied_relative_offset_y(
    document: &Document,
    cascade: &CascadeResult,
    id: usize,
) -> f32 {
    if document.nodes[id].needs_relative_block_paint_offset() {
        0.0
    } else {
        layout_relative_offset_y(document, cascade, &cascade.computed[id], id)
    }
}

/// The first in-flow block-level child of `id`, which has no line of its
/// own. Inline-level children generated no line there, so they are passed
/// over, and the children of a `display: contents` element count as its
/// parent's.
fn first_block_child(
    document: &Document,
    id: usize,
    in_flow: &impl Fn(usize) -> bool,
) -> Option<usize> {
    document.nodes.get(id)?.children.iter().find_map(|&child| {
        if !in_flow(child) {
            return None;
        }
        match document.nodes[child].display {
            raikiri_style::property::DisplayValue::Contents => {
                first_block_child(document, child, in_flow)
            }
            display if display.is_inline_level() => None,
            _ => Some(child),
        }
    })
}

fn marker_runs<'a>(
    document: &'a Document,
    page: &PageFragment,
    root: &ProjectedTextRoot,
    owner: usize,
    shift: (f32, f32),
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
    let x = page.content_box.x + root.x + shift.0 + marker.offset_x;
    let top = page.content_box.y + root.y + shift.1
        - if root.is_repeat {
            0.0
        } else {
            page.content_origin_y
        };
    let y = marker.line_top(top, page.content_box.y + page.content_box.height);
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
                text_offset: text_range.start,
                text: line.text().get(text_range).unwrap_or_default(),
                glyphs,
                color,
                decorations: Vec::new(),
                shadows: shadows.to_vec(),
                transform: None,
                letter_box: None,
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
        // An annotation's text is in its own line.
        let text_line = positioned
            .annotation
            .map_or(line.line, |annotation| annotation.line);
        (
            text_line.text().get(text_range.clone()).unwrap_or_default(),
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
    let baseline = positioned.annotation.map_or_else(
        || line.line.block_offset() + run.baseline(),
        |annotation| annotation.baseline,
    );
    let origin = (placed.first()?.0, dy + baseline);
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
        text_offset: positioned
            .annotation
            .map_or(text_range.start, |annotation| annotation.text_offset),
        text,
        glyphs,
        color,
        decorations: Vec::new(),
        shadows: text_shadows(positioned.style),
        transform: None,
        letter_box: (positioned.style_owner != positioned.owner)
            .then_some((positioned.style_owner, positioned.owner)),
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
    let shift = placement(document, cascade, context, root.node).ok()?;
    let transform = run_transform(document, cascade, context, page, root.node);
    let positioned = PositionedLines::new(document, cascade, root.node, root.fragmentainer)?;
    // A repeated paragraph sits at the same place on every page; any other
    // is placed in its page's slice of the flow. The paint shift moves the
    // lines, not the page they belong to.
    let x = page.content_box.x + root.x + shift.0;
    let y = page.content_box.y + root.y + shift.1 - flow_range.map_or(0.0, |(start, _)| start);
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
                run.transform = transform;
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
                    if let Ok(shift) = placement(self, cascade, context, root.node) {
                        let start = runs.len();
                        marker_runs(self, page, &root, owner, shift, &mut runs);
                        let transform = run_transform(self, cascade, context, page, root.node);
                        for run in &mut runs[start..] {
                            run.transform = transform;
                        }
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
        let context = self.page_projection.run_context.clone();
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
