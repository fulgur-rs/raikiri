//! Visual property parsers: backgrounds, gradients, `<position>`, clip-path
//! shapes, transforms, filters, masks, opacity and compositing.

use std::sync::Arc;

use cssparser::{ParseError, Parser, Token};

use crate::property::types::*;

use super::color::*;
use super::common::*;
use super::text::*;

/// `<opacity-value> = <number> | <percentage>` (CSS Color 4 §3.3
/// "Transparency: the opacity property"
/// <https://www.w3.org/TR/css-color-4/#transparency>). `<percentage>` uses
/// `expect_percentage`'s `unit_value` (already divided by 100) verbatim, no
/// 0%..100% range check.
///
/// # Deliberately does not clamp
///
/// Same §, verbatim: "Opacity values outside the range \[0, 1\] are not
/// invalid, and are preserved in specified values, but are clamped to the
/// range \[0, 1\] in computed values." Clamping is a **computed-value-time**
/// transform, so this parser — unlike [`parse_alpha_value`] (the `<alpha-value>`
/// grammar `rgb()`/`rgba()` use for their alpha channel, which shares the
/// exact same `<number> | <percentage>` grammar but clamps immediately at
/// parse time) — must preserve an out-of-range parse as-is. The two helpers
/// are kept separate rather than shared despite the identical grammar,
/// because sharing would silently store an already-clamped value at the
/// specified layer. The actual clamp lives in
/// [`crate::specified::SpecifiedValues::absolutize_with`] (phase 3) and its
/// page-context sibling.
///
/// # `!is_nan()` guard — NaN, but *not* `+Inf`/`-Inf`, must be rejected
///
/// `+Inf`/`-Inf` are spec-valid `<number>` values (CSS Color 4 §3.3 puts no
/// range restriction on `<opacity-value>`'s `<number>` alternative) and, per
/// CSS Values and Units Module Level 4 §5 "Numeric Data Types"
/// (<https://www.w3.org/TR/css-values-4/#numeric-types>), when a literal
/// exceeds the implementation's supported precision it "must be converted to
/// the closest value supported by the implementation" where "how the
/// implementation defines 'closest' is implementation-defined" — treating
/// IEEE-754 `Infinity` as that closest value (rather than e.g. `f32::MAX`)
/// is a deliberate, spec-permitted choice, not the only conformant answer,
/// and is what cssparser's `f64`->`f32` overflow naturally produces. They are
/// handled correctly by the phase-3 clamp above — `f32::clamp` maps
/// `+Inf`/`-Inf` to `1.0`/`0.0` exactly as it maps any other out-of-range
/// finite value, so a huge-magnitude literal like `opacity: -1e40` (which
/// the tokenizer's f64->f32 conversion overflows to `-Infinity`, not NaN)
/// must reach the clamp unrejected and become `0.0` (fully transparent),
/// not fall back to the initial `1.0` (fully opaque) by being dropped
/// here. **NaN is different** — `f32::clamp` returns `self` unchanged when
/// `self` is NaN (only a NaN *bound* panics), so an unguarded NaN would
/// sail through the clamp and reach
/// [`crate::computed::ComputedValues::opacity`] — this guard is what keeps
/// that field NaN-free for values that go through this parser (see that
/// field's doc for the "ordinary parse -> cascade pipeline" scoping of
/// that guarantee). An earlier iteration of this guard used `is_finite()`,
/// which rejects `+Inf`/`-Inf` too and wrongly dropped `-1e40`-shaped
/// declarations — this helper deliberately does **not** reuse
/// [`parse_nonneg_finite_number`](super::layout::parse_nonneg_finite_number)'s `is_finite()` pattern for that reason;
/// unlike `flex-grow`/`flex-shrink`, which reject `<number [0,∞]>` and so
/// have no legitimate use for `-Inf` in the first place, `opacity`'s
/// unbounded `<number>` grammar makes `+Inf`/`-Inf` legitimate inputs that
/// must reach the clamp.
///
/// `expect_number_stable`/`expect_percentage_stable` already correct the
/// one class of `NaN` a numeric token can actually carry (a huge-exponent,
/// zero-mantissa literal like `opacity: 0e999`, module doc above) before
/// this function ever sees the value, so in ordinary use `n`/`pct` here
/// are never `NaN`. This `!is_nan()` check is kept as defense-in-depth —
/// it costs nothing when the value isn't `NaN` and still protects
/// [`crate::computed::ComputedValues::opacity`]'s NaN-free invariant if
/// that upstream recovery is ever bypassed or extended incorrectly.
pub(super) fn parse_opacity_value(input: &mut Parser<'_, '_>) -> Option<f32> {
    let val = if let Ok(pct) = input.try_parse(|i| expect_percentage_stable(i)) {
        if pct.is_nan() {
            return None;
        }
        pct
    } else {
        let n = expect_number_stable(input).ok()?;
        if n.is_nan() {
            return None;
        }
        n
    };
    if input.try_parse(|i| i.expect_exhausted()).is_err() {
        return None;
    }
    Some(val)
}

/// Parse `visibility: <ident>` (CSS Display 3 §4
/// <https://www.w3.org/TR/css-display-3/#visibility>).
///
/// Value grammar (verbatim from the spec): `visible | hidden | collapse`. Accept all three
/// keywords (see the Scope carving section of [`Visibility`] —
/// the formatting-context-specific space-saving behavior of `collapse` is not implemented,
/// but the keyword itself is accepted as spec-valid). Compare identifiers
/// ASCII case-insensitively (as in [`parse_font_style`]).
pub(super) fn parse_visibility(input: &mut Parser<'_, '_>) -> Option<Visibility> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "visible" => Some(Visibility::Visible),
        "hidden" => Some(Visibility::Hidden),
        "collapse" => Some(Visibility::Collapse),
        _ => None,
    }
}

/// Shared parser for the CSS `<url>` value type (CSS Values and Units 4 §4.4
/// <https://www.w3.org/TR/css-values-4/#urls>) as a general property value.
/// Accepts only the forms described below.
///
/// Grammar: `<url> = <url()> | <src()>`.
/// `<url()> = url( <string> <url-modifier>* ) | <url-token>`. This helper accepts
/// only the two `<url()>` forms: unquoted `url(foo.png)` (which the tokenizer
/// produces as a `<url-token>`) and quoted `url("foo.png")` (the tokenizer sees the quote,
/// recognizes `url` as a function token, and reads a
/// `<string-token>` in the nested block). A `url()` with `<url-modifier>*`
/// (such as `crossorigin()`, §4.4.1) is rejected because the nested block is
/// not exhausted by `<string>` alone. This is an intentional unsupported case
/// caused by cssparser's block-exhaustion rule
/// (`Parser::parse_nested_block` returns Err unless its closure consumes the block).
/// `<src()>` is unsupported for a different reason: `expect_url()` only recognizes
/// function tokens named `url`, so it does not even recognize `src(...)` as a
/// valid function token here (independent of block exhaustion).
///
/// For a general property value, `<url>` includes only these two forms; it excludes a bare
/// `<string>` without a `url()` wrapper. The spec explicitly says
/// `"Some CSS contexts (such as @import) also allow a <url> to be
/// represented by a bare <string>, without the function wrapper"`;
/// thus accepting a bare string is legacy behavior limited to contexts such as `@import`,
/// not the general property's `<url>` value type.
/// (Properties with an explicit `[ <string> | <url> ]` alternative,
/// such as [`parse_target_url`](super::content::parse_target_url), handle `<string>` separately at the call site.)
///
/// A CSS-wide keyword (a bare identifier such as `inherit`) matches no form of this grammar,
/// so cssparser's `expect_url` returns Err and this helper returns
/// `None`. That fits the caller's `parse_value` convention:
/// `None` drops the entire declaration.
// Use plain `fn`, not `pub(crate)`: the caller (`parse_background_image`) is
// implemented in property.rs, and no other module calls this helper.
// Only widen visibility when a caller from another module actually lands, as with
// `parse_non_negative_length`/`parse_length_allow_negative` (precedents in this file
// whose docs explicitly name their outside callers); then add `pub(crate)` with
// a doc justification.
pub(crate) fn parse_url_value(input: &mut Parser<'_, '_>) -> Option<String> {
    input.expect_url().ok().map(|s| s.as_ref().to_string())
}

/// `<length-percentage>` with no sign restriction ([`parse_length_value`] with
/// `allow_percentage=true`, wrapped in `Result` for `try_parse` callers).
/// Used for [`CssPosition`] offsets, where negative values are spec-valid (see the docs for
/// [`parse_position_horizontal_edge`] and related functions).
pub(super) fn parse_length_percentage_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<Length, ParseError<'i, ()>> {
    parse_length_value(input, true).ok_or_else(|| input.new_custom_error(()))
}

/// Fold a `Percent` payload of [`CssPositionOffset::End`] into the equivalent
/// [`CssPositionOffset::Start`] (see the "Why two variants?" section in the
/// [`CssPositionOffset`] docs). `right 30%` (`End(Percent(30.0))`) is exactly
/// identical to `Start(Percent(70.0))`, so normalize to `Start` to keep one
/// canonical representation. `End` survives only for offsets that cannot be
/// represented as percentages (such as `right 10px`).
fn normalize_css_position_offset(offset: CssPositionOffset) -> CssPositionOffset {
    match offset {
        CssPositionOffset::End(Length::Percent(p)) => {
            CssPositionOffset::Start(Length::Percent(100.0 - p))
        }
        other => other,
    }
}

pub(crate) fn normalize_css_position(position: CssPosition) -> CssPosition {
    CssPosition {
        horizontal: normalize_css_position_offset(position.horizontal),
        vertical: normalize_css_position_offset(position.vertical),
    }
}

fn css_position_center() -> CssPositionOffset {
    CssPositionOffset::Start(Length::Percent(50.0))
}

/// Shared helper for reading an edge with an optional offset in the `<position>`
/// grammar (see [`CssPosition`]). Map `start_kw` (`left`/`top`) to [`CssPositionOffset::Start`]
/// and `end_kw` (`right`/`bottom`) to [`CssPositionOffset::End`].
/// See the [`parse_position_horizontal_edge`] docs for the second return value.
fn parse_position_edge(
    input: &mut Parser<'_, '_>,
    start_kw: &str,
    end_kw: &str,
) -> Option<(CssPositionOffset, bool)> {
    if input
        .try_parse(|i| i.expect_ident_matching(start_kw))
        .is_ok()
    {
        return Some(match input.try_parse(parse_length_percentage_res) {
            Ok(offset) => (CssPositionOffset::Start(offset), true),
            Err(_) => (CssPositionOffset::Start(Length::Percent(0.0)), false),
        });
    }
    if input.try_parse(|i| i.expect_ident_matching(end_kw)).is_ok() {
        return Some(match input.try_parse(parse_length_percentage_res) {
            Ok(offset) => (CssPositionOffset::End(offset), true),
            Err(_) => (CssPositionOffset::End(Length::Percent(0.0)), false),
        });
    }
    None
}

/// The `[ left | right ] <length-percentage>?` alternative of the `<position>`
/// grammar (see [`CssPosition`]): read a horizontal edge keyword
/// with an optional offset. With no offset, return the edge itself (offset
/// `0`). If neither `left` nor `right` matches, return `None`
/// without consuming a token.
///
/// The second return value says whether a `<length-percentage>` token was actually
/// authored, rather than supplying an implicit `0` for a missing offset. [`parse_position_branch3`]
/// (for `<bg-position>`, used by `background-position`) ignores it, but
/// [`parse_position_branch3_strict`] (for plain `<position>`, used by `object-position`)
/// uses it to detect the three-value form, where only one axis has an
/// authored offset (see the Grammar section of the `<position>` docs), and
/// reject it.
fn parse_position_horizontal_edge(input: &mut Parser<'_, '_>) -> Option<(CssPositionOffset, bool)> {
    parse_position_edge(input, "left", "right")
}

/// Vertical-axis counterpart to [`parse_position_horizontal_edge`] (`top`/`bottom`).
/// See that function's docs for the meaning of the second return value.
fn parse_position_vertical_edge(input: &mut Parser<'_, '_>) -> Option<(CssPositionOffset, bool)> {
    parse_position_edge(input, "top", "bottom")
}

