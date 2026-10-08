use super::*;
use anyrender::Scene;
use anyrender::recording::RenderCommand;
use raikiri_dom::{Document, StandaloneAlign};
use raikiri_style::{build_rule_tree, cascade};
use taffy::Style;

#[test]
fn vertical_align_length_uses_css_raise_lower_sign_in_y_down_space() {
    assert_eq!(
        vertical_align_shift_px(
            VerticalAlign::Length(Length::Px(96.0)),
            DisplayValue::Inline,
            16.0,
        ),
        -96.0
    );
    assert_eq!(
        vertical_align_shift_px(
            VerticalAlign::Length(Length::Px(-12.0)),
            DisplayValue::InlineBlock,
            16.0,
        ),
        12.0
    );
    assert_eq!(
        vertical_align_shift_px(
            VerticalAlign::Length(Length::Px(96.0)),
            DisplayValue::Block,
            16.0,
        ),
        0.0
    );
}

fn list_fixture(
    first_style: &str,
    second_style: &str,
    marker_stylesheet: Option<&str>,
) -> (Document, CascadeResult, usize, usize) {
    let mut document = Document::new();
    if let Some(stylesheet) = marker_stylesheet {
        let style = document.append_element(
            Some(document.root_index()),
            "style",
            Style::default(),
            None::<&str>,
        );
        document.append_text(style, stylesheet);
    }
    let first = document.append_element(
        Some(document.root_index()),
        "li",
        Style::default(),
        Some(first_style),
    );
    let second = document.append_element(
        Some(document.root_index()),
        "li",
        Style::default(),
        Some(second_style),
    );
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    (document, cascade, first, second)
}

#[test]
fn generated_content_resolves_dom_attributes_and_fallbacks() {
    let mut document = Document::new();
    let element = document.append_element(
        Some(document.root_index()),
        "div",
        Style::default(),
        Some("display:block"),
    );
    document.set_element_attributes(element, vec![("data-value".into(), "Actual".into())]);
    let components = vec![
        ContentComponent::AttrFallback {
            name: "missing".into(),
            fallback: Some("Fallback".into()),
        },
        ContentComponent::Literal(" ".into()),
        ContentComponent::Attr {
            name: "data-value".into(),
        },
        ContentComponent::Literal(" ".into()),
        ContentComponent::AttrFallback {
            name: "missing-invalid".into(),
            fallback: None,
        },
    ];
    let registry = CounterStyleRegistry::new();
    let rendered = raikiri_dom::generated_content::content_components_to_text_with_quotes(
        &document,
        element,
        &components,
        &[] as &[(&str, &str)],
        false,
        &CounterSnapshot::default(),
        &registry,
    );
    assert_eq!(rendered.as_deref(), Some("Fallback Actual "));
}

#[test]
fn marker_render_info_uses_author_content_and_falls_back_to_list_style() {
    let (document, cascade, first, second) = list_fixture(
        "display: list-item; list-style-type: decimal",
        "display: list-item; list-style-type: none",
        Some(r##"li::marker { content: "#"; color: blue }"##),
    );
    let (_, content) = marker_render_info(&document, &cascade, first).expect("marker info");
    assert_eq!(content, "#");
    let (_, content) = marker_render_info(&document, &cascade, second)
        .expect("marker rule supplies content even when list-style is none");
    assert_eq!(content, "#");

    let (document, cascade, first, second) = list_fixture(
        "display: list-item; list-style-type: decimal",
        "display: list-item; list-style-type: none",
        None,
    );
    let (_, content) = marker_render_info(&document, &cascade, first).expect("fallback marker");
    assert_eq!(content, "1. ");
    assert!(marker_render_info(&document, &cascade, second).is_none());

    let (document, cascade, first, _) = list_fixture(
        "display: list-item; list-style-type: decimal; counter-reset: list-item 9",
        "display: list-item",
        None,
    );
    let (_, content) = marker_render_info(&document, &cascade, first)
        .expect("explicit list-item counter fallback marker");
    assert_eq!(content, "9. ");

    let (document, cascade, first, _) = list_fixture(
        "display: list-item; list-style-type: disc",
        "display: list-item",
        Some("li::marker { display: none }"),
    );
    assert!(marker_render_info(&document, &cascade, first).is_none());
}

#[test]
fn generated_pseudo_metrics_preserve_before_after_order() {
    let (document, cascade, first, _) = list_fixture(
        "display: list-item",
        "display: list-item",
        Some(r##"li::before { content: "A " } li::after { content: "B" }"##),
    );
    let snapshots = raikiri_dom::counter_snapshots(&document, &cascade)
        .expect("test counter snapshots stay within budget");
    assert!(
        generated_pseudo_text_height(
            &document,
            &cascade,
            first,
            raikiri_style::PseudoElem::Before,
            &snapshots
        ) > 0.0
    );
    assert!(
        generated_pseudo_text_height(
            &document,
            &cascade,
            first,
            raikiri_style::PseudoElem::After,
            &snapshots
        ) > 0.0
    );

    let mut scene = Scene::new();
    let before_advance = paint_generated_pseudo(
        &mut scene,
        &document,
        &cascade,
        first,
        raikiri_style::PseudoElem::Before,
        0.0,
        0.0,
        200.0,
        30.0,
        &snapshots,
    );
    let after_start = scene.commands.len();
    assert!(before_advance > 0.0);
    let _ = paint_generated_pseudo(
        &mut scene,
        &document,
        &cascade,
        first,
        raikiri_style::PseudoElem::After,
        before_advance,
        0.0,
        200.0 - before_advance,
        30.0,
        &snapshots,
    );
    assert!(scene.commands.len() > after_start);

    let (document, cascade, first, _) = list_fixture(
        "display: list-item",
        "display: list-item",
        Some(r##"li::before { display: none; content: "hidden" }"##),
    );
    let snapshots = raikiri_dom::counter_snapshots(&document, &cascade)
        .expect("test counter snapshots stay within budget");
    assert_eq!(
        generated_pseudo_text_height(
            &document,
            &cascade,
            first,
            raikiri_style::PseudoElem::Before,
            &snapshots
        ),
        0.0
    );
    let mut hidden_scene = Scene::new();
    assert_eq!(
        paint_generated_pseudo(
            &mut hidden_scene,
            &document,
            &cascade,
            first,
            raikiri_style::PseudoElem::Before,
            0.0,
            0.0,
            200.0,
            30.0,
            &snapshots,
        ),
        0.0
    );
}

#[test]
fn paint_document_skips_hidden_table_decoration() {
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = document.append_element(Some(html), "body", Style::default(), Some("display:block"));
    let table = document.append_element(
            Some(body),
            "table",
            Style::default(),
            Some(
                "display: table; width: 100px; height: 100px; box-sizing: border-box; border: 20px solid red; border-collapse: collapse; visibility: hidden",
            ),
        );
    let row = document.append_element(
        Some(table),
        "tr",
        Style::default(),
        Some("display: table-row"),
    );
    document.append_element(
        Some(row),
        "td",
        Style::default(),
        Some("display: table-cell"),
    );
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    raikiri_dom::layout_single_page(&mut document, &cascade, PageBox::A4).expect("layout Ok");

    let mut scene = Scene::new();
    crate::paint_single_page(&mut scene, &document, &cascade, PageBox::A4).expect("paint succeeds");
    let draws = scene
        .commands
        .iter()
        .filter(|command| matches!(command, RenderCommand::Fill(_) | RenderCommand::Stroke(_)))
        .count();
    // The only draw is the page canvas; the hidden table's border is absent.
    assert_eq!(draws, 1);
}

#[test]
fn paint_document_orders_literal_pseudos_around_direct_text() {
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let head = document.append_element(Some(html), "head", Style::default(), None::<&str>);
    let style = document.append_element(Some(head), "style", Style::default(), None::<&str>);
    document.append_text(
        style,
        r##"div::before { content: "BEFORE " } div::after { content: " AFTER" }"##,
    );
    let body = document.append_element(Some(html), "body", Style::default(), Some("display:block"));
    let div = document.append_element(Some(body), "div", Style::default(), Some("display:block"));
    document.append_text(div, "BODY");
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    raikiri_dom::layout_single_page(&mut document, &cascade, PageBox::A4).expect("layout Ok");

    let mut scene = Scene::new();
    crate::paint_single_page(&mut scene, &document, &cascade, PageBox::A4).expect("paint succeeds");
    let glyph_x: Vec<_> = scene
        .commands
        .iter()
        .filter_map(|command| match command {
            RenderCommand::GlyphRun(glyph_run) => Some(
                glyph_run.transform.as_coeffs()[4]
                    + glyph_run
                        .glyphs
                        .first()
                        .map_or(0.0, |glyph| f64::from(glyph.x)),
            ),
            _ => None,
        })
        .collect();
    assert!(
        glyph_x.len() >= 3,
        // cov:ignore: assert! diagnostic is only evaluated on failure
        "expected before, text, and after glyph runs"
    );
    let last_three = &glyph_x[glyph_x.len() - 3..];
    assert!(
        last_three[0] < last_three[1] && last_three[1] < last_three[2],
        // cov:ignore: assert! diagnostic is only evaluated on failure
        "literal pseudo/text runs must advance in source order: {last_three:?}"
    );
}

#[test]
fn paint_document_offsets_generated_counter_inline_siblings() {
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let head = document.append_element(Some(html), "head", Style::default(), None::<&str>);
    let style = document.append_element(Some(head), "style", Style::default(), None::<&str>);
    document.append_text(
            style,
            r##"div { counter-reset: c } div span { counter-increment: c } div span::before { content: counter(c) } div span::after { display: none; content: "hidden" }"##,
        );
    let body = document.append_element(Some(html), "body", Style::default(), Some("display:block"));
    let test = document.append_element(Some(body), "div", Style::default(), Some("display:block"));
    document.append_text(test, "\n");
    document.append_element(Some(test), "span", Style::default(), None::<&str>);
    document.append_text(test, "\n");
    document.append_element(Some(test), "span", Style::default(), None::<&str>);

    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    raikiri_dom::layout_single_page(&mut document, &cascade, PageBox::A4).expect("layout Ok");

    let mut scene = Scene::new();
    crate::paint_single_page(&mut scene, &document, &cascade, PageBox::A4).expect("paint succeeds");
    assert!(
        scene
            .commands
            .iter()
            .any(|command| matches!(command, RenderCommand::GlyphRun(_)))
    );
}

#[test]
fn paint_document_expands_auto_height_for_empty_pseudo_box() {
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let head = document.append_element(Some(html), "head", Style::default(), None::<&str>);
    let style = document.append_element(Some(head), "style", Style::default(), None::<&str>);
    document.append_text(
        style,
        r##"div { border: 2px solid black } div::before { content: "A" }"##,
    );
    let body = document.append_element(Some(html), "body", Style::default(), Some("display:block"));
    document.append_element(Some(body), "div", Style::default(), Some("display:block"));
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    raikiri_dom::layout_single_page(&mut document, &cascade, PageBox::A4).expect("layout Ok");

    let mut scene = Scene::new();
    crate::paint_single_page(&mut scene, &document, &cascade, PageBox::A4).expect("paint succeeds");
    assert!(
        scene
            .commands
            .iter()
            .any(|command| matches!(command, RenderCommand::GlyphRun(_)))
    );
}

#[test]
fn paint_generated_pseudo_emits_counter_content() {
    let (document, cascade, first, _) = list_fixture(
        "display: list-item; counter-reset: marker 3",
        "display: list-item",
        Some(r##"li::before { content: counter(marker) }"##),
    );
    let snapshots = raikiri_dom::counter_snapshots(&document, &cascade)
        .expect("test counter snapshots stay within budget");
    let mut scene = Scene::new();
    paint_generated_pseudo(
        &mut scene,
        &document,
        &cascade,
        first,
        raikiri_style::PseudoElem::Before,
        0.0,
        0.0,
        200.0,
        30.0,
        &snapshots,
    );
    assert!(
        scene
            .commands
            .iter()
            .any(|command| matches!(command, RenderCommand::GlyphRun(_)))
    );
}

#[test]
fn paint_list_marker_emits_text_and_honors_display_none() {
    let (document, cascade, first, second) = list_fixture(
        "display: list-item; list-style-type: disc; list-style-position: outside",
        "display: list-item; list-style-type: none",
        None,
    );
    let mut scene = Scene::new();
    paint_list_marker(
        &mut scene, &document, &cascade, first, 0.0, 0.0, 200.0, 30.0, 0.0,
    );
    assert!(
        scene
            .commands
            .iter()
            .any(|command| matches!(command, RenderCommand::GlyphRun(_)))
    );
    let before = scene.commands.len();
    paint_list_marker(
        &mut scene, &document, &cascade, second, 0.0, 0.0, 200.0, 30.0, 0.0,
    );
    assert_eq!(scene.commands.len(), before);

    // Inside markers use the DOM bridge's reserved gutter.
    let (document, cascade, first, _) = list_fixture(
        "display: list-item; list-style-position: inside",
        "display: list-item",
        None,
    );
    paint_list_marker(
        &mut scene, &document, &cascade, first, 0.0, 0.0, 200.0, 30.0, 0.0,
    );

    // Empty generated content and a zero-sized marker both fail closed
    // before a glyph command is emitted.
    let (document, cascade, first, _) = list_fixture(
        "display: list-item",
        "display: list-item",
        Some(r##"li::marker { content: none }"##),
    );
    let (_, content) = marker_render_info(&document, &cascade, first).expect("none marker");
    assert_eq!(content, "");
    let before = scene.commands.len();
    paint_list_marker(
        &mut scene, &document, &cascade, first, 0.0, 0.0, 200.0, 30.0, 0.0,
    );
    assert_eq!(scene.commands.len(), before);
    let (document, cascade, first, _) = list_fixture(
        "display: list-item; font-size: 0",
        "display: list-item",
        None,
    );
    paint_list_marker(
        &mut scene, &document, &cascade, first, 0.0, 0.0, 200.0, 30.0, 0.0,
    );
    paint_list_marker(
        &mut scene, &document, &cascade, first, 0.0, 0.0, 0.0, 30.0, 0.0,
    );
    let (document, cascade, first, _) = list_fixture(
        "display: list-item",
        "display: list-item",
        Some(r##"li::marker { content: "\u{200b}" }"##),
    );
    paint_list_marker(
        &mut scene, &document, &cascade, first, 0.0, 0.0, 200.0, 30.0, 0.0,
    );
}

#[test]
fn border_radius_normalization_scales_adjacent_edges() {
    assert_eq!(
        used_border_radii(
            &ComputedBorderRadius::all(ComputedLength(80.0)),
            100.0,
            100.0
        ),
        [[50.0, 50.0]; 4]
    );
}

#[test]
fn elliptical_background_paths_keep_both_axes_and_independent_insets() {
    let radius = ComputedBorderRadius::elliptical(
        [ComputedLengthPercentage::Px(30.0); 4],
        [ComputedLengthPercentage::Px(15.0); 4],
    );
    let path = rounded_background_path(
        0.0,
        0.0,
        100.0,
        50.0,
        &radius,
        (0.0, 0.0, 0.0, 0.0),
        (100.0, 50.0),
    )
    .unwrap();
    assert_eq!(
        path.elements()[0],
        kurbo::PathEl::MoveTo(Point::new(0.0, 15.0))
    );
    let inset = rounded_background_path(
        4.0,
        2.0,
        94.0,
        47.0,
        &radius,
        (4.0, 2.0, 6.0, 3.0),
        (100.0, 50.0),
    )
    .unwrap();
    assert_eq!(
        inset.elements()[0],
        kurbo::PathEl::MoveTo(Point::new(4.0, 15.0))
    );
    assert_eq!(
        kurbo::Shape::bounding_box(&inset),
        Rect::new(4.0, 2.0, 94.0, 47.0)
    );
}

#[test]
fn elliptical_background_raster_distinguishes_the_vertical_axis() {
    let radius = ComputedBorderRadius::elliptical(
        [ComputedLengthPercentage::Px(30.0); 4],
        [ComputedLengthPercentage::Px(15.0); 4],
    );
    let mut scene = Scene::new();
    fill_rounded_background(
        &mut scene,
        Color::from_rgba8(255, 0, 0, 255),
        0.0,
        0.0,
        100.0,
        50.0,
        &radius,
        (0.0, 0.0, 0.0, 0.0),
        (100.0, 50.0),
    );
    let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(scene, Affine::IDENTITY),
        100,
        50,
    );
    assert_eq!(
        &rgba[(8 * 100 + 8) * 4..(8 * 100 + 8) * 4 + 4],
        &[255, 0, 0, 255]
    );
    assert_eq!(rgba[(2 * 100 + 8) * 4 + 3], 0);
}

#[test]
fn border_radius_paint_keeps_both_axes_and_percentages() {
    let radius = ComputedBorderRadius::elliptical(
        [ComputedLengthPercentage::Px(12.0); 4],
        [ComputedLengthPercentage::Percent(25.0); 4],
    );
    assert_eq!(paintable_border_radius(&radius, true), radius);
    assert_eq!(
        paintable_border_radius(&radius, false).used(100.0, 50.0),
        [[0.0, 0.0]; 4]
    );
}

#[test]
fn background_image_geometry_covers_supported_size_and_position_forms() {
    let decoded = raikiri_traits::DecodedImage {
        width: 2,
        height: 1,
        rgba: vec![255, 0, 0, 255, 0, 255, 0, 255],
    };
    // Construct the non-exhaustive computed position values through the
    // real parser/cascade boundary rather than bypassing their visibility
    // contract with struct literals.
    let mut document = Document::new();
    let start_px_id = document.append_element(
        Some(document.root_index()),
        "div",
        Style::default(),
        Some("display:block;background-position: 3px 4px"),
    );
    let start_percent_id = document.append_element(
        Some(document.root_index()),
        "div",
        Style::default(),
        Some("display:block;background-position: 25% 50%"),
    );
    let end_px_id = document.append_element(
        Some(document.root_index()),
        "div",
        Style::default(),
        Some("display:block;background-position: right 3px bottom 4px"),
    );
    let end_percent_id = document.append_element(
        Some(document.root_index()),
        "div",
        Style::default(),
        Some("display:block;background-position: bottom 50% right 25%"),
    );
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    let repeat = cascade.computed[start_px_id].background_repeat;
    let start_px = cascade.computed[start_px_id].background_position;
    let start_percent = cascade.computed[start_percent_id].background_position;
    let end_px = cascade.computed[end_px_id].background_position;
    let end_percent = cascade.computed[end_percent_id].background_position;
    let mut scene = Scene::new();
    paint_background_image(
        &mut scene,
        &decoded,
        kurbo::Rect::new(0.0, 0.0, 100.0, 50.0),
        kurbo::Rect::new(0.0, 0.0, 100.0, 50.0),
        100.0,
        50.0,
        &start_px,
        &repeat,
    );
    paint_background_image(
        &mut scene,
        &decoded,
        kurbo::Rect::new(0.0, 0.0, 100.0, 50.0),
        kurbo::Rect::new(0.0, 0.0, 100.0, 50.0),
        100.0,
        50.0,
        &start_percent,
        &repeat,
    );
    paint_background_image(
        &mut scene,
        &decoded,
        kurbo::Rect::new(0.0, 0.0, 100.0, 50.0),
        kurbo::Rect::new(0.0, 0.0, 100.0, 50.0),
        20.0,
        10.0,
        &end_px,
        &repeat,
    );
    paint_background_image(
        &mut scene,
        &decoded,
        kurbo::Rect::new(0.0, 0.0, 100.0, 50.0),
        kurbo::Rect::new(0.0, 0.0, 100.0, 50.0),
        50.0,
        12.5,
        &end_percent,
        &repeat,
    );
    paint_background_image(
        &mut scene,
        &decoded,
        kurbo::Rect::new(0.0, 0.0, 100.0, 50.0),
        kurbo::Rect::new(0.0, 0.0, 100.0, 50.0),
        12.0,
        6.0,
        &start_px,
        &repeat,
    );
    paint_background_image(
        &mut scene,
        &decoded,
        kurbo::Rect::new(0.0, 0.0, 100.0, 50.0),
        kurbo::Rect::new(0.0, 0.0, 100.0, 50.0),
        22.0,
        11.0,
        &start_px,
        &repeat,
    );
    paint_background_image(
        &mut scene,
        &decoded,
        kurbo::Rect::new(0.0, 0.0, 100.0, 50.0),
        kurbo::Rect::new(0.0, 0.0, 100.0, 50.0),
        2.0,
        1.0,
        &start_px,
        &repeat,
    );
    assert!(!scene.commands.is_empty());

    let before = scene.commands.len();
    paint_background_image(
        &mut scene,
        &decoded,
        kurbo::Rect::new(0.0, 0.0, 0.0, 50.0),
        kurbo::Rect::new(0.0, 0.0, 0.0, 50.0),
        100.0,
        50.0,
        &start_px,
        &repeat,
    );
    let zero = raikiri_traits::DecodedImage {
        width: 0,
        height: 1,
        rgba: Vec::new(),
    };
    paint_background_image(
        &mut scene,
        &zero,
        kurbo::Rect::new(0.0, 0.0, 100.0, 50.0),
        kurbo::Rect::new(0.0, 0.0, 100.0, 50.0),
        100.0,
        50.0,
        &start_px,
        &repeat,
    );
    paint_background_image(
        &mut scene,
        &decoded,
        kurbo::Rect::new(0.0, 0.0, 100.0, 50.0),
        kurbo::Rect::new(0.0, 0.0, 100.0, 50.0),
        -1.0,
        10.0,
        &start_px,
        &repeat,
    );
    assert_eq!(scene.commands.len(), before);
}

#[test]
fn background_image_dimensions_resolve_size_against_intrinsic_metadata() {
    let intrinsic = raikiri_traits::ImageIntrinsicSize {
        width: Some(2.0),
        height: Some(1.0),
        aspect_ratio: Some(2.0),
    };

    assert_eq!(
        background_image_dimensions(&ComputedBackgroundSize::Cover, 100.0, 50.0, intrinsic),
        Some((100.0, 50.0))
    );
    assert_eq!(
        background_image_dimensions(&ComputedBackgroundSize::Contain, 100.0, 50.0, intrinsic),
        Some((100.0, 50.0))
    );
    assert_eq!(
        background_image_dimensions(
            &ComputedBackgroundSize::Explicit {
                width: ComputedLengthPercentageOrAuto::Px(20.0),
                height: ComputedLengthPercentageOrAuto::Px(10.0),
            },
            100.0,
            50.0,
            intrinsic,
        ),
        Some((20.0, 10.0))
    );
    assert_eq!(
        background_image_dimensions(
            &ComputedBackgroundSize::Explicit {
                width: ComputedLengthPercentageOrAuto::Percent(50.0),
                height: ComputedLengthPercentageOrAuto::Percent(25.0),
            },
            100.0,
            50.0,
            intrinsic,
        ),
        Some((50.0, 12.5))
    );
    assert_eq!(
        background_image_dimensions(
            &ComputedBackgroundSize::Explicit {
                width: ComputedLengthPercentageOrAuto::Calc(
                    raikiri_style::property::CalcLengthPercentage {
                        percent: 10.0,
                        px: 2.0,
                    },
                ),
                height: ComputedLengthPercentageOrAuto::Auto,
            },
            100.0,
            50.0,
            intrinsic,
        ),
        Some((12.0, 6.0))
    );
    assert_eq!(
        background_image_dimensions(
            &ComputedBackgroundSize::Explicit {
                width: ComputedLengthPercentageOrAuto::Auto,
                height: ComputedLengthPercentageOrAuto::Calc(
                    raikiri_style::property::CalcLengthPercentage {
                        percent: 20.0,
                        px: 1.0,
                    },
                ),
            },
            100.0,
            50.0,
            intrinsic,
        ),
        Some((22.0, 11.0))
    );
    assert_eq!(
        background_image_dimensions(
            &ComputedBackgroundSize::Explicit {
                width: ComputedLengthPercentageOrAuto::Auto,
                height: ComputedLengthPercentageOrAuto::Auto,
            },
            100.0,
            50.0,
            intrinsic,
        ),
        Some((2.0, 1.0))
    );
    assert_eq!(
        background_image_dimensions(
            &ComputedBackgroundSize::Explicit {
                width: ComputedLengthPercentageOrAuto::Px(-1.0),
                height: ComputedLengthPercentageOrAuto::Px(10.0),
            },
            100.0,
            50.0,
            intrinsic,
        ),
        None
    );
}

#[test]
fn inherited_margin_box_font_uses_root_computed_family() {
    let mut document = Document::new();
    let style = document.append_element(
        Some(document.root_index()),
        "style",
        Style::default(),
        None::<&str>,
    );
    document.append_text(style, "@page { @top-left { content: 'x'; } }");
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    let rule = cascade
        .page
        .margin_boxes()
        .first()
        .expect("the @page fixture has a margin box");

    assert_eq!(
        inherited_margin_box_font(&document, &cascade, rule),
        (16.0, "serif".to_owned())
    );
}

#[test]
fn ratio_only_auto_background_uses_positioning_area_as_default_size() {
    let intrinsic = raikiri_traits::ImageIntrinsicSize {
        width: None,
        height: None,
        aspect_ratio: Some(2.0),
    };
    let auto = ComputedBackgroundSize::Explicit {
        width: ComputedLengthPercentageOrAuto::Auto,
        height: ComputedLengthPercentageOrAuto::Auto,
    };

    assert_eq!(
        background_image_dimensions(&auto, 600.0, 400.0, intrinsic),
        Some((600.0, 300.0))
    );
    assert_eq!(
        background_image_dimensions(&ComputedBackgroundSize::Contain, 600.0, 400.0, intrinsic),
        Some((600.0, 300.0))
    );
    assert_eq!(
        background_image_dimensions(&ComputedBackgroundSize::Cover, 600.0, 400.0, intrinsic),
        Some((800.0, 400.0))
    );
}

#[test]
fn paint_root_element_border_without_html_is_noop() {
    let document = Document::new();
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    let mut scene = Scene::new();
    paint_root_element_border(&mut scene, &document, &cascade, PageBox::A4);
    assert!(scene.commands.is_empty());
}

#[test]
fn paint_root_element_border_paints_html_border_sides() {
    let mut document = Document::new();
    document.append_element(
        Some(document.root_index()),
        "html",
        Style::default(),
        Some("border: 5px solid red"),
    );
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    let mut scene = Scene::new();
    paint_root_element_border(&mut scene, &document, &cascade, PageBox::A4);
    let fills = scene
        .commands
        .iter()
        .filter(|command| matches!(command, RenderCommand::Fill(_)))
        .count();
    assert_eq!(fills, 4, "one fill strip per border side");
}

fn canvas_fixture(page_css: Option<&str>, body_style: Option<&str>) -> (Document, CascadeResult) {
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let head = document.append_element(Some(html), "head", Style::default(), None::<&str>);
    if let Some(css) = page_css {
        let style = document.append_element(Some(head), "style", Style::default(), None::<&str>);
        document.append_text(style, css);
    }
    let body = document.append_element(Some(html), "body", Style::default(), body_style);
    document.append_text(body, "hi");
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    (document, cascade)
}

fn fill_count(document: &Document, cascade: &CascadeResult) -> usize {
    let mut scene = Scene::new();
    let mut warnings = Vec::new();
    paint_canvas_background(
        &mut scene,
        document,
        cascade,
        PageBox::A4,
        None,
        &mut warnings,
    );
    assert!(warnings.is_empty());
    scene
        .commands
        .iter()
        .filter(|command| matches!(command, RenderCommand::Fill(_)))
        .count()
}

#[test]
fn canvas_background_paints_body_color_over_white_page() {
    let (document, cascade) = canvas_fixture(None, Some("background-color: blue"));
    assert!(raikiri_dom::page_margins(&cascade, PageBox::A4).is_zero());
    assert_eq!(fill_count(&document, &cascade), 2);
}

#[test]
fn canvas_background_without_body_color_paints_page_only() {
    let (document, cascade) = canvas_fixture(None, None);
    assert_eq!(fill_count(&document, &cascade), 1);
}

#[test]
fn canvas_background_with_page_margins_keeps_canvas_layer() {
    let (document, cascade) = canvas_fixture(
        Some("@page { margin: 5px; }"),
        Some("background-color: blue"),
    );
    assert!(!raikiri_dom::page_margins(&cascade, PageBox::A4).is_zero());
    assert_eq!(fill_count(&document, &cascade), 2);
}

fn margin_row_margins(top: f32) -> raikiri_dom::PageMargins {
    raikiri_dom::PageMargins {
        top,
        right: 10.0,
        bottom: 10.0,
        left: 10.0,
    }
}

fn fixed_margin_spec() -> MarginBoxPaintSpec {
    let initial = ComputedValues::initial();
    MarginBoxPaintSpec {
        slot: PageMarginBoxSlot::TopCenter,
        content: String::new(),
        background: Some(Color::from_rgba8(255, 0, 0, 255)),
        background_image_url: None,
        background_image_lime: false,
        background_size: initial.background_size,
        background_position: initial.background_position,
        background_repeat: initial.background_repeat,
        background_origin: initial.background_origin,
        background_clip: initial.background_clip,
        content_image_lime: false,
        border_top: None,
        border_right: None,
        border_bottom: None,
        border_left: None,
        margin_auto: [false; 4],
        margin: [0.0; 4],
        padding: [0.0; 4],
        width: Some(100.0),
        height: None,
        text_color: Color::from_rgba8(0, 0, 0, 255),
        text_style: crate::standalone_text::style(16.0, ""),
        alignment: StandaloneAlign::Start,
        vertical_align: text::MarginTextVerticalAlign::Top,
    }
}

fn paint_margin_row(
    specs: &[MarginBoxPaintSpec],
    top: bool,
    margins: raikiri_dom::PageMargins,
) -> usize {
    let mut scene = Scene::new();
    let mut warnings = Vec::new();
    paint_horizontal_margin_boxes(
        &mut scene,
        &Document::new(),
        specs,
        top,
        PageBox::A4.width,
        PageBox::A4.height,
        margins,
        None,
        &mut warnings,
    );
    assert!(warnings.is_empty());
    scene
        .commands
        .iter()
        .filter(|command| matches!(command, RenderCommand::Fill(_)))
        .count()
}

#[test]
fn margin_row_without_specs_paints_nothing() {
    assert_eq!(paint_margin_row(&[], true, margin_row_margins(10.0)), 0);
}

#[test]
fn margin_row_without_row_height_paints_nothing() {
    assert_eq!(
        paint_margin_row(&[fixed_margin_spec()], true, margin_row_margins(0.0)),
        0
    );
}

#[test]
fn margin_row_paints_fixed_width_background() {
    assert_eq!(
        paint_margin_row(&[fixed_margin_spec()], true, margin_row_margins(10.0)),
        1
    );
}

fn transformed_box_scene(transform: &str, origin: &str) -> Scene {
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = document.append_element(Some(html), "body", Style::default(), Some("margin:0"));
    document.append_element(Some(body), "div", Style::default(), Some(format!(
        "display:block;position:absolute;left:10px;top:20px;width:100px;height:50px;background:green;transform:{transform};transform-origin:{origin}"
    )));
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).unwrap();
    raikiri_dom::layout_single_page(&mut document, &cascade, PageBox::A4).unwrap();
    let mut scene = Scene::new();
    crate::paint_single_page(&mut scene, &document, &cascade, PageBox::A4).expect("paint succeeds");
    scene
}

#[test]
fn css_transforms_move_painted_box_corners_about_the_authored_origin() {
    // Hand-derived points for the border box (10,20)..(110,70).
    // Omitting a matrix function, reversing composition, or ignoring the
    // authored origin changes these painted corner positions.
    for (transform, origin, first, opposite) in [
        ("skew(45deg, 0deg)", "0 0", (10., 20.), (160., 70.)),
        ("skewX(45deg)", "0 0", (10., 20.), (160., 70.)),
        ("skewY(45deg)", "0 0", (10., 20.), (110., 170.)),
        ("scale(2, 3)", "0 0", (10., 20.), (210., 170.)),
        ("scaleX(2)", "0 0", (10., 20.), (210., 70.)),
        ("scaleY(3)", "0 0", (10., 20.), (110., 170.)),
        ("rotate(90deg)", "0 0", (10., 20.), (-40., 120.)),
        ("rotate(90deg)", "50% 50%", (85., -5.), (35., 95.)),
        ("rotate(90deg)", "right bottom", (160., -30.), (110., 70.)),
        ("matrix(1, 0, 1, 1, 5, 7)", "0 0", (15., 27.), (165., 77.)),
        (
            "translate(10px, 5px) scale(2)",
            "0 0",
            (20., 25.),
            (220., 125.),
        ),
        (
            "scale(2) translate(10px, 5px)",
            "0 0",
            (30., 30.),
            (230., 130.),
        ),
        (
            "scale(2) translateX(10%) translateY(20%)",
            "0 0",
            (30., 40.),
            (230., 140.),
        ),
    ] {
        let scene = transformed_box_scene(transform, origin);
        let fill = scene
            .commands
            .iter()
            .filter_map(|command| match command {
                RenderCommand::Fill(fill) => Some(fill),
                _ => None,
            })
            .next_back()
            .unwrap();
        let bbox = kurbo::Shape::bounding_box(&fill.shape);
        for (point, expected) in [
            (bbox.origin(), first),
            (Point::new(bbox.x1, bbox.y1), opposite),
        ] {
            let actual = fill.transform * point;
            assert!(
                (actual.x - expected.0).abs() < 1e-5 && (actual.y - expected.1).abs() < 1e-5,
                "{transform} / {origin}: {actual:?} != {expected:?}"
            );
        }
    }
}

fn transform_markup_scene(markup: &str) -> Scene {
    let mut parsed = raikiri_html::parse(
        markup.as_bytes(),
        &raikiri_html::ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        },
    )
    .unwrap();
    let cascade = raikiri_html::build_cascaded(&parsed);
    raikiri_dom::layout_single_page(&mut parsed.dom, &cascade, PageBox::A4).unwrap();
    let mut scene = Scene::new();
    crate::paint_single_page(&mut scene, &parsed.dom, &cascade, PageBox::A4)
        .expect("paint succeeds");
    scene
}

#[test]
fn review_atomic_column_descendants_paint_once_after_the_legal_break() {
    let scene = transform_markup_scene(
        "<!DOCTYPE html><body style='margin:0;background:white'><div style='columns:2;column-fill:auto;gap:0;width:100px;height:100px'><div style='height:30px'></div><div style='height:150px;break-before:avoid'><div style='height:75px;background:red'></div><div style='height:75px;background:green'></div></div></div></body>",
    );
    let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |renderer| renderer.append_scene(scene, Affine::IDENTITY),
        800,
        600,
    );
    let pixel = |x: usize, y: usize| &rgba[(y * 800 + x) * 4..(y * 800 + x) * 4 + 4];
    assert_eq!(pixel(10, 50), &[255, 255, 255, 255]);
    assert_eq!(pixel(60, 10), &[255, 0, 0, 255]);
    assert_eq!(pixel(60, 100), &[0, 128, 0, 255]);
}

#[test]
fn review_column_container_background_and_border_use_its_border_box() {
    let scene = transform_markup_scene(
        "<!DOCTYPE html><body style='margin:0;background:white'><div style='columns:2;column-fill:auto;gap:0;box-sizing:border-box;width:120px;height:130px;padding:10px;border:5px solid blue;background:red'><div style='height:20px;break-before:avoid'></div></div></body>",
    );
    let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |renderer| renderer.append_scene(scene, Affine::IDENTITY),
        800,
        600,
    );
    let pixel = |x: usize, y: usize| &rgba[(y * 800 + x) * 4..(y * 800 + x) * 4 + 4];
    assert_eq!(pixel(110, 50), &[255, 0, 0, 255]);
    assert_eq!(pixel(118, 50), &[0, 0, 255, 255]);
    assert_eq!(pixel(125, 50), &[255, 255, 255, 255]);
}

#[test]
fn relative_block_offsets_are_applied_once_to_boxes_and_descendants() {
    for display in [
        "block",
        "flex",
        "grid",
        "inline-block",
        "inline-flex",
        "inline-grid",
    ] {
        for (insets, x, y) in [
            ("left:-10px;top:-10px", 15.0, 25.0),
            ("left:10px;top:12px", 35.0, 47.0),
            ("right:10px;bottom:10px", 15.0, 25.0),
        ] {
            let scene = transform_markup_scene(&format!(
                "<body style='margin:0'><div style='position:absolute;left:10px;top:20px;width:100px;height:80px;padding:10px;border:5px solid red'><div style='display:{display};position:relative;{insets};width:20px;height:20px;background:green'><div style='width:10px;height:10px;background:blue'></div></div></div>",
            ));
            let fills: Vec<_> = scene
                .commands
                .iter()
                .filter_map(|command| match command {
                    RenderCommand::Fill(fill) => Some(fill),
                    _ => None,
                })
                .collect();
            let green = fills[fills.len() - 2];
            let blue = fills[fills.len() - 1];
            for (fill, size) in [(green, 20.0), (blue, 10.0)] {
                let rect = kurbo::Shape::bounding_box(&fill.shape);
                assert_eq!(fill.transform * rect.origin(), Point::new(x, y), "{insets}");
                assert_eq!(
                    fill.transform * Point::new(rect.x1, rect.y1),
                    Point::new(x + size, y + size),
                    "{insets}"
                );
            }
        }
    }
}

#[test]
fn a_manually_placed_table_caption_retains_its_relative_offset() {
    let scene = transform_markup_scene(
        "<body style='margin:0'><table style='width:100px'><caption style='width:100px;height:50px;margin-left:200px;position:relative;left:-200px;background:green'></caption><tr><td></td></tr></table>",
    );
    let fill = scene
        .commands
        .iter()
        .rev()
        .find_map(|command| match command {
            RenderCommand::Fill(fill) => Some(fill),
            _ => None,
        })
        .expect("caption background");
    let rect = kurbo::Shape::bounding_box(&fill.shape);
    assert_eq!((fill.transform * rect.origin()).x, 0.0);
}

#[test]
fn manually_placed_roots_floats_and_multicol_children_keep_relative_offsets() {
    for (markup, expected) in [
        (
            "<body style='margin:0;position:relative;left:10px;top:12px'><div style='width:20px;height:20px;background:green'></div>",
            Point::new(10.0, 12.0),
        ),
        (
            "<body style='margin:0'><div style='width:100px;height:80px'><div style='float:left;position:relative;left:10px;top:12px;width:20px;height:20px;background:green'></div></div>",
            Point::new(10.0, 12.0),
        ),
        (
            "<body style='margin:0'><div style='display:flex;width:100px;height:80px'><div style='float:left;position:relative;left:10px;top:12px;width:20px;height:20px;background:green'></div></div>",
            Point::new(10.0, 12.0),
        ),
        (
            "<body style='margin:0'><div style='display:grid;width:100px;height:80px'><div style='float:left;position:relative;left:10px;top:12px;width:20px;height:20px;background:green'></div></div>",
            Point::new(10.0, 12.0),
        ),
        (
            "<body style='margin:0'><div style='width:200px;column-count:2;column-gap:0'><div style='display:grid;position:relative;top:12px;width:20px;height:20px;background:green'></div></div>",
            Point::new(0.0, 12.0),
        ),
        (
            "<body style='margin:0'><div style='column-count:2;width:200px;height:60px'><div style='min-block-size:40px;break-inside:avoid;position:relative;left:10px;top:12px;width:20px;background:green'></div></div>",
            Point::new(10.0, 12.0),
        ),
        (
            "<body style='margin:0'><div style='column-count:2;width:200px;height:60px'><div style='min-block-size:40px;break-inside:avoid'></div><div style='position:relative;left:10px;top:12px;width:20px;height:10px;background:green'></div></div>",
            Point::new(10.0, 92.0),
        ),
    ] {
        let scene = transform_markup_scene(markup);
        let fill = scene
            .commands
            .iter()
            .rev()
            .find_map(|command| match command {
                RenderCommand::Fill(fill) => Some(fill),
                _ => None,
            })
            .expect("child background");
        let rect = kurbo::Shape::bounding_box(&fill.shape);
        assert_eq!(fill.transform * rect.origin(), expected, "{markup}");
    }
}

#[test]
fn transformed_opacity_group_keeps_its_clip_in_page_coordinates() {
    let scene = transform_markup_scene(
        "<body style='margin:0'><div style='position:absolute;left:900px;top:0;width:100px;height:100px;background:green;opacity:.5;transform:matrix(1,0,0,1,-900,0)'></div>",
    );
    let clip = scene
        .commands
        .iter()
        .find_map(|command| match command {
            RenderCommand::PushLayer(layer) => Some(layer),
            _ => None,
        })
        .unwrap();
    let bounds = kurbo::Shape::bounding_box(&clip.clip);
    assert_eq!(clip.transform * bounds.origin(), Point::new(0.0, 0.0));
    let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(scene, Affine::IDENTITY),
        800,
        600,
    );
    let pixel = &rgba[(10 * 800 + 10) * 4..(10 * 800 + 10) * 4 + 4];
    // The backend quantizes half coverage to bytes; either rounding is valid.
    for (actual, expected) in pixel.iter().zip([128u8, 192, 128, 255]) {
        assert!(actual.abs_diff(expected) <= 1);
    }
}

