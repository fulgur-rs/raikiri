//! Box-model property parsers: margin, padding, border, sizing, overflow,
//! positioning, border-radius, box-shadow and outline.

use cssparser::{ParseError, Parser};
use smol_str::SmolStr;

use crate::property::types::*;

use super::color::*;
use super::common::*;
use super::text::*;

/// Parse `border-*-color` values by recognizing `currentcolor` first, then delegating other colors to
/// [`parse_color`].
///
/// CSS Backgrounds 3 §3.1 <https://www.w3.org/TR/css-backgrounds-3/#border-color> defines the
/// `border-*-color` grammar as `<color>`. That production includes the `currentcolor` keyword from
/// CSS Color 3 §4.4 <https://www.w3.org/TR/css-color-3/#currentColor-def>. This crate's
/// [`parse_color`] uses cssparser's `parse_named_color` (an RGB triple mapping), whose named-color table
/// excludes `currentcolor`. The helper must recognize that identifier first. See the [`BorderColor`] enum
/// doc for why used-value resolution belongs to painting.
///
pub(super) fn parse_border_color(input: &mut Parser<'_, '_>) -> Option<BorderColor> {
    // `expect_ident_matching` follows cssparser's ASCII case-insensitive matching convention. Like
    // sibling `parse_margin_side`, this intercepts a keyword before trying other forms. On failure,
    // `try_parse` rewinds so `parse_color` can handle Ident (named / transparent), Hash, or Function.
    if input
        .try_parse(|i| i.expect_ident_matching("currentcolor"))
        .is_ok()
    {
        return Some(BorderColor::CurrentColor);
    }
    parse_color(input).map(BorderColor::Resolved)
}

/// Parse `<length-percentage> | auto` for one margin longhand side.
///
/// Grammar reference: CSS Box 3 §3.1 <https://www.w3.org/TR/css-box-3/#margin-physical> "Value:
/// `<length-percentage> | auto`".
///
/// # Order of alternatives
///
/// Try the `auto` identifier first. [`parse_length_value`] consumes `input.next()` even on failure, so
/// trying length first would discard the identifier in `margin: auto` before the `auto` branch could run.
/// `try_parse` establishes a checkpoint and rewinds on failure, as in the bare `<string>` literal branch
/// of [`parse_content_list_items`](super::content::parse_content_list_items).
///
/// `expect_ident_matching` is ASCII case-insensitive, so `AUTO` and `Auto`
/// are accepted as well.
pub(crate) fn parse_margin_side(input: &mut Parser<'_, '_>) -> Option<LengthOrAuto> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(LengthOrAuto::Auto);
    }
    parse_length_value(input, true).map(LengthOrAuto::Length)
}

/// Expand the 1–4 values of the `margin: <'margin-top'>{1,4}` shorthand.
///
/// Grammar reference: CSS Box 3 §3.2 <https://www.w3.org/TR/css-box-3/#margin-shorthand>.
///
/// # Expansion rules (spec verbatim, §3.2)
///
/// "If there is only one component value, it applies to all sides. If there
/// are two values, the top and bottom margins are set to the first value and
/// the right and left margins are set to the second. If there are three
/// values, the top is set to the first value, the left and right are set to
/// the second, and the bottom is set to the third. If there are four values
/// they apply to the top, right, bottom, and left, respectively."
///
/// # Trailing garbage handling
///
/// With five values (`margin: 10px 20px 30px 40px 50px`), this helper consumes the first four and
/// leaves the last token unconsumed. The caller's `DeclParser` in [`mod@crate::rule`] detects the extra
/// token with `expect_exhausted` in its [`cssparser::DeclarationParser::parse_value`] implementation
/// and drops the entire declaration. This matches the handling of [`parse_font_family`] and the
/// `rejects_extra_length_after_font_size` test.
pub(super) fn parse_margin_shorthand(input: &mut Parser<'_, '_>) -> Option<Sides<LengthOrAuto>> {
    let v1 = parse_margin_side(input)?;
    // Without a second value, apply the first to all four sides (§3.2: "If there is only one component
    // value, it applies to all sides").
    let Some(v2) = input.try_parse(|i| parse_margin_side(i).ok_or(())).ok() else {
        return Some(Sides::all(v1));
    };
    // Without a third value: top/bottom = first, right/left = second.
    let Some(v3) = input.try_parse(|i| parse_margin_side(i).ok_or(())).ok() else {
        return Some(Sides {
            top: v1,
            right: v2,
            bottom: v1,
            left: v2,
        });
    };
    // Without a fourth value: top = first, right/left = second, bottom = third.
    let Some(v4) = input.try_parse(|i| parse_margin_side(i).ok_or(())).ok() else {
        return Some(Sides {
            top: v1,
            right: v2,
            bottom: v3,
            left: v2,
        });
    };
    // Four values go clockwise from the top (top, right, bottom, left). This helper leaves any fifth
    // value unconsumed for the caller's `expect_exhausted` to reject. The property.rs test
    // `margin_shorthand_leaves_extra_values_for_caller_exhausted_check` checks this helper; the rule.rs
    // test `margin_shorthand_five_values_declaration_dropped` checks the complete declaration.
    Some(Sides {
        top: v1,
        right: v2,
        bottom: v3,
        left: v4,
    })
}

/// Expand `margin-inline: <'margin-top'>{1,2}` / `margin-block: <'margin-top'>{1,2}` shorthand to
/// [`StartEnd<LengthOrAuto>`] — 1-2 value expansion.
///
/// Grammar reference: CSS Logical Properties and Values 1 §4.2
/// <https://www.w3.org/TR/css-logical-1/#propdef-margin-inline>: "The first value represents the start edge
/// style, and the second value represents the end edge style. If only one value is given, it applies to
/// both the start and end edges." The `margin-block` property definition uses the same wording and
/// the same grammar (`<'margin-top'>{1,2}`), this helper serves both `margin-inline` and `margin-block`.
/// The caller chooses a `PropertyValue` variant to select the physical axis (left/right or top/bottom);
/// this function does not know that axis. See the [`PropertyValue::MarginInline`] doc.
///
/// # Robustness
///
/// This helper leaves a third or later value unconsumed. The caller (`rule.rs::DeclParser`) detects
/// the leftover with `expect_exhausted` and drops the declaration, as described under "Trailing garbage
/// handling" in [`parse_margin_shorthand`].
pub(super) fn parse_margin_logical_shorthand(
    input: &mut Parser<'_, '_>,
) -> Option<StartEnd<LengthOrAuto>> {
    let start = parse_margin_side(input)?;
    // Without a second value, apply the first to both edges (spec: "If only one value is given, it
    // applies to both the start and end edges").
    let Some(end) = input.try_parse(|i| parse_margin_side(i).ok_or(())).ok() else {
        return Some(StartEnd::both(start));
    };
    Some(StartEnd { start, end })
}

