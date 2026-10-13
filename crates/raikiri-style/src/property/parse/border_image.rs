//! `border-image` and its longhands (CSS Backgrounds 3 §6
//! <https://www.w3.org/TR/css-backgrounds-3/#border-images>).

use cssparser::{ParseError, Parser, ParserInput, ToCss, Token};

use crate::property::types::*;

use super::common::*;
use super::visual::parse_background_image;

/// Expand one to four parsed values clockwise from the top, as the margin
/// and padding shorthands do.
fn sides_from<T: Copy>(values: &[T]) -> Option<Sides<T>> {
    let (top, right, bottom, left) = match *values {
        [all] => (all, all, all, all),
        [vertical, horizontal] => (vertical, horizontal, vertical, horizontal),
        [top, horizontal, bottom] => (top, horizontal, bottom, horizontal),
        [top, right, bottom, left] => (top, right, bottom, left),
        _ => return None,
    };
    Some(Sides {
        top,
        right,
        bottom,
        left,
    })
}

/// Parse up to four values with `item`, stopping at the first one that does
/// not parse.
fn parse_up_to_four<'i, T: Copy>(
    input: &mut Parser<'i, '_>,
    item: impl Fn(&mut Parser<'i, '_>) -> Result<T, ParseError<'i, ()>>,
) -> Option<Sides<T>> {
    let mut values = Vec::with_capacity(4);
    while values.len() < 4 {
        match input.try_parse(&item) {
            Ok(value) => values.push(value),
            Err(_) => break,
        }
    }
    sides_from(&values)
}

/// One math function (`calc()`, `min()`, `max()` or `clamp()`) folded to a
/// single numeric value and clamped to `[0,∞]`, returned as CSS source for
/// the slot's own parser. A math function outside a property's range is
/// clamped instead of rejected (CSS Values 4 §10.12), unlike a literal
/// negative value. A function that does not fold to one value is left to
/// the slot's other branches.
fn clamped_math_source<'i>(input: &mut Parser<'i, '_>) -> Result<String, ParseError<'i, ()>> {
    let location = input.current_source_location();
    input.skip_whitespace();
    let start = input.position();
    match input.next()? {
        Token::Function(name)
            if ["calc", "min", "max", "clamp"]
                .iter()
                .any(|function| name.eq_ignore_ascii_case(function)) => {}
        _ => return Err(location.new_custom_error(())),
    }
    input.parse_nested_block(|nested| {
        while nested.next().is_ok() {}
        Ok::<_, ParseError<'_, ()>>(())
    })?;
    let simplified = crate::cascade::simplify_math_functions(input.slice_from(start))
        .ok_or_else(|| location.new_custom_error(()))?;
    let mut token_input = ParserInput::new(&simplified);
    let mut parser = Parser::new(&mut token_input);
    let token = next_numeric_stable(&mut parser)
        .ok()
        .filter(|token| {
            matches!(
                token,
                Token::Number { .. } | Token::Percentage { .. } | Token::Dimension { .. }
            ) && parser.expect_exhausted().is_ok()
        })
        .ok_or_else(|| location.new_custom_error(()))?;
    let clamped = match token {
        Token::Number { value, .. } if value < 0.0 => Token::Number {
            has_sign: false,
            value: 0.0,
            int_value: Some(0),
        },
        Token::Percentage { unit_value, .. } if unit_value < 0.0 => Token::Percentage {
            has_sign: false,
            unit_value: 0.0,
            int_value: Some(0),
        },
        Token::Dimension { value, unit, .. } if value < 0.0 => Token::Dimension {
            has_sign: false,
            value: 0.0,
            int_value: Some(0),
            unit,
        },
        token => token,
    };
    Ok(clamped.to_css_string())
}

/// Parse `source`, a value from [`clamped_math_source`], with `item`.
fn parse_clamped<T>(
    source: &str,
    item: impl for<'a, 'b> FnOnce(&mut Parser<'a, 'b>) -> Option<T>,
) -> Option<T> {
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    let value = item(&mut parser)?;
    parser.expect_exhausted().ok()?;
    Some(value)
}

