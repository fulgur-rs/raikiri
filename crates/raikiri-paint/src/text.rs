//! Text glyph and decoration draw — parley Layout の GlyphRun を
//! anyrender::draw_glyphs に pipeし、CSS Text Decoration Level 3 の
//! line/style/color を CSS の描画順に描画する。
//!
//! Pre-shape 済 `parley::Layout<()>` を `Node::text_layout()` accessor
//! (`NodeData::Text(TextData)` 経由) から取得する design に依拠。paint は
//! line iteration + GlyphRun.positioned_glyphs()
//! を per-run 変換 (parley::Glyph → anyrender::Glyph) して scene に送る。
//!
//! 座標系: parley Layout origin (0,0) 左上、positioned_glyphs() が line 内
//! offset + baseline を stored directly。anyrender transform は text node の絶対座標
//! への平行移動のみで済む (baseline / offset 加算不要)。

use std::sync::Arc;

use anyrender::{Glyph as AnyrenderGlyph, PaintScene};
use kurbo::{Affine, BezPath, Cap, Circle, Point, Rect, Stroke, Vec2};
use parley::{
    Alignment, FontContext, FontFamily, FontStyle as ParleyFontStyle, FontWeight,
    Glyph as ParleyGlyph, LayoutContext, LineHeight, PositionedLayoutItem, StyleProperty,
};
use peniko::{Color, Fill};
use raikiri_dom::Node;
use raikiri_style::property::{
    CssColor, Direction, DisplayValue, FloatValue, PositionValue, TextAlign, TextAlignLast,
    TextDecorationColor, TextDecorationLine, TextDecorationStyle,
};
use raikiri_style::{CascadeResult, ComputedValues};

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
    fn is_empty(&self) -> bool {
        self.0.is_none()
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

#[derive(Clone, Copy, Debug)]
pub(crate) struct TextPosition {
    pub(crate) abs_x: f32,
    pub(crate) abs_y: f32,
    pub(crate) shift_y: f32,
}

pub(crate) fn draw_text_node(
    scene: &mut impl PaintScene,
    node: &Node,
    cascade: &CascadeResult,
    node_id: usize,
    position: TextPosition,
    decorations: &DecorationContext,
) {
    let TextPosition {
        abs_x,
        abs_y,
        shift_y,
    } = position;
    // Text node の pre-shape 結果を取得。preshape_text が empty text で None を返す
    // ので、None = "empty text" の signal、silent return。
    let Some(text_layout) = node.text_layout() else {
        return;
    };

    // Text node の brush = cascade で親から inherit された color (現状 color property のみ対応)。
    // ComputedValues.color は Text node 位置にも populate 済 (inheritance walk 経由)。
    let cv = &cascade.computed[node_id];
    let brush = css_color_to_peniko(cv.color);

    // parley positioned_glyphs() は line 内 offset + baseline を stored directly するので
    // scene transform は text node の絶対座標への平行移動のみ。
    let base_transform = Affine::translate((abs_x as f64, (abs_y + shift_y) as f64));
    // Parley applies one alignment to every line. CSS `text-align-last` can
    // override the final line, so compute the physical delta here while
    // keeping the shaped layout (and its line breaks) unchanged.
    let last_line_index = text_layout.len().saturating_sub(1);

    for (line_index, line) in text_layout.lines().enumerate() {
        let metrics = line.metrics();
        let last_line_delta = if line_index == last_line_index {
            text_align_last_delta(metrics, cv.text_align, cv.text_align_last, cv.direction)
        } else {
            0.0
        };
        let rtl = text_layout.is_rtl();
        let leading_whitespace = leading_whitespace_advance(line, rtl);
        let geometry = decoration_geometry(
            decorations,
            metrics,
            abs_x,
            abs_y,
            last_line_delta,
            leading_whitespace,
            rtl,
        );

        // CSS paints underline/overline before the glyphs and line-through
        // after them. Flatten the persistent context once for both phases so
        // nested origins keep their global line order without two allocations.
        let decoration_specs: Vec<_> = if geometry.is_some() {
            decorations.iter().collect()
        } else {
            Vec::new()
        };
        if let Some(geometry) = geometry {
            draw_decoration_phase(
                scene,
                &decoration_specs,
                geometry,
                DecorationPhase::BeforeGlyphs,
            );
        }

        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                // InlineBox は現状生成されない (preshape_text は inline box を
                // push しない)。将来 inline formatting context を実装する際に、
                // ここで image / replaced element 描画が入る予定。defensive: continue。
                continue;
            };

            let run = glyph_run.run();
            let font = run.font(); // &peniko::FontData (parley re-export)
            let font_size = run.font_size();
            let coords = run.normalized_coords(); // &[i16] (anyrender::NormalizedCoord alias)

            scene.draw_glyphs(
                font,
                font_size,
                true, // hint = true (blitz と揃えた値。将来的に判定を切り替える余地あり)
                coords,
                Vec2::ZERO, // embolden 無し (font-embolden feature は未実装)
                Fill::NonZero,
                brush, // peniko::Color → PaintRef auto-convert
                1.0,   // brush_alpha (color 自身が alpha 持つ)
                base_transform,
                None, // glyph_transform (rotate / skew は未実装)
                glyph_run.positioned_glyphs().map(|mut glyph| {
                    glyph.x += last_line_delta;
                    to_anyrender_glyph(glyph)
                }),
            );
        }

        if let Some(geometry) = geometry {
            draw_decoration_phase(
                scene,
                &decoration_specs,
                geometry,
                DecorationPhase::AfterGlyphs,
            );
        }
    }
}

