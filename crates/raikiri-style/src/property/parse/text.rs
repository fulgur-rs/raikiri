//! Font and text property parsers, including `text-decoration`, `text-shadow`
//! and the `font` shorthand.

use std::sync::Arc;

use cssparser::{ParseError, Parser, ParserInput, Token};
use smol_str::SmolStr;

use crate::property::types::*;

use super::color::*;
use super::common::*;

/// Parses `font-family: <family-name>#`.
///
/// A comma-separated list of family names. Each family name is either a quoted string
/// (`"Times New Roman"`) or a sequence of unquoted identifiers (`Times New Roman` =
/// three space-separated identifiers forming one family, valid in CSS4).
///
/// If no comma follows, the loop stops and leaves the remaining input (`!important`, etc.)
/// untouched for downstream handling (the caller's `parse_important` / `expect_exhausted`).
/// This addresses Finding 3 by not rejecting `!` as garbage.
pub(super) fn parse_font_family(input: &mut Parser<'_, '_>) -> Option<Vec<FontFamilyName>> {
    let mut families = Vec::new();
    loop {
        // Quoted strings are named families even when their text matches a
        // generic keyword (for example, `"serif"`).
        let family = if let Ok(s) = input.try_parse(|i| i.expect_string().cloned()) {
            FontFamilyName::named(s.as_ref())
        } else if let Ok(first) = input.try_parse(|i| i.expect_ident().cloned()) {
            // Unquoted ident sequence: `Times New Roman` = 3 idents joined by space.
            let mut buf = first.as_ref().to_string();
            let mut single_ident = true;
            while let Ok(next) = input.try_parse(|i| i.expect_ident().cloned()) {
                single_ident = false;
                buf.push(' ');
                buf.push_str(next.as_ref());
            }
            if single_ident && is_generic_font_family_keyword(&buf) {
                FontFamilyName::generic(buf)
            } else {
                FontFamilyName::named(buf)
            }
        } else {
            return None;
        };
        families.push(family);
        // Consume comma or stop (leaves remaining input alone)
        if input.try_parse(|i| i.expect_comma()).is_err() {
            break;
        }
    }
    if families.is_empty() {
        None
    } else {
        Some(families)
    }
}

fn is_generic_font_family_keyword(name: &str) -> bool {
    const GENERIC_FAMILIES: &[&str] = &[
        "serif",
        "sans-serif",
        "monospace",
        "cursive",
        "fantasy",
        "system-ui",
        "ui-serif",
        "ui-sans-serif",
        "ui-monospace",
        "ui-rounded",
        "math",
        "emoji",
        "fangsong",
    ];
    GENERIC_FAMILIES
        .iter()
        .any(|generic| name.eq_ignore_ascii_case(generic))
}

/// `text-indent`'s full grammar.
///
/// grammar reference: CSS Text 3 §8.1
/// <https://www.w3.org/TR/css-text-3/#text-indent-property>, whose full
/// grammar is `<length-percentage> && hanging? && each-line?` — this helper
/// covers all three components, returning [`TextIndentValue`]. Simple additive
/// `calc()` expressions retain their `em` coefficient until the element's
/// computed font size is known.
///
/// No non-negative filter, unlike [`parse_padding_side`](super::box_model::parse_padding_side) — the spec places no
/// `[0,∞]` restriction on this grammar (negative indents are valid, sibling
/// [`parse_margin_side`](super::box_model::parse_margin_side) applies the same "no filter" treatment for the same
/// reason its own grammar allows negative values).
pub(super) fn parse_text_indent(input: &mut Parser<'_, '_>) -> Option<TextIndentValue> {
    let mut length: Option<TextIndentLength> = None;
    let mut hanging = false;
    let mut each_line = false;
    loop {
        if length.is_none()
            && let Ok(parsed) = input.try_parse(|i| {
                parse_text_indent_length(i).ok_or_else(|| i.new_custom_error::<(), ()>(()))
            })
        {
            length = Some(parsed);
            continue;
        }
        if !hanging
            && input
                .try_parse(|i| i.expect_ident_matching("hanging"))
                .is_ok()
        {
            hanging = true;
            continue;
        }
        if !each_line
            && input
                .try_parse(|i| i.expect_ident_matching("each-line"))
                .is_ok()
        {
            each_line = true;
            continue;
        }
        break;
    }
    length.map(|length| TextIndentValue {
        length,
        hanging,
        each_line,
    })
}

fn parse_text_indent_length(input: &mut Parser<'_, '_>) -> Option<TextIndentLength> {
    input
        .try_parse(|i| {
            parse_text_indent_calc_with_ch(i).ok_or_else(|| i.new_custom_error::<(), ()>(()))
        })
        .ok()
        .or_else(|| parse_length_value(input, true).map(TextIndentLength::Length))
}

fn parse_text_indent_calc(input: &mut Parser<'_, '_>) -> Option<TextIndentLength> {
    parse_text_indent_calc_impl(input, false)
}

/// Like [`parse_text_indent_calc`] but also accepts `ch` terms. Only the
/// properties whose computed value can carry font-metric provenance
/// (`text-indent`, `letter-spacing`, `word-spacing`) opt in; every other
/// caller keeps rejecting `ch` inside `calc()` rather than dropping it later.
fn parse_text_indent_calc_with_ch(input: &mut Parser<'_, '_>) -> Option<TextIndentLength> {
    parse_text_indent_calc_impl(input, true)
}

fn parse_text_indent_calc_impl(
    input: &mut Parser<'_, '_>,
    allow_ch: bool,
) -> Option<TextIndentLength> {
    input
        .try_parse(|i| -> Result<TextIndentLength, ParseError<'_, ()>> {
            match i.next()?.clone() {
                Token::Function(name) if name.eq_ignore_ascii_case("calc") => {}
                token => return Err(i.new_unexpected_token_error(token)),
            }
            i.parse_nested_block(|i| parse_text_indent_calc_terms(i, allow_ch))
        })
        .ok()
}

fn parse_text_indent_calc_terms<'i>(
    input: &mut Parser<'i, '_>,
    allow_ch: bool,
) -> Result<TextIndentLength, ParseError<'i, ()>> {
    let value = parse_text_indent_calc_sum(input, allow_ch)?;
    if value.ch != 0.0 {
        if value.percent == 0.0 && value.px == 0.0 && value.em == 0.0 {
            Ok(TextIndentLength::Length(Length::Ch(value.ch)))
        } else {
            Ok(TextIndentLength::Calc(value))
        }
    } else if value.em == 0.0 {
        if value.percent == 0.0 {
            Ok(TextIndentLength::Length(Length::Px(value.px)))
        } else if value.px == 0.0 {
            Ok(TextIndentLength::Length(Length::Percent(value.percent)))
        } else {
            Ok(TextIndentLength::Calc(value))
        }
    } else if value.percent == 0.0 && value.px == 0.0 {
        Ok(TextIndentLength::Length(Length::Em(value.em)))
    } else {
        Ok(TextIndentLength::Calc(value))
    }
}

fn parse_text_indent_calc_sum<'i>(
    input: &mut Parser<'i, '_>,
    allow_ch: bool,
) -> Result<LengthPercentageCalc, ParseError<'i, ()>> {
    let mut value = parse_text_indent_calc_term(input, allow_ch)?;
    while !input.is_exhausted() {
        let sign = match input.next()?.clone() {
            Token::Delim('+') => 1.0,
            Token::Delim('-') => -1.0,
            token => return Err(input.new_unexpected_token_error(token)),
        };
        let term = parse_text_indent_calc_term(input, allow_ch)?;
        value.percent += sign * term.percent;
        value.px += sign * term.px;
        value.em += sign * term.em;
        value.ch += sign * term.ch;
        if !value.percent.is_finite()
            || !value.px.is_finite()
            || !value.em.is_finite()
            || !value.ch.is_finite()
        {
            return Err(input.new_custom_error(()));
        }
    }
    Ok(value)
}

fn parse_text_indent_calc_term<'i>(
    input: &mut Parser<'i, '_>,
    allow_ch: bool,
) -> Result<LengthPercentageCalc, ParseError<'i, ()>> {
    let start = input.state();
    if matches!(input.next()?.clone(), Token::ParenthesisBlock) {
        return input.parse_nested_block(|i| parse_text_indent_calc_sum(i, allow_ch));
    }
    input.reset(&start);

    // A bare number is not a `<length-percentage>` calc term, even for zero.
    let token = next_numeric_stable(input)?;
    if !matches!(&token, Token::Dimension { .. } | Token::Percentage { .. }) {
        return Err(input.new_unexpected_token_error(token));
    }
    input.reset(&start);
    let length = parse_length_value(input, true).ok_or_else(|| input.new_custom_error(()))?;
    let value = match length {
        Length::Px(px) => LengthPercentageCalc {
            percent: 0.0,
            px,
            em: 0.0,
            ch: 0.0,
        },
        Length::Pt(v) => LengthPercentageCalc {
            percent: 0.0,
            px: v * (96.0 / 72.0),
            em: 0.0,
            ch: 0.0,
        },
        Length::Cm(v) => LengthPercentageCalc {
            percent: 0.0,
            px: v * (96.0 / 2.54),
            em: 0.0,
            ch: 0.0,
        },
        Length::Mm(v) => LengthPercentageCalc {
            percent: 0.0,
            px: v * (96.0 / 25.4),
            em: 0.0,
            ch: 0.0,
        },
        Length::Q(v) => LengthPercentageCalc {
            percent: 0.0,
            px: v * (96.0 / 101.6),
            em: 0.0,
            ch: 0.0,
        },
        Length::In(v) => LengthPercentageCalc {
            percent: 0.0,
            px: v * 96.0,
            em: 0.0,
            ch: 0.0,
        },
        Length::Pc(v) => LengthPercentageCalc {
            percent: 0.0,
            px: v * 16.0,
            em: 0.0,
            ch: 0.0,
        },
        Length::Em(em) => LengthPercentageCalc {
            percent: 0.0,
            px: 0.0,
            em,
            ch: 0.0,
        },
        Length::Percent(percent) => LengthPercentageCalc {
            percent,
            px: 0.0,
            em: 0.0,
            ch: 0.0,
        },
        Length::Ch(ch) if allow_ch => LengthPercentageCalc {
            percent: 0.0,
            px: 0.0,
            em: 0.0,
            ch,
        },
        _ => return Err(input.new_custom_error(())),
    };
    if value.percent.is_finite()
        && value.px.is_finite()
        && value.em.is_finite()
        && value.ch.is_finite()
    {
        Ok(value)
    } else {
        Err(input.new_custom_error(()))
    }
}

/// `font-size: <absolute-size> | <relative-size> | <length-percentage [0,∞]> |
/// math`.
///
/// Grammar (CSS Fonts 4 §2.5 "Font size: the font-size property"
/// <https://www.w3.org/TR/css-fonts-4/#font-size-prop>):
/// The grammar is `<absolute-size> | <relative-size> | <length-percentage [0,∞]> | math`.
///
/// - `<absolute-size>` (`xx-small` … `xxx-large`, `medium`) — [`parse_font_size_keyword`]
///   resolves the §2.5.1 scaling-factor table relative to `medium` = 16px and
///   returns [`PropertyValue::FontSize`] (`Length::Px`).
/// - `<relative-size>` (`larger` / `smaller`) — Its result depends on inheritance,
///   so this parser returns [`PropertyValue::FontSizeRelative`] and leaves resolution
///   [`crate::cascade::apply_value`] / [`crate::cascade::resolve_against_inherited`]
///   to that code (see the variant's documentation for details).
/// - `<length-percentage [0,∞]>` — Handled later in this function via [`parse_length_value`].
/// - `math` — Not implemented (the entire MathML scaling algorithm is unsupported),
///   so the parser returns `None`.
///
/// # Why the identifier branch uses `try_parse` first
///
/// `<absolute-size>`, `<relative-size>`, and `math` are all single identifier tokens.
/// As in the `auto` branch of [`parse_margin_side`](super::box_model::parse_margin_side), [`parse_length_value`]
/// consumes `input.next()` unconditionally, so the identifier branch must be tried
/// first with a rewind checkpoint (`try_parse`).
///
/// Relative and percentage lengths are resolved by the cascade using the
/// parent and root font-size bases required by CSS Values 4.
///
/// # Non-negative constraint
///
/// The grammar's `[0,∞]` constraint is enforced at parse time. As in [`parse_padding_side`](super::box_model::parse_padding_side) /
/// [`parse_width`](super::box_model::parse_width), [`Length::payload`] is used to check every [`Length`] variant
/// — not only `-5px` but also `-50%` and `-1em` are dropped. `<absolute-size>` and
/// `<relative-size>` cannot be signed under the grammar and therefore are not subject
/// to this constraint (the identifier branch returns before `parse_length_value`).
///
/// # Accept `lh` / `rlh` and resolve them against the parent
///
/// See the "self-reference" section in the [`Length::Lh`] documentation: CSS Values 4 §6.1.1
/// specifies that `lh`/`rlh` used in a `line-height` **or font-\* property** on the
/// element they reference use the parent's line height or font metrics (or initial
/// values if there is no parent). `font-size` is precisely such a
/// font-\* property, and its grammar does not exclude `lh`/`rlh`
/// (the CSS Fonts 4 `font-size` grammar `<absolute-size> | <relative-size> |
/// <length-percentage [0,∞]>` includes `<length>` through `<length-percentage>`, and
/// the CSS Values 4 §6.1.1 `<length>` production does not exclude `lh`/`rlh`).
///
/// Originally, we dropped these units because passing the parent's computed line height
/// as the basis for font-size resolution seemed more expensive than for `line-height`
/// (`finalize`/`finalize_as_root` already have `parent: &ComputedValues` available).
/// In practice, the implementation was local: it only needed
/// the `self_reference_basis` argument of [`crate::resolve::resolve_font_size`] and
/// a two- or three-line reorder in [`crate::specified::SpecifiedValues::finalize`]
/// (the parent is already computed before this function is called during the
/// tree walk, so no cross-node phase ordering change is needed; see the
/// "expected four stages" section of the [`mod@crate::resolve`] module documentation).
pub(crate) fn parse_font_size(input: &mut Parser<'_, '_>) -> Option<PropertyValue> {
    if let Ok(ident) = input.try_parse(|i| i.expect_ident().cloned()) {
        return parse_font_size_keyword(&ident);
    }
    let length = parse_length_value(input, true)?;
    (length.payload() >= 0.0).then_some(PropertyValue::FontSize(length))
}