/// `center | [ left | right ] <length-percentage>?`: the horizontal-axis
/// branch-three group (see [`parse_position_branch3`]). `center` has
/// no offset, so the second return value is always `false`
/// (see [`parse_position_horizontal_edge`]).
fn parse_position_horizontal_group(
    input: &mut Parser<'_, '_>,
) -> Option<(CssPositionOffset, bool)> {
    if input
        .try_parse(|i| i.expect_ident_matching("center"))
        .is_ok()
    {
        return Some((css_position_center(), false));
    }
    parse_position_horizontal_edge(input)
}

/// Vertical-axis counterpart to [`parse_position_horizontal_group`].
fn parse_position_vertical_group(input: &mut Parser<'_, '_>) -> Option<(CssPositionOffset, bool)> {
    if input
        .try_parse(|i| i.expect_ident_matching("center"))
        .is_ok()
    {
        return Some((css_position_center(), false));
    }
    parse_position_vertical_edge(input)
}

/// Shared core of [`parse_position_branch3`] and [`parse_position_branch3_strict`]:
/// describe the third `<position>` alternative once and return whether each axis
/// has an offset (two `bool` values). `center` never has an offset (`false`)
/// (see [`parse_position_horizontal_group`]).
fn parse_position_branch3_core(input: &mut Parser<'_, '_>) -> Option<(CssPosition, bool, bool)> {
    if let Some((horizontal, h_offset)) = parse_position_horizontal_edge(input) {
        let (vertical, v_offset) = parse_position_vertical_group(input)?;
        return Some((
            CssPosition {
                horizontal,
                vertical,
            },
            h_offset,
            v_offset,
        ));
    }
    if let Some((vertical, v_offset)) = parse_position_vertical_edge(input) {
        let (horizontal, h_offset) = parse_position_horizontal_group(input)?;
        return Some((
            CssPosition {
                horizontal,
                vertical,
            },
            h_offset,
            v_offset,
        ));
    }
    if input
        .try_parse(|i| i.expect_ident_matching("center"))
        .is_ok()
    {
        if let Some((vertical, v_offset)) = parse_position_vertical_edge(input) {
            return Some((
                CssPosition {
                    horizontal: css_position_center(),
                    vertical,
                },
                false,
                v_offset,
            ));
        }
        if let Some((horizontal, h_offset)) = parse_position_horizontal_edge(input) {
            return Some((
                CssPosition {
                    horizontal,
                    vertical: css_position_center(),
                },
                h_offset,
                false,
            ));
        }
        if input
            .try_parse(|i| i.expect_ident_matching("center"))
            .is_ok()
        {
            return Some((
                CssPosition {
                    horizontal: css_position_center(),
                    vertical: css_position_center(),
                },
                false,
                false,
            ));
        }
        return None;
    }
    None
}

/// The third `<position>` grammar alternative (see [`CssPosition`]):
/// `[ center | [ left | right ] <length-percentage>? ] && [ center | [ top |
/// bottom ] <length-percentage>? ]`. Both `&&` groups are required
/// in either order. If only one group can be read, return `None`
/// (the caller wraps this in `input.try_parse`) and let it rewind
/// the consumed tokens.
///
/// Try the first token as horizontal-only (`left`/`right`), then vertical-only
/// (`top`/`bottom`), then ambiguous `center`. Because `center` may belong
/// to either group, its axis is determined only after reading the second
/// token (the other group).
///
/// This is exactly the third alternative of `<bg-position>` (CSS Backgrounds 3 §2.6):
/// each group has its own `<length-percentage>?`, allowing an independently
/// optional offset on one axis of a three-value form. For plain
/// `<position>` (CSS Values 4 §8.3), instead use
/// Use [`parse_position_branch3_strict`] to reject this intermediate form —
/// `background-position` uses `<bg-position>`;
/// `object-position` uses `<position>`. This distinction
/// [`CssPosition`].
fn parse_position_branch3(input: &mut Parser<'_, '_>) -> Option<CssPosition> {
    let (position, _, _) = parse_position_branch3_core(input)?;
    Some(position)
}

/// Plain-`<position>` variant of [`parse_position_branch3`].
/// Its only difference: reject cases where the horizontal and vertical groups
/// **disagree about the presence of a `<length-percentage>` offset** (the three-value form).
///
/// # Why
///
/// CSS Values 4 §8.3 has no intermediate `<position>` form where an offset
/// is authored on only one axis. Its fourth alternative
/// (`<position-four>`; see the formal syntax on MDN's "`<position>` CSS type")
/// is `[[left|right] <length-percentage>] && [[top|bottom]
/// <length-percentage>]`: offsets are required on both axes, not optional (`?`).
/// The second alternative separately covers a bare keyword pair with no offsets
/// (the `&&` form of `<position-two>`). Thus the valid cases are either
/// offsets on both axes (four values), or offsets on neither axis;
/// one offset alone is always invalid.
///
/// `<bg-position>` (CSS Backgrounds 3 §2.6, for `background-position`)
/// has no such restriction. It explicitly allows the three-value form (the spec says
/// "For 3-value productions (which are not valid in `<position>`)"),
/// which is precisely how `<bg-position>` extends `<position>`. `object-position`
/// (CSS Images 3 §5.2, Value: `<position>`) lacks that extension;
/// reusing [`parse_position_branch3`] unchanged would wrongly accept a
/// three-value input such as `right 10px center`. It consumes every token,
/// leaving no leftover for the caller's `expect_exhausted` check to detect.
/// This sibling checks whether offsets are present symmetrically on both axes
/// to prevent that mistake.
pub(crate) fn parse_position_branch3_strict(input: &mut Parser<'_, '_>) -> Option<CssPosition> {
    let (position, h_offset, v_offset) = parse_position_branch3_core(input)?;
    if h_offset != v_offset {
        return None;
    }
    Some(position)
}

/// `[ <start_kw> | center | <end_kw> | <length-percentage> ]`: `<position>`
/// grammar second alternative (see [`CssPosition`]); shared axis helper.
/// Map `start_kw` (`left`/`top`) to `Start(0%)` and `end_kw`
/// (`right`/`bottom`) to `Start(100%)`. A bare `<length-percentage>` is accepted
/// only in this alternative: neither branch-three group accepts a bare length-percentage
/// alone. With no edge keyword, it is the value itself, not an offset,
/// and is stored as `Start`.
fn parse_position_branch2_axis(
    input: &mut Parser<'_, '_>,
    start_kw: &str,
    end_kw: &str,
) -> Option<CssPositionOffset> {
    if input
        .try_parse(|i| i.expect_ident_matching(start_kw))
        .is_ok()
    {
        return Some(CssPositionOffset::Start(Length::Percent(0.0)));
    }
    if input
        .try_parse(|i| i.expect_ident_matching("center"))
        .is_ok()
    {
        return Some(css_position_center());
    }
    if input.try_parse(|i| i.expect_ident_matching(end_kw)).is_ok() {
        return Some(CssPositionOffset::Start(Length::Percent(100.0)));
    }
    let lp = input.try_parse(parse_length_percentage_res).ok()?;
    Some(CssPositionOffset::Start(lp))
}

/// `[ left | center | right | <length-percentage> ]`: `<position>` grammar
/// second alternative, horizontal side (see [`CssPosition`]). A bare
/// `<length-percentage>` is accepted only here: neither branch-three
/// group accepts a bare length-percentage alone. With no edge keyword, it is
/// the value itself rather than an offset, and is stored as `Start`.
fn parse_position_branch2_horizontal(input: &mut Parser<'_, '_>) -> Option<CssPositionOffset> {
    parse_position_branch2_axis(input, "left", "right")
}

/// Vertical side of [`parse_position_branch2_horizontal`]
/// (`[ top | center | bottom | <length-percentage> ]`).
fn parse_position_branch2_vertical(input: &mut Parser<'_, '_>) -> Option<CssPositionOffset> {
    parse_position_branch2_axis(input, "top", "bottom")
}

// spec: https://www.w3.org/TR/css-transforms-1/#transform-origin-property
// Unlike background-position, origin permits at most two axis values followed
// by a Z length. Edge-offset positions are not part of this grammar.
pub(super) fn parse_transform_origin(input: &mut Parser<'_, '_>) -> Option<PropertyValue> {
    let (position, two_axes) = input
        .try_parse(|input| {
            let position = parse_position_branch2(input)
                .ok_or_else(|| input.new_custom_error::<(), ()>(()))?;
            Ok::<_, ParseError<'_, ()>>((position, true))
        })
        .or_else(|_| {
            input.try_parse(|input| {
                let vertical = match input.expect_ident()?.to_ascii_lowercase().as_str() {
                    "top" => 0.0,
                    "center" => 50.0,
                    "bottom" => 100.0,
                    _ => return Err(input.new_custom_error::<(), ()>(())),
                };
                let horizontal = match input.expect_ident()?.to_ascii_lowercase().as_str() {
                    "left" => 0.0,
                    "center" => 50.0,
                    "right" => 100.0,
                    _ => return Err(input.new_custom_error::<(), ()>(())),
                };
                Ok((
                    CssPosition {
                        horizontal: CssPositionOffset::Start(Length::Percent(horizontal)),
                        vertical: CssPositionOffset::Start(Length::Percent(vertical)),
                    },
                    true,
                ))
            })
        })
        .or_else(|_| {
            input
                .try_parse(parse_position_branch1_res)
                .map(|position| (position, false))
        })
        .ok()?;
    let z = if input.is_exhausted() {
        Length::Px(0.0)
    } else if two_axes {
        super::common::parse_length_allow_negative(input)?
    } else {
        return None;
    };
    input.expect_exhausted().ok()?;
    Some(PropertyValue::TransformOrigin(position, z))
}

/// The second alternative of the `<position>` grammar (see [`CssPosition`]): read
/// exactly two tokens, horizontal then vertical. Keyword reordering is forbidden;
/// unlike branch three, this is simple concatenation, not `&&`.
fn parse_position_branch2(input: &mut Parser<'_, '_>) -> Option<CssPosition> {
    let horizontal = parse_position_branch2_horizontal(input)?;
    let vertical = parse_position_branch2_vertical(input)?;
    Some(CssPosition {
        horizontal,
        vertical,
    })
}

/// The first alternative of the `<position>` grammar (see [`CssPosition`]): a single
/// keyword or a single `<length-percentage>`. The unspecified axis becomes
/// `center` (50%). Although not explicit in this spec, this follows the
/// `background-position` rule (CSS Backgrounds 3 §2.6): "if only one value is specified, the second
/// value is assumed to be center", represented by the general `<position>` type.
fn parse_position_branch1(input: &mut Parser<'_, '_>) -> Option<CssPosition> {
    if input
        .try_parse(|i| i.expect_ident_matching("center"))
        .is_ok()
    {
        return Some(CssPosition {
            horizontal: css_position_center(),
            vertical: css_position_center(),
        });
    }
    if input.try_parse(|i| i.expect_ident_matching("left")).is_ok() {
        return Some(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(0.0)),
            vertical: css_position_center(),
        });
    }
    if input
        .try_parse(|i| i.expect_ident_matching("right"))
        .is_ok()
    {
        return Some(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(100.0)),
            vertical: css_position_center(),
        });
    }
    if input.try_parse(|i| i.expect_ident_matching("top")).is_ok() {
        return Some(CssPosition {
            horizontal: css_position_center(),
            vertical: CssPositionOffset::Start(Length::Percent(0.0)),
        });
    }
    if input
        .try_parse(|i| i.expect_ident_matching("bottom"))
        .is_ok()
    {
        return Some(CssPosition {
            horizontal: css_position_center(),
            vertical: CssPositionOffset::Start(Length::Percent(100.0)),
        });
    }
    let lp = input.try_parse(parse_length_percentage_res).ok()?;
    Some(CssPosition {
        horizontal: CssPositionOffset::Start(lp),
        vertical: css_position_center(),
    })
}

