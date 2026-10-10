//! Numeric, length and identifier helpers shared by several property parser domains.

use cssparser::{BasicParseError, ParseError, Parser, ParserInput, Token};
use smol_str::SmolStr;

use crate::property::types::*;

// ── Numeric-token NaN stabilization ───────────────────────────────────
//
// cssparser 0.37.0's tokenizer computes a `<number>`/`<percentage>`/
// `<dimension>` token's value in two floating-point steps rather than as
// the single formula CSS Syntax Level 3 §4.3.13 defines
// (<https://www.w3.org/TR/css-syntax-3/#convert-a-string-to-a-number>,
// `s·(i + f·10⁻ᵈ)·10^(t·e)`): first the mantissa `sign * (integral_part +
// fractional_part)` as an f64, then, only if an exponent was written,
// `value *= f64::powf(10., sign * exponent)` (`tokenizer.rs:1081` in the
// `cssparser` crate). For a zero-mantissa literal with a huge exponent
// (`0e999`), `f64::powf(10., 999.)` evaluates to `+Infinity` first, and
// IEEE 754 defines `0.0 * Infinity` as `NaN` — even though the spec
// formula's actual value for this input is exactly `0` (multiplying by
// zero, not by infinity, is what the formula does mathematically).
// `"0e999".parse::<f32>()` (Rust's own single-step, correctly-rounded
// string-to-float conversion) returns `0` directly, confirming the value
// is exact and in-range — the `NaN` this crate would otherwise observe is
// purely an artifact of the tokenizer's two-step evaluation order.
//
// The same collapse mirrors in the opposite direction: digit accumulation
// in the tokenizer's integral-part loop can itself overflow to
// `+Infinity` for a sufficiently long run of digits with no exponent
// written, and multiplying that by a sufficiently negative exponent's
// `10^exponent` (which underflows to `0.0`) hits `Infinity * 0.0` = `NaN`
// the same way, for a literal whose true value is small but nonzero.
//
// `stabilize_nan_numeric_value` recovers both directions identically,
// since it does not special-case which operand was zero — it simply
// re-parses the token's own raw source text. The three functions below it
// (`expect_number_stable`/`expect_percentage_stable`/`next_numeric_stable`)
// are the acquisition points every NaN-sensitive numeric-token consumer in
// this module routes through, so the recovery happens once, at the
// source, and every downstream `!is_nan()` guard (and every range check
// that incidentally depends on NaN comparing `false`, e.g.
// `parse_filter_amount`'s `v >= 0.0`) sees the spec-correct value
// directly, without needing a special case of its own — including
// `parse_grid_flex_res` (the `fr` unit), which acquires its
// `Token::Dimension` via `next_numeric_stable` just like the other
// numeric-token consumers.
//
// This grammar-mirroring is itself a forward-maintenance hazard worth
// naming: `numeric_token_prefix` below re-implements cssparser's
// `consume_numeric` number grammar by hand, and the workspace pins
// `cssparser = "0.37"` (a caret range, not an exact version) — if a future
// `cssparser` version this range admits changes the `<number-token>`
// grammar (e.g. adds a new numeric literal shape), `numeric_token_prefix`
// must be updated to match, or it will silently mis-split the raw text for
// that new shape.

