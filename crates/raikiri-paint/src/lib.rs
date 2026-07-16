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
    use super::*;
    use anyrender::Scene;
    use anyrender::recording::RenderCommand;
    use raikiri_dom::{Document, layout_single_page};
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;
    use taffy::{Dimension, Display, Size, Style};

    /// hello-world (`<html><head></head><body><p style="color:red">Hi</p></body></html>`) を
    /// m1.6 layout_single_page まで完了させた Document + CascadeResult を返す。
    /// m1.7 test 全ての共通 setup。
    fn hello_world_paint_setup() -> (Document, raikiri_style::CascadeResult) {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let p = doc.append_element(Some(body), "p", Style::default(), Some("color:red"));
        let _t = doc.append_text(p, "Hi");
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        layout_single_page(&mut doc, &cr, PageBox::A4).expect("layout Ok");
        (doc, cr)
    }

    #[test]
    fn paint_single_page_compiles_and_returns_unit() {
        let doc = Document::new();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        let mut scene = Scene::new();
        paint_single_page(&mut scene, &doc, &cr, PageBox::A4);
        // fragment (no body) なので paint_document は早期 return、canvas は no-op site
        assert!(scene.commands.is_empty(), "empty Document should emit no commands");
    }

    #[test]
    fn paint_single_page_canvas_background_site_is_noop_at_m1_4() {
        // canvas fill site が M1.4 で命令を積まないことを pin。
        // M4 で background-color が cascade に入った時にこの test が反転する
        // ("commands should be non-empty" で fail する)、その時に M4 実装完了の合図。
        let (doc, cr) = hello_world_paint_setup();
        let mut scene = Scene::new();
        // canvas fill 単体を呼ぶ相当の効果を得るため、paint_single_page 全体 - text glyph の
        // 差分で判定 (canvas site が emit すれば Fill command が glyph より前に来る)。
        // Task 4 完了までは text も stub なので、全体 commands 0 でも OK。
        // Task 4 完了後にこの test は "GlyphRun のみ、Fill (canvas) は無い" に精緻化予定。
        paint_single_page(&mut scene, &doc, &cr, PageBox::A4);
        let fill_commands: Vec<_> = scene
            .commands
            .iter()
            .filter(|c| matches!(c, RenderCommand::Fill(_)))
            .collect();
        assert!(
            fill_commands.is_empty(),
            "M1.4 canvas site should emit no Fill, got {:?}",
            fill_commands.len()
        );
    }

    #[test]
    fn paint_single_page_without_body_returns_early() {
        // fragment (Document → <p> 直子、no <body>) → paint は何も emit しない。
        // m1.6 layout_single_page は fragment で Err を返すが、paint 側は
        // defensive に silent return する契約 (early return without touching scene)。
        let mut doc = Document::new();
        let p = doc.append_element(Some(0), "p", Style::default(), None::<&str>);
        let _t = doc.append_text(p, "Hi");
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        // layout_single_page はここでは呼ばない (Err になる)、paint 単体で silent
        // return するかを test。
        let mut scene = Scene::new();
        paint_single_page(&mut scene, &doc, &cr, PageBox::A4);
        assert!(scene.commands.is_empty(), "fragment (no body) should emit no commands");
    }

    #[test]
    fn paint_single_page_skips_zero_size_subtree() {
        // display:none 相当を模した zero-size element (display: None, size 明示) が
        // walk されないことを pin。paint_element の early return が正しく発火する
        // regression pin (m1.6 の layout side でも empty_display_none_leaf... で pin 済)。
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        // hidden element を body 直下に置く (display: None、size 0)
        let hidden_style = Style {
            display: Display::None,
            size: Size {
                width: Dimension::length(100.0),
                height: Dimension::length(50.0),
            },
            ..Default::default()
        };
        let hidden = doc.append_element(Some(body), "hidden", hidden_style, None::<&str>);
        let _child_of_hidden = doc.append_text(hidden, "should not be painted");
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        layout_single_page(&mut doc, &cr, PageBox::A4).expect("layout Ok");
        // hidden の layout size は 0 になっているはず (taffy LayoutOutput::HIDDEN)
        let hidden_layout = doc.get_node(hidden).unwrap().unrounded_layout;
        assert_eq!(
            hidden_layout.size.width, 0.0,
            "display:none should have zero size after taffy layout"
        );
        let mut scene = Scene::new();
        paint_single_page(&mut scene, &doc, &cr, PageBox::A4);
        // Task 4 完了までは text も stub、Task 4 後は glyph run 0 個 (hidden 配下の text は skip)
        let glyph_commands: Vec<_> = scene
            .commands
            .iter()
            .filter(|c| matches!(c, RenderCommand::GlyphRun(_)))
            .collect();
        assert!(
            glyph_commands.is_empty(),
            "text inside display:none subtree should not be painted, got {}",
            glyph_commands.len()
        );
    }
}
