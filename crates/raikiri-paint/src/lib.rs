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
        // `"width: 0px; height: 0px"` を与え cv.{width,height} =
        // ComputedLengthPercentageOrAuto::Px(0.0) を bridge が翻訳するようにする。
        // **height は Wave 3 (raikiri-spike-01up) まで bridge されない**ため、
        // `zero_block_style.size.height = length(0.0)` の手構築値を **保持** して
        // sanity assert を維持する (Wave 3 landing 後は inline `height: 0px` が
        // bridge_size で反映されるので手構築値は冗長になる)。
        //
        // raikiri-spike-01up (Sprint 18 Wave 3) 以降: bridge_size が
        // `style.size = Size { width, height }` の struct literal を書くように
        // なり、height も bridge 対象になった。上記 `zero_block_style` の
        // `size.height = length(0.0)` 手構築値は inline `height: 0px` 由来の
        // 同値で上書きされるため現在は冗長 (behavior 差はなし)。
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
            // width / height とも bridge が clobber するため inline で明示
            // (height は raikiri-spike-01up で bridge 対象に加わった)。
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
        // Affine の translation 成分は as_coeffs() の `[4, 5]` (2 次元 identity +
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

/// bd raikiri-spike-3653 point 3 — empirical + source-verified characterization of
/// what the CPU rasterizer (`anyrender_vello_cpu` → `vello_cpu` → `glifo`, all
/// third-party, none of them Stylo) does when an extreme (`+Inf` or far beyond
/// the production clamp) glyph scale reaches
/// [`anyrender::PaintScene::draw_glyphs`].
///
/// # Why this bypasses `raikiri-dom`'s own guard
///
/// `font_size` here is passed to `draw_glyphs` directly, **bypassing**
/// `raikiri_dom::layout`'s `MAX_FONT_SIZE_PX` clamp (installed at the
/// `preshape_text` boundary by bd raikiri-spike-2ui0's site 5). Production
/// code always shapes through that guard first, so today this input is not
/// reachable via `raikiri_paint::paint_single_page`. The probe exists to
/// characterize the **rasterizer's own** robustness independent of whether an
/// upstream guard currently prevents the input — bd raikiri-spike-2ui0's own
/// framing was that "an input guard exists" is not the same claim as "every
/// downstream sink is safe if that guard is ever bypassed, loosened, or a new
/// call path is added that skips it".
///
/// # Finding: no hang, no panic, no OOM — measured, not assumed
///
/// The first hypothesis written for this module (before running it) was
/// wrong, and is worth recording as a caution against trusting source-reading
/// alone: `glifo-0.1.1/src/renderer.rs:591`'s `calculate_raster_metrics` does
/// contain an `i32` overflow (`bounds.x1.ceil() as i32 + 1`, which panics
/// under `overflow-checks = true` — Cargo's `dev`/`test` default, and this
/// workspace has no `[profile]` override) when a glyph's *scaled* bounding
/// box exceeds `i32::MAX`. Reading that function in isolation predicts a
/// panic for `f32::INFINITY`. **Running the probe below refutes that**: all
/// three font sizes below (the production clamp `1e6`, `i32::MAX as f32`, and
/// literal `f32::INFINITY`) complete normally, in well under the 30s bound,
/// every time.
///
/// The reason, traced after the empirical result forced a second look:
/// `glifo`'s glyph-atlas *caching* path (the one containing the overflowing
/// arithmetic) is gated by `GlyphCacheConfig::max_cached_font_size`, which
/// defaults to **128.0px** (`glifo-0.1.1/src/atlas/cache.rs:69`,
/// `glifo-0.1.1/src/glyph.rs:385`:
/// `draw_props.font_size <= config.max_cached_font_size`). Any `font_size`
/// above that — which includes the production clamp (`1e6`) and, a fortiori,
/// every value this probe tries — is *never* eligible for atlas caching in
/// the first place, so `calculate_raster_metrics` is never called for it.
/// Large glyphs instead take `glifo`'s uncached direct-fill path
/// (`fill_uncached_outline_glyph`, `glifo-0.1.1/src/renderer.rs:205-217`),
/// which does `renderer.set_transform(outline_transform.pre_scale(scale));
/// renderer.fill_path(path)` — i.e. it fills the glyph's `BezPath` straight
/// into the caller-sized destination canvas via `vello_cpu`'s tile/scanline
/// rasterizer, which only ever touches pixels inside that destination
/// surface. No intermediate buffer sized to the glyph's (possibly huge or
/// infinite) *scaled* extent is ever allocated on this path — allocation is
/// bounded by the caller-chosen canvas size (`CANVAS_W * CANVAS_H * 4` here),
/// not by `font_size`.
///
/// (Separately, even glyphs that *do* stay under the 128px cache threshold
/// can't reach the `i32` overflow above through a realistic font: for a
/// typical 1000..2048-upem font, a scaled bbox only approaches `i32::MAX`
/// around `font_size` ~1e9, many orders of magnitude past the point where
/// `max_cached_font_size` has already routed the glyph to the uncached path.
/// The overflow looks structurally unreachable via `draw_glyphs`, though this
/// module does not exhaustively prove that for every font/transform
/// combination — see the residual-risk note below.)
///
/// **Net conclusion**: no OOM, hang, or panic path was found in the CPU
/// rasterizer for an extreme/`+Inf` scale reaching `draw_glyphs`, for the
/// translate-only-transform / no-`glyph_transform` calling convention
/// `raikiri_paint::text::draw_text_node` actually uses (this probe matches it
/// exactly). This is a *stronger* result than bd raikiri-spike-2ui0's
/// "PLAUSIBLE, uncharacterized" framing assumed was likely — the "glyph bbox
/// 比例の scanline buffer" failure shape that motivated point 3 does not
/// exist in this backend. Every extreme-`font_size` case also draws **zero**
/// visible ink into the probe canvas, which by itself would be uninformative
/// (indistinguishable from "this call never reaches rasterization at all")
/// were it not for
/// [`nonfinite_rasterizer_probe::draw_glyphs_at_natural_font_size_is_a_non_vacuous_control`]
/// pinning that the identical font/glyphs/canvas/transform *does* draw when
/// `font_size` is ordinary — isolating `font_size` as the one variable that
/// silences the output, rather than the harness being unable to draw at all.
///
/// # Residual risk (not this module's job to close)
///
/// This probe exercises exactly one glyph, one font, one transform shape, and
/// one backend (`anyrender_vello_cpu`/CPU). It does not prove every backend
/// (`anyrender_vello`/GPU, `anyrender_vello_hybrid`) or every transform shape
/// (rotation/skew, which *would* route through a different `glifo` code path
/// per `supports_atlas_caching`'s skew check) is equally safe — raikiri does
/// not use those today (`paint_single_page`'s doc lists "CSS transform" as
/// M4+ non-goal, and the VRT/production backend is CPU-only per
/// `raikiri::html_to_png`), so they're out of this probe's scope, not
/// asserted safe.
#[cfg(test)]
mod nonfinite_rasterizer_probe {
    use std::panic::AssertUnwindSafe;
    use std::sync::mpsc::{self, RecvTimeoutError};
    use std::time::Duration;