/// Parse `width: auto | <length-percentage [0,∞]>`.
///
/// Grammar reference: CSS Sizing 3 §3.1.1 <https://www.w3.org/TR/css-sizing-3/#preferred-size-properties>
/// "Value: `auto | <length-percentage [0,∞]> | min-content | max-content |
/// fit-content(<length-percentage>)`", "Initial: auto", "Inherited: no".
///
/// # Not supported (spec-valid, future support)
///
/// The intrinsic sizing forms `min-content`, `max-content`, and `fit-content()` are not implemented.
/// They return `None`: `expect_ident_matching("auto")` rejects their identifiers, and the following
/// `parse_length_value` rejects their keyword or function tokens in its Dimension / Percentage match.
/// Negative values (`width: -10px`) also violate the specification's `[0,∞]` grammar and are dropped.
///
/// # Order of alternatives
///
/// Like [`parse_margin_side`], try the `auto` identifier branch first. [`parse_length_value`] consumes
/// `input.next()` unconditionally, even when parsing fails. If the length branch ran first, it would
/// discard the `auto` identifier in `width: auto` before the fallback could match it.
/// `try_parse` establishes a checkpoint and rewinds on failure.
///
/// # Non-negative constraint
///
/// As in [`parse_padding_side`], an OR-pattern checks every [`Length`] variant at parse time for the
/// closed interval `[0,∞]` (Verification #4: `width: -10px` → `None` → declaration dropped). The only
/// structural difference is the preceding `auto` branch: padding's `<length-percentage [0,∞]>` grammar
/// does not allow `auto`.
pub(super) fn parse_width(input: &mut Parser<'_, '_>) -> Option<LengthOrAuto> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(LengthOrAuto::Auto);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("min-content"))
        .is_ok()
    {
        return Some(LengthOrAuto::MinContent);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("max-content"))
        .is_ok()
    {
        return Some(LengthOrAuto::Auto);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("fit-content"))
        .is_ok()
    {
        let _ = input.try_parse(|i| {
            i.expect_parenthesis_block()?;
            i.parse_nested_block(|nested| {
                parse_length_value(nested, true)
                    .ok_or_else(|| nested.new_custom_error::<_, ()>(()))?;
                Ok::<_, cssparser::ParseError<'_, ()>>(())
            })
        });
        return Some(LengthOrAuto::Auto);
    }
    if input
        .try_parse(|i| {
            i.expect_function_matching("fit-content")?;
            i.parse_nested_block(|nested| {
                parse_length_value(nested, true)
                    .ok_or_else(|| nested.new_custom_error::<_, ()>(()))?;
                nested.expect_exhausted()?;
                Ok::<_, cssparser::ParseError<'_, ()>>(())
            })
        })
        .is_ok()
    {
        return Some(LengthOrAuto::Auto);
    }
    let length = parse_length_value(input, true)?;
    (length.payload() >= 0.0).then_some(LengthOrAuto::Length(length))
}

/// Parse the single-side value of `padding-{top,right,bottom,left}`.
///
/// Grammar: `<length-percentage [0,∞]>` (CSS Box 3 §4.1
/// <https://www.w3.org/TR/css-box-3/#padding-physical>). The specification states verbatim: "Negative
/// values for padding properties are invalid." A negative value invalidates the declaration.
///
/// # Implementation note
///
/// 1. Call [`parse_length_value`] with `allow_percentage=true` for the `<length-percentage>` grammar.
///    Unsupported dimension units, the `auto` keyword, and other nonnumeric tokens return `None`, as
///    they do in the font-size parser.
/// 2. Check that every [`Length`] variant's payload (via [`Length::payload`]) is `>= 0.0`.
///    Negative values return `None`, including `Percent(-10.0)` = `-10%` (Verification #5).
///
/// # Sibling pattern
///
/// This follows the `<length-percentage>` tail of [`parse_font_size`] after its identifier branch
/// returns `None`. Both call [`parse_length_value`] with `allow_percentage=true`, extract payloads
/// through [`Length::payload`] for every [`Length`] variant, then check `>= 0.0`. Their tail bodies match.
/// The preceding identifier branch (`parse_font_size_keyword`) for `<absolute-size>`, `<relative-size>`,
/// and `math` now makes the complete functions different, but not those tails.
///
/// An older description of their asymmetry no longer applies: font-size once accepted only px lengths,
/// while padding accepted all five `<length-percentage>` variants. Support for font-relative units
/// removed that difference.
pub(super) fn parse_padding_side(input: &mut Parser<'_, '_>) -> Option<Length> {
    let length = parse_length_value(input, true)?;
    // CSS Box 3 §4.1: "Negative values for padding properties are invalid."
    (length.payload() >= 0.0).then_some(length)
}

/// Expand `padding: <'padding-top'>{1,4}` shorthand to [`Sides<Length>`].
///
/// The 1–4 value expansion follows CSS Box 3 §4.2
/// <https://www.w3.org/TR/css-box-3/#padding-shorthand>. This is a paraphrase, not a verbatim quote:
///
/// - 1 value: all 4 sides = value
/// - 2 values: top/bottom = first, left/right = second
/// - 3 values: top = first, left/right = second, bottom = third
/// - 4 values: top / right / bottom / left (clockwise from top)
///
/// # Robustness
///
/// - This function leaves a fifth or later value unconsumed. The caller (`rule.rs::DeclParser`) uses
///   `expect_exhausted` to drop the entire declaration (`padding: 1px 2px 3px 4px 5px`).
/// - With no values, the first `parse_padding_side` returns `None`, which propagates to the whole parse.
/// - [`parse_padding_side`] enforces each value's nonnegative constraint. For `padding: 10px -5px`,
///   the second value returns `None` and the whole declaration is dropped.
pub(super) fn parse_padding_shorthand(input: &mut Parser<'_, '_>) -> Option<Sides<Length>> {
    // The first value is required. Without it, the 0-value form violates the grammar.
    let v1 = parse_padding_side(input)?;
    // Values 2–4 are optional and parsed with successive `try_parse` calls. Each failed call rewinds,
    // so if one value is absent, later calls see the same token and fail too; no guard is needed.
    let v2 = input.try_parse(parse_padding_side_res).ok();
    let v3 = input.try_parse(parse_padding_side_res).ok();
    let v4 = input.try_parse(parse_padding_side_res).ok();
    // CSS Box 3 §4.2: 1–4 value expansion (this is a paraphrase, not a specification quote).
    let sides = match (v2, v3, v4) {
        (None, _, _) => Sides::all(v1),
        (Some(h), None, _) => Sides {
            top: v1,
            right: h,
            bottom: v1,
            left: h,
        },
        (Some(h), Some(b), None) => Sides {
            top: v1,
            right: h,
            bottom: b,
            left: h,
        },
        (Some(r), Some(b), Some(l)) => Sides {
            top: v1,
            right: r,
            bottom: b,
            left: l,
        },
    };
    Some(sides)
}

/// `Result` version of [`parse_padding_side`] — `try_parse` requires `Result` in a closure, so it is a
/// wrapper.
pub(super) fn parse_padding_side_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<Length, ParseError<'i, ()>> {
    parse_padding_side(input).ok_or_else(|| input.new_custom_error(()))
}

/// Expand `padding-inline: <'padding-top'>{1,2}` / `padding-block: <'padding-top'>{1,2}` shorthand to
/// [`StartEnd<Length>`] — 1-2 value expansion.
///
/// Grammar reference: CSS Logical Properties and Values 1 §4.4
/// <https://www.w3.org/TR/css-logical-1/#propdef-padding-inline> — same
/// "first value = start, second value = end, one value spreads to both"
/// text as [`parse_margin_logical_shorthand`] documents in full for the
/// margin sibling; `padding-block`'s propdef shares the same grammar, so
/// this one helper backs both parser arms (axis choice is the caller's,
/// via which `PropertyValue` variant wraps the result).
///
/// # Robustness
///
/// A negative first value returns `None` through [`parse_padding_side`] and `?`. If the second value
/// is invalid, `try_parse` rewinds and the helper temporarily returns the one-value `Some` form. The
/// invalid token remains unconsumed, so the caller (`rule.rs::DeclParser`) detects it through
/// `expect_exhausted` and drops the declaration. See "Robustness" in [`parse_padding_shorthand`].
pub(super) fn parse_padding_logical_shorthand(
    input: &mut Parser<'_, '_>,
) -> Option<StartEnd<Length>> {
    let start = parse_padding_side(input)?;
    let end = input.try_parse(parse_padding_side_res).ok();
    Some(match end {
        None => StartEnd::both(start),
        Some(end) => StartEnd { start, end },
    })
}

/// Pixel value for the `border-width` `medium` keyword (its specification-defined initial value).
///
/// CSS Backgrounds 3 §3.3 "Line Thickness: the border-width properties"
/// (<https://www.w3.org/TR/css-backgrounds-3/#border-width>) states verbatim: "The thin, medium,
/// and thick keywords are equivalent to 1px, 3px, and 5px, respectively." Unlike `font-size`'s
/// `medium` (a UA choice; see [`crate::computed::INITIAL_FONT_SIZE_PX`]), these exact values are
/// normative requirements of the specification, not Raikiri choices.
///
/// The implementation uses one shared constant for the `medium` keyword and
/// the omitted shorthand width. The initial border value reuses the same
/// constant.
pub(crate) const BORDER_WIDTH_MEDIUM_PX: f32 = 3.0;

