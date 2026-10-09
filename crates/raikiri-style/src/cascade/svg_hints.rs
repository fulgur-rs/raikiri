//! Outermost SVG sizing attributes used by CSS layout and vector consumers.

use cssparser::{Parser, ParserInput};

use crate::property::{Length, LengthOrAuto, PropertyValue};
use crate::style_dom::StyleElement;

use super::collect::CascadedDecl;

/// Add geometry and font sizing of a namespace-checked outermost SVG root.
///
/// SVG 2 geometry sizing maps these attributes to CSS properties only on
/// outermost SVG roots. Presentation attributes precede author stylesheets
/// with zero specificity: <https://www.w3.org/TR/SVG2/geometry.html#Sizing>
/// and <https://www.w3.org/TR/SVG2/styling.html#PresentationAttributes>.
pub(super) fn push_dimension_hints(elem: &impl StyleElement, decls: &mut Vec<CascadedDecl>) {
    for name in ["width", "height", "font-size"] {
        let Some(value) = elem.attr(name).and_then(|raw| {
            if name == "font-size" {
                parse_font_size(raw)
            } else {
                parse_dimension(name, raw)
            }
        }) else {
            continue;
        };
        decls.push(CascadedDecl::hint(value));
    }
}

/// Add a SVG descendant's font-size presentation hint without geometry hints.
pub(super) fn push_font_size_hint(elem: &impl StyleElement, decls: &mut Vec<CascadedDecl>) {
    if let Some(value) = elem.attr("font-size").and_then(parse_font_size) {
        decls.push(CascadedDecl::hint(value));
    }
}

fn parse_font_size(raw: &str) -> Option<PropertyValue> {
    let mut input = ParserInput::new(raw);
    let mut parser = Parser::new(&mut input);
    let value = if let Ok(number) = parser.try_parse(|parser| parser.expect_number()) {
        if !number.is_finite() || number < 0.0 {
            return None;
        }
        PropertyValue::FontSize(Length::Px(number))
    } else {
        crate::property::parse_value("font-size", &mut parser)?
    };
    parser.expect_exhausted().ok()?;
    Some(value)
}

fn parse_dimension(name: &str, raw: &str) -> Option<PropertyValue> {
    let mut input = ParserInput::new(raw);
    let mut parser = Parser::new(&mut input);
    let dimension = if let Ok(number) = parser.try_parse(|parser| parser.expect_number()) {
        // SVG unitless lengths use user units, which are CSS px at the root.
        if !number.is_finite() || number < 0.0 {
            return None;
        }
        LengthOrAuto::Length(Length::Px(number))
    } else if parser
        .try_parse(|parser| parser.expect_ident_matching("auto"))
        .is_ok()
    {
        LengthOrAuto::Auto
    } else {
        let value = crate::property::parse_value(name, &mut parser)?;
        match value {
            PropertyValue::Width(LengthOrAuto::Length(length))
            | PropertyValue::Height(LengthOrAuto::Length(length))
                if length.payload().is_finite() =>
            {
                LengthOrAuto::Length(length)
            }
            PropertyValue::Deferred(_) if parser.expect_exhausted().is_ok() => return Some(value),
            _ => return None,
        }
    };
    parser.expect_exhausted().ok()?;
    Some(if name == "width" {
        PropertyValue::Width(dimension)
    } else {
        PropertyValue::Height(dimension)
    })
}
