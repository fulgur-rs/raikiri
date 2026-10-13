//! Canonical serialization of `calc()` sums of lengths and percentages from
//! their source text.
//!
//! A `calc()` that mixes units the cascade cannot combine (such as
//! `calc(10px + 0.5em)`) is kept as source text until computed-value time, so
//! CSSOM would otherwise read it back as written. CSS Values 4 §10.12
//! "Simplification" and §10.13 "Serialization"
//! (<https://www.w3.org/TR/css-values-4/#calc-simplification>) combine the
//! terms that share a unit, convert absolute lengths to `px`, and write the
//! percentage first, then the dimensions sorted by unit. Units that cancel in
//! a product (`1px / 1px`) drop out, and `min()` / `max()` / `clamp()` keep
//! their arguments as calculations of their own, combining those that share a
//! unit.

use cssparser::{Delimiter, ParseError, Parser, ParserInput, Token};

use super::calc_serialize::format_css_number;
use super::css_component_values_are_bounded;
use super::parse::{next_numeric_stable, parse_length_value};

/// How deeply nested parentheses and `calc()` are followed.
const MAX_SUM_NESTING: usize = 32;

/// `raw_value` with every `calc()`, `min()`, `max()` or `clamp()` over lengths
/// and percentages rewritten in its canonical form, including those inside other functions
/// such as `translateX()`, or `None` when there is no
/// such `calc()` (callers keep the source text).
pub fn serialize_math_sums(raw_value: &str) -> Option<String> {
    if !css_component_values_are_bounded(raw_value) {
        return None;
    }
    let mut input = ParserInput::new(raw_value);
    let mut parser = Parser::new(&mut input);
    let mut rewrite = Rewrite {
        source: raw_value,
        out: String::with_capacity(raw_value.len()),
        copied_up_to: 0,
        changed: false,
    };
    rewrite.walk(&mut parser, 0);
    let Rewrite {
        mut out,
        copied_up_to,
        changed,
        ..
    } = rewrite;
    if !changed {
        return None;
    }
    out.push_str(&raw_value[copied_up_to..]);
    Some(out.trim().to_owned())
}

/// Functions whose arguments are substituted before the value is parsed.
const SUBSTITUTION_FUNCTIONS: [&str; 5] = ["var", "env", "attr", "if", "inherit"];

/// Math functions other than `calc()` (CSS Values 4 §10).
const MATH_FUNCTIONS: [&str; 21] = [
    "min", "max", "clamp", "round", "mod", "rem", "sin", "cos", "tan", "asin", "acos", "atan",
    "atan2", "pow", "sqrt", "hypot", "log", "exp", "abs", "sign", "progress",
];

/// The source text being copied with its `calc()` sums rewritten.
struct Rewrite<'s> {
    source: &'s str,
    out: String,
    /// The end of the source text already copied into `out`.
    copied_up_to: usize,
    changed: bool,
}

impl Rewrite<'_> {
    /// Rewrites the `calc()` sums among the component values of `parser`,
    /// following other functions and blocks down to [`MAX_SUM_NESTING`].
    /// `min()`, `max()` and `clamp()` are simplified like `calc()`.
    /// Substitution functions are left as written, since their arguments are
    /// not values of the property, and so are the other math functions, whose
    /// canonical form folds a nested `calc()` into their own calculation.
    fn walk(&mut self, parser: &mut Parser<'_, '_>, nesting: usize) {
        loop {
            parser.skip_whitespace();
            let start = parser.position().byte_index();
            let Ok(token) = parser.next() else {
                break;
            };
            let name = match token {
                Token::Function(name) => Some(name.clone()),
                Token::ParenthesisBlock | Token::SquareBracketBlock | Token::CurlyBracketBlock => {
                    None
                }
                _ => continue,
            };
            let root = name.as_ref().and_then(|name| {
                let lower = name.to_ascii_lowercase();
                (lower == "calc" || MathFunction::from_name(&lower).is_some()).then_some(lower)
            });
            if let Some(root) = root {
                let canonical = parser
                    .parse_nested_block(|block| {
                        let node = parse_function_body(&root, block, 0)
                            .ok_or_else(|| block.new_custom_error(()))?;
                        block.expect_exhausted()?;
                        Ok::<_, ParseError<'_, ()>>(node)
                    })
                    .ok()
                    .and_then(|node| node.serialize_root());
                if let Some(canonical) = canonical {
                    self.out.push_str(&self.source[self.copied_up_to..start]);
                    self.out.push_str(&canonical);
                    self.copied_up_to = parser.position().byte_index();
                    self.changed = true;
                }
                continue;
            }
            let opaque = name.as_ref().is_some_and(|name| {
                SUBSTITUTION_FUNCTIONS
                    .iter()
                    .chain(&MATH_FUNCTIONS)
                    .any(|function| name.eq_ignore_ascii_case(function))
            });
            if opaque || nesting >= MAX_SUM_NESTING {
                continue;
            }
            let _ = parser.parse_nested_block(|block| {
                self.walk(block, nesting + 1);
                Ok::<_, ParseError<'_, ()>>(())
            });
        }
    }
}

