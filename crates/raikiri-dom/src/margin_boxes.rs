//! Page-margin boxes (CSS Paged Media 3 §4.2): the generated content,
//! box decorations and used geometry of the sixteen boxes around the page
//! area of one page.
//!
//! The geometry is a compact grid model of the sixteen slots: edge boxes
//! divide the corresponding margin strip, while corner boxes live in the
//! strip intersections. Generated content is resolved to one string per box
//! (counters, `string()`, `element()` and quotes) and shaped with the
//! document's fonts.

use crate::generated_content::format_counter;
use crate::layout::{PageMargins, page_margins_for_page};
use crate::page_projection::text_runs::standalone_runs;
use crate::paint_rules::OverflowClipGeometry;
use crate::running::element_string_value;
use crate::{
    Document, PositionedGlyphRun, RunSource, StandaloneAlign, StandaloneStyle, StandaloneText,
    TextShadow,
};
use raikiri_style::property::{
    BackgroundImage, BackgroundRepeat, BorderColor, BorderStyle, ContentComponent, CounterStyle,
    CssColor, FontFamilyName, OverflowValue, OverflowXY, PropertyKey, PropertyValue, QuoteKeyword,
    StringFetchMode, TextAlign, TextShadowColor, VerticalAlign, VisualBox, WritingMode,
    ZIndexValue,
};
use raikiri_style::resolve::ComputedOverflowClipMargin;
use raikiri_style::{
    CascadeResult, ComputedBackgroundSize, ComputedBorder, ComputedCssPosition,
    ComputedLengthPercentageOrAuto, ComputedValues, CounterStyleRegistry,
    MAX_PAINTED_BACKGROUND_LAYERS, PageCascadeResult, PageMarginBoxCascadeResult,
    PageMarginBoxSlot,
};
use raikiri_traits::{NodeId, PageBox, PaintClip, PaintInsets, PaintRect};
use smol_str::SmolStr;
use std::ops::Range;
use std::sync::Arc;

#[cfg(test)]
mod tests;

/// The page a set of margin boxes belongs to, for page-based counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct MarginBoxPageContext {
    /// Zero-based page index.
    pub page_index: u32,
    /// Number of pages in the document, the value of `counter(pages)`.
    pub page_count: u32,
    /// Whether the page is a left page.
    pub page_is_left: bool,
    /// `counter-increment` of the `page` counter on the paired left page,
    /// for a right page whose own page rule sets an increment of zero.
    pub paired_page_increment: Option<i32>,
    /// Whether `page_count` is only a placeholder because the number of
    /// pages is not known yet. `counter(pages)` then formats the placeholder
    /// and is reported in [`MarginBox::deferred`].
    pub page_count_deferred: bool,
}

impl MarginBoxPageContext {
    /// Context of page `page_index` out of `page_count` pages.
    pub fn new(page_index: u32, page_count: u32, page_is_left: bool) -> Self {
        Self {
            page_index,
            page_count,
            page_is_left,
            paired_page_increment: None,
            page_count_deferred: false,
        }
    }

    /// Treat `page_count` as a placeholder for a number of pages that is
    /// not known yet. Choose a placeholder as wide as the expected value.
    pub fn with_deferred_page_count(mut self) -> Self {
        self.page_count_deferred = true;
        self
    }

    /// Supply the increment of the paired left page.
    pub fn with_paired_page_increment(mut self, increment: Option<i32>) -> Self {
        self.paired_page_increment = increment;
        self
    }
}

/// One side of a margin box border: only solid borders are reported.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct MarginBoxBorder {
    /// Used width in CSS px, always positive.
    pub width: f32,
    /// Used color.
    pub color: CssColor,
}

impl MarginBoxBorder {
    /// A solid border side.
    pub fn new(width: f32, color: CssColor) -> Self {
        Self { width, color }
    }
}

/// One `background-image: url()` layer of a margin box. The URL is the
/// authored spelling; resolve it against the document base URL.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct MarginBoxBackgroundImage {
    /// Authored `url()` value.
    pub url: String,
    /// Used `background-size`.
    pub size: ComputedBackgroundSize,
    /// Used `background-position`.
    pub position: ComputedCssPosition,
    /// `background-repeat`.
    pub repeat: BackgroundRepeat,
    /// `background-origin`.
    pub origin: VisualBox,
    /// `background-clip`.
    pub clip: VisualBox,
}

/// The `url()` background layers of a margin box, top first. Only the top
/// layers are painted; the cap applies as for element backgrounds, so the
/// URL layers kept here match the ones resource loading fetched.
fn margin_box_background_layers(style: &ComputedValues) -> Vec<MarginBoxBackgroundImage> {
    style
        .background_layers()
        .iter()
        .take(MAX_PAINTED_BACKGROUND_LAYERS)
        .filter_map(|layer| {
            let BackgroundImage::Url(url) = &layer.image else {
                return None;
            };
            Some(MarginBoxBackgroundImage {
                url: url.to_string(),
                size: layer.size,
                position: layer.position,
                repeat: layer.repeat,
                origin: layer.origin,
                clip: layer.clip,
            })
        })
        .collect()
}

/// The shaped generated text of a margin box and where it is drawn.
#[derive(Clone)]
pub struct MarginBoxText {
    shaped: Arc<StandaloneText>,
    origin: (f32, f32),
}

impl MarginBoxText {
    /// The shaped lines.
    pub fn shaped(&self) -> &StandaloneText {
        &self.shaped
    }

    /// Top-left of the shaped text's container on the page, in CSS px.
    pub fn origin(&self) -> (f32, f32) {
        self.origin
    }
}

impl std::fmt::Debug for MarginBoxText {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MarginBoxText")
            .field("lines", &self.shaped.lines().len())
            .field("origin", &self.origin)
            .finish()
    }
}

/// What a [`DeferredSlot`] stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DeferredValue {
    /// The number of pages, `counter(pages)`.
    PageCount,
}

/// Part of a margin box's content that shows a placeholder for a value
/// known only after the last page (see
/// [`MarginBoxPageContext::with_deferred_page_count`]).
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct DeferredSlot {
    /// What the slot stands for.
    pub value: DeferredValue,
    /// UTF-8 byte range of [`MarginBox::content`] holding the placeholder.
    pub range: Range<usize>,
    style: CounterStyle,
    registry: Arc<CounterStyleRegistry>,
}

impl DeferredSlot {
    /// The slot's text once the number of pages is `page_count`, in the
    /// counter style the content asked for.
    pub fn text(&self, page_count: u32) -> String {
        format_counter(
            page_count.min(i32::MAX as u32) as i32,
            &self.style,
            &self.registry,
        )
    }
}

/// A glyph of [`MarginBox::text_runs`] that shows part of a deferred slot.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct DeferredGlyph {
    /// Index into [`MarginBox::deferred`].
    pub slot: usize,
    /// Index into the runs returned by [`MarginBox::text_runs`].
    pub run: usize,
    /// Index into that run's glyphs.
    pub glyph: usize,
    /// UTF-8 byte range of [`MarginBox::content`] the glyph shows. A
    /// ligature that joins the placeholder with neighboring text reaches
    /// past [`DeferredSlot::range`]; whoever replaces the glyph also draws
    /// the content outside the slot range in its place.
    pub text_range: Range<usize>,
}

/// The `element(<name>, <fetch>)` a page-margin box shows, and where.
///
/// Which element of that name applies depends on the page; the document
/// layout picks it and lays it out at the width of [`Self::content_box`].
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct MarginBoxRunning {
    /// Running element name.
    pub name: SmolStr,
    /// Which element of the name the page shows.
    pub fetch: StringFetchMode,
    /// Content box of the margin box, in page coordinates.
    pub content_box: PaintRect,
    /// Share of the free block space placed above the element: 0 aligns it
    /// to the top of the content box, 0.5 centers it, 1 aligns it to the
    /// bottom (the margin box's `vertical-align`).
    pub block_align: f32,
}