/// Parses the identifier portion of `<absolute-size>`, `<relative-size>`, or `math`
/// (a helper for [`parse_font_size`]).
///
/// # `<absolute-size>` scaling-factor table
///
/// CSS Fonts 4 §2.5.1 "Absolute Size Keyword Mapping Table"
/// The table at <https://www.w3.org/TR/css-fonts-4/#absolute-size-mapping> is copied
/// directly (for the same reason as the "do not use an arithmetic expression" policy
/// in `resolve_relative_weight`: keeping fractions allows the rounding-error
/// discussion to rely solely on the specification). `medium` reuses Raikiri's fixed
/// baseline ([`crate::computed::INITIAL_FONT_SIZE_PX`] = 16px; see
/// the documentation for [`crate::specified::SpecifiedValues::initial`]):
///
/// | keyword | xx-small | x-small | small | medium | large | x-large | xx-large | xxx-large |
/// |---|---|---|---|---|---|---|---|---|
/// | factor | 3/5 | 3/4 | 8/9 | 1 | 6/5 | 3/2 | 2/1 | 3/1 |
///
/// The guideline in that section, "an UA applying these guidelines should nevertheless avoid creating
/// font sizes of less than 9 device pixels per EM unit," says "should" (an RFC 2119
/// weak recommendation). The minimum in this table, `xx-small` = `16 * 3/5 = 9.6px`,
/// exceeds 9px, so no clamp is needed. This is not an unsupported feature:
/// a `medium` baseline of 16px already satisfies the guideline.
///
/// # `<relative-size>`
///
/// See the [`RelativeFontSize`] documentation.
///
/// # `math`
///
/// Not implemented (valid under the specification, but unsupported).
fn parse_font_size_keyword(ident: &str) -> Option<PropertyValue> {
    const MEDIUM_PX: f32 = crate::computed::INITIAL_FONT_SIZE_PX;
    let px = match ident.to_ascii_lowercase().as_str() {
        "xx-small" => MEDIUM_PX * (3.0 / 5.0),
        "x-small" => MEDIUM_PX * (3.0 / 4.0),
        "small" => MEDIUM_PX * (8.0 / 9.0),
        "medium" => MEDIUM_PX,
        "large" => MEDIUM_PX * (6.0 / 5.0),
        "x-large" => MEDIUM_PX * (3.0 / 2.0),
        "xx-large" => MEDIUM_PX * (2.0 / 1.0),
        "xxx-large" => MEDIUM_PX * (3.0 / 1.0),
        "larger" => return Some(PropertyValue::FontSizeRelative(RelativeFontSize::Larger)),
        "smaller" => return Some(PropertyValue::FontSizeRelative(RelativeFontSize::Smaller)),
        // `math` ends up here (valid under the specification, but unsupported).
        // Unknown identifiers are also dropped here.
        _ => return None,
    };
    Some(PropertyValue::FontSize(Length::Px(px)))
}

/// Parses `line-height: normal | <number> | <length-percentage>`.
///
/// Grammar: CSS Inline 3 §5.1 "Line Spacing: the line-height property"
/// (<https://www.w3.org/TR/css-inline-3/#line-height-property>) — value
/// There are three alternatives:
///
/// 1. `normal` keyword → [`LineHeight::Normal`]
/// 2. `<number [0,∞]>` is a bare number (Token::Number, without a unit) → [`LineHeight::Number`]
/// 3. `<length-percentage [0,∞]>` → [`LineHeight::Length`] with reused Length variant
///
/// # Number vs Length grammar distinction
///
/// The specification treats `<number>` and `<length-percentage>` as separate alternatives,
/// so they must be distinguished at the token level (Token::Number is unitless,
/// Token::Dimension has a unit, and Token::Percentage is a percentage). Mapping
/// unitless `1.5` and dimensioned `1.5em` to different variants lets downstream
/// paint code distinguish the special rule for unitless numbers ("the specified value
/// is inherited by children," §5.1 "When a child element inherits a computed value...")
/// from ordinary length resolution.
///
/// # Ordering
///
/// `normal` (`try_parse` + `expect_ident_matching`) → bare number
/// First, `try_parse(|i| expect_number_stable(i))` fails and rewinds for Dimension/Percentage;
/// then [`parse_length_value`] (`allow_percentage = true`) is tried. Thus,
/// `1.5` takes the Number branch, whereas `1.5em`, `1.5px`, and `150%` take the Length branch.
///
/// # Non-negative
///
/// The specification's `[0,∞]` rejects negative values in every branch:
/// - Number branch: the `n >= 0.0` guard returns `None` for negatives, dropping the declaration.
/// - Length branch: a `>= 0.0` guard on each inner f32 payload drops negatives.
///
/// A negative value is invalid under the specification and is dropped because the grammar
/// constrains the range at parse time; rejection itself conforms to the specification.
///
/// # Non-goals
///
/// - **(b) Unsupported**: CSS-wide keywords are not implemented yet and are silently dropped.
///   See the "CSS-wide keywords" section of the [`PropertyValue`] documentation for
///   the canonical list of five keywords and the rationale.
/// - **(b) Unsupported**: `calc()` / `var()` are not implemented (css-variables-and-math);
///   both are silently dropped.
/// - **(a) Invalid under the specification → dropped**: negative `<number>` / `<length-percentage>` values;
///   `auto`, `medium`, and other invalid keywords violate the grammar and are dropped.
pub(super) fn parse_line_height(input: &mut Parser<'_, '_>) -> Option<LineHeight> {
    // 1. Match the `normal` keyword (the initial value in the specification).
    if input
        .try_parse(|i| i.expect_ident_matching("normal"))
        .is_ok()
    {
        return Some(LineHeight::Normal);
    }
    // 2. Match a bare `<number [0,∞]>` (Token::Number, without a unit).
    //       For Dimension (`1.5em`) / Percentage (`150%`), `expect_number` returns
    //       Err and `try_parse` rewinds, falling back to the Length branch.
    //       Once a Number token is committed, this branch must decide to accept or drop it:
    //       `try_parse` does not rewind the cursor on an `Ok` path. Rejecting with an outer `&& n >= 0.0`
    //       would leave the consumed cursor in the Length branch, so
    //       `line-height: -0.5 20px` would silently accept `20px`
    //       (a correctness bug that accepts CSS invalid under the specification).
    if let Ok(n) = input.try_parse(|i| expect_number_stable(i)) {
        // A value outside the specification's `<number [0,∞]>` range drops the declaration (do not fall through to Length).
        return (n >= 0.0).then_some(LineHeight::Number(n));
    }
    // 3. Match `<length-percentage [0,∞]>`; the helper accepts every unit and `%`.
    //       Negative values are dropped by a post-filter (the helper deliberately does not check signs;
    //       see "Sign / range" in the parse_length_value documentation).
    let l = parse_length_value(input, true)?;
    // The specification's `[0,∞]` makes a negative value invalid, so drop the declaration.
    (l.payload() >= 0.0).then_some(LineHeight::Length(l))
}

/// Parses `tab-size: <number [0,∞]> | <length [0,∞]>` (CSS Text
/// Module Level 3 §4.2 "Tab Character Size: the tab-size property"
/// <https://www.w3.org/TR/css-text-3/#tab-size-property>).
///
/// # Ordering
///
/// This follows the two-branch shape of [`parse_line_height`]: try a bare `<number>` first,
/// then rewind for Dimension/Percentage and try the Length branch. It omits the `normal`
/// branch because `tab-size` has no `normal` alternative in its grammar.
/// The `<length>` branch uses `allow_percentage = false`
/// (see [`parse_length_value`]; the specification says "Percentages: N/A", and
/// the [`TabSize::Length`] documentation explains why).
///
/// # Non-negative
///
/// The specification's `[0,∞]` applies to both branches, rejecting negatives:
/// - Number branch: `n >= 0.0` returns `None` for negatives and drops the declaration.
/// - Length branch: `>= 0.0` on the inner f32 payload drops negatives.
///
/// The concern in the "commit a Number token" section of the [`parse_line_height`]
/// documentation also applies here: `try_parse` does not rewind an `Ok` path. If an
/// outer check rejects after the Number branch, `tab-size: -1 20px` would silently
/// accept `20px` instead (a correctness bug that accepts invalid CSS).
/// Do not fall through after a committed Number token.
///
/// # Handling `calc()`
///
/// The additive `<length>` form of `calc()` (such as `calc(10px + 0.5em)`) is accepted
/// through the same `parse_text_indent_calc` path as [`parse_word_spacing`] and
/// [`parse_letter_spacing`], and is stored as [`TabSize::Calc`]. Percentage
/// terms are rejected per "Percentages: N/A" in the property definition (see
/// the [`TabSize::Length`] documentation). Calculations with `sign()` or
/// container-relative units such as `cqw` are still dropped because the helper
/// does not accept them (pending general math and container query support).
///
/// # Non-goals
///
/// - **(b) Unsupported**: CSS-wide keywords are not implemented yet and are silently dropped.
///   See the "CSS-wide keywords" section of the [`PropertyValue`] documentation for
///   the canonical list of five keywords and the rationale.
/// - **(b) Unsupported**: `var()` is silently dropped. `calc()` only supports the additive
///   `px` + `em` subset described in "Handling `calc()`" above; calculations with `sign()` or
///   container-relative units remain unsupported and are dropped.
/// - **(a) Invalid under the specification → dropped**: negative `<number>` / `<length>` values,
///   invalid keywords such as `auto`, and percentages violate the grammar and are dropped.
pub(super) fn parse_tab_size(input: &mut Parser<'_, '_>) -> Option<TabSize> {
    // 0. Try additive `<length>` calc with the same helper as `parse_word_spacing`.
    //       Reject percentage terms because the property definition says "Percentages: N/A".
    //       Apply the Length branch's non-negative filter to a collapsed single length
    //       (such as `calc(10px)`). Allow negative results from mixed calc expressions,
    //       deferring the clamp at computed-value time to `resolve_tab_size`
    //       (CSS Values 4 §10.7).
    if let Some(parsed) = parse_text_indent_calc(input) {
        return match parsed {
            TextIndentLength::Length(l) => {
                if matches!(l, Length::Percent(_)) {
                    None
                } else {
                    (l.payload() >= 0.0).then_some(TabSize::Length(l))
                }
            }
            TextIndentLength::Calc(calc) => {
                if calc.percent != 0.0 {
                    None
                } else {
                    Some(TabSize::Calc(calc))
                }
            }
        };
    }
    // 1. Match a bare `<number [0,∞]>` (Token::Number, without a unit). For Dimension
    //       (`4px`), `expect_number` returns Err and `try_parse` rewinds,
    //       falling back to the Length branch.
    if let Ok(n) = input.try_parse(|i| expect_number_stable(i)) {
        // A value outside the specification's `[0,∞]` range drops the declaration; do not fall through to Length.
        // See the documentation above.
        return (n >= 0.0).then_some(TabSize::Number(n));
    }
    // 2. Match `<length [0,∞]>` without percentages (`allow_percentage = false`).
    let l = parse_length_value(input, false)?;
    (l.payload() >= 0.0).then_some(TabSize::Length(l))
}

/// Parse `word-spacing: normal | <length-percentage>`.
///
/// Preserve mixed calc terms until the element's computed font size is known,
/// using the same length-percentage representation as `letter-spacing`.
pub(crate) fn parse_word_spacing(input: &mut Parser<'_, '_>) -> Option<WordSpacingValue> {
    if input
        .try_parse(|i| i.expect_ident_matching("normal"))
        .is_ok()
    {
        return Some(WordSpacingValue::Normal);
    }
    if let Some(parsed) = parse_text_indent_calc_with_ch(input) {
        return Some(match parsed {
            TextIndentLength::Length(length) => WordSpacingValue::Length(length),
            TextIndentLength::Calc(calc) => WordSpacingValue::Calc(calc),
        });
    }
    parse_length_value(input, true).map(WordSpacingValue::Length)
}

/// Parse `letter-spacing: normal | <length-percentage>`.
///
/// Preserve a mixed calc until the element's computed font size is known.
pub(crate) fn parse_letter_spacing(input: &mut Parser<'_, '_>) -> Option<LetterSpacingValue> {
    if input
        .try_parse(|i| i.expect_ident_matching("normal"))
        .is_ok()
    {
        return Some(LetterSpacingValue::Normal);
    }
    if let Some(parsed) = parse_text_indent_calc_with_ch(input) {
        return Some(match parsed {
            TextIndentLength::Length(length) => LetterSpacingValue::Length(length),
            TextIndentLength::Calc(calc) => LetterSpacingValue::Calc(calc),
        });
    }
    parse_length_value(input, true).map(LetterSpacingValue::Length)
}

