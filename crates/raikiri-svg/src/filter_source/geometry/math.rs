//! The math functions of CSS: `calc()`, `min()`, `round()` and the rest.
//!
//! A browser checks a math function when it parses the declaration that holds
//! it: the grammar, and the type of every term, so that a length is not added
//! to a number or to an angle. A declaration that fails the check is dropped.
//! What the function comes to is worked out later, and this does not do it: it
//! only tells whether the declaration is valid.
//!
//! The types follow CSS Values 4, typed arithmetic included: a product or a
//! quotient has the type that the units of its factors make, so
//! `calc(30px * 2px / 1px)` is a length.

use super::{LENGTH_UNITS, number_len};

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

/// The numbers that a name stands for.
fn is_constant(name: &str) -> bool {
    ["e", "pi", "infinity", "-infinity", "nan"]
        .iter()
        .any(|constant| name.eq_ignore_ascii_case(constant))
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
    if let Some(number) = number_len(trimmed) {
        // A number, and then a percent sign, a unit or nothing.
        let after = &trimmed[number..];
        len = number;
        kind = Kind::Value(NUMBER);
        if after.starts_with('%') {
            len += 1;
            kind = Kind::Value(LENGTH);
        } else if starts_name(after) {
            let unit = name_len(after);
            len += unit;
            kind = unit_type(&after[..unit]).map_or(Kind::Other, Kind::Value);
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
    }
}

/// The arguments of a function, as far as the check of their types needs.
struct Arguments {
    count: usize,
    first: Dim,
    /// Whether every argument has the type of the first.
    same: bool,
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
    fn sum(&mut self) -> Option<Dim> {
        let dim = self.product()?;
        loop {
            let sign = self.peek();
            if !matches!(sign.kind, Kind::Plus | Kind::Minus) {
                return Some(dim);
            }
            // `1px+2px` and `1px +2px` are not sums, as the sign would be one
            // of the number.
            if !sign.space || !token(self.text, sign.end).space {
                return None;
            }
            self.at = sign.end;
            if self.product()? != dim {
                return None;
            }
        }
    }

    fn product(&mut self) -> Option<Dim> {
        let mut dim = self.value()?;
        loop {
            let sign = self.peek();
            match sign.kind {
                Kind::Star => {
                    self.at = sign.end;
                    dim = dim.times(self.value()?);
                }
                Kind::Slash => {
                    self.at = sign.end;
                    dim = dim.over(self.value()?);
                }
                _ => return Some(dim),
            }
        }
    }

    fn value(&mut self) -> Option<Dim> {
        match self.take().kind {
            Kind::Value(dim) => Some(dim),
            Kind::Ident(name) => is_constant(name).then_some(NUMBER),
            Kind::Open => {
                self.enter()?;
                let dim = self.sum()?;
                self.close()?;
                self.depth -= 1;
                Some(dim)
            }
            Kind::Function(name) => {
                self.enter()?;
                let dim = self.function(name)?;
                self.depth -= 1;
                Some(dim)
            }
            _ => None,
        }
    }

    /// The arguments of a function, which are sums that commas separate, and
    /// the closing parenthesis.
    fn arguments(&mut self) -> Option<Arguments> {
        let first = self.sum()?;
        let mut arguments = Arguments {
            count: 1,
            first,
            same: true,
        };
        loop {
            match self.take().kind {
                Kind::Comma => {
                    arguments.same &= self.sum()? == first;
                    arguments.count += 1;
                }
                Kind::Close | Kind::End => return Some(arguments),
                _ => return None,
            }
        }
    }

    /// A bound of `clamp()`, which may be `none`.
    fn bound(&mut self) -> Option<Option<Dim>> {
        let token = self.peek();
        if let Kind::Ident(name) = token.kind
            && name.eq_ignore_ascii_case("none")
        {
            self.at = token.end;
            return Some(None);
        }
        self.sum().map(Some)
    }

    /// The type of a function that is called with the arguments that follow,
    /// or `None` when the call is not valid.
    fn function(&mut self, name: &str) -> Option<Dim> {
        let name = name.to_ascii_lowercase();
        match name.as_str() {
            "calc" | "-webkit-calc" => {
                let dim = self.sum()?;
                self.close()?;
                Some(dim)
            }
            "clamp" => {
                let low = self.bound()?;
                self.comma()?;
                let value = self.sum()?;
                self.comma()?;
                let high = self.bound()?;
                self.close()?;
                (low.is_none_or(|dim| dim == value) && high.is_none_or(|dim| dim == value))
                    .then_some(value)
            }
            "round" => {
                // The rounding strategy may come first.
                let token = self.peek();
                if let Kind::Ident(strategy) = token.kind
                    && ["nearest", "up", "down", "to-zero"]
                        .iter()
                        .any(|known| strategy.eq_ignore_ascii_case(known))
                {
                    self.at = token.end;
                    self.comma()?;
                }
                let arguments = self.arguments()?;
                // The step may be left out of a number.
                match arguments.count {
                    2 if arguments.same => Some(arguments.first),
                    1 if arguments.first == NUMBER => Some(NUMBER),
                    _ => None,
                }
            }
            _ => {
                let Arguments { count, first, same } = self.arguments()?;
                match name.as_str() {
                    "min" | "max" | "hypot" => same.then_some(first),
                    "mod" | "rem" => (count == 2 && same).then_some(first),
                    "atan2" => (count == 2 && same).then_some(ANGLE),
                    "abs" => (count == 1).then_some(first),
                    "sign" => (count == 1).then_some(NUMBER),
                    "sin" | "cos" | "tan" => {
                        (count == 1 && (first == NUMBER || first == ANGLE)).then_some(NUMBER)
                    }
                    "asin" | "acos" | "atan" => (count == 1 && first == NUMBER).then_some(ANGLE),
                    "sqrt" | "exp" => (count == 1 && first == NUMBER).then_some(NUMBER),
                    "pow" => (count == 2 && first == NUMBER && same).then_some(NUMBER),
                    "log" => {
                        ((1..=2).contains(&count) && first == NUMBER && same).then_some(NUMBER)
                    }
                    _ => None,
                }
            }
        }
    }
}

/// Whether `value` is one math function whose result is a length or a
/// percentage, or, where `unitless` is set, a number, which the properties of
/// the SVG geometry take as a length.
pub(super) fn is_length(value: &str, unitless: bool) -> bool {
    let mut parser = Parser {
        text: value,
        at: 0,
        depth: 0,
    };
    let Kind::Function(name) = parser.take().kind else {
        return false;
    };
    let Some(dim) = parser
        .enter()
        .and_then(|()| parser.function(name))
        .filter(|_| matches!(parser.take().kind, Kind::End))
    else {
        return false;
    };
    dim == LENGTH || (unitless && dim == NUMBER)
}

#[cfg(test)]
mod tests;