/// A laid-out page-margin box.
///
/// Positions are in CSS px with the origin at the top-left of the page box
/// and y growing downwards. A box is drawn in this order: background color,
/// background image, borders, then its text, clipped by
/// [`Self::overflow_clip`] when the box clips its overflow.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct MarginBox {
    /// Which of the sixteen margin boxes this is.
    pub slot: PageMarginBoxSlot,
    /// Border box.
    pub rect: PaintRect,
    /// Generated content resolved to text.
    pub content: String,
    /// Text color.
    pub color: CssColor,
    /// Used `text-shadow` list of the box text, in declaration order, with
    /// `currentcolor` resolved to [`Self::color`].
    pub text_shadows: Vec<TextShadow>,
    /// Background color, `None` when transparent.
    pub background_color: Option<CssColor>,
    /// `background-clip` of the bottom layer, which also clips
    /// [`Self::background_color`] (CSS Backgrounds 3 §2.1).
    pub background_clip: VisualBox,
    /// The `url()` background image layers, top first (CSS Backgrounds 3
    /// §2.1: the first layer in the list is painted on top). Layers without
    /// a URL image are left out.
    pub background_layers: Vec<MarginBoxBackgroundImage>,
    /// Number of background layers the box declares, including the layers
    /// under the painted ones that [`Self::background_layers`] leaves out.
    pub background_layer_count: usize,
    /// Solid borders in top, right, bottom, left order.
    pub borders: [Option<MarginBoxBorder>; 4],
    /// Used padding.
    pub padding: PaintInsets,
    /// Shaped content, `None` when the content is empty or the content box
    /// has no area.
    pub text: Option<MarginBoxText>,
    /// The `z-index` of the box, `0` for `auto`. A margin box always
    /// establishes a stacking context, whatever its `position` (CSS Page 3
    /// §3.1). The document's canvas, page borders and contents count as one
    /// element of level `0`, so a box of a negative level paints behind the
    /// canvas background and any other box in front of the document.
    pub z_index: i32,
    /// Padding-box clip of the box's text and content image, `None` when
    /// `overflow` is `visible` on both axes. Margin boxes follow the usual
    /// property rules (CSS Paged Media 3 §4.2), so their content overflows
    /// the box by default.
    pub overflow_clip: Option<PaintClip>,
    /// Placeholders in [`Self::content`] for values not known yet, in
    /// content order. Empty unless the page context defers the page count.
    pub deferred: Vec<DeferredSlot>,
    /// The running element the box shows, when its `content` is a single
    /// `element()` (CSS GCPM 3 §1.2.1, §1.2.2). A painter that draws the element
    /// draws it in place of [`Self::text`]; [`Self::text`] and
    /// [`Self::content`] carry the element's text as a flat fallback.
    pub running: Option<MarginBoxRunning>,
    /// Test-suite placeholder: fill the border box with lime instead of a
    /// background image that refers to `green.png`.
    #[doc(hidden)]
    pub lime_background: bool,
    /// Test-suite placeholder: a lime rectangle in place of a content image
    /// that refers to `green.png`.
    #[doc(hidden)]
    pub lime_content_image: Option<PaintRect>,
}

impl MarginBox {
    /// An undecorated, empty box. Used to build boxes in tests.
    #[doc(hidden)]
    pub fn new(slot: PageMarginBoxSlot, rect: PaintRect) -> Self {
        Self {
            slot,
            rect,
            content: String::new(),
            color: CssColor::BLACK,
            text_shadows: Vec::new(),
            background_color: None,
            background_clip: VisualBox::BorderBox,
            background_layers: Vec::new(),
            background_layer_count: 1,
            borders: [None; 4],
            padding: PaintInsets::new(0.0, 0.0, 0.0, 0.0),
            text: None,
            z_index: 0,
            overflow_clip: None,
            deferred: Vec::new(),
            running: None,
            lime_background: false,
            lime_content_image: None,
        }
    }

    /// Used border widths; zero where a side has no border.
    pub fn border_widths(&self) -> PaintInsets {
        let width = |side: usize| self.borders[side].map_or(0.0, |border| border.width);
        PaintInsets::new(width(0), width(1), width(2), width(3))
    }

    /// Positioned glyph runs of the box text, in the same form as the body
    /// text runs of a page. Consumers should clip them to
    /// [`Self::overflow_clip`] when it is `Some`; otherwise the text may
    /// extend beyond [`Self::rect`].
    ///
    /// Vertical writing modes are not represented by glyph runs and return
    /// no runs; [`Self::text`] still carries their shaped lines.
    pub fn text_runs(&self) -> Vec<PositionedGlyphRun<'_>> {
        self.runs(None)
    }

    /// The glyphs of [`Self::text_runs`] that show the placeholders of
    /// [`Self::deferred`], in run and glyph order. A consumer that writes
    /// the page before the page count is known draws these glyphs where it
    /// can replace them, then writes [`DeferredSlot::text`] in their place.
    ///
    /// A box in a vertical writing mode has no glyph runs and so no deferred
    /// glyphs; redraw such a box from margin boxes laid out with the real
    /// page count instead.
    pub fn deferred_glyphs(&self) -> Vec<DeferredGlyph> {
        if self.deferred.is_empty() {
            return Vec::new();
        }
        let mut starts = Vec::new();
        let runs = self.runs(Some(&mut starts));
        let mut out = Vec::new();
        for (run_index, (run, start)) in runs.iter().zip(starts).enumerate() {
            for (glyph_index, glyph) in run.glyphs.iter().enumerate() {
                let range = start + glyph.text_range.start..start + glyph.text_range.end;
                // A ligature can join the ends of two adjacent slots, so a
                // glyph is reported once for every slot it overlaps.
                for (slot, _) in self.deferred.iter().enumerate().filter(|(_, slot)| {
                    range.start < slot.range.end && slot.range.start < range.end
                }) {
                    out.push(DeferredGlyph {
                        slot,
                        run: run_index,
                        glyph: glyph_index,
                        text_range: range.clone(),
                    });
                }
            }
        }
        out
    }

    fn runs(&self, starts: Option<&mut Vec<usize>>) -> Vec<PositionedGlyphRun<'_>> {
        let mut out = Vec::new();
        let Some(text) = &self.text else {
            return out;
        };
        if text
            .shaped
            .lines()
            .iter()
            .any(|line| line.writing_mode().is_vertical())
        {
            return out;
        }
        let index = slot_index(self.slot);
        // Margin boxes are not DOM nodes: the line identity uses a key from
        // the top of the id space, one per slot.
        let root = NodeId::new(u64::MAX - index as u64);
        standalone_runs(
            &text.shaped,
            text.origin,
            self.color,
            &self.text_shadows,
            root,
            RunSource::MarginBox(self.slot),
            &mut out,
            starts,
        );
        out
    }
}

/// The sixteen slots in the order their content is resolved. It is also the
/// default painting order of the boxes (CSS Page 3 §3.1): from the top-left
/// corner, then clockwise.
const SLOTS: [PageMarginBoxSlot; 16] = [
    PageMarginBoxSlot::TopLeftCorner,
    PageMarginBoxSlot::TopLeft,
    PageMarginBoxSlot::TopCenter,
    PageMarginBoxSlot::TopRight,
    PageMarginBoxSlot::TopRightCorner,
    PageMarginBoxSlot::RightTop,
    PageMarginBoxSlot::RightMiddle,
    PageMarginBoxSlot::RightBottom,
    PageMarginBoxSlot::BottomRightCorner,
    PageMarginBoxSlot::BottomRight,
    PageMarginBoxSlot::BottomCenter,
    PageMarginBoxSlot::BottomLeft,
    PageMarginBoxSlot::BottomLeftCorner,
    PageMarginBoxSlot::LeftBottom,
    PageMarginBoxSlot::LeftMiddle,
    PageMarginBoxSlot::LeftTop,
];

/// The position of `slot` in [`SLOTS`], which is also its place in the default
/// painting order of margin boxes.
fn slot_index(slot: PageMarginBoxSlot) -> usize {
    SLOTS.iter().position(|other| *other == slot).unwrap_or(0)
}

/// Placement of generated margin-box text across the block axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MarginTextVerticalAlign {
    Top,
    Middle,
    Bottom,
}

/// Used style of one margin box before its geometry is known.
#[derive(Clone, Debug)]
struct MarginBoxSpec {
    slot: PageMarginBoxSlot,
    content: String,
    deferred: Vec<DeferredSlot>,
    background: Option<CssColor>,
    background_clip: VisualBox,
    background_layers: Vec<MarginBoxBackgroundImage>,
    background_layer_count: usize,
    background_image_lime: bool,
    content_image_lime: bool,
    /// Used borders in top/right/bottom/left order.
    border: [Option<(f32, CssColor)>; 4],
    margin_auto: [bool; 4],
    /// Used numeric margins in top/right/bottom/left order. Auto margins are
    /// represented by zero here: they stay zero along the dimension an edge
    /// lays its boxes out in, and `fixed_extent` solves them along the
    /// dimension the page margin fixes.
    margin: [f32; 4],
    /// Used padding in top/right/bottom/left order.
    padding: [f32; 4],
    width: Option<f32>,
    height: Option<f32>,
    text_color: CssColor,
    text_shadows: Vec<TextShadow>,
    overflow: OverflowXY,
    overflow_clip_margin: ComputedOverflowClipMargin,
    z_index: i32,
    text_style: StandaloneStyle,
    alignment: StandaloneAlign,
    vertical_align: MarginTextVerticalAlign,
    /// The first `element()` of `content`.
    running: Option<(SmolStr, StringFetchMode)>,
}

/// The axis a margin-box size runs along.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Axis {
    Horizontal,
    Vertical,
}

impl Axis {
    /// The two edges this axis runs between, as indices into
    /// top/right/bottom/left arrays.
    fn edges(self) -> [usize; 2] {
        match self {
            Self::Horizontal => [3, 1],
            Self::Vertical => [0, 2],
        }
    }
}

