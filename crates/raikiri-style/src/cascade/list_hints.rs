//! HTML presentational hints of the `ol` `start` and `li` `value` attributes.
//!
//! HTML Living Standard §15.3.8 "Lists"
//! (<https://html.spec.whatwg.org/multipage/rendering.html#lists>) maps them
//! onto the `list-item` counter:
//!
//! - An `ol` element's `start` attribute, when it parses as an integer, sets
//!   `counter-reset: list-item <start - 1>` so the first item, after its
//!   implicit increment, shows `start`.
//! - An `li` element's `value` attribute, when it parses as an integer, sets
//!   `counter-set: list-item <value>`.
//!
//! A reversed list counts down from a value computed from its item count,
//! which needs `reversed()` counter support; a `start` attribute on an `ol`
//! with `reversed` therefore gives no hint here.

use std::sync::Arc;

use smol_str::SmolStr;

use crate::error::CascadeError;
use crate::property::PropertyValue;
use crate::style_dom::StyleElement;

use super::candidate::CandidateSink;

const LIST_ITEM_COUNTER: &str = "list-item";

/// Pushes the `list-item` counter hints of HTML `ol` and `li` elements.
pub(crate) fn push_list_attribute_hints(
    elem: &impl StyleElement,
    sink: &mut CandidateSink<'_>,
) -> Result<(), CascadeError> {
    // The mapping belongs to the HTML namespace; `namespace_uri()` is `None`
    // for it.
    if elem.namespace_uri().is_some() {
        return Ok(());
    }
    let counter = |value: i32| Arc::new(vec![(SmolStr::new_static(LIST_ITEM_COUNTER), value)]);
    if elem.tag_name().eq_ignore_ascii_case("ol") {
        if elem.attr("reversed").is_none()
            && let Some(start) = elem.attr("start").and_then(parse_integer)
        {
            sink.push_hint(PropertyValue::CounterReset(counter(
                start.saturating_sub(1),
            )))?;
        }
    } else if elem.tag_name().eq_ignore_ascii_case("li")
        && let Some(value) = elem.attr("value").and_then(parse_integer)
    {
        sink.push_hint(PropertyValue::CounterSet(counter(value)))?;
    }
    Ok(())
}

/// HTML LS §2.3.4.1 "rules for parsing integers": leading ASCII whitespace,
/// an optional sign, then at least one digit; anything after the digits is
/// ignored. Values outside the `i32` range saturate.
fn parse_integer(input: &str) -> Option<i32> {
    let rest = input.trim_start_matches([' ', '\t', '\n', '\x0C', '\r']);
    let (negative, rest) = match rest.as_bytes().first() {
        Some(b'-') => (true, &rest[1..]),
        Some(b'+') => (false, &rest[1..]),
        _ => (false, rest),
    };
    let mut value: Option<i64> = None;
    for digit in rest.bytes().take_while(u8::is_ascii_digit) {
        let digit = i64::from(digit - b'0');
        value = Some((value.unwrap_or(0) * 10 + digit).min(i64::from(i32::MAX) + 1));
    }
    let value = if negative { -value? } else { value? };
    Some(value.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32)
}

#[cfg(test)]
mod tests;
