//! Numeric grammar used to retain valid, unevaluated paper media features.

use cssparser::{ParseError, Parser, ParserInput, Token};

use super::{
    ColorMathType, MAX_DEFERRED_VALUE_NESTING_DEPTH, color_math_dimension_type, is_math_constant,
};

type MathResult<'i, T> = Result<T, ParseError<'i, ()>>;

/// Numeric types accepted by paper-dimension feature values.
#[derive(Clone, Copy)]
pub(crate) enum MediaNumericType {
    Number,
    Length,
}

// Exponents for length, percentage, angle, time, frequency, resolution, flex.
// Retaining intermediate powers permits valid dimensional cancellation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct NumericType([i32; 7]);

impl NumericType {
    const NUMBER: Self = Self([0; 7]);

    const fn dimension(index: usize) -> Self {
        let mut powers = [0; 7];
        powers[index] = 1;
        Self(powers)
    }

    fn product(mut self, other: Self, operator: char) -> Option<Self> {
        for (power, right) in self.0.iter_mut().zip(other.0) {
            *power = if operator == '*' {
                power.checked_add(right)
            } else {
                power.checked_sub(right)
            }?;
        }
        Some(self)
    }
}

/// Validate complete math grammar and numeric type for a media feature.
/// This separate parser exposes strict feature validation without changing
/// the existing property parser's deferred color-math approximations.
pub(crate) fn math_value_has_type(source: &str, expected: MediaNumericType) -> bool {
    let expected = match expected {
        MediaNumericType::Number => NumericType::NUMBER,
        MediaNumericType::Length => NumericType::dimension(0),
    };
    let mut parser_input = ParserInput::new(source);
    let mut parser = Parser::new(&mut parser_input);
    parser
        .parse_entirely(|input| parse_sum(input, 0))
        .is_ok_and(|kind| kind == expected)
}

fn parse_sum<'i>(input: &mut Parser<'i, '_>, depth: usize) -> MathResult<'i, NumericType> {
    let mut kind = parse_product(input, depth)?;
    while input.try_parse(sum_operator).is_ok() {
        let right = parse_product(input, depth)?;
        if right != kind {
            return Err(input.new_custom_error(()));
        }
        kind = right;
    }
    Ok(kind)
}

// CSS Values 4 requires whitespace on both sides of binary + and -.
fn sum_operator<'i>(input: &mut Parser<'i, '_>) -> MathResult<'i, ()> {
    let mut space = false;
    loop {
        match input.next_including_whitespace_and_comments()? {
            Token::WhiteSpace(_) => space = true,
            Token::Comment(_) => {}
            Token::Delim('+' | '-') if space => break,
            _ => return Err(input.new_custom_error(())),
        }
    }
    loop {
        match input.next_including_whitespace_and_comments()? {
            Token::WhiteSpace(_) => return Ok(()),
            Token::Comment(_) => {}
            _ => return Err(input.new_custom_error(())),
        }
    }
}

fn parse_product<'i>(input: &mut Parser<'i, '_>, depth: usize) -> MathResult<'i, NumericType> {
    let mut kind = parse_value(input, depth)?;
    while let Ok(operator) = input.try_parse(|input| match input.next()? {
        Token::Delim(operator @ ('*' | '/')) => Ok::<_, ParseError<'i, ()>>(*operator),
        _ => Err(input.new_custom_error(())),
    }) {
        let right = parse_value(input, depth)?;
        kind = kind
            .product(right, operator)
            .ok_or_else(|| input.new_custom_error(()))?;
    }
    Ok(kind)
}

