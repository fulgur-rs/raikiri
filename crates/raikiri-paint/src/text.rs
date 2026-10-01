//! Text decorations (CSS Text Decoration Level 3 line, style and color, in
//! CSS painting order) and the text of page-margin boxes and generated
//! content, which lies outside the paragraphs of the document.

use std::sync::Arc;

use anyrender::{Glyph as AnyrenderGlyph, PaintScene};
use kurbo::{Affine, BezPath, Cap, Circle, Point, Rect, Stroke, Vec2};
use parley::{
    Alignment, FontContext, FontFamily, FontStyle as ParleyFontStyle, FontWeight,
    Glyph as ParleyGlyph, LayoutContext, LineHeight, PositionedLayoutItem, StyleProperty,
};
use peniko::{Color, Fill};
use raikiri_dom::{Document, StandaloneAlign};
use raikiri_style::property::{
    CssColor, Direction, DisplayValue, FloatValue, PositionValue, TextDecorationColor,
    TextDecorationLine, TextDecorationStyle,
};
use raikiri_style::{ComputedTextDecorationInset, ComputedTextUnderlineOffset, ComputedValues};

/// A decoration line carried from the element that originated it.
///
/// `text-decoration-line` is not an inherited property, but CSS Text
/// Decoration propagates a line from an element to its in-flow descendants.
/// The paint walker therefore carries these values separately from the
/// computed-value inheritance walk. The origin metrics are retained so a
/// descendant with a different font size cannot change the line's thickness
/// or vertical offsets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DecorationSpec {
    line: TextDecorationLine,
    style: TextDecorationStyle,
    color: CssColor,
    origin_thickness: f64,
    origin_ascent: f64,
    origin_descent: f64,
    /// Cumulative vertical-align shift at the decorating box.
    ///
    /// Descendant shifts must not change the decoration's initial position.
    origin_shift_y: f32,
    /// Inline-start/end endpoint offsets from `text-decoration-inset`.
    inset_start: f64,
    inset_end: f64,
    /// Fixed offset for underlines originating at this element.
    underline_offset: f64,
    origin_rtl: bool,
}

/// Return whether an element is a boundary for decoration propagation.
///
/// CSS Text Decoration propagates through in-flow descendants, but not into
/// out-of-flow boxes, floats, or atomic inline-level boxes. The boundary box
/// may still originate its own decoration, which is added after the ancestor
/// context has been cleared.
fn is_decoration_propagation_boundary(cv: &ComputedValues) -> bool {
    let out_of_flow = matches!(cv.position, PositionValue::Absolute | PositionValue::Fixed)
        || !matches!(cv.float, FloatValue::None);
    let atomic_inline = matches!(
        cv.display,
        DisplayValue::InlineBlock
            | DisplayValue::InlineFlex
            | DisplayValue::InlineGrid
            | DisplayValue::InlineTable
    );
    out_of_flow || atomic_inline
}

/// Persistent paint-only context for propagated decorations.
///
/// Each originating element adds one linked node. Cloning the context for a
/// child frame only clones the outer `Arc`; ancestor specifications are never
/// copied into a new vector, even for deeply nested decorated elements.
#[derive(Clone, Debug, Default)]
pub(crate) struct DecorationContext(Option<Arc<DecorationLink>>);

#[derive(Debug)]
struct DecorationLink {
    spec: DecorationSpec,
    parent: DecorationContext,
}

impl DecorationContext {
    /// The specifications in paint order (ancestor to descendant).
    pub(crate) fn specs(&self) -> Vec<&DecorationSpec> {
        self.iter().collect()
    }

    fn push(&self, spec: DecorationSpec) -> Self {
        Self(Some(Arc::new(DecorationLink {
            spec,
            parent: self.clone(),
        })))
    }

    fn iter(&self) -> DecorationContextIter<'_> {
        // The linked context is newest-first, while CSS paint order follows
        // the originating elements from ancestor to descendant. This small
        // per-text-node stack reverses traversal without copying contexts or
        // their specifications during the tree walk.
        let mut stack = Vec::new();
        let mut next = self.0.as_deref();
        while let Some(link) = next {
            stack.push(link);
            next = link.parent.0.as_deref();
        }
        DecorationContextIter { stack }
    }
}

struct DecorationContextIter<'a> {
    stack: Vec<&'a DecorationLink>,
}

impl<'a> Iterator for DecorationContextIter<'a> {
    type Item = &'a DecorationSpec;

    fn next(&mut self) -> Option<Self::Item> {
        self.stack.pop().map(|link| &link.spec)
    }
}