/// Parse the `<position>` value type (see the [`CssPosition`] grammar).
/// # Alternative order: third → second → first (reverse of grammar order)
///
/// The three alternatives can overlap, so their trial order affects the result.
///
/// **Try the third alternative first (reorderable edges).** Its failure safely
/// rewinds the parser for this reason:
/// Consider `left 10px`. The third alternative greedily assigns `left` to the
///
/// horizontal group, followed by the optional offset `10px`. This leaves
/// no token for the vertical group (`&&` requires both), so the entire third
/// alternative fails and rewinds. For this particular input, trying the second
/// alternative first would also read `left` with `[left|center|right|<LP>]`
/// and then consume `10px` with `[top|center|bottom|<LP>]` as a bare
/// `<length-percentage>`, yielding the same horizontal value `left` (0%) and
/// vertical value `10px`. Thus this input alone does not depend on trial order:
/// it illustrates the safety invariant below. The third alternative can consume
/// extra tokens before failing but cannot succeed with the wrong result.
/// Order really matters for reordered keywords such as `top left`:
/// the second alternative requires horizontal→vertical and rejects `top`
/// as horizontal; only the third alternative's order-independent `&&` can
/// interpret it. It also matters for the edge-offset forms, especially
/// three- or four-value input
/// such as `bottom 10px right 20px`: the second alternative consumes at most
/// two tokens and would leave a remainder that `expect_exhausted` drops.
///
/// For the same reason, try the second alternative before the first: the first
/// consumes only one token. Without trying the second first, an input with
/// at least two tokens (such as `0px 20px`) would leave the second value,
/// and the caller's `expect_exhausted` (`rule.rs`) would drop the declaration.
///
/// This order is safe even if the third alternative only consumes a prefix and
/// leaves tokens behind: the second consumes at most two tokens and the first
/// at most one. Starting from the same position, neither can consume **more**
/// tokens than a successful third alternative and reduce leftovers to zero.
/// Once the third succeeds, it always leaves the fewest (possibly zero) tokens.
///
/// Wrap each `Option`-returning helper (`parse_position_branch3` and the others)
/// in `input.try_parse`. If a helper consumes tokens before failing, the caller's
/// `try_parse` rewinds the entire attempted alternative.
fn parse_position_branch3_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<CssPosition, ParseError<'i, ()>> {
    parse_position_branch3(input).ok_or_else(|| input.new_custom_error(()))
}

fn parse_position_branch2_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<CssPosition, ParseError<'i, ()>> {
    parse_position_branch2(input).ok_or_else(|| input.new_custom_error(()))
}

fn parse_position_branch1_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<CssPosition, ParseError<'i, ()>> {
    parse_position_branch1(input).ok_or_else(|| input.new_custom_error(()))
}

/// Parse `<bg-position>` (CSS Backgrounds 3 §2.6), a superset of plain `<position>` (CSS Values 4 §8.3) that permits three-value edge-offset forms. Used by `background-position`, the position component of the `background` shorthand, and gradient `at <position>` clauses. For plain `<position>` (which rejects three-value forms), use the sibling [`parse_position_strict`].
///
/// Try three alternatives ([`parse_position_branch3`] / [`parse_position_branch2`] / [`parse_position_branch1`])
/// in third → second → first order (see the alternative-order docs on [`parse_position_branch3_res`]).
pub fn parse_bg_position(input: &mut Parser<'_, '_>) -> Option<CssPosition> {
    let position = input
        .try_parse(parse_position_branch3_res)
        .or_else(|_| input.try_parse(parse_position_branch2_res))
        .or_else(|_| input.try_parse(parse_position_branch1_res))
        .ok()?;
    Some(normalize_css_position(position))
}

fn parse_position_branch3_strict_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<CssPosition, ParseError<'i, ()>> {
    parse_position_branch3_strict(input).ok_or_else(|| input.new_custom_error(()))
}

/// Parse the plain `<position>` value type (CSS Values 4 §8.3),
/// a sibling of [`parse_bg_position`] (for `<bg-position>` and `background-position`).
/// `object-position` (CSS Images 3 §5.2, Value: `<position>`) uses this parser.
/// Only the third alternative differs.
///
/// Replace [`parse_position_branch3`] with [`parse_position_branch3_strict`]
/// to reject the three-value edge-offset form (see that function's "Why" section).
/// The second and first alternatives ([`parse_position_branch2`]/[`parse_position_branch1`])
/// are identical for both grammars; the two parsers share them.
///
pub fn parse_position_strict(input: &mut Parser<'_, '_>) -> Option<CssPosition> {
    let position = input
        .try_parse(parse_position_branch3_strict_res)
        .or_else(|_| input.try_parse(parse_position_branch2_res))
        .or_else(|_| input.try_parse(parse_position_branch1_res))
        .ok()?;
    Some(normalize_css_position(position))
}

/// Parse `background-image: <bg-image>` (see the [`BackgroundImage`] docs for the
/// grammar: `<image> | none`, with `<image> = <url> | <gradient>`).
///
/// Try the `none` keyword before a `<gradient>` function.
/// The order does not affect the result: [`parse_url_value`] for `<url>` cannot
/// match any function token. It does match the order used by sibling parsers
/// with keyword-versus-function alternatives, such as [`parse_position`](super::box_model::parse_position).
pub(super) fn parse_background_image(input: &mut Parser<'_, '_>) -> Option<BackgroundImage> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(BackgroundImage::None);
    }
    if let Ok(gradient) = input.try_parse(parse_gradient) {
        return Some(BackgroundImage::Gradient(gradient));
    }
    parse_url_value(input).map(BackgroundImage::Url)
}

/// Parse `mask-image: <mask-reference>` (one layer; see the scope carving
/// section of the [`MaskImage`] docs).
///
/// Its grammar (`none | <image> | <mask-source>`, with `<image> = <url> |
/// <gradient>` and `<mask-source> = <url>`) has the same concrete syntax as
/// [`parse_background_image`]'s `<bg-image> = <url> | <gradient>`
/// (see the [`MaskImage`] section "The `url` alternatives of `<mask-source>` and `<image>`
/// have the same concrete syntax"). Aliasing [`BackgroundImage`] allows
/// both to share the parser body.
pub(crate) fn parse_mask_image(input: &mut Parser<'_, '_>) -> Option<MaskImage> {
    parse_background_image(input)
}

/// Parse `<geometry-box>` (see the [`GeometryBox`] grammar: seven
/// keywords).
fn parse_geometry_box(input: &mut Parser<'_, '_>) -> Option<GeometryBox> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "border-box" => Some(GeometryBox::BorderBox),
        "padding-box" => Some(GeometryBox::PaddingBox),
        "content-box" => Some(GeometryBox::ContentBox),
        "margin-box" => Some(GeometryBox::MarginBox),
        "fill-box" => Some(GeometryBox::FillBox),
        "stroke-box" => Some(GeometryBox::StrokeBox),
        "view-box" => Some(GeometryBox::ViewBox),
        _ => None,
    }
}

/// `clip-path: <clip-source> | [ <basic-shape> || <geometry-box> ] | none`
/// (see the [`ClipPath`] docs).
///
/// `none` → `<clip-source>` (`<url>`) → `[ <basic-shape> || <geometry-box> ]`
/// Try them in that order. The final `[ … || … ]` accepts either ordering
/// (shape before box or box before shape), so try both. `basic-shape` function
/// tokens and geometry-box identifiers are mutually exclusive; order does not affect the result.
pub(super) fn parse_clip_path(input: &mut Parser<'_, '_>) -> Option<ClipPath> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(ClipPath::None);
    }
    // Try the basic-shape / geometry-box pair — either order, at least one.
    // First try basic-shape with optional trailing geometry-box.
    if let Ok(shape_with_box) = input.try_parse(|i| -> Result<ClipPath, ParseError<'_, ()>> {
        let shape = parse_basic_shape(i).ok_or_else(|| i.new_custom_error(()))?;
        let geometry_box = i
            .try_parse(|j| -> Result<GeometryBox, ParseError<'_, ()>> {
                parse_geometry_box(j).ok_or_else(|| j.new_custom_error(()))
            })
            .ok();
        Ok(ClipPath::BasicShape {
            shape: Box::new(shape),
            geometry_box,
        })
    }) {
        return Some(shape_with_box);
    }
    // Then try geometry-box with optional trailing basic-shape (the other order).
    if let Ok(box_with_shape) = input.try_parse(|i| -> Result<ClipPath, ParseError<'_, ()>> {
        let geometry_box = parse_geometry_box(i).ok_or_else(|| i.new_custom_error(()))?;
        if let Ok(shape) = i.try_parse(|j| -> Result<BasicShape, ParseError<'_, ()>> {
            parse_basic_shape(j).ok_or_else(|| j.new_custom_error(()))
        }) {
            Ok(ClipPath::BasicShape {
                shape: Box::new(shape),
                geometry_box: Some(geometry_box),
            })
        } else {
            Ok(ClipPath::GeometryBox(geometry_box))
        }
    }) {
        return Some(box_with_shape);
    }
    parse_url_value(input).map(ClipPath::Url)
}

/// `<basic-shape>` — `circle()` / `ellipse()` / `inset()` / `polygon()` / `path()`.
fn parse_basic_shape(input: &mut Parser<'_, '_>) -> Option<BasicShape> {
    let name = input
        .try_parse(|i| -> Result<String, ParseError<'_, ()>> {
            let token = i.next()?.clone();
            match token {
                Token::Function(n) => Ok(n.as_ref().to_ascii_lowercase()),
                _ => Err(i.new_unexpected_token_error(token)),
            }
        })
        .ok()?;
    match name.as_str() {
        "circle" => input
            .parse_nested_block(|i| -> Result<CircleShape, ParseError<'_, ()>> {
                parse_circle_shape(i).ok_or_else(|| i.new_custom_error(()))
            })
            .ok()
            .map(BasicShape::Circle),
        "ellipse" => input
            .parse_nested_block(|i| -> Result<EllipseShape, ParseError<'_, ()>> {
                parse_ellipse_shape(i).ok_or_else(|| i.new_custom_error(()))
            })
            .ok()
            .map(BasicShape::Ellipse),
        "inset" => input
            .parse_nested_block(|i| -> Result<InsetShape, ParseError<'_, ()>> {
                parse_inset_shape(i).ok_or_else(|| i.new_custom_error(()))
            })
            .ok()
            .map(BasicShape::Inset),
        "polygon" => input
            .parse_nested_block(|i| -> Result<PolygonShape, ParseError<'_, ()>> {
                parse_polygon_shape(i).ok_or_else(|| i.new_custom_error(()))
            })
            .ok()
            .map(BasicShape::Polygon),
        "path" => input
            .parse_nested_block(|i| -> Result<PathShape, ParseError<'_, ()>> {
                parse_path_shape(i).ok_or_else(|| i.new_custom_error(()))
            })
            .ok()
            .map(BasicShape::Path),
        _ => None,
    }
}

fn parse_fill_rule(input: &mut Parser<'_, '_>) -> Option<FillRule> {
    let ident = input.expect_ident().ok()?.as_ref().to_ascii_lowercase();
    match ident.as_str() {
        "nonzero" => Some(FillRule::NonZero),
        "evenodd" => Some(FillRule::EvenOdd),
        _ => None,
    }
}

/// Try to parse `<shape-radius>` as `closest-side` / `farthest-side` or `<length-percentage [0,∞]>`.
fn try_parse_shape_radius(input: &mut Parser<'_, '_>) -> Option<ShapeRadius> {
    if let Ok(sr) = input.try_parse(|i| -> Result<ShapeRadius, ParseError<'_, ()>> {
        let ident = i.expect_ident()?.as_ref().to_ascii_lowercase();
        match ident.as_str() {
            "closest-side" => Ok(ShapeRadius::ClosestSide),
            "farthest-side" => Ok(ShapeRadius::FarthestSide),
            _ => Err(i.new_custom_error(())),
        }
    }) {
        return Some(sr);
    }
    let length = parse_length_value(input, true)?;
    if length.payload() < 0.0 || length.payload().is_nan() {
        return None;
    }
    Some(ShapeRadius::Length(length))
}

fn parse_circle_shape(input: &mut Parser<'_, '_>) -> Option<CircleShape> {
    let radius = input
        .try_parse(|i| -> Result<ShapeRadius, ParseError<'_, ()>> {
            try_parse_shape_radius(i).ok_or_else(|| i.new_custom_error(()))
        })
        .ok();
    let position = if input.try_parse(|i| i.expect_ident_matching("at")).is_ok() {
        let pos = parse_position_strict(input)?;
        Some(pos)
    } else {
        None
    };
    if !input.is_exhausted() {
        return None;
    }
    Some(CircleShape { radius, position })
}

fn parse_ellipse_shape(input: &mut Parser<'_, '_>) -> Option<EllipseShape> {
    let radius_x = input
        .try_parse(|i| -> Result<ShapeRadius, ParseError<'_, ()>> {
            try_parse_shape_radius(i).ok_or_else(|| i.new_custom_error(()))
        })
        .ok();
    let radius_y = if radius_x.is_some() {
        input
            .try_parse(|i| -> Result<ShapeRadius, ParseError<'_, ()>> {
                try_parse_shape_radius(i).ok_or_else(|| i.new_custom_error(()))
            })
            .ok()
    } else {
        None
    };
    let position = if input.try_parse(|i| i.expect_ident_matching("at")).is_ok() {
        let pos = parse_position_strict(input)?;
        Some(pos)
    } else {
        None
    };
    if !input.is_exhausted() {
        return None;
    }
    Some(EllipseShape {
        radius_x,
        radius_y,
        position,
    })
}

