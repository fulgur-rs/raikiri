use super::{
    DecorationContext, DecorationGeometry, DecorationPhase, DecorationSpec,
    MAX_DECORATION_SEGMENTS, dashed_lengths, decoration_span, decoration_spans,
    decorations_for_element, draw_decoration_phase, measure_margin_text_advance,
    measure_margin_text_height, paint_decoration_style, synthetic_embolden,
};
use anyrender::{Scene, recording::RenderCommand};
use kurbo::Vec2;
use raikiri_style::ComputedValues;
use raikiri_style::property::{
    CssColor, DisplayValue, FloatValue, PositionValue, TextDecorationLine, TextDecorationStyle,
};

#[test]
fn dashed_segment_keeps_the_unsplit_pattern_phase() {
    let mut cv = ComputedValues::initial();
    cv.text_decoration_line = TextDecorationLine::UNDERLINE;
    cv.text_decoration_style = TextDecorationStyle::Dashed;
    let context = decorations_for_element(
        &DecorationContext::default(),
        raikiri_traits::NodeId::new(1),
        &cv,
        0.0,
    );
    let mut scene = Scene::new();
    draw_decoration_phase(
        &mut scene,
        &context.specs(),
        DecorationGeometry {
            x0: 12.0,
            x1: 20.0,
            abs_y: 0.0,
            line_top: 0.0,
            baseline: Some(0.0),
            pattern_origin_x: Some(0.0),
            pattern_end_x: None,
        },
        DecorationPhase::BeforeGlyphs,
    );
    let RenderCommand::Stroke(stroke) = &scene.commands[0] else {
        panic!("expected dashed stroke")
    };
    assert_eq!(stroke.style.dash_offset, 2.0);
}

#[test]
fn decoration_span_applies_logical_insets_from_origin_direction() {
    let mut spec = DecorationSpec {
        origin: raikiri_traits::NodeId::new(0),
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
fn decorations_are_unchanged_by_the_geometry_field() {
    // A geometry without an explicit baseline derives it from the
    // decorating element, exactly as before the field existed. An
    // explicit baseline is the line's; the decorating element's shift is
    // added to it, so the line's baseline without the shift draws the
    // same line.
    let spec = DecorationSpec {
        origin: raikiri_traits::NodeId::new(0),
        line: TextDecorationLine::UNDERLINE,
        style: TextDecorationStyle::Solid,
        color: CssColor::BLACK,
        origin_thickness: 1.0,
        origin_ascent: 8.0,
        origin_descent: 2.0,
        origin_shift_y: 1.5,
        inset_start: 0.0,
        inset_end: 0.0,
        underline_offset: 0.0,
        origin_rtl: false,
    };
    let (abs_y, line_top) = (100.0_f64, 2.0_f32);
    let derived = abs_y + f64::from(line_top) + spec.origin_ascent;
    let draw = |baseline: Option<f64>| {
        let mut scene = anyrender::recording::Scene::new();
        let geometry = DecorationGeometry {
            pattern_origin_x: None,
            pattern_end_x: None,
            x0: 0.0,
            x1: 10.0,
            abs_y,
            line_top,
            baseline,
        };
        draw_decoration_phase(
            &mut scene,
            &[&spec],
            geometry,
            DecorationPhase::BeforeGlyphs,
        );
        scene
    };
    let (implicit, explicit) = (draw(None), draw(Some(derived)));
    assert!(!implicit.commands.is_empty());
    assert_eq!(
        format!("{:?}", implicit.commands),
        format!("{:?}", explicit.commands)
    );
    // A different explicit baseline moves the line.
    let moved = draw(Some(derived + 3.0));
    assert_ne!(
        format!("{:?}", implicit.commands),
        format!("{:?}", moved.commands)
    );
}

#[test]
fn decoration_span_rejects_empty_or_nonfinite_ranges() {
    let spec = DecorationSpec {
        origin: raikiri_traits::NodeId::new(0),
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
        origin: raikiri_traits::NodeId::new(0),
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
        origin: raikiri_traits::NodeId::new(0),
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
    let doc = raikiri_dom::Document::new();
    assert_eq!(
        measure_margin_text_advance(&doc, "", f32::NAN, "serif"),
        0.0
    );
    assert!(measure_margin_text_advance(&doc, "A", f32::NAN, "serif") > 0.0);
    assert_eq!(measure_margin_text_height(&doc, "", f32::NAN, "serif"), 0.0);
    assert!(measure_margin_text_height(&doc, "A", f32::NAN, "serif") > 0.0);
}

fn ancestor_context() -> DecorationContext {
    let mut cv = ComputedValues::initial();
    cv.text_decoration_line = TextDecorationLine::UNDERLINE;
    decorations_for_element(
        &DecorationContext::default(),
        raikiri_traits::NodeId::new(0),
        &cv,
        0.0,
    )
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
    let context = decorations_for_element(
        &DecorationContext::default(),
        raikiri_traits::NodeId::new(0),
        &ancestor,
        0.0,
    );

    let mut contents = ComputedValues::initial();
    contents.display = DisplayValue::Contents;
    contents.text_decoration_line = TextDecorationLine::OVERLINE;
    let propagated =
        decorations_for_element(&context, raikiri_traits::NodeId::new(0), &contents, 0.0);
    let specs: Vec<_> = propagated.iter().collect();
    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].line, TextDecorationLine::UNDERLINE);
}

#[test]
fn decoration_retains_origin_vertical_align_shift() {
    let mut cv = ComputedValues::initial();
    cv.text_decoration_line = TextDecorationLine::UNDERLINE;
    let decorations = decorations_for_element(
        &DecorationContext::default(),
        raikiri_traits::NodeId::new(0),
        &cv,
        7.5,
    );
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
    let decorations = decorations_for_element(
        &DecorationContext::default(),
        raikiri_traits::NodeId::new(0),
        &cv,
        0.0,
    );
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
    let decorations = decorations_for_element(
        &DecorationContext::default(),
        raikiri_traits::NodeId::new(0),
        &cv,
        0.0,
    );
    let decoration = decorations.iter().next().expect("origin decoration");
    assert_eq!(decoration.underline_offset, 20.0);
}

#[test]
fn underline_offset_calc_resolves_percentage_against_decorating_font_size() {
    let mut cv = ComputedValues::initial();
    cv.font_size = raikiri_style::resolve::ComputedLength(40.0);
    cv.text_decoration_line = TextDecorationLine::UNDERLINE;
    cv.text_underline_offset = raikiri_style::ComputedTextUnderlineOffset::Calc(
        raikiri_style::property::CalcLengthPercentage {
            percent: 50.0,
            px: 8.0,
        },
    );
    let decorations = decorations_for_element(
        &DecorationContext::default(),
        raikiri_traits::NodeId::new(0),
        &cv,
        0.0,
    );
    let decoration = decorations.iter().next().expect("origin decoration");
    assert_eq!(decoration.underline_offset, 28.0);
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
            decorations_for_element(&ancestor, raikiri_traits::NodeId::new(0), &cv, 0.0)
                .iter()
                .next()
                .is_none(),
            "ancestor decoration crossed {name} boundary"
        );
    }

    let mut in_flow = ComputedValues::initial();
    in_flow.display = DisplayValue::Block;
    assert_eq!(
        decorations_for_element(&ancestor, raikiri_traits::NodeId::new(0), &in_flow, 0.0)
            .iter()
            .count(),
        1
    );

    let mut boundary_origin = ComputedValues::initial();
    boundary_origin.display = DisplayValue::InlineBlock;
    boundary_origin.text_decoration_line = TextDecorationLine::OVERLINE;
    let boundary_decorations = decorations_for_element(
        &ancestor,
        raikiri_traits::NodeId::new(0),
        &boundary_origin,
        0.0,
    );
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

