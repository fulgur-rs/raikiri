//! The math functions of CSS: `calc()`, `min()`, `round()` and the rest.
//!
//! A browser checks a math function when it parses the declaration that holds
//! it: the grammar, and the type of every term, so that a length is not added
//! to a number or to an angle. A declaration that fails the check is dropped.
//!
//! The types follow CSS Values 4, typed arithmetic included: a product or a
//! quotient has the type that the units of its factors make, so
//! `calc(30px * 2px / 1px)` is a length.
//!
//! What a function comes to is worked out along with the check, for the terms
//! that need nothing but themselves: numbers, the absolute units of length and
//! the units of angle. A term that depends on where it is used (a percentage,
//! `em`, `vw`) has no value here, and neither does a function of it, though its
//! type is still checked.

use super::{LENGTH_UNITS, number_len};
use std::f64::consts::{E, PI};

/// The deepest nesting of function and parenthesis blocks that a browser reads
/// in a math function. A value that goes deeper is invalid.
const MAX_DEPTH: usize = 100;

/// The type of a term: the power of each kind of unit that it is made of, for
/// lengths, angles, times, frequencies and resolutions. A number has no unit,
/// so every power is zero. A percentage counts as a length, which is what a
/// property of lengths and percentages makes of it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Dim([i32; 5]);

const NUMBER: Dim = Dim([0; 5]);
const LENGTH: Dim = Dim([1, 0, 0, 0, 0]);
const ANGLE: Dim = Dim([0, 1, 0, 0, 0]);

impl Dim {
    /// The type of a product of terms of types `self` and `other`.
    fn times(self, other: Self) -> Self {
        let mut powers = self.0;
        for (power, extra) in powers.iter_mut().zip(other.0) {
            *power = power.saturating_add(extra);
        }
        Self(powers)
    }

    /// The type of the quotient of a term of type `self` by one of type
    /// `other`.
    fn over(self, other: Self) -> Self {
        let mut powers = self.0;
        for (power, extra) in powers.iter_mut().zip(other.0) {
            *power = power.saturating_sub(extra);
        }
        Self(powers)
    }
}

/// The type of a number that `unit` follows, or `None` for a unit that CSS
/// does not have.
fn unit_type(unit: &str) -> Option<Dim> {
    let is = |known: &[&str]| known.iter().any(|name| unit.eq_ignore_ascii_case(name));
    if unit == "%"
        || LENGTH_UNITS
            .iter()
            .any(|name| unit.eq_ignore_ascii_case(name))
    {
        Some(LENGTH)
    } else if is(&["deg", "grad", "rad", "turn"]) {
        Some(ANGLE)
    } else if is(&["s", "ms"]) {
        Some(Dim([0, 0, 1, 0, 0]))
    } else if is(&["hz", "khz"]) {
        Some(Dim([0, 0, 0, 1, 0]))
    } else if is(&["dpi", "dpcm", "dppx", "x"]) {
        Some(Dim([0, 0, 0, 0, 1]))
    } else {
        None
    }
}

/// How many pixels one `unit` is, or how many degrees: the units that a value
/// is worked out in. Units that depend on where the value is used have none,
/// and neither do the times, the frequencies and the resolutions.
fn unit_scale(unit: &str) -> Option<f64> {
    let scale = match unit.to_ascii_lowercase().as_str() {
        "px" | "deg" => 1.0,
        "in" => 96.0,
        "cm" => 96.0 / 2.54,
        "mm" => 96.0 / 25.4,
        "q" => 96.0 / 101.6,
        "pt" => 96.0 / 72.0,
        "pc" => 16.0,
        "grad" => 0.9,
        "rad" => 180.0 / PI,
        "turn" => 360.0,
        _ => return None,
    };
    Some(scale)
}

/// What a term comes to: its type, and its value in pixels, in degrees or as a
/// plain number, when that can be worked out here.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Term {
    dim: Dim,
    value: Option<f64>,
}

/// The term of type `dim` that has `value`, which has none when it is not a
/// number that a length can be: a division by zero, or a result too large.
fn term(dim: Dim, value: Option<f64>) -> Term {
    Term {
        dim,
        value: value.filter(|value| value.is_finite()),
    }
}