fn parse_value<'i>(input: &mut Parser<'i, '_>, depth: usize) -> MathResult<'i, NumericType> {
    if depth > MAX_DEFERRED_VALUE_NESTING_DEPTH {
        return Err(input.new_custom_error(()));
    }
    match input.next()?.clone() {
        Token::Number { .. } => Ok(NumericType::NUMBER),
        Token::Percentage { .. } => Ok(NumericType::dimension(1)),
        Token::Dimension { unit, .. } => {
            let index = if color_math_dimension_type(&unit) == ColorMathType::Length {
                0
            } else {
                match unit.to_ascii_lowercase().as_str() {
                    "deg" | "grad" | "rad" | "turn" => 2,
                    "s" | "ms" => 3,
                    "hz" | "khz" => 4,
                    "dpi" | "dpcm" | "dppx" | "x" => 5,
                    "fr" => 6,
                    _ => return Err(input.new_custom_error(())),
                }
            };
            Ok(NumericType::dimension(index))
        }
        Token::Ident(name) if is_math_constant(&name) => Ok(NumericType::NUMBER),
        Token::ParenthesisBlock => input.parse_nested_block(|input| parse_sum(input, depth + 1)),
        Token::Function(name) => {
            input.parse_nested_block(|input| parse_function(input, &name, depth + 1))
        }
        _ => Err(input.new_custom_error(())),
    }
}

fn parse_function<'i>(
    input: &mut Parser<'i, '_>,
    name: &str,
    depth: usize,
) -> MathResult<'i, NumericType> {
    let name = name.to_ascii_lowercase();
    if name == "round" {
        let _ = input.try_parse(|input| {
            let strategy = input.expect_ident()?;
            if !matches!(
                strategy.to_ascii_lowercase().as_str(),
                "nearest" | "up" | "down" | "to-zero"
            ) {
                return Err(input.new_custom_error(()));
            }
            input.expect_comma()?;
            Ok::<_, ParseError<'i, ()>>(())
        });
    }
    let mut count = 0;
    let arguments = input.parse_comma_separated(|input| {
        // Support more than the required 32 arguments with bounded storage.
        if count >= 128 {
            return Err(input.new_custom_error(()));
        }
        count += 1;
        if name == "clamp"
            && input
                .try_parse(|input| {
                    input.parse_entirely(|input| {
                        input.expect_ident_matching("none")?;
                        Ok::<_, ParseError<'i, ()>>(())
                    })
                })
                .is_ok()
        {
            Ok(None)
        } else {
            parse_sum(input, depth).map(Some)
        }
    })?;
    if name == "clamp" {
        if let [minimum, Some(center), maximum] = arguments.as_slice()
            && minimum.is_none_or(|kind| kind == *center)
            && maximum.is_none_or(|kind| kind == *center)
        {
            return Ok(*center);
        }
        return Err(input.new_custom_error(()));
    }
    let first = arguments
        .first()
        .copied()
        .flatten()
        .ok_or_else(|| input.new_custom_error(()))?;
    let consistent = arguments.iter().all(|kind| *kind == Some(first));
    let number = arguments
        .iter()
        .all(|kind| *kind == Some(NumericType::NUMBER));
    let arity = arguments.len();
    let kind = match name.as_str() {
        "calc" | "abs" if arity == 1 => first,
        "min" | "max" | "hypot" if consistent => first,
        "round" if consistent && (arity == 2 || (arity == 1 && first == NumericType::NUMBER)) => {
            first
        }
        "mod" | "rem" if arity == 2 && consistent => first,
        "sign" if arity == 1 => NumericType::NUMBER,
        "pow" if arity == 2 && number => NumericType::NUMBER,
        "sqrt" | "exp" if arity == 1 && number => NumericType::NUMBER,
        "log" if (1..=2).contains(&arity) && number => NumericType::NUMBER,
        "sin" | "cos" | "tan" if arity == 1 && (number || first == NumericType::dimension(2)) => {
            NumericType::NUMBER
        }
        "asin" | "acos" | "atan" if arity == 1 && number => NumericType::dimension(2),
        "atan2" if arity == 2 && consistent => NumericType::dimension(2),
        _ => return Err(input.new_custom_error(())),
    };
    Ok(kind)
}

#[cfg(test)]
mod tests;
