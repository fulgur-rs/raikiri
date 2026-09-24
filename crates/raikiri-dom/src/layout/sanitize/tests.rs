use super::*;
use taffy::Style;

fn embedded_ic_font_dir() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().expect("temporary embedded ic font directory");
    for (name, bytes) in [
        (
            "Ahem.ttf",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/data/text-autospace/Ahem.ttf"
            )) as &[u8],
        ),
        (
            "CanvasTest-nospace.ttf",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/data/text-autospace/CanvasTest-nospace.ttf"
            )) as &[u8],
        ),
    ] {
        std::fs::write(tmp.path().join(name), bytes).expect("write embedded font");
    }
    for (name, bytes) in [
        (
            "ZeroWidth",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/data/text-autospace/IcTestZeroWidth.woff2"
            )) as &[u8],
        ),
        (
            "HalfWidth",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/data/text-autospace/IcTestHalfWidth.woff2"
            )) as &[u8],
        ),
        (
            "FullWidth",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/data/text-autospace/IcTestFullWidth.woff2"
            )) as &[u8],
        ),
    ] {
        let decoded = wuff::decompress_woff2(bytes).expect("decode embedded ic fixture");
        std::fs::write(tmp.path().join(format!("IcTest{name}.ttf")), decoded)
            .expect("write decoded embedded ic fixture");
    }
    tmp
}

#[test]
fn collapse_single_node_break_becomes_space_and_lone_wide_break_drops() {
    use raikiri_style::{build_rule_tree, cascade};
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(Some(body), "p", Style::default(), None::<&str>);
    // Single-node wide neighbors also follow the segment-break rule;
    // this is handled before the generic whitespace merge.
    let t = doc.append_text(p, "\u{FF24}\u{FF26}\n\u{FF24}\u{FF26}");
    // Split nodes around a lone break: wide/fullwidth neighbors drop
    // it (CSS Text 3 §4.1.2; WPT rules-001), narrow neighbors migrate
    // a space (rules-004 shape).
    let q = doc.append_element(Some(body), "p", Style::default(), None::<&str>);
    let w1 = doc.append_text(q, "\u{FF24}");
    let wb = doc.append_text(q, "\n");
    let w2 = doc.append_text(q, "\u{FF24}");
    let r = doc.append_element(Some(body), "p", Style::default(), None::<&str>);
    let n1 = doc.append_text(r, "a");
    let nb = doc.append_text(r, "\n");
    let n2 = doc.append_text(r, "b");
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    let mut parent_of: Vec<Option<usize>> = vec![None; doc.nodes.len()];
    for idx in 0..doc.nodes.len() {
        for &c in &doc.nodes[idx].children.clone() {
            if c < parent_of.len() {
                parent_of[c] = Some(idx);
            }
        }
    }
    let collapse = |idx: usize, text: &str| {
        collapse_text_for_shaping(
            &doc,
            &cr,
            &parent_of,
            idx,
            text,
            cr.computed[idx].white_space,
        )
    };
    let out = collapse(t, "\u{FF24}\u{FF26}\n\u{FF24}\u{FF26}");
    assert_eq!(out.text, "\u{FF24}\u{FF26}\u{FF24}\u{FF26}");
    assert_eq!(out.migrate_count, 0);
    let out = collapse(wb, "\n");
    assert_eq!(out.text, "");
    assert_eq!(out.migrate_count, 0);
    let out = collapse(nb, "\n");
    assert_eq!(out.text, "");
    assert_eq!(out.migrate_count, 1);
    let _ = (w1, w2, n1, n2);
}

#[test]
fn multicol_definite_dimension_resolves_calc_via_the_taffy_calc_resolver() {
    let mut doc = Document::new();
    doc.calc_values
        .push(std::sync::Arc::new(CalcLengthPercentage {
            percent: 50.0,
            px: 10.0,
        }));
    let pointer = doc
        .calc_values
        .last()
        .map(|value| (&**value) as *const _ as *const ())
        .expect("calc value was just pushed");
    assert_eq!(
        multicol_definite_dimension(&doc, Dimension::calc(pointer), Some(200.0)),
        Some(110.0)
    );
}

#[test]
fn text_align_center_offsets_glyphs_to_container_middle() {
    // `text-align: center` の最小 regression check:
    // 単独 Text の block container (`<p>` + 1 Text は minimal line box の
    // 2-child threshold 未満のため plain block path) で、glyph run の
    // 先頭 x が container 幅の中央付近に寄ること。
    use parley::{FontContext, PositionedLayoutItem};
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(
        Some(body),
        "p",
        Style::default(),
        Some("display: block; text-align: center"),
    );
    let t = doc.append_text(p, "Hello");

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    let p_width = doc.nodes[p].unrounded_layout.size.width;
    let layout = doc.nodes[t].text_layout().expect("text shaped");
    let first_x: f32 = layout
        .lines()
        .next()
        .expect("one line")
        .items()
        .filter_map(|it| match it {
            PositionedLayoutItem::GlyphRun(gr) => gr.positioned_glyphs().next().map(|g| g.x),
            _ => None,
        })
        .next()
        .expect("glyph");
    let text_w = layout.width();
    let expected = (p_width - text_w) * 0.5;
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (first_x - expected).abs() < 2.0,
        "centered glyph x={} must be near (container-text)/2={} (p_w={} text_w={})",
        first_x,
        expected,
        p_width,
        text_w
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        first_x > 10.0,
        "centered text must not sit at the left edge, got x={}",
        first_x
    );
}

#[test]
fn text_indent_px_offsets_first_line() {
    // `text-indent` 基本配線の regression check:
    // 単独 Text の block container で first line の先頭 x が indent 分
    // 右に寄ること。parley `set_text_indent` 経由 (basic のみ —
    // hanging/each-line は parse 層 drop のため常に default)。
    use parley::{FontContext, PositionedLayoutItem};
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(
        Some(body),
        "p",
        Style::default(),
        Some("display: block; text-indent: 20px"),
    );
    let t = doc.append_text(p, "Hello");

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    let layout = doc.nodes[t].text_layout().expect("text shaped");
    let first_x: f32 = layout
        .lines()
        .next()
        .expect("one line")
        .items()
        .filter_map(|it| match it {
            PositionedLayoutItem::GlyphRun(gr) => gr.positioned_glyphs().next().map(|g| g.x),
            _ => None,
        })
        .next()
        .expect("glyph");
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (first_x - 20.0).abs() < 2.0,
        "indented first glyph x={} must be near indent 20px",
        first_x
    );
}

#[test]
fn text_indent_amount_bounds_nonfinite_values() {
    assert_eq!(
        bounded_text_indent_amount(ComputedLengthPercentage::Px(f32::INFINITY), 100.0, None,),
        MAX_TAFFY_MAGNITUDE
    );
    assert_eq!(
        bounded_text_indent_amount(ComputedLengthPercentage::Px(f32::NEG_INFINITY), 100.0, None,),
        -MAX_TAFFY_MAGNITUDE
    );
    assert_eq!(
        bounded_text_indent_amount(ComputedLengthPercentage::Px(f32::NAN), 100.0, None),
        0.0
    );
    assert_eq!(
        bounded_text_indent_amount(
            ComputedLengthPercentage::Px(1.0),
            100.0,
            Some(MAX_TAFFY_MAGNITUDE * 2.0),
        ),
        MAX_TAFFY_MAGNITUDE
    );
}

#[test]
fn text_indent_negative_protrudes_before_box() {
    // negative indent は box 始端より前に張り出す (CSS Text 3 §8.1)。
    use parley::{FontContext, PositionedLayoutItem};
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(
        Some(body),
        "p",
        Style::default(),
        Some("display: block; margin-left: 20px; text-indent: -20px"),
    );
    let t = doc.append_text(p, "Hello");

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    let layout = doc.nodes[t].text_layout().expect("text shaped");
    let first_x: f32 = layout
        .lines()
        .next()
        .expect("one line")
        .items()
        .filter_map(|it| match it {
            PositionedLayoutItem::GlyphRun(gr) => gr.positioned_glyphs().next().map(|g| g.x),
            _ => None,
        })
        .next()
        .expect("glyph");
    // glyph x は layout-local で -20 (paint が box origin x=20 に
    // 足して最終 x=0 になる)。
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (first_x + 20.0).abs() < 2.0,
        "negative-indented first glyph run-local x={} must be near -20",
        first_x
    );
}

#[test]
fn text_indent_zero_leaves_first_line_at_edge() {
    // indent 無しは preshape のまま (realign の indent 経路を通らない)。
    use parley::{FontContext, PositionedLayoutItem};
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(Some(body), "p", Style::default(), Some("display: block"));
    let t = doc.append_text(p, "Hello");

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    let layout = doc.nodes[t].text_layout().expect("text shaped");
    let first_x: f32 = layout
        .lines()
        .next()
        .expect("one line")
        .items()
        .filter_map(|it| match it {
            PositionedLayoutItem::GlyphRun(gr) => gr.positioned_glyphs().next().map(|g| g.x),
            _ => None,
        })
        .next()
        .expect("glyph");
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        first_x.abs() < 2.0,
        "unindented first glyph x={} must be near the left edge",
        first_x
    );
}

#[test]
fn tab_replacement_without_tabs_preserves_text() {
    let (text, ranges, _) = replace_tabs_with_styled_spaces("plain", 20.0, 10.0, |_| 10.0);
    assert_eq!(text, "plain");
    assert!(ranges.is_empty());
}

#[test]
fn tab_replacement_measures_prefix_and_ranges_the_gap() {
    let (text, ranges, _) =
        replace_tabs_with_styled_spaces("ab\tc", 20.0, 10.0, |segment| segment.len() as f32 * 6.0);
    assert_eq!(text, "ab c");
    assert_eq!(ranges, vec![(2..3, -2.0)]);
}

#[test]
fn tab_replacement_resets_its_cursor_after_newline() {
    let (text, ranges, _) = replace_tabs_with_styled_spaces("ab\n\tc", 20.0, 10.0, |segment| {
        segment.len() as f32 * 6.0
    });
    assert_eq!(text, "ab\n c");
    assert_eq!(ranges, vec![(3..4, 10.0)]);
}

#[test]
fn tab_replacement_with_zero_interval_removes_tabs() {
    let (text, ranges, _) = replace_tabs_with_styled_spaces("a\tb", 0.0, 10.0, |_| 10.0);
    assert_eq!(text, "ab");
    assert!(ranges.is_empty());
}

#[test]
#[ignore] // 明示的に cargo test -- --ignored で実行
fn font_context_new_cost_is_reasonable() {
    let start = std::time::Instant::now();
    for _ in 0..10 {
        let _ = parley::FontContext::new();
    }
    let elapsed = start.elapsed();
    // 10 回 total で 5 秒未満なら現行実装の per-call new() は許容
    // (10 連ラン determinism test が timeout しないため)
    assert!(
        elapsed.as_secs() < 5,
        "FontContext::new() too slow: 10x = {:?}",
        elapsed
    );
}

// ── 非有限 f32 guard ────────
//
// untrusted author CSS から +Inf / NaN が taffy / parley に到達しないことを
// **5 site すべて**で check する。reproducer は元の probe comment 由来。
//
// 期待値は「非有限でない」ではなく **clamp 後の具体値** で書く — NaN は
// `NaN != NaN` なので `assert_ne!(x, ...NAN)` は無条件に pass してしまい
// guard の有無を判別できない。

/// cascade → `apply_computed_to_style` を通した後の対象 element の
/// `taffy::Style` を返す。
///
/// fixture は **非 body element** (`<p>`) — `<body>` は後段
/// `apply_page_box_to_body` で size を clobber されるため。
fn guarded_style_for(inline: &str) -> taffy::Style {
    use raikiri_style::{build_rule_tree, cascade};
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(Some(body), "p", Style::default(), Some(inline));
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    apply_computed_to_style(&mut doc, &cr);
    doc.nodes[p].style.clone()
}

/// site 1 — `computed_length_percentage_to_taffy_length_percentage` (padding)。
#[test]
fn nonfinite_padding_is_clamped_before_taffy() {
    use taffy::LengthPercentage;

    // Reproducer A': `1e40px` は cssparser の f64→f32 変換で +Inf になり、
    // `parse_padding_side` の `v >= 0.0` を **通過する** (inf >= 0.0 は true)。
    assert_eq!(
        guarded_style_for("padding-top: 1e40px").padding.top,
        LengthPercentage::length(MAX_TAFFY_MAGNITUDE),
    );

    // Reproducer A: IEEE 754 `0.0 * inf = NaN` — em の乗算で NaN が生まれる。
    // かつては `Em(_) => length(0.0)` arm がこれを吸収していた。
    assert_eq!(
        guarded_style_for("font-size: 0px; padding-top: 1e40em")
            .padding
            .top,
        LengthPercentage::length(0.0),
        "NaN は clamp では潰れないので is_nan() → 0.0 で処理する",
    );

    // percentage 側 (`Percent` arm) も同じ guard を通す。
    // (`1e40%` は raikiri の `parse_percentage` が cssparser の unit_value
    //  1e38 を `* 100.0` して +Inf にする — 実測。)
    assert_eq!(
        guarded_style_for("padding-top: 1e40%").padding.top,
        LengthPercentage::percent(MAX_TAFFY_MAGNITUDE),
    );

    // **有限だが巨大**な値も clamp する。上の 3 case はすべて f32 で既に
    // 非有限 (`1e40` は f32 で +Inf) なので、実装を
    // `if v.is_finite() { v } else { ... }` に「簡素化」しても全部 pass して
    // しまう。`1e38%` は `Percent(1e38)` = **有限** (実測) で fraction は
    // 1e36 になるため、この 1 本だけがその簡素化を殺す。
    assert_eq!(
        guarded_style_for("padding-top: 1e38%").padding.top,
        LengthPercentage::percent(MAX_TAFFY_MAGNITUDE),
        "有限だが巨大な percentage も clamp する (is_finite() だけの実装への regression guard)",
    );
}