/// Whether `unit` (ASCII lowercase) is a length unit values accept.
fn is_length_unit(unit: &str) -> bool {
    let text = format!("1{unit}");
    let mut input = ParserInput::new(&text);
    let mut parser = Parser::new(&mut input);
    parse_length_value(&mut parser, false).is_some() && parser.is_exhausted()
}

/// The units of one term with their exponents, sorted by unit with no zero
/// exponent: empty for a plain number, `[("%", 1)]` for a percentage, and an
/// ASCII-lowercase length unit otherwise. Only identical units cancel, since
/// relative lengths cannot be converted before computed-value time.
type Units = Vec<(String, i32)>;

fn unit_of(unit: String) -> Units {
    vec![(unit, 1)]
}

/// The product of two terms' units.
fn multiply_units(mut left: Units, right: &Units, sign: i32) -> Units {
    for (unit, power) in right {
        match left.iter_mut().find(|(existing, _)| existing == unit) {
            Some((_, existing)) => *existing += sign * power,
            None => left.push((unit.clone(), sign * power)),
        }
    }
    left.retain(|(_, power)| *power != 0);
    left.sort();
    left
}

/// The unit a term is written with: `""` for a plain number, or `None` for a
/// product of units that no single unit spells.
fn simple_unit(units: &Units) -> Option<&str> {
    match units.as_slice() {
        [] => Some(""),
        [(unit, 1)] => Some(unit),
        _ => None,
    }
}

/// The terms of a sum, one coefficient per unit.
#[derive(Debug, Default)]
struct Terms(Vec<(Units, f32)>);

impl Terms {
    fn single(units: Units, value: f32) -> Self {
        Self(vec![(units, value)])
    }

    fn is_number(&self) -> bool {
        self.0.iter().all(|(units, _)| units.is_empty())
    }

    fn number(&self) -> Option<f32> {
        match self.0.as_slice() {
            [(units, value)] if units.is_empty() => Some(*value),
            _ => None,
        }
    }

    /// The only term, when there is exactly one.
    fn monomial(&self) -> Option<(&Units, f32)> {
        match self.0.as_slice() {
            [(units, value)] => Some((units, *value)),
            _ => None,
        }
    }

    /// The only term and its unit, when it is written with one unit.
    fn simple_monomial(&self) -> Option<(&str, f32)> {
        let (units, value) = self.monomial()?;
        Some((simple_unit(units)?, value))
    }

    /// Whether every term is written with one unit, as a sum's terms must be
    /// once a product is complete.
    fn is_simple(&self) -> bool {
        self.0.iter().all(|(units, _)| simple_unit(units).is_some())
    }

