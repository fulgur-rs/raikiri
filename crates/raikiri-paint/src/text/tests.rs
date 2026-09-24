use super::{
    AUTOSPACE_INLINE_BOX_ID_MIN, DecorationContext, DecorationSpec, MAX_DECORATION_SEGMENTS,
    dashed_lengths, decoration_line_width, decoration_span, decoration_spans,
    decorations_for_element, is_autospace_inline_box, measure_margin_text_advance,
    measure_margin_text_height, paint_decoration_style, run_baseline_delta,
    standalone_run_baseline, synthetic_embolden, text_align_last_delta,
};
use anyrender::{Scene, recording::RenderCommand};
use kurbo::Vec2;
use parley::{InlineBoxKind, LineMetrics, PositionedInlineBox, PositionedLayoutItem, RunMetrics};
use raikiri_style::ComputedValues;
use raikiri_style::property::{
    CssColor, Direction, DisplayValue, FloatValue, PositionValue, TextAlign, TextAlignLast,
    TextDecorationLine, TextDecorationStyle,
};

fn metrics(offset: f32) -> LineMetrics {
    LineMetrics {
        offset,
        advance: 40.0,
        inline_max_coord: 100.0,
        ..LineMetrics::default()
    }
}

#[test]
fn baseline_helpers_cover_inline_box_detection_and_baselines() {
    let high_id = PositionedLayoutItem::InlineBox(PositionedInlineBox {
        x: 0.0,
        y: 0.0,
        width: 5.0,
        height: 0.0,
        id: AUTOSPACE_INLINE_BOX_ID_MIN,
        kind: InlineBoxKind::InFlow,
    });
    let ordinary_id = PositionedLayoutItem::InlineBox(PositionedInlineBox {
        x: 0.0,
        y: 0.0,
        width: 5.0,
        height: 0.0,
        id: AUTOSPACE_INLINE_BOX_ID_MIN - 1,
        kind: InlineBoxKind::InFlow,
    });
    assert!(is_autospace_inline_box(&high_id));
    assert!(!is_autospace_inline_box(&ordinary_id));

    assert_eq!(standalone_run_baseline(10.4, 2.6, 16.0), 11.0);
    let line = LineMetrics {
        ascent: 10.4,
        descent: 2.6,
        line_height: 16.0,
        ..LineMetrics::default()
    };
    let run = RunMetrics {
        ascent: 8.4,
        descent: 1.6,
        line_height: 14.0,
        ..RunMetrics::default()
    };
    assert_eq!(run_baseline_delta(&line, &run), -1.0);
}

#[test]
fn decoration_span_applies_logical_insets_from_origin_direction() {
    let mut spec = DecorationSpec {
        line: TextDecorationLine::UNDERLINE,
        style: TextDecorationStyle::Solid,
        color: CssColor::BLACK,
        origin_thickness: 1.0,
        origin_ascent: 8.0,
        origin_descent: 2.0,
        origin_shift_y: 0.0,
        inset_start: 10.0,
        inset_end: -10.0,
        underline_offset: 0.0,
        origin_rtl: false,
    };
    assert_eq!(decoration_span(100.0, 200.0, &spec), Some((110.0, 210.0)));

    spec.origin_rtl = true;
    assert_eq!(decoration_span(100.0, 200.0, &spec), Some((90.0, 190.0)));
}

#[test]
fn decoration_span_rejects_empty_or_nonfinite_ranges() {
    let spec = DecorationSpec {
        line: TextDecorationLine::UNDERLINE,
        style: TextDecorationStyle::Solid,
        color: CssColor::BLACK,
        origin_thickness: 1.0,
        origin_ascent: 8.0,
        origin_descent: 2.0,
        origin_shift_y: 0.0,
        inset_start: 60.0,
        inset_end: 50.0,
        underline_offset: 0.0,
        origin_rtl: false,
    };
    assert_eq!(decoration_span(100.0, 200.0, &spec), None);
    let mut nonfinite = spec;
    nonfinite.inset_start = f64::NAN;
    assert_eq!(decoration_span(100.0, 200.0, &nonfinite), None);
}

#[test]
fn decoration_spans_preserve_asymmetric_mixed_sign_endpoint_overlap() {
    let spec = DecorationSpec {
        line: TextDecorationLine::UNDERLINE,
        style: TextDecorationStyle::Solid,
        color: CssColor::BLACK,
        origin_thickness: 1.0,
        origin_ascent: 8.0,
        origin_descent: 2.0,
        origin_shift_y: 0.0,
        inset_start: 10.0,
        inset_end: -12.0,
        underline_offset: 0.0,
        origin_rtl: false,
    };
    assert_eq!(
        decoration_spans(100.0, 140.0, &spec),
        Some(([(110.0, 150.0), (112.0, 152.0)], 2))
    );
}