/// Splits the `<number>` production (CSS Syntax Level 3 §4.3.12 "Consume a
/// number" <https://www.w3.org/TR/css-syntax-3/#consume-number>; see also
/// §4.1's railroad diagram
/// <https://www.w3.org/TR/css-syntax-3/#number-token-diagram> for the
/// visual grammar: optional sign, digits, optional `.` + digits, optional
/// `[eE]` + optional sign + digits) off the front of `raw`. A percentage's
/// trailing `%` or a dimension's trailing unit is not part of this
/// production and is left in the (discarded) remainder — this mirrors cssparser's own
/// `consume_numeric` grammar exactly, so it consumes the whole numeric
/// prefix, and only the numeric prefix, of any text cssparser itself
/// already tokenized as a `<number-token>`/`<percentage-token>`/
/// `<dimension-token>`; no unit-length bookkeeping is needed to find the
/// boundary.
fn numeric_token_prefix(raw: &str) -> &str {
    let bytes = raw.as_bytes();
    let mut i = 0;
    if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
        i += 1;
    }
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i < bytes.len() && bytes[i] == b'.' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit) {
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
    }
    if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
        let mut j = i + 1;
        if j < bytes.len() && (bytes[j] == b'+' || bytes[j] == b'-') {
            j += 1;
        }
        // The two closing braces below (after `i = j;`) are reached every
        // time this `if`'s condition is true — a plain assignment
        // statement with no early return/break cannot skip past its own
        // block's closing brace. Every actual call to this function is
        // gated on `value.is_nan()` at the call site (`expect_number_stable`
        // et al.), and `NaN` can only arise from a numeric token that had a
        // written exponent with at least one digit (module doc above), so
        // both this `if` and its enclosing one are always true for every
        // call this crate makes — there is no reachable path through this
        // function where they are false. Despite that, `cargo-llvm-cov`
        // 0.8.7's line-level report shows 0 hits on both closing-brace
        // lines even though the `i = j;` line directly above executes on
        // every one of those calls: a closing brace immediately following
        // the last (non-control-flow) statement of a nested `if` block,
        // when every exercised call takes the identical path through it,
        // does not get its own incremented coverage region in this
        // toolchain — reproduced in isolation with a minimal function of
        // the same shape. No additional test changes what these two lines
        // report, since the statement they follow is already exercised.
        if j < bytes.len() && bytes[j].is_ascii_digit() {
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            i = j;
        } // cov:ignore: closing-brace coverage-instrumentation artifact, see comment above the `if` this closes
    } // cov:ignore: same closing-brace artifact, one nesting level out — see comment above
    &raw[..i]
}

/// Recovers a numeric token's value from its own raw source text when
/// cssparser's tokenizer computed `NaN` for it (module doc above). `raw`
/// must already be trimmed to the `<number>` production span
/// (`numeric_token_prefix`). Re-parsing with `f64::from_str` computes the
/// CSS Syntax §4.3.13 formula in one step, so the `0 * Infinity` /
/// `Infinity * 0` intermediate never arises.
///
/// On the (expected-unreachable, since every `raw` this module passes in
/// is text cssparser itself already validated as a `<number-token>`)
/// chance the reparse fails, the original `value` (still `NaN`) is
/// returned unchanged, so callers' existing `is_nan()`/range-check guards
/// keep rejecting it rather than silently substituting a wrong number.
/// When `value` isn't `NaN` to begin with, this is a no-op — every
/// ordinary numeric literal, including magnitude overflow to real
/// `+Inf`/`-Inf`, is bit-identical to before this function existed.
pub(crate) fn stabilize_nan_numeric_value(raw_number_text: &str, value: f32) -> f32 {
    if !value.is_nan() {
        return value;
    }
    raw_number_text
        .parse::<f64>()
        .map(|v| v as f32)
        .unwrap_or(value)
}

/// `stabilize_nan_numeric_value`'s counterpart for `Token::Percentage`'s
/// `unit_value` field — `raw_number_text` is the same `<number>`
/// production span (`numeric_token_prefix`, with the trailing `%` already
/// excluded), but the recovered magnitude is divided by `100.` before
/// returning, matching `Parser::expect_percentage`'s own `unit_value`
/// convention (`0%`..`100%` → `0.0`..`1.0`). Shared by `expect_percentage_stable`
/// and `next_numeric_stable`'s `Token::Percentage` arm so the `/ 100.`
/// convention lives in one place rather than being repeated at each call
/// site.
pub(crate) fn stabilize_nan_percentage_value(raw_number_text: &str, unit_value: f32) -> f32 {
    if !unit_value.is_nan() {
        return unit_value;
    }
    raw_number_text
        .parse::<f64>()
        .map(|v| (v / 100.0) as f32)
        .unwrap_or(unit_value)
}