fn element_decoration(cv: &ComputedValues, origin_shift_y: f32) -> Option<DecorationSpec> {
    if !has_paintable_line(cv.text_decoration_line) {
        return None;
    }
    let color = match cv.text_decoration_color {
        TextDecorationColor::CurrentColor => cv.color,
        TextDecorationColor::Resolved(color) => color,
        // `TextDecorationColor` is non-exhaustive so a future value must not
        // make the paint path silently lose an otherwise valid decoration.
        // cov:ignore: `TextDecorationColor` has no future variant in this
        // pinned implementation; this arm is a non-exhaustive forward guard.
        _ => cv.color,
    };
    let (inset_start, inset_end) = match cv.text_decoration_inset {
        // `auto` remains distinct in computed style. The current per-text-node
        // paint segmentation already supplies the automatic boundary behavior,
        // so no additional endpoint offset is needed here.
        ComputedTextDecorationInset::Auto => (0.0, 0.0),
        ComputedTextDecorationInset::Lengths { start, end } => (start.px() as f64, end.px() as f64),
    };
    let underline_offset = match cv.text_underline_offset {
        ComputedTextUnderlineOffset::Auto => 0.0,
        ComputedTextUnderlineOffset::Length(value) => value.px() as f64,
        // A percentage is relative to 1em of the decorating element itself,
        // not of the ancestor that declared it.
        ComputedTextUnderlineOffset::Percent(percent) => {
            cv.font_size.px() as f64 * percent as f64 / 100.0
        }
        ComputedTextUnderlineOffset::Calc(value) => {
            value.px as f64 + cv.font_size.px() as f64 * value.percent as f64 / 100.0
        }
    };
    let underline_offset = if underline_offset.is_finite() {
        underline_offset
    } else {
        // cov:ignore: computed style values are sanitized before paint.
        0.0
    };
    let raw_font_size = cv.font_size.px() as f64;
    let origin_font_size = if raw_font_size.is_finite() {
        raw_font_size.max(1.0)
    } else {
        // cov:ignore: computed font sizes are sanitized before paint; retain a
        // finite fallback at this sink boundary for hostile/future inputs.
        1.0
    };
    Some(DecorationSpec {
        line: cv.text_decoration_line,
        style: cv.text_decoration_style,
        color,
        origin_thickness: (origin_font_size / 16.0).max(1.0),
        // These normalized metrics keep the position tied to the decorating
        // element even when a descendant uses a different font size.
        origin_ascent: origin_font_size * 0.8,
        origin_descent: origin_font_size * 0.2,
        origin_shift_y,
        inset_start,
        inset_end,
        underline_offset,
        origin_rtl: matches!(cv.direction, Direction::Rtl),
    })
}

/// Build the decoration context for an element's children.
///
/// A boundary drops ancestor lines first, then retains a line originated by
/// the boundary element itself. This models the CSS rule that a child cannot
/// cancel an ancestor decoration while atomic/out-of-flow boxes do not receive
/// that ancestor decoration.
pub(crate) fn decorations_for_element(
    decorations: &DecorationContext,
    cv: &ComputedValues,
    origin_shift_y: f32,
) -> DecorationContext {
    // `display: contents` generates no box, so its own decoration has no
    // effect. It also must not block a decoration propagated through it.
    if matches!(cv.display, DisplayValue::Contents) {
        return decorations.clone();
    }
    let base = if is_decoration_propagation_boundary(cv) {
        DecorationContext::default()
    } else {
        decorations.clone()
    };
    match element_decoration(cv, origin_shift_y) {
        Some(spec) => base.push(spec),
        None => base,
    }
}

fn has_paintable_line(line: TextDecorationLine) -> bool {
    line.underline || line.overline || line.line_through
}

pub(crate) fn synthetic_embolden(enabled: bool, font_size: f32) -> Vec2 {
    if enabled {
        let size = font_size as f64;
        Vec2::new((0.015125 * size).min(0.3), (0.0121 * size).min(0.3))
    } else {
        Vec2::ZERO
    }
}

/// Vertical placement of generated margin-box text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MarginTextVerticalAlign {
    Top,
    Middle,
    Bottom,
}

