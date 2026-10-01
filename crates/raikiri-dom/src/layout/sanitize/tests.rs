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
    // Minimal regression check for `text-align: center`: In a block container of a single Text (`<p>` + 1
    // Text is below the 2-child threshold for a minimal line box, so it's a plain block path), the leading x
    // of the glyph run should be centered near the container width.
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
    // Regression check for `text-indent` basic wiring: In a block container of a single Text, the leading x
    // of the first line should be shifted to the right by the indent amount. Via parley `set_text_indent`
    // (basic only — hanging/each-line are always default due to parse layer drop).
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
        bounded_text_indent_amount(ComputedTextIndent::Px(f32::INFINITY), 100.0, None,),
        MAX_TAFFY_MAGNITUDE
    );
    assert_eq!(
        bounded_text_indent_amount(ComputedTextIndent::Px(f32::NEG_INFINITY), 100.0, None,),
        -MAX_TAFFY_MAGNITUDE
    );
    assert_eq!(
        bounded_text_indent_amount(ComputedTextIndent::Px(f32::NAN), 100.0, None),
        0.0
    );
    assert_eq!(
        bounded_text_indent_amount(
            ComputedTextIndent::Px(1.0),
            100.0,
            Some(MAX_TAFFY_MAGNITUDE * 2.0),
        ),
        MAX_TAFFY_MAGNITUDE
    );
    assert_eq!(
        bounded_text_indent_amount(
            ComputedTextIndent::Calc(CalcLengthPercentage {
                percent: 25.0,
                px: 10.0,
            }),
            200.0,
            None,
        ),
        60.0
    );
}

#[test]
fn text_indent_negative_protrudes_before_box() {
    // Negative indent protrudes before the start of the box (CSS Text 3 §8.1).
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
    // glyph x is layout-local at -20 (paint adds to box origin x=20 for a final x=0).
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
    // No indent means it remains as preshape (does not go through the indent path of realign).
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
#[ignore] // Explicitly run with cargo test -- --ignored
fn font_context_new_cost_is_reasonable() {
    let start = std::time::Instant::now();
    for _ in 0..10 {
        let _ = parley::FontContext::new();
    }
    let elapsed = start.elapsed();
    // If 10 total calls take less than 5 seconds, the current per-call new() implementation is acceptable (to
    // prevent the 10-run determinism test from timing out).
    assert!(
        elapsed.as_secs() < 5,
        "FontContext::new() too slow: 10x = {:?}",
        elapsed
    );
}

// ── Non-finite f32 guard ────────
//
// Check that +Inf / NaN from untrusted author CSS does not reach taffy / parley on **all 5 sites**. The
// reproducer comes from the original probe comment.
//
// The expected value should be written as the **specific value after clamping**, not "not non-finite" —
// because NaN is `NaN != NaN`, `assert_ne!(x, ...NAN)` will unconditionally pass, making it impossible to
// determine the presence or absence of a guard.

/// Returns the `taffy::Style` of the target element after passing through cascade →
/// `apply_computed_to_style`.
///
/// The fixture is a **non-body element** (`<p>`) — because `<body>`'s size will be clobbered by the
/// subsequent `apply_page_content_box_to_body`.
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

/// site 1 — `computed_length_percentage_to_taffy_length_percentage` (padding).
#[test]
fn nonfinite_padding_is_clamped_before_taffy() {
    use taffy::LengthPercentage;

    // Reproducer A': `1e40px` becomes +Inf during cssparser's f64→f32 conversion, and **passes** `v >= 0.0`
    // of `parse_padding_side` (inf >= 0.0 is true).
    assert_eq!(
        guarded_style_for("padding-top: 1e40px").padding.top,
        LengthPercentage::length(MAX_TAFFY_MAGNITUDE),
    );

    // Reproducer A: IEEE 754 `0.0 * inf = NaN` — NaN is produced by em multiplication. Previously,
    // `Em(_) => length(0.0)` arm absorbed this.
    assert_eq!(
        guarded_style_for("font-size: 0px; padding-top: 1e40em")
            .padding
            .top,
        LengthPercentage::length(0.0),
        "NaN は clamp では潰れないので is_nan() → 0.0 で処理する",
    );

    // The percentage side (`Percent` arm) also passes through the same guard. (`1e40%` makes raikiri's
    // `parse_percentage` +Inf by `* 100.0` cssparser's unit_value 1e38 — measured.)
    assert_eq!(
        guarded_style_for("padding-top: 1e40%").padding.top,
        LengthPercentage::percent(MAX_TAFFY_MAGNITUDE),
    );

    // Clamp even **finite but huge** values. All 3 cases above are already non-finite in f32 (`1e40` is +Inf
    // in f32), so even if the implementation is "simplified" to `if v.is_finite() { v } else { ... }`, they
    // all pass. `1e38%` is `Percent(1e38)` = **finite** (measured) and the fraction becomes 1e36, so only
    // this one kills that simplification.
    assert_eq!(
        guarded_style_for("padding-top: 1e38%").padding.top,
        LengthPercentage::percent(MAX_TAFFY_MAGNITUDE),
        "有限だが巨大な percentage も clamp する (is_finite() だけの実装への regression guard)",
    );
}

/// site 2 — `computed_length_percentage_or_auto_to_taffy_dimension` (width / height).
#[test]
fn nonfinite_size_is_clamped_before_taffy() {
    use taffy::Dimension;
    let s = guarded_style_for("width: 1e40px; height: 1e40%");
    assert_eq!(s.size.width, Dimension::length(MAX_TAFFY_MAGNITUDE));
    assert_eq!(s.size.height, Dimension::percent(MAX_TAFFY_MAGNITUDE));

    // NaN path (em × font-size 0).
    let n = guarded_style_for("font-size: 0px; width: 1e40em");
    assert_eq!(n.size.width, Dimension::length(0.0));
}

/// site 3 — `computed_length_percentage_or_auto_to_taffy_length_percentage_auto` (margin).
///
/// Margin **negative values are spec-valid** (CSS Box 3 §3.1), so clamping must be symmetric
/// (`[-MAX, MAX]`).
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
    // Negative path for `Percent` (`parse_margin_side` is allow-negative, so `-1e40%` passes parsing and
    // becomes `Percent(-inf)` — observed). If only the `Px` side is considered, a change that removes
    // `sanitize_taffy` from the `Percent` arm would pass tests unnoticed.
    assert_eq!(
        guarded_style_for("margin-left: -1e40%").margin.left,
        LengthPercentageAuto::percent(-MAX_TAFFY_MAGNITUDE),
    );
}

