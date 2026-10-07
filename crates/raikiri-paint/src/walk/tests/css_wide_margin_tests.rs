use super::*;

fn margin_spec(css: &str) -> MarginBoxPaintSpec {
    let mut doc = engine_document();
    let html = doc.append_element(
        Some(0),
        "html",
        Style::default(),
        Some("color:green;font-size:40px"),
    );
    let style = doc.append_element(Some(html), "style", Style::default(), None::<&str>);
    doc.append_text(style, css);
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).unwrap();
    let rule = margin_box_rule(&cascade.page, PageMarginBoxSlot::TopCenter).unwrap();
    margin_box_spec(&doc, &cascade, &rule, 100.0, 40.0, 0, 1, false, None).unwrap()
}

#[test]
fn css_wide_margin_defaults_reach_actual_paint_spec() {
    for (keyword, expected_color, expected_font, expected_background) in [
        ("initial", Color::from_rgba8(0, 0, 0, 255), 16.0, None),
        (
            "inherit",
            Color::from_rgba8(255, 0, 0, 255),
            30.0,
            Some(Color::from_rgba8(0, 0, 255, 255)),
        ),
        ("unset", Color::from_rgba8(255, 0, 0, 255), 30.0, None),
    ] {
        let spec = margin_spec(&format!(
            "@page{{color:red;font-size:30px;background-color:blue;@top-center{{content:'x';color:{keyword};font-size:{keyword};background-color:{keyword}}}}}"
        ));
        assert_eq!(spec.text_color, expected_color, "{keyword}");
        assert_eq!(spec.text_style.font_size, expected_font, "{keyword}");
        assert_eq!(spec.background, expected_background, "{keyword}");
    }
}

#[test]
fn css_wide_margin_variable_layer_rollback_preserves_surviving_values() {
    for marker in ["revert-layer", "var(--missing, revert-layer)"] {
        let spec = margin_spec(&"@page{color:red;font-size:30px;background-color:red} @layer a,b; @layer a{@page{@top-center{content:'x';color:blue;font-size:20px;background-color:blue}}} @layer b{@page{@top-center{color:KEYWORD;font-size:KEYWORD;background-color:KEYWORD}}}".replace("KEYWORD", marker));
        assert_eq!(
            spec.text_color,
            Color::from_rgba8(0, 0, 255, 255),
            "{marker}"
        );
        assert_eq!(spec.text_style.font_size, 20.0, "{marker}");
        assert_eq!(
            spec.background,
            Some(Color::from_rgba8(0, 0, 255, 255)),
            "{marker}"
        );
    }
}

#[test]
fn css_wide_margin_variables_use_local_cascade_and_page_inheritance() {
    let spec = margin_spec(
        "@page{color:red;font-size:30px;background-color:blue;--color:blue;--background:blue;@top-center{content:'x';--color:red;--size:20px;color:var(--color);font-size:var(--size);background-color:var(--background)}}",
    );
    assert_eq!(spec.text_color, Color::from_rgba8(255, 0, 0, 255));
    assert_eq!(spec.text_style.font_size, 20.0);
    assert_eq!(spec.background, Some(Color::from_rgba8(0, 0, 255, 255)));
}
