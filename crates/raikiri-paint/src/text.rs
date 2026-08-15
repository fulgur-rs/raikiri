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
use parley::{Glyph as ParleyGlyph, PositionedLayoutItem};
use peniko::{Color, Fill};
use raikiri_dom::Node;
use raikiri_style::CascadeResult;
use raikiri_style::property::CssColor;

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

    for line in text_layout.lines() {
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
                glyph_run.positioned_glyphs().map(to_anyrender_glyph),
            );
        }
    }
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
