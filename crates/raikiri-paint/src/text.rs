//! Text glyph draw — parley Layout の GlyphRun を anyrender::draw_glyphs に pipe。
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
use kurbo::{Affine, Vec2};
use parley::{Alignment, Glyph as ParleyGlyph, PositionedLayoutItem};
use peniko::{Color, Fill};
use raikiri_dom::Node;
use raikiri_style::CascadeResult;
use raikiri_style::property::{CssColor, Direction, TextAlign, TextAlignLast};

pub(crate) fn draw_text_node(
    scene: &mut impl PaintScene,
    node: &Node,
    cascade: &CascadeResult,
    node_id: usize,
    abs_x: f32,
    abs_y: f32,
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
        let last_line_delta = if line_index == last_line_index {
            text_align_last_delta(
                line.metrics(),
                cv.text_align,
                cv.text_align_last,
                cv.direction,
            )
        } else {
            0.0
        };
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
    }
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
    use super::text_align_last_delta;
    use parley::LineMetrics;
    use raikiri_style::property::{Direction, TextAlign, TextAlignLast};

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
}