/// Parse the single-side value of `border-{top,right,bottom,left}-width`.
///
/// Grammar: `<line-width>` = `<length [0,∞]> | thin | medium | thick` (CSS Backgrounds 3 §3.3
/// <https://www.w3.org/TR/css-backgrounds-3/#border-width>). Unlike padding, this grammar excludes
/// `<percentage>`; pass `parse_length_value(input, false)` to select `<length>` mode.
///
/// # Keyword mapping (specification-defined values)
///
/// Section 3.3 defines three keyword values verbatim: "The thin, medium, and thick keywords are
/// equivalent to 1px, 3px, and 5px, respectively." The mapping is:
/// - `thin`   → `Length::Px(1.0)`
/// - `medium` → `Length::Px(3.0)` (initial value)
/// - `thick`  → `Length::Px(5.0)`
///
/// These values come from the specification, not UA discretion. Using them does not depend on another
/// implementation; Chromium, Firefox, and WebKit also use the same mapping.
///
/// # Sign / range
///
/// This helper enforces the `<length [0,∞]>` constraint: a negative value returns `None` and drops the
/// declaration. It uses the same post-filter pattern as [`parse_padding_side`], but never creates a
/// `Length::Percent` variant because `allow_percentage=false` rejects Percentage tokens.
///
/// # Non-goals
///
/// - **(a) spec-invalid → drop**: Negative value (`-1px`), unknown keyword (`fat`, etc.), spec-invalid unit
///   (`%` is not included in grammar → drop).
/// - **(b) Unsupported**: CSS-wide keywords are not yet implemented and are silently dropped. The
///   "CSS-wide keyword" section of the [`PropertyValue`] doc lists the five keywords and explains why.
/// - **(b) Unsupported**: `calc()` / `var()` are not yet implemented and are silently dropped.
pub(super) fn parse_border_width_side(input: &mut Parser<'_, '_>) -> Option<Length> {
    // 1. Try thin / medium / thick first. `parse_length_value` consumes a token even on failure, so
    // `try_parse` must rewind first, as in the `auto` branch of `parse_margin_side`.
    let keyword = input.try_parse(|i| -> Result<Length, ParseError<'_, ()>> {
        let ident = i.expect_ident()?.clone();
        match ident.to_ascii_lowercase().as_str() {
            "thin" => Ok(Length::Px(1.0)),
            "medium" => Ok(Length::Px(BORDER_WIDTH_MEDIUM_PX)),
            "thick" => Ok(Length::Px(5.0)),
            _ => Err(i.new_custom_error(())),
        }
    });
    if let Ok(l) = keyword {
        return Some(l);
    }
    // 2. `<length [0,∞]>` — `<length>` mode with allow_percentage=false (Percentage token is rejected,
    // `<percentage>` is outside the grammar).
    let length = parse_length_value(input, false)?;
    // Enforce `<length [0,∞]>`. `Length::payload` also works for `Percent`, but
    // `allow_percentage=false` prevents that variant from reaching this function. The variant is
    // unreachable here, not dead code: the shared helper also serves other properties.
    (length.payload() >= 0.0).then_some(length)
}

/// `Result` version of [`parse_border_width_side`] — `try_parse` requires `Result` within the closure, so
/// it is made into a wrapper (same pattern as [`parse_padding_side_res`]).
fn parse_border_width_side_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<Length, ParseError<'i, ()>> {
    parse_border_width_side(input).ok_or_else(|| input.new_custom_error(()))
}

/// Parse the single-side value of `border-{top,right,bottom,left}-style`.
///
/// Grammar: `<line-style>` = `none | hidden | dotted | dashed | solid | double | groove | ridge | inset |
/// outset` (CSS Backgrounds 3 §3.2 <https://www.w3.org/TR/css-backgrounds-3/#border-style>).
/// Match identifiers ASCII case-insensitively, as do
/// [`parse_display`](super::layout::parse_display) and [`parse_text_align`].
///
/// # Non-goals
///
/// - **(a) Spec-invalid → drop**: Unknown keywords such as `wavy` are silently dropped.
/// - **(b) Unsupported**: CSS-wide keywords are not yet implemented and are silently dropped. The
///   "CSS-wide keyword" section of the [`PropertyValue`] doc lists the five keywords and explains why.
pub(super) fn parse_border_style_side(input: &mut Parser<'_, '_>) -> Option<BorderStyle> {
    BorderStyle::from_css_ident(input.expect_ident().ok()?)
}

/// `Result` version of `parse_border_style_side` (for `try_parse`).
fn parse_border_style_side_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<BorderStyle, ParseError<'i, ()>> {
    parse_border_style_side(input).ok_or_else(|| input.new_custom_error(()))
}

/// `Result` version of `parse_border_color` (for `try_parse`).
fn parse_border_color_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<BorderColor, ParseError<'i, ()>> {
    parse_border_color(input).ok_or_else(|| input.new_custom_error(()))
}

/// Parse a single CSS-wide keyword (`inherit` / `initial` / `unset` / `revert` /
/// `revert-layer`) ASCII case-insensitively.
///
/// CSS Cascading 4 §7.3 and CSS Cascading 5 §7.3.5 require every property to accept
/// these keywords as a lone value. Border longhands and the `border` / `border-right`
/// shorthands implement that contract through [`CssWideKeyword`]; other properties
/// keep their existing silent-drop behavior. The caller decides the payload mapping;
/// this helper only recognizes the keyword.
///
/// Returns `None` for any other identifier so the caller can fall through to its
/// component grammar. A CSS-wide keyword combined with other components
/// (`border-right: inherit solid`) is rejected downstream by the caller's
/// `expect_exhausted` (see [`crate::rule`] `DeclParser`), not here.
pub(super) fn parse_css_wide_keyword(input: &mut Parser<'_, '_>) -> Option<CssWideKeyword> {
    let ident = input.expect_ident().ok()?.clone();
    CssWideKeyword::from_css_ident(ident.as_ref())
}

/// `Result` version of [`parse_css_wide_keyword`] for `try_parse`.
pub(super) fn parse_css_wide_keyword_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<CssWideKeyword, ParseError<'i, ()>> {
    parse_css_wide_keyword(input).ok_or_else(|| input.new_custom_error(()))
}

/// Parse `border-style: <line-style>{1,4}` (CSS Backgrounds 3 §3.4). Expand values as in
/// [`parse_padding_shorthand`]: one for all sides, two for vertical/horizontal, three for
/// top/horizontal/bottom, or four clockwise. The caller's `expect_exhausted` rejects a fifth value.
pub(super) fn parse_border_style_shorthand(
    input: &mut Parser<'_, '_>,
) -> Option<Sides<BorderStyle>> {
    let v1 = parse_border_style_side(input)?;
    let v2 = input.try_parse(parse_border_style_side_res).ok();
    let v3 = input.try_parse(parse_border_style_side_res).ok();
    let v4 = input.try_parse(parse_border_style_side_res).ok();
    let sides = match (v2, v3, v4) {
        (None, _, _) => Sides::all(v1),
        (Some(h), None, _) => Sides {
            top: v1,
            right: h,
            bottom: v1,
            left: h,
        },
        (Some(h), Some(b), None) => Sides {
            top: v1,
            right: h,
            bottom: b,
            left: h,
        },
        (Some(r), Some(b), Some(l)) => Sides {
            top: v1,
            right: r,
            bottom: b,
            left: l,
        },
    };
    Some(sides)
}

