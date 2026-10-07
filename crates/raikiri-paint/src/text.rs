//! Text decorations (CSS Text Decoration Level 3 line, style and color, in
//! CSS painting order) and the text of page-margin boxes and generated
//! content, which lies outside the paragraphs of the document.

use anyrender::PaintScene;
use kurbo::{Affine, BezPath, Cap, Circle, Point, Rect, Stroke, Vec2};
use peniko::{Color, Fill};
use raikiri_dom::{Document, StandaloneAlign};
use raikiri_style::property::{CssColor, TextDecorationStyle};

pub(crate) use raikiri_dom::text_decoration::{DecorationContext, decorations_for_element};
#[cfg(test)]
use raikiri_dom::text_decoration::{
    DecorationGeometry, DecorationSpec, decoration_span, decoration_spans, resolve_decoration_lines,
};
use raikiri_dom::{DecorationKind, DecorationLine};

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

/// Draw generated text in a page-margin box.
///
/// Margin-box content is not a DOM text node, so it is shaped here as a run
/// of its own with the document's fonts, from only the style data the
/// page-context caller has.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_margin_text(
    document: &Document,
    scene: &mut impl PaintScene,
    content: &str,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    color: Color,
    font_size: f32,
    font_family: &str,
    alignment: StandaloneAlign,
    vertical_align: MarginTextVerticalAlign,
) {
    let style = crate::standalone_text::style(font_size, font_family);
    draw_margin_text_styled(
        document,
        scene,
        content,
        x,
        y,
        width,
        height,
        color,
        &style,
        alignment,
        vertical_align,
    );
}

/// Draw margin text using its resolved flow, inside a physical content box.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_margin_text_styled(
    document: &Document,
    scene: &mut impl PaintScene,
    content: &str,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    color: Color,
    style: &raikiri_dom::StandaloneStyle,
    alignment: StandaloneAlign,
    vertical_align: MarginTextVerticalAlign,
) {
    if content.is_empty() || width <= 0.0 || height <= 0.0 {
        return;
    }
    let vertical = style.writing_mode.is_vertical();
    let inline_size = if vertical { height } else { width };
    let Some(shaped) = document.shape_standalone_text(content, style, Some(inline_size), alignment)
    else {
        return;
    };
    // The cross-axis alignment follows block progression. Inline alignment
    // is already included in the engine's glyph positions.
    if vertical {
        let free = (width - shaped.width()).max(0.0);
        let offset = match vertical_align {
            MarginTextVerticalAlign::Top => 0.0,
            MarginTextVerticalAlign::Middle => free * 0.5,
            MarginTextVerticalAlign::Bottom => free,
        };
        let from_right = matches!(
            style.writing_mode,
            shodo::geometry::WritingMode::VerticalRl | shodo::geometry::WritingMode::SidewaysRl
        );
        let offset_x = if from_right {
            width - shaped.width() - offset
        } else {
            offset
        };
        crate::standalone_text::draw(scene, &shaped, x + offset_x, y, color);
        return;
    }
    let free_y = (height - shaped.height()).max(0.0);
    let offset_y = match vertical_align {
        MarginTextVerticalAlign::Top => 0.0,
        MarginTextVerticalAlign::Middle => free_y * 0.5,
        MarginTextVerticalAlign::Bottom => free_y,
    };
    crate::standalone_text::draw(scene, &shaped, x, y + offset_y, color);
}

/// Measure one-line generated margin text using the same font defaults as
/// [`draw_margin_text`].  Replaced content such as an image can use the result
/// as its inline origin without leaking URL syntax into the painted text.
pub(crate) fn measure_margin_text(
    document: &Document,
    content: &str,
    font_size: f32,
    font_family: &str,
) -> f32 {
    crate::standalone_text::shape(
        document,
        content,
        font_size,
        font_family,
        None,
        StandaloneAlign::Start,
    )
    .map_or(0.0, |shaped| shaped.width())
}

/// Measure one-line generated text including trailing whitespace.
///
/// [`measure_margin_text`] intentionally returns the ink/content width used by
/// markers and margin boxes, where trailing whitespace must not move the box.
/// A generated `::before` run is different: its trailing spaces are part of
/// the inline advance consumed before `::after`, so use the first line's
/// advance, which keeps them.
pub(crate) fn measure_margin_text_advance(
    document: &Document,
    content: &str,
    font_size: f32,
    font_family: &str,
) -> f32 {
    crate::standalone_text::shape(
        document,
        content,
        font_size,
        font_family,
        None,
        StandaloneAlign::Start,
    )
    .map_or(0.0, |shaped| shaped.advance())
}

