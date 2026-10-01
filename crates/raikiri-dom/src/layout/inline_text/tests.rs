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
fn vertical_align_linebox_extent_splits_ascent_and_descent() {
    assert_eq!(
        vertical_align_linebox_extent(VerticalAlign::Length(Length::Px(12.0)), 16.0),
        (12.0, 0.0)
    );
    assert_eq!(
        vertical_align_linebox_extent(VerticalAlign::Length(Length::Px(-4.0)), 16.0),
        (0.0, 4.0)
    );
    assert_eq!(
        vertical_align_linebox_extent(VerticalAlign::Super, 16.0),
        (16.0 / 3.0, 0.0)
    );
    assert_eq!(
        vertical_align_linebox_extent(VerticalAlign::Sub, 16.0),
        (0.0, 16.0 / 5.0)
    );
    assert_eq!(
        vertical_align_linebox_extent(VerticalAlign::Baseline, 16.0),
        (0.0, 0.0)
    );
    assert_eq!(
        vertical_align_linebox_extent(VerticalAlign::Length(Length::Px(f32::NAN)), 16.0),
        (0.0, 0.0)
    );
}
#[test]
fn collect_inline_linebox_extents_composes_shifts_and_stops_at_boundaries() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(
        Some(body),
        "p",
        Style::default(),
        Some("display:block;font-size:16px"),
    );
    let outer = doc.append_element(
        Some(p),
        "span",
        Style::default(),
        Some("display:inline;vertical-align:super"),
    );
    let inner = doc.append_element(
        Some(outer),
        "span",
        Style::default(),
        Some("display:inline;vertical-align:super"),
    );
    doc.append_text(inner, "nested");
    let block_child = doc.append_element(
        Some(outer),
        "div",
        Style::default(),
        Some("display:block;vertical-align:super"),
    );
    let block_inner = doc.append_element(
        Some(block_child),
        "span",
        Style::default(),
        Some("display:inline;vertical-align:super"),
    );
    doc.append_text(block_inner, "block");
    let atomic = doc.append_element(
        Some(p),
        "span",
        Style::default(),
        Some("display:inline-block;vertical-align:super"),
    );
    let atomic_child = doc.append_element(
        Some(atomic),
        "span",
        Style::default(),
        Some("display:inline;vertical-align:super"),
    );
    doc.append_text(atomic_child, "atomic");

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    let mut synthetic_line_roots = vec![false; doc.nodes.len()];
    let mut extents = SyntheticLineboxExtents::default();

    collect_inline_linebox_extents(
        &doc,
        &cr,
        &synthetic_line_roots,
        outer,
        16.0,
        0.0,
        &mut extents,
    );
    assert_eq!(extents.leading, 16.0 / 3.0 + 16.0 / 3.0);
    assert_eq!(extents.trailing, 0.0);

    // A nested synthetic root owns the extent of its descendants; the
    // containing root sees only that wrapper's outer vertical-align shift.
    synthetic_line_roots[outer] = true;
    extents = SyntheticLineboxExtents::default();
    collect_inline_linebox_extents(
        &doc,
        &cr,
        &synthetic_line_roots,
        outer,
        16.0,
        0.0,
        &mut extents,
    );
    assert_eq!(extents.leading, 16.0 / 3.0);
    assert_eq!(extents.trailing, 0.0);

    // An inline-block contributes its own outer shift, not its internal
    // line-box shifts, to the containing line root.
    extents = SyntheticLineboxExtents::default();
    collect_inline_linebox_extents(
        &doc,
        &cr,
        &synthetic_line_roots,
        atomic,
        16.0,
        0.0,
        &mut extents,
    );
    assert_eq!(extents.leading, 16.0 / 3.0);
    assert_eq!(extents.trailing, 0.0);
}

#[test]
fn padding_with_linebox_extent_preserves_percent_components() {
    let mut doc = Document::new();
    let without_extra = padding_with_linebox_extent(
        &mut doc,
        ComputedLengthPercentage::Px(4.0),
        0.0,
        "test-linebox-padding",
    );
    assert_eq!(without_extra, LengthPercentage::length(4.0));
    assert!(doc.calc_values.is_empty());

    let _with_extra = padding_with_linebox_extent(
        &mut doc,
        ComputedLengthPercentage::Percent(25.0),
        8.0,
        "test-linebox-padding",
    );
    assert_eq!(doc.calc_values.len(), 1);
    assert_eq!(doc.calc_values[0].percent, 25.0);
    assert_eq!(doc.calc_values[0].px, 8.0);
}