#[cfg(test)]
thread_local! {
    // Per thread, like the context itself: `cargo test` runs tests on several
    // threads, and a process-wide counter would count their contexts too.
    static PARLEY_CONTEXT_CREATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn parley_context_creations() -> usize {
    PARLEY_CONTEXT_CREATIONS.with(std::cell::Cell::get)
}

thread_local! {
    static PARLEY_FONTS: std::cell::RefCell<Option<FontContext>> =
        const { std::cell::RefCell::new(None) };
}

/// Run `body` with this thread's parley font context, created on first use.
///
/// System fonts are scanned when a context is created, so building one per
/// call made every margin-box and generated-text measurement pay for the scan.
/// The context registers no fonts and changes no generic family, so reusing it
/// does not change what it resolves.
fn with_parley_fonts<R>(body: impl FnOnce(&mut FontContext) -> R) -> R {
    PARLEY_FONTS.with(|cell| {
        let mut slot = cell.borrow_mut();
        let fonts = slot.get_or_insert_with(|| {
            #[cfg(test)]
            PARLEY_CONTEXT_CREATIONS.with(|count| count.set(count.get() + 1));
            FontContext::new()
        });
        body(fonts)
    })
}

/// The engine's alignment for a parley alignment.
fn standalone_align(alignment: Alignment) -> StandaloneAlign {
    match alignment {
        Alignment::Start => StandaloneAlign::Start,
        Alignment::End => StandaloneAlign::End,
        Alignment::Left => StandaloneAlign::Left,
        Alignment::Right => StandaloneAlign::Right,
        Alignment::Center => StandaloneAlign::Center,
        Alignment::Justify => StandaloneAlign::Justify,
    }
}

/// Draw generated text in a page-margin box.
///
/// Margin-box content is not a DOM text node, so it has no pre-shaped layout
/// to borrow from the document arena.  This small sink-local shaper uses the
/// same parley/anyrender path as ordinary text and deliberately accepts only
/// the style data needed by the page-context caller.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_margin_text(
    document: Option<&Document>,
    scene: &mut impl PaintScene,
    content: &str,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    color: Color,
    font_size: f32,
    font_family: &str,
    alignment: Alignment,
    vertical_align: MarginTextVerticalAlign,
) {
    if content.is_empty() || width <= 0.0 || height <= 0.0 {
        return;
    }
    if let Some(shaped) = crate::standalone_text::shape(
        document,
        content,
        font_size,
        font_family,
        Some(width),
        standalone_align(alignment),
    ) {
        let free_y = (height - shaped.height()).max(0.0);
        let offset_y = match vertical_align {
            MarginTextVerticalAlign::Top => 0.0,
            MarginTextVerticalAlign::Middle => free_y * 0.5,
            MarginTextVerticalAlign::Bottom => free_y,
        };
        crate::standalone_text::draw(scene, &shaped, x, y + offset_y, color);
        return;
    }
    let font_size = if font_size.is_finite() && font_size > 0.0 {
        font_size
    } else {
        16.0
    };
    let mut layout = with_parley_fonts(|fonts| {
        let mut layout_cx = LayoutContext::<()>::new();
        let mut builder = layout_cx.ranged_builder(fonts, content, 1.0, true);
        builder.push_default(StyleProperty::FontFamily(FontFamily::from(font_family)));
        builder.push_default(StyleProperty::FontSize(font_size));
        builder.push_default(StyleProperty::FontWeight(FontWeight::new(400.0)));
        builder.push_default(StyleProperty::FontStyle(ParleyFontStyle::Normal));
        builder.push_default(StyleProperty::LineHeight(LineHeight::MetricsRelative(1.0)));
        builder.build(content)
    });
    layout.break_all_lines(Some(width));
    layout.align(alignment, parley::AlignmentOptions::default());

    let free_y = (height - layout.height()).max(0.0);
    let offset_y = match vertical_align {
        MarginTextVerticalAlign::Top => 0.0,
        MarginTextVerticalAlign::Middle => free_y * 0.5,
        MarginTextVerticalAlign::Bottom => free_y,
    } as f64;
    let base_transform = Affine::translate((x as f64, y as f64 + offset_y));
    for line in layout.lines() {
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                continue;
            };
            let run = glyph_run.run();
            let font_size = run.font_size();
            let synthesis = run.synthesis();
            let glyph_transform = synthesis
                .skew()
                .map(|angle| Affine::skew(angle.to_radians().tan() as f64, 0.0));
            scene.draw_glyphs(
                run.font(),
                font_size,
                true,
                run.normalized_coords(),
                synthetic_embolden(synthesis.embolden(), font_size),
                Fill::NonZero,
                color,
                1.0,
                base_transform,
                glyph_transform,
                glyph_run.positioned_glyphs().map(to_anyrender_glyph),
            );
        }
    }
}

/// Measure one-line generated margin text using the same font defaults as
/// [`draw_margin_text`].  Replaced content such as an image can use the result
/// as its inline origin without leaking URL syntax into the painted text.
pub(crate) fn measure_margin_text(
    document: Option<&Document>,
    content: &str,
    font_size: f32,
    font_family: &str,
) -> f32 {
    if content.is_empty() {
        return 0.0;
    }
    if let Some(shaped) = crate::standalone_text::shape(
        document,
        content,
        font_size,
        font_family,
        None,
        StandaloneAlign::Start,
    ) {
        return shaped.width();
    }
    let font_size = if font_size.is_finite() && font_size > 0.0 {
        font_size
    } else {
        16.0
    };
    let mut layout = with_parley_fonts(|fonts| {
        let mut layout_cx = LayoutContext::<()>::new();
        let mut builder = layout_cx.ranged_builder(fonts, content, 1.0, true);
        builder.push_default(StyleProperty::FontFamily(FontFamily::from(font_family)));
        builder.push_default(StyleProperty::FontSize(font_size));
        builder.push_default(StyleProperty::FontWeight(FontWeight::new(400.0)));
        builder.push_default(StyleProperty::FontStyle(ParleyFontStyle::Normal));
        builder.push_default(StyleProperty::LineHeight(LineHeight::MetricsRelative(1.0)));
        builder.build(content)
    });
    layout.break_all_lines(None);
    layout.width().max(0.0)
}