#[test]
fn non_replaced_inline_ignores_own_transform_and_retains_ancestor_matrix() {
    for display in ["inline", "inline-block"] {
        let scene = transform_markup_scene(&format!(
            "<body style='margin:0'><div style='transform:scale(2);transform-origin:0 0'><span style='display:{display};transform:scale(0)'>TEXT</span></div>"
        ));
        let glyph = scene.commands.iter().find_map(|command| match command {
            RenderCommand::GlyphRun(glyph) => Some(glyph),
            _ => None,
        });
        if display == "inline" {
            let coeffs = glyph
                .expect("non-replaced inline text must remain visible")
                .transform
                .as_coeffs();
            assert_eq!(&coeffs[..4], &[2.0, 0.0, 0.0, 2.0]);
        } else {
            assert!(
                glyph.is_none(),
                "singular inline-block transform hides text"
            );
        }
    }
}

#[test]
fn nested_transforms_cover_descendant_glyphs_and_overflow_without_leaking_to_siblings() {
    let scene = transform_markup_scene(
        "<body style='margin:0'><div style='position:absolute;left:10px;top:20px;width:100px;height:50px;overflow:hidden;transform:scale(2);transform-origin:0 0'><div style='position:absolute;left:5px;top:7px;width:10px;height:8px;background:blue;transform:rotate(90deg);transform-origin:0 0'>TEXT</div></div><div style='position:absolute;left:300px;top:0;width:10px;height:10px;background:red'></div>",
    );
    let fills: Vec<_> = scene
        .commands
        .iter()
        .filter_map(|command| match command {
            RenderCommand::Fill(fill) => Some(fill),
            _ => None,
        })
        .collect();
    let blue = fills[fills.len() - 2];
    let bbox = kurbo::Shape::bounding_box(&blue.shape);
    let first = blue.transform * bbox.origin();
    let opposite = blue.transform * Point::new(bbox.x1, bbox.y1);
    assert!((first.x - 20.0).abs() < 1e-5 && (first.y - 34.0).abs() < 1e-5);
    assert!((opposite.x - 4.0).abs() < 1e-5 && (opposite.y - 54.0).abs() < 1e-5);
    assert_eq!(fills.last().unwrap().transform, Affine::IDENTITY);
    let clip = scene
        .commands
        .iter()
        .find_map(|command| match command {
            RenderCommand::PushClipLayer(clip) => Some(clip),
            _ => None,
        })
        .unwrap();
    assert_eq!(&clip.transform.as_coeffs()[..4], &[2.0, 0.0, 0.0, 2.0]);
    let glyph = scene
        .commands
        .iter()
        .find_map(|command| match command {
            RenderCommand::GlyphRun(glyph) => Some(glyph),
            _ => None,
        })
        .unwrap();
    let coeffs = glyph.transform.as_coeffs();
    for (actual, expected) in coeffs[..4].iter().zip([0.0, 2.0, -2.0, 0.0]) {
        assert!((actual - expected).abs() < 1e-5);
    }
}

#[test]
fn transformed_text_and_image_fallback_keep_page_content_clips_untransformed() {
    let scene = transform_markup_scene(
        "<style>@page { margin-left:200px; margin-right:200px }</style><body style='margin:0'><div style='width:100px;height:50px;background:green;transform:scale(2);transform-origin:0 0'>TEXT<img style='display:block;width:10px;height:10px' src='green-100.png'></div>",
    );
    let clips: Vec<_> = scene
        .commands
        .iter()
        .filter_map(|command| match command {
            RenderCommand::PushClipLayer(clip) => Some(clip),
            _ => None,
        })
        .collect();
    assert!(
        clips.len() >= 3,
        "background, glyph and fallback page clips must be exercised"
    );
    for clip in clips {
        assert_eq!(clip.transform, Affine::IDENTITY);
    }
    assert!(
        scene
            .commands
            .iter()
            .any(|command| matches!(command, RenderCommand::GlyphRun(_)))
    );
}