/// The term that a name stands for. A name that is a number but not a finite
/// one is a term without a value.
fn constant(name: &str) -> Option<Term> {
    let is = |known: &str| name.eq_ignore_ascii_case(known);
    if is("e") {
        Some(term(NUMBER, Some(E)))
    } else if is("pi") {
        Some(term(NUMBER, Some(PI)))
    } else if is("infinity") || is("-infinity") || is("nan") {
        Some(term(NUMBER, None))
    } else {
        None
    }
}

/// What a token is. Names and the end of the input are all that a token keeps
/// of its text, and a unit that CSS does not have makes it `Other`.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind<'a> {
    End,
    /// A number, a dimension or a percentage, and its type.
    Value(Dim),
    Ident(&'a str),
    /// A name followed by an opening parenthesis.
    Function(&'a str),
    Open,
    Close,
    Comma,
    Plus,
    Minus,
    Star,
    Slash,
    Other,
}

#[derive(Clone, Copy)]
struct Token<'a> {
    kind: Kind<'a>,
    /// Whether white space stands before the token. The sign of a sum has to
    /// stand apart from both of its terms.
    space: bool,
    /// Where the token ends.
    end: usize,
    /// What a number, a dimension or a percentage comes to, if it can be told.
    value: Option<f64>,
}

fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0c')
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_') || !c.is_ascii()
}

/// Whether `text` starts a name: a letter, an underscore, a character that is
/// not ASCII, or a hyphen and then one of them or another hyphen.
fn starts_name(text: &str) -> bool {
    let is_start = |c: char| c == '_' || c.is_ascii_alphabetic() || !c.is_ascii();
    let mut chars = text.chars();
    match chars.next() {
        Some('-') => chars.next().is_some_and(|c| c == '-' || is_start(c)),
        Some(c) => is_start(c),
        None => false,
    }
}

/// The length of the name that starts `text`.
fn name_len(text: &str) -> usize {
    text.find(|c| !is_name_char(c)).unwrap_or(text.len())
}

/// The token of `text` that starts at or after `from`.
fn token(text: &str, from: usize) -> Token<'_> {
    let rest = &text[from..];
    let trimmed = rest.trim_start_matches(is_space);
    let space = trimmed.len() != rest.len();
    let start = text.len() - trimmed.len();
    let Some(first) = trimmed.chars().next() else {
        return Token {
            kind: Kind::End,
            space,
            end: text.len(),
            value: None,
        };
    };
    let mut kind = match first {
        '(' => Kind::Open,
        ')' => Kind::Close,
        ',' => Kind::Comma,
        '*' => Kind::Star,
        '/' => Kind::Slash,
        '+' => Kind::Plus,
        '-' => Kind::Minus,
        _ => Kind::Other,
    };
    let mut len = first.len_utf8();
    let mut value = None;
    if let Some(number) = number_len(trimmed) {
        // A number, and then a percent sign, a unit or nothing.
        let after = &trimmed[number..];
        len = number;
        kind = Kind::Value(NUMBER);
        value = trimmed[..number].parse::<f64>().ok();
        if after.starts_with('%') {
            len += 1;
            kind = Kind::Value(LENGTH);
            value = None;
        } else if starts_name(after) {
            let unit = name_len(after);
            len += unit;
            kind = unit_type(&after[..unit]).map_or(Kind::Other, Kind::Value);
            value = value
                .zip(unit_scale(&after[..unit]))
                .map(|(number, scale)| number * scale);
        }
    } else if starts_name(trimmed) {
        len = name_len(trimmed);
        kind = if trimmed[len..].starts_with('(') {
            len += 1;
            Kind::Function(&trimmed[..len - 1])
        } else {
            Kind::Ident(&trimmed[..len])
        };
    }
    Token {
        kind,
        space,
        end: start + len,
        value,
    }
}

/// The arguments of a function.
struct Arguments {
    terms: Vec<Term>,
    /// Whether every argument has the type of the first.
    same: bool,
}

impl Arguments {
    fn first(&self) -> Dim {
        self.terms[0].dim
    }

    /// The term of type `dim` that `work` makes of the values of the arguments,
    /// which has none if an argument has none.
    fn apply(&self, dim: Dim, work: impl FnOnce(&[f64]) -> f64) -> Term {
        let values: Option<Vec<f64>> = self.terms.iter().map(|term| term.value).collect();
        term(dim, values.map(|values| work(&values)))
    }
}

/// How `round()` picks the multiple of the step.
#[derive(Clone, Copy)]
enum Rounding {
    Nearest,
    Up,
    Down,
    ToZero,
}