/// Measure one-line generated text including trailing whitespace.
///
/// [`measure_margin_text`] intentionally returns the ink/content width used by
/// markers and margin boxes, where trailing whitespace must not move the box.
/// A generated `::before` run is different: its trailing spaces are part of
/// the inline advance consumed before `::after`, so use the line metrics'
/// advance (which retains [`parley::LineMetrics::trailing_whitespace`]).
pub(crate) fn measure_margin_text_advance(
    document: Option<&Document>,
    content: &str,
    font_size: f32,
    font_family: &str,
) -> f32 {
    if content.is_empty() {
        return 0.0;
    }
    if let Some(shaped) = crate::standalone_text::shape(
        document,
        content,
        font_size,
        font_family,
        None,
        StandaloneAlign::Start,
    ) {
        return shaped.advance();
    }
    let font_size = if font_size.is_finite() && font_size > 0.0 {
        font_size
    } else {
        16.0
    };
    let mut layout = with_parley_fonts(|fonts| {
        let mut layout_cx = LayoutContext::<()>::new();
        let mut builder = layout_cx.ranged_builder(fonts, content, 1.0, true);
        builder.push_default(StyleProperty::FontFamily(FontFamily::from(font_family)));
        builder.push_default(StyleProperty::FontSize(font_size));
        builder.push_default(StyleProperty::FontWeight(FontWeight::new(400.0)));
        builder.push_default(StyleProperty::FontStyle(ParleyFontStyle::Normal));
        builder.push_default(StyleProperty::LineHeight(LineHeight::MetricsRelative(1.0)));
        builder.build(content)
    });
    layout.break_all_lines(None);
    layout
        .lines()
        .next()
        .map(|line| line.metrics().advance.max(0.0))
        .unwrap_or_else(|| layout.width().max(0.0))
}