/// Parses `font-weight: <font-weight-absolute> | bolder | lighter`.
///
/// CSS Fonts 4 §2.2 "Font weight: the font-weight property"
/// <https://www.w3.org/TR/css-fonts-4/#font-weight-prop>:
///
/// ```text
/// <font-weight-absolute> = [ normal | bold | <number [1,1000]> ]
/// ```
///
/// - `normal` = 400 / `bold` = 700 (keyword definitions in §2.2 of the specification).
/// - `bolder` / `lighter` are relative weights that depend on the inherited value.
///   They cannot be resolved during parsing, so they are stored as sentinel variants
///   ([`FontWeightValue::Bolder`] / [`FontWeightValue::Lighter`]), and [`crate::cascade::apply_value`]
///   resolves them from the parent's computed weight.
///
/// ASCII case-insensitive matching follows CSS Values 3 §3.1 "Pre-defined Keywords"
/// <https://www.w3.org/TR/css-values-3/#keywords> (the same convention as the sibling
/// `parse_display` and `parse_content_*` parsers).
///
/// # Range (spec grammar)
///
/// spec §2.2: "Only values greater than or equal to 1, and less than or equal
/// to 1000, are valid, and all other values are invalid." Therefore rejecting `0`,
/// `1001`, and `-100` follows **the specification's grammar itself**, not a stricter
/// policy. Check the **specified value before rounding** ("values" in the specification
/// means the author's `<number>`; `0.6` and `1000.4` are invalid even though rounding
/// would bring them into range).
///
/// Fractional values are preserved as `f32`; relative-weight resolution uses
/// the unrounded computed value.
///
pub(crate) fn parse_font_weight(input: &mut Parser<'_, '_>) -> Option<FontWeightValue> {
    match &next_numeric_stable(input).ok()? {
        // Check `<number [1,1000]>`. Inspect the f32 `value` field, so scientific notation
        // such as `1e3` and fractional values are accepted as written (both are valid
        // productions of CSS Values 3 `<number>`). Carry fractions to the computed value
        // without rounding (see the documentation above).
        // Read tokens via `next_numeric_stable` (see the module's "Numeric-token
        // NaN stabilization" section). It corrects both zero mantissas with huge exponents
        // (`0e999`) and huge mantissas with underflowing exponents (for example,
        // `5` followed by 400 zeros and `e-399`, which equals `50`). Both cssparser
        // tokenizer artifacts have already been corrected here, so this range check
        // will no longer see `NaN`. An actual magnitude overflow to ±inf (`1e400`, etc.)
        // still fails one of the comparisons and is rejected, in agreement with
        // §2.2, "all other values are invalid."
        Token::Number { value, .. } if *value >= 1.0 && *value <= 1000.0 => {
            Some(FontWeightValue::Absolute(*value))
        }
        Token::Ident(name) if name.eq_ignore_ascii_case("normal") => {
            Some(FontWeightValue::Absolute(400.0))
        }
        Token::Ident(name) if name.eq_ignore_ascii_case("bold") => {
            Some(FontWeightValue::Absolute(700.0))
        }
        Token::Ident(name) if name.eq_ignore_ascii_case("bolder") => Some(FontWeightValue::Bolder),
        Token::Ident(name) if name.eq_ignore_ascii_case("lighter") => {
            Some(FontWeightValue::Lighter)
        }
        _ => None,
    }
}

/// Parses `font-style: <ident>` (CSS Fonts 4 §2.4,
/// <https://www.w3.org/TR/css-fonts-4/#font-style-prop>).
///
/// Value grammar (§2.4, full property grammar): `normal | italic | left |
/// right | oblique <angle [-90deg,90deg]>?`. This parser accepts only three keywords:
/// `normal`, `italic`, and bare `oblique` (see the Scope carving section of the
/// [`FontStyle`] documentation). An `<angle>` argument after `oblique`, and `left` / `right`,
/// are valid under the specification but unsupported; like other unknown identifiers,
/// they cause a silent drop (`None`). For `oblique <angle>` (such as `oblique 14deg`),
/// this function consumes only the `oblique` identifier and returns success, but the
/// following `<angle>` token remains unconsumed. The caller's exhaustive-consumption
/// check (the `DeclParser` in [`mod@crate::rule`]) drops the entire declaration
/// (the same mechanism as the "case keyword first" example in [`parse_text_transform`]).
/// Compare identifiers case-insensitively in ASCII (as the sibling [`parse_direction`]
/// parser does).
pub(super) fn parse_text_spacing_trim(input: &mut Parser<'_, '_>) -> Option<TextSpacingTrim> {
    TextSpacingTrim::from_css_ident(input.expect_ident().ok()?)
}

pub(super) fn parse_font_kerning(input: &mut Parser<'_, '_>) -> Option<FontKerning> {
    FontKerning::from_css_ident(input.expect_ident().ok()?)
}

pub(super) fn parse_font_optical_sizing(input: &mut Parser<'_, '_>) -> Option<FontOpticalSizing> {
    FontOpticalSizing::from_css_ident(input.expect_ident().ok()?)
}

pub(super) fn parse_font_variant_emoji(input: &mut Parser<'_, '_>) -> Option<FontVariantEmoji> {
    FontVariantEmoji::from_css_ident(input.expect_ident().ok()?)
}

pub(super) fn parse_font_language_override(
    input: &mut Parser<'_, '_>,
) -> Option<FontLanguageOverride> {
    if let Ok(ident) = input.try_parse(|input| input.expect_ident_cloned()) {
        return ident
            .eq_ignore_ascii_case("normal")
            .then_some(FontLanguageOverride::Normal);
    }

    let value = input.expect_string().ok()?;
    let value = value.as_ref().trim_end_matches(' ');
    Some(FontLanguageOverride::String(SmolStr::new(value)))
}

pub(super) fn parse_font_synthesis(input: &mut Parser<'_, '_>) -> Option<FontSynthesisValue> {
    let mut value = FontSynthesisValue::none();
    let mut seen_any = false;

    while !input.is_exhausted() {
        let ident = input.expect_ident().ok()?.clone();
        if ident.eq_ignore_ascii_case("none") {
            if seen_any || !input.is_exhausted() {
                return None;
            }
            return Some(value);
        }

        seen_any = true;
        if ident.eq_ignore_ascii_case("weight") {
            if value.weight {
                return None;
            }
            value.weight = true;
        } else if ident.eq_ignore_ascii_case("style") {
            if value.style != FontSynthesisStyle::None {
                return None;
            }
            value.style = FontSynthesisStyle::Auto;
        } else if ident.eq_ignore_ascii_case("oblique-only") {
            if value.style != FontSynthesisStyle::None {
                return None;
            }
            value.style = FontSynthesisStyle::ObliqueOnly;
        } else if ident.eq_ignore_ascii_case("small-caps") {
            if value.small_caps {
                return None;
            }
            value.small_caps = true;
        } else if ident.eq_ignore_ascii_case("position") {
            if value.position {
                return None;
            }
            value.position = true;
        } else {
            return None;
        }
    }

    seen_any.then_some(value)
}

pub(super) fn parse_font_variant_ligatures(
    input: &mut Parser<'_, '_>,
) -> Option<FontVariantLigatures> {
    FontVariantLigatures::from_css_ident(input.expect_ident().ok()?)
}

pub(super) fn parse_font_variant_position(
    input: &mut Parser<'_, '_>,
) -> Option<FontVariantPosition> {
    FontVariantPosition::from_css_ident(input.expect_ident().ok()?)
}

pub(super) fn parse_font_palette(input: &mut Parser<'_, '_>) -> Option<FontPaletteValue> {
    let ident = input.expect_ident().ok()?.clone();
    if ident.eq_ignore_ascii_case("normal") {
        Some(FontPaletteValue::Normal)
    } else if ident.eq_ignore_ascii_case("light") {
        Some(FontPaletteValue::Light)
    } else if ident.eq_ignore_ascii_case("dark") {
        Some(FontPaletteValue::Dark)
    } else if ident.starts_with("--") && ident.len() > 2 {
        Some(FontPaletteValue::Palette(SmolStr::new(ident.as_ref())))
    } else {
        None
    }
}

pub(super) fn parse_font_variant_numeric(input: &mut Parser<'_, '_>) -> Option<FontVariantNumeric> {
    let mut value = FontVariantNumeric::initial();
    let mut seen_any = false;

    while !input.is_exhausted() {
        let ident = input.expect_ident().ok()?.clone();
        if ident.eq_ignore_ascii_case("normal") {
            if seen_any || !input.is_exhausted() {
                return None;
            }
            return Some(value);
        }

        seen_any = true;
        match ident.to_ascii_lowercase().as_str() {
            "lining-nums" if !value.lining_nums && !value.oldstyle_nums => {
                value.lining_nums = true;
            }
            "oldstyle-nums" if !value.lining_nums && !value.oldstyle_nums => {
                value.oldstyle_nums = true;
            }
            "proportional-nums" if !value.proportional_nums && !value.tabular_nums => {
                value.proportional_nums = true;
            }
            "tabular-nums" if !value.proportional_nums && !value.tabular_nums => {
                value.tabular_nums = true;
            }
            "diagonal-fractions" if !value.diagonal_fractions && !value.stacked_fractions => {
                value.diagonal_fractions = true;
            }
            "stacked-fractions" if !value.diagonal_fractions && !value.stacked_fractions => {
                value.stacked_fractions = true;
            }
            "ordinal" if !value.ordinal => value.ordinal = true,
            "slashed-zero" if !value.slashed_zero => value.slashed_zero = true,
            _ => return None,
        }
    }

    seen_any.then_some(value)
}

pub(super) fn parse_font_variant_east_asian(
    input: &mut Parser<'_, '_>,
) -> Option<FontVariantEastAsian> {
    let mut value = FontVariantEastAsian::initial();
    let mut seen_any = false;

    while !input.is_exhausted() {
        let ident = input.expect_ident().ok()?.clone();
        if ident.eq_ignore_ascii_case("normal") {
            if seen_any || !input.is_exhausted() {
                return None;
            }
            return Some(value);
        }

        seen_any = true;
        match ident.to_ascii_lowercase().as_str() {
            "jis78" if value.variant.is_none() => {
                value.variant = Some(FontVariantEastAsianVariant::Jis78);
            }
            "jis83" if value.variant.is_none() => {
                value.variant = Some(FontVariantEastAsianVariant::Jis83);
            }
            "jis90" if value.variant.is_none() => {
                value.variant = Some(FontVariantEastAsianVariant::Jis90);
            }
            "jis04" if value.variant.is_none() => {
                value.variant = Some(FontVariantEastAsianVariant::Jis04);
            }
            "simplified" if value.variant.is_none() => {
                value.variant = Some(FontVariantEastAsianVariant::Simplified);
            }
            "traditional" if value.variant.is_none() => {
                value.variant = Some(FontVariantEastAsianVariant::Traditional);
            }
            "full-width" if value.width.is_none() => {
                value.width = Some(FontVariantEastAsianWidth::FullWidth);
            }
            "proportional-width" if value.width.is_none() => {
                value.width = Some(FontVariantEastAsianWidth::ProportionalWidth);
            }
            "ruby" if !value.ruby => value.ruby = true,
            _ => return None,
        }
    }

    seen_any.then_some(value)
}

/// Parse CSS Fonts 4 `font-variation-settings` while preserving the specified list.
///
/// Specified-value serialization retains authored order and duplicate tags;
/// computed-value canonicalization occurs during style finalization.
pub(super) fn parse_font_variation_settings(
    input: &mut Parser<'_, '_>,
) -> Option<FontVariationSettings> {
    if let Ok(ident) = input.try_parse(|input| input.expect_ident_cloned()) {
        return ident
            .eq_ignore_ascii_case("normal")
            .then_some(FontVariationSettings::Normal);
    }

    let mut settings = Vec::new();
    loop {
        let tag = {
            let tag = input.expect_string().ok()?;
            if tag.len() != 4 || !tag.bytes().all(|byte| (0x20..=0x7e).contains(&byte)) {
                return None;
            }
            SmolStr::new(tag.as_ref())
        };
        let value = expect_number_stable(input).ok()?;
        settings.push(FontVariationSetting { tag, value });

        if input.try_parse(|input| input.expect_comma()).is_err() {
            break;
        }
    }

    Some(FontVariationSettings::Settings(settings))
}

pub(super) fn parse_font_style(input: &mut Parser<'_, '_>) -> Option<FontStyle> {
    FontStyle::from_css_ident(input.expect_ident().ok()?)
}

/// Parses `font-variant-caps: <ident>` (CSS Fonts Module Level 3 §6.6,
/// <https://www.w3.org/TR/css-fonts-3/#font-variant-caps-prop>).
///
/// Value grammar (§6.6, full property grammar): `normal | small-caps |
/// all-small-caps | petite-caps | all-petite-caps | unicase |
/// titling-caps`. This parser accepts all seven keywords (see "Meaning of the seven
/// keywords" in the [`FontVariantCaps`] documentation). Other identifiers are
/// invalid under the specification and are silently dropped as unknown (`None`).
/// Compare identifiers case-insensitively in ASCII (as the sibling [`parse_font_style`]
/// parser does).
pub(super) fn parse_font_variant_caps(input: &mut Parser<'_, '_>) -> Option<FontVariantCaps> {
    FontVariantCaps::from_css_ident(input.expect_ident().ok()?)
}