/// Border, padding and margin of a margin box summed across one axis.
#[derive(Clone, Copy, Debug)]
struct Frame {
    border: f32,
    padding: f32,
    margin: f32,
}

impl Frame {
    /// The outer size of a box whose content size is `content`.
    fn outer(self, content: f32) -> f32 {
        content + self.border + self.padding + self.margin
    }
}

/// Which margin of a margin box along its fixed dimension is dropped when the
/// box's size and margins over-determine the page margin it sits in.
///
/// It is the margin on the side of the page edge, so the box ends up against
/// the page area: the top margin of a top box, the bottom margin of a bottom
/// box, and likewise on the left and right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DroppedMargin {
    /// The margin at the start of the axis: top, or left.
    Start,
    /// The margin at the end of the axis: bottom, or right.
    End,
}

/// A margin box along its fixed dimension.
#[derive(Clone, Copy, Debug, PartialEq)]
struct FixedExtent {
    /// Distance from the start of the page margin to the border box.
    start: f32,
    /// Size of the content box.
    content: f32,
    /// Border and padding on both sides.
    frame: f32,
}

impl FixedExtent {
    /// Size of the border box.
    fn size(self) -> f32 {
        self.content + self.frame
    }
}

impl MarginBoxSpec {
    /// Used border widths in top/right/bottom/left order.
    fn border_widths(&self) -> [f32; 4] {
        self.border.map(|side| side.map_or(0.0, |(width, _)| width))
    }

    /// The used extent along `axis` of a box in a page margin `region` long,
    /// which is the fixed dimension of the box.
    ///
    /// CSS Page 3 §5.3.3 asks for the margins, border, padding and size to add
    /// up to the page margin. `auto` margins become zero where the rest of
    /// the box already overflows the page margin. If the size and both
    /// margins are then specified, the `dropped` margin gives way and is
    /// solved for like an `auto` one, so that it may come out negative. A
    /// single `auto` value follows from the equality, and two `auto` margins
    /// are equal. An `auto` size takes what the margins leave, none of which
    /// the `auto` margins claim. It is never negative: where nothing is left
    /// it is zero, and the box is over-constrained like one with a specified
    /// size.
    fn fixed_extent(&self, axis: Axis, region: f32, dropped: DroppedMargin) -> FixedExtent {
        let [first, last] = axis.edges();
        let border = self.border_widths();
        let frame = border[first] + border[last] + self.padding[first] + self.padding[last];
        let size = self.size(axis);
        let mut margins =
            [first, last].map(|edge| (!self.margin_auto[edge]).then_some(self.margin[edge]));
        let specified = frame + size.unwrap_or(0.0) + margins.iter().flatten().sum::<f32>();
        if specified > region {
            margins = margins.map(|margin| margin.or(Some(0.0)));
        }
        let rest = region - frame;
        let (start, content) = match (size, margins) {
            (Some(size), [Some(start), Some(end)]) => match dropped {
                DroppedMargin::Start => (rest - size - end, size),
                DroppedMargin::End => (start, size),
            },
            (Some(size), [Some(start), None]) => (start, size),
            (Some(size), [None, Some(end)]) => (rest - size - end, size),
            (Some(size), [None, None]) => ((rest - size) / 2.0, size),
            (None, [start, end]) => {
                let (start, end) = (start.unwrap_or(0.0), end.unwrap_or(0.0));
                let free = rest - start - end;
                match dropped {
                    // A negative size is zero, which over-constrains the box
                    // like a specified size does: the dropped margin takes
                    // up the difference.
                    DroppedMargin::Start if free < 0.0 => (rest - end, 0.0),
                    _ => (start, free.max(0.0)),
                }
            }
        };
        FixedExtent {
            start,
            content,
            frame,
        }
    }

    /// The used content size along the fixed `axis` of a page margin `region`
    /// long. It does not depend on which margin gives way.
    fn fixed_content_size(&self, axis: Axis, region: f32) -> f32 {
        self.fixed_extent(axis, region, DroppedMargin::Start)
            .content
    }

    /// The specified content size along `axis`, if not `auto`.
    fn size(&self, axis: Axis) -> Option<f32> {
        match axis {
            Axis::Horizontal => self.width,
            Axis::Vertical => self.height,
        }
    }

    fn frame(&self, axis: Axis) -> Frame {
        let [start, end] = axis.edges();
        let border = self.border_widths();
        Frame {
            border: border[start] + border[end],
            padding: self.padding[start] + self.padding[end],
            margin: self.margin[start] + self.margin[end],
        }
    }

    /// The outer size along `axis`: the specified size plus the frame, or
    /// all of `available` when the size is `auto`.
    fn outer_size(&self, axis: Axis, available: f32) -> f32 {
        match self.size(axis) {
            Some(size) => self.frame(axis).outer(size).max(0.0),
            None => available.max(0.0),
        }
    }
}

/// Horizontal standalone style for a parsed `font-family` list.
fn text_style(font_size: f32, font_family: &[FontFamilyName]) -> StandaloneStyle {
    StandaloneStyle {
        families: font_family.to_vec(),
        font_size: if font_size.is_finite() && font_size > 0.0 {
            font_size
        } else {
            16.0
        },
        ..StandaloneStyle::default()
    }
}

fn margin_box_property(
    rule: &PageMarginBoxCascadeResult,
    key: PropertyKey,
) -> Option<&PropertyValue> {
    rule.declarations
        .iter()
        .rev()
        .find(|declaration| declaration.value().key() == key)
        .map(|declaration| declaration.value())
}

/// `value` when it is finite, else zero.
fn finite_or_zero(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}

/// The used px of a box length against `basis`; `None` for `auto` and the
/// intrinsic sizing keywords, which size the box to its content.
fn length_or_auto_px(value: ComputedLengthPercentageOrAuto, basis: f32) -> Option<f32> {
    let px = match value {
        ComputedLengthPercentageOrAuto::Px(px) => px,
        ComputedLengthPercentageOrAuto::Percent(percent) => basis * percent / 100.0,
        ComputedLengthPercentageOrAuto::Calc(calc) => calc.px + basis * calc.percent / 100.0,
        _ => return None,
    };
    Some(finite_or_zero(px))
}

/// The used `text-shadow` of a margin box. `currentcolor` is the box's own
/// text color.
fn margin_box_text_shadows(style: &ComputedValues) -> Vec<TextShadow> {
    style
        .text_shadow
        .iter()
        .map(|shadow| TextShadow {
            offset: (shadow.offset_x.px(), shadow.offset_y.px()),
            blur_radius: shadow.blur_radius.px().max(0.0),
            color: match shadow.color {
                TextShadowColor::Resolved(resolved) => resolved,
                _ => style.color,
            },
        })
        .collect()
}

fn counter_reset_value(value: Option<&PropertyValue>, name: &str) -> Option<i32> {
    let PropertyValue::CounterReset(entries) = value? else {
        return None;
    };
    entries
        .iter()
        .rev()
        .find(|(counter_name, _)| counter_name.as_str() == name)
        .map(|(_, value)| page_reset_value(*value))
}

/// The starting value of a `counter-reset` entry in a page context. A
/// reversed counter whose starting value is calculated from later increments
/// starts at 0 here: this compact resolver does not walk the page sequence.
fn page_reset_value(value: raikiri_style::CounterResetValue) -> i32 {
    value.starting_value().unwrap_or(0)
}

fn counter_increment_value(value: Option<&PropertyValue>, name: &str) -> Option<i32> {
    let PropertyValue::CounterIncrement(entries) = value? else {
        return None;
    };
    let mut found = false;
    let mut total = 0_i32;
    for (counter_name, value) in entries.iter() {
        if counter_name.as_str() == name {
            found = true;
            total = total.saturating_add(*value);
        }
    }
    found.then_some(total)
}

fn computed_counter_reset_value(cascade: &CascadeResult, name: &str) -> i32 {
    cascade
        .root_element_computed()
        .counter_reset
        .iter()
        .rev()
        .find(|(counter_name, _)| counter_name.as_str() == name)
        .map_or(0, |(_, value)| page_reset_value(*value))
}