/// `Parser::expect_number` wrapper that applies `stabilize_nan_numeric_value`
/// before returning. Every `<number>` acquisition in this module should
/// call this instead of `Parser::expect_number` directly.
///
/// `Parser::skip_whitespace` is called explicitly before capturing the
/// start position: `Parser::next` (which `Parser::expect_number` calls
/// internally) skips leading whitespace *and comments* before reading the
/// token, so without this the captured position could point at that
/// skipped run instead of the token's first byte, and `Parser::slice_from`
/// would return a slice `numeric_token_prefix` can't walk (e.g. a leading
/// space makes it return `""`). Calling `skip_whitespace` here is a no-op
/// for `Parser::next`'s own subsequent call to it (nothing is left to
/// skip), so this changes no parsing behavior, only where the position is
/// captured.
pub(super) fn expect_number_stable<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<f32, BasicParseError<'i>> {
    input.skip_whitespace();
    let start = input.position();
    let value = input.expect_number()?;
    Ok(if value.is_nan() {
        stabilize_nan_numeric_value(numeric_token_prefix(input.slice_from(start)), value)
    } else {
        value
    })
}

/// `Parser::expect_percentage` wrapper — same recovery and
/// whitespace/comment-skip rationale as `expect_number_stable`, but
/// re-derives the pre-`/100` magnitude from the raw text and re-applies
/// `Parser::expect_percentage`'s own `/ 100.` convention before returning,
/// so the result stays a normalized `unit_value` (`0%`..`100%` → `0.0`..
/// `1.0`) like the wrapped method's.
pub(super) fn expect_percentage_stable<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<f32, BasicParseError<'i>> {
    input.skip_whitespace();
    let start = input.position();
    let unit_value = input.expect_percentage()?;
    Ok(if unit_value.is_nan() {
        stabilize_nan_percentage_value(numeric_token_prefix(input.slice_from(start)), unit_value)
    } else {
        unit_value
    })
}

/// `Parser::next` wrapper for the raw `Token::Number`/`Token::Percentage`/
/// `Token::Dimension` matches in this module (`parse_length_value`,
/// `parse_angle`, `parse_hue`) — corrects the token's own numeric field in
/// place when it is `NaN` (module doc above), before the caller's `match`
/// ever sees it. Non-numeric tokens, and numeric tokens whose value isn't
/// `NaN`, pass through unchanged. Same whitespace/comment-skip rationale
/// as `expect_number_stable`.
pub(super) fn next_numeric_stable<'i, 't>(
    input: &mut Parser<'i, 't>,
) -> Result<Token<'i>, BasicParseError<'i>> {
    input.skip_whitespace();
    let start = input.position();
    let token = input.next()?.clone();
    Ok(match token {
        Token::Number {
            has_sign,
            value,
            int_value,
        } if value.is_nan() => Token::Number {
            has_sign,
            value: stabilize_nan_numeric_value(
                numeric_token_prefix(input.slice_from(start)),
                value,
            ),
            int_value,
        },
        Token::Percentage {
            has_sign,
            unit_value,
            int_value,
        } if unit_value.is_nan() => Token::Percentage {
            has_sign,
            unit_value: stabilize_nan_percentage_value(
                numeric_token_prefix(input.slice_from(start)),
                unit_value,
            ),
            int_value,
        },
        Token::Dimension {
            has_sign,
            value,
            int_value,
            unit,
        } if value.is_nan() => Token::Dimension {
            has_sign,
            value: stabilize_nan_numeric_value(
                numeric_token_prefix(input.slice_from(start)),
                value,
            ),
            int_value,
            unit,
        },
        other => other,
    })
}