/// Measure the line box height of generated text using the same shaping path as
/// [`draw_margin_text`].  This is needed before an originating auto-height box
/// is painted: generated content contributes to that box's used height even
/// though the current arena has no synthetic child node for it.
pub(crate) fn measure_margin_text_height(
    document: &Document,
    content: &str,
    font_size: f32,
    font_family: &str,
) -> f32 {
    crate::standalone_text::shape(
        document,
        content,
        font_size,
        font_family,
        None,
        StandaloneAlign::Start,
    )
    .map_or(0.0, |shaped| shaped.height())
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum DecorationPhase {
    BeforeGlyphs,
    AfterGlyphs,
}

#[cfg(test)]
pub(crate) fn draw_decoration_phase(
    scene: &mut impl PaintScene,
    decorations: &[&DecorationSpec],
    geometry: DecorationGeometry,
    phase: DecorationPhase,
) {
    draw_resolved_decoration_phase(
        scene,
        &resolve_decoration_lines(decorations, geometry),
        phase,
    );
}

pub(crate) fn draw_resolved_decoration_phase(
    scene: &mut impl PaintScene,
    lines: &[DecorationLine],
    phase: DecorationPhase,
) {
    for line in lines {
        let before = line.kind != DecorationKind::LineThrough;
        if before != matches!(phase, DecorationPhase::BeforeGlyphs) {
            continue;
        }
        paint_decoration_pattern(
            scene,
            line.style,
            css_color_to_peniko(line.color),
            f64::from(line.x_start),
            f64::from(line.x_end),
            f64::from(line.y),
            f64::from(line.thickness),
            f64::from(line.pattern_origin_x),
        );
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

#[cfg(test)]
fn paint_decoration_style(
    scene: &mut impl PaintScene,
    style: TextDecorationStyle,
    color: Color,
    x0: f64,
    x1: f64,
    center: f64,
    thickness: f64,
) {
    paint_decoration_pattern(scene, style, color, x0, x1, center, thickness, x0);
}

#[allow(clippy::too_many_arguments)]
fn paint_decoration_pattern(
    scene: &mut impl PaintScene,
    style: TextDecorationStyle,
    color: Color,
    x0: f64,
    x1: f64,
    center: f64,
    thickness: f64,
    pattern_origin: f64,
) {
    if !pattern_origin.is_finite()
        || !x0.is_finite()
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
            let step =
                natural_step.max((x1 - pattern_origin).max(span) / MAX_DECORATION_SEGMENTS as f64);
            let first_center = pattern_origin + radius;
            let index = ((x0 - radius - first_center) / step).ceil();
            let mut x = first_center + index * step;
            scene.push_clip_layer(
                Affine::IDENTITY,
                &Rect::new(x0, center - radius, x1, center + radius),
            );
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
            scene.pop_layer();
        }
        TextDecorationStyle::Dashed => {
            let (dash, gap) = dashed_lengths((x1 - pattern_origin).max(x1 - x0), thickness);
            let mut path = BezPath::new();
            path.move_to((x0, center));
            path.line_to((x1, center));
            let stroke = Stroke::new(thickness)
                .with_caps(Cap::Butt)
                .with_dashes((x0 - pattern_origin).rem_euclid(dash + gap), [dash, gap]);
            scene.stroke(&stroke, Affine::IDENTITY, color, None, &path);
        }
        TextDecorationStyle::Wavy => {
            let span = (x1 - pattern_origin).max(x1 - x0);
            let wavelength = (thickness * 4.0).max(4.0);
            let amplitude = (thickness * 1.5).max(0.75);
            // Limit path complexity while retaining more detail for normal
            // spans. The cap is important for huge finite advances.
            let half_wave = (wavelength * 0.5).max(span / MAX_DECORATION_SEGMENTS as f64);
            let mut path = BezPath::new();
            let mut index = ((x0 - pattern_origin) / half_wave).floor();
            let mut x = pattern_origin + index * half_wave;
            path.move_to((x, center));
            let mut segments = 0;
            while x < x1 && segments <= MAX_DECORATION_SEGMENTS {
                let end = x + half_wave;
                if end <= x {
                    break;
                }
                let sign = if index.rem_euclid(2.0) < 1.0 {
                    -1.0
                } else {
                    1.0
                };
                let control_x = (x + end) * 0.5;
                path.quad_to((control_x, center + sign * amplitude), (end, center));
                x = end;
                index += 1.0;
                segments += 1;
            }
            let stroke = Stroke::new(thickness).with_caps(Cap::Round);
            scene.push_clip_layer(
                Affine::IDENTITY,
                &Rect::new(
                    x0,
                    center - amplitude - thickness,
                    x1,
                    center + amplitude + thickness,
                ),
            );
            scene.stroke(&stroke, Affine::IDENTITY, color, None, &path);
            scene.pop_layer();
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

/// Convert raikiri-style CssColor (r/g/b/a: u8) to peniko::Color.
///
/// Keep this helper separate for a future move to `peniko::AlphaColor`
/// (it is one line today but easier to find this way).
pub(crate) fn css_color_to_peniko(c: CssColor) -> Color {
    Color::from_rgba8(c.r, c.g, c.b, c.a)
}

#[cfg(test)]
mod tests;