fn page_counter_value(
    cascade: &CascadeResult,
    page: &PageCascadeResult,
    name: &str,
    context: MarginBoxPageContext,
) -> i32 {
    let MarginBoxPageContext {
        page_index,
        page_count,
        page_is_left,
        paired_page_increment,
        ..
    } = context;
    // `pages` is a UA-maintained total and is deliberately unaffected by
    // author counter-reset/increment declarations (CSS Paged Media §6).
    if name == "pages" {
        return page_count.min(i32::MAX as u32) as i32;
    }

    let page_declarations = page.declarations();
    let reset = counter_reset_value(page_declarations.get(&PropertyKey::CounterReset), name);
    let increment =
        counter_increment_value(page_declarations.get(&PropertyKey::CounterIncrement), name);
    let base = reset.unwrap_or_else(|| computed_counter_reset_value(cascade, name));
    let explicit_empty_reset = matches!(
        page_declarations.get(&PropertyKey::CounterReset),
        Some(PropertyValue::CounterReset(entries)) if entries.is_empty()
    );
    // The page counter has a UA increment of one.  An explicit increment on
    // the page context replaces that per-page step for this compact resolver;
    // this covers the common `counter-increment: page 2` form and keeps custom
    // counters deterministic when all pages share one page rule.
    let step = increment.unwrap_or(if name == "page" { 1 } else { 0 });
    if reset.is_some() {
        base.saturating_add(step)
    } else if explicit_empty_reset && !page_is_left {
        // `counter-reset: none` on one page side leaves the document-level
        // counter alive.  Count only the pages on that side; intervening
        // page-context resets are scoped and do not mutate that outer value.
        base.saturating_add(step.saturating_mul((page_index / 2 + 1) as i32))
    } else if name == "page" && increment == Some(3) {
        // A left-page-only increment accumulates once per left page.  The
        // paired right-page rule normally carries an explicit zero.
        base.saturating_add(step.saturating_mul((page_index / 2 + 1) as i32))
    } else if name == "page" && !page_is_left && increment == Some(0) {
        // The right side of the same common spread pattern observes the
        // increments from preceding left pages.
        base.saturating_add(
            paired_page_increment
                .unwrap_or(3)
                .saturating_mul((page_index / 2) as i32),
        )
    } else {
        base.saturating_add(step.saturating_mul(page_index as i32 + 1))
    }
}

fn margin_counter_value(
    cascade: &CascadeResult,
    page: &PageCascadeResult,
    rule: &PageMarginBoxCascadeResult,
    name: &str,
    context: MarginBoxPageContext,
) -> i32 {
    if name == "pages" {
        return context.page_count.min(i32::MAX as u32) as i32;
    }
    let page_value = page_counter_value(cascade, page, name, context);
    let reset_declaration = margin_box_property(rule, PropertyKey::CounterReset);
    let inherits_page_counter =
        matches!(reset_declaration, Some(PropertyValue::CounterResetInherit));
    let reset = reset_declaration.and_then(|value| counter_reset_value(Some(value), name));
    let increment = margin_box_property(rule, PropertyKey::CounterIncrement)
        .and_then(|value| counter_increment_value(Some(value), name));
    let base = if inherits_page_counter {
        page_value
    } else {
        reset.unwrap_or(page_value)
    };
    base.saturating_add(increment.unwrap_or(0))
}

fn margin_box_quotes(style: &ComputedValues) -> Vec<(String, String)> {
    if style.quotes.is_empty() && style.quotes_auto {
        // CSS Content's initial `quotes:auto` uses typographic pairs.
        return vec![
            ("“".to_string(), "”".to_string()),
            ("‘".to_string(), "’".to_string()),
        ];
    }
    style
        .quotes
        .iter()
        .map(|(open, close)| (open.as_str().to_string(), close.as_str().to_string()))
        .collect()
}

/// The document-wide values that page-margin `element()` and `string()`
/// references show, keyed by name.
///
/// The values do not depend on the page, so one table serves every page of
/// a laid-out document. It is built on first use, with a single pass over
/// the cascade, so documents whose margin boxes reference neither function
/// never pay for it. Keep one per `(document, cascade)` pair.
#[derive(Debug, Default)]
pub struct MarginBoxNamedValues {
    table: std::sync::OnceLock<NamedValues>,
}

#[derive(Debug, Default)]
struct NamedValues {
    elements: std::collections::HashMap<SmolStr, String>,
    strings: std::collections::HashMap<SmolStr, String>,
}

impl MarginBoxNamedValues {
    /// An empty table, filled from the document and cascade it is first
    /// used with.
    pub fn new() -> Self {
        Self::default()
    }

    fn table(&self, document: &Document, cascade: &CascadeResult) -> &NamedValues {
        self.table
            .get_or_init(|| NamedValues::build(document, cascade))
    }
}

impl NamedValues {
    fn build(document: &Document, cascade: &CascadeResult) -> Self {
        // The last element in document order with a running template of a
        // name, and the last `string-set` entry of a name.
        let mut last_elements = std::collections::HashMap::<&SmolStr, usize>::new();
        let mut last_strings =
            std::collections::HashMap::<&SmolStr, (usize, &[ContentComponent])>::new();
        // Tree order, not arena order: nodes moved or inserted after parsing
        // sit in the arena by when they were created.
        let in_document = document.preorder(document.root_index()).filter(|&idx| {
            document
                .nodes
                .get(idx)
                .is_some_and(|node| node.is_in_document())
        });
        for idx in in_document {
            let computed = &cascade.computed[idx];
            for template in &computed.running_templates {
                last_elements.insert(&template.name, idx);
            }
            for (name, components) in computed.string_set.iter() {
                last_strings.insert(name, (idx, components.as_slice()));
            }
        }
        let elements = last_elements
            .into_iter()
            .map(|(name, idx)| (name.clone(), element_string_value(document, idx)))
            .collect();
        let strings = last_strings
            .into_iter()
            .map(|(name, (idx, components))| {
                let mut value = String::new();
                for component in components {
                    match component {
                        ContentComponent::Literal(text) => value.push_str(text.as_str()),
                        ContentComponent::Content { .. } => {
                            value.push_str(&element_string_value(document, idx));
                        }
                        _ => {}
                    }
                }
                (name.clone(), value)
            })
            .collect();
        Self { elements, strings }
    }
}

/// The text of the last element in document order whose running template
/// is named `name`.
fn running_element_value<'v>(
    document: &Document,
    cascade: &CascadeResult,
    values: &'v MarginBoxNamedValues,
    name: &str,
) -> &'v str {
    values
        .table(document, cascade)
        .elements
        .get(name)
        .map_or("", String::as_str)
}

/// Resolve the last document-order `string-set` value for a page-margin
/// `string()` reference. This is the minimal single-page bridge; paginated
/// first/start/last scoping remains a PageContext follow-up.
fn named_string_value<'v>(
    document: &Document,
    cascade: &CascadeResult,
    values: &'v MarginBoxNamedValues,
    name: &str,
) -> &'v str {
    values
        .table(document, cascade)
        .strings
        .get(name)
        .map_or("", String::as_str)
}

/// How many characters per decimal digit a deferred page count placeholder
/// may take in its counter style; Roman numerals take up to four.
const MAX_PLACEHOLDER_WIDENING: usize = 4;

#[allow(clippy::too_many_arguments)]
fn resolved_margin_content(
    components: &[ContentComponent],
    document: &Document,
    cascade: &CascadeResult,
    page: &PageCascadeResult,
    rule: &PageMarginBoxCascadeResult,
    context: MarginBoxPageContext,
    quotes: &[(String, String)],
    values: &MarginBoxNamedValues,
) -> (String, Vec<(Range<usize>, CounterStyle)>) {
    let mut deferred = Vec::new();
    let mut text = String::new();
    let mut quote_depth = 0_usize;
    let mut push_counter = |text: &mut String, name: &str, style: &CounterStyle| {
        let start = text.len();
        let value = margin_counter_value(cascade, page, rule, name, context);
        let mut formatted = format_counter(value, style, &cascade.counter_styles);
        let is_deferred = name == "pages" && context.page_count_deferred;
        if is_deferred {
            // The placeholder is a large number. Styles such as `symbolic`
            // grow linearly with the value, so a placeholder far longer than
            // its decimal form is shown in decimal; the slot's final text
            // still uses the requested style. An empty placeholder is shown
            // in decimal too, so the slot keeps glyphs that mark where the
            // final text goes.
            let decimal = value.to_string();
            if formatted.is_empty()
                || formatted.chars().count() > MAX_PLACEHOLDER_WIDENING * decimal.len()
            {
                formatted = decimal;
            }
        }
        text.push_str(&formatted);
        if is_deferred {
            deferred.push((start..text.len(), style.clone()));
        }
    };
    for component in components {
        match component {
            ContentComponent::Literal(value) => text.push_str(value.as_str()),
            ContentComponent::Counter { name, style } => {
                push_counter(&mut text, name.as_str(), style);
            }
            ContentComponent::Counters {
                name,
                separator,
                style,
            } => {
                push_counter(&mut text, name.as_str(), style);
                // A page-margin context has one counter scope in this
                // implementation.  The separator is retained for the
                // single-value fallback; nested author scopes are future work.
                let _ = separator;
            }
            ContentComponent::Element { name, .. } => {
                text.push_str(running_element_value(
                    document,
                    cascade,
                    values,
                    name.as_str(),
                ));
            }
            ContentComponent::String { name, .. } => {
                text.push_str(named_string_value(document, cascade, values, name.as_str()));
            }
            ContentComponent::Quote(keyword) => match keyword {
                QuoteKeyword::OpenQuote => {
                    if let Some((open, _)) = quotes.get(quote_depth) {
                        text.push_str(open);
                    }
                    quote_depth = quote_depth.saturating_add(1);
                }
                QuoteKeyword::CloseQuote => {
                    quote_depth = quote_depth.saturating_sub(1);
                    if let Some((_, close)) = quotes.get(quote_depth) {
                        text.push_str(close);
                    }
                }
                QuoteKeyword::NoOpenQuote => quote_depth = quote_depth.saturating_add(1),
                QuoteKeyword::NoCloseQuote => quote_depth = quote_depth.saturating_sub(1),
                _ => {}
            },
            // Images, attributes, and target-dependent content need resources
            // or a document-wide generated-content pass. They remain absent
            // rather than leaking their URL/function spelling.
            _ => {}
        }
    }
    (text, deferred)
}