fn parse_inset_shape(input: &mut Parser<'_, '_>) -> Option<InsetShape> {
    let mut insets: Vec<Length> = Vec::new();
    for _ in 0..4 {
        if let Ok(lp) = input.try_parse(|i| -> Result<Length, ParseError<'_, ()>> {
            parse_length_value(i, true).ok_or_else(|| i.new_custom_error(()))
        }) {
            if lp.payload().is_nan() {
                return None;
            }
            insets.push(lp);
        } else {
            break;
        }
    }
    if insets.is_empty() {
        return None;
    }
    let border_radius = if input
        .try_parse(|i| i.expect_ident_matching("round"))
        .is_ok()
    {
        Some(parse_inset_border_radius(input)?)
    } else {
        None
    };
    if !input.is_exhausted() {
        return None;
    }
    let (top, right, bottom, left) = match insets.len() {
        1 => (insets[0], insets[0], insets[0], insets[0]),
        2 => (insets[0], insets[1], insets[0], insets[1]),
        3 => (insets[0], insets[1], insets[2], insets[1]),
        4 => (insets[0], insets[1], insets[2], insets[3]),
        _ => unreachable!(),
    };
    Some(InsetShape {
        top,
        right,
        bottom,
        left,
        border_radius,
    })
}

fn parse_inset_border_radius(input: &mut Parser<'_, '_>) -> Option<InsetBorderRadius> {
    let mut horiz: Vec<Length> = Vec::new();
    for _ in 0..4 {
        if let Ok(lp) = input.try_parse(|i| -> Result<Length, ParseError<'_, ()>> {
            let l = parse_length_value(i, true).ok_or_else(|| i.new_custom_error(()))?;
            if l.payload() < 0.0 || l.payload().is_nan() {
                return Err(i.new_custom_error(()));
            }
            Ok(l)
        }) {
            horiz.push(lp);
        } else {
            break;
        }
    }
    if horiz.is_empty() {
        return None;
    }
    let vertical = if input.try_parse(|i| i.expect_delim('/')).is_ok() {
        let mut vert: Vec<Length> = Vec::new();
        for _ in 0..4 {
            if let Ok(lp) = input.try_parse(|i| -> Result<Length, ParseError<'_, ()>> {
                let l = parse_length_value(i, true).ok_or_else(|| i.new_custom_error(()))?;
                if l.payload() < 0.0 || l.payload().is_nan() {
                    return Err(i.new_custom_error(()));
                }
                Ok(l)
            }) {
                vert.push(lp);
            } else {
                break;
            }
        }
        if vert.is_empty() {
            return None;
        }
        Some(vert)
    } else {
        None
    };
    let horiz_expanded = expand_to_four(&horiz);
    let vert_expanded = vertical.as_ref().map(|v| expand_to_four(v));
    Some(InsetBorderRadius {
        horizontal: horiz_expanded,
        vertical: vert_expanded,
    })
}

fn expand_to_four(values: &[Length]) -> [Length; 4] {
    match values.len() {
        1 => [values[0]; 4],
        2 => [values[0], values[1], values[0], values[1]],
        3 => [values[0], values[1], values[2], values[1]],
        4 => [values[0], values[1], values[2], values[3]],
        _ => unreachable!(),
    }
}

fn parse_polygon_shape(input: &mut Parser<'_, '_>) -> Option<PolygonShape> {
    let mut fill_rule = FillRule::NonZero;
    let mut fill_rule_consumed = false;
    if let Ok(fr) = input.try_parse(|i| -> Result<FillRule, ParseError<'_, ()>> {
        parse_fill_rule(i).ok_or_else(|| i.new_custom_error(()))
    }) {
        fill_rule = fr;
        fill_rule_consumed = true;
    }
    if fill_rule_consumed {
        let _ = input.try_parse(|i| i.expect_comma()).ok();
    }
    let round = if input
        .try_parse(|i| i.expect_ident_matching("round"))
        .is_ok()
    {
        let len = parse_length_value(input, true)?;
        if len.payload() < 0.0 || len.payload().is_nan() {
            return None;
        }
        let _ = input.try_parse(|i| i.expect_comma()).ok();
        Some(len)
    } else {
        None
    };
    let mut points: Vec<(Length, Length)> = Vec::new();
    loop {
        let point = input.try_parse(|i| -> Result<(Length, Length), ParseError<'_, ()>> {
            let x = parse_length_value(i, true).ok_or_else(|| i.new_custom_error(()))?;
            if x.payload().is_nan() {
                return Err(i.new_custom_error(()));
            }
            let y = parse_length_value(i, true).ok_or_else(|| i.new_custom_error(()))?;
            if y.payload().is_nan() {
                return Err(i.new_custom_error(()));
            }
            Ok((x, y))
        });
        match point {
            Ok(p) => {
                points.push(p);
                let _ = input.try_parse(|i| i.expect_comma()).ok();
            }
            Err(_) => break,
        }
    }
    if points.is_empty() {
        return None;
    }
    if !input.is_exhausted() {
        return None;
    }
    Some(PolygonShape {
        fill_rule,
        round,
        points,
    })
}

fn parse_path_shape(input: &mut Parser<'_, '_>) -> Option<PathShape> {
    let mut fill_rule = FillRule::NonZero;
    let mut consumed = false;
    if let Ok(fr) = input.try_parse(|i| -> Result<FillRule, ParseError<'_, ()>> {
        parse_fill_rule(i).ok_or_else(|| i.new_custom_error(()))
    }) {
        fill_rule = fr;
        consumed = true;
    }
    if consumed {
        if input.try_parse(|i| i.expect_comma()).is_err() {
            return None;
        }
    } else {
        let _ = input.try_parse(|i| i.expect_comma()).ok();
    }
    let path_str = input.expect_string().ok()?.as_ref().to_string();
    if !input.is_exhausted() {
        return None;
    }
    Some(PathShape {
        fill_rule,
        path: path_str,
    })
}

/// `<number>` for the `transform` functions that take a bare number
/// (`matrix()`/`scale()`/`scaleX()`/`scaleY()`) — CSS Transforms Level 1
/// §9.1 places no range restriction on any of these, so — unlike
/// [`parse_filter_amount`], whose `>= 0.0` filter incidentally also
/// rejects NaN (`NaN >= 0.0` is `false` under IEEE 754) — this helper
/// cannot lean on a range check to catch the same hazard and must guard
/// explicitly. Same class of hazard as
/// [`PropertyValue::Opacity`]'s parser (`parse_opacity_value`'s
/// `!is_nan()` guard doc is canonical for the *mechanism*) — but the
/// *reason a guard is needed at all* here is `transform`-specific: this
/// crate's `Length`-typed box fields (`width`/`margin`/etc.) can go
/// unguarded because `raikiri-dom::layout::sanitize_finite` normalizes
/// NaN at its single consuming sink (the [`Length`] docs' "The type does not
/// express its layer" note describes this pipeline); `transform`'s
/// `f32`/[`Length`]/[`Angle`] payloads have no such downstream sink (no
/// paint-side consumer exists yet at all), so nothing else in this
/// crate's pipeline will ever normalize a NaN that slips past this
/// parser. `+Inf`/`-Inf` (ordinary magnitude overflow, a different hazard
/// class per `parse_opacity_value` doc's own distinction) are preserved —
/// no bound restricts a plain `<number>`.
///
/// `expect_number_stable` already corrects a huge-exponent, zero-mantissa
/// literal like `scale(0e999)` (module doc's "Numeric-token NaN
/// stabilization" section) before this function sees `n`, so in ordinary
/// use this `!is_nan()` check is defense-in-depth, not the primary
/// mechanism.
pub(crate) fn parse_transform_number(input: &mut Parser<'_, '_>) -> Option<f32> {
    let n = expect_number_stable(input).ok()?;
    (!n.is_nan()).then_some(n)
}

/// `<length-percentage>` for the `transform` functions that take one
/// (`translate()`/`translateX()`/`translateY()`) — same `!is_nan()`
/// rationale as [`parse_transform_number`], applied on top of
/// [`parse_length_value`]'s output instead of a bare `<number>`. Unlike
/// [`parse_non_negative_length`] (whose `>= 0.0` filter incidentally also
/// rejects NaN), `translate()`'s `<length-percentage>` has no sign
/// restriction, so there is no such incidental filter here — the guard
/// must be explicit, same shape as [`parse_transform_number`]'s own doc.
/// [`parse_length_value`] already routes through `next_numeric_stable`
/// (module doc above), so this is likewise defense-in-depth in ordinary
/// use.
pub(crate) fn parse_transform_length_percentage(input: &mut Parser<'_, '_>) -> Option<Length> {
    let length = parse_length_value(input, true)?;
    (!length.payload().is_nan()).then_some(length)
}

/// `[<angle> | <zero>]` for `transform`'s `rotate()`/`skew()`/`skewX()`/
/// `skewY()` and `filter`'s `hue-rotate()` (both share this exact grammar,
/// CSS Transforms Level 1 §9.1 / CSS Filter Effects Level 1 §6.1 — neither
/// restricts `<angle>`'s range; `hue-rotate()`'s own text additionally
/// states implementations "must not normalize" the value, ruling out a
/// modulo-360 transform here). Wraps [`parse_angle`] (the shared
/// gradient-facing helper, `Result`-returning) with the same `!is_nan()`
/// guard [`parse_transform_number`] applies. `parse_angle` acquires its
/// token via `next_numeric_stable` (module doc above), which corrects a
/// huge-*exponent* literal like `rotate(0e999deg)` before `parse_angle`
/// ever computes degrees from it, so — same as the guards above — this is
/// defense-in-depth rather than the primary mechanism in ordinary use.
/// The guard is local so the shared gradient parser keeps its existing
/// behavior.
pub(crate) fn parse_angle_reject_nan(input: &mut Parser<'_, '_>) -> Option<Angle> {
    let angle = input.try_parse(parse_angle).ok()?;
    (!angle.0.is_nan()).then_some(angle)
}

fn parse_matrix_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<TransformFunction, ParseError<'i, ()>> {
    let a = parse_transform_number(input).ok_or_else(|| input.new_custom_error(()))?;
    input.expect_comma()?;
    let b = parse_transform_number(input).ok_or_else(|| input.new_custom_error(()))?;
    input.expect_comma()?;
    let c = parse_transform_number(input).ok_or_else(|| input.new_custom_error(()))?;
    input.expect_comma()?;
    let d = parse_transform_number(input).ok_or_else(|| input.new_custom_error(()))?;
    input.expect_comma()?;
    let e = parse_transform_number(input).ok_or_else(|| input.new_custom_error(()))?;
    input.expect_comma()?;
    let f = parse_transform_number(input).ok_or_else(|| input.new_custom_error(()))?;
    Ok(TransformFunction::Matrix([a, b, c, d, e, f]))
}

/// `translate(<length-percentage>, <length-percentage>?)` — 2nd argument
/// omitted defaults to `0` (CSS Transforms Level 1 §9.1 grammar's `?`
/// multiplier on the 2nd slot; the spec text names this default
/// explicitly in the 1-argument `translate()` case).
pub(crate) fn parse_translate_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<TransformFunction, ParseError<'i, ()>> {
    let tx = parse_transform_length_percentage(input).ok_or_else(|| input.new_custom_error(()))?;
    let ty = input
        .try_parse(|i| -> Result<Length, ParseError<'_, ()>> {
            i.expect_comma()?;
            parse_transform_length_percentage(i).ok_or_else(|| i.new_custom_error(()))
        })
        .unwrap_or(Length::Px(0.0));
    Ok(TransformFunction::Translate(tx, ty))
}

fn parse_translate_x_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<TransformFunction, ParseError<'i, ()>> {
    parse_transform_length_percentage(input)
        .map(TransformFunction::TranslateX)
        .ok_or_else(|| input.new_custom_error(()))
}

fn parse_translate_y_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<TransformFunction, ParseError<'i, ()>> {
    parse_transform_length_percentage(input)
        .map(TransformFunction::TranslateY)
        .ok_or_else(|| input.new_custom_error(()))
}

