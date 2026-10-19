use super::*;
use raikiri_style::cascade::SvgStyleProperty;
use raikiri_style::property::PropertyKey;

#[test]
fn descendant_declarations_ignore_properties_outside_the_svg_contract() {
    for inherited in [false, true] {
        let properties = [SvgStyleProperty {
            property: PropertyKey::Width,
            inherited,
            expression: None,
        }];
        let mut budget = 1024;
        assert_eq!(
            node_declarations(&ComputedValues::initial(), &properties, &mut budget).unwrap(),
            ""
        );
    }
}

#[test]
fn descendant_declaration_budget_rejects_before_serializing() {
    let properties = [SvgStyleProperty {
        property: PropertyKey::Opacity,
        inherited: false,
        expression: None,
    }];
    let mut remaining = 128;
    assert!(node_declarations(&ComputedValues::initial(), &properties, &mut remaining).is_err());
}

#[test]
fn descendant_declaration_budget_counts_the_text_of_a_painting_property() {
    let text = format!("url(\"#{}\")", "a".repeat(1000));
    let properties = [SvgStyleProperty {
        property: PropertyKey::Fill,
        inherited: false,
        expression: Some(text.clone()),
    }];
    // The declaration's own 128 bytes, then the text's 128 and its two copies.
    let needed = 128 + 128 + 2 * text.len();
    let mut remaining = needed - 1;
    assert!(node_declarations(&ComputedValues::initial(), &properties, &mut remaining).is_err());
    let mut remaining = needed;
    let css = node_declarations(&ComputedValues::initial(), &properties, &mut remaining).unwrap();
    assert_eq!(css, format!("fill:{text}!important;"));
    assert_eq!(remaining, 0);
}

#[test]
fn descendant_declarations_keep_relative_expressions_and_literal_inherit() {
    let properties = [
        SvgStyleProperty {
            property: PropertyKey::FontSize,
            inherited: false,
            expression: Some("2em".into()),
        },
        SvgStyleProperty {
            property: PropertyKey::Color,
            inherited: true,
            expression: None,
        },
    ];
    let mut remaining = 1024;
    assert_eq!(
        node_declarations(&ComputedValues::initial(), &properties, &mut remaining).unwrap(),
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
        .map(|&property| SvgStyleProperty {
            property,
            inherited: false,
            expression: None,
        })
        .collect();
    let mut budget = 4096;
    let css = node_declarations(&ComputedValues::initial(), &properties, &mut budget).unwrap();
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
        .map(|&property| SvgStyleProperty {
            property,
            inherited: true,
            expression: None,
        })
        .collect();
    let mut budget = 4096;
    let css = node_declarations(&ComputedValues::initial(), &properties, &mut budget).unwrap();
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
        let declared = [SvgStyleProperty {
            property,
            inherited: false,
            expression: Some(value.into()),
        }];
        let mut budget = 1024;
        assert_eq!(
            node_declarations(&ComputedValues::initial(), &declared, &mut budget).unwrap(),
            format!("{name}:{value}!important;")
        );

        let inherited = [SvgStyleProperty {
            property,
            inherited: true,
            expression: None,
        }];
        let mut budget = 1024;
        assert_eq!(
            node_declarations(&ComputedValues::initial(), &inherited, &mut budget).unwrap(),
            format!("{name}:inherit!important;")
        );

        // No computed field backs these properties, so a declaration without
        // its text has nothing to export.
        let missing = [SvgStyleProperty {
            property,
            inherited: false,
            expression: None,
        }];
        let mut budget = 1024;
        assert_eq!(
            node_declarations(&ComputedValues::initial(), &missing, &mut budget).unwrap(),
            ""
        );
    }
}