/// Vertical placement of generated margin-box text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MarginTextVerticalAlign {
    Top,
    Middle,
    Bottom,
}

/// Draw generated text in a page-margin box.
///
/// Margin-box content is not a DOM text node, so it has no pre-shaped layout
/// to borrow from the document arena.  This small sink-local shaper uses the
/// same parley/anyrender path as ordinary text and deliberately accepts only
/// the style data needed by the page-context caller.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_margin_text(
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
    let font_size = if font_size.is_finite() && font_size > 0.0 {
        font_size
    } else {
        16.0
    };
    let mut fonts = FontContext::new();
    let mut layout_cx = LayoutContext::<()>::new();
    let mut builder = layout_cx.ranged_builder(&mut fonts, content, 1.0, true);
    builder.push_default(StyleProperty::FontFamily(FontFamily::from(font_family)));
    builder.push_default(StyleProperty::FontSize(font_size));
    builder.push_default(StyleProperty::FontWeight(FontWeight::new(400.0)));
    builder.push_default(StyleProperty::FontStyle(ParleyFontStyle::Normal));
    builder.push_default(StyleProperty::LineHeight(LineHeight::MetricsRelative(1.0)));
    let mut layout = builder.build(content);
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
            scene.draw_glyphs(
                run.font(),
                run.font_size(),
                true,
                run.normalized_coords(),
                Vec2::ZERO,
                Fill::NonZero,
                color,
                1.0,
                base_transform,
                None,
                glyph_run.positioned_glyphs().map(to_anyrender_glyph),
            );
        }
    }
}

