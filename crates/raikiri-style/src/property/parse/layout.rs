//! Layout property parsers: display, float/clear, flexbox, grid, alignment,
//! multi-column, fragmentation breaks, tables and lists.

use std::sync::Arc;

use cssparser::{ParseError, Parser, Token};
use smol_str::SmolStr;

use crate::property::types::*;

use super::common::*;

/// Parses `flex-direction: row | row-reverse | column | column-reverse` (CSS
/// Flexible Box Layout Module Level 1 §5.1
/// <https://www.w3.org/TR/css-flexbox-1/#flex-direction-property>).
/// Uses the same single-ident, ASCII case-insensitive approach as [`parse_display`].
pub(super) fn parse_flex_direction(input: &mut Parser<'_, '_>) -> Option<FlexDirectionValue> {
    FlexDirectionValue::from_css_ident(input.expect_ident().ok()?)
}

/// Parses `flex-wrap: nowrap | wrap | wrap-reverse` (CSS Flexible Box
/// Layout Module Level 1 §5.2
/// <https://www.w3.org/TR/css-flexbox-1/#flex-wrap-property>).
pub(super) fn parse_flex_wrap(input: &mut Parser<'_, '_>) -> Option<FlexWrapValue> {
    FlexWrapValue::from_css_ident(input.expect_ident().ok()?)
}

/// Parses `<number [0,∞]>` for both `flex-grow` and `flex-shrink`.
///
/// # Enforce both non-negativity and finiteness while parsing
///
/// Like the `<number>` branch of [`parse_line_height`](super::text::parse_line_height), this helper checks
/// that the number is non-negative (`[0,∞]`). It also requires the number to
/// be finite (see the [`PropertyValue::FlexGrow`] docs). This differs from
/// other length helpers in this crate, such as `parse_length_value`.
/// Lengths such as `padding` and `width` always pass through the
/// **sink-boundary guards** `sanitize_taffy`/`sanitize_taffy_layout` in
/// raikiri-dom before reaching taffy. This follows the crate-wide policy of
/// placing guards at sink boundaries; see the non-finite f32 guard discussion
/// in `crates/raikiri-dom/src/layout.rs`. The parser can therefore pass those
/// lengths through. In contrast, raikiri-dom's `bridge_flex` copies
/// `flex-grow`/`flex-shrink` directly into `taffy::Style::flex_grow`/`flex_shrink`
/// (both raw `f32` fields), without a conversion or sink guard. This parse-time
/// check is the only place to reject `+Inf` along that path.
/// The f64→f32 conversion performed by the cssparser tokenizer for
/// `<number [0,∞]>` can produce `+Inf` from a huge literal such as
/// `flex-grow: 1e40`. The same `is_finite()` check also rejects `NaN`.
/// However, `expect_number_stable` (see "Numeric-token NaN stabilization"
/// in the module docs) already corrects `NaN` caused by zero-mantissa,
/// huge-exponent literals such as `flex-grow: 0e999` at acquisition time.
/// Thus normal parsing no longer reaches this check with `NaN`; it remains
/// essential for rejecting `+Inf`.
pub(crate) fn parse_nonneg_finite_number(input: &mut Parser<'_, '_>) -> Option<f32> {
    let n = expect_number_stable(input).ok()?;
    (n.is_finite() && n >= 0.0).then_some(n)
}

/// The `Result` form of [`parse_nonneg_finite_number`] for `try_parse`
/// closures (following the wrapper pattern of
/// [`parse_padding_side_res`](super::box_model::parse_padding_side_res)). Range and finiteness failures
/// also return `Err`, so the caller's `try_parse` automatically rewinds.
/// The captured Number token cannot fall through to another branch. This
/// addresses the same concern discussed in the "Commit a Number token before
/// choosing its result" section of the [`parse_line_height`](super::text::parse_line_height) docs by placing
/// the check inside the closure.
fn parse_nonneg_finite_number_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<f32, ParseError<'i, ()>> {
    parse_nonneg_finite_number(input).ok_or_else(|| input.new_custom_error(()))
}

/// Parses `flex-basis: content | <'width'>` (CSS Flexible Box Layout
/// Module Level 1 §7.2.3
/// <https://www.w3.org/TR/css-flexbox-1/#flex-basis-property>).
///
/// Extends the three-branch shape of [`parse_width`](super::box_model::parse_width) (`auto` → `content` →
/// `<length-percentage [0,∞]>`) with a `content` branch; see the
/// [`FlexBasisValue`] docs.
pub(crate) fn parse_flex_basis(input: &mut Parser<'_, '_>) -> Option<FlexBasisValue> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(FlexBasisValue::Auto);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("content"))
        .is_ok()
    {
        return Some(FlexBasisValue::Content);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("min-content"))
        .is_ok()
    {
        return Some(FlexBasisValue::MinContent);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("max-content"))
        .is_ok()
    {
        return Some(FlexBasisValue::MaxContent);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("fit-content"))
        .is_ok()
    {
        return Some(FlexBasisValue::FitContent);
    }
    let length = parse_length_value(input, true)?;
    // Reuse `<'width'>` with the CSS Sizing 3 §3.1.1 `[0,∞]` non-negative
    // constraint, following the same pattern as `parse_width`.
    (length.payload() >= 0.0).then_some(FlexBasisValue::Length(length))
}

/// The `Result` form of [`parse_flex_basis`], following the wrapper
/// pattern of [`parse_padding_side_res`](super::box_model::parse_padding_side_res), for the `try_parse` in
/// [`parse_flex_shorthand`].
fn parse_flex_basis_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<FlexBasisValue, ParseError<'i, ()>> {
    parse_flex_basis(input).ok_or_else(|| input.new_custom_error(()))
}

/// Parses `flex: none | [ <'flex-grow'> <'flex-shrink'>? || <'flex-basis'> ]`
/// (CSS Flexible Box Layout Module Level 1 §7.1 "The flex Shorthand"
/// <https://www.w3.org/TR/css-flexbox-1/#flex-property>).
///
/// Applies the shorthand-local defaults described under "Omitted-component
/// defaults" in the [`FlexShorthand`] docs: grow=1, shrink=1, basis=0px.
/// These differ from the longhands' own initial values.
///
/// # `none` — exclusive keyword
///
/// Spec §7.1 says, "The keyword none expands to 0 0 auto." It cannot be
/// combined with other components because it is a top-level grammar alternative.
///
/// # Component order and unitless-zero ambiguity
///
/// The grammar combines a `<flex-grow> <flex-shrink>?` group and a
/// `<flex-basis>` group with `||`: either order, at least one group. This
/// function tries both groups in at most three iterations. **Trying the
/// `<flex-grow>` group first each time** implements the spec's disambiguation
/// rule directly (verbatim):
///
/// > A unitless zero that is not already preceded by two flex factors must
/// > be interpreted as a flex factor. To avoid misinterpretation or invalid
/// > declarations, authors must specify a zero `<'flex-basis'>` component
/// > with a unit or precede it by two flex factors.
///
/// While `grow` remains unset, this function first consumes bare `0` as a
/// number (flex factor). Thus `flex: 0` sets `grow=0`, not `<'flex-basis'>`.
/// Only **after** both `grow` and `shrink` are set does bare `0` become
/// `flex-basis` through the unitless-zero clause in [`parse_flex_basis`]
/// (`flex: 2 3 0` → `basis: Length(Px(0.0))`).
///
/// The grammar places `<flex-shrink>` directly after `<flex-grow>` rather
/// than in a separate `||` alternative. Accordingly, this function tries
/// `shrink` only **immediately after** obtaining `grow`.
///
/// This function leaves any fifth or later token unconsumed. As with
/// [`parse_padding_shorthand`](super::box_model::parse_padding_shorthand), the caller's
/// `expect_exhausted` (`rule.rs::DeclParser`) drops the whole declaration.
pub(crate) fn parse_flex_shorthand(input: &mut Parser<'_, '_>) -> Option<FlexShorthand> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(FlexShorthand {
            grow: 0.0,
            shrink: 0.0,
            basis: FlexBasisValue::Auto,
        });
    }
    let mut grow: Option<f32> = None;
    let mut shrink: Option<f32> = None;
    let mut basis: Option<FlexBasisValue> = None;
    for _ in 0..3 {
        let mut progressed = false;
        if grow.is_none()
            && let Ok(g) = input.try_parse(parse_nonneg_finite_number_res)
        {
            grow = Some(g);
            // The grammar puts `<flex-shrink>` directly after `<flex-grow>`; do
            // not try it as an independent alternative (see the docs above).
            if let Ok(s) = input.try_parse(parse_nonneg_finite_number_res) {
                shrink = Some(s);
            }
            progressed = true;
        }
        if !progressed
            && basis.is_none()
            && let Ok(b) = input.try_parse(parse_flex_basis_res)
        {
            basis = Some(b);
            progressed = true;
        }
        if !progressed {
            break;
        }
    }
    // At least one grammar group is required; the zero-value form
    // (`flex:` with no following value) is invalid.
    if grow.is_none() && basis.is_none() {
        return None;
    }
    Some(FlexShorthand {
        // These are shorthand-local defaults (see the `FlexShorthand` docs),
        // not the longhands' initial values (grow=0 / basis=auto).
        grow: grow.unwrap_or(1.0),
        shrink: shrink.unwrap_or(1.0),
        basis: basis.unwrap_or(FlexBasisValue::Length(Length::Px(0.0))),
    })
}

