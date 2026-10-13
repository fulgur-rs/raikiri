//! Canonical serialization of `calc()` sums of lengths and percentages from
//! their source text.
//!
//! A `calc()` that mixes units the cascade cannot combine (such as
//! `calc(10px + 0.5em)`) is kept as source text until computed-value time, so
//! CSSOM would otherwise read it back as written. CSS Values 4 §10.12
//! "Simplification" and §10.13 "Serialization"
//! (<https://www.w3.org/TR/css-values-4/#calc-simplification>) combine the
//! terms that share a unit, convert absolute lengths to `px`, and write the
//! percentage first, then the dimensions sorted by unit.

use cssparser::{ParseError, Parser, ParserInput, Token};

use super::calc_serialize::format_css_number;
use super::css_component_values_are_bounded;
use super::types::RelativeCalcLengthPercentage;

/// How deeply nested parentheses and `calc()` are followed.
const MAX_SUM_NESTING: usize = 32;

/// `raw_value` with every top-level `calc()` that is a sum of lengths and
/// percentages rewritten in its canonical form, or `None` when there is no
/// such `calc()` (callers keep the source text).
pub fn serialize_math_sums(raw_value: &str) -> Option<String> {
    if !css_component_values_are_bounded(raw_value) {
        return None;
    }
    let mut input = ParserInput::new(raw_value);
    let mut parser = Parser::new(&mut input);
    let mut out = String::with_capacity(raw_value.len());
    let mut copied_up_to = 0;
    let mut changed = false;
    loop {
        parser.skip_whitespace();
        let start = parser.position().byte_index();
        let Ok(token) = parser.next() else {
            break;
        };
        let Token::Function(name) = token else {
            continue;
        };
        if !name.eq_ignore_ascii_case("calc") {
            continue;
        }
        let canonical = parser
            .parse_nested_block(|block| {
                let terms = parse_sum(block, 0).ok_or_else(|| block.new_custom_error(()))?;
                block.expect_exhausted()?;
                Ok::<_, ParseError<'_, ()>>(terms)
            })
            .ok()
            .and_then(|terms| terms.serialize());
        if let Some(canonical) = canonical {
            out.push_str(&raw_value[copied_up_to..start]);
            out.push_str(&canonical);
            copied_up_to = parser.position().byte_index();
            changed = true;
        }
    }
    if !changed {
        return None;
    }
    out.push_str(&raw_value[copied_up_to..]);
    Some(out.trim().to_owned())
}

/// The terms of a sum, one coefficient per unit: `""` for a plain number,
/// `"%"` for a percentage, and an ASCII-lowercase length unit otherwise.
#[derive(Debug, Default)]
struct Terms(Vec<(String, f32)>);

impl Terms {
    fn single(unit: String, value: f32) -> Self {
        Self(vec![(unit, value)])
    }

    fn is_number(&self) -> bool {
        self.0.iter().all(|(unit, _)| unit.is_empty())
    }

    fn number(&self) -> Option<f32> {
        match self.0.as_slice() {
            [(unit, value)] if unit.is_empty() => Some(*value),
            _ => None,
        }
    }

    /// Adds `sign * other`. A plain number cannot be added to a length.
    fn add(mut self, other: Self, sign: f32) -> Option<Self> {
        if self.is_number() != other.is_number() {
            return None;
        }
        for (unit, value) in other.0 {
            let value = sign * value;
            match self.0.iter_mut().find(|(existing, _)| *existing == unit) {
                Some((_, existing)) => *existing += value,
                None => self.0.push((unit, value)),
            }
        }
        self.0
            .iter()
            .all(|(_, value)| value.is_finite())
            .then_some(self)
    }

    fn scale(mut self, factor: f32) -> Option<Self> {
        for (_, value) in &mut self.0 {
            *value *= factor;
            if !value.is_finite() {
                return None;
            }
        }
        Some(self)
    }