/// site 4 — `computed_length_to_taffy_length_percentage` (border-width).
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

/// site 5 — `cv.font_size.px()` of `preshape_text` → parley
/// `StyleProperty::FontSize`.
///
/// Observation is `Layout::height()` after shaping — if font-size is non-finite, line metrics are
/// corrupted and height also becomes non-finite.
///
/// # Removing the guard causes a **hang** instead of a fail
///
/// Observed (running `sanitize_finite` alone after replacing it with an identity function): sites 1-4
/// immediately fail with an assert, but this site does not terminate even after 25 seconds. The mechanism
/// is that `if next_x <= max_advance` of `parley-0.10.0/src/layout/line_break.rs` becomes always false in
/// `next_x = inf`, and `while self.break_next().is_some() {}` does not advance (shaping itself is
/// complete, and it is `break_all_lines` that spins).
///
/// Therefore, this test is **bounded by a worker thread + `recv_timeout`** — if the guard disappears, it
/// will crash as an **assert failure** instead of "CI job killed in 20 minutes" (which is
/// indistinguishable from an infra flake and also loses the results of subsequent tests for the same
/// binary).
#[test]
fn nonfinite_font_size_is_clamped_before_parley() {
    // Pass parent/child inline styles separately — `em` of `font-size` is based on the **parent's** computed
    // font-size (CSS Values 4 §6.1.1), so to create NaN (`0 * inf`), the multiplier `font-size: 0px` must be
    // on the parent side. Sites 1-4 can be created with one element because the multiplier is on the same
    // element, but font-size alone requires two elements.
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

    /// A wrapper that changes a hang when a guard disappears into a **bounded-time failure**.
    ///
    /// What is bounded is the **test**, not the process — even if it times out, the worker thread remains
    /// spinning (because parley has no cancellation, and there is no way to interrupt `break_all_lines`).
    /// There is no practical harm as the entire process crashes when the test binary exits, but this is the
    /// extent of what "bounded" means.
    fn shaped_height_bounded(parent_inline: Option<&str>, child_inline: &str) -> f32 {
        use std::sync::mpsc::RecvTimeoutError;

        let parent = parent_inline.map(str::to_owned);
        let child = child_inline.to_owned();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(shaped_height(parent.as_deref(), &child));
        });
        // Do not confuse `Timeout` with `Disconnected` — `shaped_height` contains `.expect("cascade Ok")` /
        // `.unwrap()`, so if the worker panics, `tx` is dropped and `Disconnected` returns **in a few ms**.
        // Reporting this as "did not finish in 30 seconds" would cause a cascade regression to be investigated as
        // a guard disappearance, which is the inverse of the purpose of introducing this wrapper (to distinguish
        // hangs from normal failures).
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

    // (a) +Inf font-size. `1e40px` is +Inf due to f64→f32 conversion in cssparser.
    let inf_px = shaped_height_bounded(None, "font-size: 1e40px");
    assert!(
        inf_px.is_finite(),
        "font-size +Inf (px 由来) が parley に届いた: {inf_px}"
    );

    // (b) +Inf font-size (from em compounding). The parent is the initial 16px, so `16.0 * inf = +Inf` —
    // **not NaN**.
    let inf_em = shaped_height_bounded(None, "font-size: 1e40em");
    assert!(
        inf_em.is_finite(),
        "font-size +Inf (em 由来) が parley に届いた: {inf_em}"
    );

    // (c) **NaN** font-size — `0.0 * inf` (IEEE 754). `em` of `font-size` is based on the **parent's**
    // computed font-size (CSS Values 4 §6.1.1), so the multiplier `font-size: 0px` is applied to the parent.
    // Sites 1-4 can be created with one element because the multiplier is applied to the same element, but
    // font-size requires two elements.
    //
    // **The `is_nan()` branch deletion mutation does not die in this case (measured).** As long as the guard
    // is alive, parley receives 0.0, not NaN, so **even if parley's NaN tolerance changes, it won't be
    // noticed here** (it's not an "upstream canary"). Detecting mutations that kill the `is_nan()` branch
    // requires 4 e2e tests from sites 1-4 and `sanitize_finite_maps_nan_to_zero`, for a total of 5 tests
    // (measured by mutation testing).
    //
    // There are two reasons to still include it:
    //   1. Pinning that one path to create NaN (parent `0px` × child `em`) can be constructed with e2e.
    //      Unlike sites 1-4, it cannot be created with one element.
    //   2. Insurance against a compound regression of "guard disappearance × upstream NaN tolerance change"
    //      (individually, other tests would catch both).
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
/// The doc comment on `MAX_FONT_SIZE_PX` ("removing the guard... does not terminate even after 25
/// seconds") reads as "non-finite font-size ⇒ hang" in general —
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
/// `MAX_FONT_SIZE_PX`'s doc comment ("removing the guard... does not terminate even after 25 seconds"):
/// `font_size = +Inf` reaching parley directly (bypassing
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