/// Parse `border-width: <line-width>{1,4}` (CSS Backgrounds 3 §3.4). Each value follows
/// [`parse_border_width_side`] (thin/medium/thick or a nonnegative `<length>`). Expand one to four
/// values as in [`parse_padding_shorthand`].
pub(super) fn parse_border_width_shorthand(input: &mut Parser<'_, '_>) -> Option<Sides<Length>> {
    let v1 = parse_border_width_side(input)?;
    let v2 = input.try_parse(parse_border_width_side_res).ok();
    let v3 = input.try_parse(parse_border_width_side_res).ok();
    let v4 = input.try_parse(parse_border_width_side_res).ok();
    let sides = match (v2, v3, v4) {
        (None, _, _) => Sides::all(v1),
        (Some(h), None, _) => Sides {
            top: v1,
            right: h,
            bottom: v1,
            left: h,
        },
        (Some(h), Some(b), None) => Sides {
            top: v1,
            right: h,
            bottom: b,
            left: h,
        },
        (Some(r), Some(b), Some(l)) => Sides {
            top: v1,
            right: r,
            bottom: b,
            left: l,
        },
    };
    Some(sides)
}

/// Parse `border-color: <color>{1,4}` (CSS Backgrounds 3 §3.4). Each value follows
/// [`parse_border_color`] (`currentcolor`, named, hash, or function). Expand one to four values as in
/// [`parse_padding_shorthand`].
pub(super) fn parse_border_color_shorthand(
    input: &mut Parser<'_, '_>,
) -> Option<Sides<BorderColor>> {
    let v1 = parse_border_color(input)?;
    let v2 = input.try_parse(parse_border_color_res).ok();
    let v3 = input.try_parse(parse_border_color_res).ok();
    let v4 = input.try_parse(parse_border_color_res).ok();
    let sides = match (v2, v3, v4) {
        (None, _, _) => Sides::all(v1),
        (Some(h), None, _) => Sides {
            top: v1,
            right: h,
            bottom: v1,
            left: h,
        },
        (Some(h), Some(b), None) => Sides {
            top: v1,
            right: h,
            bottom: b,
            left: h,
        },
        (Some(r), Some(b), Some(l)) => Sides {
            top: v1,
            right: r,
            bottom: b,
            left: l,
        },
    };
    Some(sides)
}

/// Parse `border: <line-width> || <line-style> || <color>` shorthand.
///
/// CSS Backgrounds 3 §3.4 <https://www.w3.org/TR/css-backgrounds-3/#border-shorthands>.
/// Distribute the same [`Border`] to all four sides with `Sides::all`.
///
/// # `||` (any-order) grammar semantics
///
/// CSS Values 4 §2.2 "Component Value Combinators"
/// <https://www.w3.org/TR/css-values-4/#component-combinators> states verbatim: "A double bar (||) separates two
/// or more options: one or more of them must occur, in any order." — In this shorthand:
/// - Each component occurs at most once; a second value for the same slot invalidates the declaration.
/// - At least one component is required; an empty `border:` declaration is dropped.
/// - Components can occur in any order (`1px solid red`, `red 1px solid`, and `solid 1px` are valid).
///
/// # Loop implementation
///
/// In a loop, fill the width, style, and color slots:
/// 1. Use `try_parse` to try each available slot, independent of token order.
/// 2. Stop when a token would fill an occupied slot. For `border: 1px 2px`, `1px` fills width and
///    `2px` cannot fill it again. The caller's `expect_exhausted` drops the leftover and the declaration.
/// 3. Break when all slots are filled or no parser matches.
/// 4. `Some` if at least 1 slot is filled, `None` if 0 slot is filled.
///
/// # Initial value fill (omitted component)
///
/// Section 3.4 states verbatim: "Omitted values are set to their initial values." For each component:
/// - width omitted → `Length::Px(3.0)` (medium initial)
/// - style omitted → `BorderStyle::None` (initial, spec §3.2)
/// - color omitted → [`BorderColor::CurrentColor`] (spec §3.1 initial; used-value resolution is the
///   paint layer's responsibility).
///
/// # Non-goals (spec deviation explicit)
///
/// Section 3.4 also requires the border shorthand to reset `border-image-*` properties (verbatim:
/// "The border shorthand also resets border-image to its initial value."). This crate does not yet
/// implement border-image, so it omits that reset. Add it when border-image longhands are implemented.
///
/// # Sibling pattern
///
/// [`parse_margin_shorthand`] and [`parse_padding_shorthand`] use `{1,4}`: values have a fixed order
/// and may differ by side. This shorthand uses `||`: components appear in any order, and each side gets
/// the same result. All three use `try_parse` to rewind and fill omitted components with initial values.
pub(crate) fn parse_border_shorthand(input: &mut Parser<'_, '_>) -> Option<Sides<Border>> {
    let mut width: Option<Length> = None;
    let mut style: Option<BorderStyle> = None;
    let mut color: Option<BorderColor> = None;

    // `||` grammar: at least 1 component required, each component at most once, order optional. Break when
    // all slots are filled or no parser matches the next token.
    //
    // Each iteration tries parsers for unfilled slots in order, continuing on a match and breaking if none
    // matches. Checking whether slots are full before `continue` ensures a second value for an occupied
    // slot (`border: 1px 2px`) falls through and breaks. The caller's `expect_exhausted` then sees the remaining
    // token and drops the declaration.
    loop {
        // All slots filled → break (the caller drops leftover tokens through `expect_exhausted`).
        if width.is_some() && style.is_some() && color.is_some() {
            break;
        }

        // Width slot: try it only if empty, using a helper for thin/medium/thick and lengths.
        // `try_parse` rewinds on failure. A Rust 1.88+ let-chain keeps the nested `let Ok(..)` check
        // in one conditional and avoids clippy::collapsible-if.
        if width.is_none()
            && let Ok(v) = input.try_parse(parse_border_width_side_res)
        {
            width = Some(v);
            continue;
        }

        // Style slot: fill it if the identifier matches one of ten keywords; rewind on failure.
        // Width keywords `thin` / `medium` / `thick` are tried first and do not overlap with style
        // keywords such as `none` / `solid`.
        if style.is_none()
            && let Ok(s) = input.try_parse(|i| -> Result<BorderStyle, ParseError<'_, ()>> {
                parse_border_style_side(i).ok_or_else(|| i.new_custom_error(()))
            })
        {
            style = Some(s);
            continue;
        }

        // Color slot: reuse `parse_border_color` for hex, named colors, rgb(a), transparent, and
        // `currentcolor` (CSS Color 3 §4.4). The four border-{top,right,bottom,left}-color
        // longhands use the same helper.
        if color.is_none()
            && let Ok(c) = input.try_parse(|i| -> Result<BorderColor, ParseError<'_, ()>> {
                parse_border_color(i).ok_or_else(|| i.new_custom_error(()))
            })
        {
            color = Some(c);
            continue;
        }

        // No unfilled slot matches: the token is unknown or repeats a filled slot. Break and leave it
        // for the caller's `expect_exhausted` to reject. For `border: 1px 2px`, `2px` cannot fill the
        // already occupied width slot, so the entire declaration is dropped.
        break;
    }

    // The `||` grammar requires at least one component. An empty `border:` declaration or a single
    // unknown keyword has none, so return `None` and drop the declaration.
    if width.is_none() && style.is_none() && color.is_none() {
        return None;
    }

    // Fill in the omitted components with the initial value from spec §3.4.
    let border = Border {
        width: width.unwrap_or(Length::Px(BORDER_WIDTH_MEDIUM_PX)), // medium
        style: style.unwrap_or(BorderStyle::None),
        // §3.1 initial "currentcolor" — used-value resolution belongs to the paint layer.
        color: color.unwrap_or(BorderColor::CurrentColor),
    };
    Some(Sides::all(border))
}