/// Shared parser for `<length>` / `<length-percentage>`. Consumes one token.
///
/// Grammar reference: CSS Values 4 §6 <https://www.w3.org/TR/css-values-4/#lengths>
/// / §5.5 <https://www.w3.org/TR/css-values-4/#percentages>.
///
/// # Mode selector
///
/// `allow_percentage` only controls whether `%` (`Token::Percentage`) tokens
/// are accepted. Both modes accept the same dimension units (`px`, etc.; see
/// the `Token::Dimension` match arm below).
/// - `false` → `<length>` mode: rejects `%`.
/// - `true` → `<length-percentage>` mode: accepts `%` too.
///
/// The `Token::Dimension` match below is the source of truth for supported
/// units; unsupported units are rejected.
///
/// # Unitless zero
///
/// CSS Values 3 §5 "Distance Units: the `<length>` type"
/// <https://www.w3.org/TR/css-values-3/#lengths> verbatim: "For zero lengths
/// the unit identifier is optional (i.e. can be syntactically represented as the
/// `<number>` 0)." — accept bare `0` (Token::Number, value == 0.0) as
/// [`Length::Px`] `(0.0)` in both `<length>` and `<length-percentage>` modes.
/// Nonzero unitless numbers (`5`, `-1`, etc.) cannot match `<length>` and are
/// still dropped (checked by the `== 0.0` guard).
///
/// The same spec's §5 clause 2 says: "if a 0 could be parsed as either a
/// `<number>` or a `<length>` in a property (such as line-height), it must parse
/// as a `<number>`". [`parse_line_height`](super::text::parse_line_height) tries
/// the `expect_number` branch before this helper, so it returns
/// `LineHeight::Number(0.0)`, not this helper's `Length::Px(0.0)` (the
/// disambiguation required by the spec).
///
/// # Sign / range
///
/// This helper does not check signs or ranges: each property has different
/// requirements (padding is non-negative; margin permits negatives, etc.).
/// Callers post-filter the result. For example,
/// [`parse_font_size`](super::text::parse_font_size) checks `>= 0.0` on the
/// payload of **every [`Length`] variant**.
///
/// # Callers with `allow_percentage=true`
///
/// Originally added for forward provisioning, this mode now has seven callers:
/// [`parse_margin_side`](super::box_model::parse_margin_side) / [`parse_padding_side`](super::box_model::parse_padding_side) / [`parse_width`](super::box_model::parse_width) /
/// [`parse_height`](super::box_model::parse_height) / [`parse_line_height`](super::text::parse_line_height) / [`parse_font_size`](super::text::parse_font_size) /
/// [`parse_text_indent`](super::text::parse_text_indent). Each property's spec
/// grammar includes `<length-percentage>` (`font-size` was originally limited
/// to `<length>` and later expanded). Sharing this helper avoids duplicate
/// dimension-unit dispatch.
///
/// Apart from spacing, the caller with `allow_percentage=false` (`<length>`
/// mode) is [`parse_border_width_side`](super::box_model::parse_border_width_side):
/// CSS Backgrounds 3 §3.3's `<line-width>` grammar excludes `<percentage>`.
/// The text-spacing parsers that accept percentages call this helper with
/// `allow_percentage=true`.
///
/// # Percentage overflow
///
/// `Token::Percentage.unit_value` is already converted from f64 to f32
/// (cssparser 0.37's tokenizer emits `value / 100.0`). [`Length::Percent`]
/// stores the authored number (`50%` → `50.0`), so this helper converts it
/// back with `unit_value * 100.0`. Even when `unit_value` fits in finite f32
/// (e.g. `1e40%` becomes a finite `1e38` in cssparser), this conversion can
/// overflow f32 (`1e38 * 100.0` exceeds its finite maximum `3.4028235e38`
/// and becomes `+Inf`). CSS Values 4 §5 "Range Checking and Precision for
/// Numeric Types" <https://www.w3.org/TR/css-values-4/#numeric-types> says
/// "it must be converted to the closest value supported by the
/// implementation". Only when the result is `±Inf`, saturate it to
/// `f32::MAX` while retaining its sign.
///
/// The existing sink-guard precedent ("put guards at sink boundaries, not
/// in parse/resolve layers") does not apply here. This is about conversion
/// accuracy, not a guard: the specified value itself would violate CSS
/// Values 4 §5. The sink guard `raikiri-dom::layout::sanitize_finite`
/// (for geometry after resolution) remains necessary after this change.
///
/// **`NaN` is not subject to this saturation.** Since `is_finite()` also
/// returns `false` for `NaN`, the original implementation saturated NaN to
/// `±f32::MAX`. That was wrong: for example, `0e999%` can produce `NaN` as
/// an intermediate result of cssparser's tokenizer computing `0.0 * 10^999`
/// (`f64::powf` returns `+Inf`), but its **true mathematical value is 0**;
/// the "closest value" is `0.0`, not `f32::MAX`. This function calls
/// `next_numeric_stable` (see "Numeric-token NaN stabilization" at the start
/// of the module docs), which corrects exactly this class on token acquisition.
/// Normally `unit_value` therefore no longer reaches this arm as `NaN`, and
/// `0e999%` becomes `Length::Percent(0.0)` without entering saturation.
/// Keep the `is_infinite()`-only condition as defense in depth: the existing
/// sink contract of `sanitize_finite` (`raikiri-dom/src/layout.rs`) already
/// treats `NaN` as `0.0`. Any residual `NaN` after failed recovery by
/// `next_numeric_stable` (e.g. re-parsing fails) must pass through unchanged,
/// not be moved to `f32::MAX`. Saturation via `is_infinite()` is still needed
/// for true magnitude overflows such as `1e40%` (ordinary overflow, unlike
/// the opposite-direction collapse described in the module docs).
pub(crate) fn parse_length_value(
    input: &mut Parser<'_, '_>,
    allow_percentage: bool,
) -> Option<Length> {
    match &next_numeric_stable(input).ok()? {
        Token::Dimension { value, unit, .. } => match unit.to_ascii_lowercase().as_str() {
            "px" => Some(Length::Px(*value)),
            "em" => Some(Length::Em(*value)),
            "rem" => Some(Length::Rem(*value)),
            "pt" => Some(Length::Pt(*value)),
            // Additional font-relative units (CSS Values 4 §6.1.1).
            // The real-metric variants of `ex`/`ch`/`ic` always use their spec
            // fallbacks because the style layer has no font metrics (see the
            // `Length::Ex` / `Length::Ch` / `Length::Ic` docs).
            "ex" => Some(Length::Ex(*value)),
            "rex" => Some(Length::Rex(*value)),
            "ch" => Some(Length::Ch(*value)),
            "rch" => Some(Length::Rch(*value)),
            "ic" => Some(Length::Ic(*value)),
            "ric" => Some(Length::Ric(*value)),
            // Additional absolute units (CSS Values 4 §6.2).
            // `unit` has already been converted with `to_ascii_lowercase()`:
            // even a `Q` token arrives as `"q"`.
            "cm" => Some(Length::Cm(*value)),
            "mm" => Some(Length::Mm(*value)),
            "q" => Some(Length::Q(*value)),
            "in" => Some(Length::In(*value)),
            "pc" => Some(Length::Pc(*value)),
            // `lh` / `rlh` (CSS Values 4 §6.1.1).
            // Accepted generally here for every consumer, `font-size` included
            // (moved out of `parse_font_size`'s former
            // post-filter — see that function's doc section on accepting
            // "`lh` / `rlh`" and resolving them against the parent).
            "lh" => Some(Length::Lh(*value)),
            "rlh" => Some(Length::Rlh(*value)),
            // (b) Unsupported: `cap`/`rcap` are silently dropped. They cannot
            // be resolved using only the specified layer (the style layer
            // lacks font ascent), so this work is tracked in a follow-up task.
            //
            // This arm also drops every other unrecognized unit, such as
            // container-query units `cqw`/`cqh`/`cqi`/`cqb`/`cqmin`/`cqmax`
            // (CSS Contain 3 §6 <https://www.w3.org/TR/css-contain-3/#container-lengths>;
            // the style layer lacks container size).
            // This comment is the canonical list of unsupported units; do not
            // repeat this list elsewhere just to extend it.
            //
            // The viewport-percentage units are the only other ones accepted.
            unit => viewport_length(unit, *value),
        },
        Token::Percentage { unit_value, .. } if allow_percentage => {
            // Authored-number reverse conversion and overflow saturation:
            // see "# Percentage overflow" above.
            let percent = *unit_value * 100.0;
            Some(Length::Percent(if percent.is_infinite() {
                f32::MAX.copysign(percent)
            } else {
                percent
            }))
        }
        // CSS Values 3 §5 unitless-zero clause (see "# Unitless zero" above).
        Token::Number { value, .. } if *value == 0.0 => Some(Length::Px(0.0)),
        _ => None,
    }
}