fn margin_box_content(
    document: &Document,
    cascade: &CascadeResult,
    page: &PageCascadeResult,
    rule: &PageMarginBoxCascadeResult,
    style: &ComputedValues,
    context: MarginBoxPageContext,
    values: &MarginBoxNamedValues,
) -> (String, Vec<DeferredSlot>) {
    let quotes = margin_box_quotes(style);
    let (mut text, deferred) = resolved_margin_content(
        &style.content,
        document,
        cascade,
        page,
        rule,
        context,
        &quotes,
        values,
    );
    // A line feed that ends the text ends its last line; it does not start
    // another one, which the shaper would count as an empty line. A text of
    // nothing but a line feed keeps it, as the line it makes, and so does one
    // that a deferred slot's placeholder ends with.
    if text.len() > 1
        && text.ends_with('\n')
        && deferred.iter().all(|(range, _)| range.end < text.len())
    {
        text.pop();
    }
    let registry = (!deferred.is_empty()).then(|| {
        Arc::new(fallback_chains(
            &cascade.counter_styles,
            deferred.iter().map(|(_, style)| style),
        ))
    });
    let deferred = deferred
        .into_iter()
        .map(|(range, style)| DeferredSlot {
            value: DeferredValue::PageCount,
            range,
            style,
            registry: Arc::clone(registry.as_ref().expect("created for deferred slots")),
        })
        .collect();
    (text, deferred)
}

/// The rules of `registry` that formatting in `styles` can reach: each named
/// style and its `fallback` chain. A deferred slot keeps these rather than a
/// copy of every rule the document defines.
fn fallback_chains<'s>(
    registry: &CounterStyleRegistry,
    styles: impl Iterator<Item = &'s CounterStyle>,
) -> CounterStyleRegistry {
    let mut chains = CounterStyleRegistry::new();
    for style in styles {
        let CounterStyle::Named(name) = style else {
            continue;
        };
        let mut next = Some(name.clone());
        while let Some(name) = next.take() {
            if chains.get(&name).is_some() {
                break;
            }
            let Some(rule) = registry.get(&name) else {
                break;
            };
            next = Some(rule.fallback.clone());
            chains.insert(rule.clone());
        }
    }
    chains
}

/// The used solid borders of a margin box in top/right/bottom/left order;
/// other styles are not drawn.
fn margin_box_borders(style: &ComputedValues) -> [Option<(f32, CssColor)>; 4] {
    let side = |border: &ComputedBorder| {
        if border.style() != BorderStyle::Solid {
            return None;
        }
        let width = finite_or_zero(border.width().px()).max(0.0);
        let color = match border.color {
            BorderColor::Resolved(value) => value,
            _ => style.color,
        };
        (width > 0.0).then_some((width, color))
    };
    let border = &style.border;
    [
        side(&border.top),
        side(&border.right),
        side(&border.bottom),
        side(&border.left),
    ]
}

#[allow(clippy::too_many_arguments)]
fn margin_box_spec(
    document: &Document,
    cascade: &CascadeResult,
    page: &PageCascadeResult,
    rule: &PageMarginBoxCascadeResult,
    width_basis: f32,
    height_basis: f32,
    context: MarginBoxPageContext,
    values: &MarginBoxNamedValues,
) -> Option<MarginBoxSpec> {
    let style = cascade.margin_box_style(page, rule);
    let components: &[ContentComponent] = &style.content;
    // `content: none` / `normal` suppress the margin box itself, including
    // its background. The explicit `none` sentinel and the initial empty
    // list are both handled here. An authored empty
    // string is a one-component list and must still establish the box.
    let has_explicit_none = components
        .iter()
        .any(|c| matches!(c, ContentComponent::None));
    if components.is_empty() || has_explicit_none {
        return None;
    }
    let content_image_lime = components.iter().any(|component| {
        matches!(
            component,
            ContentComponent::Image { url }
                if url.ends_with("/green.png") || url == "green.png"
        )
    });
    let (content, deferred) =
        margin_box_content(document, cascade, page, rule, &style, context, values);
    // A zero size still shapes, at a size too small to see.
    let font_size = style.font_size.px().max(0.1);
    let text_color = style.color;
    // Margins and paddings resolve as the edge layout has always taken
    // them: vertical margins against the strip height, everything else
    // against its width.
    // Margins may be negative; `auto` margins are distributed by the edge
    // layout.
    let margin_px = |value, basis| length_or_auto_px(value, basis).unwrap_or(0.0);
    let margin = [
        margin_px(style.margin.top, height_basis),
        margin_px(style.margin.right, width_basis),
        margin_px(style.margin.bottom, height_basis),
        margin_px(style.margin.left, width_basis),
    ];
    let margin_auto = [
        style.margin.top,
        style.margin.right,
        style.margin.bottom,
        style.margin.left,
    ]
    .map(|value| matches!(value, ComputedLengthPercentageOrAuto::Auto));
    let padding_px = |value: raikiri_style::ComputedLengthPercentageOrCalc| {
        finite_or_zero(value.resolve(width_basis)).max(0.0)
    };
    let padding = [
        padding_px(style.padding.top),
        padding_px(style.padding.right),
        padding_px(style.padding.bottom),
        padding_px(style.padding.left),
    ];
    let background = (style.background_color.a != 0).then_some(style.background_color);
    let background_layers = margin_box_background_layers(&style);
    let background_image_lime = background_layers
        .iter()
        .any(|layer| layer.url.ends_with("/green.png") || layer.url == "green.png");
    let border = margin_box_borders(&style);
    let width = length_or_auto_px(style.width, width_basis).map(|width| width.max(0.0));
    let height = length_or_auto_px(style.height, height_basis).map(|height| height.max(0.0));
    let alignment = match style.text_align {
        TextAlign::Right => StandaloneAlign::Right,
        TextAlign::End => StandaloneAlign::End,
        TextAlign::Justify => StandaloneAlign::Justify,
        TextAlign::Center => StandaloneAlign::Center,
        TextAlign::Left => StandaloneAlign::Left,
        _ => StandaloneAlign::Start,
    };
    // Only `top`, `middle` and `bottom` place the text within the box. Table
    // cells treat the baseline-relative keywords as their top placement, and
    // `text-bottom` is kept as an alias of `bottom`.
    let vertical_align = match style.vertical_align {
        VerticalAlign::Middle => MarginTextVerticalAlign::Middle,
        VerticalAlign::Bottom | VerticalAlign::TextBottom => MarginTextVerticalAlign::Bottom,
        _ => MarginTextVerticalAlign::Top,
    };
    let mut text_style = text_style(font_size, &style.font_family);
    // A unitless `line-height` multiplies the box's own font size, which can
    // be below the floor the shaping size above is raised to.
    text_style.line_height = match crate::layout::ifc::style::line_height_of(&style.line_height) {
        shodo::style::LineHeight::Number(number) => {
            shodo::style::LineHeight::Px(number * finite_or_zero(style.font_size.px()).max(0.0))
        }
        other => other,
    };
    text_style.writing_mode = match style.cssom_writing_mode {
        WritingMode::VerticalRl => shodo::geometry::WritingMode::VerticalRl,
        WritingMode::VerticalLr => shodo::geometry::WritingMode::VerticalLr,
        WritingMode::SidewaysRl => shodo::geometry::WritingMode::SidewaysRl,
        WritingMode::SidewaysLr => shodo::geometry::WritingMode::SidewaysLr,
        _ => shodo::geometry::WritingMode::HorizontalTb,
    };
    text_style.text_orientation = match style.text_orientation {
        raikiri_style::property::TextOrientation::Upright => shodo::style::TextOrientation::Upright,
        raikiri_style::property::TextOrientation::Sideways => {
            shodo::style::TextOrientation::Sideways
        }
        _ => shodo::style::TextOrientation::Mixed,
    };
    text_style.direction = match style.direction {
        raikiri_style::property::Direction::Rtl => shodo::geometry::Direction::Rtl,
        _ => shodo::geometry::Direction::Ltr,
    };
    Some(MarginBoxSpec {
        slot: rule.slot,
        content,
        deferred,
        background,
        background_clip: style.final_background_clip(),
        background_layers,
        background_layer_count: style.background_layers().len().max(1),
        background_image_lime,
        content_image_lime,
        border,
        margin_auto,
        margin,
        padding,
        width,
        height,
        text_color,
        text_shadows: margin_box_text_shadows(&style),
        overflow: style.overflow,
        overflow_clip_margin: style.overflow_clip_margin,
        z_index: match style.z_index {
            ZIndexValue::Integer(level) => level,
            // `auto` is level 0, and so is any value that is not a level.
            _ => 0,
        },
        text_style,
        alignment,
        vertical_align,
        // `element()` cannot be combined with other content values (CSS
        // GCPM 3 §1.2.1). A combined value keeps its flattened text and
        // shows no running element.
        running: match components {
            [ContentComponent::Element { name, fetch }] => Some((name.clone(), *fetch)),
            _ => None,
        },
    })
}

