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
//! offset + baseline を baked-in。anyrender transform は text node の絶対座標
//! への平行移動のみで済む (baseline / offset 加算不要)。

use anyrender::{Glyph as AnyrenderGlyph, PaintScene};
use kurbo::{Affine, BezPath, Cap, Circle, Point, Rect, Stroke, Vec2};
use parley::{Alignment, Glyph as ParleyGlyph, PositionedLayoutItem};
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

/// Add the decoration originated by one element to the descendant paint
/// context. `none` does not cancel a line propagated by an ancestor.
pub(crate) fn push_element_decoration(decorations: &mut Vec<DecorationSpec>, cv: &ComputedValues) {
    if !has_paintable_line(cv.text_decoration_line) {
        return;
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
    let origin_font_size = cv.font_size.px().max(1.0) as f64;
    decorations.push(DecorationSpec {
        line: cv.text_decoration_line,
        style: cv.text_decoration_style,
        color,
        origin_thickness: (origin_font_size / 16.0).max(1.0),
        // These normalized metrics keep the position tied to the decorating
        // element even when a descendant uses a different font size.
        origin_ascent: origin_font_size * 0.8,
        origin_descent: origin_font_size * 0.2,
    });
}

/// Build the decoration context for an element's children.
///
/// A boundary drops ancestor lines first, then retains a line originated by
/// the boundary element itself. This models the CSS rule that a child cannot
/// cancel an ancestor decoration while atomic/out-of-flow boxes do not receive
/// that ancestor decoration.
pub(crate) fn decorations_for_element(
    mut decorations: Vec<DecorationSpec>,
    cv: &ComputedValues,
) -> Vec<DecorationSpec> {
    if is_decoration_propagation_boundary(cv) {
        decorations.clear();
    }
    push_element_decoration(&mut decorations, cv);
    decorations
}

fn has_paintable_line(line: TextDecorationLine) -> bool {
    line.underline || line.overline || line.line_through
}

pub(crate) fn draw_text_node(
    scene: &mut impl PaintScene,
    node: &Node,
    cascade: &CascadeResult,
    node_id: usize,
    abs_x: f32,
    abs_y: f32,
    decorations: &[DecorationSpec],
) {
    // Text node の pre-shape 結果を取得。preshape_text が empty text で None を返す
    // ので、None = "empty text" の signal、silent return。
    let Some(text_layout) = node.text_layout() else {
        return;
    };

    // Text node の brush = cascade で親から inherit された color (現状 color property のみ対応)。
    // ComputedValues.color は Text node 位置にも populate 済 (inheritance walk 経由)。
    let cv = &cascade.computed[node_id];
    let brush = css_color_to_peniko(cv.color);

    // parley positioned_glyphs() は line 内 offset + baseline を baked-in するので
    // scene transform は text node の絶対座標への平行移動のみ。
    let base_transform = Affine::translate((abs_x as f64, abs_y as f64));
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
        let geometry = decoration_geometry(decorations, metrics, abs_x, abs_y, last_line_delta);

        // CSS paints underline/overline before the glyphs and line-through
        // after them. Keeping the phases separate also makes the order stable
        // when a declaration contains several line keywords.
        if let Some(geometry) = geometry {
            for decoration in decorations {
                draw_decoration_phase(scene, decoration, geometry, DecorationPhase::BeforeGlyphs);
            }
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
            for decoration in decorations {
                draw_decoration_phase(scene, decoration, geometry, DecorationPhase::AfterGlyphs);
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct DecorationGeometry {
    x0: f64,
    x1: f64,
    abs_y: f64,
    baseline: f32,
}

fn decoration_geometry(
    decorations: &[DecorationSpec],
    metrics: &parley::LineMetrics,
    abs_x: f32,
    abs_y: f32,
    last_line_delta: f32,
) -> Option<DecorationGeometry> {
    if decorations.is_empty() {
        return None;
    }
    // `advance` includes trailing whitespace. CSS Text Decoration 4 has an
    // explicit skip-spaces property; until that property is consumed, keeping
    // the full shaped advance is the least surprising Level 3 behavior.
    let line_width = metrics.advance.max(0.0);
    if line_width <= 0.0 {
        // cov:ignore: shaped text lines in the current layout always have a
        // positive advance; retain the guard for empty/future line metrics.
        return None;
    }
    let line_start = metrics.inline_min_coord + metrics.offset + last_line_delta;
    Some(DecorationGeometry {
        x0: abs_x as f64 + line_start as f64,
        x1: abs_x as f64 + (line_start + line_width) as f64,
        abs_y: abs_y as f64,
        baseline: metrics.baseline,
    })
}

#[derive(Clone, Copy, Debug)]
enum DecorationPhase {
    BeforeGlyphs,
    AfterGlyphs,
}

fn draw_decoration_phase(
    scene: &mut impl PaintScene,
    decoration: &DecorationSpec,
    geometry: DecorationGeometry,
    phase: DecorationPhase,
) {
    let DecorationGeometry {
        x0,
        x1,
        abs_y,
        baseline: baseline_offset,
    } = geometry;
    let color = css_color_to_peniko(decoration.color);
    let baseline = abs_y + baseline_offset as f64;
    let thickness = decoration.origin_thickness;
    let center_for = |line: TextDecorationLine| -> Option<f64> {
        if line.underline {
            Some(baseline + decoration.origin_descent * 0.5)
        } else if line.overline {
            Some(baseline - decoration.origin_ascent + thickness * 0.5)
        } else if line.line_through {
            Some(baseline - decoration.origin_ascent * 0.35)
        } else {
            // cov:ignore: every caller passes a single enabled line keyword;
            // this fallback is only a defensive totality guard.
            None
        }
    };

    // Underline and overline are painted below/above the glyphs, while
    // line-through is painted over the glyphs. The metrics in DecorationSpec
    // come from the originating element, not this descendant text node.
    let positions = match phase {
        DecorationPhase::BeforeGlyphs => [
            (decoration.line.underline, TextDecorationLine::UNDERLINE),
            (decoration.line.overline, TextDecorationLine::OVERLINE),
            (false, TextDecorationLine::LINE_THROUGH),
        ],
        DecorationPhase::AfterGlyphs => [
            (false, TextDecorationLine::UNDERLINE),
            (false, TextDecorationLine::OVERLINE),
            (
                decoration.line.line_through,
                TextDecorationLine::LINE_THROUGH,
            ),
        ],
    };
    for (enabled, line) in positions {
        if !enabled {
            continue;
        }
        let Some(center) = center_for(line) else {
            // cov:ignore: `line` comes from the matching enabled flag above,
            // so the center is structurally present.
            continue;
        };
        paint_decoration_style(scene, decoration.style, color, x0, x1, center, thickness);
    }
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
    if x1 <= x0 {
        return; // cov:ignore: caller guards positive shaped line width.
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
            let step = (radius * 4.0).max(2.0);
            let mut x = x0 + radius;
            while x < x1 {
                scene.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    color,
                    None,
                    &Circle::new(Point::new(x, center), radius),
                );
                x += step;
            }
        }
        TextDecorationStyle::Dashed => {
            let dash = (thickness * 3.0).max(2.0);
            let gap = (thickness * 2.0).max(2.0);
            let mut path = BezPath::new();
            path.move_to((x0, center));
            path.line_to((x1, center));
            let stroke = Stroke::new(thickness)
                .with_caps(Cap::Butt)
                .with_dashes(0.0, [dash, gap]);
            scene.stroke(&stroke, Affine::IDENTITY, color, None, &path);
        }
        TextDecorationStyle::Wavy => {
            let wavelength = (thickness * 4.0).max(4.0);
            let amplitude = (thickness * 1.5).max(0.75);
            let half_wave = wavelength * 0.5;
            let mut path = BezPath::new();
            path.move_to((x0, center));
            let mut x = x0;
            let mut sign = -1.0;
            while x < x1 {
                let end = (x + half_wave).min(x1);
                let control_x = (x + end) * 0.5;
                path.quad_to((control_x, center + sign * amplitude), (end, center));
                x = end;
                sign = -sign;
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
    use super::{DecorationSpec, decorations_for_element, text_align_last_delta};
    use parley::LineMetrics;
    use raikiri_style::ComputedValues;
    use raikiri_style::property::{
        CssColor, Direction, DisplayValue, FloatValue, PositionValue, TextAlign, TextAlignLast,
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

    fn ancestor_decoration() -> DecorationSpec {
        DecorationSpec {
            line: TextDecorationLine::UNDERLINE,
            style: TextDecorationStyle::Solid,
            color: CssColor::BLACK,
            origin_thickness: 1.0,
            origin_ascent: 12.8,
            origin_descent: 3.2,
        }
    }

    #[test]
    fn decoration_metrics_are_taken_from_the_originating_element() {
        let mut cv = ComputedValues::initial();
        cv.font_size = raikiri_style::resolve::ComputedLength(32.0);
        cv.text_decoration_line = TextDecorationLine::UNDERLINE;
        let decorations = decorations_for_element(Vec::new(), &cv);
        assert_eq!(decorations.len(), 1);
        assert_eq!(decorations[0].origin_thickness, 2.0);
        assert_eq!(decorations[0].origin_ascent, 25.6);
        assert_eq!(decorations[0].origin_descent, 6.4);
    }

    #[test]
    fn ancestor_decoration_stops_at_out_of_flow_float_and_atomic_boundaries() {
        let ancestor = vec![ancestor_decoration()];
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
                decorations_for_element(ancestor.clone(), &cv).is_empty(),
                "ancestor decoration crossed {name} boundary"
            );
        }

        let mut in_flow = ComputedValues::initial();
        in_flow.display = DisplayValue::Block;
        assert_eq!(decorations_for_element(ancestor.clone(), &in_flow).len(), 1);

        let mut boundary_origin = ComputedValues::initial();
        boundary_origin.display = DisplayValue::InlineBlock;
        boundary_origin.text_decoration_line = TextDecorationLine::OVERLINE;
        assert_eq!(decorations_for_element(ancestor, &boundary_origin).len(), 1);
        assert_eq!(
            decorations_for_element(vec![ancestor_decoration()], &boundary_origin)[0].line,
            TextDecorationLine::OVERLINE
        );
    }
}