#[test]
fn text_indent_does_not_shift_floated_or_out_of_flow_inline_blocks() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let absolute_parent = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;text-indent:20px"),
    );
    let absolute_child = doc.append_element(
        Some(absolute_parent),
        "span",
        Style::default(),
        Some("display:inline-block;width:10px;height:10px;position:absolute"),
    );
    let float_parent = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;text-indent:20px"),
    );
    let float_child = doc.append_element(
        Some(float_parent),
        "span",
        Style::default(),
        Some("display:inline-block;width:10px;height:10px;float:left"),
    );
    doc.mark_in_document_flags();

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    assert_eq!(
        cascade.computed[absolute_child].position,
        PositionValue::Absolute
    );
    assert_eq!(cascade.computed[float_child].float, FloatValue::Left);

    doc.nodes[absolute_parent].unrounded_layout.size.width = 100.0;
    doc.nodes[absolute_child].unrounded_layout.location.x = 3.0;
    doc.nodes[float_parent].unrounded_layout.size.width = 100.0;
    doc.nodes[float_child].unrounded_layout.location.x = 7.0;
    realign_single_empty_inline_block_indent(&mut doc, &cascade);

    assert_eq!(doc.nodes[absolute_child].unrounded_layout.location.x, 3.0);
    assert_eq!(doc.nodes[float_child].unrounded_layout.location.x, 7.0);
}

#[test]
fn text_indent_skips_child_cleared_from_document_flags() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let parent = doc.append_element(
        Some(0),
        "div",
        Style::default(),
        Some("display:block;text-indent:20px"),
    );
    let child = doc.append_element(
        Some(parent),
        "span",
        Style::default(),
        Some("display:inline-block;width:10px;height:10px"),
    );
    doc.mark_in_document_flags();
    assert!(doc.nodes[parent].is_in_document());
    assert!(doc.nodes[child].is_in_document());

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    assert_eq!(cascade.computed[parent].display, DisplayValue::Block);
    assert_eq!(cascade.computed[child].display, DisplayValue::InlineBlock);
    doc.nodes[child].set_in_document(false);
    assert!(!doc.nodes[child].is_in_document());

    doc.nodes[parent].unrounded_layout.size.width = 100.0;
    doc.nodes[child].unrounded_layout.location.x = 4.0;
    realign_single_empty_inline_block_indent(&mut doc, &cascade);

    assert_eq!(doc.nodes[child].unrounded_layout.location.x, 4.0);
}

#[test]
fn text_indent_skips_negative_content_width() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let parent = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;text-indent:20px"),
    );
    let child = doc.append_element(
        Some(parent),
        "span",
        Style::default(),
        Some("display:inline-block;width:10px;height:10px"),
    );
    doc.mark_in_document_flags();

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    doc.nodes[parent].unrounded_layout.size.width = -10.0;
    doc.nodes[child].unrounded_layout.location.x = 4.0;
    assert!(doc.nodes[parent].unrounded_layout.content_box_width() < 0.0);
    realign_single_empty_inline_block_indent(&mut doc, &cascade);

    assert_eq!(doc.nodes[child].unrounded_layout.location.x, 4.0);
}

#[test]
fn establish_minimal_line_boxes_br_full_basis_resolves_against_narrow_container_width() {
    // `<br>`'s flex_basis:100% (`establish_minimal_line_boxes`'s doc)
    // must resolve against the *qualifying container's own* used main
    // size, not its containing block's — otherwise a narrower
    // container (explicit `width`, rather than filling its parent)
    // would give <br> a too-wide box. Pins that by giving the
    // container an explicit width much narrower than the page and
    // checking <br>'s own box stays within it.

    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(
        Some(body),
        "p",
        Style::default(),
        Some("display: block; width: 100px"),
    );
    let a = doc.append_text(p, "AAAA");
    let br = doc.append_element(Some(p), "br", Style::default(), None::<&str>);
    let c = doc.append_text(p, "CCCC");

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4).expect("layout Ok");

    let p_loc = doc.nodes[p].unrounded_layout;
    let br_loc = doc.nodes[br].unrounded_layout;
    let a_loc = doc.nodes[a].unrounded_layout;
    let c_loc = doc.nodes[c].unrounded_layout;

    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (p_loc.size.width - 100.0).abs() < 1e-3,
        "fixture precondition: explicit width:100px must actually take \
             effect, got p.w={}",
        p_loc.size.width
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        br_loc.size.width <= p_loc.size.width + 1e-3,
        "<br>'s flex_basis:100% must resolve against the qualifying \
             container's own (narrow) used width, not some wider \
             containing block — got br.w={} p.w={}",
        br_loc.size.width,
        p_loc.size.width
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        c_loc.location.y >= a_loc.location.y + a_loc.size.height - 1e-3,
        "the forced break must still occur on a narrow container, got \
             a.y={} a.h={} c.y={}",
        a_loc.location.y,
        a_loc.size.height,
        c_loc.location.y
    );
}