#[test]
fn decoration_spans_collapse_equal_endpoint_translation() {
    let spec = DecorationSpec {
        line: TextDecorationLine::UNDERLINE,
        style: TextDecorationStyle::Solid,
        color: CssColor::BLACK,
        origin_thickness: 1.0,
        origin_ascent: 8.0,
        origin_descent: 2.0,
        origin_shift_y: 0.0,
        inset_start: 10.0,
        inset_end: -10.0,
        underline_offset: 0.0,
        origin_rtl: false,
    };
    assert_eq!(
        decoration_spans(100.0, 140.0, &spec),
        Some(([(110.0, 150.0), (0.0, 0.0)], 1))
    );
}

#[test]
fn generated_text_measurements_handle_empty_and_nonfinite_inputs() {
    assert_eq!(measure_margin_text_advance("", f32::NAN, "serif"), 0.0);
    assert!(measure_margin_text_advance("A", f32::NAN, "serif") > 0.0);
    assert_eq!(measure_margin_text_height("", f32::NAN, "serif"), 0.0);
    assert!(measure_margin_text_height("A", f32::NAN, "serif") > 0.0);
}

#[test]
fn final_line_start_reverses_center_alignment() {
    let delta = text_align_last_delta(
        &metrics(30.0),
        TextAlign::Center,
        TextAlignLast::Start,
        Direction::Ltr,
    );
    assert_eq!(delta, -30.0);
}

#[test]
fn final_line_end_and_center_use_remaining_space() {
    assert_eq!(
        text_align_last_delta(
            &metrics(0.0),
            TextAlign::Start,
            TextAlignLast::End,
            Direction::Ltr,
        ),
        60.0
    );
    assert_eq!(
        text_align_last_delta(
            &metrics(60.0),
            TextAlign::End,
            TextAlignLast::Center,
            Direction::Ltr,
        ),
        -30.0
    );
}

#[test]
fn final_line_logical_edges_flip_in_rtl() {
    assert_eq!(
        text_align_last_delta(
            &metrics(60.0),
            TextAlign::Start,
            TextAlignLast::End,
            Direction::Rtl,
        ),
        -60.0
    );
    assert_eq!(
        text_align_last_delta(
            &metrics(0.0),
            TextAlign::End,
            TextAlignLast::Start,
            Direction::Rtl,
        ),
        60.0
    );
}

#[test]
fn physical_edges_and_match_parent_are_supported() {
    assert_eq!(
        text_align_last_delta(
            &metrics(60.0),
            TextAlign::Start,
            TextAlignLast::Left,
            Direction::Ltr,
        ),
        -60.0
    );
    assert_eq!(
        text_align_last_delta(
            &metrics(0.0),
            TextAlign::Start,
            TextAlignLast::Right,
            Direction::Ltr,
        ),
        60.0
    );
    assert_eq!(
        text_align_last_delta(
            &metrics(60.0),
            TextAlign::Start,
            TextAlignLast::MatchParent,
            Direction::Ltr,
        ),
        -60.0
    );
}

#[test]
fn auto_justify_and_explicit_justify_keep_parley_last_line() {
    assert_eq!(
        text_align_last_delta(
            &metrics(0.0),
            TextAlign::Justify,
            TextAlignLast::Auto,
            Direction::Ltr,
        ),
        0.0
    );
    assert_eq!(
        text_align_last_delta(
            &metrics(0.0),
            TextAlign::Start,
            TextAlignLast::Justify,
            Direction::Ltr,
        ),
        0.0
    );
}

fn ancestor_context() -> DecorationContext {
    let mut cv = ComputedValues::initial();
    cv.text_decoration_line = TextDecorationLine::UNDERLINE;
    decorations_for_element(&DecorationContext::default(), &cv, 0.0)
}

#[test]
fn decoration_width_skips_line_edge_whitespace_at_every_white_space_mode() {
    let metrics = LineMetrics {
        advance: 40.0,
        trailing_whitespace: 10.0,
        ..LineMetrics::default()
    };
    // Level 3 skips spacing at both line edges. The white-space property
    // controls shaping/collapsing, not this decoration edge rule.
    assert_eq!(decoration_line_width(&metrics, 0.0), 30.0);
    assert_eq!(decoration_line_width(&metrics, 5.0), 25.0);
}

#[test]
fn dotted_decoration_caps_scene_commands_for_huge_spans() {
    let mut scene = Scene::new();
    paint_decoration_style(
        &mut scene,
        TextDecorationStyle::Dotted,
        peniko::Color::BLACK,
        0.0,
        1_000_000_000_000.0,
        0.0,
        1.0,
    );
    let fills = scene
        .commands
        .iter()
        .filter(|command| matches!(command, RenderCommand::Fill(_)))
        .count();
    assert!(fills > 0);
    assert!(fills <= MAX_DECORATION_SEGMENTS);
}

#[test]
fn dashed_decoration_scales_period_for_huge_spans() {
    let (dash, gap) = dashed_lengths(1_000_000_000_000.0, 1.0);
    assert!(dash > 3.0);
    assert!(gap > 2.0);
    assert!((dash + gap) * MAX_DECORATION_SEGMENTS as f64 >= 1_000_000_000_000.0);
}

