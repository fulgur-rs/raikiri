//! Canonical serialization of `<opacity-value>` (`<number> | <percentage>`)
//! from its source text.
//!
//! `opacity` parses to a single clamped `f32`, which loses both the
//! percentage form and whether a math function was written. CSSOM reads a
//! plain percentage back as the equivalent number (CSS Color 4 §"Transparency:
//! the opacity property") and a math function as a simplified `calc()`
//! (CSS Values 4 §"Serialization"), so this works from the source text.

use cssparser::{Parser, ParserInput, Token};

use super::calc_serialize::format_css_number;
use super::css_component_values_are_bounded;
use super::parse::next_numeric_stable;

/// Serializes `raw_value` for `opacity`, or `None` for other properties and
/// for text whose math cannot be folded here (callers keep the source text).
pub fn serialize_alpha_value(name: &str, raw_value: &str) -> Option<String> {
    if !matches!(
        name,
        "opacity" | "fill-opacity" | "stroke-opacity" | "flood-opacity"
    ) || !css_component_values_are_bounded(raw_value)
    {
        return None;
    }
    let mut input = ParserInput::new(raw_value);
    let mut parser = Parser::new(&mut input);
    // Numeric tokens go through the same NaN recovery as the property
    // parser, so `0e999` reads back as `0`.
    let token = next_numeric_stable(&mut parser).ok()?;
    let value = match token {
        Token::Number { value, .. } => {
            parser.expect_exhausted().ok()?;
            return Some(format_css_number(value));
        }
        Token::Percentage { unit_value, .. } => {
            parser.expect_exhausted().ok()?;
            return Some(format_css_number(unit_value));
        }
        Token::Function(name) => {
            let value = parser
                .parse_nested_block(|block| {
                    math_function(&name, block).ok_or_else(|| block.new_custom_error::<_, ()>(()))
                })
                .ok()?;
            parser.expect_exhausted().ok()?;
            value
        }
        _ => return None,
    };
    if !value.0.is_finite() {
        return None;
    }
    Some(if value.1 {
        format!("calc({}%)", format_css_number(value.0 as f32))
    } else {
        format!("calc({})", format_css_number(value.0 as f32))
    })
}

/// A folded value and whether it is a percentage.
type Typed = (f64, bool);

/// The deepest math-function nesting folded here.
const MAX_DEPTH: usize = 32;

fn math_function(name: &str, block: &mut Parser<'_, '_>) -> Option<Typed> {
    math_function_at(name, block, 0)
}

fn math_function_at(name: &str, block: &mut Parser<'_, '_>, depth: usize) -> Option<Typed> {
    if depth > MAX_DEPTH {
        return None;
    }
    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        "calc" => {
            let value = sum(block, depth)?;
            block.expect_exhausted().ok()?;
            Some(value)
        }
        "min" | "max" | "clamp" => {
            let mut args = Vec::new();
            loop {
                args.push(sum(block, depth)?);
                if block.is_exhausted() {
                    break;
                }
                block.expect_comma().ok()?;
            }
            let percent = args[0].1;
            if args.iter().any(|arg| arg.1 != percent) {
                return None;
            }
            let values: Vec<f64> = args.iter().map(|arg| arg.0).collect();
            let value = match (lower.as_str(), values.as_slice()) {
                ("min", _) => values.iter().copied().fold(f64::INFINITY, f64::min),
                ("max", _) => values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                ("clamp", [min, value, max]) => value.min(*max).max(*min),
                _ => return None,
            };
            Some((value, percent))
        }
        _ => None,
    }
}

fn sum(input: &mut Parser<'_, '_>, depth: usize) -> Option<Typed> {
    let mut value = product(input, depth)?;
    loop {
        let state = input.state();
        let op = match input.next() {
            Ok(Token::Delim('+')) => 1.0,
            Ok(Token::Delim('-')) => -1.0,
            _ => {
                input.reset(&state);
                return Some(value);
            }
        };
        let right = product(input, depth)?;
        if right.1 != value.1 {
            return None;
        }
        value.0 += op * right.0;
    }
}

fn product(input: &mut Parser<'_, '_>, depth: usize) -> Option<Typed> {
    let mut value = leaf(input, depth)?;
    loop {
        let state = input.state();
        let multiply = match input.next() {
            Ok(Token::Delim('*')) => true,
            Ok(Token::Delim('/')) => false,
            _ => {
                input.reset(&state);
                return Some(value);
            }
        };
        let right = leaf(input, depth)?;
        value = match (multiply, value.1, right.1) {
            (true, _, false) => (value.0 * right.0, value.1),
            (true, false, true) => (value.0 * right.0, true),
            (false, _, false) => (value.0 / right.0, value.1),
            _ => return None,
        };
    }
}

fn leaf(input: &mut Parser<'_, '_>, depth: usize) -> Option<Typed> {
    match next_numeric_stable(input).ok()? {
        Token::Number { value, .. } => Some((f64::from(value), false)),
        Token::Percentage { unit_value, .. } => Some((f64::from(unit_value) * 100.0, true)),
        Token::ParenthesisBlock => input
            .parse_nested_block(|block| {
                let value = sum(block, depth + 1);
                value.ok_or_else(|| block.new_custom_error::<_, ()>(()))
            })
            .ok(),
        Token::Function(name) => input
            .parse_nested_block(|block| {
                math_function_at(&name, block, depth + 1)
                    .ok_or_else(|| block.new_custom_error::<_, ()>(()))
            })
            .ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