/// Measure the line box height of generated text using the same shaping path as
/// [`draw_margin_text`].  This is needed before an originating auto-height box
/// is painted: generated content contributes to that box's used height even
/// though the current arena has no synthetic child node for it.
pub(crate) fn measure_margin_text_height(
    document: Option<&Document>,
    content: &str,
    font_size: f32,
    font_family: &str,
) -> f32 {
    if content.is_empty() {
        return 0.0;
    }
    if let Some(shaped) = crate::standalone_text::shape(
        document,
        content,
        font_size,
        font_family,
        None,
        StandaloneAlign::Start,
    ) {
        return shaped.height();
    }
    let font_size = if font_size.is_finite() && font_size > 0.0 {
        font_size
    } else {
        16.0
    };
    let mut layout = with_parley_fonts(|fonts| {
        let mut layout_cx = LayoutContext::<()>::new();
        let mut builder = layout_cx.ranged_builder(fonts, content, 1.0, true);
        builder.push_default(StyleProperty::FontFamily(FontFamily::from(font_family)));
        builder.push_default(StyleProperty::FontSize(font_size));
        builder.push_default(StyleProperty::FontWeight(FontWeight::new(400.0)));
        builder.push_default(StyleProperty::FontStyle(ParleyFontStyle::Normal));
        builder.push_default(StyleProperty::LineHeight(LineHeight::MetricsRelative(1.0)));
        builder.build(content)
    });
    layout.break_all_lines(None);
    layout.height().max(0.0)
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct DecorationGeometry {
    pub(crate) x0: f64,
    pub(crate) x1: f64,
    pub(crate) abs_y: f64,
    pub(crate) line_top: f32,
    /// The line's baseline in page coordinates, when the layout engine
    /// supplies it; each decoration adds its element's `origin_shift_y`.
    /// `None` derives the baseline from the decorating element's metrics.
    pub(crate) baseline: Option<f64>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum DecorationPhase {
    BeforeGlyphs,
    AfterGlyphs,
}

#[derive(Clone, Copy, Debug)]
enum DecorationLineKind {
    Underline,
    Overline,
    LineThrough,
}

pub(crate) fn draw_decoration_phase(
    scene: &mut impl PaintScene,
    decorations: &[&DecorationSpec],
    geometry: DecorationGeometry,
    phase: DecorationPhase,
) {
    // CSS Text Decoration 3 orders all underline lines below all overlines,
    // then glyphs, then all line-through lines. The caller flattens the
    // persistent chain once for both phases.
    match phase {
        DecorationPhase::BeforeGlyphs => {
            paint_decoration_line(scene, decorations, geometry, DecorationLineKind::Underline);
            paint_decoration_line(scene, decorations, geometry, DecorationLineKind::Overline);
        }
        DecorationPhase::AfterGlyphs => {
            paint_decoration_line(
                scene,
                decorations,
                geometry,
                DecorationLineKind::LineThrough,
            );
        }
    }
}

fn decoration_span(x0: f64, x1: f64, decoration: &DecorationSpec) -> Option<(f64, f64)> {
    let (start, end) = if decoration.origin_rtl {
        (decoration.inset_end, decoration.inset_start)
    } else {
        (decoration.inset_start, decoration.inset_end)
    };
    let line_x0 = x0 + start;
    let line_x1 = x1 - end;
    if !start.is_finite()
        || !end.is_finite()
        || !line_x0.is_finite()
        || !line_x1.is_finite()
        || line_x1 <= line_x0
    {
        None
    } else {
        Some((line_x0, line_x1))
    }
}

/// Return the paint spans for one decoration segment.
///
/// A mixed-sign inset can be represented by two translated copies of the
/// originating segment. Keeping those copies separate preserves the endpoint
/// overlap produced by the CSS Text Decoration reference rendering. Equal
/// translations collapse to the ordinary single span.
fn decoration_spans(
    x0: f64,
    x1: f64,
    decoration: &DecorationSpec,
) -> Option<([(f64, f64); 2], usize)> {
    let span = decoration_span(x0, x1, decoration)?;
    if !decoration.origin_rtl
        && decoration.inset_start > 0.0
        && decoration.inset_end < 0.0
        && (decoration.inset_start + decoration.inset_end).abs() > f64::EPSILON
    {
        let first = (x0 + decoration.inset_start, x1 + decoration.inset_start);
        let second = (x0 - decoration.inset_end, x1 - decoration.inset_end);
        if first.0.is_finite()
            && first.1.is_finite()
            && second.0.is_finite()
            && second.1.is_finite()
            && first.1 > first.0
            && second.1 > second.0
        // cov:ignore: asymmetric endpoint overlap is covered by the ignored exact WPT reftest.
        {
            return Some(([first, second], 2));
        }
    }
    Some(([span, (0.0, 0.0)], 1))
}

fn paint_decoration_line(
    scene: &mut impl PaintScene,
    decorations: &[&DecorationSpec],
    geometry: DecorationGeometry,
    kind: DecorationLineKind,
) {
    let DecorationGeometry {
        x0,
        x1,
        abs_y,
        line_top,
        baseline: baseline_override,
    } = geometry;
    for decoration in decorations {
        let enabled = match kind {
            DecorationLineKind::Underline => decoration.line.underline,
            DecorationLineKind::Overline => decoration.line.overline,
            DecorationLineKind::LineThrough => decoration.line.line_through,
        };
        if !enabled {
            continue;
        }
        let Some((spans, span_count)) = decoration_spans(x0, x1, decoration) else {
            continue;
        };
        let color = css_color_to_peniko(decoration.color);
        let baseline = match baseline_override {
            // The layout engine gave the line's baseline; the decorating
            // element's baseline lies `origin_shift_y` below it.
            Some(line_baseline) => line_baseline + decoration.origin_shift_y as f64,
            None => {
                abs_y
                    + line_top as f64
                    + decoration.origin_ascent
                    + decoration.origin_shift_y as f64
            }
        };
        let thickness = decoration.origin_thickness;
        let center = match kind {
            DecorationLineKind::Underline => {
                baseline + decoration.origin_descent * 0.5 + decoration.underline_offset
            }
            DecorationLineKind::Overline => baseline - decoration.origin_ascent + thickness * 0.5,
            DecorationLineKind::LineThrough => baseline - decoration.origin_ascent * 0.35,
        };
        for &(span_x0, span_x1) in spans.iter().take(span_count) {
            paint_decoration_style(
                scene,
                decoration.style,
                color,
                span_x0,
                span_x1,
                center,
                thickness,
            );
        }
    }
}

const MAX_DECORATION_SEGMENTS: usize = 4096;

fn dashed_lengths(span: f64, thickness: f64) -> (f64, f64) {
    let natural_dash = (thickness * 3.0).max(2.0);
    let natural_gap = (thickness * 2.0).max(2.0);
    let natural_period = natural_dash + natural_gap;
    let scale = (span / (MAX_DECORATION_SEGMENTS as f64 * natural_period)).max(1.0);
    (natural_dash * scale, natural_gap * scale)
}

fn paint_decoration_style(
    scene: &mut impl PaintScene,
    style: TextDecorationStyle,
    color: Color,
    x0: f64,
    x1: f64,
    center: f64,
    thickness: f64,
) {
    if !x0.is_finite()
        || !x1.is_finite()
        || !center.is_finite()
        || !thickness.is_finite()
        || thickness <= 0.0
        || x1 <= x0
    {
        // cov:ignore: decoration_geometry filters production values; this is
        // a defensive sink guard for future/non-finite metrics.
        return;
    }
    match style {
        TextDecorationStyle::Solid => fill_decoration_rect(scene, color, x0, x1, center, thickness),
        TextDecorationStyle::Double => {
            // CSS Text Decoration 3 defines double as two lines with a gap.
            // Divide the total auto thickness into three equal bands.
            let band = (thickness / 3.0).max(0.5);
            let offset = band;
            fill_decoration_rect(scene, color, x0, x1, center - offset, band);
            fill_decoration_rect(scene, color, x0, x1, center + offset, band);
        }
        TextDecorationStyle::Dotted => {
            let radius = (thickness / 2.0).max(0.5);
            let span = x1 - x0;
            let natural_step = (radius * 4.0).max(2.0);
            // A hostile but finite letter-spacing/advance must not turn into
            // an unbounded number of scene commands.
            let step = natural_step.max(span / MAX_DECORATION_SEGMENTS as f64);
            let mut x = x0 + radius;
            let mut segments = 0;
            while x < x1 && segments < MAX_DECORATION_SEGMENTS {
                scene.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    color,
                    None,
                    &Circle::new(Point::new(x, center), radius),
                );
                x += step;
                segments += 1;
            }
        }
        TextDecorationStyle::Dashed => {
            let (dash, gap) = dashed_lengths(x1 - x0, thickness);
            let mut path = BezPath::new();
            path.move_to((x0, center));
            path.line_to((x1, center));
            let stroke = Stroke::new(thickness)
                .with_caps(Cap::Butt)
                .with_dashes(0.0, [dash, gap]);
            scene.stroke(&stroke, Affine::IDENTITY, color, None, &path);
        }
        TextDecorationStyle::Wavy => {
            let span = x1 - x0;
            let wavelength = (thickness * 4.0).max(4.0);
            let amplitude = (thickness * 1.5).max(0.75);
            // Limit path complexity while retaining more detail for normal
            // spans. The cap is important for huge finite advances.
            let half_wave = (wavelength * 0.5).max(span / MAX_DECORATION_SEGMENTS as f64);
            let mut path = BezPath::new();
            path.move_to((x0, center));
            let mut x = x0;
            let mut sign = -1.0;
            let mut segments = 0;
            while x < x1 && segments < MAX_DECORATION_SEGMENTS {
                let end = (x + half_wave).min(x1);
                let control_x = (x + end) * 0.5;
                path.quad_to((control_x, center + sign * amplitude), (end, center));
                x = end;
                sign = -sign;
                segments += 1;
            }
            let stroke = Stroke::new(thickness).with_caps(Cap::Round);
            scene.stroke(&stroke, Affine::IDENTITY, color, None, &path);
        }
        // `TextDecorationStyle` is non-exhaustive. Treat a future style as a
        // solid line until a dedicated paint algorithm is added.
        _ => fill_decoration_rect(scene, color, x0, x1, center, thickness), // cov:ignore: forward guard for a future style variant.
    }
}

fn fill_decoration_rect(
    scene: &mut impl PaintScene,
    color: Color,
    x0: f64,
    x1: f64,
    center: f64,
    thickness: f64,
) {
    let rect = Rect::new(x0, center - thickness * 0.5, x1, center + thickness * 0.5);
    scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &rect);
}