/// `scale(<number>, <number>?)` — 2nd argument omitted **copies the 1st**
/// (CSS Transforms Level 1 §9.1's 1-argument `scale()` text: "the second
/// value defaults to the same value as the first"), unlike `translate()`'s
/// "defaults to 0" or `skew()`'s "defaults to 0deg" — three different
/// defaulting rules across the three 2-argument functions.
pub(crate) fn parse_scale_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<TransformFunction, ParseError<'i, ()>> {
    let sx = parse_transform_number(input).ok_or_else(|| input.new_custom_error(()))?;
    let sy = input
        .try_parse(|i| -> Result<f32, ParseError<'_, ()>> {
            i.expect_comma()?;
            parse_transform_number(i).ok_or_else(|| i.new_custom_error(()))
        })
        .unwrap_or(sx);
    Ok(TransformFunction::Scale(sx, sy))
}

fn parse_scale_x_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<TransformFunction, ParseError<'i, ()>> {
    parse_transform_number(input)
        .map(TransformFunction::ScaleX)
        .ok_or_else(|| input.new_custom_error(()))
}

fn parse_scale_y_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<TransformFunction, ParseError<'i, ()>> {
    parse_transform_number(input)
        .map(TransformFunction::ScaleY)
        .ok_or_else(|| input.new_custom_error(()))
}

fn parse_rotate_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<TransformFunction, ParseError<'i, ()>> {
    parse_angle_reject_nan(input)
        .map(TransformFunction::Rotate)
        .ok_or_else(|| input.new_custom_error(()))
}

/// `skew([<angle> | <zero>], [<angle> | <zero>]?)` — 2nd argument omitted
/// defaults to `0deg` (CSS Transforms Level 1 §9.1's 1-argument `skew()`
/// text), the same "defaults to 0" shape as `translate()` (unlike
/// `scale()`'s "copies the 1st" — see `parse_scale_args` doc).
pub(crate) fn parse_skew_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<TransformFunction, ParseError<'i, ()>> {
    let ax = parse_angle_reject_nan(input).ok_or_else(|| input.new_custom_error(()))?;
    let ay = input
        .try_parse(|i| -> Result<Angle, ParseError<'_, ()>> {
            i.expect_comma()?;
            parse_angle_reject_nan(i).ok_or_else(|| i.new_custom_error(()))
        })
        .unwrap_or(Angle(0.0));
    Ok(TransformFunction::Skew(ax, ay))
}

fn parse_skew_x_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<TransformFunction, ParseError<'i, ()>> {
    parse_angle_reject_nan(input)
        .map(TransformFunction::SkewX)
        .ok_or_else(|| input.new_custom_error(()))
}

fn parse_skew_y_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<TransformFunction, ParseError<'i, ()>> {
    parse_angle_reject_nan(input)
        .map(TransformFunction::SkewY)
        .ok_or_else(|| input.new_custom_error(()))
}

/// Dispatch one `<transform-function>` (CSS Transforms Level 1 §9.1; see
/// [`TransformFunction`]) by its function-token name
/// (the same pattern as [`parse_gradient`]). Three-dimensional function names
/// (such as `translate3d`/`rotate3d`/`matrix3d`/`perspective`, §10) fall
/// through to the `_` arm as unrecognized and are rejected
/// (see the Non-goal section of [`TransformFunction`]).
pub(crate) fn parse_transform_function<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<TransformFunction, ParseError<'i, ()>> {
    let name = match input.next()?.clone() {
        Token::Function(name) => name,
        token => return Err(input.new_unexpected_token_error(token)),
    };
    match name.as_ref().to_ascii_lowercase().as_str() {
        "matrix" => input.parse_nested_block(parse_matrix_args),
        "translate" => input.parse_nested_block(parse_translate_args),
        "translatex" => input.parse_nested_block(parse_translate_x_args),
        "translatey" => input.parse_nested_block(parse_translate_y_args),
        "scale" => input.parse_nested_block(parse_scale_args),
        "scalex" => input.parse_nested_block(parse_scale_x_args),
        "scaley" => input.parse_nested_block(parse_scale_y_args),
        "rotate" => input.parse_nested_block(parse_rotate_args),
        "skew" => input.parse_nested_block(parse_skew_args),
        "skewx" => input.parse_nested_block(parse_skew_x_args),
        "skewy" => input.parse_nested_block(parse_skew_y_args),
        _ => Err(input.new_custom_error(())),
    }
}

/// Parse `transform: none | <transform-list>` (CSS Transforms Level 1 §4;
/// `<transform-list> = <transform-function>[+]`
/// <https://www.w3.org/TR/css-transforms-1/#typedef-transform-list> —
/// **whitespace**-separated, not comma-separated, one or more) values.
/// Repeat until `try_parse` fails, as in the `||` loop of
/// [`parse_text_decoration_line`]. cssparser's `Parser::next`/`try_parse`
/// automatically skip whitespace between tokens, so explicit separator handling
/// is unnecessary.
pub(super) fn parse_transform(input: &mut Parser<'_, '_>) -> Option<Vec<TransformFunction>> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(Vec::new());
    }
    let mut functions = Vec::new();
    while let Ok(function) = input.try_parse(parse_transform_function) {
        functions.push(function);
    }
    (!functions.is_empty()).then_some(functions)
}

/// `<number-percentage>` for the 7 `filter` amount functions
/// (`brightness()`/`contrast()`/`grayscale()`/`invert()`/`opacity()`/
/// `saturate()`/`sepia()`, CSS Filter Effects Level 1 §6.1). Each states
/// "Negative values are not allowed" with no opacity-property-style
/// specified/computed split (`filter`'s own Computed value is "as
/// specified", [`FilterFunction`] doc's "Range restriction is reject, not
/// clamp" section) — so this crate rejects a negative parse outright
/// (`None`), matching [`parse_nonneg_finite_number`](super::layout::parse_nonneg_finite_number)'s (flex-grow/
/// flex-shrink) reject-at-parse precedent rather than
/// [`parse_opacity_value`]'s preserve-then-clamp one.
///
/// No separate `!is_nan()` guard is needed: `v >= 0.0` is `false` for NaN
/// under IEEE 754 comparison semantics, so the same range check that
/// rejects an ordinary negative value would incidentally also reject a
/// NaN parse — unlike [`parse_transform_number`] (whose callers have no
/// range restriction to lean on and must guard explicitly), this helper's
/// range restriction alone would do the job. In practice a huge-exponent
/// literal like `brightness(0e999)` no longer reaches this check as NaN
/// at all — `expect_number_stable`/`expect_percentage_stable` (module
/// doc's "Numeric-token NaN stabilization" section) already correct it to
/// `0.0` at acquisition, so `v >= 0.0` accepts it normally instead of
/// incidentally rejecting it.
pub(crate) fn parse_filter_amount<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<f32, ParseError<'i, ()>> {
    let v = if let Ok(pct) = input.try_parse(|i| expect_percentage_stable(i)) {
        pct
    } else {
        expect_number_stable(input)?
    };
    if v >= 0.0 {
        Ok(v)
    } else {
        Err(input.new_custom_error(()))
    }
}

fn parse_blur_args<'i>(input: &mut Parser<'i, '_>) -> Result<FilterFunction, ParseError<'i, ()>> {
    let length = input
        .try_parse(|i| -> Result<Length, ParseError<'_, ()>> {
            parse_non_negative_length(i).ok_or_else(|| i.new_custom_error(()))
        })
        .unwrap_or(Length::Px(0.0));
    Ok(FilterFunction::Blur(length))
}

fn parse_brightness_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<FilterFunction, ParseError<'i, ()>> {
    Ok(FilterFunction::Brightness(
        input.try_parse(parse_filter_amount).unwrap_or(1.0),
    ))
}

fn parse_contrast_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<FilterFunction, ParseError<'i, ()>> {
    Ok(FilterFunction::Contrast(
        input.try_parse(parse_filter_amount).unwrap_or(1.0),
    ))
}

fn parse_grayscale_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<FilterFunction, ParseError<'i, ()>> {
    Ok(FilterFunction::Grayscale(
        input.try_parse(parse_filter_amount).unwrap_or(1.0),
    ))
}

fn parse_hue_rotate_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<FilterFunction, ParseError<'i, ()>> {
    let angle = input
        .try_parse(|i| -> Result<Angle, ParseError<'_, ()>> {
            parse_angle_reject_nan(i).ok_or_else(|| i.new_custom_error(()))
        })
        .unwrap_or(Angle(0.0));
    Ok(FilterFunction::HueRotate(angle))
}

fn parse_invert_args<'i>(input: &mut Parser<'i, '_>) -> Result<FilterFunction, ParseError<'i, ()>> {
    Ok(FilterFunction::Invert(
        input.try_parse(parse_filter_amount).unwrap_or(1.0),
    ))
}

fn parse_filter_opacity_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<FilterFunction, ParseError<'i, ()>> {
    Ok(FilterFunction::Opacity(
        input.try_parse(parse_filter_amount).unwrap_or(1.0),
    ))
}

fn parse_saturate_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<FilterFunction, ParseError<'i, ()>> {
    Ok(FilterFunction::Saturate(
        input.try_parse(parse_filter_amount).unwrap_or(1.0),
    ))
}

fn parse_sepia_args<'i>(input: &mut Parser<'i, '_>) -> Result<FilterFunction, ParseError<'i, ()>> {
    Ok(FilterFunction::Sepia(
        input.try_parse(parse_filter_amount).unwrap_or(1.0),
    ))
}

/// `drop-shadow(<color>? && <length>{2,3})` — CSS Filter Effects Level 1
/// §6.1: "Values are interpreted as for box-shadow but with the optional
/// 3rd `<length>` value being the standard deviation instead of blur
/// radius" — grammar-identical to `text-shadow`'s own `<shadow>` syntax
/// (no spread, no inset), so [`parse_drop_shadow_item`] reuses the item grammar
/// while keeping the filter path's existing plain-length behavior
/// (see [`FilterFunction::DropShadow`]). Its offset-x/offset-y still go
/// through [`parse_shadow_length_reject_nan`]'s `!is_nan()` guard.
pub(crate) fn parse_drop_shadow_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<FilterFunction, ParseError<'i, ()>> {
    let item = parse_drop_shadow_item(input).ok_or_else(|| input.new_custom_error(()))?;
    Ok(FilterFunction::DropShadow(item))
}

/// Dispatch one `<filter-function>` (CSS Filter Effects Level 1 §6; see
/// [`FilterFunction`]) by its function-token name. Do not include the `<url>`
/// alternative ([`parse_filter`] tries it separately); a function named `url`
/// is treated as unknown and rejected here.
fn parse_filter_function<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<FilterFunction, ParseError<'i, ()>> {
    let name = match input.next()?.clone() {
        Token::Function(name) => name,
        token => return Err(input.new_unexpected_token_error(token)),
    };
    match name.as_ref().to_ascii_lowercase().as_str() {
        "blur" => input.parse_nested_block(parse_blur_args),
        "brightness" => input.parse_nested_block(parse_brightness_args),
        "contrast" => input.parse_nested_block(parse_contrast_args),
        "grayscale" => input.parse_nested_block(parse_grayscale_args),
        "hue-rotate" => input.parse_nested_block(parse_hue_rotate_args),
        "invert" => input.parse_nested_block(parse_invert_args),
        "opacity" => input.parse_nested_block(parse_filter_opacity_args),
        "saturate" => input.parse_nested_block(parse_saturate_args),
        "sepia" => input.parse_nested_block(parse_sepia_args),
        "drop-shadow" => input.parse_nested_block(parse_drop_shadow_args),
        _ => Err(input.new_custom_error(())),
    }
}

/// Parse `filter: none | <filter-value-list>` (CSS Filter Effects Level 1 §5;
/// `<filter-value-list> = [ <filter-function> | <url> ]+` —
/// **whitespace**-separated, not comma-separated, one or more values).
/// Like [`parse_transform`], loop over elements. First try
/// [`parse_filter_function`] (a named function); if it fails, try
/// [`parse_url_value`] (the `<url>` alternative) as a two-way fallback.
/// As with the gradient-then-URL ordering of [`parse_background_image`],
/// these token shapes are mutually exclusive, so order does not affect the result.
pub(super) fn parse_filter(input: &mut Parser<'_, '_>) -> Option<Vec<FilterFunction>> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(Vec::new());
    }
    let mut functions = Vec::new();
    loop {
        if let Ok(function) = input.try_parse(parse_filter_function) {
            functions.push(function);
            continue;
        }
        if let Ok(url) = input.try_parse(|i| -> Result<String, ParseError<'_, ()>> {
            parse_url_value(i).ok_or_else(|| i.new_custom_error(()))
        }) {
            functions.push(FilterFunction::Url(url));
            continue;
        }
        break;
    }
    (!functions.is_empty()).then_some(functions)
}

