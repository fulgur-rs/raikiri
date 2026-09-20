use cssparser::{Parser, Token};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CalcUnitKind {
    Number,
    Percentage,
    Angle,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum CalcNode {
    Number(f64),
    Percentage(f64),
    Angle(f64),
    Infinity,
    NegInfinity,
    Nan,
    Sum(Box<CalcNode>, Box<CalcNode>, bool),
    Product(Box<CalcNode>, Box<CalcNode>, bool),
    Sign(Box<CalcNode>),
    Unresolved(String),
}

fn angle_to_degrees(value: f64, unit: &str) -> Option<f64> {
    match unit.to_ascii_lowercase().as_str() {
        "deg" => Some(value),
        "rad" => Some(value * 180.0 / std::f64::consts::PI),
        "grad" => Some(value * 0.9),
        "turn" => Some(value * 360.0),
        _ => None,
    }
}

/// Parses one `<calc-value>`: a leaf (number/percentage/angle/infinity/NaN/
/// sign()/an unresolved dimension) or a parenthesized sub-expression.
fn parse_calc_value(input: &mut Parser<'_, '_>, unit_kind: CalcUnitKind) -> Option<CalcNode> {
    let start = input.position();
    let token = input.next().ok()?.clone();
    match token {
        Token::Number { value, .. } if unit_kind == CalcUnitKind::Number => {
            Some(CalcNode::Number(f64::from(value)))
        }
        Token::Percentage { unit_value, .. } if unit_kind == CalcUnitKind::Percentage => {
            Some(CalcNode::Percentage(f64::from(unit_value) * 100.0))
        }
        Token::Dimension {
            value, ref unit, ..
        } if unit_kind == CalcUnitKind::Angle => {
            angle_to_degrees(f64::from(value), unit.as_ref()).map(CalcNode::Angle)
        }
        Token::Dimension { .. } | Token::Number { .. } | Token::Percentage { .. } => {
            // A dimension/number/percentage that doesn't match the
            // expected unit_kind (e.g. a length like `1em` inside a
            // number-typed calc()) — not evaluable here, but its own CSS
            // text is still valid output verbatim.
            Some(CalcNode::Unresolved(input.slice_from(start).to_owned()))
        }
        Token::Ident(ref name) if name.eq_ignore_ascii_case("infinity") => Some(CalcNode::Infinity),
        // `-infinity` is a valid CSS <ident-token> in its own right (CSS
        // identifiers may start with `-`), so it tokenizes as one Ident,
        // not a Delim('-') followed by a separate Ident("infinity").
        Token::Ident(ref name) if name.eq_ignore_ascii_case("-infinity") => {
            Some(CalcNode::NegInfinity)
        }
        Token::Ident(ref name) if name.eq_ignore_ascii_case("nan") => Some(CalcNode::Nan),
        Token::Function(ref name) if name.eq_ignore_ascii_case("sign") => {
            let inner = input
                .parse_nested_block(
                    |nested| -> Result<CalcNode, cssparser::ParseError<'_, ()>> {
                        parse_calc_sum(nested, CalcUnitKind::Number)
                            .ok_or_else(|| nested.new_custom_error(()))
                    },
                )
                .ok()?;
            Some(CalcNode::Sign(Box::new(inner)))
        }
        Token::Function(ref name) if name.eq_ignore_ascii_case("calc") => input
            .parse_nested_block(
                |nested| -> Result<CalcNode, cssparser::ParseError<'_, ()>> {
                    parse_calc_sum(nested, unit_kind).ok_or_else(|| nested.new_custom_error(()))
                },
            )
            .ok(),
        Token::ParenthesisBlock => input
            .parse_nested_block(
                |nested| -> Result<CalcNode, cssparser::ParseError<'_, ()>> {
                    parse_calc_sum(nested, unit_kind).ok_or_else(|| nested.new_custom_error(()))
                },
            )
            .ok(),
        _ => None,
    }
}