/// Parses `order: <integer>` (CSS Flexible Box Layout Module Level 1
/// §4.2 "Display Order: the order property"
/// <https://www.w3.org/TR/css-flexbox-1/#order-property>; see the
/// [`PropertyValue::Order`] docs).
///
/// Calls `expect_integer` directly, as [`parse_z_index`](super::box_model::parse_z_index) does.
/// The grammar accepts only `<integer>`, including signed values, with no range limit.
pub(super) fn parse_order(input: &mut Parser<'_, '_>) -> Option<i32> {
    input.try_parse(|i| i.expect_integer()).ok()
}

/// The `Result` form of [`parse_flex_direction`], following the wrapper
/// pattern of [`parse_padding_side_res`](super::box_model::parse_padding_side_res), for the `try_parse` in
/// [`parse_flex_flow`].
fn parse_flex_direction_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<FlexDirectionValue, ParseError<'i, ()>> {
    parse_flex_direction(input).ok_or_else(|| input.new_custom_error(()))
}

/// The `Result` form of [`parse_flex_wrap`], following the wrapper
/// pattern of [`parse_flex_direction_res`], for the `try_parse` in
/// [`parse_flex_flow`].
fn parse_flex_wrap_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<FlexWrapValue, ParseError<'i, ()>> {
    parse_flex_wrap(input).ok_or_else(|| input.new_custom_error(()))
}

/// Parses `flex-flow: <'flex-direction'> || <'flex-wrap'>` (CSS
/// Flexible Box Layout Module Level 1 §5.3 "Flex Direction and Wrap: the
/// flex-flow shorthand"
/// <https://www.w3.org/TR/css-flexbox-1/#flex-flow-property>; see the
/// [`FlexFlow`] docs).
///
/// The `||` grammar allows either order, each component at most once, and
/// requires at least one. Up to two iterations try both components. Omitted
/// components take their longhands' initial values (direction=row / wrap=nowrap).
/// This function leaves extra tokens unconsumed. The caller's `expect_exhausted`
/// (`rule.rs::DeclParser`) drops the declaration, as with [`parse_flex_shorthand`].
pub(crate) fn parse_flex_flow(input: &mut Parser<'_, '_>) -> Option<FlexFlow> {
    let mut direction: Option<FlexDirectionValue> = None;
    let mut wrap: Option<FlexWrapValue> = None;
    for _ in 0..2 {
        let mut progressed = false;
        if direction.is_none()
            && let Ok(d) = input.try_parse(parse_flex_direction_res)
        {
            direction = Some(d);
            progressed = true;
        }
        if !progressed
            && wrap.is_none()
            && let Ok(w) = input.try_parse(parse_flex_wrap_res)
        {
            wrap = Some(w);
            progressed = true;
        }
        if !progressed {
            break;
        }
    }
    if direction.is_none() && wrap.is_none() {
        return None;
    }
    Some(FlexFlow {
        direction: direction.unwrap_or(FlexDirectionValue::Row),
        wrap: wrap.unwrap_or(FlexWrapValue::NoWrap),
    })
}

/// Shared parser for `justify-content` and `align-content`; see the
/// scope carving in the [`ContentAlignmentValue`] docs. This function consumes
/// only one ident. Thus `safe`/`unsafe` prefixes, `<baseline-position>`, and
/// the `left`/`right` values specific to justify-content remain unmatched:
/// two-token sequences and unsupported idents return `_ => None`.
pub(super) fn parse_content_alignment(input: &mut Parser<'_, '_>) -> Option<ContentAlignmentValue> {
    ContentAlignmentValue::from_css_ident(input.expect_ident().ok()?)
}

/// The `Result` form of [`parse_content_alignment`], following the wrapper
/// pattern of [`parse_padding_side_res`](super::box_model::parse_padding_side_res), for the `try_parse` in
/// [`parse_place_content_shorthand`].
fn parse_content_alignment_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<ContentAlignmentValue, ParseError<'i, ()>> {
    parse_content_alignment(input).ok_or_else(|| input.new_custom_error(()))
}

/// Parser for `align-items`; see the scope carving in the
/// [`SelfAlignmentValue`] docs. [`parse_align_self`] reuses this function for
/// `align-self`, while accepting `auto` as an additional value.
pub(super) fn parse_self_alignment(input: &mut Parser<'_, '_>) -> Option<SelfAlignmentValue> {
    let safe = input.try_parse(|i| i.expect_ident_matching("safe")).is_ok();
    let _unsafe = !safe
        && input
            .try_parse(|i| i.expect_ident_matching("unsafe"))
            .is_ok();
    let ident = input.expect_ident().ok()?.clone();
    if ident.eq_ignore_ascii_case("last") {
        input.expect_ident_matching("baseline").ok()?;
        return Some(SelfAlignmentValue::Baseline);
    }
    let value = match ident.to_ascii_lowercase().as_str() {
        "normal" => SelfAlignmentValue::Normal,
        "stretch" => SelfAlignmentValue::Stretch,
        "center" => SelfAlignmentValue::Center,
        "start" | "self-start" => SelfAlignmentValue::Start,
        "end" | "self-end" => SelfAlignmentValue::End,
        "left" => SelfAlignmentValue::Start,
        "right" => SelfAlignmentValue::End,
        "flex-start" => SelfAlignmentValue::FlexStart,
        "flex-end" => SelfAlignmentValue::FlexEnd,
        "baseline" => SelfAlignmentValue::Baseline,
        _ => return None,
    };
    Some(
        if safe
            && matches!(
                value,
                SelfAlignmentValue::Center | SelfAlignmentValue::End | SelfAlignmentValue::FlexEnd
            )
        {
            SelfAlignmentValue::Start
        } else {
            value
        },
    )
}

/// The `Result` form of [`parse_self_alignment`], following the wrapper
/// pattern of [`parse_padding_side_res`](super::box_model::parse_padding_side_res), for the `try_parse` in
/// [`parse_place_items_shorthand`].
fn parse_self_alignment_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<SelfAlignmentValue, ParseError<'i, ()>> {
    parse_self_alignment(input).ok_or_else(|| input.new_custom_error(()))
}

/// Parser for `align-self: auto | …`. It tries the `auto` branch first
/// using a `try_parse` checkpoint (as explained under "Order of alternatives"
/// in the [`parse_width`](super::box_model::parse_width) docs). Other values delegate to
/// [`parse_self_alignment`], which uses the same keyword set as `align-items`.
pub(super) fn parse_align_self(input: &mut Parser<'_, '_>) -> Option<AlignSelfValue> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(AlignSelfValue::Auto);
    }
    let safe = input.try_parse(|i| i.expect_ident_matching("safe")).is_ok();
    let _unsafe = !safe
        && input
            .try_parse(|i| i.expect_ident_matching("unsafe"))
            .is_ok();
    parse_self_alignment(input).map(|value| {
        AlignSelfValue::Value(if safe && matches!(value, SelfAlignmentValue::Center) {
            SelfAlignmentValue::Start
        } else {
            value
        })
    })
}

