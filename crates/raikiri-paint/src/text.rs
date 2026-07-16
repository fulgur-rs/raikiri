//! Text glyph draw — parley Layout の GlyphRun を anyrender::draw_glyphs に pipe。
//!
//! m1.6 で `Node.text_layout: Option<parley::Layout<()>>` に pre-shape 済 Layout
//! を格納する design に依拠。paint は line iteration + GlyphRun.positioned_glyphs()
//! を per-run 変換 (parley::Glyph → anyrender::Glyph) して scene に送るだけ。

use anyrender::PaintScene;
use raikiri_dom::Node;
use raikiri_style::CascadeResult;

// walk::paint_document (Task 3) が Text node 分岐で呼び出すまで未使用。
// stub 段階では dead_code warning を抑止する (-D warnings gate 対応)。
#[allow(dead_code)]
pub(crate) fn draw_text_node(
    _scene: &mut impl PaintScene,
    _node: &Node,
    _cascade: &CascadeResult,
    _node_id: usize,
    _abs_x: f32,
    _abs_y: f32,
) {
    // Task 4 で実装
}