/// site 2 — `computed_length_percentage_or_auto_to_taffy_dimension` (width / height)。
#[test]
fn nonfinite_size_is_clamped_before_taffy() {
    use taffy::Dimension;
    let s = guarded_style_for("width: 1e40px; height: 1e40%");
    assert_eq!(s.size.width, Dimension::length(MAX_TAFFY_MAGNITUDE));
    assert_eq!(s.size.height, Dimension::percent(MAX_TAFFY_MAGNITUDE));

    // NaN 経路 (em × font-size 0)。
    let n = guarded_style_for("font-size: 0px; width: 1e40em");
    assert_eq!(n.size.width, Dimension::length(0.0));
}

/// site 3 — `computed_length_percentage_or_auto_to_taffy_length_percentage_auto` (margin)。
///
/// margin は **負値が spec-valid** (CSS Box 3 §3.1) なので clamp は対称
/// (`[-MAX, MAX]`) でなければならない。
#[test]
fn nonfinite_margin_is_clamped_symmetrically_before_taffy() {
    use taffy::LengthPercentageAuto;
    assert_eq!(
        guarded_style_for("margin-top: 1e40px").margin.top,
        LengthPercentageAuto::length(MAX_TAFFY_MAGNITUDE),
    );
    assert_eq!(
        guarded_style_for("margin-top: -1e40px").margin.top,
        LengthPercentageAuto::length(-MAX_TAFFY_MAGNITUDE),
        "負の margin は spec-valid なので -MAX 側に clamp する (0 に潰さない)",
    );
    assert_eq!(
        guarded_style_for("font-size: 0px; margin-top: 1e40em")
            .margin
            .top,
        LengthPercentageAuto::length(0.0),
    );
    // `Percent` の負値経路 (`parse_margin_side` は allow-negative なので
    // `-1e40%` が parse を通り `Percent(-inf)` になる — 実測)。
    // `Px` 側だけだと `Percent` arm から `sanitize_taffy` を外す変更が
    // test を素通りする。
    assert_eq!(
        guarded_style_for("margin-left: -1e40%").margin.left,
        LengthPercentageAuto::percent(-MAX_TAFFY_MAGNITUDE),
    );
}

/// site 4 — `computed_length_to_taffy_length_percentage` (border-width)。
#[test]
fn nonfinite_border_width_is_clamped_before_taffy() {
    use taffy::LengthPercentage;
    assert_eq!(
        guarded_style_for("border-top-width: 1e40px; border-top-style: solid")
            .border
            .top,
        LengthPercentage::length(MAX_TAFFY_MAGNITUDE),
    );
    assert_eq!(
        guarded_style_for("font-size: 0px; border-top-width: 1e40em; border-top-style: solid")
            .border
            .top,
        LengthPercentage::length(0.0),
    );
}

/// site 5 — `preshape_text` の `cv.font_size.px()` → parley
/// `StyleProperty::FontSize`。
///
/// 観測は shape 後の `Layout::height()` — font-size が非有限なら line metrics
/// が汚染されて height も非有限になる。
///
/// # guard を外すと fail ではなく **hang** する
///
/// 実測 (`sanitize_finite` を恒等関数に差し替えて単独実行): site 1-4 は即座に
/// assert 失敗するが、本 site は 25 秒経っても終了しない。機構は
/// `parley-0.10.0/src/layout/line_break.rs` の `if next_x <= max_advance` が
/// `next_x = inf` で恒偽になり、`while self.break_next().is_some() {}` が
/// 前進しないこと (shaping 自体は完了しており spin するのは `break_all_lines`)。
///
/// そのため本 test は **worker thread + `recv_timeout` で有界化**してある —
/// guard が消えた場合に「CI job が 20 分で殺される」(infra flake と区別
/// できず、同一 binary の後続 test の結果も失われる) ではなく
/// **assert failure** として落ちる。
#[test]
fn nonfinite_font_size_is_clamped_before_parley() {
    // 親 / 子の inline style を分けて渡す — `font-size` の `em` は **親**の
    // computed font-size 基準 (CSS Values 4 §6.1.1) なので、NaN (`0 * inf`)
    // を作るには乗数 `font-size: 0px` が親側に載っている必要がある。
    // site 1-4 は乗数が同一 element に載るので 1 element で作れるが、
    // font-size だけは 2 element 要る。
    fn shaped_height(parent_inline: Option<&str>, child_inline: &str) -> f32 {
        use parley::{FontContext, LayoutContext};
        use raikiri_style::{build_rule_tree, cascade};

        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), parent_inline);
        let p = doc.append_element(Some(body), "p", Style::default(), Some(child_inline));
        let text = doc.append_text(p, "Hi");
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        let mut fonts = FontContext::new();
        let mut layout_cx = LayoutContext::<()>::new();
        preshape_text(
            &mut doc,
            &cr,
            &mut fonts,
            &mut layout_cx,
            PageBox::A4.width,
            PageBox::A4.width,
        );
        doc.nodes[text].text_layout().unwrap().height()
    }

    /// guard 消失時の hang を **有界時間の失敗**に変える wrapper。
    ///
    /// 有界なのは **test** であって process ではない — timeout しても worker
    /// thread は spin したまま残る (parley に cancellation が無く、`break_all_lines`
    /// を中断する手段がないため)。test binary の終了時に process ごと落ちるので
    /// 実害は無いが、「有界化した」の射程はここまで。
    fn shaped_height_bounded(parent_inline: Option<&str>, child_inline: &str) -> f32 {
        use std::sync::mpsc::RecvTimeoutError;

        let parent = parent_inline.map(str::to_owned);
        let child = child_inline.to_owned();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(shaped_height(parent.as_deref(), &child));
        });
        // `Timeout` と `Disconnected` を混同しないこと — `shaped_height` は
        // 内部に `.expect("cascade Ok")` / `.unwrap()` を持つので、worker が
        // panic すると `tx` が drop されて **数 ms で** `Disconnected` が
        // 返る。これを「30 秒で終わらなかった」と報告すると cascade の
        // regression を guard 消失として調査させてしまい、本 wrapper の
        // 導入目的 (hang を通常の失敗と区別する) の裏返しになる。
        match rx.recv_timeout(std::time::Duration::from_secs(30)) {
            Ok(h) => h,
            Err(RecvTimeoutError::Timeout) => panic!(
                "parley shaping が 30 秒で終わらなかった — font-size の非有限 \
                     guard (sanitize_finite) が外れると break_all_lines が spin \
                     する"
            ),
            Err(RecvTimeoutError::Disconnected) => {
                panic!("worker thread が panic した (hang ではない、上の stderr を参照)")
            }
        }
    }

    // (a) +Inf font-size。`1e40px` は cssparser の f64→f32 で +Inf。
    let inf_px = shaped_height_bounded(None, "font-size: 1e40px");
    assert!(
        inf_px.is_finite(),
        "font-size +Inf (px 由来) が parley に届いた: {inf_px}"
    );

    // (b) +Inf font-size (em compounding 由来)。親は initial の 16px なので
    // `16.0 * inf = +Inf` — **NaN ではない**。
    let inf_em = shaped_height_bounded(None, "font-size: 1e40em");
    assert!(
        inf_em.is_finite(),
        "font-size +Inf (em 由来) が parley に届いた: {inf_em}"
    );

    // (c) **NaN** font-size — `0.0 * inf` (IEEE 754)。`font-size` の `em` は
    // **親**の computed font-size 基準 (CSS Values 4 §6.1.1) なので乗数
    // `font-size: 0px` は親側に載る。site 1-4 は乗数が同一 element に載るので
    // 1 element で作れるが、font-size だけは 2 element 要る。
    //
    // **`is_nan()` 分岐削除 mutation は本 case では死なない (実測)。** guard が生きている限り
    // parley が受け取るのは 0.0 であって NaN ではないので、**parley 側の
    // NaN 許容が変わってもここでは気づけない** (「上流の canary」ではない)。
    // `is_nan()` 分岐を殺す mutation を検出するのは site 1-4 の e2e 4 本と
    // `sanitize_finite_maps_nan_to_zero` の計 5 本 (mutation testing 実測)。
    //
    // それでも置く理由は 2 つ:
    //   1. NaN を作れる経路の一つ (親 `0px` × 子 `em`) が e2e で構築
    //      できることの pin。site 1-4 と違い 1 element では作れない。
    //   2. 「guard 消失 × 上流の NaN 許容変化」という複合 regression への
    //      保険 (単独ではどちらも他の test が拾う)。
    let nan = shaped_height_bounded(Some("font-size: 0px"), "font-size: 1e40em");
    assert!(nan.is_finite(), "font-size NaN が parley に届いた: {nan}");
    assert_eq!(
        nan, 0.0,
        "guard 後の font-size 0.0 に対する parley の height (上流変更の canary)",
    );
}

/// Shapes `"Hi"` via parley **directly**
/// (bypassing `preshape_text` / `sanitize_finite` entirely, not just
/// disabling them) with a raw `font_size`, bounded via worker-thread +
/// `recv_timeout`. Shared by the two `#[test]` fns below it: one pins the
/// (fast, cheap) "does not hang" cases, the other — `#[ignore]`d, see its
/// own doc — pins the one case that does.
fn shape_raw_bounded(font_size: f32, bound: std::time::Duration) -> Result<(), &'static str> {
    use std::sync::mpsc::RecvTimeoutError;

    fn shape_raw(font_size: f32) {
        let mut fonts = FontContext::new();
        let mut layout_cx = LayoutContext::<()>::new();
        let mut builder = layout_cx.ranged_builder(&mut fonts, "Hi", 1.0, true);
        builder.push_default(StyleProperty::FontSize(font_size));
        let mut layout: Layout<()> = builder.build("Hi");
        // A4 width in px, matching `PageBox::A4.width` — the same
        // `max_advance` `preshape_text` would pass in production.
        layout.break_all_lines(Some(793.7008_f32));
    }

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = std::panic::catch_unwind(move || shape_raw(font_size));
        let _ = tx.send(result.is_ok());
    });
    // cov:ignore: every call site of this helper (both this file's
    // tests) completes normally within its bound — the Err arms are
    // diagnostics for failure modes (panic, timeout, worker-disconnect)
    // this module's tests don't hit.
    match rx.recv_timeout(bound) {
        Ok(true) => Ok(()),
        Ok(false) => Err("panicked"),
        Err(RecvTimeoutError::Timeout) => Err("timeout"),
        Err(RecvTimeoutError::Disconnected) => Err("panicked"),
    }
}

/// Narrower half of a paired characterization — pins that `NaN`,
/// `-Inf`, and a merely-huge finite `font_size` (`1e9`) **do not** hang
/// parley's `break_all_lines`, at the same raw (guard-bypassing) call
/// site the `#[ignore]`d `+Inf` test below uses. Cheap (each sub-case
/// resolves in well under the 5s bound; no leaked spinning thread since
/// none of them hang), so — unlike the `+Inf` case — this runs in every
/// default `cargo test`.
///
/// # Why this exists as assertions, not just prose
///
/// The doc comment on `MAX_FONT_SIZE_PX` ("guard を外すと... 25 秒経っ
/// ても終了しない") reads as "non-finite font-size ⇒ hang" in general —
/// but that claim was written from a manual repro that only ever
/// exercised `+Inf` (the first sub-case its guarded test tries) before
/// hanging; it never got to see whether `NaN` or `-Inf` behave the same
/// way. They do not: only `+Inf` hangs, via the specific mechanism
/// documented on `MAX_FONT_SIZE_PX`
/// (`parley-0.10.0/src/layout/line_break.rs`'s `if next_x <= max_advance`
/// becoming permanently false once `next_x = +Inf`, so
/// `while self.break_next().is_some() {}` never terminates — for `NaN`
/// and `-Inf`, `next_x` does not end up stuck the same way). This test
/// turns "narrower than the prose it formalizes" from an unverified
/// assertion in a code comment into something a future `cargo test` run
/// keeps honest.
#[test]
fn parley_break_all_lines_completes_for_nan_neg_inf_and_huge_finite_font_size() {
    for (label, font_size) in [
        ("NaN", f32::NAN),
        ("-Inf", f32::NEG_INFINITY),
        ("1e9 (finite, 3 decades past MAX_FONT_SIZE_PX)", 1e9_f32),
    ] {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            shape_raw_bounded(font_size, std::time::Duration::from_secs(5)),
            Ok(()),
            "parley::Layout::break_all_lines(font_size = {label}) did not complete within 5s (bypassing raikiri's guard, same as the +Inf case) — this module's characterization that only +Inf hangs no longer holds for {label}; re-characterize rather than deleting this case"
        );
    }
}