/// A `<number [0,∞]>` token, or a math function that folds to a number.
fn parse_non_negative_number<'i>(input: &mut Parser<'i, '_>) -> Result<f32, ParseError<'i, ()>> {
    if let Ok(source) = input.try_parse(clamped_math_source) {
        return parse_clamped(&source, |parser| expect_number_stable(parser).ok())
            .ok_or_else(|| input.new_custom_error(()));
    }
    let value = expect_number_stable(input)?;
    if value >= 0.0 {
        Ok(value)
    } else {
        Err(input.new_custom_error(()))
    }
}

/// `border-image-source: none | <image>`.
pub(crate) fn parse_border_image_source(input: &mut Parser<'_, '_>) -> Option<BackgroundImage> {
    parse_background_image(input)
}

fn parse_slice_offset<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<BorderImageSliceOffset, ParseError<'i, ()>> {
    let location = input.current_source_location();
    if let Ok(source) = input.try_parse(clamped_math_source) {
        return parse_clamped(&source, |parser| parse_slice_offset(parser).ok())
            .ok_or_else(|| location.new_custom_error(()));
    }
    let offset = match next_numeric_stable(input)? {
        Token::Number { value, .. } => BorderImageSliceOffset::Number(value),
        Token::Percentage { unit_value, .. } => BorderImageSliceOffset::Percent(unit_value * 100.0),
        _ => return Err(location.new_custom_error(())),
    };
    match offset {
        BorderImageSliceOffset::Number(value) | BorderImageSliceOffset::Percent(value)
            if value >= 0.0 =>
        {
            Ok(offset)
        }
        _ => Err(location.new_custom_error(())),
    }
}

fn parse_fill<'i>(input: &mut Parser<'i, '_>) -> Result<(), ParseError<'i, ()>> {
    input.expect_ident_matching("fill")?;
    Ok(())
}

/// `border-image-slice: [<number [0,∞]> | <percentage [0,∞]>]{1,4} && fill?`.
pub(crate) fn parse_border_image_slice(input: &mut Parser<'_, '_>) -> Option<BorderImageSlice> {
    let fill_first = input.try_parse(parse_fill).is_ok();
    let offsets = parse_up_to_four(input, parse_slice_offset)?;
    let fill = fill_first || input.try_parse(parse_fill).is_ok();
    Some(BorderImageSlice { offsets, fill })
}

fn parse_width_side<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<BorderImageWidthSide<LengthPercentageOrCalc>, ParseError<'i, ()>> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Ok(BorderImageWidthSide::Auto);
    }
    // A bare number, including `0`, is a multiple of the border width.
    if let Ok(number) = input.try_parse(parse_non_negative_number) {
        return Ok(BorderImageWidthSide::Number(number));
    }
    // A math function that folds to one length or percentage.
    if let Ok(length) = input.try_parse(|input| {
        let source = clamped_math_source(input)?;
        parse_clamped(&source, |parser| {
            parse_non_negative_length_percentage_res(parser).ok()
        })
        .ok_or_else(|| input.new_custom_error(()))
    }) {
        return Ok(BorderImageWidthSide::LengthPercentage(length.into()));
    }
    // A mixed-unit `calc()` that substitution or simplification could not
    // fold keeps its terms; a math function is clamped to the range.
    if let Some(calc) = crate::cascade::parse_relative_calc_function(input) {
        return Ok(BorderImageWidthSide::LengthPercentage(
            LengthPercentageOrCalc::from_non_negative_calc(calc),
        ));
    }
    parse_non_negative_length_percentage_res(input)
        .map(|length| BorderImageWidthSide::LengthPercentage(length.into()))
}

/// `border-image-width: [<length-percentage [0,∞]> | <number [0,∞]> | auto]{1,4}`.
pub(crate) fn parse_border_image_width(
    input: &mut Parser<'_, '_>,
) -> Option<Sides<BorderImageWidthSide<LengthPercentageOrCalc>>> {
    parse_up_to_four(input, parse_width_side)
}

