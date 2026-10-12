//! Typed grammar of CSS math functions (CSS Values 4 §10).
//!
//! One grammar serves every caller that checks a math expression without
//! evaluating it: paper media features, deferred property values and color
//! components. Callers differ only in their numeric type algebra, given by a
//! [`MathTypes`] implementation.

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

/// The numeric type algebra a caller checks math expressions against.
pub(crate) trait MathTypes {
    type Kind: Copy + PartialEq;

    fn number(&self) -> Self::Kind;
    fn length(&self) -> Self::Kind;
    fn angle(&self) -> Self::Kind;
    fn percentage(&self) -> Self::Kind;
    /// The type of a dimension, or `None` for a unit the caller rejects.
    fn dimension(&self, unit: &str) -> Option<Self::Kind>;
    /// The type of an identifier other than a math constant, such as a
    /// relative color channel keyword.
    fn ident(&self, _name: &str) -> Option<Self::Kind> {
        None
    }
    /// The type of an arbitrary substitution function (`var()`, `env()`)
    /// whose value is only known later, or `None` where none is allowed.
    fn substitution(&self) -> Option<Self::Kind> {
        None
    }
    /// The type of `left + right`, or `None` when the types do not match.
    fn add(&self, left: Self::Kind, right: Self::Kind) -> Option<Self::Kind>;
    /// The type of `left * right` or `left / right`.
    fn product(&self, left: Self::Kind, right: Self::Kind, operator: char) -> Option<Self::Kind>;
    fn is_number(&self, kind: Self::Kind) -> bool {
        kind == self.number()
    }
    fn is_angle(&self, kind: Self::Kind) -> bool {
        kind == self.angle()
    }
}

/// Exact CSS Values 4 type checking for media features, which take no
/// percentages and no substitution functions.
struct MediaTypes;

impl MathTypes for MediaTypes {
    type Kind = NumericType;

    fn number(&self) -> NumericType {
        NumericType::NUMBER
    }

    fn length(&self) -> NumericType {
        NumericType::dimension(0)
    }

    fn angle(&self) -> NumericType {
        NumericType::dimension(2)
    }

    fn percentage(&self) -> NumericType {
        NumericType::dimension(1)
    }

    fn dimension(&self, unit: &str) -> Option<NumericType> {
        let index = if color_math_dimension_type(unit) == ColorMathType::Length {
            0
        } else {
            match unit.to_ascii_lowercase().as_str() {
                "deg" | "grad" | "rad" | "turn" => 2,
                "s" | "ms" => 3,
                "hz" | "khz" => 4,
                "dpi" | "dpcm" | "dppx" | "x" => 5,
                "fr" => 6,
                _ => return None,
            }
        };
        Some(NumericType::dimension(index))
    }

    fn add(&self, left: NumericType, right: NumericType) -> Option<NumericType> {
        (left == right).then_some(left)
    }

    fn product(
        &self,
        left: NumericType,
        right: NumericType,
        operator: char,
    ) -> Option<NumericType> {
        left.product(right, operator)
    }
}

/// Validate complete math grammar and numeric type for a media feature.
pub(crate) fn math_value_has_type(source: &str, expected: MediaNumericType) -> bool {
    let expected = match expected {
        MediaNumericType::Number => NumericType::NUMBER,
        MediaNumericType::Length => NumericType::dimension(0),
    };
    let mut parser_input = ParserInput::new(source);
    let mut parser = Parser::new(&mut parser_input);
    parser
        .parse_entirely(|input| parse_sum(&MediaTypes, input, 0))
        .is_ok_and(|kind| kind == expected)
}

/// Whether `name` is a math function this grammar knows.
pub(crate) fn is_math_function_name(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "calc"
            | "min"
            | "max"
            | "clamp"
            | "round"
            | "mod"
            | "rem"
            | "abs"
            | "sign"
            | "pow"
            | "sqrt"
            | "hypot"
            | "log"
            | "exp"
            | "sin"
            | "cos"
            | "tan"
            | "asin"
            | "acos"
            | "atan"
            | "atan2"
    )
}

/// Type a math function's arguments, the content of its nested block.
pub(crate) fn parse_math_function<'i, T: MathTypes>(
    types: &T,
    input: &mut Parser<'i, '_>,
    name: &str,
) -> MathResult<'i, T::Kind> {
    parse_function(types, input, name, 1)
}

/// Type a `<calc-sum>`.
pub(crate) fn parse_sum<'i, T: MathTypes>(
    types: &T,
    input: &mut Parser<'i, '_>,
    depth: usize,
) -> MathResult<'i, T::Kind> {
    let mut kind = parse_product(types, input, depth)?;
    while input.try_parse(sum_operator).is_ok() {
        let right = parse_product(types, input, depth)?;
        kind = types
            .add(kind, right)
            .ok_or_else(|| input.new_custom_error(()))?;
    }
    Ok(kind)
}

/// A binary `+` or `-` of a `<calc-sum>`, with the whitespace that CSS
/// Values 4 §10.1 requires on both sides. Comments are not whitespace.
pub(crate) fn sum_operator<'i>(input: &mut Parser<'i, '_>) -> MathResult<'i, char> {
    let mut space = false;
    let operator = loop {
        match input.next_including_whitespace_and_comments()? {
            Token::WhiteSpace(_) => space = true,
            Token::Comment(_) => {}
            Token::Delim(operator @ ('+' | '-')) if space => break *operator,
            _ => return Err(input.new_custom_error(())),
        }
    };
    loop {
        match input.next_including_whitespace_and_comments()? {
            Token::WhiteSpace(_) => return Ok(operator),
            Token::Comment(_) => {}
            _ => return Err(input.new_custom_error(())),
        }
    }
}