/// `+Inf` half of the paired characterization — formalizes into an
/// automated regression test the manual measurement recorded in
/// `MAX_FONT_SIZE_PX`'s doc comment ("guard を外すと... 25 秒経っても
/// 終了しない"): `font_size = +Inf` reaching parley directly (bypassing
/// `preshape_text` / `sanitize_finite`, not just disabling them)
/// reproducibly hangs `break_all_lines`. See
/// `parley_break_all_lines_completes_for_nan_neg_inf_and_huge_finite_font_size`
/// for why `NaN`/`-Inf`/huge-finite do *not* share this behavior (this is
/// the one case that does, and it's the one this module's own
/// repro — `1e40px`, `1e40em` compounding — actually produces).
///
/// # Why `#[ignore]` (unlike every other test added alongside it)
///
/// Every other characterization test in this pair resolves in
/// well under a second because the sink under test either doesn't hang
/// or fails fast. This one is different **in the passing case**: parley
/// has no shaping-cancellation mechanism (documented on `MAX_FONT_SIZE_PX`
/// and above), so confirming the hang costs the full `bound` below on
/// every run, *and* the spawned worker thread is never joined — it spins
/// at ~100% CPU on one core for the rest of this test binary's process
/// lifetime, degrading every test that runs after it in the same binary.
/// That's an acceptable one-time characterization cost but not a
/// standing tax worth imposing on every `cargo test --workspace` from
/// every future session — hence `#[ignore]`, matching this repo's
/// existing convention for exactly this trade-off
/// (`crates/raikiri/tests/hello_world_vrt.rs`'s doc comment). Run
/// explicitly with:
///
/// ```text
/// cargo test -p raikiri-dom --lib \
///   layout::tests::parley_break_all_lines_hangs_on_raw_infinite_font_size_bypassing_the_guard \
///   -- --ignored
/// ```
// cov:ignore: this whole test body never runs under default `cargo
// test` (it's `#[ignore]`d — a genuine ~10s hang + leaked thread, see
// the doc comment above); it's exercised explicitly via `-- --ignored`
// (verified separately to run and pass), which
// llvm-cov's default `cargo test` invocation doesn't capture.
#[test]
#[ignore = "confirms a genuine ~10s hang + leaks a spinning worker thread for the rest \
                of the process; run explicitly, see doc comment"]
fn parley_break_all_lines_hangs_on_raw_infinite_font_size_bypassing_the_guard() {
    assert_eq!(
        shape_raw_bounded(f32::INFINITY, std::time::Duration::from_secs(10)),
        Err("timeout"),
        "parley::Layout::break_all_lines(font_size = +Inf) did not hang within 10s \
             — the line_break.rs livelock this test pins no longer reproduces in parley 0.10.0; \
             re-characterize rather than deleting this test (and consider whether \
             raikiri-dom's own MAX_FONT_SIZE_PX guard is still load-bearing for this \
             specific sink if parley itself now handles it). If this instead reports \
             \"panicked\", the worker thread panicked rather than hanging — that's a \
             different (and likely worse, since panics propagate less predictably than \
             a bounded hang) finding, not a pass"
    );
}

// ── guard 関数そのものの unit test ───────────────────────────────────
//
// e2e test は site 5 が hang し得るうえ 1 本あたり FontContext 構築を伴う。
// guard の算術は純関数なので直接叩く (数 ms、hang し得ない)。

#[test]
fn sanitize_finite_maps_nan_to_zero() {
    // `f32::clamp` は NaN を NaN のまま返すので、この分岐が無いと NaN が
    // 素通りする。
    let mut diag = Vec::new();
    assert_eq!(sanitize_finite(f32::NAN, -1.0, 1.0, "test", &mut diag), 0.0);
    assert_eq!(
        sanitize_finite(f32::NAN, 0.0, MAX_FONT_SIZE_PX, "test", &mut diag),
        0.0
    );
    // 両方とも実際に clamp した (NaN != 0.0) ので、それぞれ 1 event ずつ
    // `LayoutWarn::NonFiniteClamped` が積まれる。
    assert_eq!(
        diag.len(),
        2,
        "clamp が発火した回数だけ event が積まれること"
    );
    for event in &diag {
        match event {
            LayoutWarn::NonFiniteClamped { site, raw, clamped } => {
                assert_eq!(*site, "test");
                assert!(raw.is_nan());
                assert_eq!(*clamped, 0.0);
            }
            other => panic!("unexpected LayoutWarn variant: {other:?}"),
        }
    }
}

#[test]
fn sanitize_finite_clamps_infinities_to_bounds() {
    let mut diag = Vec::new();
    assert_eq!(
        sanitize_finite(f32::INFINITY, 0.0, MAX_FONT_SIZE_PX, "test", &mut diag),
        MAX_FONT_SIZE_PX
    );
    assert_eq!(
        sanitize_finite(
            f32::NEG_INFINITY,
            -MAX_TAFFY_MAGNITUDE,
            MAX_TAFFY_MAGNITUDE,
            "test",
            &mut diag
        ),
        -MAX_TAFFY_MAGNITUDE
    );
    // 下限が 0.0 の site (font-size) では -Inf は 0.0 に落ちる。
    assert_eq!(
        sanitize_finite(f32::NEG_INFINITY, 0.0, MAX_FONT_SIZE_PX, "test", &mut diag),
        0.0
    );
    assert_eq!(diag.len(), 3, "3 回とも clamp が発火する (全て非有限入力)");
}

#[test]
fn sanitize_taffy_clamps_out_of_range_finite_values() {
    // 有限でも範囲外なら寄せる (「有限化するだけ」ではない)。
    let mut diag = Vec::new();
    assert_eq!(sanitize_taffy(1e30, "test", &mut diag), MAX_TAFFY_MAGNITUDE);
    assert_eq!(
        sanitize_taffy(-1e30, "test", &mut diag),
        -MAX_TAFFY_MAGNITUDE
    );
    assert_eq!(diag.len(), 2);
}

#[test]
fn sanitize_taffy_passes_through_in_range_values() {
    // 通常値は bit-identical に素通しする (VRT が pixel-exact である前提)。
    let mut diag = Vec::new();
    for v in [0.0_f32, 1.0, -1.0, 16.0, 793.7008, MAX_TAFFY_MAGNITUDE] {
        assert_eq!(
            sanitize_taffy(v, "test", &mut diag),
            v,
            "in-range value must pass through: {v}"
        );
    }
    // 範囲内 (clamp が実質 no-op) では何も積まない — per-node spam を
    // 避ける設計の check (`sanitize_finite` の doc参照)。
    assert!(
        diag.is_empty(),
        "in-range value must not push a LayoutWarn: {diag:?}"
    );
}

#[test]
fn sanitize_line_height_clamps_non_finite_and_out_of_range_number() {
    // site 7: `ComputedLineHeight::Number` — grammar `<number [0,∞]>`
    // なので下限 0.0、上限 `MAX_LINE_HEIGHT_NUMBER`。NaN は他の length 系
    // site と同じ `sanitize_finite` の `0.0` fallback を継承する (font-weight
    // のような専用 fallback が要らない理由: line-height の unitless
    // number に `0` は grammar 上有効な値であり、font-weight の `400.0`
    // 事情 — `0.0` が妥当域外 — が line-height には無い)。
    let mut diag = Vec::new();
    assert_eq!(
        sanitize_line_height(ComputedLineHeight::Number(f32::NAN), &mut diag),
        ComputedLineHeight::Number(0.0)
    );
    assert_eq!(
        sanitize_line_height(ComputedLineHeight::Number(f32::INFINITY), &mut diag),
        ComputedLineHeight::Number(MAX_LINE_HEIGHT_NUMBER)
    );
    assert_eq!(
        sanitize_line_height(ComputedLineHeight::Number(f32::NEG_INFINITY), &mut diag),
        ComputedLineHeight::Number(0.0)
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        sanitize_line_height(ComputedLineHeight::Number(-5.0), &mut diag),
        ComputedLineHeight::Number(0.0),
        "negative multiplier is out of the [0,∞] grammar range and must clamp to 0.0"
    );
    assert_eq!(diag.len(), 4);
    for event in &diag {
        match event {
            LayoutWarn::NonFiniteClamped { site, .. } => {
                assert_eq!(*site, "line-height (number)");
            }
            // cov:ignore: `diag` in this test only ever accumulates
            // `NonFiniteClamped` events pushed by `sanitize_line_height`
            // above — `LayoutWarn::Truncated` is pushed elsewhere
            // (the `layout_single_page` cap-limiting path), never by
            // this function, so this arm is unreachable with this
            // test's inputs; it exists only for the match's
            // exhaustiveness.
            other => panic!("unexpected LayoutWarn variant: {other:?}"),
        }
    }
}

#[test]
fn sanitize_line_height_clamps_non_finite_and_out_of_range_length() {
    // site 8: `ComputedLineHeight::Length` — grammar
    // `<length-percentage [0,∞]>` の percentage は computed 層で既に
    // px へ絶対化済み (`ComputedLineHeight::Length` の doc参照) なので
    // ここでは px の妥当域だけを見る。
    let mut diag = Vec::new();
    assert_eq!(
        sanitize_line_height(
            ComputedLineHeight::Length(ComputedLength(f32::NAN)),
            &mut diag
        ),
        ComputedLineHeight::Length(ComputedLength(0.0))
    );
    assert_eq!(
        sanitize_line_height(
            ComputedLineHeight::Length(ComputedLength(f32::INFINITY)),
            &mut diag
        ),
        ComputedLineHeight::Length(ComputedLength(MAX_FONT_SIZE_PX))
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        sanitize_line_height(ComputedLineHeight::Length(ComputedLength(-10.0)), &mut diag),
        ComputedLineHeight::Length(ComputedLength(0.0)),
        "negative absolute line-height is out of the [0,∞] grammar range and must clamp to 0.0"
    );
    assert_eq!(diag.len(), 3);
    for event in &diag {
        match event {
            LayoutWarn::NonFiniteClamped { site, .. } => {
                assert_eq!(*site, "line-height (length)");
            }
            // cov:ignore: same unreachable-exhaustiveness arm as the
            // sibling `Number` test above — `diag` here never
            // accumulates a `Truncated` event.
            other => panic!("unexpected LayoutWarn variant: {other:?}"),
        }
    }
}

// ── crate::diag 経由の generalized 診断 channel ──
// fonts.rs の FontWarn observer pattern を汎用化した LayoutWarn 側の
// 独自 unit test。fonts.rs の `observer_fires_*` test 群と対になる。

/// `emit_layout_warn` は observer が `Some` ならそれを呼び、`eprintln!`
/// はしない — fonts.rs の `emit_warn` と対称的な契約 (両方とも
/// `crate::diag::emit_warn_via` を経由するので同じ振る舞いになるはず)。 // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests (#[test]-item doc) — rustdoc-blind, confirmed via わざと壊して確かめる
#[test]
fn emit_layout_warn_calls_observer_when_some() {
    let mut collected: Vec<LayoutWarn> = Vec::new();
    let mut cb = |w: &LayoutWarn| collected.push(*w);
    let mut observer: LayoutWarnObserver<'_> = Some(&mut cb);
    emit_layout_warn(
        &mut observer,
        LayoutWarn::NonFiniteClamped {
            site: "test",
            raw: f32::NAN,
            clamped: 0.0,
        },
    );
    assert_eq!(collected.len(), 1);
    // float literal は pattern に書けない (`illegal_floating_point_literal_pattern`
    // は deny-by-default) ので variant/site だけ matches! で確認し、
    // `clamped` の値は別途 `if let` で束縛して assert する。
    assert!(matches!(
        collected[0],
        LayoutWarn::NonFiniteClamped { site: "test", .. }
    ));
    if let LayoutWarn::NonFiniteClamped { clamped, .. } = collected[0] {
        assert_eq!(clamped, 0.0);
    }
}

/// `observer == None` では代わりに `eprintln!` する — 呼び出しても panic
/// しないことだけを確認する (stderr の内容は capture しない、fonts.rs の
/// 対応する経路も同様に未検証)。
#[test]
fn emit_layout_warn_falls_back_to_eprintln_when_none() {
    let mut observer: LayoutWarnObserver<'_> = None;
    emit_layout_warn(&mut observer, LayoutWarn::Truncated { suppressed: 3 });
}

/// `push_layout_warn` は `LAYOUT_WARN_CAP` を超えた分を個別 event
/// としてではなく単一の running `Truncated` counter に畳み込む —
/// 「病的な入力で every field が毎回 clamp される」場合に buffer と
/// 後段の eprintln! replay を有界にするための cap (doc 参照)。
#[test]
fn push_layout_warn_collapses_past_cap_into_truncated_counter() {
    let mut diag: Vec<LayoutWarn> = Vec::new();
    // cap ちょうどまでは real event。
    for _ in 0..LAYOUT_WARN_CAP {
        push_layout_warn(
            &mut diag,
            LayoutWarn::NonFiniteClamped {
                site: "test",
                raw: f32::NAN,
                clamped: 0.0,
            },
        );
    }
    assert_eq!(diag.len(), LAYOUT_WARN_CAP);
    assert!(
        diag.iter()
            .all(|w| matches!(w, LayoutWarn::NonFiniteClamped { .. })),
        "cap 以内は real event のみのはず: {diag:?}"
    );

    // cap を超えた分は Vec を伸ばさず、末尾の Truncated counter に集約される。
    for _ in 0..5 {
        push_layout_warn(
            &mut diag,
            LayoutWarn::NonFiniteClamped {
                site: "test",
                raw: f32::INFINITY,
                clamped: MAX_TAFFY_MAGNITUDE,
            },
        );
    }
    assert_eq!(
        diag.len(),
        LAYOUT_WARN_CAP + 1,
        "cap 超過分は Vec を伸ばさず Truncated に畳み込まれること: {diag:?}"
    );
    assert!(matches!(
        diag.last(),
        Some(LayoutWarn::Truncated { suppressed: 5 })
    ));
}

/// `LayoutWarn` の `Display` が両 variant で人間可読な文字列を出す
/// ことの check (`crate::diag::emit_warn_via` の `eprintln!` fallback が // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests (#[test]-item doc) — rustdoc-blind, confirmed via わざと壊して確かめる
/// 実際に読める行になることの保証)。
#[test]
fn layout_warn_display_is_human_readable() {
    let clamped = LayoutWarn::NonFiniteClamped {
        site: "font-size",
        raw: f32::NAN,
        clamped: 0.0,
    };
    assert_eq!(
        clamped.to_string(),
        "font-size: clamped non-finite/out-of-range value NaN to 0"
    );
    let truncated = LayoutWarn::Truncated { suppressed: 7 };
    assert_eq!(
        truncated.to_string(),
        "7 additional layout clamp warning(s) suppressed (buffer cap reached)"
    );
}