/// `<calc-product> = <calc-value> [ [ '*' | '/' ] <calc-value> ]*`
fn parse_calc_product(input: &mut Parser<'_, '_>, unit_kind: CalcUnitKind) -> Option<CalcNode> {
    let mut node = parse_calc_value(input, unit_kind)?;
    loop {
        input.skip_whitespace();
        let is_multiply = if input.try_parse(|i| i.expect_delim('*')).is_ok() {
            true
        } else if input.try_parse(|i| i.expect_delim('/')).is_ok() {
            false
        } else {
            break;
        };
        // The right-hand side of `*`/`/` in this grammar subset is always
        // a plain number (e.g. `sign(...) * 10`) — CSS calc()
        // multiplication requires exactly one side to be a <number>.
        let rhs = parse_calc_value(input, CalcUnitKind::Number)?;
        node = CalcNode::Product(Box::new(node), Box::new(rhs), is_multiply);
    }
    Some(node)
}

/// `<calc-sum> = <calc-product> [ [ '+' | '-' ] <calc-product> ]*`
fn parse_calc_sum(input: &mut Parser<'_, '_>, unit_kind: CalcUnitKind) -> Option<CalcNode> {
    let mut node = parse_calc_product(input, unit_kind)?;
    loop {
        input.skip_whitespace();
        let is_add = if input.try_parse(|i| i.expect_delim('+')).is_ok() {
            true
        } else if input.try_parse(|i| i.expect_delim('-')).is_ok() {
            false
        } else {
            break;
        };
        let rhs = parse_calc_product(input, unit_kind)?;
        node = CalcNode::Sum(Box::new(node), Box::new(rhs), is_add);
    }
    Some(node)
}

/// `parse_calc_value` already dispatches on both a bare value and a
/// `calc(...)`-wrapped expression (see its `Token::Function` arm for
/// `"calc"`), so this is the entire top-level entry point.
pub(crate) fn parse_calc_or_plain(
    input: &mut Parser<'_, '_>,
    unit_kind: CalcUnitKind,
) -> Option<CalcNode> {
    parse_calc_value(input, unit_kind)
}

#[cfg(test)]
mod tests {
    use cssparser::ParserInput;

    use super::*;

    fn parse(source: &str, unit_kind: CalcUnitKind) -> Option<CalcNode> {
        let mut input = ParserInput::new(source);
        let mut parser = Parser::new(&mut input);
        parse_calc_or_plain(&mut parser, unit_kind)
    }

    #[test]
    fn parses_a_bare_number() {
        assert!(matches!(
            parse("50", CalcUnitKind::Number),
            Some(CalcNode::Number(v)) if v == 50.0
        ));
    }

    #[test]
    fn parses_a_bare_percentage() {
        assert!(matches!(
            parse("50%", CalcUnitKind::Percentage),
            Some(CalcNode::Percentage(v)) if v == 50.0
        ));
    }

    #[test]
    fn parses_a_bare_angle_and_normalizes_units_to_degrees() {
        assert!(matches!(
            parse("380deg", CalcUnitKind::Angle),
            Some(CalcNode::Angle(v)) if (v - 380.0).abs() < 1e-9
        ));
        assert!(matches!(
            parse("1.28rad", CalcUnitKind::Angle),
            Some(CalcNode::Angle(v)) if (v - 73.3386).abs() < 1e-3
        ));
    }

    #[test]
    fn parses_calc_with_a_simple_product() {
        let node = parse("calc(50 * 3)", CalcUnitKind::Number).unwrap();
        assert!(matches!(node, CalcNode::Product(..)));
    }

    #[test]
    fn parses_calc_with_a_sign_function() {
        let node = parse("calc(sign(1em - 10px) * 10)", CalcUnitKind::Number).unwrap();
        let CalcNode::Product(left, right, true) = node else {
            panic!("expected a multiplication product");
        };
        assert!(matches!(*left, CalcNode::Sign(_)));
        assert!(matches!(*right, CalcNode::Number(v) if v == 10.0));
    }

    #[test]
    fn parses_infinity_and_nan_keywords() {
        assert!(matches!(
            parse("calc(infinity)", CalcUnitKind::Number),
            Some(CalcNode::Infinity)
        ));
        assert!(matches!(
            parse("calc(-infinity)", CalcUnitKind::Number),
            Some(CalcNode::NegInfinity)
        ));
        assert!(matches!(
            parse("calc(NaN)", CalcUnitKind::Number),
            Some(CalcNode::Nan)
        ));
    }

    #[test]
    fn returns_none_for_unsupported_math_functions() {
        assert!(parse("calc(min(1, 2))", CalcUnitKind::Number).is_none());
    }
}