pub(super) fn parse_justify_self(input: &mut Parser<'_, '_>) -> Option<AlignSelfValue> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(AlignSelfValue::Auto);
    }
    let safe = input.try_parse(|i| i.expect_ident_matching("safe")).is_ok();
    let _unsafe = !safe
        && input
            .try_parse(|i| i.expect_ident_matching("unsafe"))
            .is_ok();
    parse_self_alignment(input).map(|value| {
        AlignSelfValue::Value(
            if safe
                && matches!(
                    value,
                    SelfAlignmentValue::Center
                        | SelfAlignmentValue::End
                        | SelfAlignmentValue::FlexEnd
                )
            {
                SelfAlignmentValue::Start
            } else {
                value
            },
        )
    })
}

/// The `Result` form of [`parse_align_self`], following the wrapper
/// pattern of [`parse_padding_side_res`](super::box_model::parse_padding_side_res), for the `try_parse` in
/// [`parse_place_self_shorthand`].
fn parse_align_self_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<AlignSelfValue, ParseError<'i, ()>> {
    parse_align_self(input).ok_or_else(|| input.new_custom_error(()))
}

/// Shared parser for `row-gap` and `column-gap`:
/// `normal | <length-percentage [0,∞]>` (CSS Box Alignment Module Level 3 §8.1
/// <https://www.w3.org/TR/css-align-3/#propdef-row-gap>).
///
/// It tries `normal` before the length branch. The shared length parser
/// accepts percentages; this property additionally rejects negative values,
/// matching the `[0,∞]` constraint with [`parse_padding_side`](super::box_model::parse_padding_side).
pub(crate) fn parse_gap_value(input: &mut Parser<'_, '_>) -> Option<LengthOrNormal> {
    if input
        .try_parse(|i| i.expect_ident_matching("normal"))
        .is_ok()
    {
        return Some(LengthOrNormal::Normal);
    }
    let length = parse_length_value(input, true)?;
    (length.payload() >= 0.0).then_some(LengthOrNormal::Length(length))
}

/// The `Result` form of [`parse_gap_value`], following the wrapper
/// pattern of [`parse_padding_side_res`](super::box_model::parse_padding_side_res), for the `try_parse` in
/// [`parse_gap_shorthand`].
fn parse_gap_value_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<LengthOrNormal, ParseError<'i, ()>> {
    parse_gap_value(input).ok_or_else(|| input.new_custom_error(()))
}

/// Parses the shorthand `gap: <'row-gap'> <'column-gap'>?` (CSS Box
/// Alignment Module Level 3 §8.2
/// <https://www.w3.org/TR/css-align-3/#propdef-gap>). When the second component
/// is omitted, the first is copied unchanged, as the spec requires; see the
/// [`GapShorthand`] docs.
pub(crate) fn parse_gap_shorthand(input: &mut Parser<'_, '_>) -> Option<GapShorthand> {
    let row = parse_gap_value(input)?;
    let column = input.try_parse(parse_gap_value_res).unwrap_or(row);
    Some(GapShorthand { row, column })
}

/// Parses the shorthand `place-content: <'align-content'> <'justify-content'>?`
/// (CSS Box Alignment Module Level 3 §5.2
/// <https://www.w3.org/TR/css-align-3/#propdef-place-content>). When the second
/// component is omitted, the first is copied unchanged, as the spec requires;
/// see the [`PlaceContentShorthand`] docs. The `<baseline-position>` exception
/// cannot arise in this crate because [`ContentAlignmentValue`] has no such variant.
pub(crate) fn parse_place_content_shorthand(
    input: &mut Parser<'_, '_>,
) -> Option<PlaceContentShorthand> {
    let align = parse_content_alignment(input)?;
    let justify = input
        .try_parse(parse_content_alignment_res)
        .unwrap_or(align);
    Some(PlaceContentShorthand { align, justify })
}

// ─────────────────────────────────────────────────────────────────────────
// CSS Grid Layout Module Level 1 parsers.
// ─────────────────────────────────────────────────────────────────────────

/// The `<custom-ident>` exclusion list used by both the [`GridLineValue`]
/// productions and `<line-names>` (§7.2.2). It adds `span` and `auto` to the
/// exclusions in [`is_reserved_custom_ident`].
///
/// CSS Grid Layout Module Level 1 §8.3 says (verbatim): "In all the above
/// productions, the `<custom-ident>` additionally excludes the keywords
/// `span` and `auto`"; §7.2.2 also says: "A line name cannot be span
/// or auto, i.e. the `<custom-ident>` in the `<line-names>` production
/// excludes the keywords span and auto." Thus §7.2.2 explicitly applies
/// this exclusion to `<line-names>`, just as
/// [`is_reserved_counter_name`](super::content::is_reserved_counter_name) adds `none` to its base list.
pub(crate) fn is_reserved_grid_line_name(ident: &str) -> bool {
    is_reserved_custom_ident(ident)
        || matches!(ident.to_ascii_lowercase().as_str(), "span" | "auto")
}

/// Parses `<custom-ident>` within [`GridLineValue`] productions.
/// It applies the extra exclusions in [`is_reserved_grid_line_name`], unlike
/// [`parse_custom_ident`].
fn parse_grid_custom_ident(input: &mut Parser<'_, '_>) -> Option<SmolStr> {
    let ident = input.expect_ident().ok()?.clone();
    if is_reserved_grid_line_name(&ident) {
        None
    } else {
        Some(SmolStr::new(ident.as_ref()))
    }
}

/// The `Result` form of [`parse_grid_custom_ident`], following the
/// wrapper pattern of [`parse_padding_side_res`](super::box_model::parse_padding_side_res), for `try_parse`
/// inside `&&`/`||` combinators.
fn parse_grid_custom_ident_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<SmolStr, ParseError<'i, ()>> {
    parse_grid_custom_ident(input).ok_or_else(|| input.new_custom_error(()))
}

/// Parses `<line-names> = '[' <custom-ident>* ']'` (CSS Grid Layout
/// Module Level 1 §7.2.2 "Naming Grid Lines: the `[<custom-ident>*]` syntax"
/// <https://www.w3.org/TR/css-grid-1/#named-lines>).
///
/// The `<custom-ident>` inside `<line-names>` **does** receive the extra
/// `span`/`auto` exclusions in [`is_reserved_grid_line_name`]. §7.2.2 says
/// (verbatim): "A line name cannot be span or auto, i.e. the `<custom-ident>`
/// in the `<line-names>` production excludes the keywords span and auto."
/// This exclusion applies here, not just to the `<grid-line>` productions
/// in §8.3.
///
/// Returns `None` when `[` is absent; the caller
/// [`parse_line_names_or_empty`] then falls back to an empty `Vec`.
/// An empty `[]` returns `Some(vec![])`.
fn parse_line_names(input: &mut Parser<'_, '_>) -> Option<Vec<SmolStr>> {
    input
        .try_parse(|i| -> Result<Vec<SmolStr>, ParseError<'_, ()>> {
            i.expect_square_bracket_block()?;
            i.parse_nested_block(|inner| {
                let mut names = Vec::new();
                while !inner.is_exhausted() {
                    names.push(
                        parse_grid_custom_ident(inner).ok_or_else(|| inner.new_custom_error(()))?,
                    );
                }
                Ok(names)
            })
        })
        .ok()
}

/// Fills each interleaved slot in [`GridTrackList::line_names`] and
/// [`GridTrackRepeat::line_names`], normalizing optional `<line-names>?`
/// to an empty `Vec`.
fn parse_line_names_or_empty(input: &mut Parser<'_, '_>) -> Vec<SmolStr> {
    parse_line_names(input).unwrap_or_default()
}