/// clamp 定数が **doc が主張する帯の中にある**ことの pin。
///
/// literal との `assert_eq!` は同語反復なので使わない — 定数を書き換えれば
/// test も一緒に書き換わり、何も検出しない。doc が根拠として挙げた
/// **関係式**を書く。
#[test]
fn clamp_limits_are_in_the_documented_range() {
    // taffy 幾何: CSSWG issue #4552 が報告する実装の LayoutUnit 上限帯
    // (1e7〜1e8 px) の中にあること。
    assert!(
        (1e7..=1e8).contains(&MAX_TAFFY_MAGNITUDE),
        "MAX_TAFFY_MAGNITUDE は CSSWG #4552 の 1e7..=1e8 px 帯に収まること: {MAX_TAFFY_MAGNITUDE}"
    );
    // doc はより強く「帯の**下端**を採る = 3 engine のいずれの上限より下」と
    // 主張している。最小は old-Edge の `2^31 / 100 ≈ 2.15e7 px`。
    assert!(
        MAX_TAFFY_MAGNITUDE <= (i32::MAX / 100) as f32,
        "MAX_TAFFY_MAGNITUDE は 3 engine の最小上限 (2^31/100 ≈ 2.15e7 px) 以下であること: {MAX_TAFFY_MAGNITUDE}"
    );
    // font-size: skrifa の 16.16 fixed 変換が saturate する
    // `i32::MAX / 64 ≈ 3.36e7` ppem より **1 桁以上**下 (doc の主張)。
    assert!(
        MAX_FONT_SIZE_PX * 10.0 < (i32::MAX / 64) as f32,
        "MAX_FONT_SIZE_PX は skrifa の saturation 点より 1 桁以上下であること: {MAX_FONT_SIZE_PX}"
    );
}

// ── 出力側 guard: nested percentage ──────────
//
// 入力側 guard (上の site 1-4) は bridge に入る f32 を有限化するが、
// percentage は used value 層 (taffy) で containing block に対して解決され
// nest ごとに複利するため、**出力**は非有限に戻りうる。以下はその出力側
// guard (`sanitize_taffy_layout`) の pin。

/// [`sanitize_taffy_layout`] が保証する invariant の述語 —
/// [`taffy::Layout`] の全 f32 field が有限。
///
/// paint が現に読む 4 field ではなく全 field を見る (guard 側と同じ理由)。
///
/// `..` を使わず網羅 destructure するのも guard 側と同じ理由 — taffy が
/// f32 field を増やしたときに guard 側 (網羅 literal) だけが compile error に
/// なり、**述語側は黙って旧 field しか見ない**、という非対称を作らないため。
fn layout_all_finite(l: &TaffyLayout) -> bool {
    fn size_ok(s: Size<f32>) -> bool {
        s.width.is_finite() && s.height.is_finite()
    }
    fn rect_ok(r: Rect<f32>) -> bool {
        r.left.is_finite() && r.right.is_finite() && r.top.is_finite() && r.bottom.is_finite()
    }
    let TaffyLayout {
        // `order` は u32 — guard 対象外 (`sanitize_taffy_layout` の doc)。
        order: _,
        location,
        size,
        scrollable_overflow_rect,
        scrollbar_size,
        border,
        padding,
        margin,
    } = l;
    location.x.is_finite()
        && location.y.is_finite()
        && size_ok(*size)
        && rect_ok(*scrollable_overflow_rect)
        && size_ok(*scrollbar_size)
        && rect_ok(*border)
        && rect_ok(*padding)
        && rect_ok(*margin)
}

/// `<html><body>` の下に `decl` を持つ `<div>` を `depth` 段 nest した
/// document を [`layout_single_page`] に通し、**各段の**
/// `unrounded_layout` を浅い順に返す。
///
/// 起点は probe 材料の depth range test setup だが、**depth ごとに document を作り直さない** —
/// depth `N` の chain は 1..=`N` の各深さの node を既に含んでおり、
/// probe が depth ごとに払っていた `FontContext::new()`
/// (`font_context_new_cost_is_reasonable` が 10 回 5 秒未満を check =
/// 決して安くない) を depth 数だけ払う理由が無いため。
fn nested_decl_layouts(decl: &str, depth: usize) -> Vec<TaffyLayout> {
    use raikiri_style::{build_rule_tree, cascade};
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let mut parent = body;
    let mut ids = Vec::with_capacity(depth);
    for _ in 0..depth {
        parent = doc.append_element(Some(parent), "div", Style::default(), Some(decl));
        ids.push(parent);
    }
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");
    ids.into_iter()
        .map(|i| doc.nodes[i].unrounded_layout)
        .collect()
}

/// 修正前は下記の depth で `unrounded_layout` が
/// 非有限に戻っていた。probe 材料 RAWDATA.txt の depth range 実測では
/// **base (guard 前) / head (入力側 guard 後) が完全に一致**していた =
/// 入力側 guard では閉じない穴であることの証拠:
///
/// | decl | test setup | 本 test setup (実測) |
/// |---|---|---|
/// | `width: 1e9%` | 6 | 6 |
/// | `width: 100000%` | 12 | 12 |
/// | `width: 10000%` | 18 | 18 |
/// | `width: 1000%` | 36 | 36 |
/// | `width: 200%` | 到達せず | 到達せず |
/// | `padding-left: 1e9%` | 4 | 5 |
/// | `padding-left: 100000%` | 8 | 9 |
/// | `padding-left: 1000%` | 25 | 25 |
/// | `padding-left: 200%` | 到達せず | 到達せず |
///
/// (`padding-left` 系 2 行の ±1 は 2 test setup の差に由来する。probe は
/// depth ごとに document を作り直すので最深段が leaf になるが、本 test setup
/// は 1 本の chain を最深まで伸ばして各段を見るので同じ段が container に
/// なる。**ただし機構は特定できていない** — この構造差が原因なら padding
/// 系 3 行すべてがずれるはずだが `padding-left: 1000%` は 25/25 で一致
/// する。数値自体は再現可能で、本 test setup 列は `set_unrounded_layout` の
/// `sanitize_taffy_layout` 呼び出しだけを外して実測した値である。
/// `width` 系 4 行は完全一致。)
///
/// 修正後はすべて「到達せず」になる。
///
/// **検査幅 45 は表の range 範囲に揃えた値であって、保証の上限ではない。**
/// 本 test が check するのは「この 9 declaration を深さ 45 まで見た範囲で
/// 保存値が全 field 有限」という**検査した点**だけである。深さ非依存性
/// そのものは test からは出てこない — 根拠は
/// `sanitize_taffy_layout` が taffy から arena への唯一の書き込み経路に
/// 置かれているという **choke point の構造的議論**の側にある。
/// `nested_percentage_output_stays_finite_far_past_the_range` も
/// 「range よりかなり深い一例」を足すだけで、全称的な深さ非依存性を
/// check するものではない。したがってこの 45 を「安全な上限」として
/// 下げないこと (下げてよい根拠は test ではなく構造の側にある)。
#[test]
fn nested_percentage_output_is_finite_through_probe_sweep_depth() {
    const SWEEP_DEPTH: usize = 45;
    for decl in [
        "width: 1e9%",
        "width: 100000%",
        "width: 10000%",
        "width: 1000%",
        "width: 200%",
        "padding-left: 1e9%",
        "padding-left: 100000%",
        "padding-left: 1000%",
        "padding-left: 200%",
    ] {
        let layouts = nested_decl_layouts(decl, SWEEP_DEPTH);
        assert_eq!(layouts.len(), SWEEP_DEPTH);
        if let Some((i, bad)) = layouts
            .iter()
            .enumerate()
            .find(|(_, l)| !layout_all_finite(l))
        {
            panic!(
                "decl {decl:?}: nest depth {} の unrounded_layout に非有限 f32 が残っている: {bad:?}",
                i + 1
            );
        }
    }
}

/// **深さ 96 でも保存値が有限**であることの pin。
///
/// `nested_percentage_output_is_finite_through_probe_range_depth` は上
/// の表に揃えた深さ 45 までしか見ないので、修正前に最も浅く破れた
/// `padding-left: 1e9%` (test setup で depth 4 / 本 test setup で depth 5)
/// を、その range 幅の 2 倍超で追加の 1 点として見る。
///
/// **本 test は深さ非依存性を check しない** — 有限深さの test が示せるのは
/// 常に「検査した深さでは有限」までである。深さ非依存性の根拠は
/// `sanitize_taffy_layout` が taffy から arena への唯一の書き込み経路に
/// 置かれているという **choke point の構造的議論**であって、本 test では
/// ない。本 test はその構造的議論に対する sanity check の位置づけ。
///
/// **本 test は (a) / (b) 案を排除しない** (できない) — 入力側 fraction
/// bound `F` に対する破綻深さ `35.6 / log10(F)`
/// (`MAX_TAFFY_MAGNITUDE` の doc の表) は深さ 96 では `F >= 2.35` しか
/// 捕まえられず、`width: 200%` を温存する最小の `F = 2.0` は `D = 118` で
/// **本 test を通ってしまう**。有限深さの test は原理的に (a) を排除できない。
/// (a) 却下の根拠は「深さ非依存には `F <= 1` が要り、それが `width: 200%` を
/// 殺す」という `MAX_TAFFY_MAGNITUDE` の doc の議論であって、本 test ではない。
#[test]
fn nested_percentage_output_stays_finite_far_past_the_sweep() {
    const DEEP: usize = 96;
    let layouts = nested_decl_layouts("padding-left: 1e9%", DEEP);
    assert_eq!(layouts.len(), DEEP);
    for (i, l) in layouts.iter().enumerate() {
        assert!(
            layout_all_finite(l),
            "nest depth {} で非有限に戻った: {l:?}",
            i + 1
        );
    }
}

/// `sanitize_taffy_layout` の field 単位の挙動 (上の 2 test は「有限で
/// ある」までしか見ないので、どの値に落ちるかはこちらで check する)。
#[test]
fn sanitize_taffy_layout_clamps_every_f32_field() {
    let poisoned = TaffyLayout {
        order: 7,
        location: Point {
            x: f32::INFINITY,
            y: f32::NEG_INFINITY,
        },
        size: Size {
            width: f32::NAN,
            height: 1e30,
        },
        scrollable_overflow_rect: Rect {
            left: 3.0,
            top: 4.0,
            right: -1e30,
            bottom: f32::NAN,
        },
        scrollbar_size: Size {
            width: f32::INFINITY,
            height: 12.0,
        },
        border: Rect {
            left: f32::NAN,
            right: f32::INFINITY,
            top: f32::NEG_INFINITY,
            bottom: 1.0,
        },
        padding: Rect {
            left: 1e30,
            right: -1e30,
            top: f32::NAN,
            bottom: 2.0,
        },
        margin: Rect {
            left: f32::NEG_INFINITY,
            right: f32::INFINITY,
            top: -3.0,
            bottom: f32::NAN,
        },
    };
    let mut diag = Vec::new();
    let s = sanitize_taffy_layout(&poisoned, &mut diag);

    // `order` は u32 なので guard 対象外 — 素通しすること。
    assert_eq!(s.order, 7, "order は clamp 対象ではない");

    // 16 field が非有限/範囲外 (下の個別 assert が数える対象と一致): location
    // 2 + size 2 + scrollable_overflow_rect 2 + scrollbar_size 1 + border 3 + padding 3
    // + margin 3。範囲内の 4 field (scrollbar_size.height / border.bottom /
    // padding.bottom / margin.top) は積まれない (per-node spam を
    // 避ける設計)。
    assert_eq!(
        diag.len(),
        16,
        "clamp が実際に発火した field の数だけ LayoutWarn が積まれること: {diag:?}"
    );
    assert!(
        diag.iter().all(|w| matches!(
            w,
            LayoutWarn::NonFiniteClamped { site, .. }
                if site.starts_with("layout.")
        )),
        "sanitize_taffy_layout 由来の event は全て layout.* site label を持つこと: {diag:?}"
    );

    assert_eq!(s.location.x, MAX_TAFFY_MAGNITUDE);
    assert_eq!(s.location.y, -MAX_TAFFY_MAGNITUDE);
    // NaN は clamp では潰れないので `is_nan()` → 0.0 (sanitize_finite)。
    assert_eq!(s.size.width, 0.0);
    assert_eq!(s.size.height, MAX_TAFFY_MAGNITUDE);
    assert_eq!(s.scrollable_overflow_rect.right, -MAX_TAFFY_MAGNITUDE);
    assert_eq!(s.scrollable_overflow_rect.bottom, 0.0);
    assert_eq!(s.scrollbar_size.width, MAX_TAFFY_MAGNITUDE);
    assert_eq!(s.scrollbar_size.height, 12.0, "範囲内の値は素通し");
    assert_eq!(s.border.left, 0.0);
    assert_eq!(s.border.right, MAX_TAFFY_MAGNITUDE);
    assert_eq!(s.border.top, -MAX_TAFFY_MAGNITUDE);
    assert_eq!(s.border.bottom, 1.0);
    assert_eq!(s.padding.left, MAX_TAFFY_MAGNITUDE);
    assert_eq!(s.padding.right, -MAX_TAFFY_MAGNITUDE);
    assert_eq!(s.padding.top, 0.0);
    assert_eq!(s.padding.bottom, 2.0);
    assert_eq!(s.margin.left, -MAX_TAFFY_MAGNITUDE);
    assert_eq!(s.margin.right, MAX_TAFFY_MAGNITUDE);
    assert_eq!(s.margin.top, -3.0);
    assert_eq!(s.margin.bottom, 0.0);

    // `[-MAX, MAX]` に収まる Layout の 1 例が bit 単位で不変であることの
    // pin。「通常 layout への影響ゼロ」を示すものではない — 影響が無いのは
    // 帯の内側に収まる場合だけで、外に出る入力 (`width: 200%` × 14 段 nest
    // など) では値が動く (`MAX_TAFFY_MAGNITUDE` の「clamp が実際に効く帯」節)。
    let benign = TaffyLayout {
        order: 3,
        location: Point { x: 10.0, y: -20.5 },
        size: Size {
            width: 793.7008,
            height: 1122.52,
        },
        scrollable_overflow_rect: Rect {
            left: 0.0,
            top: 0.0,
            right: 100.0,
            bottom: 200.0,
        },
        scrollbar_size: Size {
            width: 0.0,
            height: 0.0,
        },
        border: Rect {
            left: 1.0,
            right: 2.0,
            top: 3.0,
            bottom: 4.0,
        },
        padding: Rect {
            left: 5.0,
            right: 6.0,
            top: 7.0,
            bottom: 8.0,
        },
        margin: Rect {
            left: -9.0,
            right: 10.0,
            top: 11.0,
            bottom: 12.0,
        },
    };
    let mut diag = Vec::new();
    assert_eq!(sanitize_taffy_layout(&benign, &mut diag), benign);
    assert!(
        diag.is_empty(),
        "全 field が range 内なので LayoutWarn は積まれないこと: {diag:?}"
    );
}

