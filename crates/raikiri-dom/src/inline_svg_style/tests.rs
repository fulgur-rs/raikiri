use super::*;
use raikiri_style::cascade::SvgStyleProperty;
use raikiri_style::property::PropertyKey;

#[test]
fn descendant_declarations_ignore_properties_outside_the_svg_contract() {
    for inherited in [false, true] {
        let properties = [SvgStyleProperty::new(
            PropertyKey::Width,
            inherited,
            None,
            None,
        )];
        let mut budget = 1024;
        assert_eq!(
            node_declarations(
                &ComputedValues::initial(),
                &properties,
                &[],
                false,
                &mut budget
            )
            .unwrap(),
            ""
        );
    }
}

#[test]
fn descendant_declaration_budget_rejects_before_serializing() {
    let properties = [SvgStyleProperty::new(
        PropertyKey::Opacity,
        false,
        None,
        None,
    )];
    let mut remaining = 128;
    assert!(
        node_declarations(
            &ComputedValues::initial(),
            &properties,
            &[],
            false,
            &mut remaining
        )
        .is_err()
    );
}

#[test]
fn descendant_declaration_budget_counts_the_text_of_a_painting_property() {
    let text = format!("url(\"#{}\")", "a".repeat(1000));
    let properties = [SvgStyleProperty::new(
        PropertyKey::Fill,
        false,
        Some(text.clone()),
        None,
    )];
    // The declaration's own 128 bytes, then the text's 128 and its two copies.
    let needed = 128 + 128 + 2 * text.len();
    let mut remaining = needed - 1;
    assert!(
        node_declarations(
            &ComputedValues::initial(),
            &properties,
            &[],
            false,
            &mut remaining
        )
        .is_err()
    );
    let mut remaining = needed;
    let css = node_declarations(
        &ComputedValues::initial(),
        &properties,
        &[],
        false,
        &mut remaining,
    )
    .unwrap();
    assert_eq!(css, format!("fill:{text}!important;"));
    assert_eq!(remaining, 0);
}

#[test]
fn descendant_declarations_keep_relative_expressions_and_literal_inherit() {
    let properties = [
        SvgStyleProperty::new(PropertyKey::FontSize, false, Some("2em".into()), None),
        SvgStyleProperty::new(PropertyKey::Color, true, None, None),
    ];
    let mut remaining = 1024;
    assert_eq!(
        node_declarations(
            &ComputedValues::initial(),
            &properties,
            &[],
            false,
            &mut remaining
        )
        .unwrap(),
        "font-size:2em!important;color:inherit!important;"
    );
}

#[test]
fn descendant_declarations_serialize_all_supported_computed_properties() {
    let keys = [
        PropertyKey::Color,
        PropertyKey::Display,
        PropertyKey::Opacity,
        PropertyKey::Visibility,
        PropertyKey::FontSize,
        PropertyKey::FontFamily,
        PropertyKey::FontStyle,
        PropertyKey::FontWeight,
    ];
    let properties: Vec<_> = keys
        .iter()
        .map(|&property| SvgStyleProperty::new(property, false, None, None))
        .collect();
    let mut budget = 4096;
    let css = node_declarations(
        &ComputedValues::initial(),
        &properties,
        &[],
        false,
        &mut budget,
    )
    .unwrap();
    for declaration in [
        "color:rgba(0,0,0,1.000000)!important;",
        "display:inline!important;",
        "opacity:1!important;",
        "visibility:visible!important;",
        "font-size:16px!important;",
        "font-family:serif!important;",
        "font-style:normal!important;",
        "font-weight:400!important;",
    ] {
        assert!(css.contains(declaration), "{css}");
    }
    let properties: Vec<_> = keys
        .iter()
        .map(|&property| SvgStyleProperty::new(property, true, None, None))
        .collect();
    let mut budget = 4096;
    let css = node_declarations(
        &ComputedValues::initial(),
        &properties,
        &[],
        false,
        &mut budget,
    )
    .unwrap();
    // Inherited font sizes use `100%` so renderers do not reapply a relative
    // ancestor size.
    assert_eq!(css.matches(":inherit!important;").count(), 7, "{css}");
    assert!(css.contains("font-size:100%!important;"), "{css}");
}

#[test]
fn descendant_declarations_carry_painting_properties_as_declared() {
    let painting = [
        (PropertyKey::Fill, "fill", "rgba(255,0,0,1)"),
        (PropertyKey::Stroke, "stroke", "url(\"#g\") none"),
        (PropertyKey::FillOpacity, "fill-opacity", "0.5"),
        (PropertyKey::StrokeOpacity, "stroke-opacity", "0.25"),
        (PropertyKey::StrokeWidth, "stroke-width", "3px"),
        (PropertyKey::FillRule, "fill-rule", "evenodd"),
        (PropertyKey::ClipRule, "clip-rule", "evenodd"),
        (PropertyKey::StrokeLinecap, "stroke-linecap", "round"),
        (PropertyKey::StrokeLinejoin, "stroke-linejoin", "miter-clip"),
        (PropertyKey::StrokeMiterlimit, "stroke-miterlimit", "2.5"),
        (PropertyKey::StrokeDasharray, "stroke-dasharray", "4 2"),
        (PropertyKey::StrokeDashoffset, "stroke-dashoffset", "-1px"),
    ];
    for (property, name, value) in painting {
        let declared = [SvgStyleProperty::new(
            property,
            false,
            Some(value.into()),
            None,
        )];
        let mut budget = 1024;
        assert_eq!(
            node_declarations(
                &ComputedValues::initial(),
                &declared,
                &[],
                false,
                &mut budget
            )
            .unwrap(),
            format!("{name}:{value}!important;")
        );

        let inherited = [SvgStyleProperty::new(property, true, None, None)];
        let mut budget = 1024;
        assert_eq!(
            node_declarations(
                &ComputedValues::initial(),
                &inherited,
                &[],
                false,
                &mut budget
            )
            .unwrap(),
            format!("{name}:inherit!important;")
        );

        // No computed field backs these properties, so a declaration without
        // its text has nothing to export.
        let missing = [SvgStyleProperty::new(property, false, None, None)];
        let mut budget = 1024;
        assert_eq!(
            node_declarations(
                &ComputedValues::initial(),
                &missing,
                &[],
                false,
                &mut budget
            )
            .unwrap(),
            ""
        );
    }
}