/// Parse the CSS Text `text-transform` grammar.
///
/// The optional case keyword and width keywords may appear in any order. At
/// most one case keyword and each width keyword are accepted. `math-auto` is a
/// separate top-level alternative, not a combinable modifier.
pub(super) fn parse_text_transform(input: &mut Parser<'_, '_>) -> Option<TextTransform> {
    let mut case = None;
    let mut full_width = false;
    let mut full_size_kana = false;
    let mut count = 0;
    while count < 3 {
        let ident = match input.try_parse(|i| i.expect_ident().map(ToOwned::to_owned)) {
            Ok(ident) => ident,
            Err(_) => break,
        };
        count += 1;
        match ident.to_ascii_lowercase().as_str() {
            "none" if count == 1 => return Some(TextTransform::None),
            "math-auto" if count == 1 => return Some(TextTransform::MathAuto),
            "capitalize" if case.is_none() => case = Some(TextTransform::Capitalize),
            "uppercase" if case.is_none() => case = Some(TextTransform::Uppercase),
            "lowercase" if case.is_none() => case = Some(TextTransform::Lowercase),
            "full-width" if !full_width => full_width = true,
            "full-size-kana" if !full_size_kana => full_size_kana = true,
            _ => return None,
        }
    }
    match (case, full_width, full_size_kana) {
        (None, false, false) => None,
        (None, true, false) => Some(TextTransform::FullWidth),
        (None, false, true) => Some(TextTransform::FullSizeKana),
        (None, true, true) => Some(TextTransform::FullWidthFullSizeKana),
        (Some(TextTransform::Capitalize), false, false) => Some(TextTransform::Capitalize),
        (Some(TextTransform::Uppercase), false, false) => Some(TextTransform::Uppercase),
        (Some(TextTransform::Lowercase), false, false) => Some(TextTransform::Lowercase),
        (Some(TextTransform::Capitalize), true, false) => Some(TextTransform::CapitalizeFullWidth),
        (Some(TextTransform::Uppercase), true, false) => Some(TextTransform::UppercaseFullWidth),
        (Some(TextTransform::Lowercase), true, false) => Some(TextTransform::LowercaseFullWidth),
        (Some(TextTransform::Capitalize), false, true) => {
            Some(TextTransform::CapitalizeFullSizeKana)
        }
        (Some(TextTransform::Uppercase), false, true) => Some(TextTransform::UppercaseFullSizeKana),
        (Some(TextTransform::Lowercase), false, true) => Some(TextTransform::LowercaseFullSizeKana),
        (Some(TextTransform::Capitalize), true, true) => {
            Some(TextTransform::CapitalizeFullWidthFullSizeKana)
        }
        (Some(TextTransform::Uppercase), true, true) => {
            Some(TextTransform::UppercaseFullWidthFullSizeKana)
        }
        (Some(TextTransform::Lowercase), true, true) => {
            Some(TextTransform::LowercaseFullWidthFullSizeKana)
        }
        _ => None,
    }
}

/// Parses `word-break: <ident>` (CSS Text 3 §5.1,
/// <https://www.w3.org/TR/css-text-3/#word-break-property>).
///
/// Value grammar (§5.1, full property grammar): `normal | keep-all |
/// break-all | break-word`. This parser accepts only `normal`, `keep-all`, and
/// `break-all` (see the Scope carving section of the [`WordBreak`] documentation).
/// The fourth keyword, `break-word`, is deprecated and equivalent to the combination
/// of `word-break: normal` and `overflow-wrap: anywhere`. Although it is valid under
/// the specification, it is unsupported and is silently dropped (`None`) like other
/// unknown identifiers. Compare identifiers case-insensitively in ASCII (as the sibling
/// `parse_font_style` parser does).
pub(super) fn parse_word_break(input: &mut Parser<'_, '_>) -> Option<WordBreak> {
    WordBreak::from_css_ident(input.expect_ident().ok()?)
}

/// Parses `overflow-wrap: <ident>` (also known by the legacy alias `word-wrap`;
/// see the "legacy alias" section of the [`OverflowWrap`] documentation) (CSS Text 3
/// §5.4, <https://www.w3.org/TR/css-text-3/#overflow-wrap-property>).
///
/// The value grammar (§5.4) is `normal | break-word | anywhere`; all three keywords
/// are accepted. Unlike the deprecated `break-word` of `WordBreak`,
/// `overflow-wrap`'s own `break-word` is a valid, non-deprecated keyword (see
/// the [`OverflowWrap`] documentation). Compare identifiers case-insensitively in ASCII
/// (as the sibling `parse_word_break` parser does).
pub(super) fn parse_overflow_wrap(input: &mut Parser<'_, '_>) -> Option<OverflowWrap> {
    OverflowWrap::from_css_ident(input.expect_ident().ok()?)
}

/// Parses `white-space: <ident>` (CSS Text 3 §3,
/// <https://www.w3.org/TR/css-text-3/#white-space-property>).
///
/// Value grammar (§3, full property grammar): `normal | pre | nowrap |
/// pre-wrap | break-spaces | pre-line`. This parser accepts only five keywords:
/// `normal`, `pre`, `nowrap`, `pre-wrap`, and `pre-line` (see the Scope carving
/// section of the [`WhiteSpace`] documentation). The sixth keyword,
/// `break-spaces`, is valid under the specification but unsupported and is silently
/// dropped (`None`) like other unknown identifiers. Compare identifiers
/// case-insensitively in ASCII (as the sibling `parse_word_break` parser does).
pub(super) fn parse_white_space(input: &mut Parser<'_, '_>) -> Option<WhiteSpace> {
    WhiteSpace::from_css_ident(input.expect_ident().ok()?)
}

/// Parses one `white-space-collapse` keyword (CSS Text 4:
/// <https://www.w3.org/TR/css-text-4/#propdef-white-space-collapse>).
/// Property identifiers are matched ASCII case-insensitively.
pub(super) fn parse_white_space_collapse(input: &mut Parser<'_, '_>) -> Option<WhiteSpaceCollapse> {
    WhiteSpaceCollapse::from_css_ident(input.expect_ident().ok()?)
}

/// Parses `hyphens: <ident>` (CSS Text 3 §5.3,
/// <https://www.w3.org/TR/css-text-3/#hyphens-property>).
///
/// The value grammar (§5.3) is `none | manual | auto`; all three keywords are accepted.
/// Do not collapse `auto` into `manual`. Parsing preserves the separate keyword
/// as [`Hyphens::Auto`] (see the "Downstream handoff" section of the [`Hyphens`]
/// documentation). Whether downstream consumers treat the two values alike is
/// their implementation choice, not this parser's responsibility. Compare
/// identifiers case-insensitively in ASCII (as the sibling `parse_word_break` parser does).
/// Parses the `text-wrap-mode: wrap | nowrap` longhand (CSS Text 4 §5.1).
pub(super) fn parse_text_wrap_mode(input: &mut Parser<'_, '_>) -> Option<TextWrapMode> {
    TextWrapMode::from_css_ident(input.expect_ident().ok()?)
}

pub(super) fn parse_text_wrap_shorthand(input: &mut Parser<'_, '_>) -> Option<TextWrapShorthand> {
    let mut mode = None;
    let mut style = None;
    let mut seen_component = false;

    while !input.is_exhausted() {
        seen_component = true;
        let ident = input.expect_ident().ok()?.clone();
        match ident.to_ascii_lowercase().as_str() {
            "wrap" if mode.is_none() => mode = Some(TextWrapMode::Wrap),
            "nowrap" if mode.is_none() => mode = Some(TextWrapMode::Nowrap),
            "auto" if style.is_none() => style = Some(TextWrapStyle::Auto),
            "balance" if style.is_none() => style = Some(TextWrapStyle::Balance),
            "pretty" if style.is_none() => style = Some(TextWrapStyle::Pretty),
            "stable" if style.is_none() => style = Some(TextWrapStyle::Stable),
            _ => return None,
        }
    }

    seen_component.then_some(TextWrapShorthand {
        mode: mode.unwrap_or(TextWrapMode::Wrap),
        style: style.unwrap_or(TextWrapStyle::Auto),
    })
}

pub(super) fn parse_text_wrap_style(input: &mut Parser<'_, '_>) -> Option<TextWrapStyle> {
    TextWrapStyle::from_css_ident(input.expect_ident().ok()?)
}

// One trim keyword plus at most three autospace boundary keywords and one mode.
const MAX_TEXT_SPACING_COMPONENTS: usize = 5;

/// Parse CSS Text 4 `text-spacing` and expand its aliases to the two longhands.
/// This carries computed-value data only; spacing behavior remains out of scope.
pub(super) fn parse_text_spacing_shorthand(
    input: &mut Parser<'_, '_>,
) -> Option<TextSpacingShorthand> {
    let mut components = Vec::new();
    while !input.is_exhausted() {
        if components.len() == MAX_TEXT_SPACING_COMPONENTS {
            return None;
        }
        components.push(input.expect_ident().ok()?.to_ascii_lowercase());
    }

    let normal = TextSpacingShorthand {
        trim: TextSpacingTrim::Normal,
        autospace: TextAutospace::Normal,
    };
    if components.len() == 1 {
        match components[0].as_str() {
            // CSS-wide `initial` resets both longhands to their initial values.
            "initial" | "normal" => return Some(normal),
            // CSS Text 4 shorthand-only aliases.
            "none" => {
                return Some(TextSpacingShorthand {
                    trim: TextSpacingTrim::SpaceAll,
                    autospace: TextAutospace::NoAutospace,
                });
            }
            "auto" => {
                return Some(TextSpacingShorthand {
                    trim: TextSpacingTrim::Auto,
                    autospace: TextAutospace::Auto,
                });
            }
            _ => {}
        }
    }

    // A lone `<autospace>` component leaves `text-spacing-trim` at its
    // initial value. Reuse the longhand parser so its full token grammar is
    // accepted here as well.
    if let Some(autospace) = parse_text_autospace_components(&components) {
        return Some(TextSpacingShorthand {
            trim: TextSpacingTrim::Normal,
            autospace,
        });
    }

    // `||` permits either component order. Try each possible trim component;
    // the remaining tokens must form one complete `<autospace>` value. This
    // also resolves `normal`/`auto` by the other component when present.
    for trim_index in 0..components.len() {
        let Some(trim) = parse_text_spacing_trim_component(&components[trim_index]) else {
            continue;
        };
        let remaining: Vec<_> = components
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != trim_index)
            .map(|(_, component)| component.clone())
            .collect();
        let Some(autospace) = (if remaining.is_empty() {
            Some(TextAutospace::Normal)
        } else {
            parse_text_autospace_components(&remaining)
        }) else {
            continue;
        };
        return Some(TextSpacingShorthand { trim, autospace });
    }

    None
}

fn parse_text_spacing_trim_component(component: &str) -> Option<TextSpacingTrim> {
    let mut input = ParserInput::new(component);
    let mut parser = Parser::new(&mut input);
    let trim = parse_text_spacing_trim(&mut parser)?;
    parser.expect_exhausted().ok()?;
    Some(trim)
}

fn parse_text_autospace_components(components: &[String]) -> Option<TextAutospace> {
    if components.is_empty() {
        return None;
    }
    let source = components.join(" ");
    let mut input = ParserInput::new(&source);
    let mut parser = Parser::new(&mut input);
    let autospace = parse_text_autospace(&mut parser)?;
    parser.expect_exhausted().ok()?;
    Some(autospace)
}

pub(super) fn parse_hyphens(input: &mut Parser<'_, '_>) -> Option<Hyphens> {
    Hyphens::from_css_ident(input.expect_ident().ok()?)
}

/// Parse `hyphenate-character: auto | <string>` without applying hyphenation.
pub(super) fn parse_hyphenate_character(input: &mut Parser<'_, '_>) -> Option<HyphenateCharacter> {
    if let Ok(ident) = input.try_parse(|input| input.expect_ident_cloned()) {
        return ident
            .as_ref()
            .eq_ignore_ascii_case("auto")
            .then_some(HyphenateCharacter::Auto);
    }

    let value = input.expect_string().ok()?;
    Some(HyphenateCharacter::String(SmolStr::new(value.as_ref())))
}

/// Parse CSS Text 4's one-to-three component `hyphenate-limit-chars` value.
/// The returned model always has three computed components.
pub(super) fn parse_hyphenate_limit_chars(
    input: &mut Parser<'_, '_>,
) -> Option<HyphenateLimitChars> {
    use HyphenateLimitCharsValue::Auto;

    let total = parse_hyphenate_limit_component(input)?;
    let mut before = Auto;
    let mut after = Auto;
    if !input.is_exhausted() {
        before = parse_hyphenate_limit_component(input)?;
        after = before;
        if !input.is_exhausted() {
            after = parse_hyphenate_limit_component(input)?;
            if !input.is_exhausted() {
                return None;
            }
        }
    }
    Some(HyphenateLimitChars {
        total,
        before,
        after,
    })
}

fn parse_hyphenate_limit_component(input: &mut Parser<'_, '_>) -> Option<HyphenateLimitCharsValue> {
    use HyphenateLimitCharsValue::{Auto, Integer};

    if let Ok(ident) = input.try_parse(|input| input.expect_ident_cloned()) {
        return ident.as_ref().eq_ignore_ascii_case("auto").then_some(Auto);
    }

    let start = input.position();
    match input.next().ok()?.clone() {
        Token::Number {
            int_value: Some(value),
            ..
        } => u32::try_from(value).ok().map(Integer),
        Token::Function(name)
            if ["calc", "min", "max", "clamp"]
                .iter()
                .any(|function| name.eq_ignore_ascii_case(function)) =>
        {
            input
                .parse_nested_block(|nested| {
                    while nested.next().is_ok() {}
                    Ok::<_, ParseError<'_, ()>>(())
                })
                .ok()?;
            let source = input.slice(start..input.position());
            let simplified = crate::cascade::simplify_math_functions(source)?;
            parse_hyphenate_limit_math_integer(simplified.as_ref()).map(Integer)
        }
        _ => None,
    }
}