/// Parses `<flex [0,∞]>`, the `fr` unit (CSS Grid Layout Module Level 1
/// §7.2.4 "Flexible Lengths: the fr unit"
/// <https://www.w3.org/TR/css-grid-1/#fr-unit>).
///
/// Enforces both non-negativity and finiteness while parsing for the same
/// reason described in the [`parse_nonneg_finite_number`] docs: the raikiri-dom
/// bridge copies directly to taffy's `MaxTrackSizingFunction::fr` without a
/// conversion or sink guard.
///
/// Obtains the token through `next_numeric_stable` (see "Numeric-token NaN
/// stabilization" near the start of the module docs). The cssparser
/// tokenizer can collapse a zero-mantissa, huge-exponent literal such as
/// `grid-template-columns: 0e999fr` to `NaN`. That artifact is corrected
/// here, so the value resolves to the spec-correct `Flex(0.0)`.
fn parse_grid_flex_res<'i>(input: &mut Parser<'i, '_>) -> Result<f32, ParseError<'i, ()>> {
    match &next_numeric_stable(input)? {
        Token::Dimension { value, unit, .. } if unit.eq_ignore_ascii_case("fr") => {
            if value.is_finite() && *value >= 0.0 {
                Ok(*value)
            } else {
                Err(input.new_custom_error(()))
            }
        }
        t => Err(input.new_unexpected_token_error(t.clone())),
    }
}

/// Parses `<track-breadth> = <length-percentage [0,∞]> | <flex [0,∞]> | min-content
/// | max-content | auto` (CSS Grid Layout Module Level 1 §7.2.1 "Track Sizes"
/// <https://www.w3.org/TR/css-grid-1/#valdef-grid-template-columns-track-breadth>).
///
/// The `<length-percentage>` branch must be last: [`parse_length_value`]
/// consumes a token even on failure (as noted in the [`parse_flex_basis`]
/// docs), so no other alternative could be tried afterward.
fn parse_track_breadth(input: &mut Parser<'_, '_>) -> Option<GridTrackBreadth> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(GridTrackBreadth::Auto);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("min-content"))
        .is_ok()
    {
        return Some(GridTrackBreadth::MinContent);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("max-content"))
        .is_ok()
    {
        return Some(GridTrackBreadth::MaxContent);
    }
    if let Ok(flex) = input.try_parse(parse_grid_flex_res) {
        return Some(GridTrackBreadth::Flex(flex));
    }
    let length = input
        .try_parse(|i| parse_length_value(i, true).ok_or_else(|| i.new_custom_error::<(), ()>(())))
        .ok()?;
    (length.payload() >= 0.0).then_some(GridTrackBreadth::Length(length))
}

/// Parses `<inflexible-breadth> = <length-percentage [0,∞]> | min-content |
/// max-content | auto` (CSS Grid Layout Module Level 1 §7.2.1
/// <https://www.w3.org/TR/css-grid-1/#valdef-grid-template-columns-inflexible-breadth>).
/// It has the same shape as [`parse_track_breadth`] but no `<flex>` branch.
fn parse_inflexible_breadth(input: &mut Parser<'_, '_>) -> Option<GridInflexibleBreadth> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(GridInflexibleBreadth::Auto);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("min-content"))
        .is_ok()
    {
        return Some(GridInflexibleBreadth::MinContent);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("max-content"))
        .is_ok()
    {
        return Some(GridInflexibleBreadth::MaxContent);
    }
    let length = input
        .try_parse(|i| parse_length_value(i, true).ok_or_else(|| i.new_custom_error::<(), ()>(())))
        .ok()?;
    (length.payload() >= 0.0).then_some(GridInflexibleBreadth::Length(length))
}

/// Parses `minmax( <inflexible-breadth>, <track-breadth> )` (CSS Grid
/// Layout Module Level 1 §7.2.1
/// <https://www.w3.org/TR/css-grid-1/#funcdef-grid-template-columns-minmax>).
fn parse_grid_minmax_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<(GridInflexibleBreadth, GridTrackBreadth), ParseError<'i, ()>> {
    input.expect_function_matching("minmax")?;
    input.parse_nested_block(|inner| {
        let min = parse_inflexible_breadth(inner).ok_or_else(|| inner.new_custom_error(()))?;
        inner.expect_comma()?;
        let max = parse_track_breadth(inner).ok_or_else(|| inner.new_custom_error(()))?;
        Ok((min, max))
    })
}

/// Parses `fit-content( <length-percentage [0,∞]> )` (CSS Grid Layout
/// Module Level 1 §7.2.1
/// <https://www.w3.org/TR/css-grid-1/#funcdef-grid-template-columns-fit-content>).
fn parse_grid_fit_content_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<Length, ParseError<'i, ()>> {
    input.expect_function_matching("fit-content")?;
    input.parse_nested_block(|inner| {
        let len = parse_length_value(inner, true).ok_or_else(|| inner.new_custom_error(()))?;
        if len.payload() >= 0.0 {
            Ok(len)
        } else {
            Err(inner.new_custom_error(()))
        }
    })
}

/// Parses `<track-size>`; see the [`GridTrackSize`] docs. It tries three
/// alternatives in order: `minmax()` → `fit-content()` → bare `<track-breadth>`.
/// The first two have distinct function names, so their order matters little.
/// Bare `<track-breadth>` must come last because it can consume a token on
/// failure (see the [`parse_track_breadth`] docs).
fn parse_track_size(input: &mut Parser<'_, '_>) -> Option<GridTrackSize> {
    if let Ok((min, max)) = input.try_parse(parse_grid_minmax_res) {
        return Some(GridTrackSize::MinMax(min, max));
    }
    if let Ok(limit) = input.try_parse(parse_grid_fit_content_res) {
        return Some(GridTrackSize::FitContent(limit));
    }
    parse_track_breadth(input).map(GridTrackSize::Breadth)
}

/// Checks whether [`GridTrackBreadth`] is a `<fixed-breadth>` (only a
/// `<length-percentage>`); used by [`grid_track_size_is_fixed`].
fn grid_track_breadth_is_fixed(b: &GridTrackBreadth) -> bool {
    matches!(b, GridTrackBreadth::Length(_))
}

/// Checks whether [`GridInflexibleBreadth`] is a `<fixed-breadth>`;
/// used by [`grid_track_size_is_fixed`].
fn grid_inflexible_breadth_is_fixed(b: &GridInflexibleBreadth) -> bool {
    matches!(b, GridInflexibleBreadth::Length(_))
}

/// Checks whether [`GridTrackSize`] satisfies the `<fixed-size>` constraint
/// (CSS Grid Layout Module Level 1 §7.2.1: `<fixed-size> = <fixed-breadth> |
/// minmax( <fixed-breadth>, <track-breadth> ) |
/// minmax( <inflexible-breadth>, <fixed-breadth> )`); see "fixed-size constraint"
/// in the [`GridTrackSize`] docs.
///
/// For `minmax()`, either the minimum or maximum must be a `<fixed-breadth>`;
/// the spec does not require both to be fixed. For example,
/// `minmax(100px, 1fr)` is a valid `<fixed-size>`. `fit-content()` is always
/// `false`, because the `<fixed-size>` grammar has no such alternative.
pub(crate) fn grid_track_size_is_fixed(t: &GridTrackSize) -> bool {
    match t {
        GridTrackSize::Breadth(b) => grid_track_breadth_is_fixed(b),
        GridTrackSize::MinMax(min, max) => {
            grid_inflexible_breadth_is_fixed(min) || grid_track_breadth_is_fixed(max)
        }
        GridTrackSize::FitContent(_) => false,
    }
}

/// Parses the repetition count of `repeat()` (`<integer [1,∞]>`,
/// `auto-fill`, or `auto-fit`); see the [`GridRepeatCount`] docs.
fn parse_grid_repeat_count(input: &mut Parser<'_, '_>) -> Option<GridRepeatCount> {
    if input
        .try_parse(|i| i.expect_ident_matching("auto-fill"))
        .is_ok()
    {
        return Some(GridRepeatCount::AutoFill);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("auto-fit"))
        .is_ok()
    {
        return Some(GridRepeatCount::AutoFit);
    }
    let n = input.try_parse(|i| i.expect_integer()).ok()?;
    (n >= 1).then_some(GridRepeatCount::Count(n as u32))
}