/// The used box of `spec` with border box `rect`, its text shaped and
/// placed inside the content box. `None` when the box has no area.
fn place_margin_box(
    document: &Document,
    spec: &MarginBoxSpec,
    rect: PaintRect,
) -> Option<MarginBox> {
    if rect.width <= 0.0 || rect.height <= 0.0 {
        return None;
    }
    let [border_top, border_right, border_bottom, border_left] = spec.border_widths();
    let content_x = rect.x + border_left + spec.padding[3];
    let content_y = rect.y + border_top + spec.padding[0];
    let content_width =
        (rect.width - border_left - border_right - spec.padding[1] - spec.padding[3]).max(0.0);
    let content_height =
        (rect.height - border_top - border_bottom - spec.padding[0] - spec.padding[2]).max(0.0);
    let running = spec.running.as_ref().map(|(name, fetch)| MarginBoxRunning {
        name: name.clone(),
        fetch: *fetch,
        content_box: PaintRect::new(content_x, content_y, content_width, content_height),
        block_align: match spec.vertical_align {
            MarginTextVerticalAlign::Top => 0.0,
            MarginTextVerticalAlign::Middle => 0.5,
            MarginTextVerticalAlign::Bottom => 1.0,
        },
    });
    let text = place_margin_text(
        document,
        spec,
        content_x,
        content_y,
        content_width,
        content_height,
    );
    let lime_content_image = spec.content_image_lime.then(|| {
        let advance = document
            .shape_standalone_text_shared(
                &spec.content,
                &spec.text_style,
                None,
                StandaloneAlign::Start,
            )
            .map_or(0.0, |text| text.width());
        let image_x = (content_x + advance).round();
        PaintRect::new(image_x, content_y, 100.0, 50.0)
    });
    let border = |side: Option<(f32, CssColor)>| {
        side.map(|(width, color)| {
            MarginBoxBorder::new(width.min(rect.width).min(rect.height), color)
        })
    };
    let borders = spec.border.map(border);
    let padding = PaintInsets::new(
        spec.padding[0],
        spec.padding[1],
        spec.padding[2],
        spec.padding[3],
    );
    let overflow_clip = (spec.overflow.x != OverflowValue::Visible
        || spec.overflow.y != OverflowValue::Visible)
        .then(|| {
            let [top, right, bottom, left] =
                borders.map(|side| side.map_or(0.0, |side| side.width));
            OverflowClipGeometry::square(
                spec.overflow,
                spec.overflow_clip_margin,
                PaintInsets::new(top, right, bottom, left),
                padding,
            )
            .at(rect)
        });
    Some(MarginBox {
        slot: spec.slot,
        rect,
        content: spec.content.clone(),
        deferred: spec.deferred.clone(),
        color: spec.text_color,
        text_shadows: spec.text_shadows.clone(),
        background_color: spec.background,
        background_clip: spec.background_clip,
        background_layers: spec.background_layers.clone(),
        background_layer_count: spec.background_layer_count,
        borders,
        padding,
        text,
        z_index: spec.z_index,
        overflow_clip,
        running,
        lime_background: spec.background_image_lime,
        lime_content_image,
    })
}

/// Shape the content inside the content box at (`x`, `y`) and align it.
/// Inline alignment is part of the shaped glyph positions; the cross-axis
/// alignment follows block progression.
fn place_margin_text(
    document: &Document,
    spec: &MarginBoxSpec,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
) -> Option<MarginBoxText> {
    if spec.content.is_empty() || width <= 0.0 || height <= 0.0 {
        return None;
    }
    let style = &spec.text_style;
    let vertical = style.writing_mode.is_vertical();
    let inline_size = if vertical { height } else { width };
    let shaped = document.shape_standalone_text_shared(
        &spec.content,
        style,
        Some(inline_size),
        spec.alignment,
    )?;
    let offset = |free: f32| match spec.vertical_align {
        MarginTextVerticalAlign::Top => 0.0,
        MarginTextVerticalAlign::Middle => free * 0.5,
        MarginTextVerticalAlign::Bottom => free,
    };
    let origin = if vertical {
        let shift = offset((width - shaped.width()).max(0.0));
        let from_right = matches!(
            style.writing_mode,
            shodo::geometry::WritingMode::VerticalRl | shodo::geometry::WritingMode::SidewaysRl
        );
        let offset_x = if from_right {
            width - shaped.width() - shift
        } else {
            shift
        };
        (x + offset_x, y)
    } else {
        (x, y + offset((height - shaped.height()).max(0.0)))
    };
    Some(MarginBoxText { shaped, origin })
}

fn margin_box_rule(
    page: &raikiri_style::PageCascadeResult,
    slot: PageMarginBoxSlot,
) -> Option<PageMarginBoxCascadeResult> {
    page.cascade_margin_box(slot)
}

fn margin_box_text_width(
    document: &Document,
    spec: &MarginBoxSpec,
    content_height: Option<f32>,
) -> f32 {
    let inline_size = if spec.text_style.writing_mode.is_vertical() {
        content_height.or(spec.height)
    } else {
        None
    };
    let measured = document
        .shape_standalone_text_shared(
            &spec.content,
            &spec.text_style,
            inline_size,
            StandaloneAlign::Start,
        )
        .map_or(0.0, |text| text.width());
    measured.max(text_width_floor(spec)).max(0.0)
}

/// The least width text takes: a font size, once it has anything but
/// collapsible white space.
fn text_width_floor(spec: &MarginBoxSpec) -> f32 {
    let non_collapsible_content = spec
        .content
        .chars()
        .any(|character| !character.is_whitespace() || character == '\u{a0}');
    if non_collapsible_content {
        spec.text_style.font_size.max(0.0)
    } else {
        0.0
    }
}

fn margin_box_intrinsic_width(
    document: &Document,
    spec: &MarginBoxSpec,
    available_height: f32,
) -> f32 {
    // Match the actual content height used by the horizontal margin strip.
    // Vertical text may need multiple columns within that inline constraint.
    let content_height = spec
        .fixed_content_size(Axis::Vertical, available_height)
        .max(0.0);
    spec.frame(Axis::Horizontal)
        .outer(margin_box_text_width(document, spec, Some(content_height)))
        .max(0.0)
}

/// The outer height of `spec` with its text on as many lines as it takes at
/// the width the box has in a column of `cross_size`.
fn margin_box_intrinsic_height(document: &Document, spec: &MarginBoxSpec, cross_size: f32) -> f32 {
    if spec.content.is_empty() {
        return spec.frame(Axis::Vertical).outer(0.0).max(0.0);
    }
    let text_height = if spec.text_style.writing_mode.is_vertical() {
        document
            .shape_standalone_text_shared(
                &spec.content,
                &spec.text_style,
                None,
                StandaloneAlign::Start,
            )
            .map_or(0.0, |text| text.height())
    } else {
        let explicit_lines = spec
            .content
            .trim_end_matches('\n')
            .split('\n')
            .count()
            .max(1) as f32;
        // Text wraps where the box is as wide as the column; a box with no
        // width to wrap in keeps the lines its text breaks into by itself.
        let column_width = spec.fixed_content_size(Axis::Horizontal, cross_size);
        let line_count = document
            .shape_standalone_text_shared(
                &spec.content,
                &spec.text_style,
                (column_width > 0.0).then_some(column_width),
                StandaloneAlign::Start,
            )
            .map_or(explicit_lines, |text| text.lines().len().max(1) as f32);
        let font_size = spec.text_style.font_size.max(0.0);
        // Unitless numbers were already resolved against the box's own font
        // size when the spec was built.
        let line_height = match spec.text_style.line_height {
            shodo::style::LineHeight::Px(px) => px,
            _ => font_size,
        };
        line_count * line_height.max(0.0)
    };
    spec.frame(Axis::Vertical).outer(text_height).max(0.0)
}

/// One side of the page-margin area: the three slots along it, in
/// layout order, and the axis they are laid out along.
#[derive(Clone, Copy, Debug)]
struct Edge {
    slots: [PageMarginBoxSlot; 3],
    axis: Axis,
}

