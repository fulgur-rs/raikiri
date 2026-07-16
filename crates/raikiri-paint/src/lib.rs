//! raikiri-paint — Document + CascadeResult → anyrender::PaintScene walker.
//!
//! nzv.11 の refute 結果 (docs/feasibility-report.md §3.4) に従い、bridge trait
//! を挟まず `impl anyrender::PaintScene` を直接消費する。M1 は単一 A4 ページ +
//! text glyphs のみ、element background / border / decoration は M4+ に defer。
//!
//! ## Contract
//!
//! - `document` は m1.6 `layout_single_page` を呼び終えた post-layout 状態を前提
//!   (Node.unrounded_layout / Node.text_layout populate 済)
//! - `cascade.computed.len() == document.node_count()` を前提 (caller 責任)
//! - `scene.reset()` の呼び出しは caller 責任 (blitz-paint と同じ convention)
//! - infallible — raikiri-traits::RenderError に Paint variant はない
//!   (m1.6 で pre-shape / layout 済の Document 消費が原理的 infallible)

use anyrender::PaintScene;
use raikiri_dom::Document;
use raikiri_style::CascadeResult;
use raikiri_traits::PageBox;

mod text;
mod walk;

/// 単一 A4 (or 指定 PageBox) ページに Document + CascadeResult を paint する。
///
/// # 呼び出し順序
/// 1. `walk::paint_canvas_background` — canvas 背景 fill site (M1.4 no-op、M4 で発火)
/// 2. `walk::paint_document` — body から始まる DFS walk、Text node で glyph draw
///
/// # Non-goals (M1.7 scope)
/// - Multi-page pagination → M2
/// - Element background-color / border / box-shadow → M4+
/// - Text decoration (underline / line-through) → M3
/// - z-index / stacking context → M4+
/// - CSS transform (rotate/scale/skew) → M4+
/// - DPI scaling → m1.14 or m8 (`paint_single_page_scaled` 別関数で拡張)
pub fn paint_single_page(
    scene: &mut impl PaintScene,
    document: &Document,
    cascade: &CascadeResult,
    page_box: PageBox,
) {
    walk::paint_canvas_background(scene, document, cascade, page_box);
    walk::paint_document(scene, document, cascade);
}

#[cfg(test)]
mod tests {
    // Task 3 以降で unit test を追加。ここでは compile pass の smoke test のみ。
    use super::*;
    use anyrender::Scene;
    use raikiri_dom::Document;
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    #[test]
    fn paint_single_page_compiles_and_returns_unit() {
        let doc = Document::new();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        let mut scene = Scene::new();
        paint_single_page(&mut scene, &doc, &cr, PageBox::A4);
        // stub なので commands は積まれない
        assert!(
            scene.commands.is_empty(),
            "stub paint_single_page should emit no commands"
        );
    }
}