/// Parses `repeat( <count>, [ <line-names>? <track-size> ]+ <line-names>? )`;
/// see the [`GridTrackRepeat`] docs. Whether count is
/// [`GridRepeatCount::Count`], [`GridRepeatCount::AutoFill`], or
/// [`GridRepeatCount::AutoFit`] determines whether inner tracks must follow
/// the `<track-size>` or `<fixed-size>` grammar (see "Allowed counts and the
/// `<fixed-size>` constraint" in the [`GridTrackRepeat`] docs). This function
/// parses the common, full `<track-size>` grammar. Then
/// [`parse_grid_template_tracks`] checks the `<fixed-size>` constraint against
/// the complete track list.
fn parse_grid_repeat_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<GridTrackRepeat, ParseError<'i, ()>> {
    input.expect_function_matching("repeat")?;
    input.parse_nested_block(|inner| {
        let count = parse_grid_repeat_count(inner).ok_or_else(|| inner.new_custom_error(()))?;
        inner.expect_comma()?;
        let mut line_names = vec![parse_line_names_or_empty(inner)];
        let mut tracks = Vec::new();
        while let Some(size) = parse_track_size(inner) {
            tracks.push(size);
            line_names.push(parse_line_names_or_empty(inner));
        }
        if tracks.is_empty() {
            return Err(inner.new_custom_error(()));
        }
        Ok(GridTrackRepeat {
            count,
            line_names,
            tracks,
        })
    })
}

/// The `Option` form of [`parse_grid_repeat_res`] for alternation in
/// [`parse_grid_track_list`].
pub(crate) fn parse_grid_repeat(input: &mut Parser<'_, '_>) -> Option<GridTrackRepeat> {
    input.try_parse(parse_grid_repeat_res).ok()
}

/// Parses the sequence of components in `<track-list>` / `<auto-track-list>`:
/// `[ <line-names>? [ <track-size> | <track-repeat> ] ]+ <line-names>?`.
/// It does not check the `<fixed-size>` constraint;
/// [`grid_track_list_obeys_auto_repeat_constraint`] performs that check in the
/// caller [`parse_grid_template_tracks`].
fn parse_grid_track_list(input: &mut Parser<'_, '_>) -> Option<GridTrackList> {
    let mut line_names = vec![parse_line_names_or_empty(input)];
    let mut components = Vec::new();
    loop {
        if let Some(repeat) = parse_grid_repeat(input) {
            components.push(GridTrackListComponent::Repeat(repeat));
        } else if let Some(size) = parse_track_size(input) {
            components.push(GridTrackListComponent::Size(size));
        } else {
            break;
        }
        line_names.push(parse_line_names_or_empty(input));
    }
    if components.is_empty() {
        None
    } else {
        Some(GridTrackList {
            line_names,
            components,
        })
    }
}

/// Checks whether [`parse_grid_track_list`] returns a [`GridTrackList`]
/// that satisfies two spec constraints after parsing (see "Allowed counts and
/// the `<fixed-size>` constraint" in the [`GridTrackRepeat`] docs):
///
/// 1. CSS Grid Layout Module Level 1 §7.2.3.1 says (verbatim): "It can only appear
///    once in the track list." At most one auto-fill/auto-fit repeat is allowed.
/// 2. §7.2.3.1 says (verbatim): "Automatic repetitions (auto-fill or auto-fit)
///    cannot be combined with fully intrinsic or flexible sizes." If an
///    auto-repeat is present, **every other track** in the track list, both bare
///    tracks and those inside other `repeat()` calls, as well as the tracks
///    inside the auto-repeat itself, must satisfy [`grid_track_size_is_fixed`].
fn grid_track_list_obeys_auto_repeat_constraint(list: &GridTrackList) -> bool {
    let auto_repeat_count = list
        .components
        .iter()
        .filter(|c| {
            matches!(
                c,
                GridTrackListComponent::Repeat(r)
                    if matches!(r.count, GridRepeatCount::AutoFill | GridRepeatCount::AutoFit)
            )
        })
        .count();
    if auto_repeat_count > 1 {
        return false;
    }
    if auto_repeat_count == 0 {
        return true;
    }
    list.components.iter().all(|c| match c {
        GridTrackListComponent::Size(size) => grid_track_size_is_fixed(size),
        GridTrackListComponent::Repeat(r) => r.tracks.iter().all(grid_track_size_is_fixed),
    })
}

/// Parses `grid-template-columns` / `grid-template-rows`:
/// `none | <track-list> | <auto-track-list>` (CSS Grid Layout Module Level 1 §7.2
/// <https://www.w3.org/TR/css-grid-1/#track-sizing>); see the
/// [`GridTemplateTracks`] docs.
pub(super) fn parse_grid_area_shorthand(input: &mut Parser<'_, '_>) -> Option<GridAreaShorthand> {
    let row_start = parse_grid_line(input)?;
    input.try_parse(|i| i.expect_delim('/')).ok()?;
    let column_start = parse_grid_line(input)?;
    input.try_parse(|i| i.expect_delim('/')).ok()?;
    let row_end = parse_grid_line(input)?;
    input.try_parse(|i| i.expect_delim('/')).ok()?;
    let column_end = parse_grid_line(input)?;
    Some(GridAreaShorthand {
        row_start,
        column_start,
        row_end,
        column_end,
    })
}

pub(super) fn parse_grid_shorthand(input: &mut Parser<'_, '_>) -> Option<GridShorthand> {
    let rows = parse_grid_template_tracks(input)?;
    if input.try_parse(|i| i.expect_delim('/')).is_err() {
        return None;
    }
    let columns = parse_grid_template_tracks(input)?;
    Some(GridShorthand { rows, columns })
}

pub(crate) fn parse_grid_template_tracks(input: &mut Parser<'_, '_>) -> Option<GridTemplateTracks> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(GridTemplateTracks::None);
    }
    let list = parse_grid_track_list(input)?;
    if !grid_track_list_obeys_auto_repeat_constraint(&list) {
        return None;
    }
    Some(GridTemplateTracks::List(Arc::new(list)))
}

/// Parses `grid-auto-columns` / `grid-auto-rows`: `<track-size>+`
/// (CSS Grid Layout Module Level 1 §7.6
/// <https://www.w3.org/TR/css-grid-1/#propdef-grid-auto-columns>).
/// Unlike `<track-list>`, this grammar has neither `repeat()` nor interleaved
/// `<line-names>`: it accepts only a sequence of bare `<track-size>` values.
pub(super) fn parse_grid_auto_track_list(
    input: &mut Parser<'_, '_>,
) -> Option<Arc<Vec<GridTrackSize>>> {
    let mut tracks = Vec::new();
    while let Some(size) = parse_track_size(input) {
        tracks.push(size);
    }
    if tracks.is_empty() {
        None
    } else {
        Some(Arc::new(tracks))
    }
}

/// Parses `grid-auto-flow: [ row | column ] || dense` (CSS Grid
/// Layout Module Level 1 §7.7
/// <https://www.w3.org/TR/css-grid-1/#propdef-grid-auto-flow>).
///
/// A standalone `dense` (with no axis) maps to
/// [`GridAutoFlowValue::RowDense`] because the default axis is `row` (see the
/// isomorphic-value note in the [`GridAutoFlowValue`] docs). Both groups may
/// appear in either order (`||`), so at most two iterations try both.
pub(super) fn parse_grid_auto_flow(input: &mut Parser<'_, '_>) -> Option<GridAutoFlowValue> {
    #[derive(Clone, Copy)]
    enum Axis {
        Row,
        Column,
    }
    let mut axis: Option<Axis> = None;
    let mut dense = false;
    for _ in 0..2 {
        if axis.is_none() && input.try_parse(|i| i.expect_ident_matching("row")).is_ok() {
            axis = Some(Axis::Row);
            continue;
        }
        if axis.is_none()
            && input
                .try_parse(|i| i.expect_ident_matching("column"))
                .is_ok()
        {
            axis = Some(Axis::Column);
            continue;
        }
        if !dense
            && input
                .try_parse(|i| i.expect_ident_matching("dense"))
                .is_ok()
        {
            dense = true;
            continue;
        }
        break;
    }
    match (axis, dense) {
        (None, false) => None,
        (None, true) => Some(GridAutoFlowValue::RowDense),
        (Some(Axis::Row), false) => Some(GridAutoFlowValue::Row),
        (Some(Axis::Row), true) => Some(GridAutoFlowValue::RowDense),
        (Some(Axis::Column), false) => Some(GridAutoFlowValue::Column),
        (Some(Axis::Column), true) => Some(GridAutoFlowValue::ColumnDense),
    }
}