// ── Unit test for the guard function itself ───────────────────────────────────
//
// E2E tests can cause site 5 to hang, and each test involves FontContext construction. The guard's
// arithmetic is a pure function, so it's called directly (a few ms, cannot hang).

#[test]
fn sanitize_finite_maps_nan_to_zero() {
    // `f32::clamp` returns NaN as NaN, so without this branch, NaN would pass through.
    let mut diag = Vec::new();
    assert_eq!(sanitize_finite(f32::NAN, -1.0, 1.0, "test", &mut diag), 0.0);
    assert_eq!(
        sanitize_finite(f32::NAN, 0.0, MAX_FONT_SIZE_PX, "test", &mut diag),
        0.0
    );
    // Since both were actually clamped (NaN != 0.0), 1 event each of `LayoutWarn::NonFiniteClamped` will be
    // accumulated.
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
    // At sites with a lower bound of 0.0 (font-size), -Inf falls to 0.0.
    assert_eq!(
        sanitize_finite(f32::NEG_INFINITY, 0.0, MAX_FONT_SIZE_PX, "test", &mut diag),
        0.0
    );
    assert_eq!(diag.len(), 3, "3 回とも clamp が発火する (全て非有限入力)");
}

#[test]
fn sanitize_taffy_clamps_out_of_range_finite_values() {
    // Even if finite, out-of-range values are brought within range (it's not just about "making them
    // finite").
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
    // Normal values are passed through bit-identically (assuming VRT is pixel-exact).
    let mut diag = Vec::new();
    for v in [0.0_f32, 1.0, -1.0, 16.0, 793.7008, MAX_TAFFY_MAGNITUDE] {
        assert_eq!(
            sanitize_taffy(v, "test", &mut diag),
            v,
            "in-range value must pass through: {v}"
        );
    }
    // Nothing is accumulated when within range (clamping is effectively a no-op) — this check is designed to
    // avoid per-node spam (see `sanitize_finite`'s doc).
    assert!(
        diag.is_empty(),
        "in-range value must not push a LayoutWarn: {diag:?}"
    );
}

#[test]
fn sanitize_line_height_clamps_non_finite_and_out_of_range_number() {
    // site 7: `ComputedLineHeight::Number` — grammar `<number [0,∞]>`
    // So the lower bound is 0.0, and the upper bound is `MAX_LINE_HEIGHT_NUMBER`. NaN inherits the
    // `sanitize_finite` fallback of `0.0`, similar to other length-related sites (reason why a dedicated
    // fallback like font-weight is not needed: `0` is a grammatically valid value for line-height's unitless
    // number, and line-height does not have the `400.0` circumstances of font-weight — `0.0` being outside
    // the valid range).
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
    // Percentages in `<length-percentage [0,∞]>` are already absolute in px at the computed layer (see
    // `ComputedLineHeight::Length` documentation), so here we only look at the valid range for px.
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

// ── generalized diagnostic channel via crate::diag ──
// This is a unique unit test for LayoutWarn, which generalizes the FontWarn observer pattern in fonts.rs.
// It corresponds to the `observer_fires_*` test group in fonts.rs.

/// `emit_layout_warn` calls the observer if it is `Some`, and `eprintln!` does not — a contract
/// symmetrical to `emit_warn` in fonts.rs (both should behave the same since they go through
/// `crate::diag::emit_warn_via`). // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests
/// (#[test]-item doc) — rustdoc-blind, confirmed via intentionally breaking it to verify
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
    // Float literals cannot be written in patterns (`illegal_floating_point_literal_pattern` is
    // deny-by-default), so only the variant/site is checked with matches!, and the value of `clamped` is
    // separately bound with `if let` and asserted.
    assert!(matches!(
        collected[0],
        LayoutWarn::NonFiniteClamped { site: "test", .. }
    ));
    if let LayoutWarn::NonFiniteClamped { clamped, .. } = collected[0] {
        assert_eq!(clamped, 0.0);
    }
}