/// Dispatch the six `<gradient>` function names (CSS Images 4 §3; see [`Gradient`]).
/// Read the function-token name, then call the corresponding
/// `parse_*_gradient_body` via `parse_nested_block`, following the same shape
/// as other function dispatch (such as `color-mix()` in [`parse_color_float`]).
fn parse_gradient<'i>(input: &mut Parser<'i, '_>) -> Result<Gradient, ParseError<'i, ()>> {
    let name = match input.next()?.clone() {
        Token::Function(name) => name,
        token => return Err(input.new_unexpected_token_error(token)),
    };
    match name.as_ref().to_ascii_lowercase().as_str() {
        "linear-gradient" => input
            .parse_nested_block(|i| parse_linear_gradient_body(i, false))
            .map(Gradient::Linear),
        "repeating-linear-gradient" => input
            .parse_nested_block(|i| parse_linear_gradient_body(i, true))
            .map(Gradient::Linear),
        "radial-gradient" => input
            .parse_nested_block(|i| parse_radial_gradient_body(i, false))
            .map(Gradient::Radial),
        "repeating-radial-gradient" => input
            .parse_nested_block(|i| parse_radial_gradient_body(i, true))
            .map(Gradient::Radial),
        "conic-gradient" => input
            .parse_nested_block(|i| parse_conic_gradient_body(i, false))
            .map(Gradient::Conic),
        "repeating-conic-gradient" => input
            .parse_nested_block(|i| parse_conic_gradient_body(i, true))
            .map(Gradient::Conic),
        _ => Err(input.new_custom_error(())),
    }
}

/// `<angle> | <zero>` (CSS Values 4 §7.1; see [`Angle`]). Allow only a bare
/// `0` without units. CSS Values 4 §7.1 says `<angle>` itself does not generally
/// allow a unitless zero: "For legacy reasons, some uses of
/// `<angle>` allow a bare 0 to mean 0deg. This is not true in general".
/// Our callers (`linear-gradient()` with `[ <angle> | <zero> | to
/// <side-or-corner> ]`, and `conic-gradient()` with `from [ <angle> | <zero> ]`;
/// both in CSS Images 4 §3) have an explicit `<zero>` grammar alternative,
/// so they are among those legacy uses and accept a bare `0`.
/// This is an angle-specific reason, distinct from the unitless-zero rule for
/// `<length-percentage>` (see "Unitless zero" in [`parse_length_value`]). For
/// unit conversion (grad/rad/turn → deg), saturate overflow just as in its
/// "Percentage overflow" section: only `is_infinite()` values keep their sign
/// and move to `f32::MAX`; do not convert `NaN`. Read tokens via
/// `next_numeric_stable` (see "Numeric-token NaN stabilization" in the module docs):
/// a normal parse no longer passes `NaN` from `0e999deg` into `value`.
fn parse_angle<'i>(input: &mut Parser<'i, '_>) -> Result<Angle, ParseError<'i, ()>> {
    match next_numeric_stable(input)? {
        Token::Number { value, .. } => {
            if value == 0.0 {
                Ok(Angle(0.0))
            } else {
                Err(input.new_custom_error(()))
            }
        }
        Token::Dimension { value, unit, .. } => {
            let degrees = match unit.to_ascii_lowercase().as_str() {
                "deg" => value,
                "grad" => value * 0.9,
                "rad" => value.to_degrees(),
                "turn" => value * 360.0,
                _ => return Err(input.new_custom_error(())),
            };
            let degrees = if degrees.is_infinite() {
                f32::MAX.copysign(degrees)
            } else {
                degrees
            };
            Ok(Angle(degrees))
        }
        token => Err(input.new_unexpected_token_error(token)),
    }
}

/// `<angle-percentage>` (see [`AnglePercentage`]) for the angular color stop
/// position of `conic-gradient()`.
fn parse_angle_percentage<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<AnglePercentage, ParseError<'i, ()>> {
    if let Ok(percent) = input.try_parse(parse_percent_number) {
        return Ok(AnglePercentage::Percent(percent));
    }
    parse_angle(input).map(AnglePercentage::Angle)
}

/// Convert a `<percentage>` token to an authored number (`50%` → `50.0`).
/// Follow the overflow-saturation policy of the `Token::Percentage` arm of
/// [`parse_length_value`]: only when reversing `unit_value * 100.0` produces
/// `±Inf`, preserve its sign and saturate at `f32::MAX` (see that function's
/// "Percentage overflow" section).
fn parse_percent_number<'i>(input: &mut Parser<'i, '_>) -> Result<f32, ParseError<'i, ()>> {
    let unit_value = expect_percentage_stable(input)?;
    let percent = unit_value * 100.0;
    Ok(if percent.is_infinite() {
        f32::MAX.copysign(percent)
    } else {
        percent
    })
}

/// `in <color-space> <hue-interpolation-method>?` ([`GradientColorInterpolation`]
/// docs). Delegate parsing and validation to [`parse_color_interpolation_method`],
/// shared with `parse_color_mix_function`, then assemble the returned tuple
/// into [`GradientColorInterpolation`].
pub(crate) fn parse_gradient_color_interpolation<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<GradientColorInterpolation, ParseError<'i, ()>> {
    let (color_space, hue_method) = parse_color_interpolation_method(input, false)?;
    Ok(GradientColorInterpolation {
        color_space,
        hue_method,
    })
}

/// Spec-mandated default for [`GradientColorInterpolation`] when `in ...` is
/// omitted: CSS Images 4 §3.5.2, "Coloring the Gradient Line", says "the color
/// space used for gradient interpolation is the default interpolation
/// color space, Oklab".
fn default_gradient_color_interpolation() -> GradientColorInterpolation {
    GradientColorInterpolation {
        color_space: MixColorSpace::Oklab,
        hue_method: HueInterpolationMethod::Shorter,
    }
}

/// `at <position>` (see [`CssPosition`]), shared by `radial-gradient()` and
/// `conic-gradient()`.
fn parse_at_position<'i>(input: &mut Parser<'i, '_>) -> Result<CssPosition, ParseError<'i, ()>> {
    input.expect_ident_matching("at")?;
    parse_bg_position(input).ok_or_else(|| input.new_custom_error(()))
}

/// Spec-mandated default for `<position>`: `center` on both axes, by applying
/// [`css_position_center`] to each axis.
fn default_center_position() -> CssPosition {
    CssPosition {
        horizontal: css_position_center(),
        vertical: css_position_center(),
    }
}

/// Color of a gradient stop with `<color>` (see [`GradientStopColor`]):
/// handle the `currentcolor` keyword before delegating to [`parse_color`],
/// as in [`parse_text_shadow_color`].
fn parse_gradient_stop_color(input: &mut Parser<'_, '_>) -> Option<GradientStopColor> {
    if input
        .try_parse(|i| i.expect_ident_matching("currentcolor"))
        .is_ok()
    {
        return Some(GradientStopColor::CurrentColor);
    }
    parse_color(input).map(GradientStopColor::Resolved)
}

/// `<linear-color-stop> = <color> <length-percentage>?`
/// (see [`GradientColorStop`]).
fn parse_gradient_color_stop<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<GradientColorStop, ParseError<'i, ()>> {
    let color = parse_gradient_stop_color(input).ok_or_else(|| input.new_custom_error(()))?;
    let position = input.try_parse(parse_length_percentage_res).ok();
    Ok(GradientColorStop { color, position })
}

/// `<color-stop-list>` (see the scope carving in [`GradientColorStop`]:
/// no hints, at least two stops). Shared by `linear-gradient()` and `radial-gradient()`.
fn parse_gradient_color_stop_list<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<Vec<GradientColorStop>, ParseError<'i, ()>> {
    let stops = input.parse_comma_separated(parse_gradient_color_stop)?;
    if stops.len() < 2 {
        return Err(input.new_custom_error(()));
    }
    Ok(stops)
}

/// One-value version of `<angular-color-stop> = <color> <color-stop-angle>?`
/// (see [`AngularColorStop`]).
fn parse_angular_color_stop<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<AngularColorStop, ParseError<'i, ()>> {
    let color = parse_gradient_stop_color(input).ok_or_else(|| input.new_custom_error(()))?;
    let position = input.try_parse(parse_angle_percentage).ok();
    Ok(AngularColorStop { color, position })
}

/// `<angular-color-stop-list>`: conic version of [`parse_gradient_color_stop_list`]
/// (with the same scope carving and the same two-stop minimum).
fn parse_angular_color_stop_list<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<Vec<AngularColorStop>, ParseError<'i, ()>> {
    let stops = input.parse_comma_separated(parse_angular_color_stop)?;
    if stops.len() < 2 {
        return Err(input.new_custom_error(()));
    }
    Ok(stops)
}

/// Spec-mandated default for [`LinearGradientDirection`]: `to bottom` (CSS
/// Images 4 §3.1 "If the first argument to the function is omitted, it
/// defaults to to bottom").
fn default_linear_gradient_direction() -> LinearGradientDirection {
    LinearGradientDirection::Side(SideOrCorner {
        horizontal: None,
        vertical: Some(VerticalSide::Bottom),
    })
}

/// Either `<angle>` or the part of `to <side-or-corner>` after `to`
/// (the two alternatives in the [`LinearGradientDirection`] docs).
fn parse_linear_gradient_direction<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<LinearGradientDirection, ParseError<'i, ()>> {
    if input.try_parse(|i| i.expect_ident_matching("to")).is_ok() {
        return parse_side_or_corner(input).map(LinearGradientDirection::Side);
    }
    parse_angle(input).map(LinearGradientDirection::Angle)
}

/// `<side-or-corner> = [left | right] || [top | bottom]`
/// (see [`SideOrCorner`]); an any-order loop as in [`parse_outline`](super::box_model::parse_outline).
fn parse_side_or_corner<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<SideOrCorner, ParseError<'i, ()>> {
    let mut horizontal: Option<HorizontalSide> = None;
    let mut vertical: Option<VerticalSide> = None;
    loop {
        if horizontal.is_none()
            && let Ok(value) = input.try_parse(parse_horizontal_side)
        {
            horizontal = Some(value);
            continue;
        }
        if vertical.is_none()
            && let Ok(value) = input.try_parse(parse_vertical_side)
        {
            vertical = Some(value);
            continue;
        }
        break;
    }
    if horizontal.is_none() && vertical.is_none() {
        return Err(input.new_custom_error(()));
    }
    Ok(SideOrCorner {
        horizontal,
        vertical,
    })
}

fn parse_horizontal_side<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<HorizontalSide, ParseError<'i, ()>> {
    let ident = input.expect_ident()?.clone();
    match ident.as_ref().to_ascii_lowercase().as_str() {
        "left" => Ok(HorizontalSide::Left),
        "right" => Ok(HorizontalSide::Right),
        _ => Err(input.new_custom_error(())),
    }
}

fn parse_vertical_side<'i>(input: &mut Parser<'i, '_>) -> Result<VerticalSide, ParseError<'i, ()>> {
    let ident = input.expect_ident()?.clone();
    match ident.as_ref().to_ascii_lowercase().as_str() {
        "top" => Ok(VerticalSide::Top),
        "bottom" => Ok(VerticalSide::Bottom),
        _ => Err(input.new_custom_error(())),
    }
}

/// Nested-block body of `linear-gradient()`/`repeating-linear-gradient()`
/// (see the [`LinearGradient`] grammar). `direction` and `interpolation` are
/// optional and may appear in either order (`||`). Only when either is present
/// must a comma precede the following `<color-stop-list>`: the trailing `,` is
/// **outside** the `[...]?` group in the grammar, so an absent group has no comma.
/// That is why `linear-gradient(red, blue)` has neither direction nor interpolation
/// and needs no comma before its first stop.
fn parse_linear_gradient_body<'i>(
    input: &mut Parser<'i, '_>,
    repeating: bool,
) -> Result<LinearGradient, ParseError<'i, ()>> {
    let mut direction: Option<LinearGradientDirection> = None;
    let mut interpolation: Option<GradientColorInterpolation> = None;
    loop {
        if direction.is_none()
            && let Ok(value) = input.try_parse(parse_linear_gradient_direction)
        {
            direction = Some(value);
            continue;
        }
        if interpolation.is_none()
            && let Ok(value) = input.try_parse(parse_gradient_color_interpolation)
        {
            interpolation = Some(value);
            continue;
        }
        break;
    }
    if direction.is_some() || interpolation.is_some() {
        input.expect_comma()?;
    }
    let stops = parse_gradient_color_stop_list(input)?;
    Ok(LinearGradient {
        repeating,
        direction: direction.unwrap_or_else(default_linear_gradient_direction),
        interpolation: interpolation.unwrap_or_else(default_gradient_color_interpolation),
        stops: Arc::new(stops),
    })
}