// ── 意味的 invariant fallback ─────────────────
//
// 上の `sanitize_taffy_layout_*` test 群は「全 field が有限」までしか
// 見ない (前段の scope)。以下は `enforce_layout_invariants` が扱う
// 「field は有限だが親子関係が意味的に壊れている」層の pin。
// `enforce_layout_invariants` の doc の 2 つの probe (border-box
// padding overflow / 負 margin overflow) の数値もここで正式な
// assertion に昇格させている。

/// invariant 1 (content box 非負) の直接 pin。field 単位では
/// `sanitize_taffy_layout` を素通りする値 (`size` も `padding` もどちらも
/// `[-MAX, MAX]` 内) で `content_box_width() < 0.0` を作り、
/// `enforce_layout_invariants` が (a) その node、(b) その **subtree 内の
/// child** の両方をゼロ化すること、(c) `LayoutWarn` を積むことを確認する。
/// (b) が無いと「親だけゼロ化して子は壊れた親を基準にした古い値のまま」
/// という中途半端な状態になり、invariant 2 を新たに破ってしまう。
#[test]
fn content_box_violation_resets_subtree_to_zero_layout() {
    let mut doc = Document::new();
    let parent = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
    let child = doc.append_element(Some(parent), "div", Style::default(), None::<&str>);

    // size.width (10.0) より padding.left+right (40.0) の方が大きい ⇒
    // content_box_width() = 10.0 - 40.0 = -30.0 < 0.0。size / padding
    // どちらも個別には `[-MAX_TAFFY_MAGNITUDE, MAX_TAFFY_MAGNITUDE]` 内
    // なので `sanitize_taffy_layout` の field 単位 clamp はこれを止めない
    // — これが実際に起こりうることの直接的な再現。
    doc.nodes[parent].unrounded_layout = TaffyLayout {
        order: 3,
        location: Point::ZERO,
        size: Size {
            width: 10.0,
            height: 10.0,
        },
        scrollable_overflow_rect: Rect::ZERO,
        scrollbar_size: Size::zero(),
        border: Rect::zero(),
        padding: Rect {
            left: 20.0,
            right: 20.0,
            top: 0.0,
            bottom: 0.0,
        },
        margin: Rect::zero(),
    };
    // child 自体は (壊れた parent を無視すれば) 何の問題も無い layout —
    // subtree 全体がゼロ化されることを確認するための材料。
    doc.nodes[child].unrounded_layout = TaffyLayout {
        order: 1,
        location: Point { x: 1.0, y: 1.0 },
        size: Size {
            width: 2.0,
            height: 2.0,
        },
        scrollable_overflow_rect: Rect::ZERO,
        scrollbar_size: Size::zero(),
        border: Rect::zero(),
        padding: Rect::zero(),
        margin: Rect::zero(),
    };

    enforce_layout_invariants(&mut doc, parent);

    assert_eq!(
        doc.nodes[parent].unrounded_layout,
        TaffyLayout::with_order(3),
        "content box が負の node は order だけ残してゼロ化されること"
    );
    assert_eq!(
        doc.nodes[child].unrounded_layout,
        TaffyLayout::with_order(1),
        "破れた parent の subtree にいる child も (order だけ残して) \
             ゼロ化されること — 壊れた親を基準にした古い座標を残さない"
    );
    assert!(
        doc.layout_warnings.iter().any(|w| matches!(
            w,
            LayoutWarn::GeometryInvariantViolated {
                invariant: "content_box_non_negative",
                subtree_count: 1,
            }
        )),
        "content box invariant 違反が LayoutWarn として記録されること: {:?}",
        doc.layout_warnings
    );
}

/// **この変更で挙動が反転した直接 check (旧名
/// `saturated_child_outside_parent_resets_subtree_to_zero_layout`)**。
/// `child_within_parent_border_box` の gate (「その axis 自身の
/// `child.location` が飽和している」) を満たし、かつ旧実装なら
/// containment 違反として reset されていたはずの、直接構築した
/// maximally-非-contained な値 (`location.x == MAX_TAFFY_MAGNITUDE`
/// に対し `parent.size.width == 100.0`) を使う。この変更で `axis_ok` が
/// 符号を問わず無条件 `true` になったため、この fixture は — 実際には
/// 明らかに parent border box の外にあるにもかかわらず — もう reset
/// されない。「fixture を直接構築しても、もはやこの invariant を
/// 破らせることはできない」ことを示す regression check として残す
/// (`child_within_parent_border_box` の doc「符号を問わず無条件
/// accept になった理由」節、および将来「この check 自体を維持すべきか」
/// を判断する follow-up (別途明示的に deferred とされた問題)
/// が「現在の関数は実際に何をするか」を確認する材料として使うことを
/// 想定している)。
#[test]
fn saturated_child_outside_parent_is_not_reset() {
    let mut doc = Document::new();
    let parent = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
    let child = doc.append_element(Some(parent), "div", Style::default(), None::<&str>);
    let grandchild = doc.append_element(Some(child), "div", Style::default(), None::<&str>);

    // parent は正常 (100x100, 飽和していない)。
    doc.nodes[parent].unrounded_layout = TaffyLayout {
        order: 0,
        location: Point::ZERO,
        size: Size {
            width: 100.0,
            height: 100.0,
        },
        scrollable_overflow_rect: Rect::ZERO,
        scrollbar_size: Size::zero(),
        border: Rect::zero(),
        padding: Rect::zero(),
        margin: Rect::zero(),
    };
    // child.location.x がちょうど飽和境界 (MAX_TAFFY_MAGNITUDE) —
    // parent (100x100) には到底収まらない。この変更以降、
    // この「明らかに収まっていない」事実はもう reset の理由にならない。
    doc.nodes[child].unrounded_layout = TaffyLayout {
        order: 2,
        location: Point {
            x: MAX_TAFFY_MAGNITUDE,
            y: 0.0,
        },
        size: Size {
            width: 10.0,
            height: 10.0,
        },
        scrollable_overflow_rect: Rect::ZERO,
        scrollbar_size: Size::zero(),
        border: Rect::zero(),
        padding: Rect::zero(),
        margin: Rect::zero(),
    };
    // grandchild は child を基準にした normal な値 — reset されて
    // いないことを subtree 全体で確認する材料 (下記 assert 参照)。
    doc.nodes[grandchild].unrounded_layout = TaffyLayout {
        order: 5,
        location: Point { x: 1.0, y: 1.0 },
        size: Size {
            width: 1.0,
            height: 1.0,
        },
        scrollable_overflow_rect: Rect::ZERO,
        scrollbar_size: Size::zero(),
        border: Rect::zero(),
        padding: Rect::zero(),
        margin: Rect::zero(),
    };

    let child_before = doc.nodes[child].unrounded_layout;
    let grandchild_before = doc.nodes[grandchild].unrounded_layout;
    enforce_layout_invariants(&mut doc, parent);

    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        doc.nodes[parent].unrounded_layout.size,
        Size {
            width: 100.0,
            height: 100.0
        },
        "parent 自身は invariant を破っていないので手を付けないこと"
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        doc.nodes[child].unrounded_layout, child_before,
        "child.location.x が飽和境界にちょうど達し、かつ実際に parent border box の外にあっても、現在の実装では符号を問わず無条件 accept なので reset されないこと"
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        doc.nodes[grandchild].unrounded_layout, grandchild_before,
        "child が reset されていない以上、その下の grandchild も一切変更されないこと"
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        doc.layout_warnings
            .iter()
            .all(|w| !matches!(w, LayoutWarn::GeometryInvariantViolated { .. })),
        "reset が起きていないので GeometryInvariantViolated は積まれないこと: {:?}",
        doc.layout_warnings
    );
}

/// invariant 2 の gate が **無条件ではない**ことの check — CSS が普通に
/// 許す overflow (小さい parent + 負 margin で右/下/左にはみ出す child)
/// を `layout_single_page` のフルパイプラインで実際に layout し、
/// `enforce_layout_invariants` がそれを誤って fallback しないことを
/// 確認する。数値は `enforce_layout_invariants` の doc に記録した
/// 実測値と同じ (probe で先に確認済み)。
///
/// これが無いと「invariant 2 を無条件チェックにしてしまう」regression
/// (spec 違反 — legitimate な overflow layout を壊す) を検出できない。
#[test]
fn legitimate_negative_margin_overflow_is_not_reset() {
    use raikiri_style::{build_rule_tree, cascade};
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let parent = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("width: 50px; height: 50px;"),
    );
    let child = doc.append_element(
        Some(parent),
        "div",
        Style::default(),
        Some("width: 200px; height: 200px; margin-left: -30px;"),
    );
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    let parent_layout = doc.nodes[parent].unrounded_layout;
    let child_layout = doc.nodes[child].unrounded_layout;
    assert_eq!(
        parent_layout.size,
        Size {
            width: 50.0,
            height: 50.0
        },
        "parent の正常な layout は不変であること"
    );
    assert_eq!(
        child_layout.location,
        Point { x: -30.0, y: 0.0 },
        "負 margin による legitimate overflow の location はゼロ化されないこと \
             (parent の border box に収まらないが、これは正しい CSS layout)"
    );
    assert_eq!(
        child_layout.size,
        Size {
            width: 200.0,
            height: 200.0
        },
        "負 margin による legitimate overflow の size はゼロ化されないこと"
    );
    assert!(
        doc.layout_warnings
            .iter()
            .all(|w| !matches!(w, LayoutWarn::GeometryInvariantViolated { .. })),
        "飽和していない legitimate overflow は invariant 2 の gate を \
             通らないので GeometryInvariantViolated は 1 件も積まれないこと: {:?}",
        doc.layout_warnings
    );
}

/// gate (`child.location` 自身が飽和) が真でも child が実際には parent
/// に収まっている (境界ちょうど) ケースでは fallback しないことの pin。
///
/// **この変更が入る前の history**: この test はもともと旧
/// `saturated_child_outside_parent_resets_subtree_to_zero_layout`
/// (飽和 かつ containment 違反 → reset、この変更で
/// `saturated_child_outside_parent_is_not_reset` に改名・反転) と
/// 対にして、「gate 単独ではなく『gate かつ containment 違反』という
/// conjunction を検査している」ことを示す check だった。この変更で
/// `axis_ok` が符号を問わず無条件 `true` になったため、この
/// conjunction はもう成立しない — containment が実際にどうであっても
/// (境界ちょうどで収まっていても、明らかに外れていても) reset は
/// 起きない。本 test の assert 自体は (この fixture がたまたま
/// 「収まっている」ケースだったため) 引き続き通るが、それは
/// 「containment を検査して pass した」からではなく「そもそも
/// containment を見ていない」から — その事実を示す対の regression check
/// は `saturated_child_outside_parent_is_not_reset` を参照。
/// 実際の nested percentage chain を使った同種の check は
/// `nested_percentage_wide_child_chain_is_not_reset` を参照
/// (extent ではなく origin だけを見る現行の `child_within_parent_border_box`
/// を選んだ直接の理由になった regression)。
#[test]
fn saturated_but_contained_layout_is_not_reset() {
    let mut doc = Document::new();
    let parent = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
    let child = doc.append_element(Some(parent), "div", Style::default(), None::<&str>);

    // parent の size もちょうど飽和境界 — child の location がそこに
    // ぴったり収まる (境界値、`<=` で ok) ケースを作る。
    doc.nodes[parent].unrounded_layout = TaffyLayout {
        order: 0,
        location: Point::ZERO,
        size: Size {
            width: MAX_TAFFY_MAGNITUDE,
            height: MAX_TAFFY_MAGNITUDE,
        },
        scrollable_overflow_rect: Rect::ZERO,
        scrollbar_size: Size::zero(),
        border: Rect::zero(),
        padding: Rect::zero(),
        margin: Rect::zero(),
    };
    // child.location 自身がちょうど飽和境界 — gate
    // (`child_within_parent_border_box` の axis 単位 gate) を発火させる。
    // parent.size と同じ値なので `<=` で境界ちょうど「収まっている」。
    doc.nodes[child].unrounded_layout = TaffyLayout {
        order: 1,
        location: Point {
            x: MAX_TAFFY_MAGNITUDE,
            y: MAX_TAFFY_MAGNITUDE,
        },
        size: Size {
            width: 0.0,
            height: 0.0,
        },
        scrollable_overflow_rect: Rect::ZERO,
        scrollbar_size: Size::zero(),
        border: Rect::zero(),
        padding: Rect::zero(),
        margin: Rect::zero(),
    };

    let child_before = doc.nodes[child].unrounded_layout;
    enforce_layout_invariants(&mut doc, parent);

    assert_eq!(
        doc.nodes[parent].unrounded_layout.size,
        Size {
            width: MAX_TAFFY_MAGNITUDE,
            height: MAX_TAFFY_MAGNITUDE
        },
        "飽和した parent 自身は content box invariant を破っていないので \
             手を付けないこと"
    );
    assert_eq!(
        doc.nodes[child].unrounded_layout, child_before,
        "gate (child.location 自身の飽和) が真の child はゼロ化されないこと \
             — 現在の実装では、この fixture がたまたま境界ちょうど \
             で収まっているかどうかは無関係 (containment はもう見ていない)"
    );
    assert!(
        doc.layout_warnings
            .iter()
            .all(|w| !matches!(w, LayoutWarn::GeometryInvariantViolated { .. })),
        "reset が起きていないので GeometryInvariantViolated は積まれないこと: {:?}",
        doc.layout_warnings
    );
}

