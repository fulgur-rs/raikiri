//! Token-parser combinators corresponding to CSS grammar notation.

use cssparser::{ParseError, Parser};

pub(crate) type PResult<'i, T> = Result<T, ParseError<'i, ()>>;
pub(crate) type Production<'a, 'i, T> = &'a dyn Fn(&mut Parser<'i, '_>) -> PResult<'i, T>;

/// `A | B | ...`: the first alternative that parses.
pub(crate) fn first_of<'i, T>(
    input: &mut Parser<'i, '_>,
    alternatives: &[Production<'_, 'i, T>],
) -> PResult<'i, T> {
    let mut error = input.new_custom_error(());
    for alternative in alternatives {
        match input.try_parse(|input| alternative(input)) {
            Ok(value) => return Ok(value),
            Err(err) => error = err,
        }
    }
    Err(error)
}

/// `A?`
pub(crate) fn optional<'i, T>(
    input: &mut Parser<'i, '_>,
    production: impl FnOnce(&mut Parser<'i, '_>) -> PResult<'i, T>,
) -> Option<T> {
    input.try_parse(production).ok()
}

/// `A*`
pub(crate) fn zero_or_more<'i, T>(
    input: &mut Parser<'i, '_>,
    mut production: impl FnMut(&mut Parser<'i, '_>) -> PResult<'i, T>,
) -> Vec<T> {
    std::iter::from_fn(|| optional(input, &mut production)).collect()
}

/// `( A )`: a parenthesised block whose contents are exactly `A`.
pub(crate) fn parens<'i, T>(
    input: &mut Parser<'i, '_>,
    production: impl FnOnce(&mut Parser<'i, '_>) -> PResult<'i, T>,
) -> PResult<'i, T> {
    input.expect_parenthesis_block()?;
    input.parse_nested_block(production)
}

/// A keyword, matched ASCII case-insensitively.
pub(crate) fn keyword<'i>(input: &mut Parser<'i, '_>, name: &'static str) -> PResult<'i, ()> {
    Ok(input.expect_ident_matching(name)?)
}