#[test]
fn replaced_inline_image_composes_own_and_ancestor_transform() {
    struct Pixels;
    impl ImagePixelSource for Pixels {
        fn get_decoded(
            &self,
            _: &url::Url,
        ) -> Option<std::sync::Arc<raikiri_traits::DecodedImage>> {
            Some(std::sync::Arc::new(raikiri_traits::DecodedImage {
                width: 4,
                height: 4,
                rgba: [0u8, 128, 0, 255].repeat(16),
            }))
        }
    }
    let mut parsed = raikiri_html::parse("<body style='margin:0'><div style='transform:scale(2);transform-origin:0 0'><img src='https://example.test/green.png' style='width:10px;height:10px;transform:scale(3);transform-origin:0 0'></div>".as_bytes(), &raikiri_html::ParseOptions { extra_stylesheets: &[], network: None, base_url: None }).unwrap();
    let cascade = raikiri_html::build_cascaded(&parsed);
    raikiri_dom::layout_single_page(&mut parsed.dom, &cascade, PageBox::A4).unwrap();
    let mut scene = Scene::new();
    let warnings = crate::paint_single_page_with_images_and_warnings(
        &mut scene,
        &parsed.dom,
        &cascade,
        PageBox::A4,
        &Pixels,
        &mut raikiri_dom::CounterSnapshotBudget::default(),
    )
    .expect("test paint stays within the counter snapshot budget");
    assert!(warnings.is_empty());
    let image = scene
        .commands
        .iter()
        .find_map(|command| match command {
            RenderCommand::Fill(fill) if matches!(fill.brush, anyrender::Paint::Image(_)) => {
                Some(fill)
            }
            _ => None,
        })
        .unwrap();
    // 4 source pixels map to 10 CSS pixels, then scale by 3 and 2.
    assert_eq!(&image.transform.as_coeffs()[..4], &[15.0, 0.0, 0.0, 15.0]);
}

#[test]
fn quarter_turn_preserves_axis_aligned_pixel_edges() {
    let scene = transform_markup_scene(
        "<!DOCTYPE html><style>body {margin:0} div {width:100px;height:100px;background:green;transform:rotate(90deg);transform-origin:50px 50px}</style><div></div>",
    );
    let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |renderer| renderer.append_scene(scene, Affine::IDENTITY),
        800,
        600,
    );
    for y in 0..100 {
        for x in 0..100 {
            let index = (y * 800 + x) * 4;
            assert_eq!(&rgba[index..index + 4], &[0, 128, 0, 255]);
        }
    }
}

#[test]
fn quarter_turn_uses_the_painted_border_box_at_fractional_layout_origin() {
    let raster = |markup| {
        let scene = transform_markup_scene(markup);
        anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
            |renderer| renderer.append_scene(scene, Affine::IDENTITY),
            800,
            600,
        )
    };
    let actual = raster(
        "<!DOCTYPE html><style>body {margin:0} #outer {position:absolute;top:50.2px;left:0;width:100px;height:100px;background:red} #inner {width:100px;height:100px;background:green;transform:rotate(90deg);transform-origin:50px 50px}</style><div id='outer'><div id='inner'></div></div>",
    );
    let expected = raster(
        "<!DOCTYPE html><style>body {margin:0} div {position:absolute;top:50.2px;left:0;width:100px;height:100px;background:green}</style><div></div>",
    );
    assert!(actual == expected);
}

#[test]
fn fractional_transform_origins_and_percentage_translation_share_painted_bounds() {
    for (origin, expected) in [
        ("0 0", (0.0, 0.0)),
        ("50% 50%", (-50.5, -5.5)),
        ("right bottom", (-101.0, -11.0)),
    ] {
        let mut document = Document::new();
        document.append_element(
            Some(0),
            "div",
            Style::default(),
            Some(format!("transform:scale(2);transform-origin:{origin}")),
        );
        let rules = build_rule_tree(&document);
        let cascade = cascade(&document, &rules).unwrap();
        let matrix = element_transform(&cascade.computed[1], 0.4, 0.4, 100.2, 10.4);
        let actual = matrix * Point::new(0.0, 0.0);
        assert_eq!(actual, Point::new(expected.0, expected.1));
    }
    let mut document = Document::new();
    document.append_element(
        Some(0),
        "div",
        Style::default(),
        Some("display:block;transform:scale(1) translate(100%,100%);transform-origin:0 0"),
    );
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).unwrap();
    let actual =
        element_transform(&cascade.computed[1], 0.4, 0.4, 100.2, 10.4) * Point::new(0.0, 0.0);
    assert_eq!(actual, Point::new(101.0, 11.0));
}

fn background_repeat_fixture(
    repeat: &str,
    position: &str,
) -> (
    ComputedCssPosition,
    raikiri_style::property::BackgroundRepeat,
) {
    let mut document = Document::new();
    let id = document.append_element(
        Some(document.root_index()),
        "div",
        Style::default(),
        Some(format!(
            "background-repeat:{repeat};background-position:{position}"
        )),
    );
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    (
        cascade.computed[id].background_position,
        cascade.computed[id].background_repeat,
    )
}

fn background_fill_origins(scene: &Scene) -> Vec<(f64, f64)> {
    scene
        .commands
        .iter()
        .filter_map(|command| match command {
            RenderCommand::Fill(fill) => {
                let coeffs = fill.transform.as_coeffs();
                Some((coeffs[4], coeffs[5]))
            }
            _ => None,
        })
        .collect()
}

fn background_fill_count(scene: &Scene) -> usize {
    scene
        .commands
        .iter()
        .filter(|command| matches!(command, RenderCommand::Fill(_)))
        .count()
}

#[test]
fn background_repeat_tiles_cover_area_with_clipping() {
    let decoded = raikiri_traits::DecodedImage {
        width: 2,
        height: 1,
        rgba: vec![255, 0, 0, 255, 0, 255, 0, 255],
    };
    let (position, repeat) = background_repeat_fixture("repeat", "0px 0px");
    let mut scene = Scene::new();
    paint_background_image(
        &mut scene,
        &decoded,
        kurbo::Rect::new(0.0, 0.0, 100.0, 50.0),
        kurbo::Rect::new(0.0, 0.0, 100.0, 50.0),
        20.0,
        10.0,
        &position,
        &repeat,
    );
    // 100 / 20 by 50 / 10 tiles, edge tiles fit exactly here.
    assert_eq!(background_fill_count(&scene), 25);
    let origins = background_fill_origins(&scene);
    assert!(origins.contains(&(0.0, 0.0)));
    assert!(origins.contains(&(80.0, 40.0)));
    // The painting area clip keeps partial edge tiles inside the area.
    let clip = scene
        .commands
        .iter()
        .find_map(|command| match command {
            RenderCommand::PushClipLayer(clip) => Some(clip),
            _ => None,
        })
        .expect("repeat paint must clip to the painting area");
    assert_eq!(clip.transform, Affine::IDENTITY);
    let bounds = kurbo::Shape::bounding_box(&clip.clip);
    assert_eq!(bounds, kurbo::Rect::new(0.0, 0.0, 100.0, 50.0));

    // A partial edge tile still counts: 25px wide area with 20px tiles
    // needs an origin at 20px clipped to 25px.
    let mut clipped = Scene::new();
    paint_background_image(
        &mut clipped,
        &decoded,
        kurbo::Rect::new(0.0, 0.0, 25.0, 12.0),
        kurbo::Rect::new(0.0, 0.0, 25.0, 12.0),
        20.0,
        10.0,
        &position,
        &repeat,
    );
    assert_eq!(background_fill_count(&clipped), 4);
    let origins = background_fill_origins(&clipped);
    assert!(origins.contains(&(20.0, 10.0)));
}

#[test]
fn background_no_repeat_paints_single_origin_tile() {
    let decoded = raikiri_traits::DecodedImage {
        width: 2,
        height: 1,
        rgba: vec![255, 0, 0, 255, 0, 255, 0, 255],
    };
    let (position, repeat) = background_repeat_fixture("no-repeat", "0px 0px");
    let mut scene = Scene::new();
    paint_background_image(
        &mut scene,
        &decoded,
        kurbo::Rect::new(0.0, 0.0, 100.0, 50.0),
        kurbo::Rect::new(0.0, 0.0, 100.0, 50.0),
        20.0,
        10.0,
        &position,
        &repeat,
    );
    assert_eq!(background_fill_count(&scene), 1);
    assert_eq!(background_fill_origins(&scene), vec![(0.0, 0.0)]);

    // The single tile follows background-position.
    let (offset_position, offset_repeat) = background_repeat_fixture("no-repeat", "3px 4px");
    let mut offset = Scene::new();
    paint_background_image(
        &mut offset,
        &decoded,
        kurbo::Rect::new(0.0, 0.0, 100.0, 50.0),
        kurbo::Rect::new(0.0, 0.0, 100.0, 50.0),
        20.0,
        10.0,
        &offset_position,
        &offset_repeat,
    );
    assert_eq!(background_fill_count(&offset), 1);
    assert_eq!(background_fill_origins(&offset), vec![(3.0, 4.0)]);
}

#[test]
fn background_repeat_x_and_y_tile_single_axis() {
    let decoded = raikiri_traits::DecodedImage {
        width: 2,
        height: 1,
        rgba: vec![255, 0, 0, 255, 0, 255, 0, 255],
    };
    let (repeat_x_position, repeat_x) = background_repeat_fixture("repeat-x", "0px 0px");
    let mut scene = Scene::new();
    paint_background_image(
        &mut scene,
        &decoded,
        kurbo::Rect::new(0.0, 0.0, 100.0, 50.0),
        kurbo::Rect::new(0.0, 0.0, 100.0, 50.0),
        20.0,
        10.0,
        &repeat_x_position,
        &repeat_x,
    );
    assert_eq!(background_fill_count(&scene), 5);
    for (_, y) in background_fill_origins(&scene) {
        assert_eq!(y, 0.0);
    }

    let (repeat_y_position, repeat_y) = background_repeat_fixture("repeat-y", "0px 0px");
    let mut scene = Scene::new();
    paint_background_image(
        &mut scene,
        &decoded,
        kurbo::Rect::new(0.0, 0.0, 100.0, 50.0),
        kurbo::Rect::new(0.0, 0.0, 100.0, 50.0),
        20.0,
        10.0,
        &repeat_y_position,
        &repeat_y,
    );
    assert_eq!(background_fill_count(&scene), 5);
    for (x, _) in background_fill_origins(&scene) {
        assert_eq!(x, 0.0);
    }
}

#[test]
fn background_repeat_with_offset_covers_both_directions() {
    let decoded = raikiri_traits::DecodedImage {
        width: 2,
        height: 1,
        rgba: vec![255, 0, 0, 255, 0, 255, 0, 255],
    };
    // Origin at 90px/40px: tiling must extend left and up to cover the area,
    // not only right and down from the origin.
    let (position, repeat) = background_repeat_fixture("repeat", "90px 40px");
    let mut scene = Scene::new();
    paint_background_image(
        &mut scene,
        &decoded,
        kurbo::Rect::new(0.0, 0.0, 100.0, 50.0),
        kurbo::Rect::new(0.0, 0.0, 100.0, 50.0),
        20.0,
        10.0,
        &position,
        &repeat,
    );
    let origins = background_fill_origins(&scene);
    assert_eq!(origins.len(), 30);
    assert!(origins.contains(&(-10.0, 0.0)));
    assert!(origins.contains(&(90.0, 40.0)));
}

#[test]
fn background_space_tiles_without_clipping_and_spaces_gaps() {
    let decoded = raikiri_traits::DecodedImage {
        width: 2,
        height: 1,
        rgba: vec![255, 0, 0, 255, 0, 255, 0, 255],
    };
    // Exact fit stays 5 by 5 like `repeat` here.
    let (position, repeat) = background_repeat_fixture("space", "0px 0px");
    let mut scene = Scene::new();
    paint_background_image(
        &mut scene,
        &decoded,
        kurbo::Rect::new(0.0, 0.0, 100.0, 50.0),
        kurbo::Rect::new(0.0, 0.0, 100.0, 50.0),
        20.0,
        10.0,
        &position,
        &repeat,
    );
    assert_eq!(background_fill_count(&scene), 25);
    let origins = background_fill_origins(&scene);
    assert!(origins.contains(&(0.0, 0.0)));
    assert!(origins.contains(&(80.0, 40.0)));

    // 106px positioning with 32px tiles fits 3 per axis with 5px gaps:
    // 0, 37, 74 pinned to both edges without clipping or scaling.
    let mut spaced = Scene::new();
    paint_background_image(
        &mut spaced,
        &decoded,
        kurbo::Rect::new(0.0, 0.0, 106.0, 106.0),
        kurbo::Rect::new(0.0, 0.0, 106.0, 106.0),
        32.0,
        32.0,
        &position,
        &repeat,
    );
    assert_eq!(background_fill_count(&spaced), 9);
    let origins = background_fill_origins(&spaced);
    assert!(origins.contains(&(0.0, 0.0)));
    assert!(origins.contains(&(37.0, 37.0)));
    assert!(origins.contains(&(74.0, 74.0)));

    // A single fitting tile follows `background-position`.
    let (single_position, single_repeat) = background_repeat_fixture("space", "3px 4px");
    let mut single = Scene::new();
    paint_background_image(
        &mut single,
        &decoded,
        kurbo::Rect::new(0.0, 0.0, 25.0, 12.0),
        kurbo::Rect::new(0.0, 0.0, 25.0, 12.0),
        20.0,
        10.0,
        &single_position,
        &single_repeat,
    );
    assert_eq!(background_fill_count(&single), 1);
    assert_eq!(background_fill_origins(&single), vec![(3.0, 4.0)]);
}

#[test]
fn background_round_rescales_to_whole_tiles() {
    let decoded = raikiri_traits::DecodedImage {
        width: 2,
        height: 1,
        rgba: vec![255, 0, 0, 255, 0, 255, 0, 255],
    };
    let (position, repeat) = background_repeat_fixture("round", "0px 0px");
    // Exact fit keeps the base size.
    let mut exact = Scene::new();
    paint_background_image(
        &mut exact,
        &decoded,
        kurbo::Rect::new(0.0, 0.0, 100.0, 50.0),
        kurbo::Rect::new(0.0, 0.0, 100.0, 50.0),
        20.0,
        10.0,
        &position,
        &repeat,
    );
    assert_eq!(background_fill_count(&exact), 25);

    // 100px positioning with 30px tiles rounds to 3 tiles of 33.333px.
    let mut rescaled = Scene::new();
    paint_background_image(
        &mut rescaled,
        &decoded,
        kurbo::Rect::new(0.0, 0.0, 100.0, 30.0),
        kurbo::Rect::new(0.0, 0.0, 100.0, 30.0),
        30.0,
        30.0,
        &position,
        &repeat,
    );
    assert_eq!(background_fill_count(&rescaled), 3);
    let origins = background_fill_origins(&rescaled);
    assert_eq!(origins.len(), 3);
    assert!((origins[1].0 - 100.0 / 3.0).abs() < 0.001);

    // Mixed axes combine independently (exact fit gives 5 by 5 for both).
    for (style, expected) in [("space round", 25), ("round space", 25)] {
        let (mixed_position, mixed) = background_repeat_fixture(style, "0px 0px");
        let mut mixed_scene = Scene::new();
        paint_background_image(
            &mut mixed_scene,
            &decoded,
            kurbo::Rect::new(0.0, 0.0, 100.0, 50.0),
            kurbo::Rect::new(0.0, 0.0, 100.0, 50.0),
            20.0,
            10.0,
            &mixed_position,
            &mixed,
        );
        assert_eq!(
            background_fill_count(&mixed_scene),
            expected,
            "{style} must tile both axes"
        );
    }
}

#[test]
fn background_repeat_rejects_nonfinite_tile_geometry() {
    let decoded = raikiri_traits::DecodedImage {
        width: 1,
        height: 1,
        rgba: vec![255, 0, 0, 255],
    };
    let (position, repeat) = background_repeat_fixture("repeat", "0px 0px");
    for (image_w, image_h) in [
        (f64::INFINITY, 10.0),
        (20.0, f64::INFINITY),
        (f64::NAN, 10.0),
        (20.0, f64::NAN),
    ] {
        let mut scene = Scene::new();
        paint_background_image(
            &mut scene,
            &decoded,
            kurbo::Rect::new(0.0, 0.0, 100.0, 50.0),
            kurbo::Rect::new(0.0, 0.0, 100.0, 50.0),
            image_w,
            image_h,
            &position,
            &repeat,
        );
        assert_eq!(background_fill_count(&scene), 0);
    }
    // A non-finite painting area cannot place the origin tile.
    let mut scene = Scene::new();
    paint_background_image(
        &mut scene,
        &decoded,
        kurbo::Rect::new(f64::INFINITY, 0.0, f64::INFINITY, 50.0),
        kurbo::Rect::new(f64::INFINITY, 0.0, f64::INFINITY, 50.0),
        20.0,
        10.0,
        &position,
        &repeat,
    );
    assert_eq!(background_fill_count(&scene), 0);
}

#[test]
fn background_repeat_tiles_paint_contiguous_pixels() {
    let decoded = raikiri_traits::DecodedImage {
        width: 1,
        height: 1,
        rgba: vec![255, 0, 0, 255],
    };
    let (repeat_position, repeat) = background_repeat_fixture("repeat", "0px 0px");
    let mut repeat_scene = Scene::new();
    paint_background_image(
        &mut repeat_scene,
        &decoded,
        kurbo::Rect::new(0.0, 0.0, 4.0, 4.0),
        kurbo::Rect::new(0.0, 0.0, 4.0, 4.0),
        2.0,
        2.0,
        &repeat_position,
        &repeat,
    );
    let repeat_rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(repeat_scene, Affine::IDENTITY),
        4,
        4,
    );
    // All four 2px tiles are solid red, including the tile straddling the
    // center and the partial edge coverage.
    for chunk in repeat_rgba.chunks_exact(4) {
        assert_eq!(chunk, &[255, 0, 0, 255]);
    }

    let (single_position, single) = background_repeat_fixture("no-repeat", "0px 0px");
    let mut single_scene = Scene::new();
    paint_background_image(
        &mut single_scene,
        &decoded,
        kurbo::Rect::new(0.0, 0.0, 4.0, 4.0),
        kurbo::Rect::new(0.0, 0.0, 4.0, 4.0),
        2.0,
        2.0,
        &single_position,
        &single,
    );
    let single_rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(single_scene, Affine::IDENTITY),
        4,
        4,
    );
    let pixel =
        |x: u32, y: u32| &single_rgba[((y * 4 + x) * 4) as usize..((y * 4 + x) * 4 + 4) as usize];
    assert_eq!(pixel(0, 0), &[255, 0, 0, 255]);
    assert_eq!(pixel(3, 3), &[0, 0, 0, 0]);
}

#[test]
fn background_origin_positions_independently_from_clip() {
    let decoded = raikiri_traits::DecodedImage {
        width: 2,
        height: 1,
        rgba: vec![255, 0, 0, 255, 0, 255, 0, 255],
    };
    let (position, repeat) = background_repeat_fixture("no-repeat", "0px 0px");
    // Border box 100x50 with 10px borders: padding positioning is inset to
    // (10,10)-(90,40) while painting stays the full border box. The lone tile
    // must sit at the positioning origin, not the painting origin.
    let positioning = kurbo::Rect::new(10.0, 10.0, 90.0, 40.0);
    let painting = kurbo::Rect::new(0.0, 0.0, 100.0, 50.0);
    let mut scene = Scene::new();
    paint_background_image(
        &mut scene,
        &decoded,
        positioning,
        painting,
        20.0,
        10.0,
        &position,
        &repeat,
    );
    assert_eq!(background_fill_count(&scene), 1);
    assert_eq!(background_fill_origins(&scene), vec![(10.0, 10.0)]);

    // Repeating from the same positioning origin must still cover the full
    // painting area, extending into the border ring.
    let (_, repeat_all) = background_repeat_fixture("repeat", "0px 0px");
    let mut tiled = Scene::new();
    paint_background_image(
        &mut tiled,
        &decoded,
        positioning,
        painting,
        20.0,
        10.0,
        &position,
        &repeat_all,
    );
    let origins = background_fill_origins(&tiled);
    assert!(origins.contains(&(10.0, 10.0)));
    assert!(origins.contains(&(-10.0, 0.0)));
    assert!(origins.contains(&(90.0, 40.0)));
    let clip = tiled
        .commands
        .iter()
        .find_map(|command| match command {
            RenderCommand::PushClipLayer(clip) => Some(clip),
            _ => None,
        })
        .expect("origin/clip paint must clip to the painting area");
    assert_eq!(
        kurbo::Shape::bounding_box(&clip.clip),
        kurbo::Rect::new(0.0, 0.0, 100.0, 50.0)
    );
}

#[test]
fn background_origin_and_clip_insets_follow_visual_boxes() {
    let area = kurbo::Rect::new(0.0, 0.0, 100.0, 50.0);
    let border = (10.0, 5.0, 10.0, 5.0);
    let padding = (7.0, 3.0, 7.0, 3.0);
    let mut warnings = Vec::new();
    let url = url::Url::parse("https://example.test/red.png").unwrap();
    let positioning = origin_inset_rect(
        area,
        raikiri_style::property::VisualBox::ContentBox,
        border,
        padding,
        Some(&url),
        &mut warnings,
    );
    assert!(warnings.is_empty());
    assert_eq!(positioning, kurbo::Rect::new(17.0, 8.0, 83.0, 42.0));
    let painting = clip_inset_rect(
        area,
        raikiri_style::property::VisualBox::PaddingBox,
        border,
        padding,
    )
    .expect("padding-box clip must produce a rect");
    assert_eq!(painting, kurbo::Rect::new(10.0, 5.0, 90.0, 45.0));
    assert!(
        clip_inset_rect(
            area,
            raikiri_style::property::VisualBox::Text,
            border,
            padding
        )
        .is_none()
    );
    assert!(
        clip_inset_rect(
            area,
            raikiri_style::property::VisualBox::BorderArea,
            border,
            padding
        )
        .is_none()
    );
}