    /// The canonical `calc()` text, or `None` for a plain number sum, which
    /// other serializers own.
    fn serialize(mut self) -> Option<String> {
        if self.is_number() {
            return None;
        }
        // Terms that cancel out drop away as long as another term remains.
        if self.0.iter().any(|(_, value)| *value != 0.0) {
            self.0.retain(|(_, value)| *value != 0.0);
        }
        self.0.sort_by(|(left, _), (right, _)| {
            (left != "%", left.as_str()).cmp(&(right != "%", right.as_str()))
        });
        let mut expression = String::new();
        for (index, (unit, value)) in self.0.iter().enumerate() {
            if index == 0 {
                if value.is_sign_negative() && *value != 0.0 {
                    expression.push('-');
                }
            } else if value.is_sign_negative() && *value != 0.0 {
                expression.push_str(" - ");
            } else {
                expression.push_str(" + ");
            }
            expression.push_str(&format_css_number(value.abs()));
            expression.push_str(unit);
        }
        Some(format!("calc({expression})"))
    }
}

/// `<calc-sum>`: products joined by `+` / `-`.
fn parse_sum(input: &mut Parser<'_, '_>, nesting: usize) -> Option<Terms> {
    let mut sum = parse_product(input, nesting)?;
    while !input.is_exhausted() {
        let sign = match input.next().ok()? {
            Token::Delim('+') => 1.0,
            Token::Delim('-') => -1.0,
            _ => return None,
        };
        sum = sum.add(parse_product(input, nesting)?, sign)?;
    }
    Some(sum)
}

/// `<calc-product>`: values joined by `*` / `/`, where one side of `*` and
/// the right side of `/` must be a plain number.
fn parse_product(input: &mut Parser<'_, '_>, nesting: usize) -> Option<Terms> {
    let mut product = parse_value(input, nesting)?;
    loop {
        let operator = input.try_parse(|input| match input.next() {
            Ok(Token::Delim('*')) => Ok(true),
            Ok(Token::Delim('/')) => Ok(false),
            _ => Err(()),
        });
        let Ok(multiply) = operator else {
            return Some(product);
        };
        let right = parse_value(input, nesting)?;
        product = if multiply {
            match (product.number(), right.number()) {
                (_, Some(factor)) => product.scale(factor)?,
                (Some(factor), None) => right.scale(factor)?,
                (None, None) => return None,
            }
        } else {
            let divisor = right.number().filter(|divisor| *divisor != 0.0)?;
            product.scale(1.0 / divisor)?
        };
    }
}

/// `<calc-value>`: a number, a percentage, a length, or a parenthesized or
/// `calc()` sum.
fn parse_value(input: &mut Parser<'_, '_>, nesting: usize) -> Option<Terms> {
    let terms = match input.next().ok()?.clone() {
        Token::Number { value, .. } => Terms::single(String::new(), value),
        Token::Percentage { unit_value, .. } => Terms::single("%".to_owned(), unit_value * 100.0),
        Token::Dimension { value, unit, .. } => {
            let unit = unit.to_ascii_lowercase();
            match absolute_length_in_px(&unit) {
                Some(px) => Terms::single("px".to_owned(), value * px),
                None => {
                    RelativeCalcLengthPercentage::from_unit(1.0, &unit)?;
                    Terms::single(unit, value)
                }
            }
        }
        Token::ParenthesisBlock => parse_nested_sum(input, nesting)?,
        Token::Function(name) if name.eq_ignore_ascii_case("calc") => {
            parse_nested_sum(input, nesting)?
        }
        _ => return None,
    };
    terms
        .0
        .iter()
        .all(|(_, value)| value.is_finite())
        .then_some(terms)
}

fn parse_nested_sum(input: &mut Parser<'_, '_>, nesting: usize) -> Option<Terms> {
    if nesting >= MAX_SUM_NESTING {
        return None;
    }
    input
        .parse_nested_block(|block| {
            let terms = parse_sum(block, nesting + 1).ok_or_else(|| block.new_custom_error(()))?;
            block.expect_exhausted()?;
            Ok::<_, ParseError<'_, ()>>(terms)
        })
        .ok()
}

/// The size in `px` of one `unit` for an absolute length unit (CSS Values 4
/// §6.2), whose canonical unit is `px`.
fn absolute_length_in_px(unit: &str) -> Option<f32> {
    Some(match unit {
        "px" => 1.0,
        "in" => 96.0,
        "cm" => 96.0 / 2.54,
        "mm" => 96.0 / 25.4,
        "q" => 96.0 / 101.6,
        "pt" => 96.0 / 72.0,
        "pc" => 16.0,
        _ => return None,
    })
}

#[cfg(test)]
mod tests;