#[test]
fn display_contents_does_not_originate_or_block_decoration() {
    let mut ancestor = ComputedValues::initial();
    ancestor.text_decoration_line = TextDecorationLine::UNDERLINE;
    let context = decorations_for_element(&DecorationContext::default(), &ancestor, 0.0);

    let mut contents = ComputedValues::initial();
    contents.display = DisplayValue::Contents;
    contents.text_decoration_line = TextDecorationLine::OVERLINE;
    let propagated = decorations_for_element(&context, &contents, 0.0);
    let specs: Vec<_> = propagated.iter().collect();
    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].line, TextDecorationLine::UNDERLINE);
}

#[test]
fn decoration_retains_origin_vertical_align_shift() {
    let mut cv = ComputedValues::initial();
    cv.text_decoration_line = TextDecorationLine::UNDERLINE;
    let decorations = decorations_for_element(&DecorationContext::default(), &cv, 7.5);
    assert_eq!(
        decorations
            .iter()
            .next()
            .expect("origin decoration")
            .origin_shift_y,
        7.5
    );
}

#[test]
fn decoration_metrics_are_taken_from_the_originating_element() {
    let mut cv = ComputedValues::initial();
    cv.font_size = raikiri_style::resolve::ComputedLength(32.0);
    cv.text_decoration_line = TextDecorationLine::UNDERLINE;
    let decorations = decorations_for_element(&DecorationContext::default(), &cv, 0.0);
    let decoration = decorations.iter().next().expect("origin decoration");
    assert_eq!(decoration.origin_thickness, 2.0);
    assert_eq!(decoration.origin_ascent, 25.6);
    assert_eq!(decoration.origin_descent, 6.4);
}

#[test]
fn underline_offset_percentage_resolves_against_decorating_font_size() {
    let mut cv = ComputedValues::initial();
    cv.font_size = raikiri_style::resolve::ComputedLength(40.0);
    cv.text_decoration_line = TextDecorationLine::UNDERLINE;
    cv.text_underline_offset = raikiri_style::ComputedTextUnderlineOffset::Percent(50.0);
    let decorations = decorations_for_element(&DecorationContext::default(), &cv, 0.0);
    let decoration = decorations.iter().next().expect("origin decoration");
    assert_eq!(decoration.underline_offset, 20.0);
}

#[test]
fn ancestor_decoration_stops_at_out_of_flow_float_and_atomic_boundaries() {
    let ancestor = ancestor_context();
    let cases = [
        (
            "absolute",
            DisplayValue::Block,
            PositionValue::Absolute,
            FloatValue::None,
        ),
        (
            "fixed",
            DisplayValue::Block,
            PositionValue::Fixed,
            FloatValue::None,
        ),
        (
            "float",
            DisplayValue::Block,
            PositionValue::Static,
            FloatValue::Left,
        ),
        (
            "inline-block",
            DisplayValue::InlineBlock,
            PositionValue::Static,
            FloatValue::None,
        ),
        (
            "inline-flex",
            DisplayValue::InlineFlex,
            PositionValue::Static,
            FloatValue::None,
        ),
        (
            "inline-grid",
            DisplayValue::InlineGrid,
            PositionValue::Static,
            FloatValue::None,
        ),
        (
            "inline-table",
            DisplayValue::InlineTable,
            PositionValue::Static,
            FloatValue::None,
        ),
    ];
    for (name, display, position, float) in cases {
        let mut cv = ComputedValues::initial();
        cv.display = display;
        cv.position = position;
        cv.float = float;
        // cov:ignore: the assertion message is evaluated only when this
        // boundary regression assertion fails.
        assert!(
            decorations_for_element(&ancestor, &cv, 0.0)
                .iter()
                .next()
                .is_none(),
            "ancestor decoration crossed {name} boundary"
        );
    }

    let mut in_flow = ComputedValues::initial();
    in_flow.display = DisplayValue::Block;
    assert_eq!(
        decorations_for_element(&ancestor, &in_flow, 0.0)
            .iter()
            .count(),
        1
    );

    let mut boundary_origin = ComputedValues::initial();
    boundary_origin.display = DisplayValue::InlineBlock;
    boundary_origin.text_decoration_line = TextDecorationLine::OVERLINE;
    let boundary_decorations = decorations_for_element(&ancestor, &boundary_origin, 0.0);
    assert_eq!(boundary_decorations.iter().count(), 1);
    assert_eq!(
        boundary_decorations
            .iter()
            .next()
            .expect("own decoration")
            .line,
        TextDecorationLine::OVERLINE
    );
}
#[test]
fn synthetic_embolden_uses_zero_without_synthesis() {
    assert_eq!(synthetic_embolden(false, 32.0), Vec2::ZERO);
}

#[test]
fn synthetic_embolden_scales_and_caps_stroke_offset() {
    assert_eq!(synthetic_embolden(true, 10.0), Vec2::new(0.15125, 0.121));
    assert_eq!(synthetic_embolden(true, 100.0), Vec2::new(0.3, 0.3));
}