// ── site 6: sanitize_font_weight ─────────────
//
// Unit tests analogous to sanitize_finite / sanitize_taffy. They verify
// that non-finite font_weight values (possible because all `ComputedValues`
// fields are `pub` and the f32 promotion removed type-level exclusion)
// become finite and stay in `[1,1000]` immediately before
// `parley::FontWeight::new`.

#[test]
fn sanitize_font_weight_maps_nan_to_normal_fallback() {
    // `f32::clamp` returns NaN for NaN, so without this branch it passes
    // through. Use `FALLBACK_FONT_WEIGHT` instead of `0.0` (400.0, the
    // computed value of `normal` under CSS Fonts 4 §2.2,
    // <https://www.w3.org/TR/css-fonts-4/#valdef-font-weight-normal>).
    // See `sanitize_font_weight` for why this differs from the fallback
    // used at length-related `sanitize_finite` sites.
    let mut diag = Vec::new();
    assert_eq!(
        sanitize_font_weight(f32::NAN, &mut diag),
        FALLBACK_FONT_WEIGHT
    );
    assert_eq!(diag.len(), 1, "clamp が発火したので 1 event 積まれること");
    match diag[0] {
        LayoutWarn::NonFiniteClamped { site, raw, clamped } => {
            assert_eq!(site, "font-weight");
            assert!(raw.is_nan());
            assert_eq!(clamped, FALLBACK_FONT_WEIGHT);
        }
        other => panic!("unexpected LayoutWarn variant: {other:?}"),
    }
}

#[test]
fn sanitize_font_weight_clamps_infinities_to_bounds() {
    let mut diag = Vec::new();
    assert_eq!(
        sanitize_font_weight(f32::INFINITY, &mut diag),
        MAX_FONT_WEIGHT
    );
    assert_eq!(
        sanitize_font_weight(f32::NEG_INFINITY, &mut diag),
        MIN_FONT_WEIGHT
    );
    assert_eq!(diag.len(), 2, "+Inf / -Inf とも clamp が発火する");
}

#[test]
fn sanitize_font_weight_clamps_out_of_range_finite_values() {
    // Also clamp finite values outside the range; this is not just
    // finiteness checking (the font-weight counterpart of
    // `sanitize_taffy_clamps_out_of_range_finite_values`).
    let mut diag = Vec::new();
    assert_eq!(sanitize_font_weight(1e30, &mut diag), MAX_FONT_WEIGHT);
    assert_eq!(sanitize_font_weight(-1e30, &mut diag), MIN_FONT_WEIGHT);
    // `0.0` is valid for length sinks but below font-weight's valid
    // `[1, 1000]` range, so clamp it to MIN_FONT_WEIGHT.
    assert_eq!(sanitize_font_weight(0.0, &mut diag), MIN_FONT_WEIGHT);
    assert_eq!(diag.len(), 3);
}

#[test]
fn sanitize_font_weight_passes_through_in_range_values() {
    // Let ordinary values, including fractional weights, pass through
    // bit-identically.
    let mut diag = Vec::new();
    for v in [
        MIN_FONT_WEIGHT,
        1.0,
        100.0,
        349.5,
        400.0,
        700.0,
        MAX_FONT_WEIGHT,
    ] {
        assert_eq!(
            sanitize_font_weight(v, &mut diag),
            v,
            "in-range value must pass through: {v}"
        );
    }
    assert!(
        diag.is_empty(),
        "in-range value must not push a LayoutWarn: {diag:?}"
    );
}

