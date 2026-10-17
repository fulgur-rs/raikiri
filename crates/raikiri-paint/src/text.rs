//! Text decorations (CSS Text Decoration Level 3 line, style and color, in
//! CSS painting order) and the text of page-margin boxes and generated
//! content, which lies outside the paragraphs of the document.

use std::collections::HashMap;

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

/// Whether a glyph renderer can draw glyphs of `font`: it needs the `head`
/// table for the units per em that scale the outlines. Characters no font
/// covers are shaped with shodo's built-in stub face, which has no tables
/// and so no outline for its `.notdef` glyph; such runs draw no glyphs,
/// though their decorations and emphasis marks still paint.
pub(crate) fn has_glyph_outlines(font: &peniko::FontData) -> bool {
    use skrifa::raw::TableProvider;
    skrifa::FontRef::from_index(font.data.data(), font.index)
        .and_then(|font| font.head())
        .is_ok()
}

/// Draw the missing-glyph boxes of a run shaped with shodo's stub face, as
/// browsers draw a box for a character no font covers. Each origin from
/// [`missing_glyph_origins`] gets an outlined box inside the stub glyph's
/// natural advance of one em, rising from the baseline, in the run's
/// coordinates. The
/// box ignores the spacing in the layout advance, as glyph ink does when
/// `letter-spacing` or justification widens or narrows it.
pub(crate) fn draw_missing_glyph_boxes(
    scene: &mut impl PaintScene,
    origins: impl IntoIterator<Item = (f32, f32)>,
    font_size: f32,
    color: Color,
    transform: Affine,
) {
    let size = f64::from(font_size);
    if !(size.is_finite() && size > 0.0) {
        return;
    }
    let line = size / 16.0;
    let stroke = Stroke::new(line);
    // Inset by half the line width plus a gap, so neighboring boxes and
    // their strokes stay apart.
    let inset = size * 0.1 + line / 2.0;
    for (x, y) in origins {
        let (x, y) = (f64::from(x), f64::from(y));
        if !(x.is_finite() && y.is_finite()) {
            continue;
        }
        let rect = Rect::new(
            x + inset,
            y - size * 0.7 + line / 2.0,
            x + size - inset,
            y - line / 2.0,
        );
        scene.stroke(&stroke, transform, color, None, &rect);
    }
}

/// The origins of the stub-face glyphs of `run` that take a missing-glyph
/// box, from its glyphs as `(text offset, x, y)`.
///
/// The stub face gives a character one em of advance, and gives marks and
/// default-ignorable code points (a zero width joiner, a variation selector)
/// none, so they stay invisible. A cluster thus takes one box per em of its
/// shaping advance. A zero-advance glyph shares its origin with a neighbor
/// in its cluster, or sits half an em back for a mark, so the boxes go at
/// the cluster's distinct origins on its em grid.
pub(crate) fn missing_glyph_origins(
    run: &shodo::GlyphRunView<'_>,
    glyphs: impl IntoIterator<Item = (u32, f32, f32)>,
) -> Vec<(f32, f32)> {
    let size = run.font_size();
    // Sorted by text offset, so each cluster finds its glyphs by search
    // rather than by a scan of the whole run.
    let mut glyphs: Vec<_> = glyphs.into_iter().collect();
    glyphs.sort_by_key(|&(offset, ..)| offset);
    // Layout advances the pen in 1/64 px and rounds each stub glyph's em to
    // that, so boxes are counted and spaced by the rounded em.
    let pitch = (size * 64.0).round() / 64.0;
    let mut origins = Vec::new();
    for cluster in run.clusters() {
        let boxes = (cluster.shaping_advance / pitch).round();
        // An em that rounds to zero gives no count at all. With no font in the
        // document, the stub face also shapes white space, which stays blank.
        if boxes.is_nan() || boxes < 1.0 || cluster.flags.whitespace {
            continue;
        }
        let start =
            glyphs.partition_point(|&(offset, ..)| (offset as usize) < cluster.text_range.start);
        // A generated cluster such as a `text-overflow` ellipsis has an empty
        // text range, with its glyphs at that offset.
        let range_end = cluster.text_range.end.max(cluster.text_range.start + 1);
        let end = glyphs.partition_point(|&(offset, ..)| (offset as usize) < range_end);
        let mut points: Vec<(f32, f32)> =
            glyphs[start..end].iter().map(|&(_, x, y)| (x, y)).collect();
        points.sort_by(|a, b| a.0.total_cmp(&b.0));
        points.dedup_by(|b, a| (b.0 - a.0).abs() < size * 1e-3);
        // The characters with an em sit one em apart. A mark sits half an em
        // off that grid, so the boxes go on the grid that holds the most
        // origins, starting from the leftmost such origin.
        let count = boxes as usize;
        let on_grid = |anchor: f32, x: f32| {
            let steps = (x - anchor) / pitch;
            let step = steps.round();
            (steps - step).abs() < 1e-3 && step >= 0.0 && (step as usize) < count
        };
        // Origins on one grid share their offset from the first origin modulo
        // the pitch, so a single pass sorts them into grids, each anchored at
        // its leftmost origin.
        let first = points.first().map_or(0.0, |&(x, _)| x);
        let mut grids: HashMap<i64, (f32, usize)> = HashMap::new();
        for &(x, _) in &points {
            let phase = ((x - first) / pitch).rem_euclid(1.0);
            let key = (phase * 1000.0).round() as i64 % 1000;
            let (anchor, matches) = grids.entry(key).or_insert((x, 0));
            if on_grid(*anchor, x) {
                *matches += 1;
            }
        }
        let anchor = grids
            .into_values()
            .max_by(|a, b| a.1.cmp(&b.1).then(b.0.total_cmp(&a.0)))
            // A cluster with no glyph has no grid, and no origin is on NaN.
            .map_or(f32::NAN, |(anchor, _)| anchor);
        origins.extend(points.into_iter().filter(|&(x, _)| on_grid(anchor, x)));
    }
    origins
}