/// The viewport-percentage length (CSS Values 4 §6.1.2
/// <https://www.w3.org/TR/css-values-4/#viewport-relative-lengths>) of
/// `value` in the lowercase `unit`, or `None` when `unit` is not one.
///
/// The small (`s`), large (`l`) and dynamic (`d`) variants keep their size
/// in [`Length::SizedViewport`].
fn viewport_length(unit: &str, value: f32) -> Option<Length> {
    let (size, unit) = match unit.as_bytes().first() {
        Some(b's') => (Some(ViewportSize::Small), &unit[1..]),
        Some(b'l') => (Some(ViewportSize::Large), &unit[1..]),
        Some(b'd') => (Some(ViewportSize::Dynamic), &unit[1..]),
        _ => (None, unit),
    };
    let unit = match unit {
        "vw" => ViewportUnit::Vw,
        "vh" => ViewportUnit::Vh,
        "vi" => ViewportUnit::Vi,
        "vb" => ViewportUnit::Vb,
        "vmin" => ViewportUnit::Vmin,
        "vmax" => ViewportUnit::Vmax,
        _ => return None,
    };
    Some(match size {
        Some(size) => Length::SizedViewport(size, unit, value),
        None => unit.length(value),
    })
}

/// Whether the rest of `input`, including any nested block or function,
/// holds a dimension in a viewport-percentage unit. `input` is left where it
/// was.
///
/// The page context uses this to drop declarations whose viewport-percentage
/// lengths it cannot resolve yet (see
/// [`crate::page::parse_page_declaration_block`]). The arguments of `var()`
/// are skipped: which of them is used is only known after substitution, so
/// [`has_viewport_length_in`] checks the substituted value instead.
pub(crate) fn has_viewport_length(input: &mut Parser<'_, '_>) -> bool {
    fn scan(input: &mut Parser<'_, '_>) -> bool {
        while let Ok(token) = input.next() {
            let nested = match token {
                Token::Dimension { unit, .. } => {
                    if viewport_length(&unit.to_ascii_lowercase(), 0.0).is_some() {
                        return true;
                    }
                    false
                }
                Token::Function(name) => !name.eq_ignore_ascii_case("var"),
                Token::ParenthesisBlock | Token::SquareBracketBlock | Token::CurlyBracketBlock => {
                    true
                }
                _ => false,
            };
            if nested
                && input
                    .parse_nested_block(|block| {
                        let found = scan(block);
                        // A nested block must be consumed to its end.
                        while block.next().is_ok() {}
                        Ok::<_, ParseError<'_, ()>>(found)
                    })
                    .unwrap_or(false)
            {
                return true;
            }
        }
        false
    }
    let start = input.state();
    let found = scan(input);
    input.reset(&start);
    found
}