/// Measure one-line generated margin text using the same font defaults as
/// [`draw_margin_text`].  Replaced content such as an image can use the result
/// as its inline origin without leaking URL syntax into the painted text.
pub(crate) fn measure_margin_text(content: &str, font_size: f32, font_family: &str) -> f32 {
    if content.is_empty() {
        return 0.0;
    }
    let font_size = if font_size.is_finite() && font_size > 0.0 {
        font_size
    } else {
        16.0
    };
    let mut fonts = FontContext::new();
    let mut layout_cx = LayoutContext::<()>::new();
    let mut builder = layout_cx.ranged_builder(&mut fonts, content, 1.0, true);
    builder.push_default(StyleProperty::FontFamily(FontFamily::from(font_family)));
    builder.push_default(StyleProperty::FontSize(font_size));
    builder.push_default(StyleProperty::FontWeight(FontWeight::new(400.0)));
    builder.push_default(StyleProperty::FontStyle(ParleyFontStyle::Normal));
    builder.push_default(StyleProperty::LineHeight(LineHeight::MetricsRelative(1.0)));
    let mut layout = builder.build(content);
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
pub(crate) fn measure_margin_text_advance(content: &str, font_size: f32, font_family: &str) -> f32 {
    if content.is_empty() {
        return 0.0;
    }
    let font_size = if font_size.is_finite() && font_size > 0.0 {
        font_size
    } else {
        16.0
    };
    let mut fonts = FontContext::new();
    let mut layout_cx = LayoutContext::<()>::new();
    let mut builder = layout_cx.ranged_builder(&mut fonts, content, 1.0, true);
    builder.push_default(StyleProperty::FontFamily(FontFamily::from(font_family)));
    builder.push_default(StyleProperty::FontSize(font_size));
    builder.push_default(StyleProperty::FontWeight(FontWeight::new(400.0)));
    builder.push_default(StyleProperty::FontStyle(ParleyFontStyle::Normal));
    builder.push_default(StyleProperty::LineHeight(LineHeight::MetricsRelative(1.0)));
    let mut layout = builder.build(content);
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
pub(crate) fn measure_margin_text_height(content: &str, font_size: f32, font_family: &str) -> f32 {
    if content.is_empty() {
        return 0.0;
    }
    let font_size = if font_size.is_finite() && font_size > 0.0 {
        font_size
    } else {
        16.0
    };
    let mut fonts = FontContext::new();
    let mut layout_cx = LayoutContext::<()>::new();
    let mut builder = layout_cx.ranged_builder(&mut fonts, content, 1.0, true);
    builder.push_default(StyleProperty::FontFamily(FontFamily::from(font_family)));
    builder.push_default(StyleProperty::FontSize(font_size));
    builder.push_default(StyleProperty::FontWeight(FontWeight::new(400.0)));
    builder.push_default(StyleProperty::FontStyle(ParleyFontStyle::Normal));
    builder.push_default(StyleProperty::LineHeight(LineHeight::MetricsRelative(1.0)));
    let mut layout = builder.build(content);
    layout.break_all_lines(None);
    layout.height().max(0.0)
}

fn leading_whitespace_advance(line: parley::Line<'_, ()>, rtl: bool) -> f32 {
    // Parley exposes trailing whitespace in LineMetrics but not leading
    // whitespace. Cluster source characters let the paint layer recover the
    // logical line edge without changing shaping or layout.
    let mut clusters = Vec::new();
    for run in line.runs() {
        for cluster in run.clusters() {
            clusters.push((cluster.source_char(), cluster.advance()));
        }
    }
    let mut leading = 0.0;
    if rtl {
        for (character, advance) in clusters.iter().rev() {
            if !character.is_whitespace() {
                break;
            }
            leading += *advance;
        }
    } else {
        for (character, advance) in &clusters {
            if !character.is_whitespace() {
                break;
            }
            leading += *advance;
        }
    }
    leading.max(0.0)
}

fn decoration_line_width(metrics: &parley::LineMetrics, leading_whitespace: f32) -> f32 {
    (metrics.advance - metrics.trailing_whitespace - leading_whitespace).max(0.0)
}

#[derive(Clone, Copy, Debug)]
struct DecorationGeometry {
    x0: f64,
    x1: f64,
    abs_y: f64,
    line_top: f32,
}

fn decoration_geometry(
    decorations: &DecorationContext,
    metrics: &parley::LineMetrics,
    abs_x: f32,
    abs_y: f32,
    last_line_delta: f32,
    leading_whitespace: f32,
    rtl: bool,
) -> Option<DecorationGeometry> {
    if decorations.is_empty() {
        return None;
    }
    let line_width = decoration_line_width(metrics, leading_whitespace);
    if !line_width.is_finite() || line_width <= 0.0 {
        // cov:ignore: shaped text lines in the current layout always have a
        // finite positive advance; retain the guard for empty/future metrics.
        return None;
    }
    let line_start = metrics.inline_min_coord + metrics.offset + last_line_delta;
    let leading = leading_whitespace.max(0.0);
    let trailing = metrics.trailing_whitespace.max(0.0);
    let (left_trim, right_trim) = if rtl {
        (trailing, leading)
    } else {
        (leading, trailing)
    };
    let x0 = abs_x as f64 + (line_start + left_trim) as f64;
    let x1 = abs_x as f64 + (line_start + metrics.advance - right_trim) as f64;
    if !x0.is_finite() || !x1.is_finite() || !(abs_y as f64).is_finite() || x1 <= x0 {
        // cov:ignore: finite layout sanitization handles production values;
        // this keeps malformed/future metrics from reaching a draw loop.
        return None;
    }
    Some(DecorationGeometry {
        x0,
        x1,
        abs_y: abs_y as f64,
        // Keep the line's block origin from the current layout, but derive
        // the baseline within it from the originating decoration metrics.
        line_top: metrics.block_min_coord,
    })
}

#[derive(Clone, Copy, Debug)]
enum DecorationPhase {
    BeforeGlyphs,
    AfterGlyphs,
}

#[derive(Clone, Copy, Debug)]
enum DecorationLineKind {
    Underline,
    Overline,
    LineThrough,
}

fn draw_decoration_phase(
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
        let color = css_color_to_peniko(decoration.color);
        let baseline =
            abs_y + line_top as f64 + decoration.origin_ascent + decoration.origin_shift_y as f64;
        let thickness = decoration.origin_thickness;
        let center = match kind {
            DecorationLineKind::Underline => baseline + decoration.origin_descent * 0.5,
            DecorationLineKind::Overline => baseline - decoration.origin_ascent + thickness * 0.5,
            DecorationLineKind::LineThrough => baseline - decoration.origin_ascent * 0.35,
        };
        paint_decoration_style(scene, decoration.style, color, x0, x1, center, thickness);
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

/// Return the horizontal adjustment required by `text-align-last`.
///
/// Parley exposes line metrics but not a per-line alignment mutator. The
/// glyph positions are therefore shifted at paint time. Justification is
/// intentionally left to Parley: its public API cannot distribute spaces on
/// only the final line without rebuilding the layout.
fn text_align_last_delta(
    metrics: &parley::LineMetrics,
    text_align: TextAlign,
    text_align_last: TextAlignLast,
    direction: Direction,
) -> f32 {
    let requested = match text_align_last {
        TextAlignLast::Auto => match text_align {
            TextAlign::Justify | TextAlign::JustifyAll => return 0.0,
            value => value,
        },
        TextAlignLast::Start => TextAlign::Start,
        TextAlignLast::End => TextAlign::End,
        TextAlignLast::Left => TextAlign::Left,
        TextAlignLast::Right => TextAlign::Right,
        TextAlignLast::Center => TextAlign::Center,
        TextAlignLast::Justify => return 0.0,
        TextAlignLast::MatchParent => TextAlign::Start,
        _ => TextAlign::Start,
    };
    let desired = match requested {
        TextAlign::Start => match direction {
            Direction::Rtl => Alignment::Right,
            _ => Alignment::Left,
        },
        TextAlign::End => match direction {
            Direction::Rtl => Alignment::Left,
            _ => Alignment::Right,
        },
        TextAlign::Left => Alignment::Left,
        TextAlign::Right => Alignment::Right,
        TextAlign::Center => Alignment::Center,
        _ => Alignment::Left,
    };
    let free_space = (metrics.inline_max_coord - metrics.inline_min_coord - metrics.advance
        + metrics.trailing_whitespace)
        .max(0.0);
    let desired_offset = match desired {
        Alignment::Right => free_space,
        Alignment::Center => free_space * 0.5,
        _ => 0.0,
    };
    desired_offset - metrics.offset
}

/// parley::Glyph → anyrender::Glyph の変換 (別 crate の型境界を跨ぐための copy)。
fn to_anyrender_glyph(g: ParleyGlyph) -> AnyrenderGlyph {
    AnyrenderGlyph {
        id: g.id,
        x: g.x,
        y: g.y,
    }
}

/// raikiri-style の CssColor (r/g/b/a: u8) → peniko::Color。
///
/// 将来 `peniko::AlphaColor` へ昇格する move に備えて helper を切り出しておく
/// (今は 1 line だが grep しやすい)。
fn css_color_to_peniko(c: CssColor) -> Color {
    Color::from_rgba8(c.r, c.g, c.b, c.a)
}

#[cfg(test)]
mod tests {
    use super::{
        DecorationContext, MAX_DECORATION_SEGMENTS, dashed_lengths, decoration_line_width,
        decorations_for_element, measure_margin_text_advance, measure_margin_text_height,
        paint_decoration_style, text_align_last_delta,
    };
    use anyrender::{Scene, recording::RenderCommand};
    use parley::LineMetrics;
    use raikiri_style::ComputedValues;
    use raikiri_style::property::{
        Direction, DisplayValue, FloatValue, PositionValue, TextAlign, TextAlignLast,
        TextDecorationLine, TextDecorationStyle,
    };

    fn metrics(offset: f32) -> LineMetrics {
        LineMetrics {
            offset,
            advance: 40.0,
            inline_max_coord: 100.0,
            ..LineMetrics::default()
        }
    }

    #[test]
    fn generated_text_measurements_handle_empty_and_nonfinite_inputs() {
        assert_eq!(measure_margin_text_advance("", f32::NAN, "serif"), 0.0);
        assert!(measure_margin_text_advance("A", f32::NAN, "serif") > 0.0);
        assert_eq!(measure_margin_text_height("", f32::NAN, "serif"), 0.0);
        assert!(measure_margin_text_height("A", f32::NAN, "serif") > 0.0);
    }

    #[test]
    fn final_line_start_reverses_center_alignment() {
        let delta = text_align_last_delta(
            &metrics(30.0),
            TextAlign::Center,
            TextAlignLast::Start,
            Direction::Ltr,
        );
        assert_eq!(delta, -30.0);
    }

    #[test]
    fn final_line_end_and_center_use_remaining_space() {
        assert_eq!(
            text_align_last_delta(
                &metrics(0.0),
                TextAlign::Start,
                TextAlignLast::End,
                Direction::Ltr,
            ),
            60.0
        );
        assert_eq!(
            text_align_last_delta(
                &metrics(60.0),
                TextAlign::End,
                TextAlignLast::Center,
                Direction::Ltr,
            ),
            -30.0
        );
    }

    #[test]
    fn final_line_logical_edges_flip_in_rtl() {
        assert_eq!(
            text_align_last_delta(
                &metrics(60.0),
                TextAlign::Start,
                TextAlignLast::End,
                Direction::Rtl,
            ),
            -60.0
        );
        assert_eq!(
            text_align_last_delta(
                &metrics(0.0),
                TextAlign::End,
                TextAlignLast::Start,
                Direction::Rtl,
            ),
            60.0
        );
    }

    #[test]
    fn physical_edges_and_match_parent_are_supported() {
        assert_eq!(
            text_align_last_delta(
                &metrics(60.0),
                TextAlign::Start,
                TextAlignLast::Left,
                Direction::Ltr,
            ),
            -60.0
        );
        assert_eq!(
            text_align_last_delta(
                &metrics(0.0),
                TextAlign::Start,
                TextAlignLast::Right,
                Direction::Ltr,
            ),
            60.0
        );
        assert_eq!(
            text_align_last_delta(
                &metrics(60.0),
                TextAlign::Start,
                TextAlignLast::MatchParent,
                Direction::Ltr,
            ),
            -60.0
        );
    }

    #[test]
    fn auto_justify_and_explicit_justify_keep_parley_last_line() {
        assert_eq!(
            text_align_last_delta(
                &metrics(0.0),
                TextAlign::Justify,
                TextAlignLast::Auto,
                Direction::Ltr,
            ),
            0.0
        );
        assert_eq!(
            text_align_last_delta(
                &metrics(0.0),
                TextAlign::Start,
                TextAlignLast::Justify,
                Direction::Ltr,
            ),
            0.0
        );
    }

    fn ancestor_context() -> DecorationContext {
        let mut cv = ComputedValues::initial();
        cv.text_decoration_line = TextDecorationLine::UNDERLINE;
        decorations_for_element(&DecorationContext::default(), &cv, 0.0)
    }

    #[test]
    fn decoration_width_skips_line_edge_whitespace_at_every_white_space_mode() {
        let metrics = LineMetrics {
            advance: 40.0,
            trailing_whitespace: 10.0,
            ..LineMetrics::default()
        };
        // Level 3 skips spacing at both line edges. The white-space property
        // controls shaping/collapsing, not this decoration edge rule.
        assert_eq!(decoration_line_width(&metrics, 0.0), 30.0);
        assert_eq!(decoration_line_width(&metrics, 5.0), 25.0);
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
}