/// **この変更で挙動が反転した check (旧名
/// `saturated_location_with_legitimate_negative_margin_on_other_axis_is_not_reset`,
/// 旧主張「`y` 軸の検出力が保たれていること」)**。
///
/// この変更が入る前は、この fixture (`y` 軸が飽和かつ実際に parent に
/// 収まっていない = 真の violation、`x` 軸は飽和していない legitimate
/// な負 margin `-30`) は `y` 軸の検出力を示す check として意味があった —
/// `y` 軸の再検査だけで reset の理由が説明でき、`x` 軸の legitimate な
/// 負値は無視されることを示せた。この変更で `axis_ok` が符号を問わず
/// 無条件 `true` になったため、`y` 軸は飽和しているだけでもう
/// containment を再検査しない —「収まっていない」という事実自体が
/// reset の理由になり得なくなった。
///
/// 本 test は現在、`saturated_but_contained_axis_with_legitimate_negative_margin_on_other_axis_is_not_reset`
/// とほぼ同じ主張 (どちらの axis も reset の理由にならない) の近縁 check
/// になっている。唯一の違いは `y` 軸の値 — こちらは `y` が **実際には
/// parent に収まっていない** (`MAX_TAFFY_MAGNITUDE > 100.0`) のに対し、
/// あちらは境界ちょうどで収まっている。両方とも reset されないことで、
/// 「収まっているかどうか」が結果に一切影響しなくなったことを示す。
#[test]
fn saturated_axis_outside_parent_with_legitimate_negative_margin_on_other_axis_is_not_reset() {
    let mut doc = Document::new();
    let parent = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
    let child = doc.append_element(Some(parent), "div", Style::default(), None::<&str>);

    doc.nodes[parent].unrounded_layout = TaffyLayout {
        order: 0,
        location: Point::ZERO,
        size: Size {
            width: 100.0,
            height: 100.0,
        },
        scrollable_overflow_rect: Rect::ZERO,
        scrollbar_size: Size::zero(),
        border: Rect::zero(),
        padding: Rect::zero(),
        margin: Rect::zero(),
    };
    // y 軸: child.location.y がちょうど飽和境界かつ実際に parent
    // (height=100.0) に収まっていない。x 軸: 通常の負 margin (`-30`)
    // による legitimate overflow — 飽和していない。この変更
    // 以降、どちらの軸も reset の理由にならない。
    doc.nodes[child].unrounded_layout = TaffyLayout {
        order: 1,
        location: Point {
            x: -30.0,
            y: MAX_TAFFY_MAGNITUDE,
        },
        size: Size {
            width: 10.0,
            height: 10.0,
        },
        scrollable_overflow_rect: Rect::ZERO,
        scrollbar_size: Size::zero(),
        border: Rect::zero(),
        padding: Rect::zero(),
        margin: Rect::zero(),
    };

    let child_before = doc.nodes[child].unrounded_layout;
    enforce_layout_invariants(&mut doc, parent);

    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        doc.nodes[child].unrounded_layout, child_before,
        "y 軸 (飽和かつ実際には parent に収まっていない) も x 軸 (legitimate な負 margin、飽和していない) もどちらも現在の実装では reset の理由にならないので、child は一切変更されないこと"
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        doc.layout_warnings
            .iter()
            .all(|w| !matches!(w, LayoutWarn::GeometryInvariantViolated { .. })),
        "reset が起きていないので GeometryInvariantViolated は積まれないこと: {:?}",
        doc.layout_warnings
    );
}

/// **直前の指摘への直接回帰 pin**
/// — 直前の
/// `saturated_axis_outside_parent_with_legitimate_negative_margin_on_other_axis_is_not_reset`
/// (この変更が入る前の旧名
/// `saturated_location_with_legitimate_negative_margin_on_other_axis_is_not_reset`)
/// は当時「`y` 軸だけでも reset の説明がつく」ため、`x` 軸を実装が正しく
/// 無視しているかどうかを実際には区別できない、と指摘された
/// (旧 axis-mixing 実装でも当時の axis 単位実装でも同じ「reset される」
/// という結果になってしまうため)。
///
/// 本 test は区別できる fixture を使う: `y` 軸を「飽和境界にちょうど
/// 達しているが、それでも parent に収まっている」(= 真の violation では
/// ない) 値にし、`x` 軸には legitimate な負 margin (`-30`、飽和して
/// いない) を与える。
///
/// - **現行実装 (符号を問わず無条件 accept)**:
///   `x` 軸は飽和していないので無条件 ok。`y` 軸は飽和しているが、
///   containment を再検査せずやはり無条件 ok。→ **reset されない**。
/// - **この変更が入る直前の実装 (axis 単位・符号で分岐、正方向だけ `<=` を
///   再検査)**: `x` 軸は無条件 ok、`y` 軸は飽和かつ正なので再検査するが
///   実際に parent に収まっているので ok。→ 同じく reset されない
///   (この test はこの変更の前後で結果が変わらない — 変わったのは
///   `saturated_child_outside_parent_is_not_reset` や
///   `saturated_axis_outside_parent_with_legitimate_negative_margin_on_other_axis_is_not_reset`
///   のように実際に containment が破れているケース)。
/// - **旧 axis-mixing 実装 (`parent.size` / `child.size` /
///   `child.location` のいずれかが飽和していれば `x` / `y` 両方を
///   無条件チェック)**: `y` の飽和で gate が開き、`x >= 0.0` の
///   チェックに `-30.0` が失敗する → **誤って reset される**。
///
/// すなわち本 test が pass することは「`x` 軸の legitimate な負 margin
/// が reset の原因になっていない」ことの直接証拠であり、旧 axis-mixing
/// 実装への退行があれば本 test 単体で fail する。
#[test]
fn saturated_but_contained_axis_with_legitimate_negative_margin_on_other_axis_is_not_reset() {
    let mut doc = Document::new();
    let parent = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
    let child = doc.append_element(Some(parent), "div", Style::default(), None::<&str>);

    // parent.size.height もちょうど飽和境界 — child.location.y
    // (同じく飽和境界) がそこにぴったり収まる (`<=`、境界ちょうど) ように
    // するため。width は通常値 (x 軸は飽和させないので関係ない)。
    doc.nodes[parent].unrounded_layout = TaffyLayout {
        order: 0,
        location: Point::ZERO,
        size: Size {
            width: 100.0,
            height: MAX_TAFFY_MAGNITUDE,
        },
        scrollable_overflow_rect: Rect::ZERO,
        scrollbar_size: Size::zero(),
        border: Rect::zero(),
        padding: Rect::zero(),
        margin: Rect::zero(),
    };
    // x 軸: legitimate な負 margin (`-30`)、飽和していない — 無条件で
    // ok 扱いされるべき軸。y 軸: 飽和境界ちょうどだが、parent.size.height
    // と等しいので実際には収まっている (真の violation ではない) —
    // 「飽和している」だけでは reset の理由にならないことも同時に示す。
    doc.nodes[child].unrounded_layout = TaffyLayout {
        order: 1,
        location: Point {
            x: -30.0,
            y: MAX_TAFFY_MAGNITUDE,
        },
        size: Size {
            width: 10.0,
            height: 10.0,
        },
        scrollable_overflow_rect: Rect::ZERO,
        scrollbar_size: Size::zero(),
        border: Rect::zero(),
        padding: Rect::zero(),
        margin: Rect::zero(),
    };

    let child_before = doc.nodes[child].unrounded_layout;
    enforce_layout_invariants(&mut doc, parent);

    assert_eq!(
        doc.nodes[child].unrounded_layout, child_before,
        "x 軸 (legitimate な負 margin、飽和していない) も y 軸 (飽和\
             しているが実際には parent に収まっている) もどちらも reset の \
             理由にならないので、child は一切変更されないこと — これが \
             旧 axis-mixing 実装との直接の分岐点 (本 test の doc 参照)"
    );
    assert!(
        doc.layout_warnings
            .iter()
            .all(|w| !matches!(w, LayoutWarn::GeometryInvariantViolated { .. })),
        "reset が起きていないので GeometryInvariantViolated は積まれないこと: {:?}",
        doc.layout_warnings
    );
}

/// **直前の指摘への直接回帰 check (その 2)**
/// — 指摘された元の scenario そのもの: 同じ subtree の**無関係な
/// 別の場所** (ここでは同じ `parent` 自身) の `size` が飽和している状況で、
/// **その child 自身は何も飽和していない**のに legitimate な負 margin
/// (`location.x = -30`) を持つ。旧版の gate
/// (`parent.size` / `child.size` / `child.location` のいずれか 1 つでも
/// 飽和していれば検査する) はこれを誤って reset していた —
/// `child_within_parent_border_box` の doc「gate を axis 単位・
/// `child.location` 自身に限定する理由」節の 1. で説明した field 単位の
/// 問題を直接再現する。
#[test]
fn saturated_parent_size_with_legitimate_negative_margin_child_is_not_reset() {
    let mut doc = Document::new();
    let parent = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
    let child = doc.append_element(Some(parent), "div", Style::default(), None::<&str>);

    // parent.size が飽和 — 無関係な原因 (例えば別の subtree で起きた
    // percentage 複利) を想定した、この child とは関係の無い飽和。
    doc.nodes[parent].unrounded_layout = TaffyLayout {
        order: 0,
        location: Point::ZERO,
        size: Size {
            width: MAX_TAFFY_MAGNITUDE,
            height: MAX_TAFFY_MAGNITUDE,
        },
        scrollable_overflow_rect: Rect::ZERO,
        scrollbar_size: Size::zero(),
        border: Rect::zero(),
        padding: Rect::zero(),
        margin: Rect::zero(),
    };
    // child 自身は完全に正常 — location / size とも飽和していない、
    // 通常の負 margin による legitimate overflow
    // (`legitimate_negative_margin_overflow_is_not_reset` と同じ形)。
    doc.nodes[child].unrounded_layout = TaffyLayout {
        order: 1,
        location: Point { x: -30.0, y: 0.0 },
        size: Size {
            width: 200.0,
            height: 200.0,
        },
        scrollable_overflow_rect: Rect::ZERO,
        scrollbar_size: Size::zero(),
        border: Rect::zero(),
        padding: Rect::zero(),
        margin: Rect::zero(),
    };

    let child_before = doc.nodes[child].unrounded_layout;
    enforce_layout_invariants(&mut doc, parent);

    assert_eq!(
        doc.nodes[child].unrounded_layout, child_before,
        "child 自身は何も飽和していないので、無関係な parent.size の飽和を \
             理由に legitimate な負 margin を reset してはならない — これが \
             以前指摘された finding そのもの"
    );
    assert!(
        doc.layout_warnings
            .iter()
            .all(|w| !matches!(w, LayoutWarn::GeometryInvariantViolated { .. })),
        "reset が起きていないので GeometryInvariantViolated は積まれないこと: {:?}",
        doc.layout_warnings
    );
}

/// Regression check for a false positive found while implementing this
/// task: `width: 200%` nested `DEPTH`-ish levels deep is
/// **legitimate** CSS (each level is, by design, twice its parent — the
/// child's origin never moves off `(0, 0)`), yet an earlier version of
/// `child_within_parent_border_box` checked *extent*
/// (`location + size <= parent.size`) rather than just `location`, so it
/// flagged the transition depth where one level's saturated `size` first
/// exceeded its still-unsaturated parent's `size` — even though nothing
/// about the relationship (child = 2x parent) had changed, only the
/// absolute magnitude crossed `MAX_TAFFY_MAGNITUDE`. That cascaded
/// through `zero_layout_subtree` and silently collapsed most of a
/// legitimate deep chain to zero-size boxes.
///
/// `saturated_but_contained_layout_is_not_reset` pins the equivalent
/// minimal synthetic case (its doc records how a later change
/// changed what that check actually demonstrates); this test pins the
/// same "not reset" outcome against the exact real-world shape that
/// first surfaced the bug, so a future edit that reintroduces an
/// extent-based check (or anything else that treats "child bigger than
/// parent" as evidence of corruption) fails loudly here rather than
/// only in the synthetic test.
#[test]
fn nested_percentage_wide_child_chain_is_not_reset() {
    const DEPTH: usize = 45;
    let layouts = nested_decl_layouts("width: 200%", DEPTH);
    assert_eq!(layouts.len(), DEPTH);

    let saturated_count = layouts
        .iter()
        .filter(|l| taffy_magnitude_is_saturated(l.size.width))
        .count();
    assert!(
        saturated_count > 0,
        "this test's premise (some depth saturates `size.width` in this \
             sweep range) no longer holds — re-verify against \
             MAX_TAFFY_MAGNITUDE's doc before trusting the rest of this \
             test: {layouts:?}"
    );
    for (i, l) in layouts.iter().enumerate() {
        assert_ne!(
            *l,
            TaffyLayout::with_order(l.order),
            "nest depth {} was reset to a zero layout — a legitimate \
                 \"child is 2x parent at every depth\" declaration must \
                 survive `enforce_layout_invariants` even past the depth \
                 where `size` saturates, since the child's `location` never \
                 leaves the parent's border box: {l:?}",
            i + 1
        );
    }
}