/// Parse `border-top: <line-width> || <line-style> || <color>` single-side shorthand.
///
/// CSS Backgrounds 3 §3.4 <https://www.w3.org/TR/css-backgrounds-3/#border-shorthands>
/// defines `border-top` as the three top-side longhands with the same `||`
/// component grammar as [`parse_border_shorthand`]. Only the expansion target differs:
/// one [`Border`] for the top side instead of [`Sides::all`] for all four.
///
/// The `||` loop, occupied-slot rejection, empty-declaration drop, and omitted-component
/// initial fill all match [`parse_border_shorthand`]. CSS-wide keywords are not accepted
/// here; the `parse_value` dispatch tries [`parse_css_wide_keyword`] first and maps it
/// to [`PropertyValue::BorderTopCssWide`], so a lone `inherit` never reaches this
/// function. A combined `inherit solid` therefore fails `expect_exhausted` downstream
/// rather than being silently truncated.
pub(crate) fn parse_border_top_shorthand(input: &mut Parser<'_, '_>) -> Option<Border> {
    let mut width: Option<Length> = None;
    let mut style: Option<BorderStyle> = None;
    let mut color: Option<BorderColor> = None;

    loop {
        if width.is_some() && style.is_some() && color.is_some() {
            break;
        }
        if width.is_none()
            && let Ok(v) = input.try_parse(parse_border_width_side_res)
        {
            width = Some(v);
            continue;
        }
        if style.is_none()
            && let Ok(s) = input.try_parse(|i| -> Result<BorderStyle, ParseError<'_, ()>> {
                parse_border_style_side(i).ok_or_else(|| i.new_custom_error(()))
            })
        {
            style = Some(s);
            continue;
        }
        if color.is_none()
            && let Ok(c) = input.try_parse(|i| -> Result<BorderColor, ParseError<'_, ()>> {
                parse_border_color(i).ok_or_else(|| i.new_custom_error(()))
            })
        {
            color = Some(c);
            continue;
        }
        break;
    }
    if width.is_none() && style.is_none() && color.is_none() {
        return None;
    }
    Some(Border {
        width: width.unwrap_or(Length::Px(BORDER_WIDTH_MEDIUM_PX)),
        style: style.unwrap_or(BorderStyle::None),
        color: color.unwrap_or(BorderColor::CurrentColor),
    })
}

/// Parse `border-right: <line-width> || <line-style> || <color>` single-side shorthand.
///
/// CSS Backgrounds 3 §3.4 <https://www.w3.org/TR/css-backgrounds-3/#border-shorthands>
/// defines `border-right` as the three right-side longhands with the same `||`
/// component grammar as [`parse_border_shorthand`]. Only the expansion target differs:
/// one [`Border`] for the right side instead of [`Sides::all`] for all four.
///
/// The `||` loop, occupied-slot rejection, empty-declaration drop, and omitted-component
/// initial fill all match [`parse_border_shorthand`]. CSS-wide keywords are not accepted
/// here; the `parse_value` dispatch tries [`parse_css_wide_keyword`] first and maps it
/// to [`PropertyValue::BorderRightCssWide`], so a lone `inherit` never reaches this
/// function. A combined `inherit solid` therefore fails `expect_exhausted` downstream
/// rather than being silently truncated.
pub(crate) fn parse_border_right_shorthand(input: &mut Parser<'_, '_>) -> Option<Border> {
    let mut width: Option<Length> = None;
    let mut style: Option<BorderStyle> = None;
    let mut color: Option<BorderColor> = None;

    loop {
        if width.is_some() && style.is_some() && color.is_some() {
            break;
        }
        if width.is_none()
            && let Ok(v) = input.try_parse(parse_border_width_side_res)
        {
            width = Some(v);
            continue;
        }
        if style.is_none()
            && let Ok(s) = input.try_parse(|i| -> Result<BorderStyle, ParseError<'_, ()>> {
                parse_border_style_side(i).ok_or_else(|| i.new_custom_error(()))
            })
        {
            style = Some(s);
            continue;
        }
        if color.is_none()
            && let Ok(c) = input.try_parse(|i| -> Result<BorderColor, ParseError<'_, ()>> {
                parse_border_color(i).ok_or_else(|| i.new_custom_error(()))
            })
        {
            color = Some(c);
            continue;
        }
        break;
    }
    if width.is_none() && style.is_none() && color.is_none() {
        return None;
    }
    Some(Border {
        width: width.unwrap_or(Length::Px(BORDER_WIDTH_MEDIUM_PX)),
        style: style.unwrap_or(BorderStyle::None),
        color: color.unwrap_or(BorderColor::CurrentColor),
    })
}

/// Parse `border-bottom: <line-width> || <line-style> || <color>` single-side shorthand.
///
/// CSS Backgrounds 3 §3.4 <https://www.w3.org/TR/css-backgrounds-3/#border-shorthands>
/// defines `border-bottom` as the three bottom-side longhands with the same `||`
/// component grammar as [`parse_border_shorthand`]. Only the expansion target differs:
/// one [`Border`] for the bottom side instead of [`Sides::all`] for all four.
///
/// The `||` loop, occupied-slot rejection, empty-declaration drop, and omitted-component
/// initial fill all match [`parse_border_shorthand`]. CSS-wide keywords are not accepted
/// here; the `parse_value` dispatch tries [`parse_css_wide_keyword`] first and maps it
/// to [`PropertyValue::BorderBottomCssWide`], so a lone `inherit` never reaches this
/// function. A combined `inherit solid` therefore fails `expect_exhausted` downstream
/// rather than being silently truncated.
pub(crate) fn parse_border_bottom_shorthand(input: &mut Parser<'_, '_>) -> Option<Border> {
    let mut width: Option<Length> = None;
    let mut style: Option<BorderStyle> = None;
    let mut color: Option<BorderColor> = None;

    loop {
        if width.is_some() && style.is_some() && color.is_some() {
            break;
        }
        if width.is_none()
            && let Ok(v) = input.try_parse(parse_border_width_side_res)
        {
            width = Some(v);
            continue;
        }
        if style.is_none()
            && let Ok(s) = input.try_parse(|i| -> Result<BorderStyle, ParseError<'_, ()>> {
                parse_border_style_side(i).ok_or_else(|| i.new_custom_error(()))
            })
        {
            style = Some(s);
            continue;
        }
        if color.is_none()
            && let Ok(c) = input.try_parse(|i| -> Result<BorderColor, ParseError<'_, ()>> {
                parse_border_color(i).ok_or_else(|| i.new_custom_error(()))
            })
        {
            color = Some(c);
            continue;
        }
        break;
    }
    if width.is_none() && style.is_none() && color.is_none() {
        return None;
    }
    Some(Border {
        width: width.unwrap_or(Length::Px(BORDER_WIDTH_MEDIUM_PX)),
        style: style.unwrap_or(BorderStyle::None),
        color: color.unwrap_or(BorderColor::CurrentColor),
    })
}

/// Parse `border-left: <line-width> || <line-style> || <color>` single-side shorthand.
///
/// CSS Backgrounds 3 §3.4 <https://www.w3.org/TR/css-backgrounds-3/#border-shorthands>
/// defines `border-left` as the three left-side longhands with the same `||`
/// component grammar as [`parse_border_shorthand`]. Only the expansion target differs:
/// one [`Border`] for the left side instead of [`Sides::all`] for all four.
///
/// The `||` loop, occupied-slot rejection, empty-declaration drop, and omitted-component
/// initial fill all match [`parse_border_shorthand`]. CSS-wide keywords are not accepted
/// here; the `parse_value` dispatch tries [`parse_css_wide_keyword`] first and maps it
/// to [`PropertyValue::BorderLeftCssWide`], so a lone `inherit` never reaches this
/// function. A combined `inherit solid` therefore fails `expect_exhausted` downstream
/// rather than being silently truncated.
pub(crate) fn parse_border_left_shorthand(input: &mut Parser<'_, '_>) -> Option<Border> {
    let mut width: Option<Length> = None;
    let mut style: Option<BorderStyle> = None;
    let mut color: Option<BorderColor> = None;

    loop {
        if width.is_some() && style.is_some() && color.is_some() {
            break;
        }
        if width.is_none()
            && let Ok(v) = input.try_parse(parse_border_width_side_res)
        {
            width = Some(v);
            continue;
        }
        if style.is_none()
            && let Ok(s) = input.try_parse(|i| -> Result<BorderStyle, ParseError<'_, ()>> {
                parse_border_style_side(i).ok_or_else(|| i.new_custom_error(()))
            })
        {
            style = Some(s);
            continue;
        }
        if color.is_none()
            && let Ok(c) = input.try_parse(|i| -> Result<BorderColor, ParseError<'_, ()>> {
                parse_border_color(i).ok_or_else(|| i.new_custom_error(()))
            })
        {
            color = Some(c);
            continue;
        }
        break;
    }
    if width.is_none() && style.is_none() && color.is_none() {
        return None;
    }
    Some(Border {
        width: width.unwrap_or(Length::Px(BORDER_WIDTH_MEDIUM_PX)),
        style: style.unwrap_or(BorderStyle::None),
        color: color.unwrap_or(BorderColor::CurrentColor),
    })
}