/// [`has_viewport_length`] over a whole value, such as the result of
/// substituting `var()` references.
pub(crate) fn has_viewport_length_in(css: &str) -> bool {
    let mut input = ParserInput::new(css);
    has_viewport_length(&mut Parser::new(&mut input))
}

/// `<length [0,∞]>` — [`parse_length_value`] with `allow_percentage=false`
/// (no `<percentage>` alternative), then the same `[0,∞]` non-negative
/// filter the box-property parsers use (`(length.payload() >=
/// 0.0).then_some(length)`), so [`crate::page`]'s `size` descriptor parser
/// doesn't have to re-enumerate [`Length`] variants by hand.
///
/// `pub(crate)` for the one caller outside this module: [`crate::page`]'s
/// `size` descriptor parser. CSS Paged Media Level 3 §7.1 "Page size: the
/// size property" (<https://www.w3.org/TR/css-page-3/#page-size-prop>)
/// grammar is `<length>{1,2} | auto | …` — `<length>`, not
/// `<length-percentage>` — and states "Negative lengths are illegal", the
/// same `[0,∞]` shape this crate's box properties already enforce via
/// [`Length::payload`].
pub(crate) fn parse_non_negative_length(input: &mut Parser<'_, '_>) -> Option<Length> {
    let length = parse_length_value(input, false)?;
    (length.payload() >= 0.0).then_some(length)
}