#[test]
fn background_rounded_corners_clip_url_images() {
    // Square corners skip the extra clip layer.
    assert!(
        rounded_background_path(
            0.0,
            0.0,
            100.0,
            50.0,
            &ComputedBorderRadius::all(ComputedLength(0.0)),
            (0.0, 0.0, 0.0, 0.0),
            (100.0, 50.0),
        )
        .is_none()
    );
    // A 12px radius produces a rounded clip matching the color path.
    let rounded = rounded_background_path(
        0.0,
        0.0,
        100.0,
        50.0,
        &ComputedBorderRadius::all(ComputedLength(12.0)),
        (0.0, 0.0, 0.0, 0.0),
        (100.0, 50.0),
    )
    .expect("nonzero radius must produce a rounded clip");
    let bounds = kurbo::Shape::bounding_box(&rounded);
    for (actual, expected) in [
        (bounds.x0, 0.0),
        (bounds.y0, 0.0),
        (bounds.x1, 100.0),
        (bounds.y1, 50.0),
    ] {
        assert!(
            (actual - expected).abs() < 0.001,
            "rounded clip bounds {bounds:?} must match painting area"
        );
    }
}

#[test]
fn background_clip_text_and_border_area_warn_and_skip_images() {
    struct RedPixels;
    impl raikiri_traits::ImagePixelSource for RedPixels {
        fn get_decoded(
            &self,
            _: &url::Url,
        ) -> Option<std::sync::Arc<raikiri_traits::DecodedImage>> {
            Some(std::sync::Arc::new(raikiri_traits::DecodedImage {
                width: 2,
                height: 2,
                rgba: vec![
                    255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255,
                ],
            }))
        }
    }
    let source = RedPixels;
    let zero_border = resolve_border(
        Border::new(),
        ComputedLength(16.0),
        None,
        &ResolveContext::new(ComputedLength(16.0)),
    );
    let border = raikiri_style::property::Sides {
        top: zero_border,
        right: zero_border,
        bottom: zero_border,
        left: zero_border,
    };
    let padding = taffy::Rect {
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
        left: 0.0,
    };
    let initial = ComputedValues::initial();
    // `text` skips both color and image with a visible warning.
    let mut scene = Scene::new();
    let mut warnings = Vec::new();
    paint_element_background(
        &mut scene,
        100.0,
        50.0,
        0.0,
        0.0,
        CssColor {
            r: 0,
            g: 255,
            b: 0,
            a: 255,
        },
        &BackgroundImage::Url("https://example.test/red.png".into()),
        CssColor::BLACK,
        raikiri_style::property::VisualBox::Text,
        initial.background_origin,
        &ComputedBorderRadius::all(ComputedLength(0.0)),
        &border,
        &padding,
        &initial.background_size,
        &initial.background_position,
        &initial.background_repeat,
        Some(&source),
        &mut warnings,
    );
    assert!(!warnings.is_empty());
    assert_eq!(background_fill_count(&scene), 0);

    // `border-area` keeps the color ring but skips the image with a warning.
    // Nonzero borders establish a real ring; zero borders fall back to border-box.
    let mut authored = Border::new();
    authored.width = Length::Px(10.0);
    authored.style = BorderStyle::Solid;
    authored.color = BorderColor::Resolved(CssColor::BLACK);
    let solid_border = resolve_border(
        authored,
        ComputedLength(16.0),
        None,
        &ResolveContext::new(ComputedLength(16.0)),
    );
    let ring_border = raikiri_style::property::Sides {
        top: solid_border,
        right: solid_border,
        bottom: solid_border,
        left: solid_border,
    };
    let mut ring = Scene::new();
    let mut ring_warnings = Vec::new();
    paint_element_background(
        &mut ring,
        100.0,
        50.0,
        0.0,
        0.0,
        CssColor {
            r: 0,
            g: 255,
            b: 0,
            a: 255,
        },
        &BackgroundImage::Url("https://example.test/red.png".into()),
        CssColor::BLACK,
        raikiri_style::property::VisualBox::BorderArea,
        initial.background_origin,
        &ComputedBorderRadius::all(ComputedLength(0.0)),
        &ring_border,
        &padding,
        &initial.background_size,
        &initial.background_position,
        &initial.background_repeat,
        Some(&source),
        &mut ring_warnings,
    );
    assert!(!ring_warnings.is_empty());
    // Color ring paints but no image tiles in the ring.
    assert!(!ring.commands.is_empty());
    assert!(ring
        .commands
        .iter()
        .all(|command| !matches!(command, RenderCommand::Fill(fill) if matches!(fill.brush, anyrender::Paint::Image(_)))));
}

#[test]
fn background_origin_unsupported_falls_back_with_warning() {
    let area = kurbo::Rect::new(0.0, 0.0, 100.0, 50.0);
    let mut warnings = Vec::new();
    let url = url::Url::parse("https://example.test/red.png").unwrap();
    let positioning = origin_inset_rect(
        area,
        raikiri_style::property::VisualBox::BorderArea,
        (10.0, 10.0, 10.0, 10.0),
        (0.0, 0.0, 0.0, 0.0),
        Some(&url),
        &mut warnings,
    );
    assert!(!warnings.is_empty());
    assert_eq!(positioning, kurbo::Rect::new(10.0, 10.0, 90.0, 40.0));
}

#[test]
fn page_and_canvas_backgrounds_separate_paint_and_position_areas() {
    // `@page` with a 10px border and 5px padding: border-box painting stays
    // full paper while content-box positioning insets by both.
    let area = kurbo::Rect::new(0.0, 0.0, 200.0, 100.0);
    let border = (10.0, 10.0, 10.0, 10.0);
    let padding = (5.0, 5.0, 5.0, 5.0);
    let mut warnings = Vec::new();
    let url = url::Url::parse("https://example.test/red.png").unwrap();
    let positioning = origin_inset_rect(
        area,
        raikiri_style::property::VisualBox::ContentBox,
        border,
        padding,
        Some(&url),
        &mut warnings,
    );
    let painting = clip_inset_rect(
        area,
        raikiri_style::property::VisualBox::BorderBox,
        border,
        padding,
    )
    .expect("border-box painting must exist");
    assert_eq!(positioning, kurbo::Rect::new(15.0, 15.0, 185.0, 85.0));
    assert_eq!(painting, area);

    // Canvas with nonzero page margins keeps the canvas layer inside the page:
    // the existing color test already covers the layer split, and the image
    // path reuses the same `canvas_rect` as its border box.
    let (document, cascade) = canvas_fixture(
        Some("@page { margin: 5px; }"),
        Some("background-color: blue"),
    );
    assert!(!raikiri_dom::page_margins(&cascade, PageBox::A4).is_zero());
    assert_eq!(fill_count(&document, &cascade), 2);
}

#[test]
fn background_space_and_round_paint_spec_correct_pixels() {
    let decoded = raikiri_traits::DecodedImage {
        width: 1,
        height: 1,
        rgba: vec![255, 0, 0, 255],
    };
    // `space` with one 2px tile in a 3px positioning width fits once and
    // follows `background-position`; the gap stays transparent.
    let (space_position, space) = background_repeat_fixture("space", "0px 0px");
    let mut space_scene = Scene::new();
    paint_background_image(
        &mut space_scene,
        &decoded,
        kurbo::Rect::new(0.0, 0.0, 3.0, 2.0),
        kurbo::Rect::new(0.0, 0.0, 3.0, 2.0),
        2.0,
        2.0,
        &space_position,
        &space,
    );
    let space_rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(space_scene, kurbo::Affine::IDENTITY),
        3,
        2,
    );
    let space_pixel = |x: u32, y: u32| {
        let start = ((y * 3 + x) * 4) as usize;
        space_rgba[start..start + 4].to_vec()
    };
    assert_eq!(space_pixel(0, 0), vec![255, 0, 0, 255]);
    assert_eq!(space_pixel(2, 0), vec![0, 0, 0, 0]);

    // `round` rescales a 3px tile to exactly fill 4px, so all pixels are red.
    let (round_position, round) = background_repeat_fixture("round", "0px 0px");
    let mut round_scene = Scene::new();
    paint_background_image(
        &mut round_scene,
        &decoded,
        kurbo::Rect::new(0.0, 0.0, 4.0, 4.0),
        kurbo::Rect::new(0.0, 0.0, 4.0, 4.0),
        3.0,
        3.0,
        &round_position,
        &round,
    );
    let round_rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(round_scene, kurbo::Affine::IDENTITY),
        4,
        4,
    );
    for chunk in round_rgba.chunks_exact(4) {
        assert_eq!(chunk, &[255, 0, 0, 255]);
    }
}

#[test]
fn background_origin_clip_pixel_separation() {
    let decoded = raikiri_traits::DecodedImage {
        width: 1,
        height: 1,
        rgba: vec![255, 0, 0, 255],
    };
    let (position, repeat) = background_repeat_fixture("no-repeat", "0px 0px");
    let mut scene = Scene::new();
    paint_background_image(
        &mut scene,
        &decoded,
        kurbo::Rect::new(10.0, 10.0, 30.0, 30.0),
        kurbo::Rect::new(0.0, 0.0, 40.0, 40.0),
        20.0,
        20.0,
        &position,
        &repeat,
    );
    let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(scene, kurbo::Affine::IDENTITY),
        40,
        40,
    );
    let pixel = |x: u32, y: u32| {
        let start = ((y * 40 + x) * 4) as usize;
        rgba[start..start + 4].to_vec()
    };
    assert_eq!(pixel(0, 0), vec![0, 0, 0, 0]);
    assert_eq!(pixel(10, 10), vec![255, 0, 0, 255]);
    assert_eq!(pixel(29, 29), vec![255, 0, 0, 255]);
    assert_eq!(pixel(30, 30), vec![0, 0, 0, 0]);
}

#[test]
fn background_rounded_clip_pixel_corners() {
    let decoded = raikiri_traits::DecodedImage {
        width: 1,
        height: 1,
        rgba: vec![255, 0, 0, 255],
    };
    let (position, repeat) = background_repeat_fixture("no-repeat", "0px 0px");
    let painting = kurbo::Rect::new(0.0, 0.0, 20.0, 20.0);
    let rounded = rounded_background_path(
        painting.x0,
        painting.y0,
        painting.x1,
        painting.y1,
        &ComputedBorderRadius::all(ComputedLength(8.0)),
        (0.0, 0.0, 0.0, 0.0),
        (20.0, 20.0),
    )
    .expect("rounded clip must exist");
    let mut scene = Scene::new();
    scene.push_clip_layer(kurbo::Affine::IDENTITY, &rounded);
    paint_background_image(
        &mut scene, &decoded, painting, painting, 20.0, 20.0, &position, &repeat,
    );
    scene.pop_layer();
    let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(scene, kurbo::Affine::IDENTITY),
        20,
        20,
    );
    let pixel = |x: u32, y: u32| {
        let start = ((y * 20 + x) * 4) as usize;
        rgba[start..start + 4].to_vec()
    };
    assert_eq!(pixel(10, 10), vec![255, 0, 0, 255]);
    assert_eq!(pixel(0, 0), vec![0, 0, 0, 0]);
}

#[test]
fn page_background_borders_and_padding_inset_paint_and_position() {
    let (document, cascade) = canvas_fixture(
        Some("@page { margin: 5px; border: 4px solid black; padding: 3px; }"),
        None,
    );
    let area = kurbo::Rect::new(0.0, 0.0, 200.0, 100.0);
    let border = page_border_widths(&cascade, area);
    let padding = page_padding_widths(&cascade, area);
    // Nonzero page borders/padding must be visible to background geometry.
    assert!(border.0 > 0.0 && border.1 > 0.0);
    assert!(padding.0 > 0.0 || padding.1 > 0.0);
    let mut warnings = Vec::new();
    let url = url::Url::parse("https://example.test/red.png").unwrap();
    let positioning = origin_inset_rect(
        area,
        raikiri_style::property::VisualBox::ContentBox,
        border,
        padding,
        Some(&url),
        &mut warnings,
    );
    let painting = clip_inset_rect(
        area,
        raikiri_style::property::VisualBox::BorderBox,
        border,
        padding,
    )
    .expect("border-box painting must exist");
    assert!(warnings.is_empty());
    assert!(positioning.x0 > painting.x0);
    assert!(positioning.y0 > painting.y0);
    assert!(positioning.x1 < painting.x1);
    assert!(positioning.y1 < painting.y1);
    let _ = document;
}

#[test]
fn background_page_clip_and_origin_explicit_values() {
    let (_document, cascade) = canvas_fixture(
        Some(
            "@page { background-clip: padding-box; background-origin: content-box; background-image: url('https://example.test/red.png'); border: 4px solid black; padding: 3px; }",
        ),
        None,
    );
    assert_eq!(
        page_background_clip(&cascade),
        raikiri_style::property::VisualBox::PaddingBox
    );
    assert_eq!(
        page_background_origin(&cascade),
        raikiri_style::property::VisualBox::ContentBox
    );
    let area = kurbo::Rect::new(0.0, 0.0, 200.0, 100.0);
    assert_eq!(
        origin_inset_rect(
            area,
            raikiri_style::property::VisualBox::BorderBox,
            (10.0, 10.0, 10.0, 10.0),
            (5.0, 5.0, 5.0, 5.0),
            None,
            &mut Vec::new(),
        ),
        area
    );
    let content_painting = clip_inset_rect(
        area,
        raikiri_style::property::VisualBox::ContentBox,
        (10.0, 10.0, 10.0, 10.0),
        (5.0, 5.0, 5.0, 5.0),
    )
    .expect("content-box clip must produce a rect");
    assert_eq!(content_painting, kurbo::Rect::new(15.0, 15.0, 185.0, 85.0));
}

#[test]
fn background_page_unsupported_clip_warns_and_empty_painting_skips() {
    struct RedPixels;
    impl raikiri_traits::ImagePixelSource for RedPixels {
        fn get_decoded(
            &self,
            _: &url::Url,
        ) -> Option<std::sync::Arc<raikiri_traits::DecodedImage>> {
            Some(std::sync::Arc::new(raikiri_traits::DecodedImage {
                width: 2,
                height: 2,
                rgba: vec![
                    255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255,
                ],
            }))
        }
    }
    let source = RedPixels;
    let (document, cascade) = canvas_fixture(
        Some(
            "@page { background-image: url('https://example.test/red.png'); background-clip: text; }",
        ),
        None,
    );
    let mut scene = Scene::new();
    let mut warnings = Vec::new();
    paint_page_background_image(
        &mut scene,
        &document,
        &cascade,
        kurbo::Rect::new(0.0, 0.0, 100.0, 50.0),
        Some(&source),
        &mut warnings,
    );
    assert!(!warnings.is_empty());
    assert_eq!(background_fill_count(&scene), 0);

    // Empty painting area (tiny area with large borders) skips without warning.
    let (_, empty_cascade) = canvas_fixture(
        Some(
            "@page { background-image: url('https://example.test/red.png'); border: 20px solid black; background-clip: padding-box; }",
        ),
        None,
    );
    let mut empty_scene = Scene::new();
    let mut empty_warnings = Vec::new();
    paint_page_background_image(
        &mut empty_scene,
        &document,
        &empty_cascade,
        kurbo::Rect::new(0.0, 0.0, 10.0, 10.0),
        Some(&source),
        &mut empty_warnings,
    );
    assert_eq!(background_fill_count(&empty_scene), 0);
}

#[test]
fn background_canvas_image_paints_inside_margins_with_origin_clip() {
    struct RedPixels;
    impl raikiri_traits::ImagePixelSource for RedPixels {
        fn get_decoded(
            &self,
            _: &url::Url,
        ) -> Option<std::sync::Arc<raikiri_traits::DecodedImage>> {
            Some(std::sync::Arc::new(raikiri_traits::DecodedImage {
                width: 2,
                height: 2,
                rgba: vec![
                    255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255,
                ],
            }))
        }
    }
    let source = RedPixels;
    let (document, cascade) = canvas_fixture(
        Some("@page { margin: 5px; }"),
        Some(
            "background-image: url('https://example.test/red.png'); border: 4px solid black; padding: 2px;",
        ),
    );
    let area = kurbo::Rect::new(5.0, 5.0, 100.0, 50.0);
    let mut scene = Scene::new();
    let mut warnings = Vec::new();
    paint_canvas_background_image(
        &mut scene,
        &document,
        &cascade,
        area,
        Some(&source),
        &mut warnings,
    );
    assert!(warnings.is_empty());
    assert!(!scene.commands.is_empty());

    // Unsupported canvas clip warns and skips.
    let (_, text_cascade) = canvas_fixture(
        Some("@page { margin: 5px; }"),
        Some("background-image: url('https://example.test/red.png'); background-clip: text;"),
    );
    let mut text_scene = Scene::new();
    let mut text_warnings = Vec::new();
    paint_canvas_background_image(
        &mut text_scene,
        &document,
        &text_cascade,
        area,
        Some(&source),
        &mut text_warnings,
    );
    assert!(!text_warnings.is_empty());
    assert_eq!(background_fill_count(&text_scene), 0);
}

#[test]
fn background_element_origins_clips_and_rounded_images() {
    struct RedPixels;
    impl raikiri_traits::ImagePixelSource for RedPixels {
        fn get_decoded(
            &self,
            _: &url::Url,
        ) -> Option<std::sync::Arc<raikiri_traits::DecodedImage>> {
            Some(std::sync::Arc::new(raikiri_traits::DecodedImage {
                width: 2,
                height: 2,
                rgba: vec![
                    255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255,
                ],
            }))
        }
    }
    let source = RedPixels;
    let initial = ComputedValues::initial();
    let zero_border = resolve_border(
        Border::new(),
        ComputedLength(16.0),
        None,
        &ResolveContext::new(ComputedLength(16.0)),
    );
    let zero_sides = raikiri_style::property::Sides {
        top: zero_border,
        right: zero_border,
        bottom: zero_border,
        left: zero_border,
    };
    let padding = taffy::Rect {
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
        left: 0.0,
    };
    // Border-box origin and content-box clip with padding.
    let mut padded = taffy::Rect {
        top: 5.0,
        right: 5.0,
        bottom: 5.0,
        left: 5.0,
    };
    let _ = &mut padded;
    let content_padding = taffy::Rect {
        top: 5.0,
        right: 5.0,
        bottom: 5.0,
        left: 5.0,
    };
    let (no_repeat_position, no_repeat) = background_repeat_fixture("no-repeat", "0px 0px");
    for (origin, clip) in [
        (
            raikiri_style::property::VisualBox::BorderBox,
            raikiri_style::property::VisualBox::BorderBox,
        ),
        (
            raikiri_style::property::VisualBox::ContentBox,
            raikiri_style::property::VisualBox::ContentBox,
        ),
    ] {
        let mut scene = Scene::new();
        let mut warnings = Vec::new();
        paint_element_background(
            &mut scene,
            100.0,
            50.0,
            0.0,
            0.0,
            CssColor {
                r: 0,
                g: 0,
                b: 0,
                a: 0,
            },
            &BackgroundImage::Url("https://example.test/red.png".into()),
            CssColor::BLACK,
            clip,
            origin,
            &ComputedBorderRadius::all(ComputedLength(0.0)),
            &zero_sides,
            &content_padding,
            &initial.background_size,
            &no_repeat_position,
            &no_repeat,
            Some(&source),
            &mut warnings,
        );
        assert!(warnings.is_empty());
        // Transparent color still emits a fill plus the single image tile.
        assert_eq!(background_fill_count(&scene), 2);
    }

    // Invalid element origin warns and falls back.
    let mut warn_scene = Scene::new();
    let mut warn_warnings = Vec::new();
    paint_element_background(
        &mut warn_scene,
        100.0,
        50.0,
        0.0,
        0.0,
        CssColor {
            r: 0,
            g: 0,
            b: 0,
            a: 0,
        },
        &BackgroundImage::Url("https://example.test/red.png".into()),
        CssColor::BLACK,
        raikiri_style::property::VisualBox::BorderBox,
        raikiri_style::property::VisualBox::Text,
        &ComputedBorderRadius::all(ComputedLength(0.0)),
        &zero_sides,
        &padding,
        &initial.background_size,
        &initial.background_position,
        &initial.background_repeat,
        Some(&source),
        &mut warn_warnings,
    );
    assert!(!warn_warnings.is_empty());

    // Zero-border border-area falls back to border-box and paints the image.
    let mut fallback = Scene::new();
    let mut fallback_warnings = Vec::new();
    paint_element_background(
        &mut fallback,
        100.0,
        50.0,
        0.0,
        0.0,
        CssColor {
            r: 0,
            g: 0,
            b: 0,
            a: 0,
        },
        &BackgroundImage::Url("https://example.test/red.png".into()),
        CssColor::BLACK,
        raikiri_style::property::VisualBox::BorderArea,
        initial.background_origin,
        &ComputedBorderRadius::all(ComputedLength(0.0)),
        &zero_sides,
        &padding,
        &initial.background_size,
        &no_repeat_position,
        &no_repeat,
        Some(&source),
        &mut fallback_warnings,
    );
    assert!(fallback_warnings.is_empty());
    assert_eq!(background_fill_count(&fallback), 2);

    // Rounded element image pushes a rounded clip layer.
    let mut rounded = Scene::new();
    let mut rounded_warnings = Vec::new();
    paint_element_background(
        &mut rounded,
        20.0,
        20.0,
        0.0,
        0.0,
        CssColor {
            r: 0,
            g: 0,
            b: 0,
            a: 0,
        },
        &BackgroundImage::Url("https://example.test/red.png".into()),
        CssColor::BLACK,
        raikiri_style::property::VisualBox::BorderBox,
        raikiri_style::property::VisualBox::BorderBox,
        &ComputedBorderRadius::all(ComputedLength(8.0)),
        &zero_sides,
        &padding,
        &initial.background_size,
        &initial.background_position,
        &initial.background_repeat,
        Some(&source),
        &mut rounded_warnings,
    );
    assert!(rounded_warnings.is_empty());
    assert!(
        rounded
            .commands
            .iter()
            .filter(|c| matches!(c, RenderCommand::PushClipLayer(_)))
            .count()
            >= 2
    );
}