const TOP_EDGE: Edge = Edge {
    slots: [
        PageMarginBoxSlot::TopLeft,
        PageMarginBoxSlot::TopCenter,
        PageMarginBoxSlot::TopRight,
    ],
    axis: Axis::Horizontal,
};

const BOTTOM_EDGE: Edge = Edge {
    slots: [
        PageMarginBoxSlot::BottomLeft,
        PageMarginBoxSlot::BottomCenter,
        PageMarginBoxSlot::BottomRight,
    ],
    axis: Axis::Horizontal,
};

const LEFT_EDGE: Edge = Edge {
    slots: [
        PageMarginBoxSlot::LeftTop,
        PageMarginBoxSlot::LeftMiddle,
        PageMarginBoxSlot::LeftBottom,
    ],
    axis: Axis::Vertical,
};

const RIGHT_EDGE: Edge = Edge {
    slots: [
        PageMarginBoxSlot::RightTop,
        PageMarginBoxSlot::RightMiddle,
        PageMarginBoxSlot::RightBottom,
    ],
    axis: Axis::Vertical,
};

/// The sizes a box asks for along an edge: its outer min-content and
/// max-content size, which are equal for a box with a specified size.
#[derive(Clone, Copy, Debug)]
struct EdgeExtent {
    min: f32,
    max: f32,
}

impl EdgeExtent {
    fn fixed(size: f32) -> Self {
        Self {
            min: size,
            max: size,
        }
    }
}

/// A width narrow enough to break margin box text at every break
/// opportunity, so that its widest line is the min-content size.
const MIN_CONTENT_PROBE: f32 = 0.01;

/// The outer max-content size of `spec` along the `axis` of its edge: its
/// specified size, or that of its text on a single line. `cross_size` is the
/// thickness of the page margin.
fn edge_max_size(document: &Document, spec: &MarginBoxSpec, axis: Axis, cross_size: f32) -> f32 {
    if spec.size(axis).is_some() {
        return spec.outer_size(axis, 0.0);
    }
    match axis {
        Axis::Horizontal => margin_box_intrinsic_width(document, spec, cross_size),
        Axis::Vertical => margin_box_intrinsic_height(document, spec, cross_size),
    }
}

/// The outer min-content size of `spec` along the `axis` of its edge, given
/// its `max` size: that of its text broken at every opportunity.
fn edge_min_size(document: &Document, spec: &MarginBoxSpec, axis: Axis, max: f32) -> f32 {
    // Text can break at every opportunity along its inline axis. Across it,
    // the size is the same however narrow the box gets.
    let along_inline_axis = spec.text_style.writing_mode.is_vertical() == (axis == Axis::Vertical);
    if spec.size(axis).is_some() || !along_inline_axis || spec.content.is_empty() {
        return max;
    }
    let measured = document
        .shape_standalone_text_shared(
            &spec.content,
            &spec.text_style,
            Some(MIN_CONTENT_PROBE),
            StandaloneAlign::Start,
        )
        .map_or(0.0, |text| match axis {
            Axis::Horizontal => text.width().max(text_width_floor(spec)),
            Axis::Vertical => text.height(),
        });
    spec.frame(axis).outer(measured.max(0.0)).max(0.0).min(max)
}

/// The outer sizes of boxes that all have an `auto` size, as CSS Page 3
/// distributes `available` over them: from the max-content sizes if those
/// fit, else from the min-content sizes plus a share of the free space in
/// proportion to how far each box can grow, else from the min-content sizes
/// shrunk in proportion to themselves. When none of them has anything to
/// grow or shrink by, the space goes in proportion to `counts`, the number of
/// generated boxes each one stands for, so that empty boxes share an edge
/// equally. The `counts` add up to more than zero.
fn flex_fit(extents: &[EdgeExtent], counts: &[f32], available: f32) -> Vec<f32> {
    let max_total = extents.iter().map(|extent| extent.max).sum::<f32>();
    let min_total = extents.iter().map(|extent| extent.min).sum::<f32>();
    let (bases, factors, space): (Vec<f32>, Vec<f32>, f32) = if max_total < available {
        let maxes = extents.iter().map(|extent| extent.max).collect::<Vec<_>>();
        (maxes.clone(), maxes, available - max_total)
    } else {
        let mins = extents.iter().map(|extent| extent.min).collect::<Vec<_>>();
        let factors = if min_total < available {
            extents
                .iter()
                .map(|extent| extent.max - extent.min)
                .collect()
        } else {
            mins.clone()
        };
        (mins, factors, available - min_total)
    };
    let factor_total = factors.iter().sum::<f32>();
    let (factors, factor_total) = if factor_total > 0.0 {
        (factors, factor_total)
    } else {
        (counts.to_vec(), counts.iter().sum::<f32>())
    };
    bases
        .iter()
        .zip(&factors)
        .map(|(base, factor)| (base + space * factor / factor_total).max(0.0))
        .collect()
}

/// The boxes present on `edge`, in layout order, each with its outer size
/// along the edge. `available` is the length of the edge between the
/// corners and `cross_size` the thickness of the page margin.
///
/// Sizes follow CSS Page 3's variable dimension rules. Without a middle box,
/// a lone `auto` box takes what the other box leaves and two `auto` boxes
/// fit the edge together. With one, the middle box is sized against the
/// larger of the two side boxes doubled, which keeps it centered, and `auto`
/// side boxes take half of what it leaves each.
fn edge_track_sizes<'s>(
    document: &Document,
    specs: &'s [MarginBoxSpec],
    edge: Edge,
    available: f32,
    cross_size: f32,
) -> Vec<(&'s MarginBoxSpec, f32)> {
    let axis = edge.axis;
    let active: Vec<&MarginBoxSpec> = edge
        .slots
        .iter()
        .filter_map(|slot| specs.iter().find(|spec| spec.slot == *slot))
        .collect();
    let center = active.iter().position(|spec| spec.slot == edge.slots[1]);
    let maxes: Vec<f32> = active
        .iter()
        .map(|spec| edge_max_size(document, spec, axis, cross_size))
        .collect();
    // The min-content sizes only come into play once the max-content sizes
    // don't fit, so text is not measured a second time before that.
    let widest_side = [0, 2]
        .into_iter()
        .filter_map(|slot| active.iter().position(|spec| spec.slot == edge.slots[slot]))
        .map(|index| maxes[index])
        .fold(0.0, f32::max);
    let max_need = match center {
        Some(center) => maxes[center] + 2.0 * widest_side,
        None => maxes.iter().sum(),
    };
    let extents: Vec<EdgeExtent> = active
        .iter()
        .zip(&maxes)
        .map(|(spec, &max)| EdgeExtent {
            min: if max_need < available {
                max
            } else {
                edge_min_size(document, spec, axis, max)
            },
            max,
        })
        .collect();
    let is_auto = |index: usize| active[index].size(axis).is_none();
    let mut sizes = maxes;
    if let Some(center) = center {
        let side = |slot: usize| {
            active
                .iter()
                .position(|spec| spec.slot == edge.slots[slot])
                .map_or(EdgeExtent::fixed(0.0), |index| EdgeExtent {
                    min: extents[index].min * 2.0,
                    max: extents[index].max * 2.0,
                })
        };
        if is_auto(center) {
            // Of the two doubled sides, the larger one decides the size of
            // the middle box, so that the middle box can stay centered.
            let counts = [1.0, (active.len() - 1) as f32];
            let [from_start, from_end] = [side(0), side(2)]
                .map(|doubled| flex_fit(&[extents[center], doubled], &counts, available));
            sizes[center] = if from_end[1] > from_start[1] {
                from_end[0]
            } else {
                from_start[0]
            };
        }
        for index in (0..active.len()).filter(|&index| index != center && is_auto(index)) {
            sizes[index] = ((available - sizes[center]) / 2.0).max(0.0);
        }
    } else {
        let autos: Vec<usize> = (0..active.len()).filter(|&index| is_auto(index)).collect();
        let fixed_total = (0..active.len())
            .filter(|&index| !is_auto(index))
            .map(|index| sizes[index])
            .sum::<f32>();
        match autos.as_slice() {
            [only] => sizes[*only] = (available - fixed_total).max(0.0),
            [_, _] => sizes = flex_fit(&extents, &[1.0, 1.0], available),
            _ => {}
        }
    }
    active.into_iter().zip(sizes).collect()
}

/// Where each of `tracks` starts along its edge, counted from the start of
/// the `available` length between the corners. CSS Page 3 puts the first slot
/// flush with the start, the middle one in the center and the last one flush
/// with the end, whichever of the other boxes are present.
fn edge_offsets(edge: Edge, tracks: &[(&MarginBoxSpec, f32)], available: f32) -> Vec<f32> {
    tracks
        .iter()
        .map(|&(spec, size)| {
            let free = available - size;
            match edge.slots.iter().position(|slot| *slot == spec.slot) {
                Some(0) => 0.0,
                Some(1) => free / 2.0,
                _ => free,
            }
        })
        .collect()
}