/// Integer-typed CSS math values round to nearest, with exact ties toward
/// positive infinity. Apply the property's non-negative range after rounding.
fn parse_hyphenate_limit_math_integer(source: &str) -> Option<u32> {
    let mut parser_input = cssparser::ParserInput::new(source);
    let mut parser = Parser::new(&mut parser_input);
    let value = match parser.next().ok()?.clone() {
        Token::Number { value, .. } if value.is_finite() => f64::from(value),
        _ => return None,
    };
    parser.expect_exhausted().ok()?;
    let rounded = (value + 0.5).floor();
    if !(0.0..=f64::from(u32::MAX)).contains(&rounded) {
        return None;
    }
    Some(rounded as u32)
}

pub(super) fn parse_line_break(input: &mut Parser<'_, '_>) -> Option<LineBreak> {
    LineBreak::from_css_ident(input.expect_ident().ok()?)
}

pub(super) fn parse_text_justify(input: &mut Parser<'_, '_>) -> Option<TextJustify> {
    TextJustify::from_css_ident(input.expect_ident().ok()?)
}

/// Parses `text-autospace: normal | <autospace> | auto`
/// (CSS Text 4 §6.2.1, <https://drafts.csswg.org/css-text-4/#text-autospace-property>).
/// `<autospace>` is `no-autospace | [ ideograph-alpha || ideograph-numeric ||
/// punctuation ] || [ insert | replace ]`.
pub(super) fn parse_text_autospace(input: &mut Parser<'_, '_>) -> Option<TextAutospace> {
    let first = input.expect_ident().ok()?.clone();
    let first = first.to_ascii_lowercase();
    match first.as_str() {
        "normal" => return Some(TextAutospace::Normal),
        "auto" => return Some(TextAutospace::Auto),
        "no-autospace" => return Some(TextAutospace::NoAutospace),
        _ => {}
    }

    let mut ideograph_alpha = false;
    let mut ideograph_numeric = false;
    let mut punctuation = false;
    let mut mode = TextAutospaceMode::None;

    let mut consume = |ident: &str| -> Option<()> {
        match ident {
            "ideograph-alpha" if !ideograph_alpha => ideograph_alpha = true,
            "ideograph-numeric" if !ideograph_numeric => ideograph_numeric = true,
            "punctuation" if !punctuation => punctuation = true,
            "insert" if matches!(mode, TextAutospaceMode::None) => mode = TextAutospaceMode::Insert,
            "replace" if matches!(mode, TextAutospaceMode::None) => {
                mode = TextAutospaceMode::Replace
            }
            _ => return None,
        }
        Some(())
    };
    consume(first.as_str())?;

    loop {
        match input.next() {
            Ok(Token::Ident(ident)) => consume(ident.to_ascii_lowercase().as_str())?,
            Ok(_) => return None,
            Err(_) => break,
        }
    }

    Some(TextAutospace::Custom {
        ideograph_alpha,
        ideograph_numeric,
        punctuation,
        mode,
    })
}

/// Parse CSS Text 4's `word-space-transform` keyword combination.
///
/// The `&&` grammar permits either order for `auto-phrase` and the space
/// transform; canonical serialization puts the transform first.
pub(super) fn parse_word_space_transform(input: &mut Parser<'_, '_>) -> Option<WordSpaceTransform> {
    let first = input.expect_ident().ok()?.clone();
    let first = first.to_ascii_lowercase();
    if first == "none" {
        return input.is_exhausted().then_some(WordSpaceTransform::None);
    }

    let mut ideographic_space = None;
    let mut auto_phrase = false;
    let mut consume = |ident: &str| -> Option<()> {
        match ident {
            "space" if ideographic_space.is_none() => ideographic_space = Some(false),
            "ideographic-space" if ideographic_space.is_none() => ideographic_space = Some(true),
            "auto-phrase" if !auto_phrase => auto_phrase = true,
            _ => return None,
        }
        Some(())
    };
    consume(first.as_str())?;

    loop {
        match input.next() {
            Ok(Token::Ident(ident)) => consume(ident.to_ascii_lowercase().as_str())?,
            Ok(_) => return None,
            Err(_) => break,
        }
    }

    Some(match (ideographic_space?, auto_phrase) {
        (false, false) => WordSpaceTransform::Space,
        (true, false) => WordSpaceTransform::IdeographicSpace,
        (false, true) => WordSpaceTransform::SpaceAutoPhrase,
        (true, true) => WordSpaceTransform::IdeographicSpaceAutoPhrase,
    })
}

pub(super) fn parse_text_align_all(input: &mut Parser<'_, '_>) -> Option<TextAlignAll> {
    TextAlignAll::from_css_ident(input.expect_ident().ok()?)
}

pub(super) fn parse_text_align_last(input: &mut Parser<'_, '_>) -> Option<TextAlignLast> {
    TextAlignLast::from_css_ident(input.expect_ident().ok()?)
}

/// Parses `display: <ident>`.
///
/// CSS Display 3 §2 "Box Layout Modes: the display property"
/// <https://www.w3.org/TR/css-display-3/#propdef-display>. Currently, it accepts
/// 18 keywords:
///
/// - `block` — `<display-outside>` (block flow)
/// - `inline` — `<display-outside>` (inline flow, the initial value).
/// - `inline-block` — `<display-legacy>` (inline flow-root)
/// - `none` — `<display-box>` (subtree omitted from box tree)
/// - `flex` — a `<display-inside>` (§2.2) keyword, equivalent to `block flex`
///   under the outer-defaulting rule.
/// - `grid` — a `<display-inside>` (§2.2) keyword, equivalent to `block grid`
///   under the outer-defaulting rule.
/// - `list-item` — a `<display-listitem>` keyword, equivalent to `block flow list-item`
///   under the outer-defaulting rule. The default HTML Living Standard UA stylesheet
///   assigns this to `li`
///   (<https://html.spec.whatwg.org/multipage/rendering.html#lists>).
///   As the [`DisplayValue::ListItem`] documentation notes, only the keyword is accepted;
///   generating marker boxes is outside this crate's scope.
/// - `contents` — `<display-box>` (§2.5); the element itself generates no box
///   (see the [`DisplayValue::Contents`] documentation).
/// - `table` — `<display-internal>` (block-level table wrapper)
/// - `inline-table` — `<display-internal>` (inline-level table wrapper)
/// - `table-row-group` — `<display-internal>` (`<tbody>`)
/// - `table-header-group` — `<display-internal>` (`<thead>`)
/// - `table-footer-group` — `<display-internal>` (`<tfoot>`)
/// - `table-row` — `<display-internal>` (`<tr>`)
/// - `table-column-group` — `<display-internal>` (`<colgroup>`)
/// - `table-column` — `<display-internal>` (`<col>`)
/// - `table-cell` — `<display-internal>` (`<td>`, `<th>`)
/// - `table-caption` — `<display-internal>` (`<caption>`)
///
/// The layout bridge currently accepts `inline-flex` and `inline-grid` using the
/// same formatting contexts as `flex` and `grid`, respectively (it does not yet
/// distinguish inline-level shrink-to-fit). Other keywords such as `flow-root` are
/// unsupported and silently dropped (`None`). Compare identifiers case-insensitively
/// §3.1 "Pre-defined Keywords" <https://www.w3.org/TR/css-values-3/#keywords>:
/// in ASCII, as CSS Values 3 requires for keywords.
pub(super) fn parse_text_combine_upright(input: &mut Parser<'_, '_>) -> Option<TextCombineUpright> {
    TextCombineUpright::from_css_ident(input.expect_ident().ok()?)
}

pub(super) fn parse_text_orientation(input: &mut Parser<'_, '_>) -> Option<TextOrientation> {
    TextOrientation::from_css_ident(input.expect_ident().ok()?)
}

pub(super) fn parse_unicode_bidi(input: &mut Parser<'_, '_>) -> Option<UnicodeBidi> {
    UnicodeBidi::from_css_ident(input.expect_ident().ok()?)
}

/// Parses `text-align: <ident>`
/// (CSS Text 3 §6.1, <https://www.w3.org/TR/css-text-3/#text-align-property>).
///
/// Spec value grammar (§6.1): `start | end | left | right | center | justify |
/// match-parent | justify-all`. Additionally, CSS-wide `inherit` for this inherited
/// property and the HTML UA-only `-internal-center` are accepted for the internal cascade.
/// Compare identifiers case-insensitively in ASCII
/// (the CSS convention, also used by sibling [`parse_string_fetch`](super::content::parse_string_fetch) / [`parse_content_part`](super::content::parse_content_part) /
/// [`parse_content_text_keyword`](super::content::parse_content_text_keyword)).
///
/// # Scope carving (detailed in the [`TextAlign`] documentation)
///
/// - **(b) Unsupported**: `<string>` values are silently dropped. They are absent
///   from the grammar in CSS Text 3 §6.1 and were added in CSS Text 4 §7.1
///   <https://www.w3.org/TR/css-text-4/#text-align-property> as an
///   alternative (see §7.2, "Character-based Alignment in a Table
///   Column," for its semantics).
/// - **(b) Unsupported**: CSS-wide keywords are not implemented yet and are silently dropped.
///   See the "CSS-wide keywords" section of the [`PropertyValue`] documentation for
///   the canonical list of five keywords and the rationale.
/// - **(a) Invalid under the specification**: unknown keywords such as `middle` are silently dropped (`None`).
pub(super) fn parse_text_align(input: &mut Parser<'_, '_>) -> Option<TextAlign> {
    TextAlign::from_css_ident(input.expect_ident().ok()?)
}

/// Parses the implemented `hanging-punctuation` subset from CSS Text 3 §8.2.1.
pub(super) fn parse_hanging_punctuation(input: &mut Parser<'_, '_>) -> Option<HangingPunctuation> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "none" => Some(HangingPunctuation::None),
        "first" => Some(HangingPunctuation::First),
        _ => None,
    }
}

/// Parses `direction: <ident>`
/// (CSS Writing Modes 4 §2.1, <https://www.w3.org/TR/css-writing-modes-4/#direction>).
///
/// The value grammar (§2.1) is `ltr | rtl`. Compare identifiers case-insensitively in ASCII
/// (as the sibling [`parse_text_align`] parser does).
///
/// # Scope carving (detailed in the [`Direction`] documentation)
///
/// - **Partially supported**: the computed cascade resolves CSS-wide `inherit` to the parent's value.
///   Other CSS-wide keywords (`initial`, `unset`, `revert`, `revert-layer`) are
///   unsupported and silently dropped.
/// - **(a) Invalid under the specification**: identifiers other than `ltr` / `rtl` are silently dropped (`None`).
pub(super) fn parse_direction(input: &mut Parser<'_, '_>) -> Option<Direction> {
    Direction::from_css_ident(input.expect_ident().ok()?)
}

/// Parses `writing-mode: <ident>`
/// (CSS Writing Modes 4 §3.2, <https://www.w3.org/TR/css-writing-modes-4/#propdef-writing-mode>).
///
/// Spec value grammar (§3.2): `horizontal-tb | vertical-rl | vertical-lr |
/// sideways-rl | sideways-lr`. Compare identifiers case-insensitively in ASCII
/// (as the sibling [`parse_direction`] parser does). All five keywords are accepted
/// as specified. Normalizing computed values for the four keywords starting at `vertical-rl`
/// belongs to [`resolve_writing_mode`], not this function (see the Scope carving
/// section of the [`WritingMode`] documentation).
///
/// # Scope carving (detailed in the [`WritingMode`] documentation)
///
/// - **(b) Unsupported**: CSS-wide keywords are not implemented yet and are silently dropped.
///   See the "CSS-wide keywords" section of the [`PropertyValue`] documentation for
///   the canonical list of five keywords and the rationale.
/// - **(a) Invalid under the specification**: identifiers other than the five listed keywords are silently dropped (`None`).
pub(super) fn parse_writing_mode(input: &mut Parser<'_, '_>) -> Option<WritingMode> {
    WritingMode::from_css_ident(input.expect_ident().ok()?)
}

/// Parse `ruby-position` keywords (CSS Ruby Layout 1 §3).
pub(super) fn parse_ruby_position(input: &mut Parser<'_, '_>) -> Option<RubyPosition> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "over" => Some(RubyPosition::Over),
        "under" => Some(RubyPosition::Under),
        "inter-character" => Some(RubyPosition::InterCharacter),
        _ => None,
    }
}

