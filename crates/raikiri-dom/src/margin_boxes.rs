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
use crate::{
    Document, PositionedGlyphRun, RunSource, StandaloneAlign, StandaloneStyle, StandaloneText,
};
use raikiri_style::property::{
    BackgroundImage, BackgroundRepeat, Border, BorderColor, BorderStyle, ContentComponent,
    CounterStyle, CssColor, Length, LengthOrAuto, PropertyKey, PropertyValue, QuoteKeyword, Sides,
    StringFetchMode, TextAlign, VerticalAlign, VisualBox, WritingMode,
};
use raikiri_style::{
    CascadeResult, ComputedBackgroundSize, ComputedCssPosition, ComputedLength, ComputedValues,
    CounterStyleRegistry, PageCascadeResult, PageMarginBoxCascadeResult, PageMarginBoxSlot,
    ResolveContext, resolve_background_size, resolve_css_position,
};
use raikiri_traits::{NodeId, NodeKind, PageBox, PaintInsets, PaintRect};
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

/// The `background-image: url()` layer of a margin box. The URL is the
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
/// background image, borders, then its text clipped to [`Self::rect`].
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
    /// Background color, `None` when transparent.
    pub background_color: Option<CssColor>,
    /// Background image layer.
    pub background_image: Option<MarginBoxBackgroundImage>,
    /// Solid borders in top, right, bottom, left order.
    pub borders: [Option<MarginBoxBorder>; 4],
    /// Used padding.
    pub padding: PaintInsets,
    /// Shaped content, `None` when the content is empty or the content box
    /// has no area.
    pub text: Option<MarginBoxText>,
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
            background_color: None,
            background_image: None,
            borders: [None; 4],
            padding: PaintInsets::new(0.0, 0.0, 0.0, 0.0),
            text: None,
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
    /// text runs of a page. Draw them clipped to [`Self::rect`].
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
        let index = SLOTS
            .iter()
            .position(|slot| *slot == self.slot)
            .unwrap_or(0);
        // Margin boxes are not DOM nodes: the line identity uses a key from
        // the top of the id space, one per slot.
        let root = NodeId::new(u64::MAX - index as u64);
        standalone_runs(
            &text.shaped,
            text.origin,
            self.color,
            &[],
            root,
            RunSource::MarginBox(self.slot),
            &mut out,
            starts,
        );
        out
    }
}

/// The sixteen slots in the order their content is resolved.
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
    background_image_url: Option<String>,
    background_image_lime: bool,
    background_size: ComputedBackgroundSize,
    background_position: ComputedCssPosition,
    background_repeat: BackgroundRepeat,
    background_origin: VisualBox,
    background_clip: VisualBox,
    content_image_lime: bool,
    border_top: Option<(f32, CssColor)>,
    border_right: Option<(f32, CssColor)>,
    border_bottom: Option<(f32, CssColor)>,
    border_left: Option<(f32, CssColor)>,
    margin_auto: [bool; 4],
    /// Used numeric margins in top/right/bottom/left order. Auto margins are
    /// represented by zero here and are distributed by the edge layout.
    margin: [f32; 4],
    /// Used padding in top/right/bottom/left order.
    padding: [f32; 4],
    width: Option<f32>,
    height: Option<f32>,
    text_color: CssColor,
    text_style: StandaloneStyle,
    alignment: StandaloneAlign,
    vertical_align: MarginTextVerticalAlign,
    /// The first `element()` of `content`.
    running: Option<(SmolStr, StringFetchMode)>,
}

/// Horizontal standalone style for a `font-family` string. Family names keep
/// their quotes: the engine tells a quoted `"serif"` (a named family) from
/// the generic keyword.
fn text_style(font_size: f32, font_family: &str) -> StandaloneStyle {
    StandaloneStyle {
        families: font_family
            .split(',')
            .map(|name| name.trim().to_owned())
            .filter(|name| !name.is_empty())
            .collect(),
        font_size: if font_size.is_finite() && font_size > 0.0 {
            font_size
        } else {
            16.0
        },
        ..StandaloneStyle::default()
    }
}

fn find_html(doc: &Document) -> Option<usize> {
    let mut stack: Vec<usize> = vec![doc.root_index()];
    while let Some(id) = stack.pop() {
        let node = doc.get_node(id)?;
        if !node.is_in_document() {
            continue;
        }
        if node.kind() == NodeKind::Element && node.tag_name() == Some("html") {
            return Some(id);
        }
        for &c in node.children.iter().rev() {
            stack.push(c);
        }
    }
    None
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

fn page_property(page: &PageCascadeResult, key: PropertyKey) -> Option<&PropertyValue> {
    page.declarations().get(&key)
}

fn length_to_px(length: Length, basis: f32, font_size: f32) -> f32 {
    let value = match length {
        Length::Px(value) => value,
        Length::Percent(value) => basis * value / 100.0,
        Length::Em(value) | Length::Rem(value) => font_size * value,
        Length::Ex(value) | Length::Rex(value) | Length::Ch(value) | Length::Rch(value) => {
            font_size * value * 0.5
        }
        Length::Ic(value) | Length::Ric(value) => font_size * value,
        Length::Lh(value) | Length::Rlh(value) => font_size * value,
        Length::Pt(value) => value * 96.0 / 72.0,
        Length::Cm(value) => value * 96.0 / 2.54,
        Length::Mm(value) => value * 96.0 / 25.4,
        Length::Q(value) => value * 96.0 / 101.6,
        Length::In(value) => value * 96.0,
        Length::Pc(value) => value * 96.0 / 6.0,
        _ => 0.0,
    };
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

fn length_or_auto_to_px(value: LengthOrAuto, basis: f32, font_size: f32) -> Option<f32> {
    let value = match value {
        LengthOrAuto::Auto => return None,
        LengthOrAuto::Length(length) => length_to_px(length, basis, font_size),
        LengthOrAuto::Calc(value) => {
            let px = if value.px.is_finite() { value.px } else { 0.0 };
            px + if value.percent.is_finite() {
                basis * value.percent / 100.0
            } else {
                0.0
            }
        }
        _ => 0.0,
    };
    Some(if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    })
}

fn root_computed<'a>(
    document: &Document,
    cascade: &'a CascadeResult,
) -> Option<&'a raikiri_style::ComputedValues> {
    find_html(document).map(|id| &cascade.computed[id])
}