/// Parses `span <integer [1,∞]> || <custom-ident>` after the caller has
/// already consumed the `span` ident. This is the tail helper for
/// [`parse_grid_line`].
fn parse_grid_line_span_tail(input: &mut Parser<'_, '_>) -> Option<GridLineValue> {
    let mut number: Option<u32> = None;
    let mut name: Option<SmolStr> = None;
    for _ in 0..2 {
        if number.is_none()
            && let Ok(n) = input.try_parse(|i| i.expect_integer())
        {
            if n < 1 {
                return None;
            }
            number = Some(n as u32);
            continue;
        }
        if name.is_none()
            && let Ok(n) = input.try_parse(parse_grid_custom_ident_res)
        {
            name = Some(n);
            continue;
        }
        break;
    }
    match (number, name) {
        (Some(n), Some(name)) => Some(GridLineValue::SpanNamed(name, n)),
        (Some(n), None) => Some(GridLineValue::Span(n)),
        (None, Some(name)) => Some(GridLineValue::SpanNamed(name, 1)),
        // Nothing follows `span`; invalid because the right-hand group in
        // `span && [ <integer> || <custom-ident> ]` is required.
        (None, None) => None,
    }
}

/// Parses `<grid-line>`; see the [`GridLineValue`] docs.
///
/// The `[ [ <integer> ] && <custom-ident>? ]` alternative is order-free
/// (`&&`), so at most two iterations try `<integer>` before or after the
/// custom ident. If **no `<integer>` appears** (`number.is_none()`), the
/// input instead matches the separate, top-level `<custom-ident>` alternative.
/// This returns [`GridLineValue::Named`] (a bare ident subject to the
/// shorthand omission-copy rule), rather than [`GridLineValue::NamedLine`]
/// (which has an accompanying `<integer>` and is not copied). The verbatim
/// spec quotation in the [`GridLineShorthand`] docs requires this distinction;
/// see [`parse_grid_line_shorthand`].
pub(crate) fn parse_grid_line(input: &mut Parser<'_, '_>) -> Option<GridLineValue> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(GridLineValue::Auto);
    }
    if input.try_parse(|i| i.expect_ident_matching("span")).is_ok() {
        return parse_grid_line_span_tail(input);
    }
    let mut number: Option<i32> = None;
    let mut name: Option<SmolStr> = None;
    for _ in 0..2 {
        if number.is_none()
            && let Ok(n) = input.try_parse(|i| i.expect_integer())
        {
            number = Some(n);
            continue;
        }
        if name.is_none()
            && let Ok(n) = input.try_parse(parse_grid_custom_ident_res)
        {
            name = Some(n);
            continue;
        }
        break;
    }
    match (number, name) {
        // Spec verbatim: "Negative integers or zero are invalid." Reject `0`
        // regardless of whether a name is present.
        (Some(0), _) => None,
        (Some(n), Some(name)) => Some(GridLineValue::NamedLine(name, n)),
        (Some(n), None) => Some(GridLineValue::Line(n)),
        (None, Some(name)) => Some(GridLineValue::Named(name)),
        (None, None) => None,
    }
}

/// Parses the `grid-row` / `grid-column` shorthand:
/// `<grid-line> [ / <grid-line> ]?`; see the [`GridLineShorthand`] docs.
pub(crate) fn parse_grid_line_shorthand(input: &mut Parser<'_, '_>) -> Option<GridLineShorthand> {
    let start = parse_grid_line(input)?;
    if input.try_parse(|i| i.expect_delim('/')).is_ok() {
        let end = parse_grid_line(input)?;
        return Some(GridLineShorthand { start, end });
    }
    // The spec text quoted in the `GridLineShorthand` docs says: when
    // the second component is omitted, copy the first component's name into
    // the second only if the first is a `<custom-ident>` (the bare
    // `GridLineValue::Named` form, without an accompanying `<integer>`).
    // Otherwise the second component is `auto`.
    let end = match &start {
        GridLineValue::Named(name) => GridLineValue::Named(name.clone()),
        _ => GridLineValue::Auto,
    };
    Some(GridLineShorthand { start, end })
}

/// Splits one `<string>` from `grid-template-areas` into cell tokens.
///
/// CSS Grid Layout Module Level 1 §7.3 gives this verbatim tokenization rule
/// (<https://www.w3.org/TR/css-grid-1/#grid-template-areas-property>):
/// "Tokenize the string into a list of the following tokens, using
/// longest-match semantics." Consecutive ident code points make a named cell;
/// consecutive `.` characters make a null cell; whitespace is ignored rather
/// than tokenized; anything else produces an invalid trash token.
///
/// Returns `None` only when it finds a trash token. The spec says (verbatim):
/// "A trash token is a syntax error, and makes the declaration invalid."
///
/// For whitespace, use the CSS Syntax 3 definition
/// (<https://www.w3.org/TR/css-syntax-3/#whitespace>), rather than
/// `char::is_whitespace()` (the Unicode `White_Space` property, which also
/// includes non-ASCII whitespace such as `U+3000`). The spec says (verbatim):
/// "A newline, U+0009 CHARACTER TABULATION, or U+0020 SPACE." Its input
/// preprocessing (<https://www.w3.org/TR/css-syntax-3/#input-preprocessing>)
/// normalizes U+000D/U+000C to U+000A. Thus only `'
/// '` needs checking here:
/// CR and FF have already been removed during stylesheet tokenization.
fn tokenize_grid_area_row(s: &str) -> Option<Vec<Option<SmolStr>>> {
    let mut cells = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(&c) = chars.peek() {
        if matches!(c, '\t' | '\n' | ' ') {
            chars.next();
        } else if c == '.' {
            while chars.peek() == Some(&'.') {
                chars.next();
            }
            cells.push(None);
        } else if is_grid_area_ident_char(c) {
            let mut name = String::new();
            while let Some(&c2) = chars.peek() {
                if !is_grid_area_ident_char(c2) {
                    break;
                }
                name.push(c2);
                chars.next();
            }
            cells.push(Some(SmolStr::new(name)));
        } else {
            return None;
        }
    }
    Some(cells)
}

/// Approximates the CSS Syntax 3 "ident code point" classifier (letter,
/// digit, `-`, `_`, or non-ASCII) solely for named-cell tokenization in
/// [`tokenize_grid_area_row`]. It does not process escape sequences (`\XX`):
/// cssparser has already unescaped the `<string>` token's literal character
/// values. String tokenization for `grid-template-areas` (§7.3) does not
/// reinterpret CSS escapes; its definition classifies code points directly.
fn is_grid_area_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-' || !c.is_ascii()
}

/// Builds rows already processed by [`tokenize_grid_area_row`] into
/// [`GridTemplateAreas`]. It computes the bounding boxes of named areas and
/// checks the verbatim §7.3 requirement: "If a named grid area spans multiple
/// grid cells, but those cells do not form a single filled-in rectangle, the
/// declaration is invalid."
///
/// The caller [`parse_grid_template_areas`] guarantees that `rows` is nonempty;
/// otherwise it returns `None` before calling this function.
fn build_grid_template_areas(
    rows: Vec<Vec<Option<SmolStr>>>,
    row_strings: Vec<SmolStr>,
) -> Option<GridTemplateAreas> {
    let row_count = rows.len();
    let column_count = rows[0].len();
    // The spec says (verbatim): "All strings must define the same number
    // of cell tokens ... and at least one cell token, or else the declaration is
    // invalid."
    if column_count == 0 || rows.iter().any(|r| r.len() != column_count) {
        return None;
    }
    // (name, row_min, row_max, col_min, col_max), in first-seen order.
    // Use a linear scan because a realistic grid has few distinct area names;
    // there is no need for a HashMap.
    let mut bounds: Vec<(SmolStr, usize, usize, usize, usize)> = Vec::new();
    for (r, row) in rows.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            let Some(name) = cell else { continue };
            match bounds.iter_mut().find(|(n, ..)| n == name) {
                Some((_, r0, r1, c0, c1)) => {
                    *r0 = (*r0).min(r);
                    *r1 = (*r1).max(r);
                    *c0 = (*c0).min(c);
                    *c1 = (*c1).max(c);
                }
                None => bounds.push((name.clone(), r, r, c, c)),
            }
        }
    }
    let mut areas = Vec::with_capacity(bounds.len());
    for (name, r0, r1, c0, c1) in bounds {
        let is_filled_rectangle = rows[r0..=r1]
            .iter()
            .all(|row| row[c0..=c1].iter().all(|cell| cell.as_ref() == Some(&name)));
        if !is_filled_rectangle {
            return None;
        }
        areas.push(GridTemplateAreaEntry {
            name,
            row_start: r0 as u32 + 1,
            row_end: r1 as u32 + 2,
            column_start: c0 as u32 + 1,
            column_end: c1 as u32 + 2,
        });
    }
    Some(GridTemplateAreas {
        row_strings,
        areas,
        row_count: row_count as u32,
        column_count: column_count as u32,
    })
}

