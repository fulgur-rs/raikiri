//! Comparison functions over percentages only, for `<position>` offsets.
//!
//! `min()`, `max()` and `clamp()` of percentages pick an operand by size, and
//! CSS Values 4 "Computed Value"
//! (<https://www.w3.org/TR/css-values-4/#calc-computed-value>) keeps a
//! percentage unresolved until used-value time. For a background position the
//! percentage basis is the positioning area minus the image, and it is
//! negative when the image is larger than the area. A negative basis reverses
//! the order of the operands (`100%` of `-50px` lies below `0%`), so the
//! winner cannot be named before the basis is known. [`ComparedPercentage`]
//! keeps the winner for both signs.
//!
//! The evaluator reads the sums that CSS Values 4 §10 allows inside a math
//! function, limited to what is needed here: percentages, parentheses, `+`
//! and `-`, `calc()`, `min()`, `max()` and `clamp()`. A number or a length
//! anywhere in the expression makes it unsupported, and so does a
//! multiplication or division; the caller then falls back to the generic math
//! simplifier, which compares the percentages themselves.

use cssparser::{ParseError, Parser, Token};

use super::common::next_numeric_stable;
use crate::property::types::{ComparedPercentage, MAX_DEFERRED_VALUE_NESTING_DEPTH};

/// The percentage an expression stands for, once for a basis that is zero or
/// positive and once for a negative basis.
#[derive(Clone, Copy)]
struct SignedPercentage {
    non_negative: f32,
    negative: f32,
}

impl SignedPercentage {
    fn constant(percent: f32) -> Self {
        Self {
            non_negative: percent,
            negative: percent,
        }
    }

    /// `self + sign * other`, or `None` when a sum leaves the finite range.
    fn add_scaled(self, other: Self, sign: f32) -> Option<Self> {
        let sum = Self {
            non_negative: self.non_negative + sign * other.non_negative,
            negative: self.negative + sign * other.negative,
        };
        (sum.non_negative.is_finite() && sum.negative.is_finite()).then_some(sum)
    }

    /// The smaller operand for a basis that is zero or positive. A negative
    /// basis flips every comparison, so the larger percentage is the smaller
    /// length.
    fn min(self, other: Self) -> Self {
        Self {
            non_negative: self.non_negative.min(other.non_negative),
            negative: self.negative.max(other.negative),
        }
    }

    /// The counterpart of [`Self::min`].
    fn max(self, other: Self) -> Self {
        Self {
            non_negative: self.non_negative.max(other.non_negative),
            negative: self.negative.min(other.negative),
        }
    }
}

/// Read one `min()`, `max()`, `clamp()` or `calc()` of percentages. The parser
/// is left where it was when the next component value is anything else.
pub(super) fn parse_percentage_comparison(
    input: &mut Parser<'_, '_>,
) -> Option<ComparedPercentage> {
    input
        .try_parse(|parser| {
            let token = parser.next()?.clone();
            let Token::Function(name) = token else {
                return Err(parser.new_unexpected_token_error(token));
            };
            let value = parse_math_function(parser, &name, 0)?;
            Ok::<_, ParseError<'_, ()>>(ComparedPercentage {
                non_negative: value.non_negative,
                negative: value.negative,
            })
        })
        .ok()
}

/// The body of the math function `name`, whose opening token `parser` has just
/// consumed.
fn parse_math_function<'i>(
    parser: &mut Parser<'i, '_>,
    name: &str,
    depth: usize,
) -> Result<SignedPercentage, ParseError<'i, ()>> {
    let depth = depth + 1;
    if name.eq_ignore_ascii_case("calc") {
        return parser.parse_nested_block(|block| parse_sum(block, depth));
    }
    let is_min = name.eq_ignore_ascii_case("min");
    let is_clamp = name.eq_ignore_ascii_case("clamp");
    if !(is_min || is_clamp || name.eq_ignore_ascii_case("max")) {
        return Err(parser.new_custom_error(()));
    }
    parser.parse_nested_block(|block| {
        let mut operands = block
            .parse_comma_separated(|operand| parse_sum(operand, depth))?
            .into_iter();
        let first = operands.next().ok_or_else(|| block.new_custom_error(()))?;
        if is_clamp {
            // `clamp(MIN, VAL, MAX)` is `max(MIN, min(VAL, MAX))`, so a MIN
            // above MAX wins (CSS Values 4 §10.2).
            let (Some(value), Some(upper), None) =
                (operands.next(), operands.next(), operands.next())
            else {
                return Err(block.new_custom_error(()));
            };
            return Ok(first.max(value.min(upper)));
        }
        Ok(operands.fold(first, |best, operand| {
            if is_min {
                best.min(operand)
            } else {
                best.max(operand)
            }
        }))
    })
}

/// `<calc-sum>` restricted to percentages: terms joined by `+` and `-`, which
/// need whitespace around them (CSS Values 4 §10.1).
fn parse_sum<'i>(
    parser: &mut Parser<'i, '_>,
    depth: usize,
) -> Result<SignedPercentage, ParseError<'i, ()>> {
    let mut total = parse_term(parser, depth)?;
    while let Ok(sign) = parser.try_parse(parse_sum_operator) {
        let term = parse_term(parser, depth)?;
        total = total
            .add_scaled(term, sign)
            .ok_or_else(|| parser.new_custom_error(()))?;
    }
    Ok(total)
}

/// The `+` or `-` between two terms of a sum, as the sign it gives the term
/// after it.
fn parse_sum_operator<'i>(parser: &mut Parser<'i, '_>) -> Result<f32, ParseError<'i, ()>> {
    let token = parser.next()?.clone();
    match token {
        Token::Delim('+') => Ok(1.0),
        Token::Delim('-') => Ok(-1.0),
        token => Err(parser.new_unexpected_token_error(token)),
    }
}

/// One operand of a sum: a percentage, a parenthesised sum, or a nested math
/// function.
fn parse_term<'i>(
    parser: &mut Parser<'i, '_>,
    depth: usize,
) -> Result<SignedPercentage, ParseError<'i, ()>> {
    if depth > MAX_DEFERRED_VALUE_NESTING_DEPTH {
        return Err(parser.new_custom_error(()));
    }
    match next_numeric_stable(parser)? {
        Token::Percentage { unit_value, .. } => {
            let percent = unit_value * 100.0;
            if percent.is_finite() {
                Ok(SignedPercentage::constant(percent))
            } else {
                Err(parser.new_custom_error(()))
            }
        }
        Token::ParenthesisBlock => parser.parse_nested_block(|block| parse_sum(block, depth + 1)),
        Token::Function(name) => parse_math_function(parser, &name, depth),
        token => Err(parser.new_unexpected_token_error(token)),
    }
}