#[test]
fn background_margin_box_origin_clip_and_unsupported_images() {
    let initial = ComputedValues::initial();
    let mut spec = fixed_margin_spec();
    spec.background_image_url = Some("https://example.test/red.png".into());
    spec.background_origin = raikiri_style::property::VisualBox::ContentBox;
    spec.background_clip = raikiri_style::property::VisualBox::PaddingBox;
    spec.border_left = Some((4.0, Color::from_rgba8(0, 0, 0, 255)));
    spec.border_top = Some((4.0, Color::from_rgba8(0, 0, 0, 255)));
    spec.border_right = Some((4.0, Color::from_rgba8(0, 0, 0, 255)));
    spec.border_bottom = Some((4.0, Color::from_rgba8(0, 0, 0, 255)));
    spec.padding = [2.0, 2.0, 2.0, 2.0];
    struct RedPixels;
    impl raikiri_traits::ImagePixelSource for RedPixels {
        fn get_decoded(
            &self,
            _: &url::Url,
        ) -> Option<std::sync::Arc<raikiri_traits::DecodedImage>> {
            Some(std::sync::Arc::new(raikiri_traits::DecodedImage {
                width: 1,
                height: 1,
                rgba: vec![255, 0, 0, 255],
            }))
        }
    }
    let source = RedPixels;
    let mut scene = Scene::new();
    let mut warnings = Vec::new();
    paint_margin_box(
        &mut scene,
        &Document::new(),
        &spec,
        0.0,
        0.0,
        100.0,
        50.0,
        Some(&source),
        &mut warnings,
    );
    assert!(warnings.is_empty());

    let mut unsupported = fixed_margin_spec();
    unsupported.background_image_url = Some("https://example.test/red.png".into());
    unsupported.background_clip = raikiri_style::property::VisualBox::Text;
    let mut unsupported_scene = Scene::new();
    let mut unsupported_warnings = Vec::new();
    paint_margin_box(
        &mut unsupported_scene,
        &Document::new(),
        &unsupported,
        0.0,
        0.0,
        100.0,
        50.0,
        Some(&source),
        &mut unsupported_warnings,
    );
    assert!(!unsupported_warnings.is_empty());
    let _ = initial;
}

#[test]
fn background_tiling_edge_cases_cover_defensive_branches() {
    let decoded = raikiri_traits::DecodedImage {
        width: 1,
        height: 1,
        rgba: vec![255, 0, 0, 255],
    };
    let (position, repeat) = background_repeat_fixture("repeat", "0px 0px");
    // Non-finite positioning with finite painting skips without crashing.
    let mut nonfinite = Scene::new();
    paint_background_image(
        &mut nonfinite,
        &decoded,
        kurbo::Rect::new(f64::INFINITY, 0.0, f64::INFINITY + 100.0, 50.0),
        kurbo::Rect::new(0.0, 0.0, 100.0, 50.0),
        20.0,
        10.0,
        &position,
        &repeat,
    );
    assert_eq!(background_fill_count(&nonfinite), 0);
    // Finite positioning with non-finite painting skips without crashing.
    let mut nonfinite_paint = Scene::new();
    paint_background_image(
        &mut nonfinite_paint,
        &decoded,
        kurbo::Rect::new(0.0, 0.0, 100.0, 50.0),
        kurbo::Rect::new(0.0, 0.0, f64::INFINITY, 50.0),
        20.0,
        10.0,
        &position,
        &repeat,
    );
    assert_eq!(background_fill_count(&nonfinite_paint), 0);
    // Round with zero positioning width falls back to one tile on that axis
    // while the other axis still rounds (50/10 = 5 tiles).
    let (round_position, round) = background_repeat_fixture("round", "0px 0px");
    let mut zero_round = Scene::new();
    paint_background_image(
        &mut zero_round,
        &decoded,
        kurbo::Rect::new(0.0, 0.0, 0.0, 50.0),
        kurbo::Rect::new(0.0, 0.0, 100.0, 50.0),
        20.0,
        10.0,
        &round_position,
        &round,
    );
    assert_eq!(background_fill_count(&zero_round), 5);
    // Space with zero positioning width falls back to a single tile on that axis.
    let (zero_space_position, zero_space) = background_repeat_fixture("space", "0px 0px");
    let mut zero_space_scene = Scene::new();
    paint_background_image(
        &mut zero_space_scene,
        &decoded,
        kurbo::Rect::new(0.0, 0.0, 0.0, 50.0),
        kurbo::Rect::new(0.0, 0.0, 100.0, 50.0),
        20.0,
        10.0,
        &zero_space_position,
        &zero_space,
    );
    assert_eq!(background_fill_count(&zero_space_scene), 5);
    // Tiny tiles hitting the repeat fan-out bound skip without allocating.
    let mut tiny = Scene::new();
    paint_background_image(
        &mut tiny,
        &decoded,
        kurbo::Rect::new(0.0, 0.0, 100.0, 50.0),
        kurbo::Rect::new(0.0, 0.0, 100.0, 50.0),
        0.001,
        0.001,
        &position,
        &repeat,
    );
    assert_eq!(background_fill_count(&tiny), 0);
    // Direct helper edge cases.
    assert!(
        axis_origins(
            f64::INFINITY,
            100.0,
            0.0,
            100.0,
            20.0,
            20.0,
            &position.horizontal,
            &repeat.x,
            None,
        )
        .is_none()
    );
    assert!(
        axis_origins(
            0.0,
            100.0,
            0.0,
            f64::INFINITY,
            20.0,
            20.0,
            &position.horizontal,
            &repeat.x,
            None,
        )
        .is_none()
    );
    let (tile, count) = round_axis_tiles(0.0, 20.0, BackgroundRepeatKeyword::Round);
    assert_eq!((tile, count), (20.0, Some(1)));
}

#[test]
fn background_canvas_percent_padding_and_empty_painting() {
    // Percent padding exercises the percent branch of `canvas_used_padding`.
    let computed = ComputedValues::initial();
    let (left, _, _, _) = canvas_used_padding(&computed, 200.0);
    let _ = left;
    let mut percent_computed = ComputedValues::initial();
    percent_computed.padding.left = ComputedLengthPercentage::Percent(10.0);
    let (pl, _, _, _) = canvas_used_padding(&percent_computed, 200.0);
    assert!((pl - 20.0).abs() < 0.001);

    // Empty canvas painting (tiny area with large borders) skips silently.
    struct RedPixels;
    impl raikiri_traits::ImagePixelSource for RedPixels {
        fn get_decoded(
            &self,
            _: &url::Url,
        ) -> Option<std::sync::Arc<raikiri_traits::DecodedImage>> {
            Some(std::sync::Arc::new(raikiri_traits::DecodedImage {
                width: 2,
                height: 2,
                rgba: vec![
                    255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255,
                ],
            }))
        }
    }
    let source = RedPixels;
    let (document, cascade) = canvas_fixture(
        Some("@page { margin: 5px; }"),
        Some(
            "background-image: url('https://example.test/red.png'); border: 20px solid black; background-clip: padding-box;",
        ),
    );
    let mut scene = Scene::new();
    let mut warnings = Vec::new();
    paint_canvas_background_image(
        &mut scene,
        &document,
        &cascade,
        kurbo::Rect::new(0.0, 0.0, 10.0, 10.0),
        Some(&source),
        &mut warnings,
    );
    assert_eq!(background_fill_count(&scene), 0);
}

#[test]
fn background_space_tiny_tiles_fall_back_to_single_without_allocating() {
    let decoded = raikiri_traits::DecodedImage {
        width: 1,
        height: 1,
        rgba: vec![255, 0, 0, 255],
    };
    let (position, space) = background_repeat_fixture("space", "0px 0px");
    let mut scene = Scene::new();
    paint_background_image(
        &mut scene,
        &decoded,
        kurbo::Rect::new(0.0, 0.0, 100.0, 50.0),
        kurbo::Rect::new(0.0, 0.0, 100.0, 50.0),
        0.001,
        0.001,
        &position,
        &space,
    );
    assert_eq!(background_fill_count(&scene), 1);
}

#[test]
fn overflow_clips_to_padding_edges_inside_the_border() {
    for overflow in ["hidden", "clip", "scroll", "auto"] {
        let scene = transform_markup_scene(&format!(
            "<body style='margin:0'><div style='position:absolute;left:10px;top:20px;width:100px;height:50px;padding:10px;border:5px solid red;overflow:{overflow}'><div style='width:10px;height:10px;background:green'></div></div>",
        ));
        let clips: Vec<_> = scene
            .commands
            .iter()
            .filter_map(|command| match command {
                RenderCommand::PushClipLayer(clip) => Some(kurbo::Shape::bounding_box(&clip.clip)),
                _ => None,
            })
            .collect();
        assert!(
            clips.contains(&Rect::new(15.0, 25.0, 135.0, 95.0)),
            "{overflow}: {clips:?}"
        );
    }
}

#[test]
fn overflow_hidden_preserves_descendant_ink_in_padding() {
    let scene = transform_markup_scene(
        "<body style='margin:0'><div style='position:absolute;left:10px;top:20px;width:100px;height:50px;padding:10px;border:5px solid red;overflow:hidden'><div style='margin-left:-10px;margin-top:-10px;width:120px;height:70px;background:green'></div></div>",
    );
    let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(scene, Affine::IDENTITY),
        160,
        120,
    );
    for (x, y) in [(16, 26), (134, 26), (16, 94), (134, 94)] {
        let offset = (y * 160 + x) * 4;
        assert_eq!(
            &rgba[offset..offset + 4],
            &[0, 128, 0, 255],
            "padding at ({x}, {y})"
        );
    }
    for (x, y) in [(12, 26), (137, 26), (16, 22), (16, 97)] {
        let offset = (y * 160 + x) * 4;
        assert_eq!(
            &rgba[offset..offset + 4],
            &[255, 0, 0, 255],
            "border at ({x}, {y})"
        );
    }
}

#[test]
fn elliptical_overflow_clips_descendants_without_an_own_background() {
    for (radius, inside) in [("30px / 15px", (8, 8)), ("50% / 25%", (25, 8))] {
        let scene = transform_markup_scene(&format!(
            "<body style='margin:0'><div style='position:absolute;left:0;top:0;width:100px;height:50px;overflow:hidden;border-radius:{radius}'><div style='width:100px;height:50px;background:green'></div></div></body>"
        ));
        let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
            |out| out.append_scene(scene, Affine::IDENTITY),
            100,
            50,
        );
        let outside = (2 * 100 + 8) * 4;
        assert_eq!(
            &rgba[outside..outside + 4],
            &[255, 255, 255, 255],
            "{radius}"
        );
        let offset = (inside.1 * 100 + inside.0) * 4;
        assert_eq!(&rgba[offset..offset + 4], &[0, 128, 0, 255], "{radius}");
    }
}

#[test]
fn elliptical_border_ring_has_independent_outer_and_inner_axes() {
    let scene = transform_markup_scene(
        "<body style='margin:0'><div style='position:absolute;left:0;top:0;box-sizing:border-box;width:100px;height:50px;border:4px solid red;border-radius:30px / 15px'></div></body>",
    );
    let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(scene, Affine::IDENTITY),
        100,
        50,
    );
    assert_eq!(
        &rgba[(4 * 100 + 15) * 4..(4 * 100 + 15) * 4 + 4],
        &[255, 0, 0, 255]
    );
    for (x, y) in [(8, 2), (30, 15)] {
        let offset = (y * 100 + x) * 4;
        assert_eq!(&rgba[offset..offset + 4], &[255, 255, 255, 255]);
    }
}

#[test]
fn overflow_hidden_clip_min_edges_floor_outward() {
    // A fractional padding-box origin must not antialias-cut pixel-snapped
    // descendant backgrounds. Min edges floor outward to integer pixels while
    // max edges keep the exact extent for `overflow:hidden`.
    let scene = transform_markup_scene(
        "<body style='margin:0'><div style='position:absolute;left:10.4px;top:20.6px;width:100px;height:50px;overflow:hidden'><div style='width:10px;height:10px;background:green'></div></div>",
    );
    let bounds = scene
        .commands
        .iter()
        .filter_map(|command| match command {
            anyrender::recording::RenderCommand::PushClipLayer(clip) => {
                Some(kurbo::Shape::bounding_box(&clip.clip))
            }
            _ => None,
        })
        .find(|bounds| (bounds.x0 - 10.0).abs() < 1e-5)
        .expect("overflow clip with floored min edge must exist");
    assert!(
        (bounds.x0 - 10.0).abs() < 1e-5,
        "min x floors outward {bounds:?}"
    );
    assert!(
        (bounds.y0 - 20.0).abs() < 1e-5,
        "min y floors outward {bounds:?}"
    );
    assert!(
        (bounds.x1 - 110.4).abs() < 1e-5,
        "max x keeps exact extent {bounds:?}"
    );
    assert!(
        (bounds.y1 - 70.6).abs() < 1e-5,
        "max y keeps exact extent {bounds:?}"
    );
}

fn engine_document() -> Document {
    let mut doc = Document::new();
    let dir = std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../raikiri-dom/tests/data/text-autospace"
    ));
    let collection = raikiri_dom::build_wpt_font_collection(dir).expect("the Ahem layer");
    doc.set_font_collection_with_limits(collection, shodo::limits::Limits::default());
    doc
}

#[test]
fn a_margin_box_width_follows_the_document_font() {
    // "serif" resolves to Ahem in the document layer (30px for "abc" at 10px);
    // the system serif font is nowhere near that.
    let doc = engine_document();
    let mut spec = fixed_margin_spec();
    spec.content = "abc".to_owned();
    spec.text_style.families = vec!["serif".to_owned()];
    spec.text_style.font_size = 10.0;
    assert_eq!(margin_box_text_width(&doc, &spec, None), 30.0);
    assert_eq!(doc.standalone_text_calls(), 1);
    // Vertical content width is the block advance of one column.
    spec.text_style.writing_mode = shodo::geometry::WritingMode::VerticalRl;
    assert_eq!(margin_box_text_width(&doc, &spec, None), 10.0);
    assert_eq!(doc.standalone_text_calls(), 2);
}

struct NoImages;
impl raikiri_traits::ImagePixelSource for NoImages {
    fn get_decoded(&self, _url: &url::Url) -> Option<std::sync::Arc<raikiri_traits::DecodedImage>> {
        None
    }
}

/// `(x, y)` of the first glyph of each margin box text, left to right, and
/// the number of engine results.
fn margin_box_glyph_ys() -> (Vec<(f64, f64)>, usize) {
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let head = document.append_element(Some(html), "head", Style::default(), Some("display:none"));
    let style = document.append_element(Some(head), "style", Style::default(), None::<&str>);
    document.append_text(
        style,
        "@page { margin: 50px; \
           @top-left { content: 'a'; font-family: Ahem; font-size: 10px } \
           @top-right { content: 'a'; font-family: serif; font-size: 10px } }",
    );
    document.append_element(Some(html), "body", Style::default(), Some("display:block"));
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    let dir = std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../raikiri-dom/tests/data/text-autospace"
    ));
    document.set_font_collection(raikiri_dom::build_wpt_font_collection(dir).expect("collection"));
    raikiri_dom::layout_single_page(&mut document, &cascade, PageBox::A4).expect("layout");
    let mut scene = Scene::new();
    crate::paint_single_page_with_images(
        &mut scene,
        &document,
        &cascade,
        PageBox::A4,
        &NoImages,
        &mut raikiri_dom::CounterSnapshotBudget::default(),
    )
    .expect("paint succeeds");
    let mut out = Vec::new();
    for command in &scene.commands {
        if let RenderCommand::GlyphRun(run) = command {
            let origin = run.transform.translation();
            if let Some(glyph) = run.glyphs.first() {
                out.push((origin.x + f64::from(glyph.x), origin.y + f64::from(glyph.y)));
            }
        }
    }
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    (out, document.standalone_text_calls())
}

#[test]
fn engine_margin_boxes_do_not_add_the_ahem_baseline_correction() {
    let (boxes, _) = margin_box_glyph_ys();
    assert_eq!(boxes.len(), 2, "one glyph run per margin box: {boxes:?}");
    // Both boxes have the same height and the same resolved font, so the text
    // sits at the same y whatever the family was called.
    assert_eq!(boxes[0].1, boxes[1].1);
    // The boxes are 10px tall at the top of the page: the baseline is the
    // Ahem ascent, 8.
    assert_eq!(boxes[0].1, 8.0);
}

#[test]
fn margin_boxes_are_measured_and_drawn_by_the_engine() {
    // Two boxes, each measured once for its width (`margin_box_text_width`)
    // and drawn once: four results. The Ahem correction is decided with
    // `standalone_text_eligible`, which shapes nothing.
    let (_, calls) = margin_box_glyph_ys();
    assert_eq!(calls, 4);
}

#[test]
fn a_vertical_writing_margin_box_advances_glyphs_down_the_column() {
    let doc = engine_document();
    let mut spec = fixed_margin_spec();
    spec.content = "abc".to_owned();
    spec.text_style.families = vec!["Ahem".to_owned()];
    spec.text_style.font_size = 10.0;
    let glyphs = |spec: &MarginBoxPaintSpec| -> Vec<(f64, f64)> {
        let mut scene = Scene::new();
        paint_margin_box(
            &mut scene,
            &doc,
            spec,
            0.0,
            0.0,
            200.0,
            40.0,
            None,
            &mut Vec::new(),
        );
        scene
            .commands
            .iter()
            .filter_map(|command| match command {
                RenderCommand::GlyphRun(run) => Some(
                    run.glyphs
                        .iter()
                        .map(|g| (f64::from(g.x), f64::from(g.y)))
                        .collect::<Vec<_>>(),
                ),
                _ => None,
            })
            .flatten()
            .collect()
    };
    let horizontal = glyphs(&spec);
    assert_eq!(doc.standalone_text_calls(), 1);
    spec.text_style.writing_mode = shodo::geometry::WritingMode::VerticalRl;
    let vertical = glyphs(&spec);
    assert_eq!(doc.standalone_text_calls(), 2, "the engine drew it");
    assert!(!vertical.is_empty());
    assert_ne!(vertical, horizontal);
    assert_eq!(vertical.len(), 3);
    assert_eq!(vertical[0].0, vertical[1].0);
    assert_eq!(vertical[1].1 - vertical[0].1, 10.0);
}

#[test]
fn canvas_bitmap_paints_with_object_fit_fill() {
    let mut parsed = raikiri_html::parse(
        "<body style='margin:0'><canvas width='2' height='2' style='width:2px;height:2px'></canvas>".as_bytes(),
        &raikiri_html::ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        },
    )
    .unwrap();
    // Paint a red 2x2 bitmap.
    let canvas = {
        let mut stack = vec![parsed.dom.root_index()];
        let mut found = None;
        while let Some(id) = stack.pop() {
            if parsed.dom.is_canvas_element(id) {
                found = Some(id);
                break;
            }
            if let Some(node) = parsed.dom.get_node(id) {
                stack.extend(node.children.iter().rev().copied());
            }
        }
        found.expect("canvas element exists")
    };
    parsed
        .dom
        .canvas_fill_rect(canvas, 0, 0, 2, 2, [255, 0, 0, 255]);
    let cascade = raikiri_html::build_cascaded(&parsed);
    raikiri_dom::layout_single_page(&mut parsed.dom, &cascade, PageBox::A4).unwrap();
    let mut scene = Scene::new();
    crate::paint_single_page(&mut scene, &parsed.dom, &cascade, PageBox::A4)
        .expect("paint succeeds");
    let image = scene
        .commands
        .iter()
        .find_map(|command| match command {
            RenderCommand::Fill(fill) if matches!(fill.brush, anyrender::Paint::Image(_)) => {
                Some(fill)
            }
            _ => None,
        })
        .expect("canvas paints an image fill");
    // 2 source pixels map to 2 CSS pixels at the origin.
    assert_eq!(&image.transform.as_coeffs()[..4], &[1.0, 0.0, 0.0, 1.0]);
}

#[test]
fn paint_unmaterialized_large_canvas_without_image_pixels() {
    let mut parsed = raikiri_html::parse(
        "<body style='margin:0'><canvas width='3334' height='3000' style='width:2px;height:2px'></canvas>".as_bytes(),
        &raikiri_html::ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        },
    )
    .unwrap();
    let canvas = {
        let mut stack = vec![parsed.dom.root_index()];
        let mut found = None;
        while let Some(id) = stack.pop() {
            if parsed.dom.is_canvas_element(id) {
                found = Some(id);
                break;
            }
            if let Some(node) = parsed.dom.get_node(id) {
                stack.extend(node.children.iter().rev().copied());
            }
        }
        found.expect("canvas element exists")
    };
    assert!(parsed.dom.canvas_bitmap(canvas).is_none());

    let cascade = raikiri_html::build_cascaded(&parsed);
    raikiri_dom::layout_single_page(&mut parsed.dom, &cascade, PageBox::A4).unwrap();
    let mut scene = Scene::new();
    crate::paint_single_page(&mut scene, &parsed.dom, &cascade, PageBox::A4)
        .expect("paint succeeds");

    assert!(scene.commands.iter().all(|command| {
        !matches!(command, RenderCommand::Fill(fill) if matches!(fill.brush, anyrender::Paint::Image(_)))
    }));
}

#[test]
fn canvas_overflow_visible_shows_bitmap_beyond_content_box() {
    // Mirrors overflow-canvas.html: 50x100 bitmap in a 25x50 content box with
    // object-fit none and overflow visible must show the full bitmap.
    let mut parsed = raikiri_html::parse(
        "<body style='margin:0'><canvas width='50' height='100' style='width:25px;height:50px;object-fit:none;object-position:0% 0%;overflow:visible'></canvas>".as_bytes(),
        &raikiri_html::ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        },
    )
    .unwrap();
    let canvas = {
        let mut stack = vec![parsed.dom.root_index()];
        let mut found = None;
        while let Some(id) = stack.pop() {
            if parsed.dom.is_canvas_element(id) {
                found = Some(id);
                break;
            }
            if let Some(node) = parsed.dom.get_node(id) {
                stack.extend(node.children.iter().rev().copied());
            }
        }
        found.expect("canvas element exists")
    };
    parsed
        .dom
        .canvas_fill_rect(canvas, 0, 0, 25, 50, [0, 0, 255, 255]);
    parsed
        .dom
        .canvas_fill_rect(canvas, 25, 0, 25, 50, [0, 128, 0, 255]);
    parsed
        .dom
        .canvas_fill_rect(canvas, 0, 50, 25, 50, [255, 0, 0, 255]);
    parsed
        .dom
        .canvas_fill_rect(canvas, 25, 50, 25, 50, [255, 255, 0, 255]);
    let cascade = raikiri_html::build_cascaded(&parsed);
    raikiri_dom::layout_single_page(&mut parsed.dom, &cascade, PageBox::A4).unwrap();
    let mut scene = Scene::new();
    crate::paint_single_page(&mut scene, &parsed.dom, &cascade, PageBox::A4)
        .expect("paint succeeds");
    let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(scene, Affine::IDENTITY),
        100,
        150,
    );
    let px = |x: usize, y: usize| {
        let off = (y * 100 + x) * 4;
        [rgba[off], rgba[off + 1], rgba[off + 2], rgba[off + 3]]
    };
    // Body has zero margin, so the canvas origin is the viewport origin.
    assert_eq!(px(0, 0), [0, 0, 255, 255], "blue quadrant at origin");
    assert_eq!(
        px(25, 0),
        [0, 128, 0, 255],
        "green overflow beyond 25px content width"
    );
    assert_eq!(
        px(0, 50),
        [255, 0, 0, 255],
        "red overflow beyond 50px content height"
    );
    assert_eq!(px(30, 60), [255, 255, 0, 255], "yellow overflow corner");
}

