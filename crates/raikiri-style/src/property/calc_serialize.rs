use cssparser::{Parser, ToCss as _, Token};

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
    input.skip_whitespace();
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
        // CSS calc() multiplication only requires that *one* side resolve
        // to a plain `<number>` — the other can keep `unit_kind` (e.g.
        // `sign(1em - 10px) * 10%`, where the percentage is the right-hand
        // side). Try `unit_kind` first, fall back to a plain number.
        let rhs_start = input.state();
        let rhs = match parse_calc_value(input, unit_kind) {
            Some(node) => node,
            None => {
                input.reset(&rhs_start);
                parse_calc_value(input, CalcUnitKind::Number)?
            }
        };
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

/// Duplicates `serialize.rs`'s `serialize_number`/`serialize_percentage`
/// (same `Token::Number`/`Token::Percentage` + `to_css_string()` approach).
/// The duplication is deliberate: this module has no dependency on
/// `serialize.rs` (it depends on nothing but `cssparser`), so it can't
/// import them without inverting that direction. If a third consumer of
/// this exact formatting shows up later, that's the point to factor it
/// out — not before.
fn format_number(value: f64) -> String {
    let value = value as f32;
    let int_value = if value.fract() == 0.0 {
        Some(value as i32)
    } else {
        None
    };
    Token::Number {
        has_sign: false,
        value,
        int_value,
    }
    .to_css_string()
}

fn format_percentage(value: f64) -> String {
    let value = value as f32;
    let int_value = if value.fract() == 0.0 {
        Some(value as i32)
    } else {
        None
    };
    Token::Percentage {
        has_sign: false,
        unit_value: value / 100.0,
        int_value,
    }
    .to_css_string()
}

/// Attempts to fold `node` to a single constant. `None` means some part of
/// the tree is `Unresolved` — `sign()` also can't fold when its argument
/// isn't foldable, since it depends on the runtime sign of a value this
/// module can't compute (e.g. a font-relative length).
fn try_evaluate(node: &CalcNode) -> Option<f64> {
    match node {
        CalcNode::Number(v) | CalcNode::Percentage(v) | CalcNode::Angle(v) => Some(*v),
        CalcNode::Infinity => Some(f64::INFINITY),
        CalcNode::NegInfinity => Some(f64::NEG_INFINITY),
        CalcNode::Nan => Some(f64::NAN),
        CalcNode::Unresolved(_) => None,
        CalcNode::Sign(inner) => {
            let v = try_evaluate(inner)?;
            if v.is_nan() {
                Some(f64::NAN)
            } else if v > 0.0 {
                Some(1.0)
            } else if v < 0.0 {
                Some(-1.0)
            } else {
                Some(0.0)
            }
        }
        CalcNode::Sum(left, right, is_add) => {
            let l = try_evaluate(left)?;
            let r = try_evaluate(right)?;
            Some(if *is_add { l + r } else { l - r })
        }
        CalcNode::Product(left, right, is_multiply) => {
            let l = try_evaluate(left)?;
            let r = try_evaluate(right)?;
            Some(if *is_multiply { l * r } else { l / r })
        }
    }
}

/// Renders a node's own (non-`calc(...)`-wrapped) text — used both inside
/// the final `calc(...)` wrapper and for parenthesized sub-expressions.
fn render(node: &CalcNode) -> String {
    match node {
        CalcNode::Number(v) => format_number(*v),
        CalcNode::Percentage(v) => format_percentage(*v),
        CalcNode::Angle(v) => format_number(*v),
        CalcNode::Infinity => "infinity".to_owned(),
        CalcNode::NegInfinity => "-infinity".to_owned(),
        CalcNode::Nan => "NaN".to_owned(),
        CalcNode::Unresolved(text) => text.clone(),
        CalcNode::Sign(inner) => format!("sign({})", render(inner)),
        CalcNode::Sum(left, right, is_add) => {
            format!(
                "{} {} {}",
                render(left),
                if *is_add { "+" } else { "-" },
                render(right)
            )
        }
        CalcNode::Product(left, right, is_multiply) => {
            // Canonical order: when exactly one side is a plain literal
            // (Number/Percentage) and the other is not, the literal comes
            // first — matches the real corpus's `10 * sign(...)`
            // ordering.
            let left_is_literal =
                matches!(left.as_ref(), CalcNode::Number(_) | CalcNode::Percentage(_));
            let right_is_literal = matches!(
                right.as_ref(),
                CalcNode::Number(_) | CalcNode::Percentage(_)
            );
            let op = if *is_multiply { "*" } else { "/" };
            if right_is_literal && !left_is_literal {
                format!("{} {} {}", render(right), op, render(left))
            } else {
                format!("{} {} {}", render(left), op, render(right))
            }
        }
    }
}