fn parse_outset_side<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<BorderImageOutsetSide<Length>, ParseError<'i, ()>> {
    if let Ok(number) = input.try_parse(parse_non_negative_number) {
        return Ok(BorderImageOutsetSide::Number(number));
    }
    let location = input.current_source_location();
    if let Ok(source) = input.try_parse(clamped_math_source) {
        return parse_clamped(&source, parse_non_negative_length)
            .map(BorderImageOutsetSide::Length)
            .ok_or_else(|| location.new_custom_error(()));
    }
    parse_non_negative_length(input)
        .map(BorderImageOutsetSide::Length)
        .ok_or_else(|| location.new_custom_error(()))
}

/// `border-image-outset: [<length [0,∞]> | <number [0,∞]>]{1,4}`.
pub(crate) fn parse_border_image_outset(
    input: &mut Parser<'_, '_>,
) -> Option<Sides<BorderImageOutsetSide<Length>>> {
    parse_up_to_four(input, parse_outset_side)
}

fn parse_repeat_keyword<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<BorderImageRepeatKeyword, ParseError<'i, ()>> {
    let location = input.current_source_location();
    let ident = input.expect_ident()?;
    BorderImageRepeatKeyword::from_css_ident(ident).ok_or_else(|| location.new_custom_error(()))
}

/// `border-image-repeat: [stretch | repeat | round | space]{1,2}`.
pub(crate) fn parse_border_image_repeat(input: &mut Parser<'_, '_>) -> Option<BorderImageRepeat> {
    let horizontal = input.try_parse(parse_repeat_keyword).ok()?;
    let vertical = input.try_parse(parse_repeat_keyword).unwrap_or(horizontal);
    Some(BorderImageRepeat {
        horizontal,
        vertical,
    })
}

fn parse_slash<'i>(input: &mut Parser<'i, '_>) -> Result<(), ParseError<'i, ()>> {
    input.expect_delim('/')?;
    Ok(())
}

/// The `border-image` shorthand:
/// `<'border-image-source'> || <'border-image-slice'>
/// [ / <'border-image-width'> | / <'border-image-width'>? / <'border-image-outset'> ]?
/// || <'border-image-repeat'>`.
///
/// The width and outset may only follow the slice. Omitted longhands take
/// their initial values.
pub(crate) fn parse_border_image_shorthand(
    input: &mut Parser<'_, '_>,
) -> Option<BorderImageShorthand> {
    let mut source = None;
    let mut slice = None;
    let mut width = None;
    let mut outset = None;
    let mut repeat = None;
    loop {
        if source.is_none()
            && let Ok(image) = input.try_parse(|i| {
                parse_border_image_source(i).ok_or_else(|| i.new_custom_error::<(), ()>(()))
            })
        {
            source = Some(image);
            continue;
        }
        if slice.is_none()
            && let Ok(value) = input.try_parse(|i| {
                parse_border_image_slice(i).ok_or_else(|| i.new_custom_error::<(), ()>(()))
            })
        {
            slice = Some(value);
            if input.try_parse(parse_slash).is_ok() {
                width = input
                    .try_parse(|i| {
                        parse_border_image_width(i).ok_or_else(|| i.new_custom_error::<(), ()>(()))
                    })
                    .ok();
                if input.try_parse(parse_slash).is_ok() {
                    outset = Some(parse_border_image_outset(input)?);
                } else if width.is_none() {
                    // A slash must be followed by a width or a second slash.
                    return None;
                }
            }
            continue;
        }
        if repeat.is_none()
            && let Ok(value) = input.try_parse(|i| {
                parse_border_image_repeat(i).ok_or_else(|| i.new_custom_error::<(), ()>(()))
            })
        {
            repeat = Some(value);
            continue;
        }
        break;
    }
    if source.is_none() && slice.is_none() && repeat.is_none() {
        return None;
    }
    let initial = BorderImageShorthand::initial();
    Some(BorderImageShorthand {
        source: source.unwrap_or(initial.source),
        slice: slice.unwrap_or(initial.slice),
        width: width.unwrap_or(initial.width),
        outset: outset.unwrap_or(initial.outset),
        repeat: repeat.unwrap_or(initial.repeat),
    })
}