/// Parse `height: <length-percentage [0,∞]> | auto`.
///
/// Grammar reference: CSS Sizing 3 §3.1.1 "Preferred Size Properties"
/// <https://www.w3.org/TR/css-sizing-3/#preferred-size-properties>. The value grammar is `auto |
/// <length-percentage [0,∞]> | min-content | max-content | fit-content(<length-percentage>)`;
/// the initial value is `auto`, and inheritance is `No`.
///
/// # Scope carving
///
/// - **(a) spec-invalid → drop**: Negative value (`height: -10px`) violates grammar `[0,∞]`, rejected by
///   `>= 0.0` post-filter for every [`Length`] variant payload, as in
///   [`parse_padding_side`].
/// - **(b) Unsupported sizing forms**: `min-content`, `max-content`, and
///   `fit-content(<length-percentage>)` are outside the current scope and are silently dropped. They fail
///   `expect_ident_matching("auto")`, then the length parser rejects their keyword or function tokens
///   rather than matching a Dimension / Percentage.
/// - **(b) Unsupported — CSS-wide keywords**: Not yet implemented. The identifier parser silently
///   drops them like other unknown keywords. The "CSS-wide keyword" section of the [`PropertyValue`]
///   doc lists the five keywords and explains why.
/// - **calc() / var()**: Not implemented, outside this task scope (for `Token::Function`,
///   `parse_length_value` silently drops anything other than Dimension / Percentage).
///
/// # Order of alternative (sibling: [`parse_margin_side`])
///
/// Try the `auto` identifier branch **first**. [`parse_length_value`] consumes `input.next()` even on
/// failure; trying length first would discard the identifier in `height: auto` before the `auto` match
/// could run. `try_parse` establishes a checkpoint and rewinds on failure. This follows
/// [`parse_margin_side`], whose `<length-percentage> | auto` grammar also uses a `LengthOrAuto` payload.
///
/// `expect_ident_matching` is ASCII case-insensitive, so `AUTO` and `Auto`
/// are accepted as well.
///
/// # Non-negative filter (sibling: [`parse_padding_side`])
///
/// To enforce the specification's `<length-percentage [0,∞]>` constraint (§3.1.1), use
/// [`Length::payload`] to check every [`Length`] variant's payload against `>= 0.0`. This matches the nonnegative filter
/// in [`parse_padding_side`]. It also rejects `Percent(-10.0)` (`-10%`).
pub(super) fn parse_height(input: &mut Parser<'_, '_>) -> Option<LengthOrAuto> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(LengthOrAuto::Auto);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("min-content"))
        .is_ok()
    {
        return Some(LengthOrAuto::Auto);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("max-content"))
        .is_ok()
    {
        return Some(LengthOrAuto::Auto);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("fit-content"))
        .is_ok()
    {
        let _ = input.try_parse(|i| {
            i.expect_parenthesis_block()?;
            i.parse_nested_block(|nested| {
                parse_length_value(nested, true)
                    .ok_or_else(|| nested.new_custom_error::<_, ()>(()))?;
                Ok::<_, cssparser::ParseError<'_, ()>>(())
            })
        });
        return Some(LengthOrAuto::Auto);
    }
    if input
        .try_parse(|i| {
            i.expect_function_matching("fit-content")?;
            i.parse_nested_block(|nested| {
                parse_length_value(nested, true)
                    .ok_or_else(|| nested.new_custom_error::<_, ()>(()))?;
                nested.expect_exhausted()?;
                Ok::<_, cssparser::ParseError<'_, ()>>(())
            })
        })
        .is_ok()
    {
        return Some(LengthOrAuto::Auto);
    }
    let length = parse_length_value(input, true)?;
    (length.payload() >= 0.0).then_some(LengthOrAuto::Length(length))
}

/// Parse `min-width` / `min-height` / `min-block-size: auto | <length-percentage [0,∞]> | min-content | max-content | fit-content`.
///
/// CSS Sizing 3 §4 "Minimum Size Properties"
/// <https://www.w3.org/TR/css-sizing-3/#min-size-properties> specifies initial value `auto` and
/// inheritance `No`. This follows the shape of [`parse_max_size`] (CSS Sizing 3 §5), replacing its
/// max-only `none` branch with the minimum size's `auto` branch. Both enforce `[0,∞]` by filtering
/// through [`Length::payload`] and map intrinsic keywords to an Auto placeholder.
pub(super) fn parse_max_size(input: &mut Parser<'_, '_>) -> Option<LengthOrAuto> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(LengthOrAuto::Auto);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("min-content"))
        .is_ok()
    {
        return Some(LengthOrAuto::Auto);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("max-content"))
        .is_ok()
    {
        return Some(LengthOrAuto::Auto);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("fit-content"))
        .is_ok()
    {
        let _ = input.try_parse(|i| {
            i.expect_parenthesis_block()?;
            i.parse_nested_block(|nested| {
                parse_length_value(nested, true)
                    .ok_or_else(|| nested.new_custom_error::<_, ()>(()))?;
                Ok::<_, cssparser::ParseError<'_, ()>>(())
            })
        });
        return Some(LengthOrAuto::Auto);
    }
    if input
        .try_parse(|i| {
            i.expect_function_matching("fit-content")?;
            i.parse_nested_block(|nested| {
                parse_length_value(nested, true)
                    .ok_or_else(|| nested.new_custom_error::<_, ()>(()))?;
                nested.expect_exhausted()?;
                Ok::<_, cssparser::ParseError<'_, ()>>(())
            })
        })
        .is_ok()
    {
        return Some(LengthOrAuto::Auto);
    }
    let length = parse_length_value(input, true)?;
    (length.payload() >= 0.0).then_some(LengthOrAuto::Length(length))
}

/// Parse `min-width` / `min-height: auto | <length-percentage [0,∞]> | min-content | max-content | fit-content`.
///
/// CSS Sizing 3 §4 "Minimum Size Properties"
/// <https://www.w3.org/TR/css-sizing-3/#min-size-properties> specifies initial value `auto` and
/// inheritance `No`. Like `parse_max_size` (CSS Sizing 3 §5), this parser filters `[0,∞]` through
/// `Length::payload` and maps intrinsic keywords to an Auto placeholder. Only the first keyword branch
/// changes: minimum sizes use `auto`, while the maximum size grammar alone uses `none`.
pub(super) fn parse_min_size(input: &mut Parser<'_, '_>) -> Option<LengthOrAuto> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(LengthOrAuto::Auto);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("min-content"))
        .is_ok()
    {
        return Some(LengthOrAuto::Auto);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("max-content"))
        .is_ok()
    {
        return Some(LengthOrAuto::Auto);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("fit-content"))
        .is_ok()
    {
        let _ = input.try_parse(|i| {
            i.expect_parenthesis_block()?;
            i.parse_nested_block(|nested| {
                parse_length_value(nested, true)
                    .ok_or_else(|| nested.new_custom_error::<_, ()>(()))?;
                Ok::<_, cssparser::ParseError<'_, ()>>(())
            })
        });
        return Some(LengthOrAuto::Auto);
    }
    if input
        .try_parse(|i| {
            i.expect_function_matching("fit-content")?;
            i.parse_nested_block(|nested| {
                parse_length_value(nested, true)
                    .ok_or_else(|| nested.new_custom_error::<_, ()>(()))?;
                nested.expect_exhausted()?;
                Ok::<_, cssparser::ParseError<'_, ()>>(())
            })
        })
        .is_ok()
    {
        return Some(LengthOrAuto::Auto);
    }
    let length = parse_length_value(input, true)?;
    (length.payload() >= 0.0).then_some(LengthOrAuto::Length(length))
}