#[test]
fn font_style_to_parley_maps_normal_and_italic() {
    assert_eq!(
        font_style_to_parley(StyleFontStyle::Normal),
        FontStyle::Normal
    );
    assert_eq!(
        font_style_to_parley(StyleFontStyle::Italic),
        FontStyle::Italic
    );
}

#[test]
fn font_style_to_parley_maps_oblique_not_normal() {
    // Regression: `Oblique` previously fell through the wildcard arm
    // and was silently converted to `FontStyle::Normal`, discarding a
    // real cascade winner (e.g. a child overriding an inherited
    // `font-style: italic` with `font-style: oblique` would render
    // upright instead of slanted).
    assert_eq!(
        font_style_to_parley(StyleFontStyle::Oblique),
        FontStyle::Oblique(None)
    );
}

#[test]
fn family_candidate_has_ch_glyphs_checks_space_and_zero_glyph_coverage() {
    // CSS Values 4 §6.1.1 `ch`: the first-available face must supply a
    // usable space glyph *and* U+0030, whose advance becomes the `ch`
    // metric (see the function's own doc comment).
    let tmp = embedded_ic_font_dir();
    let mut fonts = crate::fonts::build_wpt_font_ctx(tmp.path()).expect("register WPT fonts");

    // Ahem maps every ASCII printable codepoint it supports (including
    // space and '0') to its square glyph, so both required glyphs are
    // present and the query stops at the first candidate.
    assert!(family_candidate_has_ch_glyphs(
        &mut fonts,
        "Ahem",
        400.0,
        StyleFontStyle::Normal,
    ));

    // CanvasTest-nospace.ttf deliberately omits a usable space glyph
    // (see the sibling `generic_family_has_ch_glyphs` coverage above),
    // so the query callback keeps returning `QueryStatus::Continue` and
    // the loop exhausts without ever finding a usable face.
    assert!(!family_candidate_has_ch_glyphs(
        &mut fonts,
        "CanvasTestNoSpace",
        400.0,
        StyleFontStyle::Normal,
    ));

    // An unregistered family name matches no face at all: the query
    // callback never runs and `has_ch_glyphs` keeps its initial `false`.
    assert!(!family_candidate_has_ch_glyphs(
        &mut fonts,
        "NotARegisteredFamily",
        400.0,
        StyleFontStyle::Normal,
    ));
}

#[test]
fn text_autospace_boxes_insert_expected_boundary_advances() {
    use raikiri_style::property::{TextAutospace, TextAutospaceMode};

    let boxes = text_autospace_boxes("国A国1A", TextAutospace::Normal, "", 40.0);
    assert_eq!(
        boxes
            .iter()
            .map(|inline_box| inline_box.index)
            .collect::<Vec<_>>(),
        vec![3, 4, 7]
    );
    assert!(boxes.iter().all(|inline_box| {
        inline_box.kind == InlineBoxKind::InFlow
            && inline_box.width == 5.0
            && inline_box.height == 0.0
    }));

    // The caller supplies the measured `ic` advance; the placement helper
    // must preserve its one-eighth width rather than recomputing from a
    // font-size approximation.
    let measured_ic = 37.25;
    let measured_gap = measured_ic * 0.125;
    let measured =
        text_autospace_boxes_with_width("国A", TextAutospace::Normal, "", measured_gap, None, None);
    assert_eq!(measured.len(), 1);
    assert_eq!(measured[0].width, measured_gap);

    let zh_punctuation = text_autospace_boxes("国!国", TextAutospace::Normal, "zh", 40.0);
    assert_eq!(
        zh_punctuation
            .iter()
            .map(|inline_box| inline_box.index)
            .collect::<Vec<_>>(),
        vec![3, 4]
    );
    assert!(text_autospace_boxes("国!国", TextAutospace::Normal, "en", 40.0).is_empty());

    let custom = text_autospace_boxes(
        "国A",
        TextAutospace::Custom {
            ideograph_alpha: true,
            ideograph_numeric: false,
            punctuation: false,
            mode: TextAutospaceMode::None,
        },
        "en",
        40.0,
    );
    assert_eq!(
        custom
            .iter()
            .map(|inline_box| inline_box.index)
            .collect::<Vec<_>>(),
        vec![3]
    );

    let cross_before =
        text_autospace_boxes_with_edges("A", TextAutospace::Normal, "", 40.0, Some('国'), None);
    assert_eq!(cross_before[0].index, 0);
    let cross_after =
        text_autospace_boxes_with_edges("国", TextAutospace::Normal, "", 40.0, None, Some('A'));
    assert_eq!(cross_after[0].index, 3);
}