/// `text-decoration-line: none | [ underline || overline || line-through ||
/// blink ]` (CSS Text Decoration Module Level 3 §2.1,
/// <https://www.w3.org/TR/css-text-decor-3/#text-decoration-line-property>).
///
/// # top-level alternative (`none` vs. `||` combination)
///
/// The grammar is `none | [ ... ]`: `none` cannot be combined with the other four
/// keywords; it is a **separate alternative** (`none underline` is invalid under
/// the specification), not a member of the `||` combination. Try `none` on its own
/// first and return immediately when it matches.
///
/// # `||` (any-order, each-at-most-once) loop
///
/// If `none` does not match, peel four keywords in any order without repetition,
/// using the same per-slot `try_parse` loop as [`parse_border_shorthand`](super::box_model::parse_border_shorthand)
/// (see that function's documentation for the rationale). The identifier sets of
/// the four keywords are disjoint, just as the width/style/color slots of a border
/// shorthand are disjoint: they are simply different words.
///
/// - Try only unfilled flags (bool fields that are not yet true).
/// - A second occurrence of a keyword for an already-filled flag is not tried by
///   that flag's `try_parse`; it falls through without matching and exits the loop.
///   The caller's `expect_exhausted` (the `DeclParser` in [`mod@crate::rule`])
///   detects the leftover token and drops the entire declaration
///   (`text-decoration-line: underline underline` produces zero declarations).
/// - Break when all four flags are filled or no keyword matches.
/// - If no flags were set (neither `none` nor a nonempty `||` combination), return
///   `None`: the specification's `||` grammar requires "one or more of them".
pub(crate) fn parse_text_decoration_line(input: &mut Parser<'_, '_>) -> Option<TextDecorationLine> {
    // Try the top-level `none` alternative, which cannot coexist with `||` (see above).
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(TextDecorationLine::NONE);
    }
    // Try the top-level `spelling-error` / `grammar-error` alternatives. They are mutually
    // exclusive and cannot coexist with the `||` group. Accept a bare identifier only;
    // the caller's `expect_exhausted` drops any leftover tokens
    // (per the grammar `none | [ ... ] | spelling-error | grammar-error`).
    if input
        .try_parse(|i| i.expect_ident_matching("spelling-error"))
        .is_ok()
    {
        return Some(TextDecorationLine::SPELLING_ERROR);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("grammar-error"))
        .is_ok()
    {
        return Some(TextDecorationLine::GRAMMAR_ERROR);
    }

    let mut line = TextDecorationLine::NONE;
    loop {
        if !line.underline
            && input
                .try_parse(|i| i.expect_ident_matching("underline"))
                .is_ok()
        {
            line.underline = true;
            continue;
        }
        if !line.overline
            && input
                .try_parse(|i| i.expect_ident_matching("overline"))
                .is_ok()
        {
            line.overline = true;
            continue;
        }
        if !line.line_through
            && input
                .try_parse(|i| i.expect_ident_matching("line-through"))
                .is_ok()
        {
            line.line_through = true;
            continue;
        }
        if !line.blink
            && input
                .try_parse(|i| i.expect_ident_matching("blink"))
                .is_ok()
        {
            line.blink = true;
            continue;
        }
        break;
    }

    if line == TextDecorationLine::NONE {
        // `none` has already been handled above. Reaching this point means no keyword
        // matched (the identifier is unknown or the value is empty).
        return None;
    }
    Some(line)
}

/// Parses `text-decoration-style: solid | double | dotted | dashed | wavy`.
/// See CSS Text Decoration Module Level 3 §2.2,
/// <https://www.w3.org/TR/css-text-decor-3/#text-decoration-style-property>.
/// Compare identifiers case-insensitively in ASCII (as the sibling
/// [`parse_border_style_side`](super::box_model::parse_border_style_side) parser does).
pub(super) fn parse_text_decoration_style(
    input: &mut Parser<'_, '_>,
) -> Option<TextDecorationStyle> {
    TextDecorationStyle::from_css_ident(input.expect_ident().ok()?)
}

/// Parses `text-decoration-color: <color>` (CSS Text Decoration Module
/// Level 3 §2.3
/// <https://www.w3.org/TR/css-text-decor-3/#text-decoration-color-property>).
///
/// This has the same form as [`parse_border_color`](super::box_model::parse_border_color): it consumes the
/// `currentcolor` keyword (CSS Color 3 §4.4) before delegating to [`parse_color`]
/// (hex / named / `rgb(a)` / `transparent`). It has its own helper because the
/// two properties have different payload types ([`TextDecorationColor`] / [`BorderColor`]);
/// [`parse_border_color`](super::box_model::parse_border_color) remains specific to border-*-color.
pub(super) fn parse_text_decoration_color(
    input: &mut Parser<'_, '_>,
) -> Option<TextDecorationColor> {
    if input
        .try_parse(|i| i.expect_ident_matching("currentcolor"))
        .is_ok()
    {
        return Some(TextDecorationColor::CurrentColor);
    }
    parse_color(input).map(TextDecorationColor::Resolved)
}

/// `text-decoration: <'text-decoration-line'> || <'text-decoration-style'> ||
/// <'text-decoration-color'>` shorthand (CSS Text Decoration
/// Module Level 3 §2.4
/// <https://www.w3.org/TR/css-text-decor-3/#text-decoration-property>).
///
/// This uses the same three-slot `||` loop (line / style / color) as [`parse_border_shorthand`](super::box_model::parse_border_shorthand).
/// See that function's documentation for the loop's rationale, structure, and rules
/// for filling initial values. The identifier/token sets of the three slots are disjoint:
/// line keywords (`none`/`underline`/`overline`/`line-through`/`blink`) and style
/// keywords (`solid`/`double`/`dotted`/`dashed`/`wavy`) are not named CSS colors
/// (they are absent from the table in [`parse_named_color`](cssparser::color::parse_named_color)),
/// so the identifier branch of [`parse_color`] cannot consume them by mistake.
///
/// # Filling initial values for omitted components
///
/// spec §2.4 verbatim: "Omitted values are set to their initial values."
/// - Omitted line → [`TextDecorationLine::NONE`] (§2.1 initial value).
/// - Omitted style → [`TextDecorationStyle::Solid`] (§2.2 initial value).
/// - Omitted color → [`TextDecorationColor::CurrentColor`] (§2.3 initial value).
pub(crate) fn parse_text_decoration_shorthand(
    input: &mut Parser<'_, '_>,
) -> Option<TextDecorationShorthand> {
    let mut line: Option<TextDecorationLine> = None;
    let mut style: Option<TextDecorationStyle> = None;
    let mut color: Option<TextDecorationColor> = None;
    let mut thickness: Option<TextDecorationThickness> = None;

    loop {
        if line.is_some() && style.is_some() && color.is_some() && thickness.is_some() {
            break;
        }
        if line.is_none()
            && let Ok(v) = input.try_parse(|i| -> Result<TextDecorationLine, ParseError<'_, ()>> {
                parse_text_decoration_line(i).ok_or_else(|| i.new_custom_error(()))
            })
        {
            line = Some(v);
            continue;
        }
        if style.is_none()
            && let Ok(v) = input.try_parse(|i| -> Result<TextDecorationStyle, ParseError<'_, ()>> {
                parse_text_decoration_style(i).ok_or_else(|| i.new_custom_error(()))
            })
        {
            style = Some(v);
            continue;
        }
        if color.is_none()
            && let Ok(v) = input.try_parse(|i| -> Result<TextDecorationColor, ParseError<'_, ()>> {
                parse_text_decoration_color(i).ok_or_else(|| i.new_custom_error(()))
            })
        {
            color = Some(v);
            continue;
        }
        if thickness.is_none()
            && let Ok(v) =
                input.try_parse(|i| -> Result<TextDecorationThickness, ParseError<'_, ()>> {
                    parse_text_decoration_thickness(i).ok_or_else(|| i.new_custom_error(()))
                })
        {
            thickness = Some(v);
            continue;
        }
        break;
    }

    // The specification's `||` grammar requires at least one component. Zero components
    // return `None` and drop the declaration (as in `parse_border_shorthand`).
    if line.is_none() && style.is_none() && color.is_none() && thickness.is_none() {
        return None;
    }

    Some(TextDecorationShorthand {
        line: line.unwrap_or(TextDecorationLine::NONE),
        style: style.unwrap_or(TextDecorationStyle::Solid),
        color: color.unwrap_or(TextDecorationColor::CurrentColor),
        thickness: thickness.unwrap_or(TextDecorationThickness::Auto),
    })
}

/// Parses `text-decoration-skip-ink: auto | none | all`
/// (ED §2.10.4, <https://drafts.csswg.org/css-text-decor-4/#text-decoration-skip-ink-property>).
/// Compare identifiers case-insensitively in ASCII (as the sibling [`parse_text_decoration_style`]
/// parser does).
pub(super) fn parse_text_decoration_skip_ink(
    input: &mut Parser<'_, '_>,
) -> Option<TextDecorationSkipInk> {
    TextDecorationSkipInk::from_css_ident(input.expect_ident().ok()?)
}

/// Parses `text-decoration-skip-spaces: none | all | [ start || end ]`
/// (ED §2.10.3, <https://drafts.csswg.org/css-text-decor-4/#text-decoration-skip-spaces-property>).
/// `none` / `all` are separate top-level alternatives; `start` / `end` each appear
/// at most once in the `||` loop (like the four-keyword loop in [`parse_text_decoration_line`]).
pub(super) fn parse_text_decoration_skip_spaces(
    input: &mut Parser<'_, '_>,
) -> Option<TextDecorationSkipSpaces> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(TextDecorationSkipSpaces::None);
    }
    if input.try_parse(|i| i.expect_ident_matching("all")).is_ok() {
        return Some(TextDecorationSkipSpaces::All);
    }
    let mut start = false;
    let mut end = false;
    loop {
        if !start
            && input
                .try_parse(|i| i.expect_ident_matching("start"))
                .is_ok()
        {
            start = true;
            continue;
        }
        if !end && input.try_parse(|i| i.expect_ident_matching("end")).is_ok() {
            end = true;
            continue;
        }
        break;
    }
    match (start, end) {
        (true, false) => Some(TextDecorationSkipSpaces::Start),
        (false, true) => Some(TextDecorationSkipSpaces::End),
        (true, true) => Some(TextDecorationSkipSpaces::StartEnd),
        (false, false) => None,
    }
}

/// Parses `text-decoration-thickness: auto | from-font | <length-percentage>`
/// (ED §2.4.1, <https://drafts.csswg.org/css-text-decor-4/#text-decoration-thickness-property>).
/// `<line-width>` (`thin`/`medium`/`thick`) is out of scope and is dropped
/// (see the [`TextDecorationThickness`] documentation). Delegate `<length-percentage>`
/// to [`parse_length_value`] (`allow_percentage=true`) without a sign check;
/// the specification's grammar imposes no range limit.
pub(super) fn parse_text_decoration_thickness(
    input: &mut Parser<'_, '_>,
) -> Option<TextDecorationThickness> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(TextDecorationThickness::Auto);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("from-font"))
        .is_ok()
    {
        return Some(TextDecorationThickness::FromFont);
    }
    parse_length_value(input, true).map(TextDecorationThickness::Length)
}

/// Parses `text-underline-offset: auto | <length-percentage>`.
///
/// CSS Text Decoration 4 §2.8 defines the non-`auto` form as a
/// `<length-percentage>`. Percentages stay relative in computed style and are
/// resolved against the decorating element's font size at paint time.
pub(super) fn parse_text_underline_offset(
    input: &mut Parser<'_, '_>,
) -> Option<TextUnderlineOffset> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(TextUnderlineOffset::Auto);
    }
    if let Some(value) = parse_text_indent_calc(input) {
        return Some(match value {
            TextIndentLength::Length(length) => TextUnderlineOffset::Length(length),
            TextIndentLength::Calc(calc) => TextUnderlineOffset::Calc(calc),
        });
    }
    parse_length_value(input, true).map(TextUnderlineOffset::Length)
}

/// Parses `text-decoration-inset: <length>{1,2} | auto`
/// (ED §2.9.1, <https://drafts.csswg.org/css-text-decor-4/#text-decoration-inset-property>).
/// The ED grammar specifies `<length-percentage>`, but WPT rejects `%`, so
/// `allow_percentage=false` (see the [`TextDecorationInset`] documentation). Negative
/// values and `calc()` are accepted: the deferred path in [`parse_value`](super::parse_value)
/// converts `calc` to `DeferredValue` beforehand, so only plain [`Length`]
/// values reach this function. For one value, duplicate it as the second value
/// (as in the two-value margin/padding rule).
pub(super) fn parse_text_decoration_inset(
    input: &mut Parser<'_, '_>,
) -> Option<TextDecorationInset> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(TextDecorationInset::Auto);
    }
    let start = parse_length_value(input, false)?;
    let end = input
        .try_parse(|i| -> Result<Length, ParseError<'_, ()>> {
            parse_length_value(i, false).ok_or_else(|| i.new_custom_error(()))
        })
        .unwrap_or(start);
    Some(TextDecorationInset::Lengths { start, end })
}

/// Parses `text-emphasis-position: auto | ([ over | under ] && [ right | left ]?)`
/// (ED §3.4, <https://drafts.csswg.org/css-text-decor-4/#text-emphasis-position-property>).
/// `auto` stands alone. Otherwise, the vertical component (`over`/`under`) is required,
/// and the horizontal component (`right`/`left`) is optional. Either order is allowed, and each appears at most once.
///
/// An `auto` in any other position remains as a leftover token for the caller to drop
/// (the same mechanism used for oblique angles in [`parse_font_style`]).
pub(super) fn parse_text_emphasis_position(
    input: &mut Parser<'_, '_>,
) -> Option<TextEmphasisPosition> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(TextEmphasisPosition::Auto);
    }
    let mut vertical: Option<TextEmphasisVEdge> = None;
    let mut horizontal: Option<TextEmphasisHEdge> = None;
    loop {
        if vertical.is_none() {
            if input.try_parse(|i| i.expect_ident_matching("over")).is_ok() {
                vertical = Some(TextEmphasisVEdge::Over);
                continue;
            }
            if input
                .try_parse(|i| i.expect_ident_matching("under"))
                .is_ok()
            {
                vertical = Some(TextEmphasisVEdge::Under);
                continue;
            }
        }
        if horizontal.is_none() {
            if input
                .try_parse(|i| i.expect_ident_matching("right"))
                .is_ok()
            {
                horizontal = Some(TextEmphasisHEdge::Right);
                continue;
            }
            if input.try_parse(|i| i.expect_ident_matching("left")).is_ok() {
                horizontal = Some(TextEmphasisHEdge::Left);
                continue;
            }
        }
        break;
    }
    Some(TextEmphasisPosition::Position {
        vertical: vertical?,
        horizontal,
    })
}