#[test]
fn canvas_blank_hidden_and_zero_sizes_paint_nothing() {
    // Blank (never painted) canvases stay transparent without raster pixel storage.
    let mut parsed = raikiri_html::parse(
        "<body style='margin:0'><canvas width='2' height='2' style='width:2px;height:2px'></canvas>".as_bytes(),
        &raikiri_html::ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        },
    )
    .unwrap();
    let cascade = raikiri_html::build_cascaded(&parsed);
    raikiri_dom::layout_single_page(&mut parsed.dom, &cascade, PageBox::A4).unwrap();
    let mut scene = Scene::new();
    crate::paint_single_page(&mut scene, &parsed.dom, &cascade, PageBox::A4)
        .expect("paint succeeds");
    assert!(scene.commands.iter().all(|command| !matches!(
        command,
        RenderCommand::Fill(fill) if matches!(fill.brush, anyrender::Paint::Image(_))
    )));
    let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(scene, Affine::IDENTITY),
        10,
        10,
    );
    assert_eq!(&rgba[0..4], &[255, 255, 255, 255]);

    // Hidden canvas paints nothing visible but still counts as handled (no fallback).
    let mut parsed = raikiri_html::parse(
        "<body style='margin:0'><canvas width='2' height='2' style='width:2px;height:2px;visibility:hidden'></canvas>".as_bytes(),
        &raikiri_html::ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        },
    )
    .unwrap();
    let canvas = {
        let mut stack = vec![parsed.dom.root_index()];
        let mut found = None;
        while let Some(id) = stack.pop() {
            if parsed.dom.is_canvas_element(id) {
                found = Some(id);
                break;
            }
            if let Some(node) = parsed.dom.get_node(id) {
                stack.extend(node.children.iter().rev().copied());
            }
        }
        found.expect("canvas")
    };
    parsed
        .dom
        .canvas_fill_rect(canvas, 0, 0, 2, 2, [255, 0, 0, 255]);
    let cascade = raikiri_html::build_cascaded(&parsed);
    raikiri_dom::layout_single_page(&mut parsed.dom, &cascade, PageBox::A4).unwrap();
    let mut scene = Scene::new();
    crate::paint_single_page(&mut scene, &parsed.dom, &cascade, PageBox::A4)
        .expect("paint succeeds");
    let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(scene, Affine::IDENTITY),
        10,
        10,
    );
    assert_eq!(&rgba[0..4], &[255, 255, 255, 255]);
}

#[test]
fn canvas_object_fit_variants_all_paint() {
    for fit in ["fill", "contain", "cover", "scale-down"] {
        let mut parsed = raikiri_html::parse(
            format!("<body style='margin:0'><canvas width='4' height='2' style='width:2px;height:2px;object-fit:{fit}'></canvas>").as_bytes(),
            &raikiri_html::ParseOptions {
                extra_stylesheets: &[],
                network: None,
                base_url: None,
            },
        )
        .unwrap();
        let canvas = {
            let mut stack = vec![parsed.dom.root_index()];
            let mut found = None;
            while let Some(id) = stack.pop() {
                if parsed.dom.is_canvas_element(id) {
                    found = Some(id);
                    break;
                }
                if let Some(node) = parsed.dom.get_node(id) {
                    stack.extend(node.children.iter().rev().copied());
                }
            }
            found.expect("canvas")
        };
        parsed
            .dom
            .canvas_fill_rect(canvas, 0, 0, 4, 2, [255, 0, 0, 255]);
        let cascade = raikiri_html::build_cascaded(&parsed);
        raikiri_dom::layout_single_page(&mut parsed.dom, &cascade, PageBox::A4).unwrap();
        let mut scene = Scene::new();
        crate::paint_single_page(&mut scene, &parsed.dom, &cascade, PageBox::A4)
            .expect("paint succeeds");
        assert!(
            scene.commands.iter().any(|command| matches!(
                command,
                RenderCommand::Fill(fill) if matches!(fill.brush, anyrender::Paint::Image(_))
            )),
            "object-fit:{fit} must paint an image"
        );
    }
}

#[test]
fn canvas_hidden_overflow_clips_to_content_box() {
    let mut parsed = raikiri_html::parse(
        "<body style='margin:0'><canvas width='4' height='4' style='width:2px;height:2px;object-fit:none;object-position:0% 0%;overflow:hidden'></canvas>".as_bytes(),
        &raikiri_html::ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        },
    )
    .unwrap();
    let canvas = {
        let mut stack = vec![parsed.dom.root_index()];
        let mut found = None;
        while let Some(id) = stack.pop() {
            if parsed.dom.is_canvas_element(id) {
                found = Some(id);
                break;
            }
            if let Some(node) = parsed.dom.get_node(id) {
                stack.extend(node.children.iter().rev().copied());
            }
        }
        found.expect("canvas")
    };
    parsed
        .dom
        .canvas_fill_rect(canvas, 0, 0, 4, 4, [255, 0, 0, 255]);
    let cascade = raikiri_html::build_cascaded(&parsed);
    raikiri_dom::layout_single_page(&mut parsed.dom, &cascade, PageBox::A4).unwrap();
    let mut scene = Scene::new();
    crate::paint_single_page(&mut scene, &parsed.dom, &cascade, PageBox::A4)
        .expect("paint succeeds");
    let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(scene, Affine::IDENTITY),
        10,
        10,
    );
    // Content box is 2x2 at origin; overflow hidden clips the 4x4 bitmap.
    assert_eq!(&rgba[0..4], &[255, 0, 0, 255]);
    assert_eq!(&rgba[12..16], &[255, 255, 255, 255]);
}

#[test]
fn canvas_zero_bitmap_size_paints_nothing() {
    let mut parsed = raikiri_html::parse(
        "<body style='margin:0'><canvas width='0' height='0' style='width:2px;height:2px'></canvas>".as_bytes(),
        &raikiri_html::ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        },
    )
    .unwrap();
    let cascade = raikiri_html::build_cascaded(&parsed);
    raikiri_dom::layout_single_page(&mut parsed.dom, &cascade, PageBox::A4).unwrap();
    let mut scene = Scene::new();
    crate::paint_single_page(&mut scene, &parsed.dom, &cascade, PageBox::A4)
        .expect("paint succeeds");
    // Zero-size bitmap paints nothing but does not fall back to image error paths.
    let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(scene, Affine::IDENTITY),
        10,
        10,
    );
    assert_eq!(&rgba[0..4], &[255, 255, 255, 255]);
}

#[test]
fn conic_css_color_to_dynamic_preserves_srgb_channels() {
    let dynamic = css_color_to_dynamic(CssColor {
        r: 255,
        g: 0,
        b: 0,
        a: 255,
    });
    let back = dynamic.to_alpha_color::<peniko::color::Srgb>();
    assert!((back.components[0] - 1.0).abs() < 0.01);
    assert!(back.components[1].abs() < 0.01);
    assert!(back.components[2].abs() < 0.01);
    assert!((back.components[3] - 1.0).abs() < 0.01);
}

#[test]
fn conic_interpolation_tag_covers_all_parser_spaces() {
    use raikiri_style::property::{HueInterpolationMethod, MixColorSpace};
    assert_eq!(
        conic_interpolation_tag(MixColorSpace::Srgb),
        peniko::color::ColorSpaceTag::Srgb
    );
    assert_eq!(
        conic_interpolation_tag(MixColorSpace::SrgbLinear),
        peniko::color::ColorSpaceTag::LinearSrgb
    );
    assert_eq!(
        conic_interpolation_tag(MixColorSpace::Lab),
        peniko::color::ColorSpaceTag::Lab
    );
    assert_eq!(
        conic_interpolation_tag(MixColorSpace::Lch),
        peniko::color::ColorSpaceTag::Lch
    );
    assert_eq!(
        conic_interpolation_tag(MixColorSpace::Oklab),
        peniko::color::ColorSpaceTag::Oklab
    );
    assert_eq!(
        conic_interpolation_tag(MixColorSpace::Oklch),
        peniko::color::ColorSpaceTag::Oklch
    );
    assert_eq!(
        conic_hue_direction(HueInterpolationMethod::Shorter),
        peniko::color::HueDirection::Shorter
    );
    assert_eq!(
        conic_hue_direction(HueInterpolationMethod::Longer),
        peniko::color::HueDirection::Longer
    );
    assert_eq!(
        conic_hue_direction(HueInterpolationMethod::Increasing),
        peniko::color::HueDirection::Increasing
    );
    assert_eq!(
        conic_hue_direction(HueInterpolationMethod::Decreasing),
        peniko::color::HueDirection::Decreasing
    );
}

#[test]
fn conic_angular_offsets_cover_missing_angle_and_percent() {
    use raikiri_style::property::AnglePercentage;
    assert_eq!(angular_stop_offset(None), None);
    assert_eq!(
        angular_stop_offset(Some(AnglePercentage::Percent(25.0))),
        Some(0.25)
    );
    let mut document = Document::new();
    let id = document.append_element(
        Some(document.root_index()),
        "div",
        taffy::Style::default(),
        Some("background-image: conic-gradient(red 45deg, blue)"),
    );
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    let conic = match &cascade.computed[id].background_image {
        raikiri_style::property::BackgroundImage::Gradient(
            raikiri_style::property::Gradient::Conic(g),
        ) => g.clone(),
        other => panic!("expected conic, got {other:?}"),
    };
    let first = angular_stop_offset(conic.stops[0].position);
    assert!(first.is_some_and(|v| (v - 0.125).abs() < 0.001));
}

#[test]
fn conic_fixup_covers_empty_first_last_interior_and_clamp() {
    assert!(fixup_conic_offsets(&[]).is_empty());
    assert_eq!(fixup_conic_offsets(&[None, None]), vec![0.0, 1.0]);
    assert_eq!(fixup_conic_offsets(&[Some(0.2), None]), vec![0.2, 1.0]);
    assert_eq!(fixup_conic_offsets(&[None, Some(0.8)]), vec![0.0, 0.8]);
    assert_eq!(
        fixup_conic_offsets(&[Some(0.0), None, None, Some(1.0)]),
        vec![0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0]
    );
    assert_eq!(fixup_conic_offsets(&[Some(0.8), Some(0.2)]), vec![0.8, 0.8]);
}

#[test]
fn conic_axis_center_covers_start_end_px_and_percent() {
    use raikiri_style::property::{CssPositionOffset, Length};
    assert_eq!(
        conic_axis_center(&CssPositionOffset::Start(Length::Px(10.0)), 0.0, 200.0),
        10.0
    );
    assert_eq!(
        conic_axis_center(&CssPositionOffset::Start(Length::Percent(25.0)), 0.0, 200.0),
        50.0
    );
    assert_eq!(
        conic_axis_center(&CssPositionOffset::End(Length::Px(10.0)), 0.0, 200.0),
        190.0
    );
    assert_eq!(
        conic_axis_center(&CssPositionOffset::End(Length::Percent(25.0)), 0.0, 200.0),
        150.0
    );
}

#[test]
fn conic_center_and_sweep_cover_pinned_quadrants() {
    let mut document = Document::new();
    let center_id = document.append_element(
        Some(document.root_index()),
        "div",
        taffy::Style::default(),
        Some("background-image: conic-gradient(at 25% 25%, red 0 25%, green 25% 50%, blue 50% 75%, black 75% 100%)"),
    );
    let angle_id = document.append_element(
        Some(document.root_index()),
        "div",
        taffy::Style::default(),
        Some("background-image: conic-gradient(from 90deg, red 0 25%, green 25% 50%, blue 50% 75%, black 75% 100%)"),
    );
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    let center_conic = match &cascade.computed[center_id].background_image {
        raikiri_style::property::BackgroundImage::Gradient(
            raikiri_style::property::Gradient::Conic(g),
        ) => g.clone(),
        other => panic!("expected conic, got {other:?}"),
    };
    assert_eq!(center_conic.stops.len(), 8);
    let positioning = kurbo::Rect::new(0.0, 0.0, 200.0, 200.0);
    let center = conic_center(&center_conic.position, positioning);
    assert!((center.x - 50.0).abs() < 0.01);
    assert!((center.y - 50.0).abs() < 0.01);
    let current = CssColor {
        r: 0,
        g: 0,
        b: 0,
        a: 255,
    };
    let gradient = conic_to_peniko(&center_conic, center, current).expect("sweep");
    assert_eq!(gradient.stops.len(), 8);
    let angle_conic = match &cascade.computed[angle_id].background_image {
        raikiri_style::property::BackgroundImage::Gradient(
            raikiri_style::property::Gradient::Conic(g),
        ) => g.clone(),
        other => panic!("expected conic, got {other:?}"),
    };
    let default_center = conic_center(&angle_conic.position, positioning);
    assert!((default_center.x - 100.0).abs() < 0.01);
    assert!((default_center.y - 100.0).abs() < 0.01);
    let angle_gradient =
        conic_to_peniko(&angle_conic, default_center, current).expect("angle sweep");
    assert_eq!(angle_gradient.stops.len(), 8);
}

#[test]
fn conic_paint_covers_empty_square_and_rounded_boxes() {
    let mut document = Document::new();
    let square_id = document.append_element(
        Some(document.root_index()),
        "div",
        taffy::Style::default(),
        Some("background-image: conic-gradient(red 0 25%, green 25% 50%, blue 50% 75%, black 75% 100%)"),
    );
    let round_id = document.append_element(
        Some(document.root_index()),
        "div",
        taffy::Style::default(),
        Some("background-image: conic-gradient(red 0 25%, green 25% 50%, blue 50% 75%, black 75% 100%); border-radius: 10px"),
    );
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    let square = match &cascade.computed[square_id].background_image {
        raikiri_style::property::BackgroundImage::Gradient(
            raikiri_style::property::Gradient::Conic(g),
        ) => g.clone(),
        other => panic!("expected conic, got {other:?}"),
    };
    let round = match &cascade.computed[round_id].background_image {
        raikiri_style::property::BackgroundImage::Gradient(
            raikiri_style::property::Gradient::Conic(g),
        ) => g.clone(),
        other => panic!("expected conic, got {other:?}"),
    };
    let current = CssColor {
        r: 0,
        g: 0,
        b: 0,
        a: 255,
    };
    let mut scene = Scene::new();
    let positioning = kurbo::Rect::new(0.0, 0.0, 200.0, 200.0);
    let painting = kurbo::Rect::new(0.0, 0.0, 200.0, 200.0);
    let square_radius = cascade.computed[square_id].border_radius;
    let round_radius = cascade.computed[round_id].border_radius;
    paint_conic_gradient(
        &mut scene,
        &square,
        positioning,
        painting,
        &square_radius,
        (0.0, 0.0, 0.0, 0.0),
        (200.0, 200.0),
        current,
    );
    assert!(!scene.commands.is_empty());
    let before = scene.commands.len();
    paint_conic_gradient(
        &mut scene,
        &square,
        kurbo::Rect::new(0.0, 0.0, 0.0, 0.0),
        painting,
        &square_radius,
        (0.0, 0.0, 0.0, 0.0),
        (200.0, 200.0),
        current,
    );
    assert_eq!(scene.commands.len(), before);
    paint_conic_gradient(
        &mut scene,
        &round,
        positioning,
        painting,
        &round_radius,
        (0.0, 0.0, 0.0, 0.0),
        (200.0, 200.0),
        current,
    );
    assert!(scene.commands.len() > before);
}

#[test]
fn conic_background_element_paint_covers_opaque_and_transparent_bases() {
    let mut document = Document::new();
    let conic_id = document.append_element(
        Some(document.root_index()),
        "div",
        taffy::Style::default(),
        Some("background-image: conic-gradient(red 0 25%, green 25% 50%, blue 50% 75%, black 75% 100%)"),
    );
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    let bg_image = cascade.computed[conic_id].background_image.clone();
    let initial = ComputedValues::initial();
    let border = raikiri_style::property::Sides {
        top: resolve_border(
            Border::new(),
            ComputedLength(16.0),
            None,
            &ResolveContext::new(ComputedLength(16.0)),
        ),
        right: resolve_border(
            Border::new(),
            ComputedLength(16.0),
            None,
            &ResolveContext::new(ComputedLength(16.0)),
        ),
        bottom: resolve_border(
            Border::new(),
            ComputedLength(16.0),
            None,
            &ResolveContext::new(ComputedLength(16.0)),
        ),
        left: resolve_border(
            Border::new(),
            ComputedLength(16.0),
            None,
            &ResolveContext::new(ComputedLength(16.0)),
        ),
    };
    let padding = taffy::Rect {
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
        left: 0.0,
    };
    let current = CssColor {
        r: 0,
        g: 0,
        b: 0,
        a: 255,
    };
    let mut scene = Scene::new();
    let mut warnings = Vec::new();
    paint_element_background(
        &mut scene,
        200.0,
        200.0,
        0.0,
        0.0,
        CssColor {
            r: 255,
            g: 255,
            b: 255,
            a: 255,
        },
        &bg_image,
        current,
        raikiri_style::property::VisualBox::BorderBox,
        initial.background_origin,
        &ComputedBorderRadius::all(ComputedLength(0.0)),
        &border,
        &padding,
        &initial.background_size,
        &initial.background_position,
        &initial.background_repeat,
        None,
        &mut warnings,
    );
    assert!(!scene.commands.is_empty());
    let mut transparent_scene = Scene::new();
    let mut transparent_warnings = Vec::new();
    paint_element_background(
        &mut transparent_scene,
        200.0,
        200.0,
        0.0,
        0.0,
        CssColor::TRANSPARENT,
        &bg_image,
        current,
        raikiri_style::property::VisualBox::BorderBox,
        initial.background_origin,
        &ComputedBorderRadius::all(ComputedLength(0.0)),
        &border,
        &padding,
        &initial.background_size,
        &initial.background_position,
        &initial.background_repeat,
        None,
        &mut transparent_warnings,
    );
    assert!(!transparent_scene.commands.is_empty());
}

const FONT_DIR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../raikiri-dom/tests/data/text-autospace"
);

/// Lay out and record a document with generated content. The engine's
/// layer holds Ahem.
fn generated_scene(
    css: &str,
    build: impl FnOnce(&mut Document, usize),
) -> (Document, CascadeResult, Scene, usize) {
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let head = document.append_element(Some(html), "head", Style::default(), Some("display:none"));
    let style = document.append_element(Some(head), "style", Style::default(), None::<&str>);
    document.append_text(style, css);
    let body = document.append_element(Some(html), "body", Style::default(), Some("display:block"));
    let div = document.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;font-family:Ahem;font-size:10px;line-height:10px"),
    );
    build(&mut document, div);
    document.mark_in_document_flags();
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    let dir = std::path::Path::new(FONT_DIR);
    document.set_font_collection(raikiri_dom::build_wpt_font_collection(dir).expect("collection"));
    raikiri_dom::layout_single_page(&mut document, &cascade, PageBox::A4).expect("layout");
    let mut scene = Scene::new();
    crate::paint_single_page(&mut scene, &document, &cascade, PageBox::A4).expect("paint succeeds");
    (document, cascade, scene, div)
}

fn glyph_xs(scene: &Scene) -> Vec<f64> {
    let mut xs = Vec::new();
    for command in &scene.commands {
        if let RenderCommand::GlyphRun(run) = command {
            let origin = run.transform.translation();
            xs.extend(run.glyphs.iter().map(|g| origin.x + f64::from(g.x)));
        }
    }
    xs.sort_by(f64::total_cmp);
    xs
}

#[test]
fn the_text_after_a_generated_before_starts_after_its_advance() {
    // "AB " is 30px with its trailing space, so BODY starts at x = 30.
    let (document, _, scene, _) =
        generated_scene(r#"div::before { content: "AB " }"#, |doc, div| {
            doc.append_text(div, "BODY");
        });
    assert_eq!(glyph_xs(&scene), [0.0, 10.0, 20.0, 30.0, 40.0, 50.0, 60.0]);
    // The generated text is laid out in the div's paragraph, not drawn as a
    // separately shaped overlay.
    assert_eq!(document.standalone_text_calls(), 0);
}

#[test]
fn a_generated_run_is_measured_with_the_document_font() {
    let (document, cascade, _, div) = generated_scene(
        // Out-of-flow pseudo-elements stay overlays measured by the painter.
        r#"div::before { content: "A"; position: absolute } div::after { content: "B "; float: left }"#,
        |doc, div| {
            doc.append_text(div, "x");
        },
    );
    let snapshots: Vec<CounterSnapshot> = Vec::new();
    let height = generated_pseudo_text_height(
        &document,
        &cascade,
        div,
        raikiri_style::PseudoElem::Before,
        &snapshots,
    );
    let advance = generated_pseudo_text_advance(
        &document,
        &cascade,
        div,
        raikiri_style::PseudoElem::After,
        &snapshots,
    );
    assert_eq!(height, 10.0);
    assert_eq!(advance, 20.0);
}

fn list_fixture_with_engine() -> (Document, CascadeResult, usize) {
    let (mut document, cascade, first, _) = list_fixture(
        "display:list-item;list-style-type:decimal;list-style-position:outside;\
         font-family:Ahem;font-size:10px",
        "display:list-item",
        None,
    );
    let dir = std::path::Path::new(FONT_DIR);
    let collection = raikiri_dom::build_wpt_font_collection(dir).expect("collection");
    document.set_font_collection_with_limits(collection, shodo::limits::Limits::default());
    (document, cascade, first)
}