fn parse_radial_shape_keyword<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<RadialShape, ParseError<'i, ()>> {
    let ident = input.expect_ident()?.clone();
    match ident.as_ref().to_ascii_lowercase().as_str() {
        "circle" => Ok(RadialShape::Circle),
        "ellipse" => Ok(RadialShape::Ellipse),
        _ => Err(input.new_custom_error(())),
    }
}

fn parse_radial_extent<'i>(input: &mut Parser<'i, '_>) -> Result<RadialExtent, ParseError<'i, ()>> {
    let ident = input.expect_ident()?.clone();
    match ident.as_ref().to_ascii_lowercase().as_str() {
        "closest-side" => Ok(RadialExtent::ClosestSide),
        "closest-corner" => Ok(RadialExtent::ClosestCorner),
        "farthest-side" => Ok(RadialExtent::FarthestSide),
        "farthest-corner" => Ok(RadialExtent::FarthestCorner),
        _ => Err(input.new_custom_error(())),
    }
}

/// Authored form of [`RadialSize`]: an intermediate representation before
/// [`resolve_radial_shape_and_size`] combines it with the possibly omitted `shape`
/// and produces the final `(RadialShape, RadialSize)`. `Circle`/`Ellipse` have
/// the same meanings as in [`RadialSize`], but compatibility with `shape` is not yet checked.
enum RadialSizeAuthored {
    Extent(RadialExtent),
    Circle(Length),
    Ellipse(Length, Length),
}

/// Return-value shape of [`parse_radial_shape_size_position_group`]: an alias
/// avoiding clippy's `type_complexity` lint, not a new semantic type.
type RadialShapeSizePositionGroup = (
    Option<RadialShape>,
    Option<RadialSizeAuthored>,
    Option<CssPosition>,
);

/// `<radial-size>` (CSS Images 3 §3.2.1 baseline grammar; see
/// [`RadialSize`]). Try two `<length-percentage [0,∞]>` values (ellipse form) first.
/// With just one token, parsing the second fails and rewinds the whole attempt,
/// allowing a natural fallback to one `<length [0,∞]>` (circle form).
/// This is the same "safe rewind" pattern as the alternative-order docs for
/// [`parse_bg_position`].
fn parse_radial_size<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<RadialSizeAuthored, ParseError<'i, ()>> {
    if let Ok(extent) = input.try_parse(parse_radial_extent) {
        return Ok(RadialSizeAuthored::Extent(extent));
    }
    if let Ok((a, b)) = input.try_parse(parse_two_non_negative_length_percentages) {
        return Ok(RadialSizeAuthored::Ellipse(a, b));
    }
    let length = parse_non_negative_length(input).ok_or_else(|| input.new_custom_error(()))?;
    Ok(RadialSizeAuthored::Circle(length))
}

fn parse_two_non_negative_length_percentages<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<(Length, Length), ParseError<'i, ()>> {
    let a = parse_non_negative_length_percentage_res(input)?;
    let b = parse_non_negative_length_percentage_res(input)?;
    Ok((a, b))
}

/// Resolve omitted `shape`/`size` under CSS Images 3 §3.2.1; see the
/// paired docs for [`RadialShape`]/[`RadialSize`]. Return `None` for incompatible
/// combinations (`circle` with ellipse-only size, `ellipse` with circle-only size).
fn resolve_radial_shape_and_size(
    shape: Option<RadialShape>,
    size: Option<RadialSizeAuthored>,
) -> Option<(RadialShape, RadialSize)> {
    match (shape, size) {
        (Some(RadialShape::Circle), Some(RadialSizeAuthored::Ellipse(_, _)))
        | (Some(RadialShape::Ellipse), Some(RadialSizeAuthored::Circle(_))) => None,
        (Some(RadialShape::Circle), Some(RadialSizeAuthored::Circle(l))) => {
            Some((RadialShape::Circle, RadialSize::Circle(l)))
        }
        (Some(RadialShape::Ellipse), Some(RadialSizeAuthored::Ellipse(a, b))) => {
            Some((RadialShape::Ellipse, RadialSize::Ellipse(a, b)))
        }
        (Some(shape), Some(RadialSizeAuthored::Extent(extent))) => {
            Some((shape, RadialSize::Extent(extent)))
        }
        (Some(shape), None) => Some((shape, RadialSize::Extent(RadialExtent::FarthestCorner))),
        (None, Some(RadialSizeAuthored::Circle(l))) => {
            Some((RadialShape::Circle, RadialSize::Circle(l)))
        }
        (None, Some(RadialSizeAuthored::Ellipse(a, b))) => {
            Some((RadialShape::Ellipse, RadialSize::Ellipse(a, b)))
        }
        (None, Some(RadialSizeAuthored::Extent(extent))) => {
            Some((RadialShape::Ellipse, RadialSize::Extent(extent)))
        }
        (None, None) => Some((
            RadialShape::Ellipse,
            RadialSize::Extent(RadialExtent::FarthestCorner),
        )),
    }
}

/// `[ <radial-shape> || <radial-size> ]? [ at <position> ]?`: the spec's
/// juxtaposition (space-separated) means `shape` and `size` may appear in
/// either order (the inner `||`), but `at <position>` can only appear
/// **after** their entire group (fixed order). If all of `shape`, `size`, and
/// `position` are absent, return `Err`. The outer `||` loop in
/// [`parse_radial_gradient_body`] treats this group as one item to read in
/// either order with `<color-interpolation-method>`, and must distinguish an
/// empty match from no match. If `try_parse` always succeeds on an empty match,
/// the outer loop cannot retry this group on its second iteration or later.
fn parse_radial_shape_size_position_group<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<RadialShapeSizePositionGroup, ParseError<'i, ()>> {
    let mut shape: Option<RadialShape> = None;
    let mut size: Option<RadialSizeAuthored> = None;
    loop {
        if shape.is_none()
            && let Ok(value) = input.try_parse(parse_radial_shape_keyword)
        {
            shape = Some(value);
            continue;
        }
        if size.is_none()
            && let Ok(value) = input.try_parse(parse_radial_size)
        {
            size = Some(value);
            continue;
        }
        break;
    }
    let position = input.try_parse(parse_at_position).ok();
    if shape.is_none() && size.is_none() && position.is_none() {
        return Err(input.new_custom_error(()));
    }
    Ok((shape, size, position))
}

/// Nested-block body of `radial-gradient()`/`repeating-radial-gradient()`
/// (see the [`RadialGradient`] grammar): `[ [ [ <radial-shape>
/// || <radial-size> ]? [ at <position> ]? ] || <color-interpolation-method>
/// ]?`. Read the outer `||` (between the shape/size/position group
/// and `<color-interpolation-method>`) in an any-order loop.
/// The ordering constraint inside the group (see [`parse_radial_shape_size_position_group`])
/// requires one slot, unlike the two-slot [`parse_linear_gradient_body`].
/// If `shape`, `size`, and `position` were independent outer-loop slots,
/// the parser would accept spec-invalid orderings such as `at center circle`,
/// where position precedes shape/size.
fn parse_radial_gradient_body<'i>(
    input: &mut Parser<'i, '_>,
    repeating: bool,
) -> Result<RadialGradient, ParseError<'i, ()>> {
    let mut group: Option<RadialShapeSizePositionGroup> = None;
    let mut interpolation: Option<GradientColorInterpolation> = None;
    loop {
        if group.is_none()
            && let Ok(value) = input.try_parse(parse_radial_shape_size_position_group)
        {
            group = Some(value);
            continue;
        }
        if interpolation.is_none()
            && let Ok(value) = input.try_parse(parse_gradient_color_interpolation)
        {
            interpolation = Some(value);
            continue;
        }
        break;
    }
    let any = group.is_some() || interpolation.is_some();
    if any {
        input.expect_comma()?;
    }
    let stops = parse_gradient_color_stop_list(input)?;
    let (shape, size, position) = group.unwrap_or((None, None, None));
    let (resolved_shape, resolved_size) =
        resolve_radial_shape_and_size(shape, size).ok_or_else(|| input.new_custom_error(()))?;
    Ok(RadialGradient {
        repeating,
        shape: resolved_shape,
        size: resolved_size,
        position: position.unwrap_or_else(default_center_position),
        interpolation: interpolation.unwrap_or_else(default_gradient_color_interpolation),
        stops: Arc::new(stops),
    })
}

fn parse_conic_from_angle<'i>(input: &mut Parser<'i, '_>) -> Result<Angle, ParseError<'i, ()>> {
    input.expect_ident_matching("from")?;
    parse_angle(input)
}

/// `[ from [ <angle> | <zero> ] ]? [ at <position> ]?` —
/// Conic counterpart of [`parse_radial_shape_size_position_group`]. Spec
/// grammar juxtaposition requires `from <angle>` before `at <position>`.
/// Unlike the shape/size case, the first element is just one component, so
/// there is no inner `||`. The `Err` fallback likewise distinguishes an empty match.
fn parse_conic_from_position_group<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<(Option<Angle>, Option<CssPosition>), ParseError<'i, ()>> {
    let angle = input.try_parse(parse_conic_from_angle).ok();
    let position = input.try_parse(parse_at_position).ok();
    if angle.is_none() && position.is_none() {
        return Err(input.new_custom_error(()));
    }
    Ok((angle, position))
}

/// Nested-block body of `conic-gradient()`/`repeating-conic-gradient()`
/// (see the [`ConicGradient`] grammar). Read the outer `||` (between the `from`/`at`
/// group and `<color-interpolation-method>`) in an any-order loop,
/// just as [`parse_radial_gradient_body`] reads its group as one slot.
/// This rejects spec-invalid reversed orderings such as `at center from 45deg`
/// (see [`parse_conic_from_position_group`]).
fn parse_conic_gradient_body<'i>(
    input: &mut Parser<'i, '_>,
    repeating: bool,
) -> Result<ConicGradient, ParseError<'i, ()>> {
    let mut group: Option<(Option<Angle>, Option<CssPosition>)> = None;
    let mut interpolation: Option<GradientColorInterpolation> = None;
    loop {
        if group.is_none()
            && let Ok(value) = input.try_parse(parse_conic_from_position_group)
        {
            group = Some(value);
            continue;
        }
        if interpolation.is_none()
            && let Ok(value) = input.try_parse(parse_gradient_color_interpolation)
        {
            interpolation = Some(value);
            continue;
        }
        break;
    }
    let any = group.is_some() || interpolation.is_some();
    if any {
        input.expect_comma()?;
    }
    let stops = parse_angular_color_stop_list(input)?;
    let (angle, position) = group.unwrap_or((None, None));
    Ok(ConicGradient {
        repeating,
        angle: angle.unwrap_or(Angle(0.0)),
        position: position.unwrap_or_else(default_center_position),
        interpolation: interpolation.unwrap_or_else(default_gradient_color_interpolation),
        stops: Arc::new(stops),
    })
}

/// Parse one `<repeat-style>` keyword (see the [`BackgroundRepeatKeyword`]
/// grammar). Do not handle `repeat-x`/`repeat-y` here:
/// [`parse_background_repeat`] handles them as separate top-level alternatives.
fn parse_background_repeat_keyword(input: &mut Parser<'_, '_>) -> Option<BackgroundRepeatKeyword> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "repeat" => Some(BackgroundRepeatKeyword::Repeat),
        "space" => Some(BackgroundRepeatKeyword::Space),
        "round" => Some(BackgroundRepeatKeyword::Round),
        "no-repeat" => Some(BackgroundRepeatKeyword::NoRepeat),
        _ => None,
    }
}

fn parse_background_repeat_keyword_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<BackgroundRepeatKeyword, ParseError<'i, ()>> {
    parse_background_repeat_keyword(input).ok_or_else(|| input.new_custom_error(()))
}