/// Minimal synthetic check for the new negative-
/// saturation branch of `child_within_parent_border_box`, isolated
/// from any real CSS pipeline (mirrors how
/// `saturated_but_contained_layout_is_not_reset` pins the positive-
/// saturation pass path). `child.location.x` is placed exactly at
/// `-MAX_TAFFY_MAGNITUDE` against an ordinary, unsaturated
/// `parent.size` — under the pre-fix predicate
/// (`child.location >= 0.0 && child.location <= parent.size`) this is
/// **unreachable as a pass**: no negative value ever satisfies `>= 0.0`,
/// so the old code reset this unconditionally regardless of
/// `parent.size`. This test pins that the sign-based branch introduced
/// later (see `child_within_parent_border_box`'s
/// doc, "符号を問わず無条件 accept になった理由") treats
/// saturated-negative as unconditionally ok, the same way
/// unsaturated-negative already was — and, since a later change,
/// the same way saturated-positive now is too.
#[test]
fn saturated_negative_location_is_not_reset() {
    let mut doc = Document::new();
    let parent = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
    let child = doc.append_element(Some(parent), "div", Style::default(), None::<&str>);

    doc.nodes[parent].unrounded_layout = TaffyLayout {
        order: 0,
        location: Point::ZERO,
        size: Size {
            width: 100.0,
            height: 100.0,
        },
        scrollable_overflow_rect: Rect::ZERO,
        scrollbar_size: Size::zero(),
        border: Rect::zero(),
        padding: Rect::zero(),
        margin: Rect::zero(),
    };
    // child.location.x はちょうど飽和境界の**負**側 — parent.size は
    // 飽和していない通常値 (100.0)。旧式 (`>= 0.0 && <= parent.size`) は
    // 負の値に対しては恒等的に false だったので、この fixture は
    // 旧実装では必ず reset される (`>= 0.0` を満たす負数は存在しない)。
    doc.nodes[child].unrounded_layout = TaffyLayout {
        order: 1,
        location: Point {
            x: -MAX_TAFFY_MAGNITUDE,
            y: 0.0,
        },
        size: Size {
            width: 10.0,
            height: 10.0,
        },
        scrollable_overflow_rect: Rect::ZERO,
        scrollbar_size: Size::zero(),
        border: Rect::zero(),
        padding: Rect::zero(),
        margin: Rect::zero(),
    };

    let child_before = doc.nodes[child].unrounded_layout;
    enforce_layout_invariants(&mut doc, parent);

    // これ以降の assert メッセージはあえて 1 物理行で書く (この file の
    // 他 test の慣習である backslash 継続の複数行ではない) —
    // `scripts/lib/patch_coverage.py` の `code_only()` は行ごとに
    // string 状態をリセットするため、backslash 継続行の途中に (地の文
    // としての) `)`/`]`/`}` があると `cov:ignore` の block scope 計算が
    // そこで途切れ、後続行が exempt されず patch coverage が誤って
    // FAIL する (実際に踏んだ経験がある)。1 行に畳むのは
    // その回避策であり、単なる style の揺れではない。
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        doc.nodes[child].unrounded_layout, child_before,
        "child.location.x が飽和境界にちょうど達していても、負である限り MAX_TAFFY_MAGNITUDE の doc が言う implementation-specific limit に達しただけで破綻の証拠にはならない — reset されないこと"
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        doc.layout_warnings
            .iter()
            .all(|w| !matches!(w, LayoutWarn::GeometryInvariantViolated { .. })),
        "reset が起きていないので GeometryInvariantViolated は積まれないこと: {:?}",
        doc.layout_warnings
    );
}

/// Real CSS pipeline regression check, and the
/// **discriminating fixture** between the sign-based fix and a
/// considered-and-rejected alternative ("skip re-validation whenever
/// `parent.size` on that axis is also saturated, regardless of sign").
///
/// A *single* `margin-left: -1e9%` declaration (no nesting) on a child
/// of a plain `width: 100px` parent is enough: `sanitize_taffy` clamps
/// the *fraction* (`-1e9%` → `-1e7` after `/100.0`), taffy then resolves
/// that fraction against the parent's 100px containing block
/// (`-1e7 * 100 = -1e9`), and `sanitize_taffy_layout` clamps the
/// resulting raw `location.x` to exactly `-MAX_TAFFY_MAGNITUDE` on
/// write. `parent.size.width` stays `100.0` — nowhere near saturated.
///
/// The rejected "parent-also-saturated" alternative would still reset
/// this case (parent isn't saturated on this axis), so it does not
/// close this gap. Only a rule keyed on the
/// *child's own sign* (this fix) accepts it, which is why this test is
/// pinned independently of
/// `deep_nested_negative_percentage_margin_saturating_location_is_not_reset`
/// below (that one's parent *does* happen to be saturated too, so on
/// its own it could not rule out the rejected alternative).
#[test]
fn saturated_negative_margin_percentage_child_is_not_reset() {
    use raikiri_style::{build_rule_tree, cascade};
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let parent = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("width: 100px; height: 100px;"),
    );
    let child = doc.append_element(
        Some(parent),
        "div",
        Style::default(),
        Some("width: 10px; height: 10px; margin-left: -1e9%;"),
    );
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    let parent_layout = doc.nodes[parent].unrounded_layout;
    let child_layout = doc.nodes[child].unrounded_layout;
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        parent_layout.size,
        Size {
            width: 100.0,
            height: 100.0
        },
        "this test's premise (parent stays unsaturated) no longer holds — re-verify before trusting the rest of this test"
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        taffy_magnitude_is_saturated(child_layout.location.x) && child_layout.location.x < 0.0,
        "this test's premise (margin-left: -1e9% saturates child.location.x negative) no longer holds — re-verify against MAX_TAFFY_MAGNITUDE's doc before trusting the rest of this test: {child_layout:?}"
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_ne!(
        child_layout,
        TaffyLayout::with_order(child_layout.order),
        "a single extreme-but-spec-valid negative percentage margin (no nesting needed) must not reset the subtree merely because it saturates the location — the parent here is not saturated, so a 'skip when parent is also saturated' rule would not have fixed this: {child_layout:?}"
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        doc.layout_warnings
            .iter()
            .all(|w| !matches!(w, LayoutWarn::GeometryInvariantViolated { .. })),
        "reset が起きていないので GeometryInvariantViolated は積まれないこと: {:?}",
        doc.layout_warnings
    );
}

/// **Positive-direction analog of
/// `saturated_negative_margin_percentage_child_is_not_reset`**, real
/// CSS pipeline regression check for the decision to extend
/// the negative-side unconditional-accept treatment symmetrically to
/// the positive side.
///
/// A *single* `margin-left: 1e9%` declaration (no nesting) on a child of
/// a plain `width: 100px` parent is enough: `sanitize_taffy` clamps the
/// *fraction* (`1e9%` → `1e7` after `/100.0`), taffy then resolves that
/// fraction against the parent's 100px containing block
/// (`1e7 * 100 = 1e9`), and `sanitize_taffy_layout` clamps the resulting
/// raw `location.x` to exactly `MAX_TAFFY_MAGNITUDE` on write.
/// `parent.size.width` stays `100.0` — nowhere near saturated. Before
/// this fix, `axis_ok` re-checked `location.x <=
/// parent.size.width` for saturated positive locations
/// (`1e7 <= 100.0` → false) and reset the whole child subtree to a zero
/// layout, discarding spec-legal content — this is the exact repro that
/// motivated the decision (independently reproduced
/// in a throwaway worktree before the decision was recorded). This test
/// pins that the fix actually closes the gap through the real
/// cascade+taffy pipeline, not just at the unit level
/// (`saturated_but_contained_layout_is_not_reset` and
/// `saturated_child_outside_parent_is_not_reset` build `TaffyLayout`
/// literals directly and don't exercise cascade/taffy at all).
#[test]
fn saturated_positive_margin_percentage_child_is_not_reset() {
    use raikiri_style::{build_rule_tree, cascade};
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let parent = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("width: 100px; height: 100px;"),
    );
    let child = doc.append_element(
        Some(parent),
        "div",
        Style::default(),
        Some("width: 10px; height: 10px; margin-left: 1e9%;"),
    );
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    let parent_layout = doc.nodes[parent].unrounded_layout;
    let child_layout = doc.nodes[child].unrounded_layout;
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        parent_layout.size,
        Size {
            width: 100.0,
            height: 100.0
        },
        "this test's premise (parent stays unsaturated) no longer holds — re-verify before trusting the rest of this test"
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        taffy_magnitude_is_saturated(child_layout.location.x) && child_layout.location.x > 0.0,
        "this test's premise (margin-left: 1e9% saturates child.location.x positive) no longer holds — re-verify against MAX_TAFFY_MAGNITUDE's doc before trusting the rest of this test: {child_layout:?}"
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_ne!(
        child_layout,
        TaffyLayout::with_order(child_layout.order),
        "a single extreme-but-spec-valid positive percentage margin (no nesting needed) must not reset the subtree merely because it saturates the location — this fix extends the negative-side unconditional-accept treatment symmetrically to this positive case: {child_layout:?}"
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        doc.layout_warnings
            .iter()
            .all(|w| !matches!(w, LayoutWarn::GeometryInvariantViolated { .. })),
        "reset が起きていないので GeometryInvariantViolated は積まれないこと: {:?}",
        doc.layout_warnings
    );
}

/// Negative-direction analog of
/// `nested_percentage_wide_child_chain_is_not_reset`. `width: 200%`
/// alone never moves the child's origin off `(0, 0)` (that test's own
/// premise), so it can't exercise invariant 2's location check at all.
/// Adding `margin-left: -100%` at every depth does: since each level's
/// own width is `2x` its containing block, and `margin-left: -100%`
/// resolves against that same (growing) containing block, the relation
/// `child.location.x == -parent.size.width` holds at *every* depth —
/// legitimate, consistent, and unrelated to corruption — yet it pushes
/// `location.x` negative fast enough to saturate several levels before
/// `size.width` does (verified empirically: depth 15 saturates
/// `location.x` here, one level after `size.width` alone saturates at
/// depth 14 for plain `width: 200%`). Before this fix, every depth past
/// the first saturated one collapsed to a zero layout.
#[test]
fn deep_nested_negative_percentage_margin_saturating_location_is_not_reset() {
    const DEPTH: usize = 45;
    let layouts = nested_decl_layouts("width: 200%; margin-left: -100%", DEPTH);
    assert_eq!(layouts.len(), DEPTH);

    let saturated_negative_count = layouts
        .iter()
        .filter(|l| taffy_magnitude_is_saturated(l.location.x) && l.location.x < 0.0)
        .count();
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        saturated_negative_count > 0,
        "this test's premise (some depth saturates location.x negative in this sweep range) no longer holds — re-verify against MAX_TAFFY_MAGNITUDE's doc before trusting the rest of this test: {layouts:?}"
    );
    for (i, l) in layouts.iter().enumerate() {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_ne!(
            *l,
            TaffyLayout::with_order(l.order),
            "nest depth {} was reset to a zero layout — a legitimate \
                 \"child sits exactly one containing-block-width to the \
                 left of its parent at every depth\" declaration must \
                 survive `enforce_layout_invariants` even past the depth \
                 where `location.x` saturates negative: {l:?}",
            i + 1
        );
    }
}

#[test]
fn preshape_text_uses_mapped_ic_width_for_autospace_boxes() {
    use parley::{LayoutContext, PositionedLayoutItem};
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let tmp = embedded_ic_font_dir();

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let block = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;font-family:IcTestHalfWidth;font-size:16px;text-autospace:normal"),
    );
    let text = doc.append_text(block, "水A");
    let noauto = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some(
            "display:block;font-family:IcTestHalfWidth;font-size:16px;text-autospace:no-autospace",
        ),
    );
    let _noauto_text = doc.append_text(noauto, "水A");
    let oblique = doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("display:block;font-family:IcTestHalfWidth;font-size:16px;font-style:oblique;text-autospace:normal"),
        );
    let _oblique_text = doc.append_text(oblique, "水A");

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    let mut fonts = crate::fonts::build_wpt_font_ctx(tmp.path()).expect("register WPT fonts");
    let mut layout_cx = LayoutContext::<()>::new();
    preshape_text(
        &mut doc,
        &cr,
        &mut fonts,
        &mut layout_cx,
        PageBox::A4.width,
        PageBox::A4.width,
    );

    let boxes: Vec<f32> = doc.nodes[text]
        .text_layout()
        .expect("text should be shaped")
        .lines()
        .flat_map(|line| line.items())
        .filter_map(|item| match item {
            PositionedLayoutItem::InlineBox(inline_box) => Some(inline_box.width),
            _ => None,
        })
        .collect();
    assert_eq!(boxes.len(), 1, "水|A should have one autospace box");
    assert!(
        // cov:ignore: panic-message text is only executed when the assertion fails.
        (boxes[0] - 1.0).abs() < 0.001,
        // cov:ignore: panic-message text is only executed when the assertion fails.
        "expected 1px half-width ic gap, got {boxes:?}"
    );
}