/// Convert parley::Glyph to anyrender::Glyph (copying across crate type boundaries).
fn to_anyrender_glyph(g: ParleyGlyph) -> AnyrenderGlyph {
    AnyrenderGlyph {
        id: g.id,
        x: g.x,
        y: g.y,
    }
}

/// Convert raikiri-style CssColor (r/g/b/a: u8) to peniko::Color.
///
/// Keep this helper separate for a future move to `peniko::AlphaColor`
/// (it is one line today but easier to find this way).
pub(crate) fn css_color_to_peniko(c: CssColor) -> Color {
    Color::from_rgba8(c.r, c.g, c.b, c.a)
}

#[cfg(test)]
mod tests {
    use super::{
        DecorationContext, DecorationGeometry, DecorationPhase, DecorationSpec,
        MAX_DECORATION_SEGMENTS, dashed_lengths, decoration_span, decoration_spans,
        decorations_for_element, draw_decoration_phase, measure_margin_text,
        measure_margin_text_advance, measure_margin_text_height, paint_decoration_style,
        synthetic_embolden,
    };
    use anyrender::{Scene, recording::RenderCommand};
    use kurbo::Vec2;
    use raikiri_style::ComputedValues;
    use raikiri_style::property::{
        CssColor, DisplayValue, FloatValue, PositionValue, TextDecorationLine, TextDecorationStyle,
    };

    #[test]
    fn decoration_span_applies_logical_insets_from_origin_direction() {
        let mut spec = DecorationSpec {
            line: TextDecorationLine::UNDERLINE,
            style: TextDecorationStyle::Solid,
            color: CssColor::BLACK,
            origin_thickness: 1.0,
            origin_ascent: 8.0,
            origin_descent: 2.0,
            origin_shift_y: 0.0,
            inset_start: 10.0,
            inset_end: -10.0,
            underline_offset: 0.0,
            origin_rtl: false,
        };
        assert_eq!(decoration_span(100.0, 200.0, &spec), Some((110.0, 210.0)));

        spec.origin_rtl = true;
        assert_eq!(decoration_span(100.0, 200.0, &spec), Some((90.0, 190.0)));
    }

    #[test]
    fn parley_decorations_are_unchanged_by_the_geometry_field() {
        // A geometry without an explicit baseline derives it from the
        // decorating element, exactly as before the field existed. An
        // explicit baseline is the line's; the decorating element's shift is
        // added to it, so the line's baseline without the shift draws the
        // same line.
        let spec = DecorationSpec {
            line: TextDecorationLine::UNDERLINE,
            style: TextDecorationStyle::Solid,
            color: CssColor::BLACK,
            origin_thickness: 1.0,
            origin_ascent: 8.0,
            origin_descent: 2.0,
            origin_shift_y: 1.5,
            inset_start: 0.0,
            inset_end: 0.0,
            underline_offset: 0.0,
            origin_rtl: false,
        };
        let (abs_y, line_top) = (100.0_f64, 2.0_f32);
        let derived = abs_y + f64::from(line_top) + spec.origin_ascent;
        let draw = |baseline: Option<f64>| {
            let mut scene = anyrender::recording::Scene::new();
            let geometry = DecorationGeometry {
                x0: 0.0,
                x1: 10.0,
                abs_y,
                line_top,
                baseline,
            };
            draw_decoration_phase(
                &mut scene,
                &[&spec],
                geometry,
                DecorationPhase::BeforeGlyphs,
            );
            scene
        };
        let (implicit, explicit) = (draw(None), draw(Some(derived)));
        assert!(!implicit.commands.is_empty());
        assert_eq!(
            format!("{:?}", implicit.commands),
            format!("{:?}", explicit.commands)
        );
        // A different explicit baseline moves the line.
        let moved = draw(Some(derived + 3.0));
        assert_ne!(
            format!("{:?}", implicit.commands),
            format!("{:?}", moved.commands)
        );
    }