pub(crate) fn synthetic_embolden(enabled: bool, font_size: f32) -> Vec2 {
    if enabled {
        let size = font_size as f64;
        Vec2::new((0.015125 * size).min(0.3), (0.0121 * size).min(0.3))
    } else {
        Vec2::ZERO
    }
}

/// Vertical placement of standalone text inside a box. Page-margin boxes are
/// placed by `raikiri_dom::page_margin_boxes`; the remaining callers draw at
/// the top, and the other placements are exercised by tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))]
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
    font_family: &[raikiri_style::FontFamilyName],
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

/// Measure one-line generated text including trailing whitespace.
///
/// Standalone text width excludes trailing whitespace for markers and margin
/// boxes, where trailing whitespace must not move the box.
/// A generated `::before` run is different: its trailing spaces are part of
/// the inline advance consumed before `::after`, so use the first line's
/// advance, which keeps them.
pub(crate) fn measure_margin_text_advance(
    document: &Document,
    content: &str,
    font_size: f32,
    font_family: &[raikiri_style::FontFamilyName],
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
    font_family: &[raikiri_style::FontFamilyName],
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
            f64::from(line.pattern_end_x),
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
    paint_decoration_pattern(scene, style, color, x0, x1, center, thickness, x0, x1);
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
    pattern_end: f64,
) {
    if !pattern_end.is_finite()
        || pattern_end <= pattern_origin
        || !pattern_origin.is_finite()
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
            let span = pattern_end - pattern_origin;
            let natural_step = (radius * 4.0).max(2.0);
            // A hostile but finite letter-spacing/advance must not turn into
            // an unbounded number of scene commands.
            let step = natural_step.max(span / MAX_DECORATION_SEGMENTS as f64);
            let first_center = pattern_origin + radius;
            let index = ((x0 - radius - first_center) / step).ceil();
            let mut x = first_center + index * step;
            scene.push_clip_layer(
                Affine::IDENTITY,
                &Rect::new(x0, center - radius, x1, center + radius),
            );
            let mut segments = 0;
            while x - radius < x1 && segments < MAX_DECORATION_SEGMENTS {
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
            let (dash, gap) = dashed_lengths(pattern_end - pattern_origin, thickness);
            let mut path = BezPath::new();
            path.move_to((x0, center));
            path.line_to((x1, center));
            let stroke = Stroke::new(thickness)
                .with_caps(Cap::Butt)
                .with_dashes((x0 - pattern_origin).rem_euclid(dash + gap), [dash, gap]);
            scene.stroke(&stroke, Affine::IDENTITY, color, None, &path);
        }
        TextDecorationStyle::Wavy => {
            let span = pattern_end - pattern_origin;
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
                    break; // cov:ignore: valid segments lie within the shared extent; the bounded f64 step exceeds roundoff.
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
