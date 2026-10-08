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
            svg_node_declarations(&ComputedValues::initial(), &properties, &mut budget).unwrap(),
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
    assert!(
        svg_node_declarations(&ComputedValues::initial(), &properties, &mut remaining).is_err()
    );
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
        svg_node_declarations(&ComputedValues::initial(), &properties, &mut remaining).unwrap(),
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
    let css = svg_node_declarations(&ComputedValues::initial(), &properties, &mut budget).unwrap();
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
    let css = svg_node_declarations(&ComputedValues::initial(), &properties, &mut budget).unwrap();
    // Inherited font sizes use `100%` so renderers do not reapply a relative
    // ancestor size.
    assert_eq!(css.matches(":inherit!important;").count(), 7, "{css}");
    assert!(css.contains("font-size:100%!important;"), "{css}");
}