#[test]
fn relative_presentation_attributes_resolve_against_the_declaring_font_size() {
    let attributes = [
        SvgStyleProperty::new(PropertyKey::StrokeWidth, false, Some("2em".into()), None),
        SvgStyleProperty::new(
            PropertyKey::StrokeDasharray,
            false,
            Some("2em 1ex".into()),
            None,
        ),
        SvgStyleProperty::new(
            PropertyKey::StrokeDashoffset,
            false,
            Some("-1em".into()),
            None,
        ),
    ];
    let mut style = ComputedValues::initial();
    style.font_size.0 = 20.0;
    let mut budget = 4096;
    assert_eq!(
        node_declarations(&style, &[], &attributes, false, &mut budget).unwrap(),
        "stroke-width:40px!important;stroke-dasharray:40px 10px!important;stroke-dashoffset:-20px!important;"
    );
}

#[test]
fn absolute_presentation_attributes_need_no_declaration() {
    let attributes = [SvgStyleProperty::new(
        PropertyKey::StrokeWidth,
        false,
        Some("3px".into()),
        None,
    )];
    let mut budget = 4096;
    assert_eq!(
        node_declarations(
            &ComputedValues::initial(),
            &[],
            &attributes,
            false,
            &mut budget
        )
        .unwrap(),
        ""
    );
}

#[test]
fn declared_font_relative_stroke_lengths_are_written_in_px_outside_an_instance() {
    let declared = [
        SvgStyleProperty::new(PropertyKey::StrokeWidth, false, Some("2em".into()), None),
        SvgStyleProperty::new(
            PropertyKey::StrokeDasharray,
            false,
            Some("2em 50% 1ex 3".into()),
            None,
        ),
        SvgStyleProperty::new(
            PropertyKey::StrokeDashoffset,
            false,
            Some("-1em".into()),
            None,
        ),
        SvgStyleProperty::new(
            PropertyKey::StrokeLinecap,
            false,
            Some("round".into()),
            None,
        ),
    ];
    let mut style = ComputedValues::initial();
    style.font_size.0 = 20.0;
    let mut budget = 4096;
    assert_eq!(
        node_declarations(&style, &declared, &[], false, &mut budget).unwrap(),
        "stroke-width:40px!important;stroke-dasharray:40px 50% 10px 3!important;stroke-dashoffset:-20px!important;stroke-linecap:round!important;"
    );
    // An instance resolves the same text against its own font size.
    let mut budget = 4096;
    assert_eq!(
        node_declarations(&style, &declared, &[], true, &mut budget).unwrap(),
        "stroke-width:2em!important;stroke-dasharray:2em 50% 1ex 3!important;stroke-dashoffset:-1em!important;stroke-linecap:round!important;"
    );
}

#[test]
fn the_px_text_of_a_stroke_length_counts_against_the_budget() {
    let declared = [SvgStyleProperty::new(
        PropertyKey::StrokeWidth,
        false,
        Some("2em".into()),
        None,
    )];
    let mut style = ComputedValues::initial();
    style.font_size.0 = 20.0;
    // The declaration's own 128 bytes, the text's 128 and its two copies, and
    // two copies of the `40px` that replaces it.
    let needed = 128 + (128 + 2 * 3) + 2 * 4;
    let mut remaining = needed - 1;
    assert!(node_declarations(&style, &declared, &[], false, &mut remaining).is_err());
    let mut remaining = needed;
    node_declarations(&style, &declared, &[], false, &mut remaining).unwrap();
    assert_eq!(remaining, 0);
}

#[test]
fn an_inherited_stroke_length_stays_inherit_in_any_place() {
    let inherited = [SvgStyleProperty::new(
        PropertyKey::StrokeWidth,
        true,
        Some("2em".into()),
        None,
    )];
    let mut style = ComputedValues::initial();
    style.font_size.0 = 20.0;
    for in_instance in [false, true] {
        let mut budget = 4096;
        assert_eq!(
            node_declarations(&style, &inherited, &[], in_instance, &mut budget).unwrap(),
            "stroke-width:inherit!important;"
        );
    }
}

#[test]
fn a_local_reference_is_read_as_the_renderer_reads_it() {
    assert_eq!(iri_fragment("#shape"), Some("shape"));
    assert_eq!(iri_fragment(" \n#shape  "), Some("shape"));
    // Only a space ends the id.
    assert_eq!(iri_fragment("#a\tb"), Some("a\tb"));
    for unreadable in ["", "#", "shape", "shape#", "#a b", "file.svg#shape"] {
        assert_eq!(iri_fragment(unreadable), None, "{unreadable:?}");
    }
}