    use anyrender::{Glyph as AnyrenderGlyph, PaintScene};
    use anyrender_vello_cpu::VelloCpuImageRenderer;
    use kurbo::{Affine, Vec2};
    use parley::{FontContext, PositionedLayoutItem};
    use peniko::{Color, Fill, FontData};
    use raikiri_dom::{Document, layout_single_page};
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;
    use taffy::Style;

    /// Deliberately tiny (`"Hi"` at the CSS-initial 16px comfortably fits and
    /// inks it — see
    /// [`draw_glyphs_at_natural_font_size_is_a_non_vacuous_control`], which
    /// pins that this canvas size is not itself the reason later tests draw
    /// nothing) — every extreme-`font_size` case is expected to leave it
    /// blank (a glyph many orders of magnitude larger than 16px does not
    /// happen to place any of its ink inside a 16x16 window at this
    /// position), which this module treats as an informative finding, not a
    /// probe defect, precisely because the control above proves the harness
    /// can draw when the input is ordinary.
    const CANVAS_W: u32 = 16;
    const CANVAS_H: u32 = 16;

    /// Shapes a real "Hi" paragraph through the normal (guarded)
    /// `layout_single_page` pipeline and extracts the resulting `GlyphRun`'s
    /// real `FontData` + `normalized_coords` + positioned glyphs (real glyph
    /// ids with non-degenerate bounding boxes — a synthetic all-zero glyph
    /// would turn `+Inf * 0` into `NaN`, which saturates to `0` on
    /// `as i32` and would *not* reproduce the overflow this probe measures)
    /// — plus the run's own natural `font_size` (the CSS initial `16px`,
    /// unclamped since nothing overrode it here), returned so the control
    /// test below can draw at the size the glyphs were actually shaped for.
    fn real_glyph_run_parts() -> (FontData, Vec<i16>, Vec<AnyrenderGlyph>, f32) {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let p = doc.append_element(Some(body), "p", Style::default(), None::<&str>);
        let text_id = doc.append_text(p, "Hi");
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

        let layout = doc
            .get_node(text_id)
            .expect("text node must exist (just appended it)")
            .text_layout()
            .expect("preshape_text must populate text_layout for a non-empty Text node");

        // cov:ignore: this block's `return` on match means its closing
        // braces are never "reached" as a fallthrough line once the first
        // GlyphRun is found (which it always is, for "Hi") — and the
        // assert!'s message is only executed on failure, which doesn't
        // happen while this test passes.
        for line in layout.lines() {
            for item in line.items() {
                if let PositionedLayoutItem::GlyphRun(glyph_run) = item {
                    let run = glyph_run.run();
                    let font = run.font().clone();
                    let natural_font_size = run.font_size();
                    let coords = run.normalized_coords().to_vec();
                    let glyphs: Vec<AnyrenderGlyph> = glyph_run
                        .positioned_glyphs()
                        .map(|g| AnyrenderGlyph {
                            id: g.id,
                            x: g.x,
                            y: g.y,
                        })
                        .collect();
                    assert!(
                        !glyphs.is_empty(),
                        "sanity: hello-world 'Hi' must shape to >= 1 glyph"
                    );
                    return (font, coords, glyphs, natural_font_size);
                }
            }
        }
        // cov:ignore: this panic is a setup-sanity fallback for a code path
        // ("Hi" produces no GlyphRun) that doesn't occur while the test
        // suite's font/shaping setup is intact.
        panic!(
            "sanity: hello-world 'Hi' produced no GlyphRun — setup is broken, not a rasterizer finding"
        );
    }