#[test]
fn preshape_text_applies_negative_word_spacing_in_rayon_path() {
    use parley::{FontContext, LayoutContext};
    use raikiri_style::{build_rule_tree, cascade};

    fn first_shaped_width(inline_style: &str) -> f32 {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let p = doc.append_element(Some(body), "p", Style::default(), Some(inline_style));
        // `preshape_text` switches to Rayon at 32 jobs; 33 eligible text nodes
        // ensure this assertion exercises the parallel builder path.
        let mut text_nodes = Vec::new();
        for _ in 0..33 {
            text_nodes.push(doc.append_text(p, "A B"));
        }
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        let mut fonts = FontContext::new();
        let mut layout_cx = LayoutContext::<()>::new();
        preshape_text(
            &mut doc,
            &cr,
            &mut fonts,
            &mut layout_cx,
            PageBox::A4.width,
            PageBox::A4.width,
        );
        doc.nodes[text_nodes[0]]
            .text_layout()
            .expect("text should be shaped")
            .full_width()
    }

    let normal = first_shaped_width("word-spacing: 0px");
    let negative = first_shaped_width("word-spacing: -4px");
    // cov:ignore: assertion text is evaluated only when this test fails.
    assert!(
        negative < normal - 1.0,
        "negative non-ch word spacing should reduce parallel shaped width: normal={normal}, negative={negative}"
    );
}

#[test]
fn img_element_uses_resolver_intrinsic_size_when_css_gives_no_size() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    struct FixedSizeResolver(f32, f32);
    impl raikiri_traits::ReplacedResolver for FixedSizeResolver {
        fn resolve(
            &self,
            _req: raikiri_traits::ResolverRequest<'_>,
        ) -> Result<raikiri_traits::ResolvedIntrinsic, raikiri_traits::ResolverError> {
            Ok(raikiri_traits::ResolvedIntrinsic {
                intrinsic: raikiri_traits::IntrinsicBox::new(self.0, self.1),
                disposition: raikiri_traits::ResolveDisposition::Ok,
            })
        }
    }

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    // `<img>` is a replaced element: with `width: auto` its used width is
    // its intrinsic width (CSS 2.1 §10.3.2/10.3.4), which the
    // inline-block shrink-wrap path (`compute_inline_block_shrink_wrap`)
    // resolves via `leaf_intrinsic_size`. A plain `display: block` box
    // does not take this path — taffy's block algorithm stretch-fills a
    // non-replaced block child's width before the leaf measure closure
    // ever runs, so it would not observe the resolved intrinsic size
    // here (this is why the UA default for `<img>` is `inline-block`,
    // not `block`).
    let img = doc.append_element(
        Some(body),
        "img",
        Style::default(),
        Some("display:inline-block"),
    );
    doc.set_element_attributes(img, vec![("src".into(), "file:///x.png".into())]);

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");

    layout_single_page_with_resolver(
        &mut doc,
        &cascade,
        PageBox::A4,
        parley::FontContext::new(),
        &FixedSizeResolver(64.0, 32.0),
    )
    .unwrap();

    let layout = doc.nodes[img].unrounded_layout;
    assert_eq!((layout.size.width, layout.size.height), (64.0, 32.0));

    // Reuse the same cascade generation while the image resolver reports
    // different intrinsic dimensions. The image-resolution pass must dirty
    // Taffy caches before the second layout reuses them.
    layout_single_page_with_resolver(
        &mut doc,
        &cascade,
        PageBox::A4,
        parley::FontContext::new(),
        &FixedSizeResolver(48.0, 24.0),
    )
    .unwrap();
    let updated = doc.nodes[img].unrounded_layout;
    assert_eq!((updated.size.width, updated.size.height), (48.0, 24.0));
}

#[test]
fn with_resolver_refreshes_membership_before_the_image_pre_pass() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    struct NeverCalledResolver;
    impl raikiri_traits::ReplacedResolver for NeverCalledResolver {
        fn resolve(
            &self,
            req: raikiri_traits::ResolverRequest<'_>,
        ) -> Result<raikiri_traits::ResolvedIntrinsic, raikiri_traits::ResolverError> {
            unimplemented!(
                "an <img> inside <template> must never be resolved (was called for {})",
                req.url()
            )
        }
    }

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let tmpl = doc.append_element(Some(body), "template", Style::default(), None::<&str>);
    let _hidden_text = doc.append_text(tmpl, "template text");
    let img = doc.append_element(Some(tmpl), "img", Style::default(), None::<&str>);
    doc.set_element_attributes(img, vec![("src".into(), "file:///x.png".into())]);

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    assert!(
        doc.flags_dirty,
        "test premise: membership flags are stale entering layout"
    );

    layout_single_page_with_resolver(
        &mut doc,
        &cascade,
        PageBox::A4,
        parley::FontContext::new(),
        &NeverCalledResolver,
    )
    .expect("layout Ok");

    assert_eq!(doc.nodes[img].image_intrinsic_size(), None);
}

#[test]
fn layout_page_fragments_clips_long_block_into_split_fragments() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let tall = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("width:20px;height:120px"),
    );
    doc.append_text(tall, "tall");
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 50.0;

    let pages = layout_page_fragments(&mut doc, &cascade, page, FontContext::new())
        .expect("page fragment layout should succeed");
    let fragments: Vec<_> = pages
        .iter()
        .flat_map(|page| page.items.iter())
        .filter(|item| item.node_id.0 == tall as u64)
        .collect();
    assert!(fragments.len() >= 2);
    assert!(fragments.iter().all(|item| item.is_split()));
    assert!(
        fragments
            .windows(2)
            .all(|items| items[0].fragment_index < items[1].fragment_index)
    );
    assert!(
        fragments
            .iter()
            .all(|item| item.page_index < pages.len() as u32)
    );
    let total_height: f32 = fragments.iter().map(|item| item.rect.height).sum();
    assert!((total_height - 120.0).abs() < 0.01);
}

#[test]
fn layout_page_fragments_exposes_text_line_ranges_for_continuations() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let paragraph = doc.append_element(
        Some(body),
        "p",
        Style::default(),
        Some("width:20px;font-size:10px;line-height:10px"),
    );
    let text = doc.append_text(paragraph, "a ".repeat(200));
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 50.0;

    let pages = layout_page_fragments(&mut doc, &cascade, page, FontContext::new())
        .expect("page fragment layout should succeed");
    let mut text_items: Vec<_> = pages
        .iter()
        .flat_map(|page| page.items.iter())
        .filter(|item| item.node_id.0 == text as u64)
        .collect();
    text_items.sort_by_key(|item| item.fragment_index);
    assert!(text_items.len() >= 2);
    assert!(text_items.iter().all(|item| item.line_range.is_some()));
    assert_eq!(
        text_items
            .first()
            .and_then(|item| item.line_range)
            .map(|range| range.start),
        Some(0)
    );
    assert_eq!(
        text_items
            .last()
            .and_then(|item| item.line_range)
            .map(|range| range.end),
        Some(doc.nodes[text].text_layout().expect("text shaped").len() as u32)
    );
    assert!(text_items.windows(2).all(|items| {
        items[0].line_range.expect("range").end <= items[1].line_range.expect("range").start
    }));
}

#[test]
fn layout_pages_moves_fitting_text_block_for_orphans_and_widows() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    fn paginate(style: &str) -> (usize, f32, f32, usize) {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        // Leave room for exactly two 10px lines on page zero.
        let _lead = doc.append_element(Some(body), "div", Style::default(), Some("height:30px"));
        let paragraph = doc.append_element(Some(body), "p", Style::default(), Some(style));
        let text = doc.append_text(paragraph, "a\nb\nc");
        let following =
            doc.append_element(Some(body), "div", Style::default(), Some("height:10px"));

        let rules = build_rule_tree(&doc);
        let cascade = cascade(&doc, &rules).expect("cascade Ok");
        let mut page = PageBox::new();
        page.width = 100.0;
        page.height = 50.0;
        let slices = layout_pages(&mut doc, &cascade, page, parley::FontContext::new())
            .expect("pagination Ok");

        let paragraph_y = doc.nodes[paragraph].unrounded_layout.location.y;
        let following_y = doc.nodes[following].unrounded_layout.location.y;
        let line_count = doc.nodes[text].text_layout().expect("text shaped").len();
        (slices.len(), paragraph_y, following_y, line_count)
    }

    let common = "font-size:10px;line-height:10px;white-space:pre-line";
    let (pages, paragraph_y, following_y, line_count) =
        paginate(&format!("{common};orphans:3;widows:1"));
    assert_eq!(pages, 2, "the paragraph should continue on page two");
    assert!(
        paragraph_y >= 50.0 - 0.001,
        "orphans:3 should move the fitting paragraph intact to page two, got y={paragraph_y}"
    );
    assert!(
        following_y >= paragraph_y + 30.0 - 0.001,
        "following content must stay after the moved paragraph (paragraph y={paragraph_y}, following y={following_y})"
    );
    assert_eq!(line_count, 3);

    let (_, paragraph_y, _, _) = paginate(&format!("{common};orphans:1;widows:2"));
    assert!(
        paragraph_y >= 50.0 - 0.001,
        "widows:2 should also move a 3-line fitting paragraph when only one line would remain, got y={paragraph_y}"
    );
}

#[test]
fn autospace_edges_cross_plain_inline_elements() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let div = doc.append_element(Some(body), "div", Style::default(), None::<&str>);
    let left = doc.append_text(div, "国");
    let span = doc.append_element(Some(div), "span", Style::default(), Some("display:inline"));
    let right = doc.append_text(span, "A");
    // An atomic inline nested at the neighbor's edge stops the search
    // instead of being looked past.
    let atomic_div = doc.append_element(Some(body), "div", Style::default(), None::<&str>);
    let atomic_left = doc.append_text(atomic_div, "国");
    let outer = doc.append_element(
        Some(atomic_div),
        "span",
        Style::default(),
        Some("display:inline"),
    );
    let atomic = doc.append_element(
        Some(outer),
        "span",
        Style::default(),
        Some("display:inline-block"),
    );
    let _ = doc.append_text(atomic, "B");
    let _ = doc.append_text(outer, "A");
    doc.mark_in_document_flags();

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut parent_of = vec![None; doc.nodes.len()];
    for parent in 0..doc.nodes.len() {
        for &child in &doc.nodes[parent].children {
            parent_of[child] = Some(parent);
        }
    }
    assert_eq!(
        autospace_adjacent_edge_char(&doc, &cascade, &parent_of, right, -1),
        Some('国')
    );
    assert_eq!(
        autospace_adjacent_edge_char(&doc, &cascade, &parent_of, left, 1),
        Some('A')
    );
    assert_eq!(
        autospace_adjacent_edge_char(&doc, &cascade, &parent_of, atomic_left, 1),
        None
    );
}

#[test]
fn layout_owns_cross_inline_autospace_once() {
    use parley::{FontContext, PositionedLayoutItem};
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let div = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("font-family:Ahem;font-size:40px;text-autospace:normal"),
    );
    let left = doc.append_text(div, "国");
    let span = doc.append_element(Some(div), "span", Style::default(), Some("display:inline"));
    let right = doc.append_text(span, "A");

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cascade, PageBox::A4, FontContext::new()).expect("layout Ok");

    let inline_box_count = |idx: usize| {
        doc.nodes[idx]
            .text_layout()
            .expect("text shaped")
            .lines()
            .flat_map(|line| line.items())
            .filter(|item| matches!(item, PositionedLayoutItem::InlineBox(_)))
            .count()
    };
    assert_eq!(inline_box_count(left), 0);
    assert_eq!(inline_box_count(right), 1);
}

#[test]
fn autospace_boxes_follow_tab_rewrites() {
    use parley::{FontContext, PositionedLayoutItem};
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let style = "display:block;font-size:40px;text-autospace:normal;white-space:pre";
    let mut run = |tab_size: &str, text: &str, with_sibling: bool| {
        let block = doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some(format!("{style};tab-size:{tab_size}")),
        );
        let text = doc.append_text(block, text);
        if with_sibling {
            let sibling = doc.append_element(
                Some(block),
                "span",
                Style::default(),
                Some("display:inline"),
            );
            let _ = doc.append_text(sibling, "x");
        }
        text
    };
    // A lone text run in a `pre` block drops the tab for `tab-size:0`.
    let dropped = run("0", "\t国A", false);
    let dropped_ref = run("0", "国A", false);
    // With an inline sibling the tab expands to two spaces, which moves
    // the boundary by one byte and would otherwise land inside `国`.
    let expanded = run("2", "\t国A", true);
    let expanded_ref = run("2", "  国A", true);

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cascade, PageBox::A4, FontContext::new()).expect("layout Ok");

    let box_x = |idx: usize| {
        doc.nodes[idx]
            .text_layout()
            .expect("text shaped")
            .lines()
            .flat_map(|line| line.items())
            .filter_map(|item| match item {
                PositionedLayoutItem::InlineBox(inline_box) => Some(inline_box.x),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(box_x(dropped).len(), 1);
    assert_eq!(box_x(dropped), box_x(dropped_ref));
    assert_eq!(box_x(expanded).len(), 1);
    assert_eq!(box_x(expanded), box_x(expanded_ref));
}

#[test]
fn text_autospace_boxes_skip_default_ignorables_for_boundaries() {
    use raikiri_style::property::TextAutospace;

    let variation_selector = text_autospace_boxes("国\u{fe00}A", TextAutospace::Normal, "", 40.0);
    assert_eq!(
        variation_selector
            .iter()
            .map(|inline_box| inline_box.index)
            .collect::<Vec<_>>(),
        vec![6]
    );

    let disabled = text_autospace_boxes("国A", TextAutospace::NoAutospace, "", 40.0);
    assert!(disabled.is_empty());
}