/// Parses `grid-template-areas: none | <string>+` (CSS Grid Layout
/// Module Level 1 §7.3
/// <https://www.w3.org/TR/css-grid-1/#grid-template-areas-property>);
/// see the [`GridTemplateAreasValue`] docs.
pub(crate) fn parse_grid_template_areas(
    input: &mut Parser<'_, '_>,
) -> Option<GridTemplateAreasValue> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(GridTemplateAreasValue::None);
    }
    let mut row_strings: Vec<SmolStr> = Vec::new();
    let mut rows: Vec<Vec<Option<SmolStr>>> = Vec::new();
    while let Ok(s) = input.try_parse(|i| i.expect_string().map(|s| SmolStr::new(s.as_ref()))) {
        let tokens = tokenize_grid_area_row(&s)?;
        row_strings.push(s);
        rows.push(tokens);
    }
    if rows.is_empty() {
        return None;
    }
    build_grid_template_areas(rows, row_strings)
        .map(|areas| GridTemplateAreasValue::Areas(Arc::new(areas)))
}

/// Parses the shorthand `place-items: <'align-items'> <'justify-items'>?`
/// (CSS Box Alignment Module Level 3 §7.3
/// <https://www.w3.org/TR/css-align-3/#propdef-place-items>). When the second
/// component is omitted, the first is copied unchanged, as the spec requires;
/// see the [`PlaceItemsShorthand`] docs.
pub(crate) fn parse_place_items_shorthand(
    input: &mut Parser<'_, '_>,
) -> Option<PlaceItemsShorthand> {
    let align = parse_self_alignment(input)?;
    let justify = input.try_parse(parse_self_alignment_res).unwrap_or(align);
    Some(PlaceItemsShorthand { align, justify })
}

/// Parses the shorthand `place-self: <'align-self'> <'justify-self'>?`
/// (CSS Box Alignment Module Level 3 §6.3
/// <https://www.w3.org/TR/css-align-3/#propdef-place-self>). When the second
/// component is omitted, the copy rule matches [`parse_place_items_shorthand`].
pub(crate) fn parse_place_self_shorthand(input: &mut Parser<'_, '_>) -> Option<PlaceSelfShorthand> {
    let align = parse_align_self(input)?;
    let justify = input.try_parse(parse_align_self_res).unwrap_or(align);
    Some(PlaceSelfShorthand { align, justify })
}

/// Parses `float: <ident>` (CSS2 §9.5.1
/// <https://www.w3.org/TR/CSS2/visuren.html#propdef-float>; see the
/// [`FloatValue`] docs).
///
/// Value grammar: `left | right | none`. This parser does not support
/// `inherit` because of the "CSS-wide keyword (canonical)" policy described
/// above. ASCII case-insensitive matching follows sibling `parse_word_break`.
pub(super) fn parse_float(input: &mut Parser<'_, '_>) -> Option<FloatValue> {
    FloatValue::from_css_ident(input.expect_ident().ok()?)
}

/// Parses `clear: <ident>` (CSS2 §9.5.2
/// <https://www.w3.org/TR/CSS2/visuren.html#propdef-clear>; see the
/// [`ClearValue`] docs).
///
/// Value grammar: `none | left | right | both`. This parser does not support
/// `inherit` because of the "CSS-wide keyword (canonical)" policy described
/// above. ASCII case-insensitive matching follows sibling `parse_float`.
pub(super) fn parse_clear(input: &mut Parser<'_, '_>) -> Option<ClearValue> {
    ClearValue::from_css_ident(input.expect_ident().ok()?)
}

/// Parses `table-layout: <ident>` (CSS Tables 3 §4
/// <https://www.w3.org/TR/css-tables-3/#table-layout-property>); see the
/// [`TableLayoutValue`] docs.
///
/// Value grammar: `auto | fixed`. ASCII case-insensitive matching follows
/// sibling [`parse_unicode_bidi`](super::text::parse_unicode_bidi). The caller's `expect_exhausted`
/// (`rule.rs::DeclParser`) drops any extra tokens, such as
/// `table-layout: auto fixed`.
pub(super) fn parse_table_layout(input: &mut Parser<'_, '_>) -> Option<TableLayoutValue> {
    TableLayoutValue::from_css_ident(input.expect_ident().ok()?)
}

/// Parses `border-collapse: <ident>` (CSS Tables 3 §6
/// <https://www.w3.org/TR/css-tables-3/#border-collapse-property>); see the
/// [`BorderCollapseValue`] docs.
///
/// Value grammar: `collapse | separate`. Matching follows the same rules as
/// sibling [`parse_table_layout`].
pub(super) fn parse_border_collapse(input: &mut Parser<'_, '_>) -> Option<BorderCollapseValue> {
    BorderCollapseValue::from_css_ident(input.expect_ident().ok()?)
}

/// Parses `border-spacing: <length>{1,2}` (CSS Tables 3 §6.1
/// <https://www.w3.org/TR/css-tables-3/#border-spacing-property>); see the
/// [`BorderSpacingValue`] docs.
///
/// Each component uses [`parse_non_negative_length`] (`<length [0,∞]>`),
/// with no `<percentage>` alternative. This enforces both the spec's
/// "Percentages: N/A" and "Negative lengths are illegal" requirements.
/// Unitless `0` is accepted as `Px(0.0)` under the CSS Values 3 §5
/// unitless-zero clause in [`parse_length_value`] (the WPT computed case
/// `"0"` → `"0px"`). When the second component is omitted, the first is
/// copied, as required by the spec and as in [`GapShorthand`].
/// The caller's `expect_exhausted` (`rule.rs::DeclParser`) or a component's
/// `None` result drops declarations with three or more components, a bare
/// non-zero number, or `%`.
pub(super) fn parse_border_spacing(input: &mut Parser<'_, '_>) -> Option<BorderSpacingValue> {
    let horizontal = parse_non_negative_length(input)?;
    let vertical = input
        .try_parse(|i| parse_non_negative_length(i).ok_or(()))
        .unwrap_or(horizontal);
    Some(BorderSpacingValue {
        horizontal,
        vertical,
    })
}

/// Parses `caption-side: <ident>` (CSS Tables 3 §7
/// <https://www.w3.org/TR/css-tables-3/#caption-side-property>); see the
/// [`CaptionSideValue`] docs.
///
/// Value grammar: `top | bottom`. ASCII case-insensitive matching follows
/// sibling [`parse_table_layout`]. The caller's `expect_exhausted`
/// (`rule.rs::DeclParser`) drops extra tokens such as
/// `caption-side: top bottom`.
pub(super) fn parse_caption_side(input: &mut Parser<'_, '_>) -> Option<CaptionSideValue> {
    CaptionSideValue::from_css_ident(input.expect_ident().ok()?)
}

/// Parses `empty-cells: <ident>` (CSS Tables 3 §8
/// <https://www.w3.org/TR/css-tables-3/#empty-cells-property>); see the
/// [`EmptyCellsValue`] docs.
///
/// Value grammar: `show | hide`. Matching follows the same rules as sibling
/// [`parse_caption_side`].
pub(super) fn parse_empty_cells(input: &mut Parser<'_, '_>) -> Option<EmptyCellsValue> {
    EmptyCellsValue::from_css_ident(input.expect_ident().ok()?)
}

pub(super) fn parse_display(input: &mut Parser<'_, '_>) -> Option<DisplayValue> {
    DisplayValue::from_css_ident(input.expect_ident().ok()?)
}

pub(super) fn parse_list_style_image(input: &mut Parser<'_, '_>) -> Option<BackgroundImage> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(BackgroundImage::None);
    }
    let url = input.expect_url().ok()?;
    Some(BackgroundImage::Url(url.as_ref().to_string()))
}