    /// Runs `draw_glyphs` against a fresh `VelloCpuImageRenderer` canvas and
    /// reports both the raw buffer length and how many bytes in it are
    /// non-zero (i.e. whether anything was actually drawn). Shared by the
    /// control test and the bounded extreme-value probe below so both use
    /// the exact same canvas geometry / transform — the only variable
    /// between them is `font_size`.
    fn render_glyphs_unbounded(
        font: &FontData,
        coords: &[i16],
        glyphs: &[AnyrenderGlyph],
        font_size: f32,
    ) -> (usize, usize) {
        let buf = anyrender::render_to_buffer::<VelloCpuImageRenderer, _>(
            |scene| {
                scene.draw_glyphs(
                    font,
                    font_size,
                    true, // hint
                    coords,
                    Vec2::ZERO, // embolden
                    Fill::NonZero,
                    Color::BLACK,
                    1.0,              // brush_alpha
                    Affine::IDENTITY, // == Affine::translate((0.0, 0.0)); parley's
                    // `positioned_glyphs()` already bakes line offset + baseline into
                    // `g.x`/`g.y` relative to a top-left (0,0) origin (see
                    // `crates/raikiri-paint/src/text.rs` module doc), same convention
                    // `draw_text_node` uses for its own `base_transform`.
                    None, // glyph_transform
                    glyphs.iter().copied(),
                );
            },
            CANVAS_W,
            CANVAS_H,
        );
        let nonzero = buf.iter().filter(|&&b| b != 0).count();
        (buf.len(), nonzero)
    }

