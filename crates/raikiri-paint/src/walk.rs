//! DOM walker — Document arena を DFS で walk し PaintScene に emit する。
//!
//! `paint_document` / `paint_element` / `paint_node` の 3 関数分離は
//! separation of concerns — paint_element = element 描画責任 (背景/border/
//! children walk、M4 で肉付け)、paint_node = kind dispatch のみ。M3 で inline
//! formatting context を実装する時に paint_element を変えずに paint_node の
//! Text 分岐だけ unreachable 化できる。
//!
//! find_body は raikiri-dom::layout::find_body と重複するが、5 行の helper
//! を crate 境界越境で pub 化するよりも paint 側で持つ方が clean。

use anyrender::PaintScene;
use raikiri_dom::Document;
use raikiri_style::CascadeResult;
use raikiri_traits::PageBox;

pub(crate) fn paint_canvas_background(
    _scene: &mut impl PaintScene,
    _document: &Document,
    _cascade: &CascadeResult,
    _page_box: PageBox,
) {
    // Task 3 で実装
}

pub(crate) fn paint_document(
    _scene: &mut impl PaintScene,
    _document: &Document,
    _cascade: &CascadeResult,
) {
    // Task 3 で実装
}
