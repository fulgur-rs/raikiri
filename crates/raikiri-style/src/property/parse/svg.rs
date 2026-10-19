//! SVG presentation properties the host cascade resolves for inline SVG.
//!
//! Each parser validates the value and returns it in the form an SVG
//! renderer reads: colors as `rgba()`, lengths and opacities as plain
//! numbers, keywords in lowercase. The text goes into a style declaration, so
//! it is rebuilt from parsed tokens and never copied from the source.

use cssparser::{Delimiter, ParseError, Parser, Token};

use super::super::calc_serialize::format_css_number;
use super::super::types::*;
use super::color::parse_contextual_color;
use super::common::next_numeric_stable;
use super::visual::parse_opacity_value;

/// Parses the value of an SVG painting property (`fill`, `stroke`, their
/// opacities, `stroke-width`, the fill and clip rules and the stroke line and
/// dash properties). `None` for any other key and for an invalid value.
pub(super) fn parse_svg_presentation(
    key: PropertyKey,
    input: &mut Parser<'_, '_>,
) -> Option<PropertyValue> {
    let text = match key {
        PropertyKey::Fill | PropertyKey::Stroke => parse_svg_paint(input)?,
        PropertyKey::FillOpacity | PropertyKey::StrokeOpacity => {
            format_css_number(parse_opacity_value(input)?.clamp(0.0, 1.0))
        }
        PropertyKey::StrokeWidth => parse_svg_length(input, false)?,
        PropertyKey::StrokeDashoffset => parse_svg_length(input, true)?,
        PropertyKey::FillRule | PropertyKey::ClipRule => {
            parse_svg_keyword(input, &["nonzero", "evenodd"])?
        }
        PropertyKey::StrokeLinecap => parse_svg_keyword(input, &["butt", "round", "square"])?,
        PropertyKey::StrokeLinejoin => parse_svg_linejoin(input)?,
        PropertyKey::StrokeMiterlimit => parse_svg_miterlimit(input)?,
        PropertyKey::StrokeDasharray => parse_svg_dasharray(input)?,
        _ => return None,
    };
    PropertyValue::svg_presentation(key, text.into_boxed_str())
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

/// One keyword out of `keywords`, in the lowercase form the list spells.
fn parse_svg_keyword(input: &mut Parser<'_, '_>, keywords: &[&str]) -> Option<String> {
    let ident = input.expect_ident().ok()?.clone();
    let keyword = keywords
        .iter()
        .find(|keyword| ident.eq_ignore_ascii_case(keyword))?;
    ends_declaration(input).then(|| (*keyword).to_owned())
}

/// `miter | miter-clip | round | bevel | arcs` (SVG 2 §13.5). `arcs` is
/// written as `miter`, the fallback the specification gives a renderer that
/// does not draw arcs joins.
fn parse_svg_linejoin(input: &mut Parser<'_, '_>) -> Option<String> {
    let keyword = parse_svg_keyword(input, &["miter", "miter-clip", "round", "bevel", "arcs"])?;
    Some(if keyword == "arcs" {
        "miter".to_owned()
    } else {
        keyword
    })
}

/// `<number>` of at least `1` (SVG 2 §13.5).
fn parse_svg_miterlimit(input: &mut Parser<'_, '_>) -> Option<String> {
    let Token::Number { value, .. } = next_numeric_stable(input).ok()? else {
        return None;
    };
    (value.is_finite() && value >= 1.0).then_some(())?;
    ends_declaration(input).then(|| format_css_number(value))
}

/// `none | [ [ <length-percentage> | <number> ]+ ]#`, no item negative (SVG 2
/// §13.6). The items are written separated by single spaces.
fn parse_svg_dasharray(input: &mut Parser<'_, '_>) -> Option<String> {
    if input
        .try_parse(|input| input.expect_ident_matching("none"))
        .is_ok()
    {
        return ends_declaration(input).then(|| "none".to_owned());
    }
    let mut items = vec![optional(input, |input| {
        parse_svg_length_item(input, false)
    })?];
    loop {
        // A comma must be followed by an item; a space may be followed by the
        // end of the value or `!important`.
        let after_comma = input.try_parse(|input| input.expect_comma()).is_ok();
        match optional(input, |input| parse_svg_length_item(input, false)) {
            Some(item) => items.push(item),
            None if after_comma => return None,
            None => break,
        }
    }
    ends_declaration(input).then(|| items.join(" "))
}

/// A single `<length-percentage> | <number>` (SVG 2 §13.3), negative only when
/// `allow_negative`.
fn parse_svg_length(input: &mut Parser<'_, '_>, allow_negative: bool) -> Option<String> {
    let text = parse_svg_length_item(input, allow_negative)?;
    ends_declaration(input).then_some(text)
}

/// The next `<length-percentage> | <number>` token. The units are the ones an
/// SVG renderer resolves; a math function is not accepted.
fn parse_svg_length_item(input: &mut Parser<'_, '_>, allow_negative: bool) -> Option<String> {
    let accepted =
        |value: f32| (value.is_finite() && (allow_negative || value >= 0.0)).then_some(value);
    Some(match next_numeric_stable(input).ok()? {
        Token::Number { value, .. } => format_css_number(accepted(value)?),
        Token::Percentage { unit_value, .. } => {
            format!("{}%", format_css_number(accepted(unit_value)? * 100.0))
        }
        Token::Dimension { value, unit, .. } => {
            let unit = ["px", "em", "ex", "pt", "pc", "mm", "cm", "in"]
                .into_iter()
                .find(|known| unit.eq_ignore_ascii_case(known))?;
            format!("{}{unit}", format_css_number(accepted(value)?))
        }
        _ => return None,
    })
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
