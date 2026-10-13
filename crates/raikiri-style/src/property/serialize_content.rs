//! Canonical serialization of the `content` property from its source text.
//!
//! The parsed `content` value drops the alternative text after `/`, which
//! CSSOM still reads back, so this re-parses the source text with
//! [`parse_content_with_alt`] and serializes both lists (CSSOM
//! "serialize a CSS value"; CSS Content 3 §1).

use cssparser::{Parser, ParserInput, serialize_identifier, serialize_string};

use super::css_component_values_are_bounded;
use super::parse::parse_content_with_alt;
use super::types::{ContentComponent, CounterStyle};

/// Serializes `raw_value` for `content`, or `None` for other properties and
/// for values holding a component this function does not spell (callers
/// keep the source text).
pub fn serialize_content_value(name: &str, raw_value: &str) -> Option<String> {
    if name != "content" || !css_component_values_are_bounded(raw_value) {
        return None;
    }
    let mut input = ParserInput::new(raw_value);
    let mut parser = Parser::new(&mut input);
    let (items, alt) = parser
        .parse_entirely(|parser| {
            parse_content_with_alt(parser).ok_or_else(|| parser.new_custom_error::<_, ()>(()))
        })
        .ok()?;
    let mut out = match items.as_slice() {
        [] => "normal".to_owned(),
        [ContentComponent::None] => "none".to_owned(),
        _ => serialize_components(&items)?,
    };
    if let Some(alt) = alt {
        out.push_str(" / ");
        out.push_str(&serialize_components(&alt)?);
    }
    Some(out)
}

/// The space-separated serialization of `items`, or `None` when one of them
/// is not spelled here.
fn serialize_components(items: &[ContentComponent]) -> Option<String> {
    let parts = items
        .iter()
        .map(serialize_component)
        .collect::<Option<Vec<_>>>()?;
    Some(parts.join(" "))
}

fn serialize_component(item: &ContentComponent) -> Option<String> {
    let mut out = String::new();
    // Writing into a `String` cannot fail.
    match item {
        ContentComponent::Literal(text) => {
            let _ = serialize_string(text, &mut out);
        }
        ContentComponent::Image { url } => {
            out.push_str("url(");
            let _ = serialize_string(url, &mut out);
            out.push(')');
        }
        ContentComponent::Quote(keyword) => out.push_str(keyword.as_css_str()),
        ContentComponent::Contents => out.push_str("contents"),
        ContentComponent::Counter { name, style } => {
            out.push_str("counter(");
            let _ = serialize_identifier(name, &mut out);
            push_counter_style(&mut out, style);
            out.push(')');
        }
        ContentComponent::Counters {
            name,
            separator,
            style,
        } => {
            out.push_str("counters(");
            let _ = serialize_identifier(name, &mut out);
            out.push_str(", ");
            let _ = serialize_string(separator, &mut out);
            push_counter_style(&mut out, style);
            out.push(')');
        }
        ContentComponent::Attr { name } => {
            out.push_str("attr(");
            let _ = serialize_identifier(name, &mut out);
            out.push(')');
        }
        ContentComponent::AttrFallback {
            name,
            fallback: Some(fallback),
        } => {
            out.push_str("attr(");
            let _ = serialize_identifier(name, &mut out);
            out.push_str(", ");
            let _ = serialize_string(fallback, &mut out);
            out.push(')');
        }
        _ => return None,
    }
    Some(out)
}

/// The `, <counter-style>` argument, omitted for the default `decimal`
/// (CSSOM serializes the shortest form that round-trips).
fn push_counter_style(out: &mut String, style: &CounterStyle) {
    if let CounterStyle::Named(style) = style {
        out.push_str(", ");
        let _ = serialize_identifier(style, out);
    }
}

#[cfg(test)]
mod tests;
