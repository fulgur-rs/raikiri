//! Outermost SVG geometry attributes used by CSS layout.

use cssparser::{Parser, ParserInput};

use crate::layer::LayerPosition;
use crate::property::{Length, LengthOrAuto, PropertyValue};
use crate::ruletree::Origin;
use crate::style_dom::StyleElement;

use super::collect::{
    CascadedDecl, PRESENTATIONAL_HINT_SOURCE_ORDER, PRESENTATIONAL_HINT_SPECIFICITY,
};

/// Add dimensions of an already namespace-checked outermost SVG root.
///
/// SVG 2 geometry sizing maps these attributes to CSS properties only on
/// outermost SVG roots. Presentation attributes precede author stylesheets
/// with zero specificity: <https://www.w3.org/TR/SVG2/geometry.html#Sizing>
/// and <https://www.w3.org/TR/SVG2/styling.html#PresentationAttributes>.
pub(super) fn push_dimension_hints(elem: &impl StyleElement, decls: &mut Vec<CascadedDecl>) {
    for name in ["width", "height"] {
        let Some(value) = elem.attr(name).and_then(|raw| parse_dimension(name, raw)) else {
            continue;
        };
        decls.push((
            value,
            false,
            Origin::AuthorPresentationalHint,
            PRESENTATIONAL_HINT_SPECIFICITY,
            PRESENTATIONAL_HINT_SOURCE_ORDER,
            LayerPosition::default(),
        ));
    }
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
            PropertyValue::Width(LengthOrAuto::Calc(calc))
            | PropertyValue::Height(LengthOrAuto::Calc(calc)) => LengthOrAuto::Calc(calc),
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