impl Rounding {
    fn named(name: &str) -> Option<Self> {
        [
            ("nearest", Self::Nearest),
            ("up", Self::Up),
            ("down", Self::Down),
            ("to-zero", Self::ToZero),
        ]
        .into_iter()
        .find_map(|(known, strategy)| name.eq_ignore_ascii_case(known).then_some(strategy))
    }

    /// `value` rounded to a multiple of `step`, whose sign does not matter. Of
    /// two multiples that are as close, the nearest strategy takes the one
    /// toward positive infinity.
    fn round(self, value: f64, step: f64) -> f64 {
        let step = step.abs();
        let multiples = value / step;
        step * match self {
            // The distance to the multiple below tells the nearest one without
            // adding to the quotient, which would round it up when it is the
            // largest number under one half.
            Self::Nearest => {
                let below = multiples.floor();
                if multiples - below < 0.5 {
                    below
                } else {
                    below + 1.0
                }
            }
            Self::Up => multiples.ceil(),
            Self::Down => multiples.floor(),
            Self::ToZero => multiples.trunc(),
        }
    }
}

struct Parser<'a> {
    text: &'a str,
    at: usize,
    depth: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Token<'a> {
        token(self.text, self.at)
    }

    fn take(&mut self) -> Token<'a> {
        let token = self.peek();
        self.at = token.end;
        token
    }

    /// Enters a block, which `None` refuses when it is nested too deep.
    fn enter(&mut self) -> Option<()> {
        self.depth += 1;
        (self.depth <= MAX_DEPTH).then_some(())
    }

    /// The end of a block: its closing parenthesis, or the end of the input,
    /// which closes every block that is open.
    fn close(&mut self) -> Option<()> {
        matches!(self.take().kind, Kind::Close | Kind::End).then_some(())
    }

    fn comma(&mut self) -> Option<()> {
        matches!(self.take().kind, Kind::Comma).then_some(())
    }

    /// A sum: terms and the signs between them, and the type of the terms,
    /// which is the same for all of them.
    fn sum(&mut self) -> Option<Term> {
        let mut sum = self.product()?;
        loop {
            let sign = self.peek();
            let subtract = match sign.kind {
                Kind::Plus => false,
                Kind::Minus => true,
                _ => return Some(sum),
            };
            // `1px+2px` and `1px +2px` are not sums, as the sign would be one
            // of the number.
            if !sign.space || !token(self.text, sign.end).space {
                return None;
            }
            self.at = sign.end;
            let term = self.product()?;
            if term.dim != sum.dim {
                return None;
            }
            let value = sum
                .value
                .zip(term.value)
                .map(|(a, b)| if subtract { a - b } else { a + b });
            sum = self::term(sum.dim, value);
        }
    }

    fn product(&mut self) -> Option<Term> {
        let mut product = self.value()?;
        loop {
            let sign = self.peek();
            match sign.kind {
                Kind::Star => {
                    self.at = sign.end;
                    let factor = self.value()?;
                    product = term(
                        product.dim.times(factor.dim),
                        product.value.zip(factor.value).map(|(a, b)| a * b),
                    );
                }
                Kind::Slash => {
                    self.at = sign.end;
                    let divisor = self.value()?;
                    product = term(
                        product.dim.over(divisor.dim),
                        product.value.zip(divisor.value).map(|(a, b)| a / b),
                    );
                }
                _ => return Some(product),
            }
        }
    }

    fn value(&mut self) -> Option<Term> {
        let token = self.take();
        match token.kind {
            Kind::Value(dim) => Some(term(dim, token.value)),
            Kind::Ident(name) => constant(name),
            Kind::Open => {
                self.enter()?;
                let term = self.sum()?;
                self.close()?;
                self.depth -= 1;
                Some(term)
            }
            Kind::Function(name) => {
                self.enter()?;
                let term = self.function(name)?;
                self.depth -= 1;
                Some(term)
            }
            _ => None,
        }
    }

    /// The arguments of a function, which are sums that commas separate, and
    /// the closing parenthesis.
    fn arguments(&mut self) -> Option<Arguments> {
        let first = self.sum()?;
        let mut arguments = Arguments {
            terms: vec![first],
            same: true,
        };
        loop {
            match self.take().kind {
                Kind::Comma => {
                    let term = self.sum()?;
                    arguments.same &= term.dim == first.dim;
                    arguments.terms.push(term);
                }
                Kind::Close | Kind::End => return Some(arguments),
                _ => return None,
            }
        }
    }

    /// A bound of `clamp()`, which may be `none`.
    fn bound(&mut self) -> Option<Option<Term>> {
        let token = self.peek();
        if let Kind::Ident(name) = token.kind
            && name.eq_ignore_ascii_case("none")
        {
            self.at = token.end;
            return Some(None);
        }
        self.sum().map(Some)
    }

    /// The term that a function makes of the arguments that follow, or `None`
    /// when the call is not valid.
    fn function(&mut self, name: &str) -> Option<Term> {
        let name = name.to_ascii_lowercase();
        match name.as_str() {
            "calc" | "-webkit-calc" => {
                let term = self.sum()?;
                self.close()?;
                Some(term)
            }
            "clamp" => {
                let low = self.bound()?;
                self.comma()?;
                let value = self.sum()?;
                self.comma()?;
                let high = self.bound()?;
                self.close()?;
                if low.is_some_and(|low| low.dim != value.dim)
                    || high.is_some_and(|high| high.dim != value.dim)
                {
                    return None;
                }
                // A bound that is left out does not limit, and one without a
                // value leaves the result without one.
                let limit = |bound: Option<Term>, default: f64| match bound {
                    Some(bound) => bound.value,
                    None => Some(default),
                };
                let (low, high) = (limit(low, f64::NEG_INFINITY), limit(high, f64::INFINITY));
                Some(term(
                    value.dim,
                    value
                        .value
                        .zip(low)
                        .zip(high)
                        .map(|((value, low), high)| value.min(high).max(low)),
                ))
            }
            "round" => {
                // The rounding strategy may come first.
                let token = self.peek();
                let mut strategy = Rounding::Nearest;
                if let Kind::Ident(named) = token.kind
                    && let Some(named) = Rounding::named(named)
                {
                    self.at = token.end;
                    self.comma()?;
                    strategy = named;
                }
                let arguments = self.arguments()?;
                // The step may be left out of a number.
                match arguments.terms.len() {
                    2 if arguments.same => Some(arguments.apply(arguments.first(), |values| {
                        strategy.round(values[0], values[1])
                    })),
                    1 if arguments.first() == NUMBER => {
                        Some(arguments.apply(NUMBER, |values| strategy.round(values[0], 1.0)))
                    }
                    _ => None,
                }
            }
            _ => {
                let arguments = self.arguments()?;
                let (count, first, same) =
                    (arguments.terms.len(), arguments.first(), arguments.same);
                // The functions of an angle take it in degrees here.
                let radians = |degrees: f64| {
                    if first == ANGLE {
                        degrees.to_radians()
                    } else {
                        degrees
                    }
                };
                let trigonometric = count == 1 && (first == NUMBER || first == ANGLE);
                let number = count == 1 && first == NUMBER;
                Some(match name.as_str() {
                    "min" if same => arguments.apply(first, |values| {
                        values.iter().copied().fold(f64::INFINITY, f64::min)
                    }),
                    "max" if same => arguments.apply(first, |values| {
                        values.iter().copied().fold(f64::NEG_INFINITY, f64::max)
                    }),
                    "hypot" if same => arguments.apply(first, |values| {
                        values.iter().map(|value| value * value).sum::<f64>().sqrt()
                    }),
                    // The result of `mod()` has the sign of the divisor, and the
                    // one of `rem()` the sign of the dividend.
                    "mod" if count == 2 && same => arguments.apply(first, |values| {
                        values[0] - values[1] * (values[0] / values[1]).floor()
                    }),
                    "rem" if count == 2 && same => {
                        arguments.apply(first, |values| values[0] % values[1])
                    }
                    "atan2" if count == 2 && same => {
                        arguments.apply(ANGLE, |values| values[0].atan2(values[1]).to_degrees())
                    }
                    "abs" if count == 1 => arguments.apply(first, |values| values[0].abs()),
                    "sign" if count == 1 => arguments.apply(NUMBER, |values| {
                        if values[0] == 0.0 {
                            values[0]
                        } else {
                            values[0].signum()
                        }
                    }),
                    "sin" if trigonometric => {
                        arguments.apply(NUMBER, |values| radians(values[0]).sin())
                    }
                    "cos" if trigonometric => {
                        arguments.apply(NUMBER, |values| radians(values[0]).cos())
                    }
                    "tan" if trigonometric => {
                        arguments.apply(NUMBER, |values| radians(values[0]).tan())
                    }
                    "asin" if number => {
                        arguments.apply(ANGLE, |values| values[0].asin().to_degrees())
                    }
                    "acos" if number => {
                        arguments.apply(ANGLE, |values| values[0].acos().to_degrees())
                    }
                    "atan" if number => {
                        arguments.apply(ANGLE, |values| values[0].atan().to_degrees())
                    }
                    "sqrt" if number => arguments.apply(NUMBER, |values| values[0].sqrt()),
                    "exp" if number => arguments.apply(NUMBER, |values| values[0].exp()),
                    "pow" if count == 2 && first == NUMBER && same => {
                        arguments.apply(NUMBER, |values| values[0].powf(values[1]))
                    }
                    "log" if (1..=2).contains(&count) && first == NUMBER && same => arguments
                        .apply(NUMBER, |values| match values {
                            [value, base] => value.ln() / base.ln(),
                            _ => values[0].ln(),
                        }),
                    _ => return None,
                })
            }
        }
    }
}