    #[test]
    fn decoration_span_rejects_empty_or_nonfinite_ranges() {
        let spec = DecorationSpec {
            line: TextDecorationLine::UNDERLINE,
            style: TextDecorationStyle::Solid,
            color: CssColor::BLACK,
            origin_thickness: 1.0,
            origin_ascent: 8.0,
            origin_descent: 2.0,
            origin_shift_y: 0.0,
            inset_start: 60.0,
            inset_end: 50.0,
            underline_offset: 0.0,
            origin_rtl: false,
        };
        assert_eq!(decoration_span(100.0, 200.0, &spec), None);
        let mut nonfinite = spec;
        nonfinite.inset_start = f64::NAN;
        assert_eq!(decoration_span(100.0, 200.0, &nonfinite), None);
    }

    #[test]
    fn decoration_spans_preserve_asymmetric_mixed_sign_endpoint_overlap() {
        let spec = DecorationSpec {
            line: TextDecorationLine::UNDERLINE,
            style: TextDecorationStyle::Solid,
            color: CssColor::BLACK,
            origin_thickness: 1.0,
            origin_ascent: 8.0,
            origin_descent: 2.0,
            origin_shift_y: 0.0,
            inset_start: 10.0,
            inset_end: -12.0,
            underline_offset: 0.0,
            origin_rtl: false,
        };
        assert_eq!(
            decoration_spans(100.0, 140.0, &spec),
            Some(([(110.0, 150.0), (112.0, 152.0)], 2))
        );
    }

    #[test]
    fn decoration_spans_collapse_equal_endpoint_translation() {
        let spec = DecorationSpec {
            line: TextDecorationLine::UNDERLINE,
            style: TextDecorationStyle::Solid,
            color: CssColor::BLACK,
            origin_thickness: 1.0,
            origin_ascent: 8.0,
            origin_descent: 2.0,
            origin_shift_y: 0.0,
            inset_start: 10.0,
            inset_end: -10.0,
            underline_offset: 0.0,
            origin_rtl: false,
        };
        assert_eq!(
            decoration_spans(100.0, 140.0, &spec),
            Some(([(110.0, 150.0), (0.0, 0.0)], 1))
        );
    }

    #[test]
    fn generated_text_measurements_handle_empty_and_nonfinite_inputs() {
        assert_eq!(
            measure_margin_text_advance(None, "", f32::NAN, "serif"),
            0.0
        );
        assert!(measure_margin_text_advance(None, "A", f32::NAN, "serif") > 0.0);
        assert_eq!(measure_margin_text_height(None, "", f32::NAN, "serif"), 0.0);
        assert!(measure_margin_text_height(None, "A", f32::NAN, "serif") > 0.0);
    }

    fn ancestor_context() -> DecorationContext {
        let mut cv = ComputedValues::initial();
        cv.text_decoration_line = TextDecorationLine::UNDERLINE;
        decorations_for_element(&DecorationContext::default(), &cv, 0.0)
    }

    #[test]
    fn dotted_decoration_caps_scene_commands_for_huge_spans() {
        let mut scene = Scene::new();
        paint_decoration_style(
            &mut scene,
            TextDecorationStyle::Dotted,
            peniko::Color::BLACK,
            0.0,
            1_000_000_000_000.0,
            0.0,
            1.0,
        );
        let fills = scene
            .commands
            .iter()
            .filter(|command| matches!(command, RenderCommand::Fill(_)))
            .count();
        assert!(fills > 0);
        assert!(fills <= MAX_DECORATION_SEGMENTS);
    }

    #[test]
    fn dashed_decoration_scales_period_for_huge_spans() {
        let (dash, gap) = dashed_lengths(1_000_000_000_000.0, 1.0);
        assert!(dash > 3.0);
        assert!(gap > 2.0);
        assert!((dash + gap) * MAX_DECORATION_SEGMENTS as f64 >= 1_000_000_000_000.0);
    }