#[test]
fn inside_marker_legacy_painter_remains_available_without_an_ifc_owner() {
    let (document, mut cascade, first) = list_fixture_with_engine();
    cascade.computed[first].list_style_position = raikiri_style::ListStylePosition::Inside;
    assert!(!document.get_node(first).unwrap().is_ifc_root());
    let mut scene = Scene::new();
    paint_list_marker(
        &mut scene, &document, &cascade, first, 0.0, 0.0, 200.0, 30.0, 0.0,
    );
    assert_eq!(glyph_xs(&scene), [0.0, 10.0, 20.0]);
}

#[test]
fn inside_ruby_multicol_markers_paint_on_the_legacy_path() {
    struct Pixels;
    impl ImagePixelSource for Pixels {
        fn get_decoded(
            &self,
            _: &url::Url,
        ) -> Option<std::sync::Arc<raikiri_traits::DecodedImage>> {
            Some(std::sync::Arc::new(raikiri_traits::DecodedImage {
                width: 16,
                height: 8,
                rgba: [255, 0, 0, 255].repeat(128),
            }))
        }
        fn intrinsic_size(&self, _: &url::Url) -> Option<raikiri_traits::ImageIntrinsicSize> {
            Some(raikiri_traits::ImageIntrinsicSize {
                width: Some(8.0),
                height: Some(4.0),
                aspect_ratio: Some(2.0),
            })
        }
    }
    for (image, padding, authored_padding) in [
        (false, "4px", 4.0),
        (true, "4px", 4.0),
        (false, "4ch", 40.0),
        (true, "4ch", 40.0),
        (false, "10%", 20.0),
        (true, "10%", 20.0),
    ] {
        let style = if image {
            "display:list-item;list-style:inside url(marker.png);columns:2;height:100px;font-family:Ahem;font-size:10px;padding-left:4px"
        } else {
            "display:list-item;list-style:inside decimal;columns:2;height:100px;font-family:Ahem;font-size:10px;padding-left:4px"
        };
        let style = format!("{style};padding-left:{padding}");
        let (mut doc, _, item, _) = list_fixture(&style, "display:none", None);
        let html = doc.append_element(
            Some(doc.root_index()),
            "html",
            Style::default(),
            None::<&str>,
        );
        let body = doc.append_element(Some(html), "body", Style::default(), Some("display:block"));
        let container = doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("display:block;width:200px"),
        );
        doc.append_child(container, item).unwrap();
        let ruby = doc.append_element(Some(item), "ruby", Style::default(), Some("display:inline"));
        doc.append_element(
            Some(ruby),
            "span",
            Style::default(),
            Some("display:inline-block;width:10px;height:20px"),
        );
        doc.append_element(Some(item), "br", Style::default(), Some("display:inline"));
        doc.mark_in_document_flags();
        let cascade = cascade(&doc, &build_rule_tree(&doc)).unwrap();
        let base = url::Url::parse("https://images.test/doc").unwrap();
        doc.prepare_list_marker_images(&cascade, &Pixels, Some(&base));
        doc.set_font_collection(
            raikiri_dom::build_wpt_font_collection(std::path::Path::new(FONT_DIR)).unwrap(),
        );
        raikiri_dom::layout_single_page(&mut doc, &cascade, PageBox::A4).unwrap();
        assert!(!doc.get_node(item).unwrap().is_ifc_root());
        let expected_advance = if image { 18.0 } else { 30.0 };
        let item_layout = doc.get_node(item).unwrap().unrounded_layout;
        let ruby_layout = doc.get_node(ruby).unwrap().unrounded_layout;
        assert_eq!(
            item_layout.padding.left,
            authored_padding + expected_advance
        );
        assert!(ruby_layout.location.x >= authored_padding + expected_advance);
        assert_eq!(doc.legacy_inside_marker_advance(item), expected_advance);
        let (marker_cv, marker_text) =
            marker_render_info_with_snapshots(&doc, &cascade, item, &[]).unwrap();
        let paint_advance = if image { 8.0 } else { 0.0 }
            + text::measure_margin_text_advance(
                &doc,
                if image { " " } else { &marker_text },
                marker_cv.font_size.px(),
                marker_cv.font_family.first().unwrap().as_str(),
            );
        assert_eq!(doc.legacy_inside_marker_advance(item), paint_advance);
        raikiri_dom::layout_single_page(&mut doc, &cascade, PageBox::A4).unwrap();
        assert_eq!(
            doc.get_node(item).unwrap().unrounded_layout.padding.left,
            item_layout.padding.left
        );
        let mut scene = Scene::new();
        paint_list_marker_with_snapshots(
            &mut scene,
            &doc,
            &cascade,
            item,
            2.0,
            3.0,
            100.0,
            100.0,
            item_layout.padding.left,
            &[],
            Some(&Pixels),
        );
        if image {
            let fill = scene
                .commands
                .iter()
                .find_map(|command| match command {
                    RenderCommand::Fill(fill)
                        if matches!(fill.brush, anyrender::Paint::Image(_)) =>
                    {
                        Some(fill)
                    }
                    _ => None,
                })
                .unwrap();
            assert_eq!(
                fill.transform.as_coeffs(),
                [0.5, 0.0, 0.0, 0.5, f64::from(2.0 + authored_padding), 3.0]
            );
        } else {
            assert_eq!(
                glyph_xs(&scene),
                [
                    f64::from(2.0 + authored_padding),
                    f64::from(12.0 + authored_padding),
                    f64::from(22.0 + authored_padding)
                ]
            );
        }
        let suppressed = if image { "''" } else { "none" };
        doc.set_element_inline_style(item, Some(format!(
            "display:list-item;list-style:inside {suppressed};columns:2;height:100px;font-family:Ahem;font-size:10px;padding-left:{padding}"
        ).into()));
        let updated = raikiri_style::cascade(&doc, &build_rule_tree(&doc)).unwrap();
        doc.prepare_list_marker_images(&updated, &Pixels, Some(&base));
        raikiri_dom::layout_single_page(&mut doc, &updated, PageBox::A4).unwrap();
        assert_eq!(doc.legacy_inside_marker_advance(item), 0.0);
        assert_eq!(
            doc.get_node(item).unwrap().unrounded_layout.padding.left,
            authored_padding
        );
    }
}

#[test]
fn explicit_marker_content_suppresses_legacy_image_paint() {
    struct Pixels;
    impl ImagePixelSource for Pixels {
        fn get_decoded(
            &self,
            _: &url::Url,
        ) -> Option<std::sync::Arc<raikiri_traits::DecodedImage>> {
            Some(std::sync::Arc::new(raikiri_traits::DecodedImage {
                width: 8,
                height: 8,
                rgba: [255, 0, 0, 255].repeat(64),
            }))
        }
    }
    for content in ["none", "'text'"] {
        let sheet = format!("li::marker{{content:{content}}}");
        let (mut doc, cascade, item, _) = list_fixture(
            "display:list-item;list-style:inside url(https://images.test/marker.png)",
            "display:none",
            Some(&sheet),
        );
        doc.set_font_collection(
            raikiri_dom::build_wpt_font_collection(std::path::Path::new(FONT_DIR)).unwrap(),
        );
        let mut scene = Scene::new();
        paint_list_marker_with_snapshots(
            &mut scene,
            &doc,
            &cascade,
            item,
            0.0,
            0.0,
            100.0,
            30.0,
            0.0,
            &[],
            Some(&Pixels),
        );
        assert!(!scene.commands.iter().any(|command| matches!(command,
            RenderCommand::Fill(fill) if matches!(fill.brush, anyrender::Paint::Image(_)))));
        assert_eq!(glyph_xs(&scene).is_empty(), content == "none");
    }
}

#[test]
fn a_list_marker_is_measured_and_drawn_with_the_document_font() {
    // The marker text is "1. " (three glyphs, the last one a space). Its width
    // is 20 (the trailing space is left out of the width), so an outside
    // marker starts at 0 + 0 - 20 - 4 = -24: glyphs at -24, -14 and -4.
    let (document, cascade, first) = list_fixture_with_engine();
    let mut scene = Scene::new();
    paint_list_marker(
        &mut scene, &document, &cascade, first, 0.0, 0.0, 200.0, 30.0, 0.0,
    );
    assert_eq!(glyph_xs(&scene), [-24.0, -14.0, -4.0]);
    // One engine result for the measurement, one for the drawing.
    assert_eq!(document.standalone_text_calls(), 2);
}

#[test]
fn margin_box_flow_inherits_from_root_and_page_and_allows_local_override() {
    use shodo::geometry::{Direction, WritingMode as Mode};
    use shodo::style::TextOrientation as Orientation;
    for (page, local, mode, orientation, direction) in [
        (
            "",
            "",
            Mode::VerticalRl,
            Orientation::Upright,
            Direction::Rtl,
        ),
        (
            "writing-mode:sideways-lr;text-orientation:sideways;direction:ltr;",
            "",
            Mode::SidewaysLr,
            Orientation::Sideways,
            Direction::Ltr,
        ),
        (
            "writing-mode:sideways-lr;",
            "writing-mode:vertical-lr;text-orientation:mixed;direction:ltr;",
            Mode::VerticalLr,
            Orientation::Mixed,
            Direction::Ltr,
        ),
        (
            "writing-mode:vertical-rl;",
            "writing-mode:horizontal-tb;",
            Mode::HorizontalTb,
            Orientation::Upright,
            Direction::Rtl,
        ),
        (
            "",
            "writing-mode:sideways-rl;",
            Mode::SidewaysRl,
            Orientation::Upright,
            Direction::Rtl,
        ),
    ] {
        let mut doc = engine_document();
        let html = doc.append_element(
            Some(0),
            "html",
            Style::default(),
            Some("display:block;writing-mode:vertical-rl;text-orientation:upright;direction:rtl"),
        );
        let node = doc.append_element(Some(html), "style", Style::default(), Some("display:none"));
        doc.append_text(node, format!("@page {{ {page} @top-left {{ content:'ab';font-family:Ahem;font-size:10px;text-align:end; {local} }} }}"));
        let rules = build_rule_tree(&doc);
        let cascade = cascade(&doc, &rules).unwrap();
        let rule = cascade.page.margin_boxes().first().unwrap();
        let spec = margin_box_spec(&doc, &cascade, rule, 100.0, 40.0, 0, 1, false, None).unwrap();
        assert_eq!(spec.text_style.writing_mode, mode, "{page} {local}");
        assert_eq!(spec.text_style.text_orientation, orientation);
        assert_eq!(spec.text_style.direction, direction);
        assert_eq!(spec.alignment, StandaloneAlign::End);
        let mut scene = Scene::new();
        paint_margin_box(
            &mut scene,
            &doc,
            &spec,
            0.0,
            0.0,
            100.0,
            40.0,
            None,
            &mut Vec::new(),
        );
        let positions: Vec<_> = scene
            .commands
            .iter()
            .filter_map(|cmd| match cmd {
                RenderCommand::GlyphRun(run) => Some(run.glyphs.iter().map(|g| (g.x, g.y))),
                _ => None,
            })
            .flatten()
            .collect();
        assert_eq!(positions.len(), 2);
        if mode.is_vertical() {
            assert_eq!(positions[0].0, positions[1].0);
            assert_eq!((positions[1].1 - positions[0].1).abs(), 10.0);
        } else {
            assert_eq!(positions[0].1, positions[1].1);
            assert_eq!((positions[1].0 - positions[0].0).abs(), 10.0);
        }
    }
}

#[test]
fn vertical_margin_box_intrinsics_use_physical_axes() {
    let doc = engine_document();
    let mut spec = fixed_margin_spec();
    spec.content = "abc\ndef".into();
    spec.text_style = crate::standalone_text::style(10.0, "Ahem");
    spec.text_style.writing_mode = shodo::geometry::WritingMode::VerticalRl;
    assert_eq!(margin_box_text_width(&doc, &spec, None), 20.0);
    assert_eq!(margin_box_intrinsic_height(&doc, &spec), 30.0);
}

#[test]
fn vertical_auto_width_counts_columns_wrapped_to_the_content_height() {
    let doc = engine_document();
    let mut spec = fixed_margin_spec();
    spec.content = "ab cd ef".into();
    spec.text_style = crate::standalone_text::style(10.0, "Ahem");
    spec.text_style.writing_mode = shodo::geometry::WritingMode::VerticalRl;
    spec.height = Some(25.0);
    assert_eq!(margin_box_text_width(&doc, &spec, None), 30.0);
}

#[test]
fn vertical_margin_row_distributes_auto_width_using_wrapped_columns() {
    let doc = engine_document();
    let mut long = fixed_margin_spec();
    long.slot = PageMarginBoxSlot::TopLeft;
    long.content = "ab cd ef".into();
    long.text_style = crate::standalone_text::style(10.0, "Ahem");
    long.text_style.writing_mode = shodo::geometry::WritingMode::VerticalRl;
    long.width = None;
    let mut short = long.clone();
    short.slot = PageMarginBoxSlot::TopRight;
    short.content = "ab".into();
    let mut scene = Scene::new();
    paint_horizontal_margin_boxes(
        &mut scene,
        &doc,
        &[long, short],
        true,
        220.0,
        200.0,
        margin_row_margins(25.0),
        None,
        &mut Vec::new(),
    );
    let widths: Vec<_> = scene
        .commands
        .iter()
        .filter_map(|command| match command {
            RenderCommand::Fill(fill) => Some(kurbo::Shape::bounding_box(&fill.shape).width()),
            _ => None,
        })
        .collect();
    assert_eq!(widths, [150.0, 50.0]);
}

fn fragmented_flex_float_paint_fixture() -> (Document, CascadeResult, Scene, usize) {
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = document.append_element(Some(html), "body", Style::default(), Some("display:block"));
    let multicol = document.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;width:300px;columns:100px auto;max-height:160px;border:3px solid pink"),
    );
    let flex = document.append_element(
        Some(multicol),
        "div",
        Style::default(),
        Some("display:flex"),
    );
    let flex_item = document.append_element(
        Some(flex),
        "div",
        Style::default(),
        Some("border:4px solid teal;outline:4px solid blue"),
    );
    document.append_element(
        Some(flex_item),
        "div",
        Style::default(),
        Some("float:left;border:3px solid black;height:500px;width:100px;background:yellow"),
    );
    document.append_element(Some(flex_item), "br", Style::default(), None::<&str>);
    let second_float = document.append_element(
        Some(flex_item),
        "div",
        Style::default(),
        Some("float:left;background:cyan;width:100px"),
    );
    document.append_element(
        Some(second_float),
        "div",
        Style::default(),
        Some("display:inline-block;width:30px;height:30px;background:purple"),
    );
    document.mark_in_document_flags();
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    let mut page = raikiri_traits::PageBox::new();
    page.width = 800.0;
    page.height = 600.0;
    raikiri_dom::layout_single_page(&mut document, &cascade, page).expect("layout Ok");
    let mut scene = Scene::new();
    crate::paint_single_page(&mut scene, &document, &cascade, page).expect("paint succeeds");
    (document, cascade, scene, second_float)
}

#[test]
fn paint_document_uses_each_nested_float_fragment_once() {
    let (_, _, scene, _) = fragmented_flex_float_paint_fixture();
    let cyan = anyrender::Paint::Solid(peniko::Color::from_rgba8(0, 255, 255, 255));
    let cyan_fills: Vec<_> = scene
        .commands
        .iter()
        .enumerate()
        .filter_map(|(index, command)| match command {
            RenderCommand::Fill(fill) if fill.brush == cyan => Some((index, fill)),
            _ => None,
        })
        .collect();
    assert_eq!(cyan_fills.len(), 1, "the float background paints once");
    let (fill_index, fill) = cyan_fills[0];
    let fill_bounds = kurbo::Shape::bounding_box(&fill.shape);
    let fill_origin = fill.transform * fill_bounds.origin();
    assert!(
        fill_origin.x > 150.0,
        "the cyan float should be painted in column two, at x={}, not its unfragmented location",
        fill_origin.x
    );
    assert!(
        fill_origin.y < 200.0,
        "the stale y=510 placement must not be painted: y={}",
        fill_origin.y
    );

    let column_clip = scene.commands[..fill_index]
        .iter()
        .rev()
        .find_map(|command| match command {
            RenderCommand::PushClipLayer(clip) => {
                let bounds = kurbo::Shape::bounding_box(&clip.clip);
                ((bounds.width() - 142.0).abs() < 0.01 && (bounds.height() - 160.0).abs() < 0.01)
                    .then_some((clip, bounds))
            }
            _ => None,
        })
        .expect("the second column clips the float fragment");
    let clip_origin = column_clip.0.transform * column_clip.1.origin();
    assert!(clip_origin.x <= fill_origin.x);
    assert!(clip_origin.x + 142.0 >= fill_origin.x + fill_bounds.width());
}

#[test]
fn generated_flow_height_skips_a_node_already_queued_from_a_duplicate_child() {
    let mut document = Document::new();
    let parent = document.append_element(Some(0), "div", Style::default(), Some("display:block"));
    let child =
        document.append_element(Some(parent), "div", Style::default(), Some("display:block"));
    document.attach_child(parent, child);

    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    let mut cache = std::collections::HashMap::new();

    assert_eq!(
        generated_flow_height(&document, &cascade, parent, &[], &mut cache),
        0.0
    );
}

#[test]
fn generated_flow_height_caches_zero_for_an_unknown_node_id() {
    let document = Document::new();
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    let mut cache = std::collections::HashMap::new();

    assert_eq!(
        generated_flow_height(&document, &cascade, usize::MAX, &[], &mut cache),
        0.0
    );
    assert_eq!(cache.get(&usize::MAX), Some(&0.0));
}

#[test]
fn margin_box_layer_precedence_reaches_the_paint_consumer() {
    for (sheets, expected) in [
        (
            [
                "@layer a,b; @layer b{@page{@top-left{color:blue}}}",
                "@layer a{@page{@top-left{color:red}}}",
            ],
            raikiri_style::CssColor {
                r: 0,
                g: 0,
                b: 255,
                a: 255,
            },
        ),
        (
            [
                "@layer a,b; @layer a{@page{@top-left{color:red!important}}}",
                "@layer b{@page{@top-left{color:blue!important}}} @page{@top-left{color:green!important}}",
            ],
            raikiri_style::CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255,
            },
        ),
    ] {
        let mut tree = raikiri_style::RuleTree::empty();
        for sheet in sheets {
            tree.add_stylesheet(sheet, raikiri_style::Origin::Author);
        }
        let page = raikiri_style::cascade_page(
            &tree,
            &raikiri_style::PageContextQuery::default(),
            raikiri_style::PageInheritance::LegacyInitialValues,
        );
        let rule = margin_box_rule(&page, PageMarginBoxSlot::TopLeft).unwrap();
        assert_eq!(
            margin_box_property(&rule, PropertyKey::Color),
            Some(&PropertyValue::Color(expected))
        );
    }
}

mod css_wide_margin_tests;

#[test]
fn large_inset_corner_background_is_cropped_not_rescaled() {
    for radius in ["100px 0 0 0", "100px 0 0 0 / 80px 0 0 0"] {
        let scene = transform_markup_scene(&format!(
            "<body style='margin:0'><div style='position:absolute;left:0;top:0;box-sizing:border-box;width:100px;height:100px;padding:20px;background:red;background-clip:content-box;border-radius:{radius}'></div></body>"
        ));
        let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
            |out| out.append_scene(scene, Affine::IDENTITY),
            100,
            100,
        );
        let offset = (21 * 100 + 85) * 4;
        assert_eq!(
            &rgba[offset..offset + 4],
            &[255, 255, 255, 255],
            "{radius}: outside content edge"
        );
        let inside = (70 * 100 + 70) * 4;
        assert_eq!(
            &rgba[inside..inside + 4],
            &[255, 0, 0, 255],
            "{radius}: missing inside curve"
        );
        if radius == "100px 0 0 0" {
            let outside_curve = (30 * 100 + 50) * 4;
            assert_eq!(
                &rgba[outside_curve..outside_curve + 4],
                &[255, 255, 255, 255],
                "curve was rescaled instead of cropped"
            );
        }
    }
}

#[test]
fn large_inset_corner_border_has_no_hole_outside_padding_edge() {
    let scene = transform_markup_scene(
        "<body style='margin:0'><div style='position:absolute;left:0;top:0;box-sizing:border-box;width:100px;height:100px;border:20px solid red;border-radius:100px 0 0 0'></div></body>",
    );
    let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(scene, Affine::IDENTITY),
        100,
        100,
    );
    let border = (21 * 100 + 85) * 4;
    assert_eq!(&rgba[border..border + 4], &[255, 0, 0, 255]);
    let padding = (70 * 100 + 70) * 4;
    assert_eq!(&rgba[padding..padding + 4], &[255, 255, 255, 255]);
}

#[test]
fn one_visible_overflow_axis_keeps_corner_content() {
    for axes in [
        "overflow-x:clip;overflow-y:visible",
        "overflow-x:visible;overflow-y:clip",
    ] {
        let scene = transform_markup_scene(&format!(
            "<body style='margin:0'><div style='position:absolute;left:0;top:0;width:100px;height:50px;{axes};border-radius:30px / 15px'><div style='width:100px;height:50px;background:green'></div></div></body>"
        ));
        let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
            |out| out.append_scene(scene, Affine::IDENTITY),
            100,
            50,
        );
        let corner = (2 * 100 + 8) * 4;
        assert_eq!(&rgba[corner..corner + 4], &[0, 128, 0, 255], "{axes}");
    }
}

#[test]
fn cropped_corner_paths_stay_inside_each_inner_edge() {
    use kurbo::Shape;
    for vertical_radius in [80.0, 100.0] {
        for corner in 0..4 {
            let mut horizontal = [ComputedLengthPercentage::Px(0.0); 4];
            let mut vertical = horizontal;
            horizontal[corner] = ComputedLengthPercentage::Px(100.0);
            vertical[corner] = ComputedLengthPercentage::Px(vertical_radius);
            let radius = ComputedBorderRadius::elliptical(horizontal, vertical);
            let path = rounded_background_path(
                20.0,
                20.0,
                80.0,
                80.0,
                &radius,
                (20.0, 20.0, 20.0, 20.0),
                (100.0, 100.0),
            )
            .unwrap();
            let bounds = path.bounding_box();
            assert!(
                bounds.x0 >= 20.0 - 1e-8
                    && bounds.y0 >= 20.0 - 1e-8
                    && bounds.x1 <= 80.0 + 1e-8
                    && bounds.y1 <= 80.0 + 1e-8,
                "corner {corner}: {bounds:?}"
            );
            assert_ne!(path.winding(Point::new(50.0, 50.0)), 0);
        }
    }
    let radius = ComputedBorderRadius::elliptical(
        [
            ComputedLengthPercentage::Px(100.0),
            ComputedLengthPercentage::Px(0.0),
            ComputedLengthPercentage::Px(0.0),
            ComputedLengthPercentage::Px(0.0),
        ],
        [
            ComputedLengthPercentage::Px(100.0),
            ComputedLengthPercentage::Px(0.0),
            ComputedLengthPercentage::Px(0.0),
            ComputedLengthPercentage::Px(0.0),
        ],
    );
    let empty = rounded_background_path(
        60.0,
        60.0,
        70.0,
        70.0,
        &radius,
        (60.0, 60.0, 30.0, 30.0),
        (100.0, 100.0),
    )
    .unwrap();
    assert!(empty.elements().is_empty());
    assert!(
        rounded_rect_path(0.0, 0.0, 0.0, 10.0, [[5.0, 5.0]; 4])
            .elements()
            .is_empty()
    );
}