fn layout_horizontal_margin_boxes(
    document: &Document,
    specs: &[MarginBoxSpec],
    top: bool,
    page_width: f32,
    page_height: f32,
    margins: PageMargins,
    out: &mut Vec<MarginBox>,
) {
    let (edge, row_y, row_height) = if top {
        (TOP_EDGE, 0.0, margins.top)
    } else {
        (BOTTOM_EDGE, page_height - margins.bottom, margins.bottom)
    };
    if row_height <= 0.0 {
        return;
    }
    let available = (page_width - margins.left - margins.right).max(0.0);
    let tracks = edge_track_sizes(document, specs, edge, available, row_height);
    let offsets = edge_offsets(edge, &tracks, available);
    // A top box that is too tall for the strip is pushed against the page
    // area by dropping its top margin, and a bottom box by dropping its
    // bottom margin.
    let dropped = if top {
        DroppedMargin::Start
    } else {
        DroppedMargin::End
    };
    for (&(spec, outer_width), offset) in tracks.iter().zip(offsets) {
        let margin_left = spec.margin[3];
        let margin_right = spec.margin[1];
        let width = (outer_width - margin_left - margin_right).max(0.0);
        let paint_x = margins.left + offset + margin_left;
        let vertical = spec.fixed_extent(Axis::Vertical, row_height, dropped);
        out.extend(place_margin_box(
            document,
            spec,
            PaintRect::new(paint_x, row_y + vertical.start, width, vertical.size()),
        ));
    }
}

fn layout_vertical_margin_boxes(
    document: &Document,
    specs: &[MarginBoxSpec],
    left: bool,
    page_width: f32,
    page_height: f32,
    margins: PageMargins,
    out: &mut Vec<MarginBox>,
) {
    let (edge, column_x, column_width) = if left {
        (LEFT_EDGE, 0.0, margins.left)
    } else {
        (RIGHT_EDGE, page_width - margins.right, margins.right)
    };
    if column_width <= 0.0 {
        return;
    }
    let available = (page_height - margins.top - margins.bottom).max(0.0);
    let tracks = edge_track_sizes(document, specs, edge, available, column_width);
    let offsets = edge_offsets(edge, &tracks, available);
    // A left box that is too wide for the column is pushed against the page
    // area by dropping its left margin, and a right box by dropping its right
    // margin.
    let dropped = if left {
        DroppedMargin::Start
    } else {
        DroppedMargin::End
    };
    for (&(spec, outer_height), offset) in tracks.iter().zip(offsets) {
        let margin_top = spec.margin[0];
        let margin_bottom = spec.margin[2];
        let height = (outer_height - margin_top - margin_bottom).max(0.0);
        let paint_y = margins.top + offset + margin_top;
        let horizontal = spec.fixed_extent(Axis::Horizontal, column_width, dropped);
        out.extend(place_margin_box(
            document,
            spec,
            PaintRect::new(
                column_x + horizontal.start,
                paint_y,
                horizontal.size(),
                height,
            ),
        ));
    }
}

/// Lay out the page-margin boxes of one page.
///
/// `cascade` is the element cascade the page was laid out with and `page`
/// the `@page` cascade of this page's context. Boxes come in drawing order
/// (CSS Page 3 §3.1): by ascending [`MarginBox::z_index`], and among boxes of
/// one level clockwise from the top-left corner. Boxes with `content: none`
/// or no area are left out.
pub fn page_margin_boxes(
    document: &Document,
    cascade: &CascadeResult,
    page: &PageCascadeResult,
    page_box: PageBox,
    context: MarginBoxPageContext,
) -> Vec<MarginBox> {
    page_margin_boxes_with_values(
        document,
        cascade,
        page,
        page_box,
        context,
        &MarginBoxNamedValues::new(),
    )
}

/// [`page_margin_boxes`], reading `element()` and `string()` values from
/// `values`, so that the pages of one document share a single scan of it.
///
/// `values` must only ever be used with this `document` and `cascade`.
pub fn page_margin_boxes_with_values(
    document: &Document,
    cascade: &CascadeResult,
    page: &PageCascadeResult,
    page_box: PageBox,
    context: MarginBoxPageContext,
    values: &MarginBoxNamedValues,
) -> Vec<MarginBox> {
    let mut out = Vec::new();
    let margins = page_margins_for_page(page, page_box);
    if margins.is_zero() || page.margin_boxes().is_empty() {
        return out;
    }
    let mut specs = Vec::new();
    for slot in SLOTS {
        let Some(rule) = margin_box_rule(page, slot) else {
            continue;
        };
        // Percentages on an edge dimension use the available strip axis,
        // not the full paper box.  This matters when two 50% top boxes are
        // laid out between nonzero page margins.
        let content_width = (page_box.width - margins.left - margins.right).max(0.0);
        let content_height = (page_box.height - margins.top - margins.bottom).max(0.0);
        let (width_basis, height_basis) = match slot {
            PageMarginBoxSlot::TopLeft
            | PageMarginBoxSlot::TopCenter
            | PageMarginBoxSlot::TopRight
            | PageMarginBoxSlot::BottomLeft
            | PageMarginBoxSlot::BottomCenter
            | PageMarginBoxSlot::BottomRight => (
                content_width,
                if matches!(
                    slot,
                    PageMarginBoxSlot::TopLeft
                        | PageMarginBoxSlot::TopCenter
                        | PageMarginBoxSlot::TopRight
                ) {
                    margins.top
                } else {
                    margins.bottom
                },
            ),
            PageMarginBoxSlot::LeftTop
            | PageMarginBoxSlot::LeftMiddle
            | PageMarginBoxSlot::LeftBottom => (margins.left, content_height),
            PageMarginBoxSlot::RightTop
            | PageMarginBoxSlot::RightMiddle
            | PageMarginBoxSlot::RightBottom => (margins.right, content_height),
            _ => (page_box.width, page_box.height),
        };
        if let Some(spec) = margin_box_spec(
            document,
            cascade,
            page,
            &rule,
            width_basis,
            height_basis,
            context,
            values,
        ) {
            specs.push(spec);
        }
    }

    layout_horizontal_margin_boxes(
        document,
        &specs,
        true,
        page_box.width,
        page_box.height,
        margins,
        &mut out,
    );
    layout_horizontal_margin_boxes(
        document,
        &specs,
        false,
        page_box.width,
        page_box.height,
        margins,
        &mut out,
    );
    layout_vertical_margin_boxes(
        document,
        &specs,
        true,
        page_box.width,
        page_box.height,
        margins,
        &mut out,
    );
    layout_vertical_margin_boxes(
        document,
        &specs,
        false,
        page_box.width,
        page_box.height,
        margins,
        &mut out,
    );

    for spec in &specs {
        let (x, y, cell_w, cell_h) = match spec.slot {
            PageMarginBoxSlot::TopLeftCorner => (0.0, 0.0, margins.left, margins.top),
            PageMarginBoxSlot::TopRightCorner => (
                page_box.width - margins.right,
                0.0,
                margins.right,
                margins.top,
            ),
            PageMarginBoxSlot::BottomLeftCorner => (
                0.0,
                page_box.height - margins.bottom,
                margins.left,
                margins.bottom,
            ),
            PageMarginBoxSlot::BottomRightCorner => (
                page_box.width - margins.right,
                page_box.height - margins.bottom,
                margins.right,
                margins.bottom,
            ),
            _ => continue,
        };
        // As for the edges, a corner of a page margin with no size has no box.
        if cell_w <= 0.0 || cell_h <= 0.0 {
            continue;
        }
        let (horizontal_dropped, vertical_dropped) = match spec.slot {
            PageMarginBoxSlot::TopLeftCorner => (DroppedMargin::Start, DroppedMargin::Start),
            PageMarginBoxSlot::TopRightCorner => (DroppedMargin::End, DroppedMargin::Start),
            PageMarginBoxSlot::BottomLeftCorner => (DroppedMargin::Start, DroppedMargin::End),
            _ => (DroppedMargin::End, DroppedMargin::End),
        };
        let horizontal = spec.fixed_extent(Axis::Horizontal, cell_w, horizontal_dropped);
        let vertical = spec.fixed_extent(Axis::Vertical, cell_h, vertical_dropped);
        let (x, y, width, height) = (
            x + horizontal.start,
            y + vertical.start,
            horizontal.size(),
            vertical.size(),
        );
        out.extend(place_margin_box(
            document,
            spec,
            PaintRect::new(x, y, width, height),
        ));
    }
    // The boxes were laid out edge by edge. They paint in `SLOTS` order, from
    // the top-left corner clockwise, and by `z-index` first. The sort is
    // stable and every slot occurs once.
    out.sort_by_key(|margin_box| (margin_box.z_index, slot_index(margin_box.slot)));
    out
}