/// Parse `background-repeat: <repeat-style>` (see the [`BackgroundRepeat`]
/// grammar: `repeat-x | repeat-y | [repeat | space | round |
/// no-repeat]{1,2}`).
///
/// Try `repeat-x`/`repeat-y` first as shorthand for their two-keyword forms
/// (the spec's computed value: `repeat-x` = `repeat no-repeat`, `repeat-y` =
/// `no-repeat repeat`). When only one keyword is specified, apply it to both axes
/// (for example, `repeat` = `repeat repeat`).
pub(crate) fn parse_background_repeat(input: &mut Parser<'_, '_>) -> Option<BackgroundRepeat> {
    if input
        .try_parse(|i| i.expect_ident_matching("repeat-x"))
        .is_ok()
    {
        return Some(BackgroundRepeat {
            x: BackgroundRepeatKeyword::Repeat,
            y: BackgroundRepeatKeyword::NoRepeat,
        });
    }
    if input
        .try_parse(|i| i.expect_ident_matching("repeat-y"))
        .is_ok()
    {
        return Some(BackgroundRepeat {
            x: BackgroundRepeatKeyword::NoRepeat,
            y: BackgroundRepeatKeyword::Repeat,
        });
    }
    let first = parse_background_repeat_keyword(input)?;
    let second = input.try_parse(parse_background_repeat_keyword_res).ok();
    Some(match second {
        None => BackgroundRepeat { x: first, y: first },
        Some(second) => BackgroundRepeat {
            x: first,
            y: second,
        },
    })
}

/// Parse `background-attachment: <attachment>`
/// (see the [`BackgroundAttachment`] grammar: `scroll | fixed | local`).
pub(super) fn parse_background_attachment(
    input: &mut Parser<'_, '_>,
) -> Option<BackgroundAttachment> {
    BackgroundAttachment::from_css_ident(input.expect_ident().ok()?)
}

/// Parse `object-fit: <fit>` (see the [`ObjectFit`] grammar:
/// `fill | contain | cover | none | scale-down`).
pub(super) fn parse_object_fit(input: &mut Parser<'_, '_>) -> Option<ObjectFit> {
    ObjectFit::from_css_ident(input.expect_ident().ok()?)
}

/// Parse `mix-blend-mode: <blend-mode>` (see the [`MixBlendMode`]
/// grammar: 16 keywords).
pub(super) fn parse_mix_blend_mode(input: &mut Parser<'_, '_>) -> Option<MixBlendMode> {
    MixBlendMode::from_css_ident(input.expect_ident().ok()?)
}

/// Parse `<visual-box>` (see the [`VisualBox`] grammar:
/// `border-box | padding-box | content-box`). Shared by `background-clip` and
/// `background-origin`; differences in their initial values are handled in
/// `specified.rs`/`computed.rs`, not by the caller.
pub(super) fn parse_visual_box(input: &mut Parser<'_, '_>) -> Option<VisualBox> {
    VisualBox::from_css_ident(input.expect_ident().ok()?)
}

/// One axis of `<bg-size>`: `<length-percentage [0,∞]> | auto`.
/// Enforce non-negativity as in [`parse_width`](super::box_model::parse_width).
fn parse_background_size_axis(input: &mut Parser<'_, '_>) -> Option<LengthOrAuto> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(LengthOrAuto::Auto);
    }
    let length = parse_length_value(input, true)?;
    (length.payload() >= 0.0).then_some(LengthOrAuto::Length(length))
}

fn parse_background_size_axis_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<LengthOrAuto, ParseError<'i, ()>> {
    parse_background_size_axis(input).ok_or_else(|| input.new_custom_error(()))
}

/// Parse `background-size: <bg-size>` (see the [`BackgroundSize`]
/// grammar: `[ <length-percentage [0,∞]> | auto ]{1,2} | cover |
/// contain`).
///
/// Try `cover`/`contain` before the axis values: each keyword takes the whole value.
/// When the second axis is omitted, it defaults to **`auto`** (the spec says, "If only
/// one value is given the second is assumed to be auto."). This differs from
/// [`parse_border_radius`](super::box_model::parse_border_radius) and related fill rules, which copy the first value;
/// do not reuse those rules here.
pub(crate) fn parse_background_size(input: &mut Parser<'_, '_>) -> Option<BackgroundSize> {
    if input
        .try_parse(|i| i.expect_ident_matching("cover"))
        .is_ok()
    {
        return Some(BackgroundSize::Cover);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("contain"))
        .is_ok()
    {
        return Some(BackgroundSize::Contain);
    }
    let width = parse_background_size_axis(input)?;
    let height = input
        .try_parse(parse_background_size_axis_res)
        .unwrap_or(LengthOrAuto::Auto);
    Some(BackgroundSize::Explicit { width, height })
}

/// `<bg-position> [ / <bg-size> ]?` — the `background` shorthand's
/// position+size unit ([`BackgroundShorthand`] doc's grammar section).
/// Unlike the shorthand's other `||` components, position and size are
/// **not** independently orderable — a size may only follow a position,
/// separated by a literal `/` (same fixed-pair shape as
/// [`parse_grid_line_shorthand`](super::layout::parse_grid_line_shorthand)'s `<grid-line> [ / <grid-line> ]?`).
///
/// [`parse_bg_position`]'s 3 internal alternatives (`branch3`/`branch2`/
/// `branch1`, see that function's doc) each consume exactly their own
/// production and stop — they never overrun into tokens that belong to a
/// later shorthand component. That is what lets this function's caller
/// ([`parse_background_shorthand`]) safely hand any leftover tokens back to
/// its own any-order loop instead of treating them as part of the position.
pub(crate) fn parse_background_position_and_size(
    input: &mut Parser<'_, '_>,
) -> Option<(CssPosition, Option<BackgroundSize>)> {
    let position = parse_bg_position(input)?;
    let size = input
        .try_parse(|i| -> Result<BackgroundSize, ParseError<'_, ()>> {
            i.expect_delim('/')?;
            parse_background_size(i).ok_or_else(|| i.new_custom_error(()))
        })
        .ok();
    Some((position, size))
}

/// Parse the `background` shorthand (see the grammar section of the
/// [`BackgroundShorthand`] docs).
///
/// # `||` (any-order) grammar semantics
///
/// Use the same loop as [`parse_border_shorthand`](super::box_model::parse_border_shorthand): try each
/// unfilled slot with `try_parse`; on success, fill it and restart the loop.
/// Stop when every slot is full or a token matches none. The caller's
/// `expect_exhausted` in the `rule.rs` declaration parser drops leftover tokens.
/// The "Single-layer support only" section of [`BackgroundShorthand`] explains
/// how this rejects multiple comma-separated layers.
///
/// Of the six slots, only `visual_boxes` can match twice: the grammar has
/// two independent `||` alternatives for `<visual-box>`
/// (see [`BackgroundShorthand`]). Treat `position_and_size` as one slot via
/// [`parse_background_position_and_size`]: the spec's
/// `<bg-position> [ / <bg-size> ]?` is a single `||` alternative.
///
/// The token sets of all slots are disjoint: image uses `none`/`url()`/gradient
/// functions, position uses directional keywords or lengths, repeat-style,
/// attachment, and visual-box each use their own keywords, and color uses named
/// colors, hex, or functions. Thus slot trial order cannot affect the result;
/// see the same point in the [`parse_border_shorthand`](super::box_model::parse_border_shorthand) docs.
///
/// # At least 1 component required
///
/// spec CSS Values 4 §2.2 `||` semantics: "one or more of them must occur,
/// in any order." Zero components (an empty `background:` or only unknown keywords)
/// return `None`, dropping the declaration (as in [`parse_border_shorthand`](super::box_model::parse_border_shorthand)).
///
/// # Initial value fill (omitted components)
///
/// See the "Initial value fill" section of [`BackgroundShorthand`]. How the
/// 0/1/2 occurrences of `visual_boxes` set origin/clip follows spec §2.10:
/// ("If one `<visual-box>` value is present then it sets both
/// background-origin and background-clip to that value. If two values are
/// present, then the first sets background-origin and the second
/// background-clip.") With zero occurrences, use each longhand's spec initial value:
/// origin is padding-box and clip is border-box. This differs from one occurrence,
/// which sets both to that value; only the zero case is asymmetric.
pub(crate) fn parse_background_shorthand(
    input: &mut Parser<'_, '_>,
) -> Option<BackgroundShorthand> {
    let mut image: Option<BackgroundImage> = None;
    let mut position_and_size: Option<(CssPosition, Option<BackgroundSize>)> = None;
    let mut repeat: Option<BackgroundRepeat> = None;
    let mut attachment: Option<BackgroundAttachment> = None;
    let mut visual_boxes: Vec<VisualBox> = Vec::new();
    let mut color: Option<CssColor> = None;

    loop {
        if image.is_none()
            && let Ok(v) = input.try_parse(|i| -> Result<BackgroundImage, ParseError<'_, ()>> {
                parse_background_image(i).ok_or_else(|| i.new_custom_error(()))
            })
        {
            image = Some(v);
            continue;
        }
        if position_and_size.is_none()
            && let Ok(v) = input.try_parse(
                |i| -> Result<(CssPosition, Option<BackgroundSize>), ParseError<'_, ()>> {
                    parse_background_position_and_size(i).ok_or_else(|| i.new_custom_error(()))
                },
            )
        {
            position_and_size = Some(v);
            continue;
        }
        if repeat.is_none()
            && let Ok(v) = input.try_parse(|i| -> Result<BackgroundRepeat, ParseError<'_, ()>> {
                parse_background_repeat(i).ok_or_else(|| i.new_custom_error(()))
            })
        {
            repeat = Some(v);
            continue;
        }
        if attachment.is_none()
            && let Ok(v) =
                input.try_parse(|i| -> Result<BackgroundAttachment, ParseError<'_, ()>> {
                    parse_background_attachment(i).ok_or_else(|| i.new_custom_error(()))
                })
        {
            attachment = Some(v);
            continue;
        }
        if visual_boxes.len() < 2
            && let Ok(v) = input.try_parse(|i| -> Result<VisualBox, ParseError<'_, ()>> {
                parse_visual_box(i).ok_or_else(|| i.new_custom_error(()))
            })
        {
            visual_boxes.push(v);
            continue;
        }
        if color.is_none()
            && let Ok(v) = input.try_parse(|i| -> Result<CssColor, ParseError<'_, ()>> {
                parse_color(i).ok_or_else(|| i.new_custom_error(()))
            })
        {
            color = Some(v);
            continue;
        }

        // No unfilled slot matched: the token is a second value for a filled slot
        // (a third for `visual_boxes`), a comma (multiple layers), or an unknown token.
        // Break the loop and let the caller's `expect_exhausted` drop the leftover
        // (as in `parse_border_shorthand`).
        break;
    }

    if image.is_none()
        && position_and_size.is_none()
        && repeat.is_none()
        && attachment.is_none()
        && visual_boxes.is_empty()
        && color.is_none()
    {
        return None;
    }

    let (origin, clip) = match visual_boxes.as_slice() {
        [] => (VisualBox::PaddingBox, VisualBox::BorderBox),
        [one] => (*one, *one),
        [first, second] => (*first, *second),
        // cov:ignore: the loop above only pushes while
        // `visual_boxes.len() < 2`, so `visual_boxes` can never hold more
        // than 2 elements by the time this match runs — there is no input
        // that reaches this arm, and constructing one would require
        // bypassing the loop guard entirely.
        _ => unreachable!("visual_boxes never grows past 2 (loop guard above)"),
    };

    let (position, size) = match position_and_size {
        Some((position, size)) => (
            position,
            size.unwrap_or(BackgroundSize::Explicit {
                width: LengthOrAuto::Auto,
                height: LengthOrAuto::Auto,
            }),
        ),
        None => (
            CssPosition {
                horizontal: CssPositionOffset::Start(Length::Percent(0.0)),
                vertical: CssPositionOffset::Start(Length::Percent(0.0)),
            },
            BackgroundSize::Explicit {
                width: LengthOrAuto::Auto,
                height: LengthOrAuto::Auto,
            },
        ),
    };

    Some(BackgroundShorthand {
        color: color.unwrap_or(CssColor::TRANSPARENT),
        image: image.unwrap_or(BackgroundImage::None),
        repeat: repeat.unwrap_or(BackgroundRepeat {
            x: BackgroundRepeatKeyword::Repeat,
            y: BackgroundRepeatKeyword::Repeat,
        }),
        attachment: attachment.unwrap_or(BackgroundAttachment::Scroll),
        position,
        size,
        clip,
        origin,
    })
}