/// `observer == None` instead of `eprintln!` — only confirm that calling it does not panic (stderr content
/// is not captured, and the corresponding path in fonts.rs is similarly untested).
#[test]
fn emit_layout_warn_falls_back_to_eprintln_when_none() {
    let mut observer: LayoutWarnObserver<'_> = None;
    emit_layout_warn(&mut observer, LayoutWarn::Truncated { suppressed: 3 });
}

/// `push_layout_warn` folds events exceeding `LAYOUT_WARN_CAP` into a single running `Truncated` counter
/// instead of individual events — a cap to bound the buffer and subsequent eprintln! replay in cases of
/// "pathological input where every field is clamped every time" (see doc).
#[test]
fn push_layout_warn_collapses_past_cap_into_truncated_counter() {
    let mut diag: Vec<LayoutWarn> = Vec::new();
    // Up to the cap, these are real events.
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

    // Events exceeding the cap do not extend the Vec; they are aggregated into the Truncated counter at the
    // end.
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

/// Check that `LayoutWarn`'s `Display` produces human-readable strings for both variants (guaranteeing
/// that `crate::diag::emit_warn_via`'s `eprintln!` fallback becomes a readable line, confirmed by
/// intentionally breaking it // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests (#[test]-item
/// doc) — rustdoc-blind).
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

/// Pin that the clamp constant is **within the range asserted by the doc**.
///
/// Do not use `assert_eq!` with a literal, as it is tautological — if the constant is rewritten, the test
/// will also be rewritten, detecting nothing. Write the **relational expression** that the doc cites as
/// its basis.
#[test]
fn clamp_limits_are_in_the_documented_range() {
    // Taffy geometry: that it is within the LayoutUnit upper bound range (1e7-1e8 px) of implementations
    // reported by CSSWG issue #4552.
    assert!(
        (1e7..=1e8).contains(&MAX_TAFFY_MAGNITUDE),
        "MAX_TAFFY_MAGNITUDE は CSSWG #4552 の 1e7..=1e8 px 帯に収まること: {MAX_TAFFY_MAGNITUDE}"
    );
    // The doc asserts more strongly that "taking the **lower bound** of the range = below the upper limit of
    // any of the 3 engines." The minimum is old-Edge's `2^31 / 100 ≈ 2.15e7 px`.
    assert!(
        MAX_TAFFY_MAGNITUDE <= (i32::MAX / 100) as f32,
        "MAX_TAFFY_MAGNITUDE は 3 engine の最小上限 (2^31/100 ≈ 2.15e7 px) 以下であること: {MAX_TAFFY_MAGNITUDE}"
    );
    // Font-size: **more than 1 digit** below `i32::MAX / 64 ≈ 3.36e7` ppem where skrifa's 16.16 fixed
    // conversion saturates (as asserted by the doc).
    assert!(
        MAX_FONT_SIZE_PX * 10.0 < (i32::MAX / 64) as f32,
        "MAX_FONT_SIZE_PX は skrifa の saturation 点より 1 桁以上下であること: {MAX_FONT_SIZE_PX}"
    );
}

// Output guard: nested percentage
//
// While the input guard (sites 1-4 above) makes the f32 values entering the bridge finite, percentages
// are resolved against the containing block at the used value layer (Taffy) and compound with each nest.
// Therefore, the **output** can become non-finite again. Below is the pin for that output guard
// (`sanitize_taffy_layout`).

/// Predicate for the invariant guaranteed by [`sanitize_taffy_layout`] — all f32 fields of
/// [`taffy::Layout`] are finite.
///
/// Examine all fields, not just the 4 fields that paint actually reads (for the same reason as the guard
/// side).
///
/// Using exhaustive destructuring instead of `..` is for the same reason as the guard side — to avoid
/// creating an asymmetry where, if Taffy adds an f32 field, only the guard side (exhaustive literal) would
/// result in a compile error, while the **predicate side would silently only examine the old fields**.
fn layout_all_finite(l: &TaffyLayout) -> bool {
    fn size_ok(s: Size<f32>) -> bool {
        s.width.is_finite() && s.height.is_finite()
    }
    fn rect_ok(r: Rect<f32>) -> bool {
        r.left.is_finite() && r.right.is_finite() && r.top.is_finite() && r.bottom.is_finite()
    }
    let TaffyLayout {
        // `order` is u32 — not subject to guard (`sanitize_taffy_layout`'s doc).
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

/// Pass a document with `<div>`, which has `decl` under `<html><body>`, nested `depth` levels deep, to
/// [`layout_single_page`], and return the `unrounded_layout` for **each level** in order from shallowest
/// to deepest.
///
/// The starting point is the depth range test setup for the probe material, but **we do not recreate the
/// document for each depth** — the chain for depth `N` already contains nodes for each depth from 1 to
/// `N`, and there is no reason to pay the `FontContext::new()` (checking
/// `font_context_new_cost_is_reasonable` 10 times in less than 5 seconds = not cheap at all) for each
/// depth, which the probe used to pay.
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

/// Before the fix, `unrounded_layout` returned to non-finite at the depth below. In the actual measurement
/// of the depth range of the probe material RAWDATA.txt, **base (before guard) / head (after input-side
/// guard) matched perfectly** = evidence that this is a hole not closed by the input-side guard:
///
/// | decl | test setup | this test setup (actual measurement) |
/// |---|---|---|
/// | `width: 1e9%` | 6 | 6 |
/// | `width: 100000%` | 12 | 12 |
/// | `width: 10000%` | 18 | 18 |
/// | `width: 1000%` | 36 | 36 |
/// | `width: 200%` | Not reached | Not reached |
/// | `padding-left: 1e9%` | 4 | 5 |
/// | `padding-left: 100000%` | 8 | 9 |
/// | `padding-left: 1000%` | 25 | 25 |
/// | `padding-left: 200%` | Not reached | Not reached |
///
/// (The ±1 in the two `padding-left`-related lines is due to the difference in the two test setups. The
/// probe recreates documents for each depth, so the deepest level becomes a leaf. However, this test setup
/// extends a single chain to the deepest level and examines each stage, so the same stage becomes a
/// container. **However, the mechanism has not been identified** — if this structural difference were the
/// cause, all three padding-related lines should be shifted, but `padding-left: 1000%` matches 25/25. The
/// numerical values themselves are reproducible, and the values in this test setup column were measured by
/// removing only the `sanitize_taffy_layout` call from `set_unrounded_layout`. The four `width`-related
/// lines match perfectly.)
///
/// After the fix, all will be "Not reached".
///
/// **The inspection width of 45 is a value aligned with the range in the table, not a guaranteed upper
/// limit.** This test only checks the **inspected point**: "within the range of these 9 declarations
/// viewed up to depth 45, all saved fields are finite." Depth independence itself does not come from the
/// test — the basis lies in the **structural argument of the choke point**, which is that
/// `sanitize_taffy_layout` is placed at the sole write path from taffy to the arena.
/// `nested_percentage_output_stays_finite_far_past_the_range` also merely adds "an example considerably
/// deeper than the range" and does not check for universal depth independence. Therefore, do not lower
/// this 45 as a "safe upper limit" (the justification for lowering it lies in the structure, not the
/// test).
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

/// Pin that the saved value is finite even at depth 96.
///
/// Since `nested_percentage_output_is_finite_through_probe_range_depth` only looks up to depth 45, aligned
/// with the table above, we will look at the shallowest broken `padding-left: 1e9%` (depth 4 in test setup
/// / depth 5 in this test setup) before the fix, as an additional point more than double its range width.
///
/// **This test does not check depth independence** — a finite-depth test can always only show "finite at
/// the depths examined." The basis for depth independence is the **structural argument of a choke point**,
/// that `sanitize_taffy_layout` is placed as the sole write path from taffy to the arena, not this test.
/// This test is positioned as a sanity check for that structural argument.
///
/// **This test does not (cannot) exclude proposals (a) / (b)** — the breakdown depth `35.6 / log10(F)`
/// (table in `MAX_TAFFY_MAGNITUDE`'s doc) for the input-side fraction bound `F` can only catch `F >= 2.35`
/// at depth 96, and the smallest `F = 2.0` that preserves `width: 200%` is `D = 118`, which **passes this
/// test**. A finite-depth test cannot, in principle, exclude (a). The basis for rejecting (a) is the
/// argument in `MAX_TAFFY_MAGNITUDE`'s doc that "depth independence requires `F <= 1`, and that kills
/// `width: 200%`," not this test.
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

/// Field-unit behavior of `sanitize_taffy_layout` (the two tests above only check "is finite," so we check
/// here what value it falls to).
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

    // `order` is u32, so it's not subject to guarding — let it pass through.
    assert_eq!(s.order, 7, "order は clamp 対象ではない");

    // 16 fields are non-finite/out of range (matches what the individual asserts below count): location
    // 2 + size 2 + scrollable_overflow_rect 2 + scrollbar_size 1 + border 3 + padding 3
    // + margin 3. The 4 in-range fields (scrollbar_size.height / border.bottom / padding.bottom / margin.top)
    //   are not accumulated (designed to avoid per-node spam).
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
    // NaN is not crushed by clamp, so `is_nan()`
    // → 0.0 (sanitize_finite).
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

    // Pinning that one example of Layout that fits within `[-MAX, MAX]` is bit-wise invariant. This does not
    // indicate "zero impact on normal layout" — there is no impact only when it fits within the band; for
    // inputs that go outside (such as `width: 200%` × 14 levels of nesting), the value changes (see
    // `MAX_TAFFY_MAGNITUDE`'s "band where clamp actually takes effect" section).
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

// Semantic invariant fallback
//
// The `sanitize_taffy_layout_*` test group above only checks up to "all fields are finite" (the preceding
// scope). The following pins the layer that `enforce_layout_invariants` handles, where "fields are finite
// but the parent-child relationship is semantically broken." The numerical values of the two probes
// (border-box padding overflow / negative margin overflow) in `enforce_layout_invariants`'s doc are also
// formally promoted to assertions here.

/// Direct pin of invariant 1 (content box non-negative). For fields, values that pass through
/// `sanitize_taffy_layout` (both `size` and `padding` are within `[-MAX, MAX]`) are used to create
/// `content_box_width() < 0.0`, and `enforce_layout_invariants` is confirmed to (a) zero out that node,
/// (b) zero out both the **child within that subtree**, and (c) push `LayoutWarn`. Without (b), it would
/// be a half-baked state where "only the parent is zeroed out, and the child remains with old values based
/// on the broken parent," which would newly violate invariant 2.
#[test]
fn content_box_violation_resets_subtree_to_zero_layout() {
    let mut doc = Document::new();
    let parent = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
    let child = doc.append_element(Some(parent), "div", Style::default(), None::<&str>);

    // padding.left+right (40.0) is greater than size.width (10.0) =>
    // content_box_width() = 10.0 - 40.0 = -30.0 < 0.0.size / padding
    // Both are individually within `[-MAX_TAFFY_MAGNITUDE, MAX_TAFFY_MAGNITUDE]`, so
    // `sanitize_taffy_layout`'s field-level clamp does not stop this—a direct reproduction of something that
    // can actually happen.
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
    // The child itself has no layout problems (if we ignore the broken parent) — this is material to confirm
    // that the entire subtree is zeroed out.
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

/// **Direct check of behavior reversed by this change (formerly
/// `saturated_child_outside_parent_resets_subtree_to_zero_layout`)**.
/// Use a directly constructed maximally-uncontained value (`location.x == MAX_TAFFY_MAGNITUDE` vs.
/// `parent.size.width == 100.0`) that satisfies the gate of `child_within_parent_border_box` ("its own
/// `child.location` is saturated") and would have been reset as a containment violation in the old
/// implementation. Since this change made `axis_ok` unconditionally `true` regardless of sign, this
/// fixture is no longer reset — even though it is clearly outside the parent border box. This is kept as a
/// regression check to show that "even by directly constructing the fixture, this invariant can no longer
/// be broken" (see the "Reason why it became unconditionally accepted regardless of sign" section in the
/// doc of `child_within_parent_border_box`, and it is intended to be used as material for a future
/// follow-up (a problem explicitly deferred separately) to determine "whether this check itself should be
/// maintained" to confirm "what the current function actually does").
#[test]
fn saturated_child_outside_parent_is_not_reset() {
    let mut doc = Document::new();
    let parent = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
    let child = doc.append_element(Some(parent), "div", Style::default(), None::<&str>);
    let grandchild = doc.append_element(Some(child), "div", Style::default(), None::<&str>);

    // Parent is normal (100x100, not saturated).
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
    // child.location.x is exactly at the saturation boundary (MAX_TAFFY_MAGNITUDE) — it cannot possibly fit
    // within the parent (100x100). After this change, this "clearly not fitting" fact is no longer a reason
    // for reset.
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
    // grandchild is a normal value based on child — material to confirm that it has not been reset throughout
    // the subtree (see assert below).
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

/// A check that the gate for invariant 2 is **not unconditional** — layout overflow that CSS normally
/// allows (a child overflowing to the right/bottom/left with a small parent + negative margin) is actually
/// laid out by `layout_single_page`'s full pipeline, and `enforce_layout_invariants` does not incorrectly
/// fall back. The numerical values are the same as the actual measured values recorded in
/// `enforce_layout_invariants`'s doc (already confirmed by probe).
///
/// Without this, we cannot detect a regression that "makes invariant 2 an unconditional check" (a spec
/// violation — breaking legitimate overflow layout).
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

/// Pinning that fallback does not occur even if the gate (`child.location` itself is saturated) is true,
/// but the child actually fits within the parent (exactly at the boundary).
///
/// **History before this change**: This test was originally old
/// `saturated_child_outside_parent_resets_subtree_to_zero_layout`
/// This was a check (renamed to `saturated_child_outside_parent_is_not_reset` and inverted with this
/// change) paired with "saturated AND containment violation → reset" to indicate that "we are checking the
/// conjunction 'gate AND containment violation' rather than just 'gate'." With this change, `axis_ok`
/// unconditionally became `true` regardless of its sign, so this conjunction no longer holds — a reset
/// will not occur regardless of actual containment (whether it's exactly within the boundary or clearly
/// outside). The assert in this test itself continues to pass (because this fixture happened to be a
/// "contained" case), but that's not because "containment was checked and passed," but because
/// "containment is not being looked at in the first place." For the paired regression check demonstrating
/// this fact, see `saturated_child_outside_parent_is_not_reset`. For a similar check using an actual
/// nested percentage chain, see `nested_percentage_wide_child_chain_is_not_reset` (a regression that was
/// the direct reason for choosing the current `child_within_parent_border_box`, which only looks at
/// origin, not extent).
#[test]
fn saturated_but_contained_layout_is_not_reset() {
    let mut doc = Document::new();
    let parent = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
    let child = doc.append_element(Some(parent), "div", Style::default(), None::<&str>);

    // Create a case where the parent's size is also exactly at the saturation boundary — the child's location
    // fits perfectly within it (boundary value, OK by `<=`).
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
    // child.location itself is exactly at the saturation boundary — trigger the gate
    // (`child_within_parent_border_box` axis unit gate). Since it's the same value as parent.size, it's
    // exactly "contained" at the boundary by `<=`.
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

/// **Check whose behavior was inverted by this change (formerly named**
/// `saturated_location_with_legitimate_negative_margin_on_other_axis_is_not_reset`,
/// Former assertion: "The detection capability of the `y` axis is maintained."
///
/// Before this change, this fixture (`y` axis saturated and actually not contained by parent = true
/// violation, `x` axis not saturated, legitimate negative margin `-30`) was meaningful as a check to
/// demonstrate the detection capability of the `y` axis — showing that re-checking only the `y` axis could
/// explain the reason for the reset, and that the legitimate negative value of the `x` axis would be
/// ignored. With this change, `axis_ok` became unconditionally `true` regardless of sign, so the `y` axis
/// no longer re-checks containment just by being saturated — the fact of "not being contained" itself can
/// no longer be a reason for a reset.
///
/// This test is now a closely related check making almost the same assertion as
/// `saturated_but_contained_axis_with_legitimate_negative_margin_on_other_axis_is_not_reset` (neither axis
/// is a reason for reset). The only difference is the value of the `y` axis — here, `y` is **actually not
/// contained by the parent** (`MAX_TAFFY_MAGNITUDE > 100.0`), whereas there, it is contained exactly at
/// the boundary. The fact that neither is reset shows that "whether it is contained or not" no longer
/// affects the result at all.
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
    // y-axis: child.location.y is exactly at the saturation boundary and is actually not contained by the
    // parent (height=100.0). x-axis: legitimate overflow due to normal negative margin (`-30`) — not
    // saturated. After this change, neither axis is a reason for reset.
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

/// **Direct regression pin to the previous point** — the previous
/// `saturated_axis_outside_parent_with_legitimate_negative_margin_on_other_axis_is_not_reset`
/// (Old name before this change
/// `saturated_location_with_legitimate_negative_margin_on_other_axis_is_not_reset`)
/// It was pointed out at the time that "the reset could be explained by the `y` axis alone," making it
/// impossible to distinguish whether the `x` axis was correctly ignored by the implementation (because
/// both the old axis-mixing implementation and the axis-unit implementation at the time would result in
/// the same "reset").
///
/// This test uses a distinguishable fixture: the `y` axis is set to a value that "just reaches the
/// saturation boundary but still fits within the parent" (= not a true violation), and the `x` axis is
/// given a legitimate negative margin (`-30`, not saturated).
///
/// - **Current implementation (unconditionally accepts regardless of sign)**: The `x` axis is not
///   saturated, so it's unconditionally OK. The `y` axis is saturated, but containment is not re-checked,
///   so it's also unconditionally OK. -> **Not reset**.
/// - **Implementation just before this change (axis-unit, branches by sign, re-checks `<=` only for
///   positive direction)**: The `x` axis is unconditionally OK. The `y` axis is saturated and positive, so
///   it's re-checked, but it actually fits within the parent, so it's OK. -> Also not reset (this test's
///   result does not change before and after this change — what changed was
///   `saturated_child_outside_parent_is_not_reset` and
///   `saturated_axis_outside_parent_with_legitimate_negative_margin_on_other_axis_is_not_reset`
///   cases where containment is actually broken).
/// - **Old axis-mixing implementation (if any of `parent.size` / `child.size` / `child.location` is
///   saturated, unconditionally checks both `x` / `y`)**: Saturation of `y` opens the gate, and `-30.0`
///   fails the check for `x >= 0.0` -> **Incorrectly reset**.
///
/// In other words, this test passing is direct evidence that "the legitimate negative margin of the `x`
/// axis is not causing a reset," and if there were a regression to the old axis-mixing implementation,
/// this test alone would fail.
#[test]
fn saturated_but_contained_axis_with_legitimate_negative_margin_on_other_axis_is_not_reset() {
    let mut doc = Document::new();
    let parent = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
    let child = doc.append_element(Some(parent), "div", Style::default(), None::<&str>);

    // This is to ensure that parent.size.height is also exactly at the saturation boundary, so that
    // child.location.y (also at the saturation boundary) fits perfectly there (`<=`, exactly at the
    // boundary). The width is a normal value (the x-axis is not saturated, so it's irrelevant).
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
    // x-axis: legitimate negative margin (`-30`), not saturated — this axis should be unconditionally treated
    // as OK. y-axis: exactly at the saturation boundary, but since it's equal to parent.size.height, it
    // actually fits (not a true violation) — this also shows that "saturated" alone is not a reason for a
    // reset.
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

/// **Direct regression check to the previous point (part 2)** — The original scenario itself that was
/// pointed out: `size` in an **unrelated different location** (here, `parent` itself) of the same subtree
/// is saturated, while **the child itself is not saturated at all** but has a legitimate negative margin
/// (`location.x = -30`). The old gate (which would check if any one of `parent.size` / `child.size` /
/// `child.location` was saturated) incorrectly reset this — directly reproducing the field-unit problem
/// explained in section 1 of `child_within_parent_border_box`'s doc, "Reasons for limiting the gate to
/// axis units and `child.location` itself".
#[test]
fn saturated_parent_size_with_legitimate_negative_margin_child_is_not_reset() {
    let mut doc = Document::new();
    let parent = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
    let child = doc.append_element(Some(parent), "div", Style::default(), None::<&str>);

    // parent.size is saturated — assuming an unrelated cause (e.g., percentage compounding that occurred in
    // another subtree), this saturation is unrelated to this child.
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
    // The child itself is completely normal — neither location nor size is saturated, a legitimate overflow
    // due to a normal negative margin (same form as `legitimate_negative_margin_overflow_is_not_reset`).
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
/// doc, "The reason why it became unconditional accept regardless of sign") treats
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
    // child.location.x is exactly on the **negative** side of the saturation boundary — parent.size is
    // a normal, unsaturated value (100.0). Since the old implementation (`>= 0.0 && <= parent.size`) was
    // identically false for negative values, this fixture would
    // always be reset in the old implementation (no negative number satisfies `>= 0.0`).
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

    // The assert messages from this point onward are intentionally written on a single physical line (not
    // multiple lines with backslash continuation, which is the convention for other tests in this file) —
    // because `code_only()` in `scripts/lib/patch_coverage.py` resets the string state for each line, if
    // there is `)`/`]`/`}` (as plain text) in the middle of a backslash-continued line, the block scope
    // calculation for `cov:ignore` will be interrupted there, subsequent lines will not be exempt, and patch
    // coverage will incorrectly FAIL (this has actually been encountered). Folding them into one line is a
    // workaround for this, not just a style variation.
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
    let frag = doc.allocate_template_fragment_root(tmpl);
    let _hidden_text = doc.append_text(frag, "template text");
    let img = doc.append_element(Some(frag), "img", Style::default(), None::<&str>);
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

/// An element below an ifc root carries the bounding box of its line pieces,
/// and a piece that does not end the element has no padding or border on that
/// side: the box can be narrower than the element's two paddings. The check
/// leaves such elements alone instead of zeroing them as broken.
#[test]
fn an_element_of_an_ifc_subtree_is_not_zeroed_for_a_narrow_content_box() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let inner = doc.append_element(Some(body), "span", Style::default(), None::<&str>);
    doc.nodes[inner].flags.insert(NodeFlags::IN_IFC_SUBTREE);
    let mut layout = TaffyLayout::with_order(0);
    layout.size = Size {
        width: 10.0,
        height: 10.0,
    };
    layout.padding = Rect {
        left: 30.0,
        right: 30.0,
        top: 0.0,
        bottom: 0.0,
    };
    doc.nodes[inner].unrounded_layout = layout;
    enforce_layout_invariants(&mut doc, body);
    assert_eq!(doc.nodes[inner].unrounded_layout.size.width, 10.0);
    assert!(doc.layout_warnings.is_empty());
}