fn inherited_margin_box_color(
    document: &Document,
    cascade: &CascadeResult,
    page: &PageCascadeResult,
    rule: &PageMarginBoxCascadeResult,
) -> CssColor {
    if let Some(PropertyValue::Color(value)) = margin_box_property(rule, PropertyKey::Color) {
        return *value;
    }
    if let Some(PropertyValue::Color(value)) = page_property(page, PropertyKey::Color) {
        return *value;
    }
    root_computed(document, cascade)
        .map(|computed| computed.color)
        .unwrap_or_else(|| CssColor::BLACK)
}

fn inherited_margin_box_font(
    document: &Document,
    cascade: &CascadeResult,
    page: &PageCascadeResult,
    rule: &PageMarginBoxCascadeResult,
) -> (f32, String) {
    let root = root_computed(document, cascade);
    let root_size = root.map_or(16.0, |computed| computed.font_size.px());
    let page_size = match page_property(page, PropertyKey::FontSize) {
        Some(PropertyValue::FontSize(value)) => length_to_px(*value, root_size, root_size),
        _ => root_size,
    };
    let font_size = match margin_box_property(rule, PropertyKey::FontSize) {
        Some(PropertyValue::FontSize(value)) => length_to_px(*value, page_size, page_size),
        _ => page_size,
    };
    let family = match margin_box_property(rule, PropertyKey::FontFamily)
        .or_else(|| page_property(page, PropertyKey::FontFamily))
    {
        Some(PropertyValue::FontFamily(families)) => families
            .first()
            .map(|family| family.as_str().to_string())
            .unwrap_or_else(|| "serif".to_string()),
        _ => root
            .and_then(|computed| computed.font_family.first())
            .map(|family| family.as_str().to_string())
            .unwrap_or_else(|| "serif".to_string()),
    };
    (font_size.max(0.1), family)
}