    /// Adds `sign * other`. A plain number cannot be added to a length.
    fn add(mut self, other: Self, sign: f32) -> Option<Self> {
        if self.is_number() != other.is_number() {
            return None;
        }
        for (units, value) in other.0 {
            let value = sign * value;
            match self.0.iter_mut().find(|(existing, _)| *existing == units) {
                Some((_, existing)) => *existing += value,
                None => self.0.push((units, value)),
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

    /// `self * other`, or `self / other` when `divide`. A sum of several
    /// terms only scales by a plain number; two single terms multiply their
    /// units, so `1px / 1px` is the number `1` (CSS Values 4 §10.12).
    fn multiply(self, other: Self, divide: bool) -> Option<Self> {
        if divide {
            let (units, divisor) = other.monomial()?;
            if divisor == 0.0 {
                return None;
            }
            if units.is_empty() {
                return self.scale(1.0 / divisor);
            }
            let (left_units, value) = self.monomial()?;
            let value = value / divisor;
            return value
                .is_finite()
                .then(|| Self::single(multiply_units(left_units.clone(), units, -1), value));
        }
        if let Some(factor) = other.number() {
            return self.scale(factor);
        }
        if let Some(factor) = self.number() {
            return other.scale(factor);
        }
        let (left_units, left) = self.monomial()?;
        let (right_units, right) = other.monomial()?;
        let value = left * right;
        value
            .is_finite()
            .then(|| Self::single(multiply_units(left_units.clone(), right_units, 1), value))
    }

    /// The terms in canonical order joined by `+` / `-`, without a `calc()`
    /// wrapper.
    fn expression(mut self) -> Option<String> {
        // Terms that cancel out stay as zero: a unit's presence can change
        // behavior, so only like units combine (CSS Values 4 §10.12).
        let mut terms = Vec::with_capacity(self.0.len());
        for (units, value) in self.0.drain(..) {
            terms.push((simple_unit(&units)?.to_owned(), value));
        }
        terms.sort_by(|(left, _), (right, _)| {
            (left != "%", left.as_str()).cmp(&(right != "%", right.as_str()))
        });
        let mut expression = String::new();
        for (index, (unit, value)) in terms.iter().enumerate() {
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
        Some(expression)
    }
}

/// A comparison math function that keeps its arguments as calculations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MathFunction {
    Min,
    Max,
    Clamp,
}

impl MathFunction {
    fn from_name(lower: &str) -> Option<Self> {
        Some(match lower {
            "min" => Self::Min,
            "max" => Self::Max,
            "clamp" => Self::Clamp,
            _ => return None,
        })
    }

    fn name(self) -> &'static str {
        match self {
            Self::Min => "min",
            Self::Max => "max",
            Self::Clamp => "clamp",
        }
    }
}

/// A simplified calculation: a sum, or a comparison function whose
/// arguments could not all be combined.
#[derive(Debug)]
enum Node {
    Sum(Terms),
    Function(MathFunction, Vec<Node>),
}

impl Node {
    /// The only term and its unit, for a sum of one term written with one
    /// unit.
    fn simple_monomial(&self) -> Option<(&str, f32)> {
        match self {
            Self::Sum(terms) => terms.simple_monomial(),
            Self::Function(..) => None,
        }
    }

    fn is_number(&self) -> bool {
        match self {
            Self::Sum(terms) => terms.is_number(),
            Self::Function(_, args) => args.first().is_some_and(Node::is_number),
        }
    }

    /// The canonical text of a whole math function: a sum keeps a `calc()`
    /// wrapper, and a comparison function is written as itself (CSS Values 4
    /// §10.13). `None` for a plain number, which other serializers own.
    fn serialize_root(self) -> Option<String> {
        if self.is_number() {
            return None;
        }
        match self {
            Self::Sum(terms) => Some(format!("calc({})", terms.expression()?)),
            function => function.serialize_argument(),
        }
    }

    /// The canonical text of a calculation nested in a comparison function,
    /// where a sum needs no `calc()` wrapper.
    fn serialize_argument(self) -> Option<String> {
        match self {
            Self::Sum(terms) => terms.expression(),
            Self::Function(function, args) => {
                let args = args
                    .into_iter()
                    .map(Node::serialize_argument)
                    .collect::<Option<Vec<_>>>()?;
                Some(format!("{}({})", function.name(), args.join(", ")))
            }
        }
    }
}

/// Simplifies a comparison function: arguments that are single terms of the
/// same unit combine, and a function left with one argument is replaced by
/// it (CSS Values 4 §10.12).
fn simplify_function(function: MathFunction, args: Vec<Node>) -> Option<Node> {
    let number = args.first()?.is_number();
    if args.iter().any(|arg| arg.is_number() != number) {
        return None;
    }
    if function == MathFunction::Clamp {
        let [min, value, max] = args.as_slice() else {
            return None;
        };
        if let (Some((unit, min)), Some((value_unit, value)), Some((max_unit, max))) = (
            min.simple_monomial(),
            value.simple_monomial(),
            max.simple_monomial(),
        ) && unit == value_unit
            && unit == max_unit
        {
            let units = if unit.is_empty() {
                Units::new()
            } else {
                unit_of(unit.to_owned())
            };
            return Some(Node::Sum(Terms::single(units, value.min(max).max(min))));
        }
        return Some(Node::Function(function, args));
    }
    let mut kept: Vec<Node> = Vec::with_capacity(args.len());
    for arg in args {
        let simple = arg
            .simple_monomial()
            .map(|(unit, value)| (unit.to_owned(), value));
        let Some((unit, value)) = simple else {
            kept.push(arg);
            continue;
        };
        let same_unit = kept.iter_mut().find_map(|existing| match existing {
            Node::Sum(terms) => match terms.0.as_mut_slice() {
                [(units, existing)] if simple_unit(units) == Some(unit.as_str()) => Some(existing),
                _ => None,
            },
            Node::Function(..) => None,
        });
        match same_unit {
            Some(existing) if function == MathFunction::Min => *existing = existing.min(value),
            Some(existing) => *existing = existing.max(value),
            None => kept.push(arg),
        }
    }
    if kept.len() == 1 {
        return kept.pop();
    }
    Some(Node::Function(function, kept))
}

/// The contents of `calc()` (`lower == "calc"`) or of a comparison function.
fn parse_function_body(lower: &str, input: &mut Parser<'_, '_>, nesting: usize) -> Option<Node> {
    if lower == "calc" {
        return parse_sum(input, nesting);
    }
    let function = MathFunction::from_name(lower)?;
    let mut args = Vec::new();
    loop {
        let arg = input
            .parse_until_before(Delimiter::Comma, |arg| {
                let node = parse_sum(arg, nesting).ok_or_else(|| arg.new_custom_error(()))?;
                arg.expect_exhausted()?;
                Ok::<_, ParseError<'_, ()>>(node)
            })
            .ok()?;
        args.push(arg);
        if input.is_exhausted() {
            break;
        }
        input.expect_comma().ok()?;
    }
    if function == MathFunction::Clamp && args.len() != 3 {
        return None;
    }
    simplify_function(function, args)
}

/// `<calc-sum>`: products joined by `+` / `-`. A comparison function only
/// stands alone; one combined with other terms is not folded here.
fn parse_sum(input: &mut Parser<'_, '_>, nesting: usize) -> Option<Node> {
    let first = parse_product(input, nesting)?;
    if input.is_exhausted() {
        return Some(first);
    }
    let Node::Sum(mut sum) = first else {
        return None;
    };
    while !input.is_exhausted() {
        let sign = match input.next().ok()? {
            Token::Delim('+') => 1.0,
            Token::Delim('-') => -1.0,
            _ => return None,
        };
        let Node::Sum(terms) = parse_product(input, nesting)? else {
            return None;
        };
        sum = sum.add(terms, sign)?;
    }
    Some(Node::Sum(sum))
}

/// `<calc-product>`: values joined by `*` / `/`. Units may cancel along the
/// way, but the finished product must be written with one unit.
fn parse_product(input: &mut Parser<'_, '_>, nesting: usize) -> Option<Node> {
    let mut product = parse_value(input, nesting)?;
    loop {
        let operator = input.try_parse(|input| match input.next() {
            Ok(Token::Delim('*')) => Ok(true),
            Ok(Token::Delim('/')) => Ok(false),
            _ => Err(()),
        });
        let Ok(multiply) = operator else {
            break;
        };
        let (Node::Sum(left), Node::Sum(right)) = (product, parse_value(input, nesting)?) else {
            return None;
        };
        product = Node::Sum(left.multiply(right, !multiply)?);
    }
    match &product {
        Node::Sum(terms) if !terms.is_simple() => None,
        _ => Some(product),
    }
}

/// `<calc-value>`: a number, a percentage, a length, or a parenthesized,
/// `calc()` or comparison-function calculation.
fn parse_value(input: &mut Parser<'_, '_>, nesting: usize) -> Option<Node> {
    let terms = match next_numeric_stable(input).ok()? {
        Token::Number { value, .. } => Terms::single(Units::new(), value),
        Token::Percentage { unit_value, .. } => {
            Terms::single(unit_of("%".to_owned()), unit_value * 100.0)
        }
        Token::Dimension { value, unit, .. } => {
            let unit = unit.to_ascii_lowercase();
            match absolute_length_in_px(&unit) {
                Some(px) => Terms::single(unit_of("px".to_owned()), value * px),
                None if is_length_unit(&unit) => Terms::single(unit_of(unit), value),
                None => return None,
            }
        }
        Token::ParenthesisBlock => return parse_nested(input, "calc", nesting),
        Token::Function(name) => {
            let lower = name.to_ascii_lowercase();
            if lower != "calc" && MathFunction::from_name(&lower).is_none() {
                return None;
            }
            return parse_nested(input, &lower, nesting);
        }
        _ => return None,
    };
    terms
        .0
        .iter()
        .all(|(_, value)| value.is_finite())
        .then_some(Node::Sum(terms))
}

fn parse_nested(input: &mut Parser<'_, '_>, lower: &str, nesting: usize) -> Option<Node> {
    if nesting >= MAX_SUM_NESTING {
        return None;
    }
    input
        .parse_nested_block(|block| {
            let node = parse_function_body(lower, block, nesting + 1)
                .ok_or_else(|| block.new_custom_error(()))?;
            block.expect_exhausted()?;
            Ok::<_, ParseError<'_, ()>>(node)
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