#[test]
fn crossing_inner_corner_paths_keep_only_the_common_region() {
    use kurbo::Shape;
    for (pair, outside) in [
        ([0, 2], Point::new(78.0, 21.0)),
        ([1, 3], Point::new(21.0, 21.0)),
    ] {
        let mut values = [ComputedLengthPercentage::Px(0.0); 4];
        for corner in pair {
            values[corner] = ComputedLengthPercentage::Px(100.0);
        }
        let radius = ComputedBorderRadius::elliptical(values, values);
        let path = rounded_background_path(
            20.0,
            20.0,
            80.0,
            80.0,
            &radius,
            (20.0, 20.0, 20.0, 20.0),
            (100.0, 100.0),
        )
        .unwrap();
        assert_eq!(path.winding(outside), 0);
        assert_ne!(path.winding(Point::new(50.0, 50.0)), 0);
        let empty = rounded_background_path(
            40.0,
            40.0,
            60.0,
            60.0,
            &radius,
            (40.0, 40.0, 40.0, 40.0),
            (100.0, 100.0),
        )
        .unwrap();
        assert!(empty.elements().is_empty());
    }
}

const CROSSING_RADII: [&str; 2] = [
    "border-top-left-radius:100px;border-bottom-right-radius:100px",
    "border-top-right-radius:100px;border-bottom-left-radius:100px",
];

#[test]
fn large_crossing_inner_ellipses_stay_inside_the_outer_outline() {
    use kurbo::{PathEl, Shape};
    for (pair, outside) in [
        ([0, 2], Point::new(567.5, 6676.5)),
        ([1, 3], Point::new(9432.5, 6676.5)),
    ] {
        let mut radii = [[0.0, 0.0]; 4];
        for index in pair {
            radii[index] = [10000.0, 10000.0];
        }
        let outer = rounded_rect_path(0.0, 0.0, 10000.0, 10000.0, radii);
        assert_eq!(outer.winding(outside), 0);
        let inner = rounded_rect_path(
            1.0,
            1.0,
            9999.0,
            9999.0,
            inset_border_radii(radii, (1.0, 1.0, 1.0, 1.0)),
        );
        assert_eq!(inner.winding(outside), 0);
        assert_ne!(inner.winding(Point::new(5000.0, 5000.0)), 0);
        for element in inner.elements() {
            if let PathEl::MoveTo(point) | PathEl::LineTo(point) = element {
                assert_ne!(outer.winding(*point), 0);
            }
        }
    }
}

#[test]
fn large_crossing_ellipse_border_does_not_paint_outside_its_box_shape() {
    for (radii, left) in [
        (
            "border-top-left-radius:10000px;border-bottom-right-radius:10000px",
            -520,
        ),
        (
            "border-top-right-radius:10000px;border-bottom-left-radius:10000px",
            -9385,
        ),
    ] {
        let scene = transform_markup_scene(&format!(
            "<body style='margin:0'><div style='position:absolute;left:{left}px;top:-6640px;box-sizing:border-box;width:10000px;height:10000px;border:1px solid red;{radii}'></div></body>"
        ));
        let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
            |out| out.append_scene(scene, Affine::IDENTITY),
            100,
            100,
        );
        let offset = (36 * 100 + 47) * 4;
        assert_eq!(&rgba[offset..offset + 4], &[255, 255, 255, 255]);
        assert!(
            rgba.chunks_exact(4)
                .any(|pixel| u16::from(pixel[0]) > u16::from(pixel[1]) + 30)
        );
    }
}

fn crossing_corner_pixels(style: &str, content: &str) -> Vec<u8> {
    let scene = transform_markup_scene(&format!(
        "<body style='margin:0'><div style='position:absolute;left:0;top:0;box-sizing:border-box;width:100px;height:100px;{style}'>{content}</div></body>"
    ));
    anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(scene, Affine::IDENTITY),
        100,
        100,
    )
}

#[test]
fn crossing_inner_corner_backgrounds_do_not_paint_reversed_islands() {
    for (radii, outside) in CROSSING_RADII.into_iter().zip([(78, 21), (21, 21)]) {
        let partial = crossing_corner_pixels(
            &format!(
                "{radii};border:20px solid transparent;background:red;background-clip:padding-box"
            ),
            "",
        );
        let offset = (outside.1 * 100 + outside.0) * 4;
        assert_eq!(&partial[offset..offset + 4], &[255, 255, 255, 255]);
        let center = (50 * 100 + 50) * 4;
        assert_eq!(&partial[center..center + 4], &[255, 0, 0, 255]);
        let empty = crossing_corner_pixels(
            &format!(
                "{radii};border:40px solid transparent;background:red;background-clip:padding-box"
            ),
            "",
        );
        assert_eq!(&empty[center..center + 4], &[255, 255, 255, 255]);
    }
}

#[test]
fn crossing_inner_corner_empty_ring_retains_the_outer_border() {
    for radii in CROSSING_RADII {
        let rgba = crossing_corner_pixels(
            &format!("{radii};border:40px solid red;background:blue;background-clip:padding-box"),
            "",
        );
        let center = (50 * 100 + 50) * 4;
        assert_eq!(&rgba[center..center + 4], &[255, 0, 0, 255]);
    }
}

#[test]
fn crossing_inner_corner_empty_overflow_clip_hides_descendants() {
    for radii in CROSSING_RADII {
        let rgba = crossing_corner_pixels(
            &format!("{radii};border:40px solid transparent;overflow:hidden"),
            "<div style='width:100px;height:100px;background:red'></div>",
        );
        let center = (50 * 100 + 50) * 4;
        assert_eq!(&rgba[center..center + 4], &[255, 255, 255, 255]);
    }
}

#[test]
fn corner_arc_flattening_preserves_endpoints_with_bounded_finite_output() {
    let collapsed = Point::new(10.0, 20.0);
    let mut points = vec![collapsed];
    flatten_corner_arc(
        Arc::new(collapsed, Vec2::ZERO, 0.0, FRAC_PI_2, 0.0),
        FRAC_PI_2,
        0,
        &mut points,
    );
    assert!(points.iter().all(|point| *point == collapsed));
    let extent = 1e18;
    let arc = Arc::new(
        Point::ORIGIN,
        Vec2::new(extent, extent),
        0.0,
        FRAC_PI_2,
        0.0,
    );
    let mut points = vec![Point::new(extent, 0.0)];
    flatten_corner_arc(arc, FRAC_PI_2, 0, &mut points);
    assert!(points.len() <= 1025);
    assert_eq!(points.first(), Some(&Point::new(extent, 0.0)));
    let (sin, cos) = FRAC_PI_2.sin_cos();
    assert_eq!(points.last(), Some(&Point::new(extent * cos, extent * sin)));
    assert!(points.iter().all(|point| point.x.is_finite()
        && point.y.is_finite()
        && (0.0..=extent).contains(&point.x)
        && (0.0..=extent).contains(&point.y)));
}

#[test]
fn review_positioned_subtree_is_not_replayed_with_each_plain_wrapper_fragment() {
    let scene = transform_markup_scene(
        "<!DOCTYPE html><body style='margin:0;background:white'><div style='columns:2;column-fill:auto;gap:0;width:100px;height:100px'><div style='position:relative'><div style='height:75px;break-before:avoid'></div><div style='height:75px'></div><div style='position:absolute;left:0;top:0;width:10px;height:10px;background:red'></div></div></div></body>",
    );
    let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |renderer| renderer.append_scene(scene, Affine::IDENTITY),
        800,
        600,
    );
    let pixel = |x: usize, y: usize| &rgba[(y * 800 + x) * 4..(y * 800 + x) * 4 + 4];
    assert_eq!(pixel(5, 5), &[255, 0, 0, 255]);
    assert_eq!(pixel(55, 5), &[255, 255, 255, 255]);
}

#[test]
fn review_float_overflow_does_not_extend_the_plain_wrapper_background() {
    let scene = transform_markup_scene(
        "<!DOCTYPE html><body style='margin:0;background:white'><div style='columns:2;column-fill:auto;gap:0;width:100px;height:100px'><div style='background:red'><div style='float:left;width:10px;height:80px'></div><div style='height:20px;break-before:avoid'></div></div></div></body>",
    );
    let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |renderer| renderer.append_scene(scene, Affine::IDENTITY),
        800,
        600,
    );
    let pixel = |x: usize, y: usize| &rgba[(y * 800 + x) * 4..(y * 800 + x) * 4 + 4];
    assert_eq!(pixel(40, 10), &[255, 0, 0, 255]);
    assert_eq!(pixel(40, 50), &[255, 255, 255, 255]);
}

#[test]
fn review_replaced_canvas_paints_its_monolithic_content_once() {
    let mut parsed = raikiri_html::parse(
        "<!DOCTYPE html><body style='margin:0;background:white'><div style='columns:2;column-fill:auto;gap:0;width:100px;height:100px'><div style='height:30px;break-before:avoid'></div><canvas id='canvas' width='20' height='150' style='display:block;width:20px;height:150px'></canvas></div></body>".as_bytes(),
        &raikiri_html::ParseOptions { extra_stylesheets: &[], network: None, base_url: None },
    ).unwrap();
    let canvas = (0..parsed.dom.node_count())
        .find(|&id| parsed.dom.is_canvas_element(id))
        .unwrap();
    parsed
        .dom
        .canvas_fill_rect(canvas, 0, 0, 20, 75, [255, 0, 0, 255]);
    parsed
        .dom
        .canvas_fill_rect(canvas, 0, 75, 20, 75, [0, 128, 0, 255]);
    let cascade = raikiri_html::build_cascaded(&parsed);
    raikiri_dom::layout_single_page(&mut parsed.dom, &cascade, PageBox::A4).unwrap();
    let mut scene = Scene::new();
    crate::paint_single_page(&mut scene, &parsed.dom, &cascade, PageBox::A4).unwrap();
    let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |renderer| renderer.append_scene(scene, Affine::IDENTITY),
        800,
        600,
    );
    let pixel = |x: usize, y: usize| &rgba[(y * 800 + x) * 4..(y * 800 + x) * 4 + 4];
    assert_eq!(pixel(10, 50), &[255, 255, 255, 255]);
    assert_eq!(pixel(55, 10), &[255, 0, 0, 255]);
    assert_eq!(pixel(55, 100), &[0, 128, 0, 255]);
    assert_eq!(pixel(105, 10), &[255, 255, 255, 255]);
}

#[test]
fn review_rtl_column_colors_follow_the_containers_inline_direction() {
    let scene = transform_markup_scene(
        "<!DOCTYPE html><body style='margin:0;background:white'><div style='direction:rtl;columns:2;column-fill:auto;gap:0;width:100px;height:100px'><div style='height:60px;background:red'></div><div style='height:60px;break-before:avoid;background:green'></div></div></body>",
    );
    let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |renderer| renderer.append_scene(scene, Affine::IDENTITY),
        800,
        600,
    );
    let pixel = |x: usize, y: usize| &rgba[(y * 800 + x) * 4..(y * 800 + x) * 4 + 4];
    assert_eq!(pixel(60, 10), &[255, 0, 0, 255]);
    assert_eq!(pixel(10, 10), &[0, 128, 0, 255]);
}

#[test]
fn review_zero_height_forced_boundary_paints_only_in_the_next_column() {
    let scene = transform_markup_scene(
        "<!DOCTYPE html><body style='margin:0;background:white'><div style='columns:2;column-fill:auto;gap:0;width:100px;height:100px'><div style='height:0;break-after:column'></div><div style='height:20px;background:green'></div></div></body>",
    );
    let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |renderer| renderer.append_scene(scene, Affine::IDENTITY),
        800,
        600,
    );
    let pixel = |x: usize, y: usize| &rgba[(y * 800 + x) * 4..(y * 800 + x) * 4 + 4];
    assert_eq!(pixel(10, 10), &[255, 255, 255, 255]);
    assert_eq!(pixel(60, 10), &[0, 128, 0, 255]);
}

#[test]
fn review_fragmented_list_item_paints_its_marker_on_the_first_fragment_only() {
    let scene = transform_markup_scene(
        "<!DOCTYPE html><body style='margin:0;background:white'><div style='margin-left:40px;columns:2;column-fill:auto;gap:0;width:100px;height:100px'><div style='display:list-item;list-style:disc outside'><div style='height:60px'></div><div style='height:60px;break-before:avoid'></div></div></div></body>",
    );
    let marker_runs = scene
        .commands
        .iter()
        .filter(|c| matches!(c, RenderCommand::GlyphRun(_)))
        .count();
    let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |renderer| renderer.append_scene(scene, Affine::IDENTITY),
        800,
        600,
    );
    let ink = |start: usize, end: usize| {
        (0..30)
            .flat_map(|y| (start..end).map(move |x| (y * 800 + x) * 4))
            .filter(|&i| rgba[i..i + 4] != [255, 255, 255, 255])
            .count()
    };
    assert!(ink(15, 40) > 0);
    assert_eq!(ink(65, 90), 0);
    assert_eq!(marker_runs, 1);
}

#[test]
fn review_text_bearing_box_background_follows_its_forced_column_edge() {
    let scene = transform_markup_scene(
        "<!DOCTYPE html><body style='margin:0;background:white'><div style='columns:2;column-fill:auto;gap:0;width:100px;height:100px'><div style='height:20px'></div><div style='height:40px;break-before:column;background:green;color:white'>X</div></div></body>",
    );
    let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |renderer| renderer.append_scene(scene, Affine::IDENTITY),
        800,
        600,
    );
    let pixel = |x: usize, y: usize| &rgba[(y * 800 + x) * 4..(y * 800 + x) * 4 + 4];
    assert_eq!(pixel(40, 30), &[255, 255, 255, 255]);
    assert_eq!(pixel(90, 30), &[0, 128, 0, 255]);
}

#[test]
fn review_normal_forced_after_moves_float_ink_to_the_new_column() {
    let scene = transform_markup_scene(
        "<!DOCTYPE html><body style='margin:0;background:white'><div style='columns:2;column-fill:auto;gap:0;width:100px;height:100px'><div style='height:30px;break-after:column'></div><div style='float:left;width:10px;height:20px;background:green'></div><div style='height:20px'></div></div></body>",
    );
    let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |renderer| renderer.append_scene(scene, Affine::IDENTITY),
        800,
        600,
    );
    let pixel = |x: usize, y: usize| &rgba[(y * 800 + x) * 4..(y * 800 + x) * 4 + 4];
    assert_eq!(pixel(5, 35), &[255, 255, 255, 255]);
    assert_eq!(pixel(55, 5), &[0, 128, 0, 255]);
}

fn review_formatting_context_background_follows_its_forced_edge(display: &str) {
    let scene = transform_markup_scene(&format!(
        "<!DOCTYPE html><body style='margin:0;background:white'><div style='columns:2;column-fill:auto;gap:0;width:100px;height:100px'><div style='height:20px'></div><div style='display:{display};height:40px;break-before:column;background:green'><div style='width:10px;height:10px'></div></div></div></body>",
    ));
    let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |renderer| renderer.append_scene(scene, Affine::IDENTITY),
        800,
        600,
    );
    let pixel = |x: usize, y: usize| &rgba[(y * 800 + x) * 4..(y * 800 + x) * 4 + 4];
    assert_eq!(pixel(40, 30), &[255, 255, 255, 255]);
    assert_eq!(pixel(90, 30), &[0, 128, 0, 255]);
}

#[test]
fn review_flex_root_background_follows_its_forced_column_edge() {
    review_formatting_context_background_follows_its_forced_edge("flex");
}

#[test]
fn review_grid_root_background_follows_its_forced_column_edge() {
    review_formatting_context_background_follows_its_forced_edge("grid");
}

#[test]
fn review_column_content_keeps_the_container_border_and_padding_clear() {
    let scene = transform_markup_scene(
        "<!DOCTYPE html><body style='margin:0;background:white'><div style='columns:2;column-fill:auto;gap:0;box-sizing:border-box;width:130px;height:130px;padding:10px;border:5px solid blue'><div style='height:20px;background:green;break-before:avoid'></div></div></body>",
    );
    let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |renderer| renderer.append_scene(scene, Affine::IDENTITY),
        800,
        600,
    );
    let pixel = |x: usize, y: usize| &rgba[(y * 800 + x) * 4..(y * 800 + x) * 4 + 4];
    assert_eq!(pixel(2, 2), &[0, 0, 255, 255]);
    assert_eq!(pixel(10, 10), &[255, 255, 255, 255]);
    assert_eq!(pixel(20, 20), &[0, 128, 0, 255]);
}

fn review_generated_content_is_not_repeated_across_columns(atomic: bool) {
    let wrapper_style = if atomic { "height:150px" } else { "" };
    let children = if atomic {
        ""
    } else {
        "<div style='height:60px'></div><div style='height:60px;break-before:avoid'></div>"
    };
    let scene = transform_markup_scene(&format!(
        "<!DOCTYPE html><style>#wrapper::before{{content:'X';color:red}}#wrapper::after{{content:'Y';color:blue}}</style><body style='margin:0;background:white'><div style='columns:2;column-fill:auto;gap:0;width:100px;height:100px'><div id='wrapper' style='{wrapper_style}'>{children}</div><div style='height:1px;break-before:avoid'></div></div></body>",
    ));
    let glyph_runs = scene
        .commands
        .iter()
        .filter(|command| matches!(command, RenderCommand::GlyphRun(_)))
        .count();
    let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |renderer| renderer.append_scene(scene, Affine::IDENTITY),
        800,
        600,
    );
    let red_ink = |start: usize, end: usize| {
        (0..30)
            .flat_map(|y| (start..end).map(move |x| (y * 800 + x) * 4))
            .filter(|&i| rgba[i] > rgba[i + 1] && rgba[i] > rgba[i + 2])
            .count()
    };
    assert!(red_ink(0, 50) > 0);
    assert_eq!(red_ink(50, 100), 0);
    assert_eq!(glyph_runs, 2);
}

#[test]
fn review_resumed_wrapper_generated_content_paints_once() {
    review_generated_content_is_not_repeated_across_columns(false);
}

#[test]
fn review_generated_only_atomic_box_keeps_one_pseudo_subtree() {
    review_generated_content_is_not_repeated_across_columns(true);
}

#[test]
fn review_list_marker_preserves_unrelated_ancestor_fragment_state() {
    let scene = transform_markup_scene(
        "<!DOCTYPE html><body style='margin:0;background:white'><div style='margin-left:40px;columns:2;column-fill:auto;gap:0;width:100px;height:100px'><div style='height:20px'></div><div style='height:60px;break-before:column'><div style='display:list-item;list-style:disc outside;height:20px'></div></div></div></body>",
    );
    let marker_runs = scene
        .commands
        .iter()
        .filter(|c| matches!(c, RenderCommand::GlyphRun(_)))
        .count();
    assert_eq!(marker_runs, 1);
    let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |renderer| renderer.append_scene(scene, Affine::IDENTITY),
        800,
        600,
    );
    let ink = |start: usize, end: usize| {
        (0..30)
            .flat_map(|y| (start..end).map(move |x| (y * 800 + x) * 4))
            .filter(|&i| rgba[i..i + 4] != [255, 255, 255, 255])
            .count()
    };
    assert_eq!(ink(15, 40), 0);
    assert!(ink(65, 90) > 0);
}

#[test]
fn review_float_text_ink_keeps_joint_exclusion_when_a_break_constraint_is_added() {
    let mut baseline_ink = None;
    for edge in ["", "break-before:avoid"] {
        let markup = format!(
            "<!DOCTYPE html><body style='margin:0;background:white'><div style='columns:2;column-fill:auto;gap:0;width:100px;height:100px'><div style='float:left;width:20px;height:40px;background:green'></div><div style='height:40px;font-family:Ahem;font-size:20px;line-height:20px;color:red'>M M</div><div style='height:1px;{edge}'></div></div></body>"
        );
        let mut parsed = raikiri_html::parse(
            markup.as_bytes(),
            &raikiri_html::ParseOptions {
                extra_stylesheets: &[],
                network: None,
                base_url: None,
            },
        )
        .unwrap();
        let fonts = raikiri_dom::build_wpt_font_collection(std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../raikiri-dom/tests/data/text-autospace"
        )))
        .unwrap();
        parsed
            .dom
            .set_font_collection_with_limits(fonts, shodo::limits::Limits::default());
        let cascade = raikiri_html::build_cascaded(&parsed);
        raikiri_dom::layout_single_page(&mut parsed.dom, &cascade, PageBox::A4).unwrap();
        let mut scene = Scene::new();
        crate::paint_single_page(&mut scene, &parsed.dom, &cascade, PageBox::A4).unwrap();
        let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
            |renderer| renderer.append_scene(scene, Affine::IDENTITY),
            800,
            600,
        );
        let red_over_float = (2..18)
            .flat_map(|y| (2..18).map(move |x| (y * 800 + x) * 4))
            .filter(|&i| rgba[i] > rgba[i + 1] && rgba[i] > rgba[i + 2])
            .count();
        let all_red = (0..40)
            .flat_map(|y| (0..100).map(move |x| (y * 800 + x) * 4))
            .filter(|&i| rgba[i] > rgba[i + 1] && rgba[i] > rgba[i + 2])
            .count();
        assert!(all_red > 0);
        if let Some(expected) = baseline_ink {
            assert_eq!(all_red, expected);
        } else {
            baseline_ink = Some(all_red);
        }
        assert_eq!(red_over_float, 0, "{edge}");
    }
}
