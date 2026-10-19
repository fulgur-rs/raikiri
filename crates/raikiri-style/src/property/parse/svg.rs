//! SVG presentation properties the host cascade resolves for inline SVG.
//!
//! Each parser validates the value and returns it in the form an SVG
//! renderer reads: colors as `rgba()`, lengths and opacities as plain
//! numbers. The text goes into a style declaration, so it is rebuilt from
//! parsed tokens and never copied from the source.

use cssparser::{Delimiter, ParseError, Parser, Token};

use super::super::calc_serialize::format_css_number;
use super::super::types::*;
use super::color::parse_contextual_color;
use super::common::next_numeric_stable;
use super::visual::parse_opacity_value;

/// Parses the value of `fill`, `stroke`, `fill-opacity`, `stroke-opacity` or
/// `stroke-width`. `None` for any other key and for an invalid value.
pub(super) fn parse_svg_presentation(
    key: PropertyKey,
    input: &mut Parser<'_, '_>,
) -> Option<PropertyValue> {
    let text = match key {
        PropertyKey::Fill | PropertyKey::Stroke => parse_svg_paint(input)?,
        PropertyKey::FillOpacity | PropertyKey::StrokeOpacity => {
            format_css_number(parse_opacity_value(input)?.clamp(0.0, 1.0))
        }
        PropertyKey::StrokeWidth => parse_svg_stroke_width(input)?,
        _ => return None,
    };
    let text = text.into_boxed_str();
    Some(match key {
        PropertyKey::Fill => PropertyValue::Fill(text),
        PropertyKey::Stroke => PropertyValue::Stroke(text),
        PropertyKey::FillOpacity => PropertyValue::FillOpacity(text),
        PropertyKey::StrokeOpacity => PropertyValue::StrokeOpacity(text),
        _ => PropertyValue::StrokeWidth(text),
    })
}

/// `<paint>` (SVG 2 §13.1): `none | <color> | <url> [none | <color>]? |
/// context-fill | context-stroke`. A `<url>` must name an element of the same
/// document (`#id`); a color that refers to `currentcolor` inside a function
/// has no value outside its element and is rejected.
fn parse_svg_paint(input: &mut Parser<'_, '_>) -> Option<String> {
    let text = if let Some(url) = optional(input, parse_svg_local_url) {
        match optional(input, parse_svg_color_or_none) {
            Some(fallback) => format!("{url} {fallback}"),
            None => url,
        }
    } else if let Some(keyword) = optional(input, parse_svg_context_keyword) {
        keyword.to_owned()
    } else {
        parse_svg_color_or_none(input)?
    };
    ends_declaration(input).then_some(text)
}

/// Runs `parse` and rewinds when it finds nothing.
fn optional<T>(
    input: &mut Parser<'_, '_>,
    parse: impl FnOnce(&mut Parser<'_, '_>) -> Option<T>,
) -> Option<T> {
    input
        .try_parse(|input| parse(input).ok_or_else(|| input.new_custom_error::<_, ()>(())))
        .ok()
}

fn parse_svg_context_keyword(input: &mut Parser<'_, '_>) -> Option<&'static str> {
    let ident = input.expect_ident().ok()?.clone();
    ["context-fill", "context-stroke"]
        .into_iter()
        .find(|keyword| ident.eq_ignore_ascii_case(keyword))
}

/// `none`, `currentcolor` or an opaque-or-translucent `<color>`.
fn parse_svg_color_or_none(input: &mut Parser<'_, '_>) -> Option<String> {
    if input
        .try_parse(|input| input.expect_ident_matching("none"))
        .is_ok()
    {
        return Some("none".to_owned());
    }
    if input
        .try_parse(|input| input.expect_ident_matching("currentcolor"))
        .is_ok()
    {
        return Some("currentColor".to_owned());
    }
    let (color, refers_to_current_color) = parse_contextual_color(input)?;
    if refers_to_current_color.is_some() {
        return None;
    }
    Some(format!(
        "rgba({},{},{},{})",
        color.r,
        color.g,
        color.b,
        format_css_number(f32::from(color.a) / 255.0),
    ))
}

/// `url(#fragment)`, serialized as `url("#fragment")`.
fn parse_svg_local_url(input: &mut Parser<'_, '_>) -> Option<String> {
    let url = match input.next().ok()?.clone() {
        Token::UnquotedUrl(url) => url.to_string(),
        Token::Function(name) if name.eq_ignore_ascii_case("url") => input
            .parse_nested_block(|input| {
                let url = input.expect_string()?.to_string();
                input.expect_exhausted()?;
                Ok::<_, ParseError<'_, ()>>(url)
            })
            .ok()?,
        _ => return None,
    };
    let fragment = url
        .strip_prefix('#')
        .filter(|fragment| !fragment.is_empty())?;
    let mut text = String::from("url(");
    // Writing to a `String` cannot fail.
    let _ = cssparser::serialize_string(&format!("#{fragment}"), &mut text);
    text.push(')');
    Some(text)
}

/// `<length-percentage> | <number>`, not negative (SVG 2 §13.3). The units
/// are the ones an SVG renderer resolves; a math function is not accepted.
fn parse_svg_stroke_width(input: &mut Parser<'_, '_>) -> Option<String> {
    let text = match next_numeric_stable(input).ok()? {
        Token::Number { value, .. } => non_negative(value).map(format_css_number)?,
        Token::Percentage { unit_value, .. } => {
            format!("{}%", format_css_number(non_negative(unit_value)? * 100.0))
        }
        Token::Dimension { value, unit, .. } => {
            let unit = ["px", "em", "ex", "pt", "pc", "mm", "cm", "in"]
                .into_iter()
                .find(|known| unit.eq_ignore_ascii_case(known))?;
            format!("{}{unit}", format_css_number(non_negative(value)?))
        }
        _ => return None,
    };
    ends_declaration(input).then_some(text)
}

fn non_negative(value: f32) -> Option<f32> {
    (value.is_finite() && value >= 0.0).then_some(value)
}

/// Whether nothing but `!important` follows.
fn ends_declaration(input: &mut Parser<'_, '_>) -> bool {
    input
        .parse_until_before(Delimiter::Bang, |input| -> Result<(), ParseError<'_, ()>> {
            Ok(input.expect_exhausted()?)
        })
        .is_ok()
}

#[cfg(test)]
mod tests;