/// Parse `box-sizing: <ident>` (CSS Sizing 3 §3.3 <https://www.w3.org/TR/css-sizing-3/#box-sizing>).
///
/// Value grammar (§3.3): `content-box | border-box`. Compare identifiers ASCII case-insensitively,
/// following CSS Values 3 §3.1 "Pre-defined Keywords" and the same approach as
/// [`parse_display`](super::layout::parse_display) and [`parse_text_align`].
///
/// # Scope carving (details in [`BoxSizing`] doc-comment)
///
/// - **(b) Unsupported**: CSS-wide keywords are not yet implemented and are silently dropped. The
///   "CSS-wide keyword" section of the [`PropertyValue`] doc lists the five keywords and explains why.
/// - **(a) Spec-invalid**: Other keywords such as `padding-box` (from a CSS-UI 3 draft, removed from
///   css-sizing-3) and `margin-box` silently return `None`.
pub(super) fn parse_box_sizing(input: &mut Parser<'_, '_>) -> Option<BoxSizing> {
    BoxSizing::from_css_ident(input.expect_ident().ok()?)
}

/// Parse `overflow-x` / `overflow-y: <ident>` (CSS Overflow 3 §3.1
/// <https://www.w3.org/TR/css-overflow-3/#overflow-properties>).
///
/// Value grammar (§3.1): `visible | hidden | clip | scroll | auto`. Match identifiers ASCII
/// case-insensitively, as do [`parse_box_sizing`] and [`parse_direction`]. CSS Overflow also defines
/// the legacy `overlay` spelling as an alias for `auto`; it is normalized to the same
/// [`OverflowValue::Auto`] variant rather than adding a separate computed value.
///
/// # Scope carving (details in [`OverflowValue`] doc-comment)
///
/// - **(a) Spec-invalid**: An identifier other than these five keywords or the legacy `overlay` alias
///   silently returns `None`.
/// - **(b) Unsupported**: CSS-wide keywords are not yet implemented and are silently dropped. The
///   "CSS-wide keyword" section of the [`PropertyValue`] doc lists the five keywords and explains why.
pub(super) fn parse_overflow_value(input: &mut Parser<'_, '_>) -> Option<OverflowValue> {
    let ident = input.expect_ident().ok()?;
    // The legacy `overlay` spelling is an alias for `auto` with no separate
    // computed-value variant (see `OverflowValue`).
    if ident.eq_ignore_ascii_case("overlay") {
        return Some(OverflowValue::Auto);
    }
    OverflowValue::from_css_ident(ident)
}

/// `overflow: <'overflow-block'>{1,2}` shorthand — 1-2 value expansion.
///
/// Grammar reference: CSS Overflow 3 §3.1 <https://www.w3.org/TR/css-overflow-3/#overflow-properties>.
///
/// # Expansion rule (spec verbatim, §3.1)
///
/// "The overflow property is a shorthand property that sets the specified
/// values of overflow-x and overflow-y in that order. If the second value is
/// omitted, it is copied from the first."
///
/// Like [`parse_margin_shorthand`], stack `try_parse` calls to expand values; here the shorthand
/// accepts one or two values rather than one to four.
///
/// # Trailing garbage handling
///
/// For `overflow: hidden scroll auto`, this helper consumes two values and leaves the third token
/// unconsumed. The caller's `DeclParser` in [`mod@crate::rule`] uses its
/// [`cssparser::DeclarationParser::parse_value`] implementation to detect the extra token with
/// `expect_exhausted` and drop the entire declaration. This follows the division
/// of responsibility described in the "Trailing garbage handling" section of the [`parse_margin_shorthand`] doc.
pub(super) fn parse_overflow_shorthand(input: &mut Parser<'_, '_>) -> Option<OverflowXY> {
    let v1 = parse_overflow_value(input)?;
    // Without a second value, copy the first to both axes (§3.1: "If the second value is omitted,
    // it is copied from the first.").
    let Some(v2) = input.try_parse(|i| parse_overflow_value(i).ok_or(())).ok() else {
        return Some(OverflowXY::both(v1));
    };
    Some(OverflowXY { x: v1, y: v2 })
}

/// Parse `position: static | sticky | running(<custom-ident>)` (CSS GCPM 3 §1.2.1
/// <https://www.w3.org/TR/css-gcpm-3/#running-syntax> and CSS Positioned Layout Module Level 3 §3
/// <https://www.w3.org/TR/css-position-3/#sticky-pos>).
///
/// Current scope:
/// - `static` — [`PositionValue::Static`], it matches the initial state of `inherit_from`, so there is no
///   problem even if apply_value is no-op. In cascade winner selection, the identity is used to overwrite and
///   suppress the preceding `running(...)` (note that the effectiveness cannot be tested with
///   standalone-static tests alone).
/// - `sticky` — [`PositionValue::Sticky`], CSS Positioned Layout §3. Parsing accepts it, but
///   `apply_value` remains a no-op like `Static` until layout support is connected.
/// - `running(<custom-ident>)` — [`PositionValue::Running`], apply_value seeds 1-item `RunningTemplate`
///   into computed.running_templates.
/// - Other keywords (`relative` / `absolute` / `fixed`) are not implemented and silently return `None`.
///
/// `<custom-ident>` follows the string-set exclusion convention: [`is_reserved_custom_ident`] rejects
/// CSS-wide keywords and `default`, and this parser also rejects `none`. Although `none` is not another
/// specification-defined keyword for `position`, reserving it follows the alternative-keyword
/// convention and prevents runtime resolution from mistakenly matching `element(none)`. String-set
/// rejects `none` for the same reason.
pub(super) fn parse_position(input: &mut Parser<'_, '_>) -> Option<PositionValue> {
    // `static` is currently one of the keywords accepted by scope.
    if input
        .try_parse(|i| i.expect_ident_matching("static"))
        .is_ok()
    {
        return Some(PositionValue::Static);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("relative"))
        .is_ok()
    {
        return Some(PositionValue::Relative);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("absolute"))
        .is_ok()
    {
        return Some(PositionValue::Absolute);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("fixed"))
        .is_ok()
    {
        return Some(PositionValue::Fixed);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("sticky"))
        .is_ok()
    {
        return Some(PositionValue::Sticky);
    }
    // `running(<custom-ident>)`. The function name is ASCII case-insensitive, and the content custom-ident
    // is case-preserving and stored in SmolStr.
    let running = input.try_parse(|i| -> Result<SmolStr, ParseError<'_, ()>> {
        let fn_name = i.expect_function()?.clone();
        if !fn_name.eq_ignore_ascii_case("running") {
            return Err(i.new_custom_error(()));
        }
        i.parse_nested_block(|inner| -> Result<SmolStr, ParseError<'_, ()>> {
            let ident = inner.expect_ident()?.clone();
            if is_reserved_custom_ident(&ident) || ident.eq_ignore_ascii_case("none") {
                return Err(inner.new_custom_error(()));
            }
            Ok(SmolStr::new(ident.as_ref()))
        })
    });
    running.ok().map(PositionValue::Running)
}

/// Parse `top` / `right` / `bottom` / `left: auto | <length-percentage>` (CSS Positioned Layout Module
/// Level 3 §3).
pub(super) fn parse_inset(input: &mut Parser<'_, '_>) -> Option<LengthOrAuto> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(LengthOrAuto::Auto);
    }
    let length = parse_length_value(input, false)?;
    Some(LengthOrAuto::Length(length))
}