/// What a math function comes to, when it is a length.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::filter_source) enum Length {
    /// A number of pixels, which is the user unit of the SVG.
    Pixels(f32),
    /// A length that depends on where it is used, such as a percentage, or one
    /// that this does not work out.
    Unresolved,
}

/// What `value` is as a math function whose result is a length or a
/// percentage, or, where `unitless` is set, a number, which the properties of
/// the SVG geometry take as a length. `None` when it is not one.
pub(in crate::filter_source) fn length(value: &str, unitless: bool) -> Option<Length> {
    let mut parser = Parser {
        text: value,
        at: 0,
        depth: 0,
    };
    let Kind::Function(name) = parser.take().kind else {
        return None;
    };
    let term = parser
        .enter()
        .and_then(|()| parser.function(name))
        .filter(|_| matches!(parser.take().kind, Kind::End))?;
    if term.dim != LENGTH && !(unitless && term.dim == NUMBER) {
        return None;
    }
    // The user unit is a single precision number, as it is for `usvg`.
    Some(
        term.value
            .map(|pixels| pixels as f32)
            .filter(|pixels| pixels.is_finite())
            .map_or(Length::Unresolved, Length::Pixels),
    )
}

/// Whether `value` is one math function whose result is a length or a
/// percentage, or, where `unitless` is set, a number.
#[cfg(test)]
pub(super) fn is_length(value: &str, unitless: bool) -> bool {
    length(value, unitless).is_some()
}