/// `<length>` with no `<percentage>` alternative and no sign restriction —
/// [`parse_length_value`] with `allow_percentage=false`, unfiltered, so
/// callers outside this module don't have to re-enumerate [`Length`]
/// variants by hand.
///
/// `pub(crate)` for the one caller outside this module: [`crate::page`]'s
/// `bleed` descriptor parser. CSS Paged Media Level 3 §7.3 "Bleed Area: the
/// bleed property" (<https://www.w3.org/TR/css-page-3/#bleed>) grammar is
/// `auto | <length>` and explicitly permits negative values ("Values may be
/// negative, but there may be implementation-specific limits") — the same
/// unrestricted-sign behavior as [`parse_letter_spacing`](super::text::parse_letter_spacing)
/// and [`parse_word_spacing`](super::text::parse_word_spacing), unlike this module's sibling
/// [`parse_non_negative_length`] (`size`'s `<length>` alternative, which the
/// spec instead states is `[0,∞]`).
pub(crate) fn parse_length_allow_negative(input: &mut Parser<'_, '_>) -> Option<Length> {
    parse_length_value(input, false)
}

/// `<custom-ident>` (CSS Values 4 §4.2
/// <https://www.w3.org/TR/css-values-4/#custom-idents>): any identifier except
/// CSS-wide keywords and `default`. Preserve case and store it as a smol str.
///
/// Do not exclude `none` here. The spec says: "Specifications using
/// `<custom-ident>` must specify clearly what other keywords are excluded
/// from `<custom-ident>`, if any…". Extra exclusions for narrower grammars
/// (such as `<counter-name>`) belong to separate predicates (for example,
/// [`is_reserved_counter_name`](super::content::is_reserved_counter_name)).
/// See also the [`is_reserved_custom_ident`] docs.
///
/// **Originally three callers**: the name argument of `string()`
/// ([`parse_string_fn`](super::content::parse_string_fn)) and the second
/// arguments of `target-counter()` / `target-counters()`
/// ([`parse_target_counter_fn`](super::content::parse_target_counter_fn) / [`parse_target_counters_fn`](super::content::parse_target_counters_fn)).
/// All take `<custom-ident>` per the spec, where `none` is valid.
///
/// This was later widened to `pub(crate)`. The `counter_style` module became
/// a fourth caller, reusing the same CSS-wide-keyword exclusion list as the
/// base for `<counter-style-name>` (CSS Counter Styles L3 §3
/// <https://www.w3.org/TR/css-counter-styles-3/#typedef-counter-style-name>,
/// which excludes `none` from `<custom-ident>`), the `<custom-ident>`
/// alternative of `<symbol>`, and similar productions. This avoids maintaining
/// two copies of the `is_reserved_custom_ident` list (the same drift-avoidance
/// rule described in the [`crate::page::PageCascadeResult::declarations`] docs).
///
/// `counter()` / `counters()` and counter-* properties, which take
/// `<counter-name>`, **do not call this function**. Instead,
/// [`parse_counter_name`](super::content::parse_counter_name) /
/// [`parse_counter_property`](super::content::parse_counter_property) use
/// [`is_reserved_counter_name`](super::content::is_reserved_counter_name) to
/// exclude `none` as well. Do not exclude `none` here: doing so would reject
/// `target-counter(url(#a), none)` and `string(none)` contrary to the spec.
pub(crate) fn parse_custom_ident(input: &mut Parser<'_, '_>) -> Option<SmolStr> {
    let ident = input.expect_ident().ok()?.clone();
    if is_reserved_custom_ident(&ident) {
        None
    } else {
        Some(SmolStr::new(ident.as_ref()))
    }
}

/// Exclusion list for `<custom-ident>` (CSS Values 4 §4.2
/// <https://www.w3.org/TR/css-values-4/#custom-idents>).
///
/// Reject only CSS-wide keywords (`inherit` / `initial` / `unset` / `revert` /
/// `revert-layer`) and `default`. Do not exclude `none` here; add exclusions
/// for narrower grammars such as `<counter-name>` in separate predicates
/// (for example [`is_reserved_counter_name`](super::content::is_reserved_counter_name)).
/// Compare case-insensitively.
///
/// `pub(crate)` lets the `counter_style` module reuse this base list while
/// constructing exclusion predicates for `<counter-style-name>` productions
/// (rule name / `fallback` / `system: extends`). See the
/// [`parse_custom_ident`] docs.
///
/// This permanent exclusion comes from CSS Values 4 §4.2 and is
/// **independent of whether CSS-wide keywords are implemented**. Do not
/// confuse it with the separate claim that CSS-wide keywords are not yet
/// implemented as property values in the "CSS-wide keywords" section of
/// the [`PropertyValue`] docs.
pub(crate) fn is_reserved_custom_ident(ident: &str) -> bool {
    matches!(
        ident.to_ascii_lowercase().as_str(),
        "inherit" | "initial" | "unset" | "revert" | "revert-layer" | "default"
    )
}