/// A `*` or `/` of a `<calc-product>`.
pub(crate) fn product_operator<'i>(input: &mut Parser<'i, '_>) -> MathResult<'i, char> {
    match input.next()? {
        Token::Delim(operator @ ('*' | '/')) => Ok(*operator),
        _ => Err(input.new_custom_error(())),
    }
}

fn parse_product<'i, T: MathTypes>(
    types: &T,
    input: &mut Parser<'i, '_>,
    depth: usize,
) -> MathResult<'i, T::Kind> {
    let mut kind = parse_value(types, input, depth)?;
    while let Ok(operator) = input.try_parse(product_operator) {
        let right = parse_value(types, input, depth)?;
        kind = types
            .product(kind, right, operator)
            .ok_or_else(|| input.new_custom_error(()))?;
    }
    Ok(kind)
}

fn parse_value<'i, T: MathTypes>(
    types: &T,
    input: &mut Parser<'i, '_>,
    depth: usize,
) -> MathResult<'i, T::Kind> {
    if depth > MAX_DEFERRED_VALUE_NESTING_DEPTH {
        return Err(input.new_custom_error(()));
    }
    let kind = match input.next()?.clone() {
        Token::Number { .. } => Some(types.number()),
        Token::Percentage { .. } => Some(types.percentage()),
        Token::Dimension { unit, .. } => types.dimension(&unit),
        Token::Ident(name) if is_math_constant(&name) => Some(types.number()),
        Token::Ident(name) => types.ident(&name),
        Token::ParenthesisBlock => {
            return input.parse_nested_block(|input| parse_sum(types, input, depth + 1));
        }
        Token::Function(name)
            if name.eq_ignore_ascii_case("var") || name.eq_ignore_ascii_case("env") =>
        {
            let kind = types
                .substitution()
                .ok_or_else(|| input.new_custom_error(()))?;
            input.parse_nested_block(|input| {
                while input.next().is_ok() {}
                Ok::<_, ParseError<'i, ()>>(())
            })?;
            Some(kind)
        }
        Token::Function(name) => {
            return input
                .parse_nested_block(|input| parse_function(types, input, &name, depth + 1));
        }
        _ => None,
    };
    kind.ok_or_else(|| input.new_custom_error(()))
}

fn parse_function<'i, T: MathTypes>(
    types: &T,
    input: &mut Parser<'i, '_>,
    name: &str,
    depth: usize,
) -> MathResult<'i, T::Kind> {
    let name = name.to_ascii_lowercase();
    if depth > MAX_DEFERRED_VALUE_NESTING_DEPTH || !is_math_function_name(&name) {
        return Err(input.new_custom_error(()));
    }
    let line_width_rounding = if name == "round" {
        input
            .try_parse(|input| {
                let strategy = input.expect_ident()?;
                let line_width = match strategy.to_ascii_lowercase().as_str() {
                    "line-width" => true,
                    "nearest" | "up" | "down" | "to-zero" => false,
                    _ => return Err(input.new_custom_error(())),
                };
                input.expect_comma()?;
                Ok::<_, ParseError<'i, ()>>(line_width)
            })
            .unwrap_or(false)
    } else {
        false
    };
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
            parse_sum(types, input, depth).map(Some)
        }
    })?;
    let invalid = || input.new_custom_error(());
    if name == "clamp" {
        if let [minimum, Some(center), maximum] = arguments.as_slice() {
            let mut kind = *center;
            for bound in [minimum, maximum].into_iter().flatten() {
                kind = types.add(kind, *bound).ok_or_else(invalid)?;
            }
            return Ok(kind);
        }
        return Err(invalid());
    }
    let arguments = arguments
        .into_iter()
        .collect::<Option<Vec<_>>>()
        .ok_or_else(invalid)?;
    let first = *arguments.first().ok_or_else(invalid)?;
    // The common type of all arguments, when they have one.
    let consistent = arguments
        .iter()
        .try_fold(first, |kind, argument| types.add(kind, *argument));
    let number = arguments.iter().all(|kind| types.is_number(*kind));
    let arity = arguments.len();
    let kind = match name.as_str() {
        "calc" | "abs" if arity == 1 => Some(first),
        "min" | "max" | "hypot" => consistent,
        "round" if line_width_rounding => consistent.filter(|kind| {
            (1..=2).contains(&arity) && types.add(*kind, types.length()) == Some(types.length())
        }),
        "round" if arity == 2 || (arity == 1 && types.is_number(first)) => consistent,
        "mod" | "rem" if arity == 2 => consistent,
        "sign" if arity == 1 => Some(types.number()),
        "pow" if arity == 2 && number => Some(types.number()),
        "sqrt" | "exp" if arity == 1 && number => Some(types.number()),
        "log" if (1..=2).contains(&arity) && number => Some(types.number()),
        "sin" | "cos" | "tan" if arity == 1 && (number || types.is_angle(first)) => {
            Some(types.number())
        }
        "asin" | "acos" | "atan" if arity == 1 && number => Some(types.angle()),
        "atan2" if arity == 2 => consistent.map(|_| types.angle()),
        _ => None,
    };
    kind.ok_or_else(invalid)
}

#[cfg(test)]
mod tests;
