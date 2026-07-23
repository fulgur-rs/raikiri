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
    use parley::FontContext;
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
        layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");
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
        assert!(
            scene.commands.is_empty(),
            "empty Document should emit no commands"
        );
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
        assert!(
            scene.commands.is_empty(),
            "fragment (no body) should emit no commands"
        );
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
        let hidden = doc.append_element(Some(body), "hidden", hidden_style, Some("display:none"));
        let _child_of_hidden = doc.append_text(hidden, "should not be painted");
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");
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

    #[test]
    fn paint_single_page_paints_zero_size_display_block_subtree() {
        // roborev job 227 finding 対応: display:none test 単独では、旧
        // "size == 0 で skip" の実装でも pass するため、修正 (is_display_none 判定
        // への切替) の regression protection にならない。この test は逆側 —
        // display:block だが size=0 の container の中に text 子供を置き、
        // GlyphRun が **emit される** ことを assert する。旧 size-based skip では
        // container が skipped → 子 text の GlyphRun が失われて test failure。
        // 現行 is_display_none 判定では container は Display::Block なので walk
        // 継続 → 子 text の GlyphRun が emit される。
        //
        // raikiri-spike-ggig (Sprint 18 Wave 2): `apply_computed_to_style` が
        // bridge_size (width component) を dispatch するようになった。fixture の
        // 手構築 `taffy::Style { size.width = length(0) }` は `cv.width` = Auto
        // 初期値で上書きされる (`<container>` に author width なしのため)。
        // width=auto の Display::Block は containing width (=A4) に stretch され、
        // `size.width == 0.0` sanity assert が失敗する。修正: inline に
        // `"width: 0px; height: 0px"` を与え cv.{width,height} = Length::Px(0.0)
        // を bridge が翻訳するようにする。**height は Wave 3 (raikiri-spike-01up)
        // まで bridge されない**ため、`zero_block_style.size.height = length(0.0)`
        // の手構築値を **保持** して sanity assert を維持する (Wave 3 landing 後は
        // inline `height: 0px` が bridge_size で反映されるので手構築値は冗長になる)。
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let zero_block_style = Style {
            display: Display::Block,
            size: Size {
                width: Dimension::length(0.0),
                height: Dimension::length(0.0),
            },
            ..Default::default()
        };
        let container = doc.append_element(
            Some(body),
            "container",
            zero_block_style,
            // width は bridge が clobber するため inline で明示。height は Wave 3
            // (01up) が bridge するまで手構築 style.size.height=length(0) が残る。
            Some("width: 0px; height: 0px"),
        );
        let _text = doc.append_text(container, "visible");
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

        // sanity: container の size は 0 (explicit width/height=0 を尊重)
        let container_layout = doc.get_node(container).unwrap().unrounded_layout;
        assert_eq!(
            container_layout.size.width, 0.0,
            "zero-size Display::Block container should keep size.width = 0"
        );
        assert_eq!(
            container_layout.size.height, 0.0,
            "zero-size Display::Block container should keep size.height = 0"
        );

        let mut scene = Scene::new();
        paint_single_page(&mut scene, &doc, &cr, PageBox::A4);
        let glyph_commands: Vec<_> = scene
            .commands
            .iter()
            .filter(|c| matches!(c, RenderCommand::GlyphRun(_)))
            .collect();
        assert_eq!(
            glyph_commands.len(),
            1,
            "text inside zero-size Display::Block container must still be painted (overflow: visible \
             default), got {}",
            glyph_commands.len()
        );
    }

    #[test]
    fn paint_single_page_hello_world_emits_one_glyph_run() {
        // hello-world "Hi" → 1 GlyphRun が emit されることを pin。
        let (doc, cr) = hello_world_paint_setup();
        let mut scene = Scene::new();
        paint_single_page(&mut scene, &doc, &cr, PageBox::A4);
        let glyph_commands: Vec<_> = scene
            .commands
            .iter()
            .filter(|c| matches!(c, RenderCommand::GlyphRun(_)))
            .collect();
        assert_eq!(
            glyph_commands.len(),
            1,
            "hello world 'Hi' should emit exactly 1 GlyphRun, got {}",
            glyph_commands.len()
        );
    }

    #[test]
    fn paint_single_page_uses_inherited_color_as_brush() {
        // <p style="color:red"> → cascade で red が inherit → text の brush が
        // (255, 0, 0, 255) になることを pin。cascade → shape 系譜の regression 保護。
        use anyrender::types::Paint;
        use peniko::Color;

        let (doc, cr) = hello_world_paint_setup();
        let mut scene = Scene::new();
        paint_single_page(&mut scene, &doc, &cr, PageBox::A4);
        let RenderCommand::GlyphRun(glyph_cmd) = scene
            .commands
            .iter()
            .find(|c| matches!(c, RenderCommand::GlyphRun(_)))
            .expect("must have 1 GlyphRun")
        else {
            unreachable!()
        };
        match &glyph_cmd.brush {
            Paint::Solid(color) => {
                assert_eq!(
                    *color,
                    Color::from_rgba8(255, 0, 0, 255),
                    "inline color:red must produce Color::from_rgba8(255,0,0,255) brush"
                );
            }
            other => panic!(
                "expected Paint::Solid, got {:?}",
                std::mem::discriminant(other)
            ),
        }
    }

    #[test]
    fn paint_single_page_positions_glyphs_via_absolute_offset() {
        // body / p / text の accumulate location が draw_glyphs の transform に
        // 正しく反映されることを exact-match で pin (roborev job 223 finding 対応)。
        //
        // 従来 `translation >= 0` の弱い assertion では identity transform (=
        // accumulation を丸ごと忘れた実装) でも pass してしまう。<p> に非ゼロの
        // taffy margin を付けて p.location.x/y を non-zero に押し、accumulation
        // logic を実 exercise する。expected = body.location + p.location +
        // text.location (block layout の flow 累積)。
        //
        // raikiri-spike-j5rz (Sprint 18) 以降 `apply_computed_to_style` が
        // `bridge_margin` で cascade → taffy 変換を行うため、hand-set した
        // taffy `Style { margin: ... }` は cascade の initial 0 で上書きされる。
        // 従って margin は CSS inline (`style="margin: ..."`) 経路で与える —
        // これが production の real code path とも整合する。
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        // `margin: 20px 0px 0px 20px` (top=20, right=0, bottom=0, left=20) — 従来の
        // hand-set と同 shape を CSS で再現。`0px` は明示 (raikiri-style
        // `parse_length_value` は bare unitless `0` を受理しない spec-subset
        // 実装のため、shorthand の 4 side で unit を全 side に付ける)。
        // `color:red` は既存 assertion で brush 経路の regression pin として
        // 保持されているため concatenate する。
        let p = doc.append_element(
            Some(body),
            "p",
            Style::default(),
            Some("margin: 20px 0px 0px 20px; color: red"),
        );
        let text = doc.append_text(p, "Hi");
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

        // margin=20 が p.location を non-zero に押していることを確認 (accumulation
        // logic を exercise する前提が satisfy されていることの sanity check)。
        let p_loc = doc.get_node(p).unwrap().unrounded_layout.location;
        assert!(
            p_loc.x >= 20.0,
            "p.location.x should reflect margin=20, got {}",
            p_loc.x
        );
        assert!(
            p_loc.y >= 20.0,
            "p.location.y should reflect margin=20, got {}",
            p_loc.y
        );

        // 期待累積 = body.location + p.location + text.location
        let (expected_x, expected_y) =
            [body, p, text]
                .iter()
                .fold((0.0f32, 0.0f32), |(ax, ay), &id| {
                    let loc = doc.get_node(id).unwrap().unrounded_layout.location;
                    (ax + loc.x, ay + loc.y)
                });

        let mut scene = Scene::new();
        paint_single_page(&mut scene, &doc, &cr, PageBox::A4);
        let RenderCommand::GlyphRun(glyph_cmd) = scene
            .commands
            .iter()
            .find(|c| matches!(c, RenderCommand::GlyphRun(_)))
            .expect("must have 1 GlyphRun")
        else {
            unreachable!()
        };
        // Affine の translation 成分は as_coeffs() の [4, 5] (2 次元 identity +
        // translation)。kurbo::Affine には translation() getter が無いため
        // as_coeffs() で decode する。
        let coeffs = glyph_cmd.transform.as_coeffs();
        let epsilon = 1e-5f64;
        assert!(
            (coeffs[4] - expected_x as f64).abs() < epsilon,
            "translation.x = {}, expected accumulated = {} (body {} + p {} + text {})",
            coeffs[4],
            expected_x,
            doc.get_node(body).unwrap().unrounded_layout.location.x,
            doc.get_node(p).unwrap().unrounded_layout.location.x,
            doc.get_node(text).unwrap().unrounded_layout.location.x,
        );
        assert!(
            (coeffs[5] - expected_y as f64).abs() < epsilon,
            "translation.y = {}, expected accumulated = {} (body {} + p {} + text {})",
            coeffs[5],
            expected_y,
            doc.get_node(body).unwrap().unrounded_layout.location.y,
            doc.get_node(p).unwrap().unrounded_layout.location.y,
            doc.get_node(text).unwrap().unrounded_layout.location.y,
        );
    }

    #[test]
    fn paint_single_page_skips_empty_text() {
        // text_layout が None (empty text) の Text node は draw_glyphs を呼ばず
        // silent skip する contract。preshape_text が empty text で text_layout = None
        // を残す仕様と対称。
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let p = doc.append_element(Some(body), "p", Style::default(), None::<&str>);
        let _t = doc.append_text(p, ""); // ← empty text
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");
        let mut scene = Scene::new();
        paint_single_page(&mut scene, &doc, &cr, PageBox::A4);
        let glyph_commands: Vec<_> = scene
            .commands
            .iter()
            .filter(|c| matches!(c, RenderCommand::GlyphRun(_)))
            .collect();
        assert!(
            glyph_commands.is_empty(),
            "empty text should not emit any GlyphRun, got {}",
            glyph_commands.len()
        );
    }

    #[test]
    fn paint_single_page_can_be_called_multiple_times() {
        // 同じ Document を 2 回 paint、2 回とも同じ command sequence を produce
        // (state mutation なし、re-entrance safety pin)。将来 paint 側で cache
        // 導入した時の silent regression 検出用 pin。
        let (doc, cr) = hello_world_paint_setup();

        let mut scene1 = Scene::new();
        paint_single_page(&mut scene1, &doc, &cr, PageBox::A4);
        let count1 = scene1.commands.len();
        let glyph_count1 = scene1
            .commands
            .iter()
            .filter(|c| matches!(c, RenderCommand::GlyphRun(_)))
            .count();

        let mut scene2 = Scene::new();
        paint_single_page(&mut scene2, &doc, &cr, PageBox::A4);
        let count2 = scene2.commands.len();
        let glyph_count2 = scene2
            .commands
            .iter()
            .filter(|c| matches!(c, RenderCommand::GlyphRun(_)))
            .count();

        assert_eq!(
            count1, count2,
            "total command count must be identical across calls"
        );
        assert_eq!(
            glyph_count1, glyph_count2,
            "GlyphRun count must be identical across calls"
        );
        assert_eq!(glyph_count1, 1, "hello world must emit exactly 1 GlyphRun");
    }

    /// raikiri-spike-d9y.5 (SEC MED, Codex Cloud Security finding) +
    /// raikiri-spike-s8w (§15.3.1 完全化): HTML の hidden elements
    /// (`<style>` / `<script>` / `<noscript>` / `<datalist>` / `<noembed>` /
    /// `<noframes>` / `<rp>` 等) 内の text が rendered artifact に混入しない
    /// ことを pin する。
    ///
    /// 各 fixture では:
    /// - inert element 手前の "before" text と後ろの "after" text を配置し、
    ///   それらは正しく painted (GlyphRun 2 個) される
    /// - inert element 内の raw text は painted されない (leak 検出)
    ///
    /// 単純な "painted 0 個" 判定だと inert filter が **全** subtree を dumb
    /// に潰しても pass してしまうため、"before/after は残る + inert 内は消える"
    /// の 3-way discriminant で filter が正確に働くことを assert する。
    ///
    /// HTML LS §15.3.1 "Hidden elements"
    /// (<https://html.spec.whatwg.org/multipage/rendering.html#hidden-elements>)
    /// が primary source。
    fn assert_inert_html_content_not_painted(html: &[u8], fixture_label: &str) {
        use raikiri_html::{ParseOptions, parse};

        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        };
        let uncascaded = parse(html, &opts).expect("parse ok");
        let mut doc = uncascaded.dom;
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

        let mut scene = Scene::new();
        paint_single_page(&mut scene, &doc, &cr, PageBox::A4);

        let glyph_commands: Vec<_> = scene
            .commands
            .iter()
            .filter_map(|c| match c {
                RenderCommand::GlyphRun(cmd) => Some(cmd),
                _ => None,
            })
            .collect();
        // GlyphRun 数 = 2 (before + after)。inert が leak なら 3、boundary が
        // 落ちるなら 1 or 0。3-way discriminant で filter over-collapse も検知。
        assert_eq!(
            glyph_commands.len(),
            2,
            "[{fixture_label}] expected 2 GlyphRuns (before + after), got {}. \
             Any extra run indicates inert-element text leaked into paint.",
            glyph_commands.len()
        );
        // codex final review finding #3: 2-of-3 selection (before + inert
        // survived、after 落ちた) でも count==2 で pass する余地を封じる。
        // 総 glyph 数を "before" (6 chars) + "after" (5 chars) = 11 の
        // exact match で pin。inert content が leak なら len が増える、
        // boundary text が落ちれば len が減る。
        // 各 assert_inert_html_content_not_painted call site が同じ
        // fixture "before…after" 文字列前提であることに依存。
        let total_glyphs: usize = glyph_commands.iter().map(|cmd| cmd.glyphs.len()).sum();
        assert_eq!(
            total_glyphs,
            "before".len() + "after".len(),
            "[{fixture_label}] total glyph count must equal len('before') + len('after') = 11, \
             got {total_glyphs}. Any deviation indicates either inert-element leak (too many) \
             or boundary text drop (too few) — 2-of-3 selection also fails this exact match."
        );
    }

    #[test]
    fn paint_single_page_skips_style_subtree_content() {
        // <body>before<style>#a{color:red}</style>after</body>
        // <style> は HTML LS §15.3.1 "Hidden elements"、
        // その raw text (`#a{color:red}`) は paint されない。
        // before / after の text は painted (GlyphRun 2 個)。
        assert_inert_html_content_not_painted(
            b"<html><head></head><body>before<style>#a{color:red}</style>after</body></html>",
            "style",
        );
    }

    #[test]
    fn paint_single_page_skips_script_subtree_content() {
        // <body>before<script>alert(1)</script>after</body>
        // <script> raw text は paint されない (HTML LS §4.12.1)。
        assert_inert_html_content_not_painted(
            b"<html><head></head><body>before<script>alert(1)</script>after</body></html>",
            "script",
        );
    }

    #[test]
    fn paint_single_page_skips_noscript_subtree_content() {
        // <body>before<noscript>fallback</noscript>after</body>
        // scripting_enabled=true (html5ever default、parse.rs で default 継承)
        // 下では <noscript> 内容は raw text tokenize されるため paint 対象外。
        // 将来 scripting_enabled=false に切替えた場合は本 test を quarantine → 再設計。
        assert_inert_html_content_not_painted(
            b"<html><head></head><body>before<noscript>fallback</noscript>after</body></html>",
            "noscript",
        );
    }

    // raikiri-spike-s8w: §15.3.1 完全化 4 element (datalist / noembed /
    // noframes / rp)。d9y.5 の style / script / noscript / template fixture と
    // 同じ 3-way discriminant (`before` + `after` = 11 glyphs、inert 内 text
    // leak なら total_glyphs > 11) を継承する。
    //
    // 各 element の HTML5 parsing 挙動:
    // - `<datalist>`: 通常 element、内部 character token は Text 子として保持
    //   (HTML LS §4.10.8)。§15.3.1 UA `display:none` を override CSS 経路で
    //   剥がしても paint 側 predicate が subtree を落とす。
    // - `<noembed>` / `<noframes>`: "in body" 挿入モードで RAWTEXT tokenizer
    //   state へ遷移 (HTML LS §13.2.6.4.7)、内部 chunk は 1 Text 子として保持。
    // - `<rp>`: 通常 element parsing。`<ruby>` 外でも "in body" 挿入モード
    //   は rp を通常挿入する ("current node が ruby / rtc でない" の条件で
    //   parse error mark が付くのみで structure は維持、HTML LS §13.2.6.4.7)。
    //   §15.3.1 hidden-elements rule 直下で unconditionally `display: none`
    //   (§15.3.4 "Phrasing content" の ruby CSS も ruby / rt のみを扱い
    //   rp を可視化しない)。ruby 実装未搭載環境でも defense-in-depth で
    //   content-leak 経路を予防閉塞。

    #[test]
    fn paint_single_page_skips_datalist_subtree_content() {
        // <body>before<datalist>hidden</datalist>after</body>
        // <datalist> は §15.3.1 hidden-elements rule で `display: none`。
        // author override で hidden content が glyph に混入する経路を閉じる。
        assert_inert_html_content_not_painted(
            b"<html><head></head><body>before<datalist>hidden</datalist>after</body></html>",
            "datalist",
        );
    }

    #[test]
    fn paint_single_page_skips_noembed_subtree_content() {
        // <body>before<noembed>hidden</noembed>after</body>
        // <noembed> は RAWTEXT parsing (§13.2.6.4.7)、内部 "hidden" は raw text
        // として Text 子で保持され、§15.3.1 で display:none。
        assert_inert_html_content_not_painted(
            b"<html><head></head><body>before<noembed>hidden</noembed>after</body></html>",
            "noembed",
        );
    }

    #[test]
    fn paint_single_page_skips_noframes_subtree_content() {
        // <body>before<noframes>hidden</noframes>after</body>
        // <noframes> も §13.2.6.4.7 で RAWTEXT parsing、§15.3.1 hidden-elements。
        assert_inert_html_content_not_painted(
            b"<html><head></head><body>before<noframes>hidden</noframes>after</body></html>",
            "noframes",
        );
    }

    #[test]
    fn paint_single_page_skips_rp_subtree_content() {
        // <body>before<rp>hidden</rp>after</body>
        // <rp> は ruby parenthesis fallback。`<ruby>` 外でも "in body" 挿入
        // モードは rp を通常挿入する (§13.2.6.4.7 "current node が ruby / rtc
        // でない" 条件で parse error mark のみ付き structure は維持)。
        // §15.3.1 hidden-elements rule 直下で `display: none` (§15.3.4 ruby
        // CSS も ruby / rt のみ扱い rp を可視化しない)。ruby 未搭載環境でも
        // defense-in-depth で content-leak を閉じる。
        assert_inert_html_content_not_painted(
            b"<html><head></head><body>before<rp>hidden</rp>after</body></html>",
            "rp",
        );
    }

    #[test]
    fn paint_single_page_skips_template_subtree_content_via_inert_predicate() {
        // <template> は既に is_in_document() gate で skip されるが、defense-in-depth
        // で is_non_rendered_html_element() 側も個別に発火するかを pin。
        // "before<template>...</template>after" で 2 GlyphRun (before + after)。
        assert_inert_html_content_not_painted(
            b"<html><head></head><body>before<template><p>secret</p></template>after</body></html>",
            "template",
        );
    }

    #[test]
    fn paint_single_page_skips_template_subtree_without_display_none_ua_rule() {
        // raikiri-spike-37c: UA CSS の template { display: none } rule 存在に
        // 依存せず、template subtree の paint を is_in_document() predicate gate で
        // 明示的に skip する contract 回帰 pin。silent bug fix regression。
        //
        // Setup: <body><template><p>should_not_paint</p></template></body>。
        // 現在 UA CSS には template rule 無し (minimal.css 確認済)、default
        // Display::Block になる。gate 追加前は paint に降りて GlyphRun が emit
        // されていた silent bug 表面。
        use raikiri_html::{ParseOptions, parse};

        let html = b"<html><head></head><body>\
                     <template><p>should_not_paint</p></template>\
                     </body></html>";
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        };
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        // parse は UncascadedDocument を返す。dom を取り出して cascade + layout + paint。
        let mut doc = uncascaded.dom;
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

        let mut scene = Scene::new();
        paint_single_page(&mut scene, &doc, &cr, PageBox::A4);

        let glyph_commands: Vec<_> = scene
            .commands
            .iter()
            .filter(|c| matches!(c, RenderCommand::GlyphRun(_)))
            .collect();
        assert!(
            glyph_commands.is_empty(),
            "text inside <template> subtree should not paint (is_in_document gate), got {} glyph runs",
            glyph_commands.len()
        );
    }
}