    #[test]
    fn display_contents_does_not_originate_or_block_decoration() {
        let mut ancestor = ComputedValues::initial();
        ancestor.text_decoration_line = TextDecorationLine::UNDERLINE;
        let context = decorations_for_element(&DecorationContext::default(), &ancestor, 0.0);

        let mut contents = ComputedValues::initial();
        contents.display = DisplayValue::Contents;
        contents.text_decoration_line = TextDecorationLine::OVERLINE;
        let propagated = decorations_for_element(&context, &contents, 0.0);
        let specs: Vec<_> = propagated.iter().collect();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].line, TextDecorationLine::UNDERLINE);
    }

    #[test]
    fn decoration_retains_origin_vertical_align_shift() {
        let mut cv = ComputedValues::initial();
        cv.text_decoration_line = TextDecorationLine::UNDERLINE;
        let decorations = decorations_for_element(&DecorationContext::default(), &cv, 7.5);
        assert_eq!(
            decorations
                .iter()
                .next()
                .expect("origin decoration")
                .origin_shift_y,
            7.5
        );
    }

    #[test]
    fn decoration_metrics_are_taken_from_the_originating_element() {
        let mut cv = ComputedValues::initial();
        cv.font_size = raikiri_style::resolve::ComputedLength(32.0);
        cv.text_decoration_line = TextDecorationLine::UNDERLINE;
        let decorations = decorations_for_element(&DecorationContext::default(), &cv, 0.0);
        let decoration = decorations.iter().next().expect("origin decoration");
        assert_eq!(decoration.origin_thickness, 2.0);
        assert_eq!(decoration.origin_ascent, 25.6);
        assert_eq!(decoration.origin_descent, 6.4);
    }

    #[test]
    fn underline_offset_percentage_resolves_against_decorating_font_size() {
        let mut cv = ComputedValues::initial();
        cv.font_size = raikiri_style::resolve::ComputedLength(40.0);
        cv.text_decoration_line = TextDecorationLine::UNDERLINE;
        cv.text_underline_offset = raikiri_style::ComputedTextUnderlineOffset::Percent(50.0);
        let decorations = decorations_for_element(&DecorationContext::default(), &cv, 0.0);
        let decoration = decorations.iter().next().expect("origin decoration");
        assert_eq!(decoration.underline_offset, 20.0);
    }

    #[test]
    fn underline_offset_calc_resolves_percentage_against_decorating_font_size() {
        let mut cv = ComputedValues::initial();
        cv.font_size = raikiri_style::resolve::ComputedLength(40.0);
        cv.text_decoration_line = TextDecorationLine::UNDERLINE;
        cv.text_underline_offset = raikiri_style::ComputedTextUnderlineOffset::Calc(
            raikiri_style::property::CalcLengthPercentage {
                percent: 50.0,
                px: 8.0,
            },
        );
        let decorations = decorations_for_element(&DecorationContext::default(), &cv, 0.0);
        let decoration = decorations.iter().next().expect("origin decoration");
        assert_eq!(decoration.underline_offset, 28.0);
    }

    #[test]
    fn ancestor_decoration_stops_at_out_of_flow_float_and_atomic_boundaries() {
        let ancestor = ancestor_context();
        let cases = [
            (
                "absolute",
                DisplayValue::Block,
                PositionValue::Absolute,
                FloatValue::None,
            ),
            (
                "fixed",
                DisplayValue::Block,
                PositionValue::Fixed,
                FloatValue::None,
            ),
            (
                "float",
                DisplayValue::Block,
                PositionValue::Static,
                FloatValue::Left,
            ),
            (
                "inline-block",
                DisplayValue::InlineBlock,
                PositionValue::Static,
                FloatValue::None,
            ),
            (
                "inline-flex",
                DisplayValue::InlineFlex,
                PositionValue::Static,
                FloatValue::None,
            ),
            (
                "inline-grid",
                DisplayValue::InlineGrid,
                PositionValue::Static,
                FloatValue::None,
            ),
            (
                "inline-table",
                DisplayValue::InlineTable,
                PositionValue::Static,
                FloatValue::None,
            ),
        ];
        for (name, display, position, float) in cases {
            let mut cv = ComputedValues::initial();
            cv.display = display;
            cv.position = position;
            cv.float = float;
            // cov:ignore: the assertion message is evaluated only when this
            // boundary regression assertion fails.
            assert!(
                decorations_for_element(&ancestor, &cv, 0.0)
                    .iter()
                    .next()
                    .is_none(),
                "ancestor decoration crossed {name} boundary"
            );
        }

        let mut in_flow = ComputedValues::initial();
        in_flow.display = DisplayValue::Block;
        assert_eq!(
            decorations_for_element(&ancestor, &in_flow, 0.0)
                .iter()
                .count(),
            1
        );

        let mut boundary_origin = ComputedValues::initial();
        boundary_origin.display = DisplayValue::InlineBlock;
        boundary_origin.text_decoration_line = TextDecorationLine::OVERLINE;
        let boundary_decorations = decorations_for_element(&ancestor, &boundary_origin, 0.0);
        assert_eq!(boundary_decorations.iter().count(), 1);
        assert_eq!(
            boundary_decorations
                .iter()
                .next()
                .expect("own decoration")
                .line,
            TextDecorationLine::OVERLINE
        );
    }
    #[test]
    fn synthetic_embolden_uses_zero_without_synthesis() {
        assert_eq!(synthetic_embolden(false, 32.0), Vec2::ZERO);
    }

    #[test]
    fn synthetic_embolden_scales_and_caps_stroke_offset() {
        assert_eq!(synthetic_embolden(true, 10.0), Vec2::new(0.15125, 0.121));
        assert_eq!(synthetic_embolden(true, 100.0), Vec2::new(0.3, 0.3));
    }

    #[test]
    fn the_parley_context_is_created_once_per_thread() {
        // The counter is per thread (see `parley_context_creations`), so other
        // tests running in parallel on other threads cannot disturb it.
        let before = super::parley_context_creations();
        for _ in 0..5 {
            let _ = measure_margin_text(None, "abc", 16.0, "serif");
            let _ = measure_margin_text_advance(None, "abc", 16.0, "serif");
            let _ = measure_margin_text_height(None, "abc", 16.0, "serif");
        }
        // At most one creation (this thread may already have created it).
        assert!(super::parley_context_creations() - before <= 1);
    }

    #[test]
    fn a_reused_context_gives_the_same_measurements() {
        let first = measure_margin_text(None, "abc def", 16.0, "serif");
        let second = measure_margin_text(None, "abc def", 16.0, "serif");
        assert_eq!(first, second);
        assert_eq!(
            measure_margin_text_advance(None, "abc ", 16.0, "serif"),
            measure_margin_text_advance(None, "abc ", 16.0, "serif")
        );
    }
}