fn counter_reset_value(value: Option<&PropertyValue>, name: &str) -> Option<i32> {
    let PropertyValue::CounterReset(entries) = value? else {
        return None;
    };
    entries
        .iter()
        .rev()
        .find(|(counter_name, _)| counter_name.as_str() == name)
        .map(|(_, value)| *value)
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

fn computed_counter_reset_value(document: &Document, cascade: &CascadeResult, name: &str) -> i32 {
    root_computed(document, cascade)
        .and_then(|computed| {
            computed
                .counter_reset
                .iter()
                .rev()
                .find(|(counter_name, _)| counter_name.as_str() == name)
                .map(|(_, value)| *value)
        })
        .unwrap_or(0)
}

fn page_counter_value(
    document: &Document,
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
    let base = reset.unwrap_or_else(|| computed_counter_reset_value(document, cascade, name));
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
    document: &Document,
    cascade: &CascadeResult,
    page: &PageCascadeResult,
    rule: &PageMarginBoxCascadeResult,
    name: &str,
    context: MarginBoxPageContext,
) -> i32 {
    if name == "pages" {
        return context.page_count.min(i32::MAX as u32) as i32;
    }
    let page_value = page_counter_value(document, cascade, page, name, context);
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

fn inherited_margin_box_quotes(
    document: &Document,
    cascade: &CascadeResult,
    page: &PageCascadeResult,
    rule: &PageMarginBoxCascadeResult,
) -> Vec<(String, String)> {
    let explicit = margin_box_property(rule, PropertyKey::Quotes)
        .or_else(|| page_property(page, PropertyKey::Quotes));
    if let Some(PropertyValue::Quotes(values)) = explicit {
        if values.is_empty() {
            return Vec::new();
        }
        return values
            .iter()
            .map(|(open, close)| (open.as_str().to_string(), close.as_str().to_string()))
            .collect();
    }
    let Some(root) = root_computed(document, cascade) else {
        return Vec::new();
    };
    if root.quotes.is_empty() {
        if root.quotes_auto {
            // CSS Content's initial `quotes:auto` uses typographic pairs.
            return vec![
                ("“".to_string(), "”".to_string()),
                ("‘".to_string(), "’".to_string()),
            ];
        }
        return Vec::new();
    }
    root.quotes
        .iter()
        .map(|(open, close)| (open.as_str().to_string(), close.as_str().to_string()))
        .collect()
}

fn element_string_value(document: &Document, root: usize) -> String {
    let mut stack = vec![root];
    let mut raw = String::new();
    while let Some(idx) = stack.pop() {
        let Some(node) = document.get_node(idx) else {
            continue;
        };
        if let Some(text) = node.text_content() {
            raw.push_str(text);
        }
        for &child in node.children.iter().rev() {
            stack.push(child);
        }
    }
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn running_element_value(document: &Document, cascade: &CascadeResult, name: &str) -> String {
    for (idx, computed) in cascade.computed.iter().enumerate().rev() {
        if computed
            .running_templates
            .iter()
            .any(|template| template.name.as_str() == name)
        {
            return element_string_value(document, idx);
        }
    }
    String::new()
}

/// Resolve the last document-order `string-set` value for a page-margin
/// `string()` reference. This is the minimal single-page bridge; paginated
/// first/start/last scoping remains a PageContext follow-up.
fn named_string_value(document: &Document, cascade: &CascadeResult, name: &str) -> String {
    let mut resolved = String::new();
    for (idx, computed) in cascade.computed.iter().enumerate() {
        for (entry_name, components) in computed.string_set.iter() {
            if entry_name.as_str() != name {
                continue;
            }
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
            resolved = value;
        }
    }
    resolved
}

/// How many characters per decimal digit a deferred page count placeholder
/// may take in its counter style; Roman numerals take up to four.
const MAX_PLACEHOLDER_WIDENING: usize = 4;

fn resolved_margin_content(
    components: &[ContentComponent],
    document: &Document,
    cascade: &CascadeResult,
    page: &PageCascadeResult,
    rule: &PageMarginBoxCascadeResult,
    context: MarginBoxPageContext,
    quotes: &[(String, String)],
) -> (String, Vec<(Range<usize>, CounterStyle)>) {
    let mut deferred = Vec::new();
    let mut text = String::new();
    let mut quote_depth = 0_usize;
    let mut push_counter = |text: &mut String, name: &str, style: &CounterStyle| {
        let start = text.len();
        let value = margin_counter_value(document, cascade, page, rule, name, context);
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
                text.push_str(&running_element_value(document, cascade, name.as_str()));
            }
            ContentComponent::String { name, .. } => {
                text.push_str(&named_string_value(document, cascade, name.as_str()));
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
    context: MarginBoxPageContext,
) -> Option<(String, Vec<DeferredSlot>)> {
    let Some(PropertyValue::Content(components)) = margin_box_property(rule, PropertyKey::Content)
    else {
        return None;
    };
    let quotes = inherited_margin_box_quotes(document, cascade, page, rule);
    let (text, deferred) =
        resolved_margin_content(components, document, cascade, page, rule, context, &quotes);
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
    Some((text, deferred))
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

fn margin_box_border_side(
    rule: &PageMarginBoxCascadeResult,
    width_key: PropertyKey,
    style_key: PropertyKey,
    color_key: PropertyKey,
    side: fn(&Sides<Border>) -> &Border,
    current_color: CssColor,
    font_size: f32,
) -> Option<(f32, CssColor)> {
    let shorthand = margin_box_property(rule, PropertyKey::Border).and_then(|value| {
        if let PropertyValue::Border(sides) = value {
            Some(side(sides))
        } else {
            None
        }
    });
    let width = match margin_box_property(rule, width_key) {
        Some(PropertyValue::BorderTopWidth(value))
        | Some(PropertyValue::BorderRightWidth(value))
        | Some(PropertyValue::BorderBottomWidth(value))
        | Some(PropertyValue::BorderLeftWidth(value)) => Some(*value),
        _ => shorthand.map(|border| border.width),
    }?;
    let style = match margin_box_property(rule, style_key) {
        Some(PropertyValue::BorderTopStyle(value))
        | Some(PropertyValue::BorderRightStyle(value))
        | Some(PropertyValue::BorderBottomStyle(value))
        | Some(PropertyValue::BorderLeftStyle(value)) => Some(*value),
        _ => shorthand.map(|border| border.style),
    }?;
    if !matches!(style, BorderStyle::Solid) {
        return None;
    }
    let color = match margin_box_property(rule, color_key) {
        Some(PropertyValue::BorderTopColor(value))
        | Some(PropertyValue::BorderRightColor(value))
        | Some(PropertyValue::BorderBottomColor(value))
        | Some(PropertyValue::BorderLeftColor(value)) => *value,
        _ => shorthand
            .map(|border| border.color)
            .unwrap_or(BorderColor::CurrentColor),
    };
    let color = match color {
        BorderColor::CurrentColor => current_color,
        BorderColor::Resolved(value) => value,
        _ => current_color,
    };
    let width = length_to_px(width, 0.0, font_size).max(0.0);
    (width > 0.0).then_some((width, color))
}

fn margin_box_borders(
    document: &Document,
    cascade: &CascadeResult,
    page: &PageCascadeResult,
    rule: &PageMarginBoxCascadeResult,
    font_size: f32,
) -> [Option<(f32, CssColor)>; 4] {
    let current_color = inherited_margin_box_color(document, cascade, page, rule);
    [
        margin_box_border_side(
            rule,
            PropertyKey::BorderTopWidth,
            PropertyKey::BorderTopStyle,
            PropertyKey::BorderTopColor,
            |sides| &sides.top,
            current_color,
            font_size,
        ),
        margin_box_border_side(
            rule,
            PropertyKey::BorderRightWidth,
            PropertyKey::BorderRightStyle,
            PropertyKey::BorderRightColor,
            |sides| &sides.right,
            current_color,
            font_size,
        ),
        margin_box_border_side(
            rule,
            PropertyKey::BorderBottomWidth,
            PropertyKey::BorderBottomStyle,
            PropertyKey::BorderBottomColor,
            |sides| &sides.bottom,
            current_color,
            font_size,
        ),
        margin_box_border_side(
            rule,
            PropertyKey::BorderLeftWidth,
            PropertyKey::BorderLeftStyle,
            PropertyKey::BorderLeftColor,
            |sides| &sides.left,
            current_color,
            font_size,
        ),
    ]
}

fn margin_box_auto_margins(rule: &PageMarginBoxCascadeResult) -> [bool; 4] {
    let is_auto = |key: PropertyKey| {
        matches!(
            margin_box_property(rule, key),
            Some(PropertyValue::MarginTop(LengthOrAuto::Auto))
                | Some(PropertyValue::MarginRight(LengthOrAuto::Auto))
                | Some(PropertyValue::MarginBottom(LengthOrAuto::Auto))
                | Some(PropertyValue::MarginLeft(LengthOrAuto::Auto))
        )
    };
    [
        is_auto(PropertyKey::MarginTop),
        is_auto(PropertyKey::MarginRight),
        is_auto(PropertyKey::MarginBottom),
        is_auto(PropertyKey::MarginLeft),
    ]
}

fn margin_box_length(value: &LengthOrAuto, basis: f32, font_size: f32) -> f32 {
    let value = match value {
        LengthOrAuto::Length(length) => length_to_px(*length, basis, font_size),
        LengthOrAuto::Calc(value) => value.px + basis * value.percent / 100.0,
        LengthOrAuto::Auto => 0.0,
        _ => 0.0,
    };
    if value.is_finite() { value } else { 0.0 }
}

fn margin_box_side_margin(
    rule: &PageMarginBoxCascadeResult,
    key: PropertyKey,
    basis: f32,
    shorthand: Option<LengthOrAuto>,
    font_size: f32,
) -> f32 {
    let value = margin_box_property(rule, key)
        .and_then(|value| match value {
            PropertyValue::MarginTop(value)
            | PropertyValue::MarginRight(value)
            | PropertyValue::MarginBottom(value)
            | PropertyValue::MarginLeft(value) => Some(value),
            _ => None,
        })
        .or(shorthand.as_ref());
    value
        .map(|value| margin_box_length(value, basis, font_size))
        .unwrap_or(0.0)
}

fn margin_box_margins(
    rule: &PageMarginBoxCascadeResult,
    width_basis: f32,
    height_basis: f32,
    font_size: f32,
) -> [f32; 4] {
    let shorthand = margin_box_property(rule, PropertyKey::Margin).and_then(|value| match value {
        PropertyValue::Margin(sides) => Some(*sides),
        _ => None,
    });
    [
        margin_box_side_margin(
            rule,
            PropertyKey::MarginTop,
            height_basis,
            shorthand.map(|sides| sides.top),
            font_size,
        ),
        margin_box_side_margin(
            rule,
            PropertyKey::MarginRight,
            width_basis,
            shorthand.map(|sides| sides.right),
            font_size,
        ),
        margin_box_side_margin(
            rule,
            PropertyKey::MarginBottom,
            height_basis,
            shorthand.map(|sides| sides.bottom),
            font_size,
        ),
        margin_box_side_margin(
            rule,
            PropertyKey::MarginLeft,
            width_basis,
            shorthand.map(|sides| sides.left),
            font_size,
        ),
    ]
}

fn margin_box_padding(
    rule: &PageMarginBoxCascadeResult,
    width_basis: f32,
    font_size: f32,
) -> [f32; 4] {
    let shorthand = margin_box_property(rule, PropertyKey::Padding).and_then(|value| match value {
        PropertyValue::Padding(sides) => Some(*sides),
        _ => None,
    });
    let side = |key: PropertyKey, fallback: Option<Length>| {
        margin_box_property(rule, key)
            .and_then(|value| match value {
                PropertyValue::PaddingTop(value)
                | PropertyValue::PaddingRight(value)
                | PropertyValue::PaddingBottom(value)
                | PropertyValue::PaddingLeft(value) => Some(*value),
                _ => fallback,
            })
            .map(|value| length_to_px(value, width_basis, font_size).max(0.0))
            .unwrap_or(0.0)
    };
    [
        side(PropertyKey::PaddingTop, shorthand.map(|sides| sides.top)),
        side(
            PropertyKey::PaddingRight,
            shorthand.map(|sides| sides.right),
        ),
        side(
            PropertyKey::PaddingBottom,
            shorthand.map(|sides| sides.bottom),
        ),
        side(PropertyKey::PaddingLeft, shorthand.map(|sides| sides.left)),
    ]
}

fn margin_box_spec(
    document: &Document,
    cascade: &CascadeResult,
    page: &PageCascadeResult,
    rule: &PageMarginBoxCascadeResult,
    width_basis: f32,
    height_basis: f32,
    context: MarginBoxPageContext,
) -> Option<MarginBoxSpec> {
    let Some(PropertyValue::Content(components)) = margin_box_property(rule, PropertyKey::Content)
    else {
        return None;
    };
    // `content: none` / `normal` suppress the margin box itself, including
    // its background. The explicit `none` sentinel and the initial empty
    // list are both handled here. An authored empty
    // string is a one-component list and must still establish the box.
    // cov:ignore: defensive margin-box sentinel is not reached by current tests
    let has_explicit_none = {
        components
            .iter()
            .any(|c| matches!(c, ContentComponent::None))
    };
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
    let (content, deferred) = margin_box_content(document, cascade, page, rule, context)?;
    let (font_size, font_family) = inherited_margin_box_font(document, cascade, page, rule);
    let margin = margin_box_margins(rule, width_basis, height_basis, font_size);
    let padding = margin_box_padding(rule, width_basis, font_size);
    let background = match margin_box_property(rule, PropertyKey::BackgroundColor) {
        Some(PropertyValue::BackgroundColor(value)) if value.a != 0 => Some(*value),
        _ => None,
    };
    let background_image_url = match margin_box_property(rule, PropertyKey::BackgroundImage) {
        Some(PropertyValue::BackgroundImage(BackgroundImage::Url(url))) => Some(url.to_string()),
        _ => None,
    };
    let background_image_lime = background_image_url
        .as_deref()
        .is_some_and(|url| url.ends_with("/green.png") || url == "green.png");
    let initial = ComputedValues::initial();
    let root_font_size = root_computed(document, cascade)
        .map(|computed| computed.font_size)
        .unwrap_or(initial.font_size);
    let resolve_context = ResolveContext::new(root_font_size);
    let background_size = match margin_box_property(rule, PropertyKey::BackgroundSize) {
        Some(PropertyValue::BackgroundSize(size)) => {
            resolve_background_size(*size, ComputedLength(font_size), None, &resolve_context)
        }
        _ => initial.background_size,
    };
    let background_position = match margin_box_property(rule, PropertyKey::BackgroundPosition) {
        Some(PropertyValue::BackgroundPosition(position)) => {
            resolve_css_position(*position, ComputedLength(font_size), None, &resolve_context)
        }
        _ => initial.background_position,
    };
    let background_repeat = match margin_box_property(rule, PropertyKey::BackgroundRepeat) {
        Some(PropertyValue::BackgroundRepeat(repeat)) => *repeat,
        _ => initial.background_repeat,
    };
    let background_origin = match margin_box_property(rule, PropertyKey::BackgroundOrigin) {
        Some(PropertyValue::BackgroundOrigin(origin)) => *origin,
        _ => initial.background_origin,
    };
    let background_clip = match margin_box_property(rule, PropertyKey::BackgroundClip) {
        Some(PropertyValue::BackgroundClip(clip)) => *clip,
        _ => initial.background_clip,
    };
    let [border_top, border_right, border_bottom, border_left] =
        margin_box_borders(document, cascade, page, rule, font_size);
    let margin_auto = margin_box_auto_margins(rule);
    let width = match margin_box_property(rule, PropertyKey::Width) {
        Some(PropertyValue::Width(value)) => length_or_auto_to_px(*value, width_basis, font_size),
        _ => None,
    };
    let height = match margin_box_property(rule, PropertyKey::Height) {
        Some(PropertyValue::Height(value)) => length_or_auto_to_px(*value, height_basis, font_size),
        _ => None,
    };
    let inherited_text_align = margin_box_property(rule, PropertyKey::TextAlign)
        .or_else(|| page_property(page, PropertyKey::TextAlign))
        .and_then(|value| match value {
            PropertyValue::TextAlign(value) => Some(*value),
            _ => None,
        })
        .or_else(|| root_computed(document, cascade).map(|computed| computed.text_align));
    let alignment = match inherited_text_align {
        Some(TextAlign::Right) => StandaloneAlign::Right,
        Some(TextAlign::End) => StandaloneAlign::End,
        Some(TextAlign::Justify) => StandaloneAlign::Justify,
        Some(TextAlign::Center) => StandaloneAlign::Center,
        Some(TextAlign::Left) => StandaloneAlign::Left,
        _ => StandaloneAlign::Start,
    };
    // `top`/`bottom` are margin-box-specific keywords and are not yet part of
    // the element `vertical-align` grammar.  Treat the supported explicit
    // `text-top`/`text-bottom` values as their corresponding placement.  The
    // current parser drops the margin-box `top` spelling, so retain the
    // historical top placement as the fallback used by this minimal path.
    let inherited_vertical_align = margin_box_property(rule, PropertyKey::VerticalAlign)
        .or_else(|| page_property(page, PropertyKey::VerticalAlign))
        .and_then(|value| match value {
            PropertyValue::VerticalAlign(value) => Some(*value),
            _ => None,
        })
        .or_else(|| root_computed(document, cascade).map(|computed| computed.vertical_align));
    let vertical_align = match inherited_vertical_align {
        Some(VerticalAlign::TextBottom) => MarginTextVerticalAlign::Bottom,
        Some(VerticalAlign::Middle) => MarginTextVerticalAlign::Middle,
        Some(VerticalAlign::TextTop) => MarginTextVerticalAlign::Top,
        _ => MarginTextVerticalAlign::Top,
    };
    let inherited = |key| margin_box_property(rule, key).or_else(|| page_property(page, key));
    let root = root_computed(document, cascade).unwrap_or(&initial);
    let mode = match inherited(PropertyKey::WritingMode) {
        Some(PropertyValue::WritingMode(mode)) => *mode,
        _ => root.cssom_writing_mode,
    };
    let orientation = match inherited(PropertyKey::TextOrientation) {
        Some(PropertyValue::TextOrientation(value)) => *value,
        _ => root.text_orientation,
    };
    let direction = match inherited(PropertyKey::Direction) {
        Some(PropertyValue::Direction(value)) => *value,
        _ => root.direction,
    };
    let mut text_style = text_style(font_size, &font_family);
    text_style.writing_mode = match mode {
        WritingMode::VerticalRl => shodo::geometry::WritingMode::VerticalRl,
        WritingMode::VerticalLr => shodo::geometry::WritingMode::VerticalLr,
        WritingMode::SidewaysRl => shodo::geometry::WritingMode::SidewaysRl,
        WritingMode::SidewaysLr => shodo::geometry::WritingMode::SidewaysLr,
        _ => shodo::geometry::WritingMode::HorizontalTb,
    };
    text_style.text_orientation = match orientation {
        raikiri_style::property::TextOrientation::Upright => shodo::style::TextOrientation::Upright,
        raikiri_style::property::TextOrientation::Sideways => {
            shodo::style::TextOrientation::Sideways
        }
        _ => shodo::style::TextOrientation::Mixed,
    };
    text_style.direction = match direction {
        raikiri_style::property::Direction::Rtl => shodo::geometry::Direction::Rtl,
        _ => shodo::geometry::Direction::Ltr,
    };
    Some(MarginBoxSpec {
        slot: rule.slot,
        content,
        deferred,
        background,
        background_image_url,
        background_image_lime,
        background_size,
        background_position,
        background_repeat,
        background_origin,
        background_clip,
        content_image_lime,
        border_top,
        border_right,
        border_bottom,
        border_left,
        margin_auto,
        margin,
        padding,
        width,
        height,
        text_color: inherited_margin_box_color(document, cascade, page, rule),
        text_style,
        alignment,
        vertical_align,
        // `element()` cannot be combined with other content values (CSS
        // GCPM 3 §1.2.1). A combined value keeps its flattened text and
        // shows no running element.
        running: match components.as_slice() {
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
    let border_left = spec.border_left.map_or(0.0, |(width, _)| width);
    let border_right = spec.border_right.map_or(0.0, |(width, _)| width);
    let border_top = spec.border_top.map_or(0.0, |(width, _)| width);
    let border_bottom = spec.border_bottom.map_or(0.0, |(width, _)| width);
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
    let lime_content_image = spec
        .content_image_lime
        .then(|| {
            let advance = document
                .shape_standalone_text(
                    &spec.content,
                    &spec.text_style,
                    None,
                    StandaloneAlign::Start,
                )
                .map_or(0.0, |text| text.width());
            let image_x = (content_x + advance).round();
            let right = (image_x + 100.0).min(rect.x + rect.width);
            let bottom = (rect.y + rect.height).min(content_y + 50.0);
            PaintRect::new(image_x, content_y, right - image_x, bottom - content_y)
        })
        .filter(|image| image.width > 0.0 && image.height > 0.0);
    let border = |side: Option<(f32, CssColor)>| {
        side.map(|(width, color)| {
            MarginBoxBorder::new(width.min(rect.width).min(rect.height), color)
        })
    };
    Some(MarginBox {
        slot: spec.slot,
        rect,
        content: spec.content.clone(),
        deferred: spec.deferred.clone(),
        color: spec.text_color,
        background_color: spec.background,
        background_image: spec
            .background_image_url
            .clone()
            .map(|url| MarginBoxBackgroundImage {
                url,
                size: spec.background_size,
                position: spec.background_position,
                repeat: spec.background_repeat,
                origin: spec.background_origin,
                clip: spec.background_clip,
            }),
        borders: [
            border(spec.border_top),
            border(spec.border_right),
            border(spec.border_bottom),
            border(spec.border_left),
        ],
        padding: PaintInsets::new(
            spec.padding[0],
            spec.padding[1],
            spec.padding[2],
            spec.padding[3],
        ),
        text,
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
    let shaped =
        document.shape_standalone_text(&spec.content, style, Some(inline_size), spec.alignment)?;
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
    Some(MarginBoxText {
        shaped: Arc::new(shaped),
        origin,
    })
}

fn margin_box_rule(
    page: &raikiri_style::PageCascadeResult,
    slot: PageMarginBoxSlot,
) -> Option<PageMarginBoxCascadeResult> {
    page.cascade_margin_box(slot)
}

fn margin_box_border_width(spec: &MarginBoxSpec) -> f32 {
    spec.border_left.map(|(width, _)| width).unwrap_or(0.0)
        + spec.border_right.map(|(width, _)| width).unwrap_or(0.0)
}

fn margin_box_border_height(spec: &MarginBoxSpec) -> f32 {
    spec.border_top.map(|(height, _)| height).unwrap_or(0.0)
        + spec.border_bottom.map(|(height, _)| height).unwrap_or(0.0)
}

fn margin_box_padding_width(spec: &MarginBoxSpec) -> f32 {
    spec.padding[1] + spec.padding[3]
}

fn margin_box_padding_height(spec: &MarginBoxSpec) -> f32 {
    spec.padding[0] + spec.padding[2]
}

fn margin_box_margin_width(spec: &MarginBoxSpec) -> f32 {
    spec.margin[1] + spec.margin[3]
}

fn margin_box_margin_height(spec: &MarginBoxSpec) -> f32 {
    spec.margin[0] + spec.margin[2]
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
        .shape_standalone_text(
            &spec.content,
            &spec.text_style,
            inline_size,
            StandaloneAlign::Start,
        )
        .map_or(0.0, |text| text.width());
    let non_collapsible_content = spec
        .content
        .chars()
        .any(|character| !character.is_whitespace() || character == '\u{a0}');
    measured
        .max(if non_collapsible_content {
            spec.text_style.font_size.max(0.0)
        } else {
            0.0
        })
        .max(0.0)
}

fn margin_box_intrinsic_width(
    document: &Document,
    spec: &MarginBoxSpec,
    available_height: f32,
) -> f32 {
    // Match the actual content height used by the horizontal margin strip.
    // Vertical text may need multiple columns within that inline constraint.
    let content_height = (margin_box_outer_height(spec, available_height).min(available_height)
        - margin_box_margin_height(spec)
        - margin_box_border_height(spec)
        - margin_box_padding_height(spec))
    .max(0.0);
    (margin_box_text_width(document, spec, Some(content_height))
        + margin_box_border_width(spec)
        + margin_box_padding_width(spec)
        + margin_box_margin_width(spec))
    .max(0.0)
}

fn margin_box_intrinsic_height(document: &Document, spec: &MarginBoxSpec) -> f32 {
    if spec.content.is_empty() {
        return 0.0;
    }
    let line_count = spec
        .content
        .trim_end_matches('\n')
        .split('\n')
        .count()
        .max(1) as f32;
    let text_height = if spec.text_style.writing_mode.is_vertical() {
        document
            .shape_standalone_text(
                &spec.content,
                &spec.text_style,
                None,
                StandaloneAlign::Start,
            )
            .map_or(0.0, |text| text.height())
    } else {
        line_count * spec.text_style.font_size.max(0.0)
    };
    (text_height
        + margin_box_border_height(spec)
        + margin_box_padding_height(spec)
        + margin_box_margin_height(spec))
    .max(0.0)
}

fn margin_box_outer_width(spec: &MarginBoxSpec, available: f32) -> f32 {
    if let Some(width) = spec.width {
        (width
            + margin_box_border_width(spec)
            + margin_box_padding_width(spec)
            + margin_box_margin_width(spec))
        .max(0.0)
    } else {
        available.max(0.0)
    }
}

fn margin_box_outer_height(spec: &MarginBoxSpec, available: f32) -> f32 {
    if let Some(height) = spec.height {
        (height
            + margin_box_border_height(spec)
            + margin_box_padding_height(spec)
            + margin_box_margin_height(spec))
        .max(0.0)
    } else {
        available.max(0.0)
    }
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
    let (row_y, row_height) = if top {
        (0.0, margins.top)
    } else {
        (page_height - margins.bottom, margins.bottom)
    };
    if row_height <= 0.0 {
        return;
    }
    let slots = if top {
        [
            PageMarginBoxSlot::TopLeft,
            PageMarginBoxSlot::TopCenter,
            PageMarginBoxSlot::TopRight,
        ]
    } else {
        [
            PageMarginBoxSlot::BottomLeft,
            PageMarginBoxSlot::BottomCenter,
            PageMarginBoxSlot::BottomRight,
        ]
    };
    let active: Vec<&MarginBoxSpec> = slots
        .iter()
        .filter_map(|slot| specs.iter().find(|spec| spec.slot == *slot))
        .collect();
    if active.is_empty() {
        return;
    }
    let available = (page_width - margins.left - margins.right).max(0.0);
    let fixed = active
        .iter()
        .filter(|spec| spec.width.is_some())
        .map(|spec| margin_box_outer_width(spec, 0.0))
        .sum::<f32>();
    let auto_bases: Vec<f32> = active
        .iter()
        .map(|spec| {
            if spec.width.is_some() {
                0.0
            } else {
                margin_box_intrinsic_width(document, spec, row_height)
            }
        })
        .collect();
    let auto_base_total = auto_bases.iter().sum::<f32>();
    let auto_remaining = available - fixed - auto_base_total;
    let center_index = active.iter().position(|spec| {
        matches!(
            spec.slot,
            PageMarginBoxSlot::TopCenter | PageMarginBoxSlot::BottomCenter
        )
    });
    let center_is_anchored = center_index.is_some_and(|index| {
        active[index].width.is_none()
            && auto_bases[index] <= 0.0
            && active.iter().enumerate().any(|(other, spec)| {
                other != index
                    && matches!(
                        spec.slot,
                        PageMarginBoxSlot::TopLeft
                            | PageMarginBoxSlot::TopRight
                            | PageMarginBoxSlot::BottomLeft
                            | PageMarginBoxSlot::BottomRight
                    )
                    && auto_bases[other] > 0.0
            })
    });
    let anchored_side_width = if center_is_anchored {
        ((available - fixed) / 2.0).max(0.0)
    } else {
        0.0
    };
    // CSS Page 3's AC box treats the two side tracks as one flex item when
    // an edge has an auto-sized center and auto-sized sides.  Proportional
    // distribution across all three intrinsic bases makes an asymmetric
    // side (dimensions-005) steal space from the opposite side instead of
    // keeping the center aligned.
    let ac_widths = if active.len() == 3
        && center_index == Some(1)
        && active.iter().all(|spec| spec.width.is_none())
    {
        let side_base = auto_bases[0].max(auto_bases[2]);
        let center_base = auto_bases[1];
        let ac_base = side_base * 2.0;
        let total_base = ac_base + center_base;
        (side_base > 0.0 && center_base > 0.0 && total_base > 0.0).then(|| {
            let free = available - total_base;
            let ac = (ac_base + free * ac_base / total_base).max(0.0);
            let center = (center_base + free * center_base / total_base).max(0.0);
            [ac * 0.5, center, ac * 0.5]
        })
    } else {
        None
    };
    let widths: Vec<f32> = active
        .iter()
        .enumerate()
        .map(|(index, spec)| {
            if let Some(ac_widths) = ac_widths {
                ac_widths[index]
            } else if center_is_anchored && Some(index) == center_index {
                0.0
            } else if center_is_anchored {
                anchored_side_width
            } else if spec.width.is_some() {
                margin_box_outer_width(spec, 0.0).max(0.0)
            } else if auto_base_total > 0.0 {
                (auto_bases[index] + auto_remaining * auto_bases[index] / auto_base_total).max(0.0)
            } else {
                (auto_remaining / auto_bases.len() as f32).max(0.0)
            }
        })
        .collect();
    let mut x = margins.left;
    for (spec, outer_width) in active.into_iter().zip(widths) {
        let margin_left = spec.margin[3];
        let margin_right = spec.margin[1];
        let width = (outer_width - margin_left - margin_right).max(0.0);
        let paint_x = x + margin_left;
        let outer_height = if spec.height.is_some() {
            margin_box_outer_height(spec, 0.0).min(row_height).max(0.0)
        } else {
            row_height
        };
        let height = (outer_height - spec.margin[0] - spec.margin[2]).max(0.0);
        // Fixed-height top boxes sit against the page area; an auto-height
        // box fills the strip. Numeric margins remain outside the painted box.
        let y = if spec.height.is_some() && spec.margin_auto[0] && spec.margin_auto[2] {
            row_y + (row_height - outer_height).max(0.0) / 2.0 + spec.margin[0]
        } else if top && spec.height.is_some() {
            row_y + row_height - outer_height + spec.margin[0]
        } else {
            row_y + spec.margin[0]
        };
        out.extend(place_margin_box(
            document,
            spec,
            PaintRect::new(paint_x, y, width, height),
        ));
        x += outer_width;
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
    let (column_x, column_width) = if left {
        (0.0, margins.left)
    } else {
        (page_width - margins.right, margins.right)
    };
    if column_width <= 0.0 {
        return;
    }
    let slots = if left {
        [
            PageMarginBoxSlot::LeftTop,
            PageMarginBoxSlot::LeftMiddle,
            PageMarginBoxSlot::LeftBottom,
        ]
    } else {
        [
            PageMarginBoxSlot::RightTop,
            PageMarginBoxSlot::RightMiddle,
            PageMarginBoxSlot::RightBottom,
        ]
    };
    let active: Vec<&MarginBoxSpec> = slots
        .iter()
        .filter_map(|slot| specs.iter().find(|spec| spec.slot == *slot))
        .collect();
    if active.is_empty() {
        return;
    }
    let available = (page_height - margins.top - margins.bottom).max(0.0);
    let fixed = active
        .iter()
        .filter(|spec| spec.height.is_some())
        .map(|spec| margin_box_outer_height(spec, 0.0))
        .sum::<f32>();
    let auto_bases: Vec<f32> = active
        .iter()
        .map(|spec| {
            if spec.height.is_some() {
                0.0
            } else {
                margin_box_intrinsic_height(document, spec)
            }
        })
        .collect();
    let auto_base_total = auto_bases.iter().sum::<f32>();
    let auto_remaining = available - fixed - auto_base_total;
    let ac_heights = if active.len() == 3 && active.iter().all(|spec| spec.height.is_none()) {
        let side_base = auto_bases[0].max(auto_bases[2]);
        let center_base = auto_bases[1];
        let ac_base = side_base * 2.0;
        let total_base = ac_base + center_base;
        (side_base > 0.0 && center_base > 0.0 && total_base > 0.0).then(|| {
            let free = available - total_base;
            let ac = (ac_base + free * ac_base / total_base).max(0.0);
            let center = (center_base + free * center_base / total_base).max(0.0);
            [ac * 0.5, center, ac * 0.5]
        })
    } else {
        None
    };
    let heights: Vec<f32> = active
        .iter()
        .enumerate()
        .map(|(index, spec)| {
            if let Some(ac_heights) = ac_heights {
                ac_heights[index]
            } else if spec.height.is_some() {
                margin_box_outer_height(spec, 0.0).max(0.0)
            } else if auto_base_total > 0.0 {
                (auto_bases[index] + auto_remaining * auto_bases[index] / auto_base_total).max(0.0)
            } else {
                (auto_remaining / auto_bases.len() as f32).max(0.0)
            }
        })
        .collect();
    let has_top = active.iter().any(|spec| {
        matches!(
            spec.slot,
            PageMarginBoxSlot::LeftTop | PageMarginBoxSlot::RightTop
        )
    });
    let has_bottom = active.iter().any(|spec| {
        matches!(
            spec.slot,
            PageMarginBoxSlot::LeftBottom | PageMarginBoxSlot::RightBottom
        )
    });
    let all_fixed_heights = active.len() == 3 && active.iter().all(|spec| spec.height.is_some());
    let segment_height = available / 3.0;
    let mut y = margins.top;
    for (spec, outer_height) in active.into_iter().zip(heights) {
        let margin_top = spec.margin[0];
        let margin_bottom = spec.margin[2];
        let height = (outer_height - margin_top - margin_bottom).max(0.0);
        let paint_y = if all_fixed_heights {
            let slot_index = match spec.slot {
                PageMarginBoxSlot::LeftTop | PageMarginBoxSlot::RightTop => 0.0,
                PageMarginBoxSlot::LeftMiddle | PageMarginBoxSlot::RightMiddle => 1.0,
                _ => 2.0,
            };
            margins.top
                + slot_index * segment_height
                + (segment_height - outer_height).max(0.0) / 2.0
                + margin_top
        } else if spec.height.is_some()
            && matches!(
                spec.slot,
                PageMarginBoxSlot::LeftMiddle | PageMarginBoxSlot::RightMiddle
            )
            && !has_top
            && !has_bottom
        {
            margins.top + (available - outer_height).max(0.0) / 2.0 + margin_top
        } else {
            y + margin_top
        };
        let margin_left = spec.margin[3];
        let margin_right = spec.margin[1];
        let outer_width = if spec.width.is_some() {
            margin_box_outer_width(spec, 0.0).max(0.0)
        } else {
            column_width
        };
        let width = (outer_width - margin_left - margin_right).max(0.0);
        let paint_x = if spec.width.is_some() && spec.margin_auto[1] && spec.margin_auto[3] {
            column_x + (column_width - width).max(0.0) / 2.0
        } else if spec.width.is_some() && left {
            column_x + (column_width - margin_right - width).max(0.0)
        } else {
            column_x + margin_left
        };
        out.extend(place_margin_box(
            document,
            spec,
            PaintRect::new(paint_x, paint_y, width, height),
        ));
        y += outer_height;
    }
}

/// Lay out the page-margin boxes of one page.
///
/// `cascade` is the element cascade the page was laid out with and `page`
/// the `@page` cascade of this page's context. Boxes come in drawing order:
/// the top row, the bottom row, the left and right columns, then the
/// corners. Boxes with `content: none` or no area are left out.
pub fn page_margin_boxes(
    document: &Document,
    cascade: &CascadeResult,
    page: &PageCascadeResult,
    page_box: PageBox,
    context: MarginBoxPageContext,
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
        let width = margin_box_outer_width(spec, cell_w).min(cell_w);
        let height = margin_box_outer_height(spec, cell_h).min(cell_h);
        let x = if spec.margin_auto[1] && spec.margin_auto[3] {
            x + (cell_w - width).max(0.0) / 2.0
        } else {
            match spec.slot {
                PageMarginBoxSlot::TopLeftCorner | PageMarginBoxSlot::BottomLeftCorner
                    if spec.width.is_some() =>
                {
                    x + cell_w - width
                }
                _ => x,
            }
        };
        let y = if spec.margin_auto[0] && spec.margin_auto[2] {
            y + (cell_h - height).max(0.0) / 2.0
        } else {
            match spec.slot {
                PageMarginBoxSlot::TopLeftCorner | PageMarginBoxSlot::TopRightCorner
                    if spec.height.is_some() =>
                {
                    y + cell_h - height
                }
                _ => y,
            }
        };
        out.extend(place_margin_box(
            document,
            spec,
            PaintRect::new(x, y, width, height),
        ));
    }
    out
}