/// Parse `list-style-type`'s `<counter-style-name> | <string>` grammar.
///
/// The built-in names are intentionally not enumerated here: CSS Lists 3
/// permits author-defined counter styles, so an otherwise valid identifier is
/// preserved for the marker resolver. Reserved CSS-wide keywords are rejected
/// as values rather than accidentally becoming custom counter-style names.
pub(super) fn parse_list_style_type(input: &mut Parser<'_, '_>) -> Option<ListStyleType> {
    if let Ok(value) = input.try_parse(|i| i.expect_string_cloned()) {
        return Some(ListStyleType::String(SmolStr::new(value.as_ref())));
    }
    let ident = parse_custom_ident(input)?;
    match ident.to_ascii_lowercase().as_str() {
        "disc" => Some(ListStyleType::Disc),
        "none" => Some(ListStyleType::None),
        _ => Some(ListStyleType::Named(ident)),
    }
}

/// Parse `list-style-position: inside | outside`.
pub(super) fn parse_list_style_position(input: &mut Parser<'_, '_>) -> Option<ListStylePosition> {
    ListStylePosition::from_css_ident(input.expect_ident().ok()?)
}

/// Parses the value of `orphans` / `widows: <integer>` (CSS Fragmentation
/// Module Level 3 §3.3 "Breaks Between Lines: orphans, widows"
/// <https://www.w3.org/TR/css-break-3/#widows-orphans>).
///
/// Calls `expect_integer` directly, like [`parse_z_index`](super::box_model::parse_z_index) and
/// [`parse_counter_property`](super::content::parse_counter_property). Unlike those siblings, these two
/// properties restrict `<integer>` to **positive values**. The spec says:
/// "Only positive integers are allowed as values of orphans and widows.
/// Negative values and zero are invalid and must cause the declaration to be
/// ignored." Values of zero or less therefore return `None` and drop the
/// whole declaration, as with other out-of-range `<integer>`/`<length>` values.
pub(super) fn parse_positive_integer(input: &mut Parser<'_, '_>) -> Option<i32> {
    let value = input.try_parse(|i| i.expect_integer()).ok()?;
    if value > 0 { Some(value) } else { None }
}

/// `column-count: auto | <integer [1,∞]>`.
pub(super) fn parse_column_count(input: &mut Parser<'_, '_>) -> Option<ColumnCountValue> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(ColumnCountValue::Auto);
    }
    let count = parse_positive_integer(input)?;
    Some(ColumnCountValue::Count(u32::try_from(count).ok()?))
}

/// `column-width: auto | <length [0,∞]>`.
pub(super) fn parse_column_width(input: &mut Parser<'_, '_>) -> Option<ColumnWidthValue> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(ColumnWidthValue::Auto);
    }
    Some(ColumnWidthValue::Length(parse_non_negative_length(input)?))
}

pub(super) fn parse_columns_shorthand(input: &mut Parser<'_, '_>) -> Option<ColumnsShorthand> {
    // The grammar is an unordered pair (`||`).  Parsing in one fixed order
    // is not enough because `auto` is valid for both components: `auto 3`
    // and `auto 200px` need opposite interpretations of the first token.
    // Each complete candidate is tried transactionally and must consume the
    // whole value, so duplicate components and trailing garbage are rejected.
    if let Ok(value) = input.try_parse(|i| {
        let width = parse_column_width(i).ok_or_else(|| i.new_custom_error::<(), ()>(()))?;
        let has_second = !i.is_exhausted();
        let count = if has_second {
            parse_column_count(i).ok_or_else(|| i.new_custom_error::<(), ()>(()))?
        } else {
            ColumnCountValue::Auto
        };
        if !i.is_exhausted() {
            return Err(i.new_custom_error::<(), ()>(()));
        }
        Ok(ColumnsShorthand { count, width })
    }) {
        return Some(value);
    }

    input
        .try_parse(|i| {
            let count = parse_column_count(i).ok_or_else(|| i.new_custom_error::<(), ()>(()))?;
            let has_second = !i.is_exhausted();
            let width = if has_second {
                parse_column_width(i).ok_or_else(|| i.new_custom_error::<(), ()>(()))?
            } else {
                ColumnWidthValue::Auto
            };
            if !i.is_exhausted() {
                return Err(i.new_custom_error::<(), ()>(()));
            }
            Ok(ColumnsShorthand { count, width })
        })
        .ok()
}

/// Parses `break-before: <ident>` / `break-after: <ident>` (CSS
/// Fragmentation Module Level 3 §3.1
/// <https://www.w3.org/TR/css-break-3/#break-between>).
///
/// This crate accepts four keywords within its scope (see "Scope carving"
/// in the [`BreakBetween`] docs): `auto`, `avoid`, `avoid-page`, and `page`.
/// The eight remaining keywords in the property definition (`left`, `right`,
/// `recto`, `verso`, `avoid-column`, `column`, `avoid-region`, `region`), along
/// with `always` and `all` (absent from the current spec grammar), silently
/// drop with `None` like other unknown idents. Idents are compared ASCII
/// case-insensitively, as in sibling
/// [`parse_word_break`](super::text::parse_word_break).
pub(super) fn parse_break_between(input: &mut Parser<'_, '_>) -> Option<BreakBetween> {
    BreakBetween::from_css_ident(input.expect_ident().ok()?)
}

/// Parses `break-inside: <ident>` (CSS Fragmentation Module Level 3
/// §3.2 <https://www.w3.org/TR/css-break-3/#break-within>).
///
/// This crate accepts three keywords within its scope (see "Scope carving"
/// in the [`BreakInside`] docs): `auto`, `avoid`, and `avoid-page`.
/// The other two keywords in the property definition, `avoid-column` and
/// `avoid-region`, silently drop with `None` like other unknown idents.
/// Idents are compared ASCII case-insensitively, as in [`parse_break_between`].
pub(super) fn parse_break_inside(input: &mut Parser<'_, '_>) -> Option<BreakInside> {
    BreakInside::from_css_ident(input.expect_ident().ok()?)
}

/// Parses `page-break-before: <ident>` / `page-break-after: <ident>`,
/// the CSS2.1 legacy shorthands for `break-before` / `break-after`. These
/// map to [`BreakBetween`] (CSS Fragmentation Module Level 3 §3.4
/// <https://www.w3.org/TR/css-break-3/#page-break-properties>; see the mapping
/// table under "legacy shorthand" in the [`BreakBetween`] docs).
///
/// CSS2.1 defines the `page-break-before` / `page-break-after` grammar as
/// `auto | always | avoid | left | right` (verbatim,
/// <https://www.w3.org/TR/CSS2/page.html#propdef-page-break-before>).
/// This parser accepts only `auto`, `avoid`, and `always`. Because `left` and
/// `right` would map to values not implemented for `break-before` or
/// `break-after` (see "Scope carving" in the [`BreakBetween`] docs), the
/// legacy shorthands do not accept them either. The spec's mapping table
/// maps `always` to [`BreakBetween::Page`], rather than leaving it unchanged;
/// `auto` and `avoid` map to themselves.
pub(super) fn parse_legacy_page_break_between(input: &mut Parser<'_, '_>) -> Option<BreakBetween> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "auto" => Some(BreakBetween::Auto),
        "avoid" => Some(BreakBetween::Avoid),
        "always" => Some(BreakBetween::Page),
        _ => None,
    }
}

/// Parses `page-break-inside: <ident>`, the CSS2.1 legacy shorthand
/// for `break-inside` (CSS Fragmentation Module Level 3 §3.4; see
/// "legacy shorthand" in the [`BreakInside`] docs).
///
/// The CSS2.1 `page-break-inside` grammar is `avoid | auto` (verbatim,
/// <https://www.w3.org/TR/CSS2/page.html#propdef-page-break-inside>).
/// It does not contain `always`, `left`, or `right`. Both accepted keywords
/// map unchanged to [`BreakInside`]. The `avoid-page` value available to
/// `break-inside` is absent from the CSS2.1 `page-break-inside` grammar, so
/// this parser does not accept it.
pub(super) fn parse_legacy_page_break_inside(input: &mut Parser<'_, '_>) -> Option<BreakInside> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "auto" => Some(BreakInside::Auto),
        "avoid" => Some(BreakInside::Avoid),
        _ => None,
    }
}