/// Wraps a rendered sub-expression in parens when it's a Sum or Product
/// nested inside another operator — matches the real corpus's
/// `(sign(1em - 10px) * 10%)` parenthesization inside a surrounding `+`.
fn render_parenthesized_if_needed(node: &CalcNode, is_top_level: bool) -> String {
    let rendered = render(node);
    if is_top_level {
        return rendered;
    }
    match node {
        CalcNode::Sum(..) | CalcNode::Product(..) => format!("({rendered})"),
        _ => rendered,
    }
}

/// Serializes a `CalcNode`: if every leaf is a resolved constant, evaluates
/// the whole expression to a single number and returns `calc(<number>)`.
/// Otherwise, returns a `calc(...)` string with operators printed in their
/// canonical order (see `render`'s `Product` arm).
pub(crate) fn serialize_calc_node(node: &CalcNode) -> String {
    if let Some(value) = try_evaluate(node) {
        if value.is_nan() {
            return "calc(NaN)".to_owned();
        }
        if value.is_infinite() {
            return if value > 0.0 {
                "calc(infinity)".to_owned()
            } else {
                "calc(-infinity)".to_owned()
            };
        }
        return match node {
            CalcNode::Percentage(_) => format!("calc({})", format_percentage(value)),
            _ => format!("calc({})", format_number(value)),
        };
    }
    match node {
        CalcNode::Sum(left, right, is_add) => format!(
            "calc({} {} {})",
            render_parenthesized_if_needed(left, true),
            if *is_add { "+" } else { "-" },
            render_parenthesized_if_needed(right, false)
        ),
        _ => format!("calc({})", render(node)),
    }
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

    #[test]
    fn folds_a_fully_constant_product() {
        let node = parse("calc(50 * 3)", CalcUnitKind::Number).unwrap();
        assert_eq!(serialize_calc_node(&node), "calc(150)");
    }

    #[test]
    fn folds_a_fully_constant_difference() {
        let node = parse("calc(0.5 - 1)", CalcUnitKind::Number).unwrap();
        assert_eq!(serialize_calc_node(&node), "calc(-0.5)");
    }

    #[test]
    fn reorders_a_sign_call_multiplied_by_a_literal() {
        let node = parse("calc(sign(1em - 10px) * 10)", CalcUnitKind::Number).unwrap();
        assert_eq!(serialize_calc_node(&node), "calc(10 * sign(1em - 10px))");
    }

    #[test]
    fn reorders_a_percentage_variant_the_same_way() {
        let node = parse("calc(sign(1em - 10px) * 10%)", CalcUnitKind::Percentage).unwrap();
        assert_eq!(serialize_calc_node(&node), "calc(10% * sign(1em - 10px))");
    }

    #[test]
    fn preserves_addition_operand_order_around_an_unresolved_sign_product() {
        let node = parse(
            "calc(50% + (sign(1em - 10px) * 10%))",
            CalcUnitKind::Percentage,
        )
        .unwrap();
        assert_eq!(
            serialize_calc_node(&node),
            "calc(50% + (10% * sign(1em - 10px)))"
        );
    }

    #[test]
    fn serializes_infinity_and_nan() {
        assert_eq!(serialize_calc_node(&CalcNode::Infinity), "calc(infinity)");
        assert_eq!(
            serialize_calc_node(&CalcNode::NegInfinity),
            "calc(-infinity)"
        );
        assert_eq!(serialize_calc_node(&CalcNode::Nan), "calc(NaN)");
    }

    #[test]
    fn zero_divided_by_zero_folds_to_nan() {
        let node = parse("calc(0 / 0)", CalcUnitKind::Number).unwrap();
        assert_eq!(serialize_calc_node(&node), "calc(NaN)");
    }
}