/// The number of pixels as the text of an attribute: a plain number, which is
/// short for any value: an exponent stands for the ones that are very large or
/// very small.
pub(in crate::filter_source) fn pixels_text(pixels: f32) -> String {
    if pixels == 0.0 {
        "0".to_owned()
    } else if pixels.abs() >= 1e9 || pixels.abs() < 1e-6 {
        format!("{pixels:e}")
    } else {
        pixels.to_string()
    }
}

/// The names of the math functions, in lowercase.
const FUNCTIONS: &[&str] = &[
    "calc",
    "-webkit-calc",
    "min",
    "max",
    "clamp",
    "round",
    "mod",
    "rem",
    "sin",
    "cos",
    "tan",
    "asin",
    "acos",
    "atan",
    "atan2",
    "pow",
    "sqrt",
    "hypot",
    "log",
    "exp",
    "abs",
    "sign",
];

/// Whether `source` may hold a math function: a name of one that a parenthesis
/// follows. A cheap test on the source, for the documents that have none to go
/// without being parsed. A miss leaves the lengths to `usvg` and a false hit
/// only costs a parse.
pub(in crate::filter_source) fn may_hold(source: &str) -> bool {
    // A character reference may spell the name or the parenthesis, which the
    // XML parser decodes and this scan does not see.
    if source.contains("&#") {
        return true;
    }
    source.match_indices('(').any(|(at, _)| {
        let head = &source[..at];
        let len = head
            .bytes()
            .rev()
            .take_while(|byte| byte.is_ascii_alphanumeric() || *byte == b'-')
            .count();
        let name = &head[head.len() - len..];
        FUNCTIONS
            .iter()
            .any(|function| name.eq_ignore_ascii_case(function))
    })
}

#[cfg(test)]
mod tests;