#[test]
fn dotted_segment_includes_dot_center_just_after_the_endpoint() {
    use kurbo::Shape;
    let mut scene = Scene::new();
    paint_decoration_style(
        &mut scene,
        TextDecorationStyle::Dotted,
        peniko::Color::BLACK,
        0.0,
        26.0,
        0.0,
        6.0,
    );
    assert!(
        scene.commands.iter().any(|command| {
            let RenderCommand::Fill(fill) = command else {
                return false;
            };
            let bounds = fill.shape.bounding_box();
            ((bounds.x0 + bounds.x1) * 0.5 - 27.0).abs() < 0.001
        }),
        "the dot centered at27 overlaps the segment24..26"
    );
}

#[test]
fn dashed_segments_share_the_full_line_complexity_budget() {
    let mut cv = ComputedValues::initial();
    cv.text_decoration_line = TextDecorationLine::UNDERLINE;
    cv.text_decoration_style = TextDecorationStyle::Dashed;
    let context = decorations_for_element(
        &DecorationContext::default(),
        raikiri_traits::NodeId::new(1),
        &cv,
        0.0,
    );
    let mut patterns = Vec::new();
    for (x0, x1) in [(0.0, 1000.0), (1000.0, 1_000_000.0)] {
        let mut scene = Scene::new();
        draw_decoration_phase(
            &mut scene,
            &context.specs(),
            DecorationGeometry {
                x0,
                x1,
                abs_y: 0.0,
                line_top: 0.0,
                baseline: Some(0.0),
                pattern_origin_x: Some(0.0),
                pattern_end_x: Some(1_000_000.0),
            },
            DecorationPhase::BeforeGlyphs,
        );
        let RenderCommand::Stroke(stroke) = &scene.commands[0] else {
            panic!("expected stroke")
        };
        patterns.push(stroke.style.dash_pattern.clone());
    }
    assert_eq!(patterns[0], patterns[1]);
}