/// Parse the tested `text-emphasis-style` shape/fill/string forms.
/// Shape and fill keywords may appear in either order; omitted components use
/// the CSS initial shape (`circle`) and fill (`filled`).
pub(super) fn parse_text_emphasis_style(input: &mut Parser<'_, '_>) -> Option<TextEmphasisStyle> {
    if let Ok(value) = input.try_parse(|i| i.expect_string().cloned()) {
        return Some(TextEmphasisStyle::String(SmolStr::new(value.as_ref())));
    }

    let start = input.state();
    let first = input.expect_ident().ok()?.clone();
    let first = first.to_ascii_lowercase();
    if first == "none" {
        return Some(TextEmphasisStyle::None);
    }

    let mut fill = None;
    let mut shape = None;
    let mut consume_ident = |ident: &str| -> bool {
        match ident {
            "filled" if fill.is_none() => fill = Some(TextEmphasisFill::Filled),
            "open" if fill.is_none() => fill = Some(TextEmphasisFill::Open),
            "dot" if shape.is_none() => shape = Some(TextEmphasisShape::Dot),
            "circle" if shape.is_none() => shape = Some(TextEmphasisShape::Circle),
            "double-circle" if shape.is_none() => shape = Some(TextEmphasisShape::DoubleCircle),
            "triangle" if shape.is_none() => shape = Some(TextEmphasisShape::Triangle),
            "sesame" if shape.is_none() => shape = Some(TextEmphasisShape::Sesame),
            _ => return false,
        }
        true
    };
    if !consume_ident(first.as_str()) {
        input.reset(&start);
        return None;
    }

    // Stop before a non-style component so the `text-emphasis` shorthand can
    // parse a following color. A longhand declaration still rejects leftover
    // tokens through the caller's exhaustive-consumption check.
    loop {
        let state = input.state();
        let ident = match input.next() {
            Ok(Token::Ident(ident)) => ident.clone(),
            _ => {
                input.reset(&state);
                break;
            }
        };
        if !consume_ident(ident.to_ascii_lowercase().as_str()) {
            input.reset(&state);
            break;
        }
    }

    let fill = fill.unwrap_or(TextEmphasisFill::Filled);
    Some(match shape {
        Some(shape) => TextEmphasisStyle::Shape { fill, shape },
        None => TextEmphasisStyle::DefaultShape { fill },
    })
}

/// Parse the tested `text-emphasis` shorthand components. Either style or
/// color may be omitted; omitted values use their longhand initials.
pub(super) fn parse_text_emphasis_shorthand(
    input: &mut Parser<'_, '_>,
) -> Option<TextEmphasisShorthand> {
    let mut style = None;
    let mut color = None;
    loop {
        if style.is_none()
            && let Ok(value) =
                input.try_parse(|i| -> Result<TextEmphasisStyle, ParseError<'_, ()>> {
                    parse_text_emphasis_style(i).ok_or_else(|| i.new_custom_error(()))
                })
        {
            style = Some(value);
            continue;
        }
        if color.is_none()
            && let Ok(value) =
                input.try_parse(|i| -> Result<TextDecorationColor, ParseError<'_, ()>> {
                    parse_text_decoration_color(i).ok_or_else(|| i.new_custom_error(()))
                })
        {
            color = Some(value);
            continue;
        }
        break;
    }

    if style.is_none() && color.is_none() {
        return None;
    }
    Some(TextEmphasisShorthand {
        style: style.unwrap_or(TextEmphasisStyle::None),
        color: color.unwrap_or(TextDecorationColor::CurrentColor),
    })
}

/// Parses `text-underline-position: auto | [ from-font | under ] || [ left | right ]`
/// (ED §2.7, <https://drafts.csswg.org/css-text-decor-4/#text-underline-position-property>).
/// The `||` loop has three slots (`from-font`, `under`, and horizontal), followed
/// by checks that reject combining `from-font` and `under`
/// (`under from-font` is invalid) or combining `left` and `right`
/// (`left right` is invalid and is already prevented by the single horizontal slot).
/// `auto` stands alone; the caller drops leftover tokens.
pub(super) fn parse_text_underline_position(
    input: &mut Parser<'_, '_>,
) -> Option<TextUnderlinePosition> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(TextUnderlinePosition::AUTO);
    }
    let mut pos = TextUnderlinePosition::AUTO;
    loop {
        if !pos.from_font
            && !pos.under
            && input
                .try_parse(|i| i.expect_ident_matching("from-font"))
                .is_ok()
        {
            pos.from_font = true;
            continue;
        }
        if !pos.under
            && !pos.from_font
            && input
                .try_parse(|i| i.expect_ident_matching("under"))
                .is_ok()
        {
            pos.under = true;
            continue;
        }
        if !pos.left && !pos.right && input.try_parse(|i| i.expect_ident_matching("left")).is_ok() {
            pos.left = true;
            continue;
        }
        if !pos.right
            && !pos.left
            && input
                .try_parse(|i| i.expect_ident_matching("right"))
                .is_ok()
        {
            pos.right = true;
            continue;
        }
        break;
    }
    if pos == TextUnderlinePosition::AUTO {
        return None;
    }
    Some(pos)
}

/// Parses `vertical-align: <ident> | <length-percentage>` (CSS 2.1 §10.8.1,
/// <https://www.w3.org/TR/CSS21/visudet.html#propdef-vertical-align>).
/// `calc()` functions are handled earlier by `parse_value`'s shared math path;
/// this helper receives their simplified value or a plain length/percentage.
///
/// Compare identifiers case-insensitively in ASCII (as the sibling [`parse_direction`]
/// and [`parse_text_decoration_style`] parsers do). Try the identifier branch first
/// with `try_parse`. If the token is not an identifier (and could be a dimension/number),
/// retry it as `<length>`, following the same keyword-to-length fallback structure as
/// [`parse_flex_basis`](super::layout::parse_flex_basis) and [`parse_letter_spacing`].
/// Both parsers use the same checkpoint-based retry.
///
/// # Scope carving (detailed in the [`VerticalAlign`] documentation)
///
/// - **(b) Unsupported**: `top` / `bottom` are silently dropped (`None`);
///   see the [`VerticalAlign`] documentation (they depend on the inline formatting context).
/// - **(b) Unsupported**: CSS-wide keywords are not implemented yet and are silently dropped.
///   See the "CSS-wide keywords" section of the [`PropertyValue`] documentation for
///   the canonical list of five keywords and the rationale.
/// - **(a) spec-invalid**: `baseline` / `sub` / `super` / `middle` /
///   `text-top` / `text-bottom` are invalid identifiers here. These and tokens outside
///   the `<length>` / `<percentage>` grammar are silently dropped (`None`).
/// - Do not apply a non-negative filter to `<length>` / `<percentage>`; the specification
///   explicitly permits negatives: "Raise (positive value) or lower (negative value)."
///   This follows [`parse_letter_spacing`] and contrasts with the non-negative
///   constraint for `padding`/`border-width`. Percentages are resolved against
///   `used_line_height_length` by [`crate::resolve::resolve_vertical_align`],
///   with a `0px` fallback for `normal`
///   (see that function's documentation).
pub(super) fn parse_vertical_align(input: &mut Parser<'_, '_>) -> Option<VerticalAlign> {
    if let Ok(ident) = input.try_parse(|i| i.expect_ident().cloned()) {
        return match ident.to_ascii_lowercase().as_str() {
            "baseline" => Some(VerticalAlign::Baseline),
            "sub" => Some(VerticalAlign::Sub),
            "super" => Some(VerticalAlign::Super),
            "middle" => Some(VerticalAlign::Middle),
            "text-top" => Some(VerticalAlign::TextTop),
            "text-bottom" => Some(VerticalAlign::TextBottom),
            "top" => Some(VerticalAlign::Top),
            "bottom" => Some(VerticalAlign::Bottom),
            _ => None,
        };
    }
    parse_length_value(input, true).map(VerticalAlign::Length)
}

/// The `<color>` slot of a `text-shadow: <color>? && <length>{2,3}` component.
///
/// This follows the same pattern as [`parse_border_color`](super::box_model::parse_border_color) and [`parse_text_decoration_color`]:
/// it consumes `currentcolor` (CSS Color 3 §4.4) first, then delegates to
/// [`parse_color`] (hex / named / `rgb(a)` / `transparent`).
/// This helper is separate because its payload type differs from the other two:
/// [`TextShadowColor`].
pub(super) fn parse_text_shadow_color(input: &mut Parser<'_, '_>) -> Option<TextShadowColor> {
    if input
        .try_parse(|i| i.expect_ident_matching("currentcolor"))
        .is_ok()
    {
        return Some(TextShadowColor::CurrentColor);
    }
    parse_color(input).map(TextShadowColor::Resolved)
}

/// Parse one `calc()` into the linear `<length>` terms accepted by
/// `text-shadow`. Percentages are rejected even when their coefficients cancel.
fn parse_text_shadow_calc(input: &mut Parser<'_, '_>) -> Option<TextShadowLength> {
    input
        .try_parse(|i| -> Result<TextShadowLength, ParseError<'_, ()>> {
            let start = i.position();
            let parsed = parse_text_indent_calc(i).ok_or_else(|| i.new_custom_error(()))?;
            if math_source_has_percentage(i.slice(start..i.position())) {
                return Err(i.new_custom_error(()));
            }
            let calc = match parsed {
                TextIndentLength::Length(Length::Px(px)) => LengthPercentageCalc {
                    percent: 0.0,
                    px,
                    em: 0.0,
                    ch: 0.0,
                },
                TextIndentLength::Length(Length::Em(em)) => LengthPercentageCalc {
                    percent: 0.0,
                    px: 0.0,
                    em,
                    ch: 0.0,
                },
                TextIndentLength::Calc(calc) => calc,
                TextIndentLength::Length(_) => return Err(i.new_custom_error(())),
            };
            if calc.percent != 0.0 {
                return Err(i.new_custom_error(()));
            }
            Ok(TextShadowLength::Calc {
                px: calc.px,
                em: calc.em,
            })
        })
        .ok()
}

/// Parse a `<length>{2,3}` run for `drop-shadow()`, whose shared parser keeps
/// the existing plain-length behavior. The text-shadow property uses the
/// calc-aware sibling below.
pub(crate) fn parse_text_shadow_lengths<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<(TextShadowLength, TextShadowLength, TextShadowLength), ParseError<'i, ()>> {
    parse_text_shadow_lengths_with_mode(input, false)
}

fn parse_text_shadow_lengths_with_calc<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<(TextShadowLength, TextShadowLength, TextShadowLength), ParseError<'i, ()>> {
    parse_text_shadow_lengths_with_mode(input, true)
}

fn parse_text_shadow_lengths_with_mode<'i>(
    input: &mut Parser<'i, '_>,
    allow_calc: bool,
) -> Result<(TextShadowLength, TextShadowLength, TextShadowLength), ParseError<'i, ()>> {
    let x = if allow_calc {
        if let Some(calc) = parse_text_shadow_calc(input) {
            calc
        } else {
            TextShadowLength::Length(parse_shadow_length_reject_nan_res(input)?)
        }
    } else {
        TextShadowLength::Length(parse_shadow_length_reject_nan_res(input)?)
    };
    let y = if allow_calc {
        if let Some(calc) = parse_text_shadow_calc(input) {
            calc
        } else {
            TextShadowLength::Length(parse_shadow_length_reject_nan_res(input)?)
        }
    } else {
        TextShadowLength::Length(parse_shadow_length_reject_nan_res(input)?)
    };
    let blur = input
        .try_parse(|i| -> Result<TextShadowLength, ParseError<'_, ()>> {
            if allow_calc && let Some(calc) = parse_text_shadow_calc(i) {
                return Ok(calc);
            }
            parse_non_negative_length(i)
                .map(TextShadowLength::Length)
                .ok_or_else(|| i.new_custom_error(()))
        })
        .unwrap_or(TextShadowLength::Length(Length::Px(0.0)));
    Ok((x, y, blur))
}

/// One `text-shadow` entry: `<color>? && <length>{2,3}`.
/// This calc-aware parser is separate from `parse_drop_shadow_item`, which
/// preserves the filter parser's existing plain-length behavior.
///
/// # `&&` (both-required, any-order) grammar semantics
///
/// spec CSS Values 4 §2.2 "Component Value Combinators"
/// <https://www.w3.org/TR/css-values-4/#component-combinators> verbatim: "A
/// double ampersand (&&) separates two or more components, all of which must
/// occur, in any order." For this grammar:
/// - A length run (`<length>{2,3}`) is required and occurs once.
/// - `<color>` occurs zero or one time, as specified by its `?` multiplier.
/// - Either order is valid (`1px 1px red` / `red 1px 1px`), but
///   the length run itself must be contiguous (`1px red 1px` cannot insert
///   `<color>` between the lengths; [`parse_text_shadow_lengths_with_calc`] parses them
///   as one unit).
///
/// # Loop implementation
///
/// Like the `||` loop of [`parse_border_shorthand`](super::box_model::parse_border_shorthand), this loop peels unfilled slots
/// (length run / color) in any order using `try_parse`.
/// Trying the length run first is arbitrary: either order produces the same result,
/// because `try_parse` always rewinds on failure.
pub(super) fn parse_text_shadow_item(input: &mut Parser<'_, '_>) -> Option<TextShadowItem> {
    parse_text_shadow_item_with_mode(input, true)
}