/// Parse `z-index: auto | <integer>` (CSS2 §9.9.1
/// <https://www.w3.org/TR/CSS2/visuren.html#z-index>; see the [`ZIndexValue`] doc).
///
/// Try the `auto` identifier first, following the order in [`parse_margin_side`] (see its doc).
/// Here the alternatives already have distinct token kinds (`auto` identifier versus integer), so
/// this order is not required for correctness, but it stays consistent with the other parser.
///
/// Extract `<integer>` directly with `expect_integer`, as in
/// [`parse_counter_property`](super::content::parse_counter_property). CSS Values 3 §4.2
/// "Integers: the `<integer>` type" allows signed values, including negative ones, without a range limit.
pub(super) fn parse_z_index(input: &mut Parser<'_, '_>) -> Option<ZIndexValue> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(ZIndexValue::Auto);
    }
    input
        .try_parse(|i| i.expect_integer())
        .ok()
        .map(ZIndexValue::Integer)
}

/// Expand one to four circular `<length>` values of `border-radius` across the four corners.
///
/// CSS Backgrounds and Borders 3 §5 orders them top-left, top-right, bottom-right, bottom-left.
/// Percentages are accepted; elliptical radii after a slash and negative values are not.
pub(super) fn parse_border_radius(input: &mut Parser<'_, '_>) -> Option<BorderRadius> {
    let first = parse_non_negative_length_percentage(input)?;
    let second = input
        .try_parse(parse_non_negative_length_percentage_res)
        .ok();
    let third = input
        .try_parse(parse_non_negative_length_percentage_res)
        .ok();
    let fourth = input
        .try_parse(parse_non_negative_length_percentage_res)
        .ok();

    Some(match (second, third, fourth) {
        (None, _, _) => BorderRadius {
            top_left: first,
            top_right: first,
            bottom_right: first,
            bottom_left: first,
        },
        (Some(opposite), None, _) => BorderRadius {
            top_left: first,
            top_right: opposite,
            bottom_right: first,
            bottom_left: opposite,
        },
        (Some(horizontal), Some(bottom), None) => BorderRadius {
            top_left: first,
            top_right: horizontal,
            bottom_right: bottom,
            bottom_left: horizontal,
        },
        (Some(top_right), Some(bottom_right), Some(bottom_left)) => BorderRadius {
            top_left: first,
            top_right,
            bottom_right,
            bottom_left,
        },
    })
}

pub(super) fn parse_non_negative_length_percentage(input: &mut Parser<'_, '_>) -> Option<Length> {
    parse_non_negative_length_percentage_res(input).ok()
}

fn parse_length_allow_negative_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<Length, ParseError<'i, ()>> {
    parse_length_allow_negative(input).ok_or_else(|| input.new_custom_error(()))
}

/// Parse the box-shadow length run of `<length>{2,4}`.
///
/// Offset-x, offset-y, and spread-radius have no sign restriction, so apply the `!is_nan()` guard via
/// [`parse_shadow_length_reject_nan`]/`_res` (see the function's doc). Blur-radius, the third slot,
/// needs no additional guard: its existing `value.payload() >= 0.0` check also rejects NaN because
/// `NaN >= 0.0` is `false` under IEEE 754.
pub(super) fn parse_box_shadow_lengths<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<(Length, Length, Length, Length), ParseError<'i, ()>> {
    let offset_x = parse_shadow_length_reject_nan_res(input)?;
    let offset_y = parse_shadow_length_reject_nan_res(input)?;
    // Parse the optional third slot without rewinding a negative length into
    // the fourth (spread) slot.  The grammar's third length is blur-radius,
    // which is non-negative; only the fourth spread-radius may be negative.
    let blur_radius = match input.try_parse(parse_length_allow_negative_res) {
        Ok(value) if value.payload() >= 0.0 => value,
        Ok(_) => return Err(input.new_custom_error(())),
        Err(_) => Length::Px(0.0),
    };
    let spread_radius = input
        .try_parse(parse_shadow_length_reject_nan_res)
        .unwrap_or(Length::Px(0.0));
    Ok((offset_x, offset_y, blur_radius, spread_radius))
}

fn parse_box_shadow_item(input: &mut Parser<'_, '_>) -> Option<BoxShadowItem> {
    let mut lengths: Option<(Length, Length, Length, Length)> = None;
    let mut color: Option<TextShadowColor> = None;
    let mut inset = false;

    // The optional color and `inset` keyword may occur before or after the
    // required length run.
    loop {
        let mut progressed = false;
        if lengths.is_none()
            && let Ok(value) = input.try_parse(parse_box_shadow_lengths)
        {
            lengths = Some(value);
            progressed = true;
        }
        if color.is_none()
            && let Ok(value) = input.try_parse(|i| -> Result<TextShadowColor, ParseError<'_, ()>> {
                parse_text_shadow_color(i).ok_or_else(|| i.new_custom_error(()))
            })
        {
            color = Some(value);
            progressed = true;
        }
        if !inset
            && input
                .try_parse(|i| i.expect_ident_matching("inset"))
                .is_ok()
        {
            inset = true;
            progressed = true;
        }
        if !progressed {
            break;
        }
    }

    let (offset_x, offset_y, blur_radius, spread_radius) = lengths?;
    Some(BoxShadowItem {
        offset_x,
        offset_y,
        blur_radius,
        spread_radius,
        color: color.unwrap_or(TextShadowColor::CurrentColor),
        inset,
    })
}

/// Parse `box-shadow: none | <shadow>#`.
pub(super) fn parse_box_shadow(input: &mut Parser<'_, '_>) -> Option<Vec<BoxShadowItem>> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(Vec::new());
    }
    input
        .parse_comma_separated(|i| -> Result<BoxShadowItem, ParseError<'_, ()>> {
            parse_box_shadow_item(i).ok_or_else(|| i.new_custom_error(()))
        })
        .ok()
}

/// Parse the width/style/color components of `outline` shorthand in any-order.
pub(super) fn parse_outline(input: &mut Parser<'_, '_>) -> Option<Outline> {
    let mut width: Option<Length> = None;
    let mut style: Option<OutlineStyle> = None;
    let mut color: Option<OutlineColor> = None;

    loop {
        if width.is_some() && style.is_some() && color.is_some() {
            break;
        }
        if width.is_none()
            && let Ok(value) = input.try_parse(parse_border_width_side_res)
        {
            width = Some(value);
            continue;
        }
        if style.is_none()
            && let Ok(value) = input.try_parse(parse_outline_style_res)
        {
            style = Some(value);
            continue;
        }
        if color.is_none()
            && let Ok(value) = input.try_parse(|i| -> Result<OutlineColor, ParseError<'_, ()>> {
                parse_outline_color(i).ok_or_else(|| i.new_custom_error(()))
            })
        {
            color = Some(value);
            continue;
        }
        break;
    }

    if width.is_none() && style.is_none() && color.is_none() {
        return None;
    }
    Some(Outline {
        width: width.unwrap_or(Length::Px(BORDER_WIDTH_MEDIUM_PX)),
        style: style.unwrap_or(OutlineStyle::None),
        color: color.unwrap_or(OutlineColor::Invert),
    })
}

/// Parse `outline-color`. Accept `invert | <color>` from CSS UI 3 §4.4, and keep the
/// `currentcolor` keyword in a dedicated variant.
pub(super) fn parse_outline_color(input: &mut Parser<'_, '_>) -> Option<OutlineColor> {
    if input
        .try_parse(|i| i.expect_ident_matching("invert"))
        .is_ok()
    {
        return Some(OutlineColor::Invert);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("currentcolor"))
        .is_ok()
    {
        return Some(OutlineColor::CurrentColor);
    }
    parse_color(input).map(OutlineColor::Resolved)
}

/// Parse one `outline-style` keyword. The outline shorthand and longhand share
/// this helper so `auto` cannot accidentally become valid for border styles.
pub(super) fn parse_outline_style_side(input: &mut Parser<'_, '_>) -> Option<OutlineStyle> {
    match OutlineStyle::from_css_ident(input.expect_ident().ok()?) {
        // Keep the existing `outline: hidden` rejection. The enum retains the
        // keyword for representation completeness, but this parser scope does
        // not accept it.
        Some(OutlineStyle::Hidden) => None,
        other => other,
    }
}

fn parse_outline_style_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<OutlineStyle, ParseError<'i, ()>> {
    parse_outline_style_side(input).ok_or_else(|| input.new_custom_error(()))
}