#[test]
fn nbsp_glue_keeps_image_group_together_on_overflow() {
    // Overflow regression for NBSP glue in the replaced-child realign pass.
    // U+00A0 forbids breaks before and after, so an image plus NBSP plus
    // image run must wrap atomically. A filler image leaves 70 units used
    // on a 100 unit line, then a 90 unit glued run must move as one unit
    // to the next line instead of stranding its first image on line one.
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let block = doc.append_element(Some(body), "div", Style::default(), Some("display: block"));
    let filler = doc.append_element(
        Some(block),
        "img",
        Style::default(),
        Some("display: inline; width: 30px; height: 20px"),
    );
    let first = doc.append_element(
        Some(block),
        "img",
        Style::default(),
        Some("display: inline; width: 40px; height: 20px"),
    );
    let nbsp = doc.append_text(block, "\u{00A0}");
    let second = doc.append_element(
        Some(block),
        "img",
        Style::default(),
        Some("display: inline; width: 40px; height: 20px"),
    );
    doc.mark_in_document_flags();

    let rules = build_rule_tree(&doc);
    let cascade_result = cascade(&doc, &rules).expect("cascade Ok");
    apply_computed_to_style(&mut doc, &cascade_result).expect("styles");
    // cov:ignore: panic-message literal only executed on assertion failure.
    assert!(
        doc.nodes[block]
            .flags
            .contains(crate::node::NodeFlags::IS_INLINE_ROOT),
        "fixture must qualify as an inline root so the realign pass runs"
    );

    doc.nodes[block].unrounded_layout.size.width = 100.0;
    doc.nodes[filler].unrounded_layout.size.width = 30.0;
    doc.nodes[filler].unrounded_layout.size.height = 20.0;
    doc.nodes[first].unrounded_layout.size.width = 40.0;
    doc.nodes[first].unrounded_layout.size.height = 20.0;
    doc.nodes[nbsp].unrounded_layout.size.width = 10.0;
    doc.nodes[second].unrounded_layout.size.width = 40.0;
    doc.nodes[second].unrounded_layout.size.height = 20.0;

    realign_inline_replaced_children(&mut doc, &cascade_result);

    let filler_loc = doc.nodes[filler].unrounded_layout;
    let first_loc = doc.nodes[first].unrounded_layout;
    let nbsp_loc = doc.nodes[nbsp].unrounded_layout;
    let second_loc = doc.nodes[second].unrounded_layout;
    // cov:ignore: panic-message literal only executed on assertion failure.
    assert!(
        (filler_loc.location.x - 0.0).abs() < 1e-3,
        "filler stays at line start, got x={}",
        filler_loc.location.x
    );
    // cov:ignore: panic-message literal only executed on assertion failure.
    assert!(
        (first_loc.location.x - 0.0).abs() < 1e-3,
        "glued run moves as one unit, so first image wraps to x=0, got x={}",
        first_loc.location.x
    );
    // cov:ignore: panic-message literal only executed on assertion failure.
    assert!(
        (nbsp_loc.location.x - 40.0).abs() < 1e-3,
        "NBSP stays glued after first image, got x={}",
        nbsp_loc.location.x
    );
    // cov:ignore: panic-message literal only executed on assertion failure.
    assert!(
        (second_loc.location.x - 50.0).abs() < 1e-3,
        "second image stays glued after NBSP, got x={}",
        second_loc.location.x
    );
    // cov:ignore: panic-message literal only executed on assertion failure.
    assert!(
        (first_loc.location.y - second_loc.location.y).abs() < 1e-3,
        "glued images share a line, got first y={} second y={}",
        first_loc.location.y,
        second_loc.location.y
    );
    // cov:ignore: panic-message literal only executed on assertion failure.
    assert!(
        first_loc.location.y > filler_loc.location.y + 1e-3,
        "glued run wrapped past filler line, got first y={} filler y={}",
        first_loc.location.y,
        filler_loc.location.y
    );
}