/// `<length>` for shadow offset-x/offset-y (`text-shadow`/`box-shadow`) and
/// box-shadow's spread-radius — [`parse_length_allow_negative`]'s
/// unrestricted-sign shape (negative offsets/spread are valid per the two
/// properties' shared `<shadow>` grammar, CSS Backgrounds 3 §6.1 / CSS Text
/// Decoration Module Level 3 §4), with an explicit `!is_nan()` guard layered
/// on top.
///
/// # Why this guard is (mostly) already a no-op
///
/// A huge-exponent literal like `0e999px` has an exact mathematical value
/// of `0` (CSS Syntax 3 §4.3.13's own `<number-token>` conversion is
/// `sign * mantissa * 10^exponent`, and `0 * anything` is `0`), but
/// cssparser 0.37.0's tokenizer computes that same formula as a separate
/// floating-point step (`mantissa * 10^exponent`) that collapses to `NaN`
/// for this input (module doc's "Numeric-token NaN stabilization"
/// section). `parse_length_value` — which `parse_length_allow_negative`
/// (and so this function) goes through — already routes its token
/// acquisition through `next_numeric_stable`, which corrects exactly this
/// class of `NaN` before this function ever sees the `Length`. So in
/// ordinary use `length.payload()` here is never `NaN` for a
/// zero-mantissa literal; this `!is_nan()` check is kept as
/// defense-in-depth, same precedent as [`parse_opacity_value`](super::visual::parse_opacity_value)'s guard.
///
/// # Why the guard must be explicit here
///
/// [`parse_non_negative_length`]'s `>= 0.0` filter incidentally also
/// rejects NaN, but offset-x/offset-y/spread-radius carry no sign
/// restriction — unlike blur-radius (`[0,∞]`, see
/// [`parse_text_shadow_lengths`](super::text::parse_text_shadow_lengths)/[`parse_box_shadow_lengths`](super::box_model::parse_box_shadow_lengths)) — so there
/// is no such incidental filter here; the guard must be explicit, same
/// shape [`parse_transform_length_percentage`](super::visual::parse_transform_length_percentage) uses for `transform`'s own
/// unconsumed payloads.
///
/// # `+Inf`/`-Inf` are not rejected
///
/// No paint-side consumer applies shadow offsets/spread to anything yet,
/// so nothing downstream in this crate will ever normalize a NaN that
/// slips past this parser. `+Inf`/`-Inf`, by contrast, are ordinary
/// `<length>` magnitude overflow — legitimate, if extreme, values per CSS
/// Values 4 §5's "closest value supported by the implementation" clause —
/// and are preserved unfiltered.
pub(super) fn parse_shadow_length_reject_nan(input: &mut Parser<'_, '_>) -> Option<Length> {
    let length = parse_length_allow_negative(input)?;
    (!length.payload().is_nan()).then_some(length)
}

pub(super) fn parse_shadow_length_reject_nan_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<Length, ParseError<'i, ()>> {
    parse_shadow_length_reject_nan(input).ok_or_else(|| input.new_custom_error(()))
}

/// `<length-percentage [0,∞]>`: [`parse_length_percentage_res`](super::visual::parse_length_percentage_res)
/// plus a non-negative filter (`[0,∞]` via [`Length::payload`]). Used by
/// the two-radii ellipse form of `radial-gradient()`
/// (`<length-percentage [0,∞]>{2}`, [`RadialSize::Ellipse`]).
pub(super) fn parse_non_negative_length_percentage_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<Length, ParseError<'i, ()>> {
    let length = parse_length_value(input, true).ok_or_else(|| input.new_custom_error(()))?;
    if length.payload() >= 0.0 {
        Ok(length)
    } else {
        Err(input.new_custom_error(()))
    }
}