    /// Non-vacuity control: **without any extreme input**, does this exact
    /// harness (font, glyphs, `CANVAS_W`×`CANVAS_H` canvas, `Affine::IDENTITY`
    /// transform) actually put visible ink on the canvas? Drawn at the run's
    /// own natural (unmodified, un-overridden) font-size — i.e. this is the
    /// one call in this module that does not touch `font_size` at all.
    ///
    /// This exists because a probe that draws nothing at *every* input
    /// (including a normal one) cannot distinguish "the rasterizer is robust
    /// to extreme scale" from "this call configuration never reaches the
    /// rasterization code path in the first place" — the three extreme-value
    /// tests below all report zero non-zero bytes, and without this control
    /// that would be uninformative rather than a finding. With it: the exact
    /// same font/glyphs/canvas/transform genuinely draws (this test), and
    /// then genuinely stops drawing once `font_size` crosses into extreme
    /// territory (the other tests) — isolating `font_size` as the one
    /// variable that changed.
    #[test]
    fn draw_glyphs_at_natural_font_size_is_a_non_vacuous_control() {
        let (font, coords, glyphs, natural_font_size) = real_glyph_run_parts();
        let (buf_len, nonzero) =
            render_glyphs_unbounded(&font, &coords, &glyphs, natural_font_size);
        assert_eq!(buf_len, (CANVAS_W * CANVAS_H * 4) as usize);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            nonzero > 0,
            "control: draw_glyphs(font_size={natural_font_size}) — the run's own natural size, no override — drew zero non-zero bytes into a {CANVAS_W}x{CANVAS_H} canvas at Affine::IDENTITY. If this control ever fails, the extreme-value tests below are vacuous (they'd pass whether or not the rasterizer is actually robust) and bd raikiri-spike-3653 point 3's conclusion is unsupported until this is fixed — do not just delete the assertion"
        );
    }

    /// Outcome of a bounded `draw_glyphs` probe call.
    enum ProbeOutcome {
        /// Rasterization returned a buffer (whether or not the glyph was
        /// actually drawn into it — `buf_len` is the raw byte count, always
        /// `CANVAS_W * CANVAS_H * 4` regardless of whether the atlas rejected
        /// the glyph, since `render_to_buffer` always allocates the *canvas*
        /// up front; `nonzero_bytes` is how many of those bytes are non-zero,
        /// i.e. whether anything was actually drawn — see
        /// [`draw_glyphs_at_natural_font_size_is_a_non_vacuous_control`] for
        /// why this field matters, not just `buf_len`).
        Completed {
            buf_len: usize,
            nonzero_bytes: usize,
        },
        /// The worker thread panicked (`catch_unwind` caught it directly, or
        /// the channel disconnected because the panic unwound past the
        /// `tx.send` — both are reported the same way since either means
        /// "did not complete normally", the distinction the caller cares
        /// about is panic vs. hang, not which detection path fired).
        Panicked,
    }

    /// worker thread + `recv_timeout` — the same bounding pattern
    /// `crates/raikiri-dom/src/layout.rs`'s
    /// `nonfinite_font_size_is_clamped_before_parley` (bd raikiri-spike-2ui0
    /// site 5) uses for parley. Necessary here for the same reason: if the
    /// rasterizer *did* hang, an un-bounded `cargo test` run would eat the
    /// CI job timeout and take the rest of the test binary's results with it
    /// (indistinguishable from infra flake). A caught panic is fine to run
    /// un-bounded (it returns immediately) but is routed through the same
    /// worker so one probe function handles both failure shapes.
    fn draw_glyphs_bounded(font_size: f32) -> ProbeOutcome {
        let (font, coords, glyphs, _natural_font_size) = real_glyph_run_parts();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
                render_glyphs_unbounded(&font, &coords, &glyphs, font_size)
            }));
            // Send unconditionally when we get here at all — if `catch_unwind`
            // itself failed to catch (shouldn't happen; the closure has no
            // `extern "C"` boundary or `panic = "abort"` in this workspace's
            // profile) the thread would already be gone and `tx` dropped,
            // which the `Disconnected` arm below handles.
            let _ = tx.send(result);
        });
        // cov:ignore: every call site of this helper completes well within
        // the 30s bound (that's the finding this module characterizes) —
        // the Timeout panic arm is a diagnostic for a hang this module found
        // does not occur, and Disconnected mirrors the same never-taken
        // defensive shape as the layout.rs/lib.rs siblings of this pattern.
        match rx.recv_timeout(Duration::from_secs(30)) {
            Ok(Ok((buf_len, nonzero_bytes))) => ProbeOutcome::Completed {
                buf_len,
                nonzero_bytes,
            },
            Ok(Err(_panic_payload)) => ProbeOutcome::Panicked,
            Err(RecvTimeoutError::Timeout) => panic!(
                "draw_glyphs(font_size={font_size}) did not return within 30s — possible CPU rasterizer hang (bd raikiri-spike-3653 point 3); note the worker thread is leaked (not joined) on this path, same caveat as the parley bound in layout.rs"
            ),
            Err(RecvTimeoutError::Disconnected) => ProbeOutcome::Panicked,
        }
    }

    /// The headline finding: literal `+Inf` font-size reaching `draw_glyphs`
    /// completes normally — no panic, no hang, the returned buffer is
    /// exactly the caller-requested canvas size (not something proportional
    /// to the infinite glyph scale), and — unlike the vacuous first draft of
    /// this test — it draws **zero** visible ink, in contrast to
    /// [`draw_glyphs_at_natural_font_size_is_a_non_vacuous_control`] drawing
    /// non-zero ink through the identical canvas/transform/font/glyphs. See
    /// this module's doc comment for the full mechanism (the
    /// `max_cached_font_size` gate) and source citations.
    ///
    /// This is the module's load-bearing regression pin: if a future
    /// `glifo`/`vello_cpu` upgrade changes this dispatch (e.g. raises/removes
    /// the atlas-caching font-size gate, or the uncached fill path stops
    /// being canvas-bounded), this test flips to `Panicked` or a timeout and
    /// must be re-characterized rather than silently left describing stale
    /// behavior — do not weaken this to
    /// `assert!(matches!(outcome, Panicked | Completed { .. }))`.
    #[test]
    fn draw_glyphs_with_inf_font_size_completes_within_bounded_canvas_sized_buffer() {
        // cov:ignore: this test's whole point is that the Completed arm is
        // always taken (no panic) — the Panicked arm's message is a
        // diagnostic for the failure mode this module found does not occur.
        match draw_glyphs_bounded(f32::INFINITY) {
            ProbeOutcome::Completed {
                buf_len,
                nonzero_bytes,
            } => {
                assert_eq!(
                    buf_len,
                    (CANVAS_W * CANVAS_H * 4) as usize,
                    "draw_glyphs(f32::INFINITY) must allocate exactly the caller-sized canvas buffer, not something proportional to glyph scale"
                );
                assert_eq!(
                    nonzero_bytes, 0,
                    "draw_glyphs(f32::INFINITY) drew {nonzero_bytes} non-zero bytes — this module's characterization was 'completes but draws nothing'; if this now draws *something*, that's a different (not necessarily worse) finding and needs its own re-characterization, not silent acceptance"
                );
            }
            ProbeOutcome::Panicked => panic!(
                "draw_glyphs(f32::INFINITY) panicked — this module's characterization (uncached direct-fill path is used above the 128px glifo atlas-cache threshold, and it's canvas-bounded, not glyph-scale-bounded) no longer holds; re-characterize bd raikiri-spike-3653 point 3 rather than deleting this test"
            ),
        }
    }

    /// Same probe, but with a merely-huge *finite* font-size
    /// (`i32::MAX as f32`, ~2.1e9) rather than literal `+Inf` — confirms the
    /// no-crash result isn't an IEEE-infinity special case somewhere in the
    /// stack; ordinary huge finite values take the same safe path. Still
    /// ~2100x above the production clamp (`MAX_FONT_SIZE_PX = 1e6`,
    /// `crates/raikiri-dom/src/layout.rs`), i.e. still outside what the
    /// guarded pipeline can ever produce.
    #[test]
    fn draw_glyphs_with_huge_finite_font_size_also_completes_normally() {
        // cov:ignore: this test's whole point is that the Completed arm is
        // always taken (no panic) — the Panicked arm's message is a
        // diagnostic for the failure mode this module found does not occur.
        match draw_glyphs_bounded(i32::MAX as f32) {
            ProbeOutcome::Completed {
                buf_len,
                nonzero_bytes,
            } => {
                assert_eq!(
                    buf_len,
                    (CANVAS_W * CANVAS_H * 4) as usize,
                    "draw_glyphs(i32::MAX as f32) must allocate exactly the caller-sized canvas buffer, not something proportional to glyph scale"
                );
                assert_eq!(
                    nonzero_bytes, 0,
                    "draw_glyphs(i32::MAX as f32) drew {nonzero_bytes} non-zero bytes — re-characterize bd raikiri-spike-3653 point 3, see module doc"
                );
            }
            ProbeOutcome::Panicked => panic!(
                "draw_glyphs(i32::MAX as f32) panicked — re-characterize bd raikiri-spike-3653 point 3, see module doc"
            ),
        }
    }

    /// Sanity / headroom check at the production clamp boundary itself
    /// (`MAX_FONT_SIZE_PX = 1e6`, `crates/raikiri-dom/src/layout.rs`) — this
    /// value must also complete normally (no panic, no timeout), confirming
    /// the existing bd raikiri-spike-2ui0 guard's chosen bound lands
    /// comfortably inside the region this module found safe (in fact, `1e6`
    /// is already ~7800x past `glifo`'s own 128px atlas-cache-eligibility
    /// cutoff, so production text has *never* exercised that caching path
    /// for any remotely large heading/display font-size — not just for
    /// pathological input). Not a new guard — a pin that the current guard's
    /// chosen bound is also safe for this previously-uncharacterized sink.
    /// Like the two tests above (and unlike the natural-size control), a
    /// production-clamp-sized glyph is *also* far too large to put visible
    /// ink on a `CANVAS_W`×`CANVAS_H` probe canvas, so this asserts
    /// `nonzero_bytes == 0` too — that's expected here, not a regression.
    #[test]
    fn draw_glyphs_at_production_font_size_clamp_completes_normally() {
        const MAX_FONT_SIZE_PX: f32 = 1e6; // mirrors raikiri-dom's private const; see doc above
        // cov:ignore: this test's whole point is that the Completed arm is
        // always taken (no panic) — the Panicked arm's message is a
        // diagnostic for the failure mode this module found does not occur.
        match draw_glyphs_bounded(MAX_FONT_SIZE_PX) {
            ProbeOutcome::Completed {
                buf_len,
                nonzero_bytes,
            } => {
                assert_eq!(
                    buf_len,
                    (CANVAS_W * CANVAS_H * 4) as usize,
                    "render_to_buffer must return exactly width*height*4 bytes"
                );
                assert_eq!(
                    nonzero_bytes, 0,
                    "draw_glyphs(MAX_FONT_SIZE_PX = 1e6) drew {nonzero_bytes} non-zero bytes into a {CANVAS_W}x{CANVAS_H} probe canvas — expected zero (the glyph is far larger than the probe canvas at this size); if this changed, it's not itself a problem but re-check the module doc's characterization still holds"
                );
            }
            ProbeOutcome::Panicked => panic!(
                "draw_glyphs(MAX_FONT_SIZE_PX = 1e6) panicked — this would mean bd raikiri-spike-2ui0's guard has a residual gap against this specific sink, escalate rather than widen this test's expectation"
            ),
        }
    }
}