/// Plain-length `TextShadowItem` parser used by `filter: drop-shadow()`.
pub(super) fn parse_drop_shadow_item(input: &mut Parser<'_, '_>) -> Option<TextShadowItem> {
    parse_text_shadow_item_with_mode(input, false)
}

fn parse_text_shadow_item_with_mode(
    input: &mut Parser<'_, '_>,
    allow_calc: bool,
) -> Option<TextShadowItem> {
    let mut lengths: Option<(TextShadowLength, TextShadowLength, TextShadowLength)> = None;
    let mut color: Option<TextShadowColor> = None;

    loop {
        if lengths.is_some() && color.is_some() {
            break;
        }
        if lengths.is_none()
            && let Ok(triple) = input.try_parse(|i| {
                if allow_calc {
                    parse_text_shadow_lengths_with_calc(i)
                } else {
                    parse_text_shadow_lengths(i)
                }
            })
        {
            lengths = Some(triple);
            continue;
        }
        if color.is_none()
            && let Ok(c) = input.try_parse(|i| -> Result<TextShadowColor, ParseError<'_, ()>> {
                parse_text_shadow_color(i).ok_or_else(|| i.new_custom_error(()))
            })
        {
            color = Some(c);
            continue;
        }
        break;
    }

    let (offset_x, offset_y, blur_radius) = lengths?;
    Some(TextShadowItem {
        offset_x,
        offset_y,
        blur_radius,
        color: color.unwrap_or(TextShadowColor::CurrentColor),
    })
}

/// Parses `text-shadow: none | <shadow>#`.
///
/// CSS Text Decoration Module Level 3 §4
/// <https://www.w3.org/TR/css-text-decor-3/#text-shadow-property>.
///
/// `none` means an empty list (a top-level alternative), as in [`parse_content`](super::content::parse_content) and
/// [`parse_counter_property`](super::content::parse_counter_property). Delegate the comma-separated list to
/// `cssparser::Parser::parse_comma_separated`; unconsumed tokens in any item
/// are detected by that method's `parse_until_before` → `parse_entirely`
/// and drop the entire declaration.
pub(super) fn parse_text_shadow(input: &mut Parser<'_, '_>) -> Option<Vec<TextShadowItem>> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(Vec::new());
    }
    input
        .parse_comma_separated(|i| -> Result<TextShadowItem, ParseError<'_, ()>> {
            parse_text_shadow_item(i).ok_or_else(|| i.new_custom_error(()))
        })
        .ok()
}

/// The `font-size` component of the `font` shorthand. [`parse_font_size`] returns
/// two forms ([`PropertyValue::FontSize`] / [`PropertyValue::FontSizeRelative`]); carry them
/// unchanged in a small enum. This is private to the [`FontShorthand`] payload and
/// never enters [`ComputedValues`] / [`SpecifiedValues`] or gets re-exported by
/// the umbrella crate (as with [`BackgroundShorthand`]).
///
/// [`ComputedValues`]: crate::computed::ComputedValues
/// [`SpecifiedValues`]: crate::specified::SpecifiedValues
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FontShorthandSize {
    /// `<length-percentage>` / absolute-size keyword: [`parse_font_size`] returns the
    /// [`PropertyValue::FontSize`] variant, into which this component expands.
    Absolute(Length),
    /// `larger` / `smaller`: [`parse_font_size`] returns the
    /// [`PropertyValue::FontSizeRelative`] variant, into which this component expands
    /// (both use [`PropertyKey::FontSize`] as their key).
    Relative(RelativeFontSize),
}

/// Specified-value carrier for the `font` shorthand: CSS Fonts 4 §2.1, "Font
/// shorthand: the font property"
/// (<https://www.w3.org/TR/css-fonts-4/#font-prop>).
///
/// Spec grammar (full): `[ [ <'font-style'> || <font-variant-css2> ||
/// <'font-weight'> || <font-width-css3> ]? <'font-size'> [ / <'line-height'> ]?
/// <'font-family'># ] | <system-font>`. This implementation supports the following subset:
///
/// # Scope carving
///
/// - **Preface** (three `||` slots): `font-style` uses the range of [`parse_font_style`]
///   (`normal`, `italic`, and bare `oblique`); `font-weight` accepts everything
///   supported by [`parse_font_weight`] (`normal`, `bold`, `bolder`,
///   `lighter`, and `<number [1,1000]>`). `font-variant-css2`
///   (`normal` / `small-caps`) maps to [`FontVariantCaps::Normal`] /
///   [`FontVariantCaps::SmallCaps`]. Only the CSS2 subset is supported, not the
///   full `font-variant` shorthand in CSS Fonts 3 §6.9. Only `font-variant-caps` is
///   parsed from the preface; other supported `font-variant-*` longhands are treated
///   as reset-only subproperties when applying the shorthand.
///   This crate has no `font-width-css3` (`font-stretch`) longhand, so
///   it consumes and discards only `normal` (the initial value, so its reset has
///   no observable effect). Other stretch keywords
///   (such as `condensed`) and variant values outside `font-variant-css2` do not
///   match any preface slot. The subsequent `font-size` parse then fails,
///   dropping the entire declaration (valid but unsupported; silently dropped
///   under this crate's general policy).
/// - **System fonts** (`caption` / `icon` / `menu` / `message-box` /
///   `small-caption` / `status-bar`) are not accepted: decomposition into longhands
///   is UA-dependent and cannot be represented by this crate's font model, so drop the declaration.
/// - **`font-size`** uses [`parse_font_size`] directly (all supported absolute-size
///   keywords, `larger`, `smaller`, and `<length-percentage>` values).
/// - **`line-height`** uses [`parse_line_height`] directly when `/ ...` is present,
///   accepting `normal`, `<number>`, and `<length-percentage>`.
/// - **`font-family`** uses [`parse_font_family`] directly,
///   requiring at least one `<family-name>` in `<family-name>#`.
///
/// # Initial value fill (omitted components)
///
/// CSS Fonts 4 §2.1 verbatim: "The 'font' property is a shorthand for
/// [font-style, font-variant, font-weight, font-size, line-height,
/// font-family]": fill omitted components with their initial values under the specification
/// (the same rule described in the [`BackgroundShorthand`] documentation).
/// The shorthand also resets the currently modeled reset-only subproperties to
/// their initial values: `font-kerning`, `font-language-override`,
/// `font-optical-sizing`, `font-variant-east-asian`, `font-variant-emoji`,
/// `font-variant-ligatures`, `font-variant-numeric`, `font-variant-position`,
/// and `font-variation-settings`. They are not part of the parsed grammar payload;
/// `font-variant-caps` is the grammar's `variant` slot. `font-palette` and
/// `font-synthesis` are not reset by `font`.
/// [`parse_font_shorthand`] fills omitted `style` with [`FontStyle::Normal`],
/// `variant` with [`FontVariantCaps::Normal`], `weight` with `400`, and
/// `line-height` with [`LineHeight::Normal`]. `size` and `family` cannot be
/// omitted. The fill values equal the standalone longhands' initial values
/// (the matching fields of [`crate::specified::SpecifiedValues::initial`] are canonical).
///
/// [`crate::rule::expand_shorthand_into`] expands [`PropertyValue::Font`] into
/// [`PropertyValue::FontStyle`] / [`PropertyValue::FontVariantCaps`] /
/// [`PropertyValue::FontWeight`] / size ([`PropertyValue::FontSize`] /
/// [`PropertyValue::FontSizeRelative`]) / [`PropertyValue::LineHeight`] /
/// [`PropertyValue::FontFamily`] and eight supported reset-only longhands
/// ([`PropertyValue::FontKerning`], [`PropertyValue::FontLanguageOverride`],
/// [`PropertyValue::FontOpticalSizing`], [`PropertyValue::FontVariantEastAsian`],
/// [`PropertyValue::FontVariantEmoji`], [`PropertyValue::FontVariantLigatures`],
/// [`PropertyValue::FontVariantNumeric`], [`PropertyValue::FontVariantPosition`]),
/// and [`PropertyValue::FontVariationSettings`] (`normal` reset), for 15 longhands in total.
/// This follows the margin/padding/border/outline shorthand precedent:
/// "parse-time expansion, never reaches cascade" (see that function's documentation).
///
/// Do not add `#[non_exhaustive]`; as with the sibling shorthand-only carriers
/// ([`GridLineShorthand`] / [`TextDecorationShorthand`] /
/// [`BackgroundShorthand`]), this is only the payload of
/// [`PropertyValue::Font`], not part of [`ComputedValues`] / [`SpecifiedValues`],
/// and is not re-exported from the umbrella crate.
///
/// [`ComputedValues`]: crate::computed::ComputedValues
/// [`SpecifiedValues`]: crate::specified::SpecifiedValues
#[derive(Clone, Debug, PartialEq)]
pub struct FontShorthand {
    /// `font-style` component: defaults to [`FontStyle::Normal`] (initial value in the specification).
    pub style: FontStyle,
    /// `font-variant-css2` component: defaults to [`FontVariantCaps::Normal`]
    /// (initial value in the specification). `small-caps` is [`FontVariantCaps::SmallCaps`].
    pub variant: FontVariantCaps,
    /// `font-weight` component: defaults to `400` (`normal`, the initial value).
    pub weight: FontWeightValue,
    /// Required `font-size` component: see [`FontShorthandSize`].
    pub size: FontShorthandSize,
    /// `line-height` component: defaults to [`LineHeight::Normal`] (the initial value).
    pub line_height: LineHeight,
    /// Required `font-family` component: shares the result of [`parse_font_family`] in an
    /// `Arc` (the same DoS protection pattern as [`PropertyValue::FontFamily`]).
    pub family: Arc<Vec<FontFamilyName>>,
}

/// Parses the `font` shorthand (see the grammar section in [`FontShorthand`]).
///
/// Like the loop in [`parse_background_shorthand`](super::visual::parse_background_shorthand), try each unfilled preface
/// slot in sequence with `try_parse`, fill it on success, and return to the top of the loop.
/// After the preface, parse the required `font-size`, optional `/ line-height`, and required
/// `font-family` in order. If either `font-size` or `font-family` is missing,
/// return `None` and drop the entire declaration. The caller's
/// `expect_exhausted` (the declaration parser in `rule.rs`) drops leftover tokens
/// (the same contract as [`parse_background_shorthand`](super::visual::parse_background_shorthand)).
pub(super) fn parse_font_shorthand(input: &mut Parser<'_, '_>) -> Option<FontShorthand> {
    // Reject system-font keywords first because they cannot be decomposed into longhands;
    // otherwise `font: menu`, etc., might be mistaken for `normal` in the preface.
    // The cursor is not consumed because the check uses `try_parse`.
    let is_system_font = [
        "caption",
        "icon",
        "menu",
        "message-box",
        "small-caption",
        "status-bar",
    ]
    .iter()
    .any(|keyword| {
        input
            .try_parse(|i| i.expect_ident_matching(keyword))
            .is_ok()
    });
    if is_system_font {
        return None;
    }

    let mut style: Option<FontStyle> = None;
    let mut variant: Option<FontVariantCaps> = None;
    let mut weight: Option<FontWeightValue> = None;
    // This crate has no `font-stretch` longhand, so consume and discard only `normal`;
    // see the Scope carving section of the `FontShorthand` documentation.
    let mut stretch_seen = false;

    loop {
        if style.is_none()
            && let Ok(v) = input.try_parse(|i| -> Result<FontStyle, ParseError<'_, ()>> {
                parse_font_style(i).ok_or_else(|| i.new_custom_error(()))
            })
        {
            style = Some(v);
            continue;
        }
        if variant.is_none()
            && let Ok(v) = input.try_parse(|i| -> Result<FontVariantCaps, ParseError<'_, ()>> {
                if i.try_parse(|j| j.expect_ident_matching("normal")).is_ok() {
                    Ok(FontVariantCaps::Normal)
                } else if i
                    .try_parse(|j| j.expect_ident_matching("small-caps"))
                    .is_ok()
                {
                    Ok(FontVariantCaps::SmallCaps)
                } else {
                    Err(i.new_custom_error(()))
                }
            })
        {
            variant = Some(v);
            continue;
        }
        if weight.is_none()
            && let Ok(v) = input.try_parse(|i| -> Result<FontWeightValue, ParseError<'_, ()>> {
                parse_font_weight(i).ok_or_else(|| i.new_custom_error(()))
            })
        {
            weight = Some(v);
            continue;
        }
        if !stretch_seen
            && input
                .try_parse(|i| i.expect_ident_matching("normal"))
                .is_ok()
        {
            stretch_seen = true;
            continue;
        }
        break;
    }

    // Parse the required `font-size`. `parse_font_size` returns a `PropertyValue`,
    // so map it into `FontShorthandSize` (only these two variants can be returned).
    let size = match parse_font_size(input)? {
        PropertyValue::FontSize(length) => FontShorthandSize::Absolute(length),
        PropertyValue::FontSizeRelative(relative) => FontShorthandSize::Relative(relative),
        // cov:ignore: `parse_font_size` returns only the two variants above;
        // its two return paths are canonical. This branch is unreachable.
        _ => return None,
    };

    // Parse the optional `/ line-height`.
    let line_height = if input.try_parse(|i| i.expect_delim('/')).is_ok() {
        parse_line_height(input)?
    } else {
        LineHeight::Normal
    };

    // Parse the required `font-family` (`<family-name>#` with at least one element).
    let family = parse_font_family(input)?;

    Some(FontShorthand {
        style: style.unwrap_or(FontStyle::Normal),
        variant: variant.unwrap_or(FontVariantCaps::Normal),
        weight: weight.unwrap_or(FontWeightValue::Absolute(400.0)),
        size,
        line_height,
        family: Arc::new(family),
    })
}
