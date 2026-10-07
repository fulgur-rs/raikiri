use super::*;
use crate::property::{
    BorderRadius, BorderStyle, BoxShadowItem, CalcLengthPercentage, CssColor, Length,
    LengthPercentageCalc, LetterSpacingValue, Outline, OutlineColor, OutlineStyle, Sides, TabSize,
    TextDecorationThickness, TextShadowColor, TextUnderlineOffset,
};
use crate::specified::SpecifiedValues;

/// Shared context for `rem`, with a 16px root font size.
const CTX: ResolveContext = ResolveContext {
    root_font_size: ComputedLength(INITIAL_FONT_SIZE_PX),
    root_line_height: None,
};

#[test]
fn border_radius_used_values_resolve_percentages_on_each_axis() {
    let radius = ComputedBorderRadius::elliptical(
        [ComputedLengthPercentage::Percent(50.0); 4],
        [ComputedLengthPercentage::Percent(25.0); 4],
    );
    assert_eq!(radius.used(200.0, 100.0), [[100.0, 25.0]; 4]);
}

#[test]
fn border_radius_used_values_scale_both_axes_by_one_edge_ratio() {
    let radius = ComputedBorderRadius::elliptical(
        [ComputedLengthPercentage::Px(80.0); 4],
        [ComputedLengthPercentage::Px(20.0); 4],
    );
    assert_eq!(radius.used(100.0, 100.0), [[50.0, 12.5]; 4]);
    let radius = ComputedBorderRadius::elliptical(
        [ComputedLengthPercentage::Px(10.0); 4],
        [ComputedLengthPercentage::Px(80.0); 4],
    );
    assert_eq!(radius.used(100.0, 100.0), [[6.25, 50.0]; 4]);
}

#[test]
fn border_radius_used_values_make_zero_axis_corners_square() {
    let radius = ComputedBorderRadius::elliptical(
        [
            ComputedLengthPercentage::Px(0.0),
            ComputedLengthPercentage::Px(20.0),
            ComputedLengthPercentage::Px(10.0),
            ComputedLengthPercentage::Px(10.0),
        ],
        [
            ComputedLengthPercentage::Px(20.0),
            ComputedLengthPercentage::Px(0.0),
            ComputedLengthPercentage::Px(10.0),
            ComputedLengthPercentage::Px(10.0),
        ],
    );
    assert_eq!(
        radius.used(100.0, 100.0),
        [[0.0, 0.0], [0.0, 0.0], [10.0, 10.0], [10.0, 10.0]]
    );
    assert_eq!(radius.used(0.0, 0.0), [[0.0, 0.0]; 4]);
}

#[test]
fn border_radius_used_values_sanitize_non_finite_or_negative_inputs() {
    let radius = ComputedBorderRadius::elliptical(
        [
            ComputedLengthPercentage::Px(f32::NAN),
            ComputedLengthPercentage::Px(f32::INFINITY),
            ComputedLengthPercentage::Px(-1.0),
            ComputedLengthPercentage::Px(10.0),
        ],
        [ComputedLengthPercentage::Px(10.0); 4],
    );
    assert_eq!(
        radius.used(100.0, 100.0),
        [[0.0, 0.0], [0.0, 0.0], [0.0, 0.0], [10.0, 10.0]]
    );
    for (width, height) in [(f32::NAN, 100.0), (100.0, f32::INFINITY), (-1.0, 100.0)] {
        assert_eq!(radius.used(width, height), [[0.0, 0.0]; 4]);
    }
}

// -----------------------------------------------------------------
// Basic contracts for ComputedLength / ResolveContext
// -----------------------------------------------------------------

#[test]
fn computed_length_zero_and_px_accessor() {
    assert_eq!(ComputedLength::ZERO, ComputedLength(0.0));
    assert_eq!(ComputedLength(12.5).px(), 12.5);
}

#[test]
fn resolve_context_new_stores_root_font_size() {
    assert_eq!(
        ResolveContext::new(ComputedLength(20.0)).root_font_size,
        ComputedLength(20.0),
    );
}

// -----------------------------------------------------------------
// phase 2: absolutize font-size (relative to the parent)
// -----------------------------------------------------------------

#[test]
fn font_size_px_is_identity() {
    assert_eq!(
        resolve_font_size(Length::Px(18.0), ComputedLength(16.0), None, &CTX),
        ComputedLength(18.0),
    );
}

/// `1pt = 1/72in`, `1in = 96px` → `12pt = 16px`
/// (CSS Values 4 §6.2 <https://www.w3.org/TR/css-values-4/#absolute-lengths>).
#[test]
fn font_size_pt_converts_at_96px_per_inch() {
    assert_eq!(
        resolve_font_size(Length::Pt(12.0), ComputedLength(16.0), None, &CTX),
        ComputedLength(16.0),
    );
}

/// In `font-size`, `em` uses the **parent's** computed font-size as its basis
/// (CSS Values 4 §6.1.1 parent-metrics clause).
#[test]
fn font_size_em_resolves_against_parent_font_size() {
    assert_eq!(
        resolve_font_size(Length::Em(1.5), ComputedLength(16.0), None, &CTX),
        ComputedLength(24.0),
    );
}

/// `em` compounding: 16px → 1.5em → 1.5em = 24px → 36px.
#[test]
fn font_size_em_compounds_across_two_levels() {
    let child = resolve_font_size(Length::Em(1.5), ComputedLength(16.0), None, &CTX);
    let grandchild = resolve_font_size(Length::Em(1.5), child, None, &CTX);
    assert_eq!(child, ComputedLength(24.0));
    assert_eq!(grandchild, ComputedLength(36.0));
}

/// `rem` uses the root element's computed font-size as its basis
/// (CSS Values 4 §6.1.1 <https://www.w3.org/TR/css-values-4/#rem>).
/// It **does not depend** on the parent's font-size.
#[test]
fn font_size_rem_resolves_against_root_font_size() {
    let ctx = ResolveContext::new(ComputedLength(20.0));
    assert_eq!(
        resolve_font_size(Length::Rem(2.0), ComputedLength(64.0), None, &ctx),
        ComputedLength(40.0),
    );
}

/// `font-size: Nrem` on the root element itself uses the initial value (16px)
/// as its basis (CSS Values 4 §6.1.1 parent-metrics clause —
/// the root element has no parent, so it uses initial values).
#[test]
fn font_size_rem_on_root_element_uses_initial_font_size() {
    let ctx = ResolveContext::initial();
    assert_eq!(
        resolve_font_size(
            Length::Rem(2.0),
            ComputedLength(INITIAL_FONT_SIZE_PX),
            None,
            &ctx
        ),
        ComputedLength(32.0),
    );
}

/// In `font-size`, `<percentage>` **becomes a length** based on the parent's font-size
/// (CSS Fonts 4 `font-size` propdef "Percentages: refer to parent element's
/// font size" — an explicit exception in CSS Values 4 §5.5.1).
#[test]
fn font_size_percent_resolves_against_parent_font_size() {
    assert_eq!(
        resolve_font_size(Length::Percent(150.0), ComputedLength(16.0), None, &CTX),
        ComputedLength(24.0),
    );
}

/// `ex` / `ch` always use the spec's unknown-metric fallback,
/// `0.5em`, because the style layer has no real font metrics (`Length::Ex` / `Length::Ch`
/// docs). In `font-size` they use the **parent's** basis, like other font-relative units,
/// avoiding self-reference.
#[test]
fn font_size_ex_and_ch_resolve_against_parent_font_size_with_half_em_fallback() {
    assert_eq!(
        resolve_font_size(Length::Ex(2.0), ComputedLength(16.0), None, &CTX),
        ComputedLength(16.0), // 2 * 0.5 * 16
    );
    assert_eq!(
        resolve_font_size(Length::Ch(2.0), ComputedLength(16.0), None, &CTX),
        ComputedLength(16.0),
    );
}

/// The unknown-metric fallback for `ic` is `1em` (`Length::Ic` docs).
#[test]
fn font_size_ic_resolves_against_parent_font_size_with_one_em_fallback() {
    assert_eq!(
        resolve_font_size(Length::Ic(1.5), ComputedLength(16.0), None, &CTX),
        ComputedLength(24.0),
    );
}

/// `rex` / `rch` / `ric` use the root element as their basis (like `rem`,
/// independent of the parent's font-size).
#[test]
fn font_size_r_prefixed_font_relative_units_resolve_against_root_font_size() {
    let ctx = ResolveContext::new(ComputedLength(20.0));
    assert_eq!(
        resolve_font_size(Length::Rex(2.0), ComputedLength(64.0), None, &ctx),
        ComputedLength(20.0), // 2 * 0.5 * 20 (ignore the parent's 64px)
    );
    assert_eq!(
        resolve_font_size(Length::Rch(2.0), ComputedLength(64.0), None, &ctx),
        ComputedLength(20.0),
    );
    assert_eq!(
        resolve_font_size(Length::Ric(2.0), ComputedLength(64.0), None, &ctx),
        ComputedLength(40.0),
    );
}

/// `font-size: 1lh` resolves against the **parent's** used line-height
/// (CSS Values 4 §6.1.1's self-reference clause,
/// same reasoning as `resolve_line_height`'s `Length::Lh` arm). The `parent`
/// argument to `resolve_font_size` (16px, unrelated) is deliberately
/// different from `parent_line_height_basis` (30px) so a bug that
/// conflates "parent's font-size" with "parent's line-height" would be
/// caught.
#[test]
fn font_size_lh_resolves_against_parent_line_height_basis() {
    assert_eq!(
        resolve_font_size(
            Length::Lh(2.0),
            ComputedLength(16.0),
            Some(ComputedLength(30.0)),
            &CTX,
        ),
        ComputedLength(60.0), // 2 * 30
    );
}

/// `font-size: 1rlh` resolves against `ctx.root_line_height` — a
/// tree-global constant, **not** `parent_line_height_basis` (mirrors
/// `resolve_line_height`'s `Length::Rlh`
/// arm and its "not self-referential for non-root elements" rationale).
/// `parent_line_height_basis` is deliberately set to a different value
/// (30px) than `ctx.root_line_height` (50px) so a bug that swaps the two
/// bases would be caught.
#[test]
fn font_size_rlh_resolves_against_root_line_height_not_parent() {
    let ctx =
        ResolveContext::with_root_line_height(ComputedLength(16.0), Some(ComputedLength(50.0)));
    assert_eq!(
        resolve_font_size(
            Length::Rlh(2.0),
            ComputedLength(16.0),
            Some(ComputedLength(30.0)), // must NOT be read by the `Rlh` arm
            &ctx,
        ),
        ComputedLength(100.0), // 2 * 50, not 2 * 30
    );
}

/// `font-size: 1lh` / `1rlh` fall back to `font-size`'s own spec initial
/// (`medium` = `INITIAL_FONT_SIZE_PX`) when their basis is unresolvable
/// (`normal` with no font metrics — the same wall as `cap`/`rcap`, or a
/// root element with no parent) — **not** `0px`. Unlike the generic,
/// multi-property `resolve_length`/`resolve_length_percentage` (whose
/// flat `0px` fallback is a known compromise for `border-width`, tracked
/// separately), `resolve_font_size` is a
/// dedicated single-property resolver and can fall back to its own true
/// initial directly — same convention as `resolve_line_height`'s `Lh`
/// arm falling back to `line-height`'s own initial `normal`.
#[test]
fn font_size_lh_and_rlh_fall_back_to_initial_when_basis_is_unresolved() {
    assert_eq!(
        resolve_font_size(Length::Lh(2.0), ComputedLength(20.0), None, &CTX),
        ComputedLength(INITIAL_FONT_SIZE_PX),
    );
    assert_eq!(
        resolve_font_size(Length::Rlh(2.0), ComputedLength(20.0), None, &CTX),
        ComputedLength(INITIAL_FONT_SIZE_PX),
    );
}

/// CSS Values 4 §6.2 "Absolute Lengths" conversion table, verbatim: `1in = 96px` /
/// `1cm = 96px/2.54` / `1mm = 1/10th of 1cm` / `1Q = 1/40th of 1cm` /
/// `1pc = 1/6th of 1in`. The expected values use the spec's expressions rather
/// than decimal literals: `96.0/2.54` has no exact decimal representation, so
/// matching the implementation's evaluation order (the `cm_to_px` / `pc_to_px`
/// chain) also matches f32 rounding bit for bit.
#[test]
fn font_size_additional_absolute_units_convert_per_spec_table() {
    assert_eq!(
        resolve_font_size(Length::In(1.0), ComputedLength(16.0), None, &CTX),
        ComputedLength(96.0),
    );
    assert_eq!(
        resolve_font_size(Length::Cm(1.0), ComputedLength(16.0), None, &CTX),
        ComputedLength(96.0 / 2.54),
    );
    assert_eq!(
        resolve_font_size(Length::Mm(1.0), ComputedLength(16.0), None, &CTX),
        ComputedLength(96.0 / 2.54 / 10.0),
    );
    assert_eq!(
        resolve_font_size(Length::Q(1.0), ComputedLength(16.0), None, &CTX),
        ComputedLength(96.0 / 2.54 / 40.0),
    );
    assert_eq!(
        resolve_font_size(Length::Pc(1.0), ComputedLength(16.0), None, &CTX),
        ComputedLength(96.0 / 6.0),
    );
}

// -----------------------------------------------------------------
// phase 3: absolutize `<length>` (border-width) using this node's basis
// -----------------------------------------------------------------

#[test]
fn length_px_and_pt_are_absolute() {
    assert_eq!(
        resolve_length(Length::Px(3.0), ComputedLength(16.0), None, &CTX),
        ComputedLength(3.0),
    );
    assert_eq!(
        resolve_length(Length::Pt(9.0), ComputedLength(16.0), None, &CTX),
        ComputedLength(12.0),
    );
}

/// On properties other than `font-size`, `em` uses the **element's own** computed
/// font-size as its basis (CSS Values 4 §6.1.1 <https://www.w3.org/TR/css-values-4/#em>).
#[test]
fn length_em_resolves_against_own_font_size() {
    assert_eq!(
        resolve_length(Length::Em(2.0), ComputedLength(20.0), None, &CTX),
        ComputedLength(40.0),
    );
}

#[test]
fn length_rem_resolves_against_root_font_size() {
    let ctx = ResolveContext::new(ComputedLength(10.0));
    assert_eq!(
        resolve_length(Length::Rem(2.5), ComputedLength(64.0), None, &ctx),
        ComputedLength(25.0),
    );
}

/// The `border-*-width` grammar (`<line-width>`) excludes `<percentage>`,
/// so parsing drops it. This only checks that the unreachable arm is total.
#[test]
fn length_percent_is_grammar_unreachable_and_falls_to_zero() {
    assert_eq!(
        resolve_length(Length::Percent(50.0), ComputedLength(20.0), None, &CTX),
        ComputedLength::ZERO,
    );
}

/// For properties other than `font-size` (the `resolve_length` caller `border-*-width`
/// and the `line-height` `<length>` part), `ex` / `ch` / `ic` use
/// the **element's own** computed font-size. This checks the `resolve_font_size`
/// unit tests' parent basis against the own basis here (see
/// the parent-metrics clause in the `Length::Ex` docs).
#[test]
fn length_ex_ch_ic_resolve_against_own_font_size() {
    assert_eq!(
        resolve_length(Length::Ex(2.0), ComputedLength(20.0), None, &CTX),
        ComputedLength(20.0), // 2 * 0.5 * 20
    );
    assert_eq!(
        resolve_length(Length::Ch(2.0), ComputedLength(20.0), None, &CTX),
        ComputedLength(20.0),
    );
    assert_eq!(
        resolve_length(Length::Ic(2.0), ComputedLength(20.0), None, &CTX),
        ComputedLength(40.0),
    );
}

#[test]
fn length_or_normal_with_ch_retains_authored_factor() {
    let resolved = resolve_length_or_normal_with_ch(
        LengthOrNormal::Length(Length::Ch(2.0)),
        ComputedLength(20.0),
        None,
        &CTX,
    );
    assert_eq!(resolved.value, ComputedLength(20.0));
    assert_eq!(resolved.ch_factor, Some(2.0));

    let normal =
        resolve_length_or_normal_with_ch(LengthOrNormal::Normal, ComputedLength(20.0), None, &CTX);
    assert_eq!(normal.value, ComputedLength::ZERO);
    assert_eq!(normal.ch_factor, None);
}

#[test]
fn letter_spacing_computed_value_preserves_percentages_and_calcs() {
    let font_size = ComputedLength(40.0);
    assert_eq!(
        resolve_letter_spacing(LetterSpacingValue::Normal, font_size, None, &CTX),
        ComputedLetterSpacing::Px(0.0),
    );
    assert_eq!(
        resolve_letter_spacing(
            LetterSpacingValue::Length(Length::Percent(110.0)),
            font_size,
            None,
            &CTX,
        ),
        ComputedLetterSpacing::Percent(110.0),
    );
    assert_eq!(
        resolve_letter_spacing(
            LetterSpacingValue::Calc(LengthPercentageCalc {
                percent: 0.0,
                px: 10.0,
                em: -0.5,
                ch: 0.0,
            }),
            font_size,
            None,
            &CTX,
        ),
        ComputedLetterSpacing::Px(-10.0),
    );
    assert_eq!(
        resolve_letter_spacing(
            LetterSpacingValue::Calc(LengthPercentageCalc {
                percent: -15.0,
                px: 10.0,
                em: 0.0,
                ch: 0.0,
            }),
            font_size,
            None,
            &CTX,
        ),
        ComputedLetterSpacing::Calc(CalcLengthPercentage {
            percent: -15.0,
            px: 10.0,
        }),
    );
}

#[test]
fn letter_spacing_layout_fallback_remains_zero_for_deferred_values() {
    let font_size = ComputedLength(40.0);
    for specified in [
        LetterSpacingValue::Length(Length::Percent(110.0)),
        LetterSpacingValue::Calc(LengthPercentageCalc {
            percent: -15.0,
            px: 10.0,
            em: 0.0,
            ch: 0.0,
        }),
    ] {
        assert_eq!(
            resolve_letter_spacing_with_ch(specified, font_size, None, &CTX).value,
            ComputedLength::ZERO,
        );
    }
}

#[test]
fn letter_spacing_lift_preserves_computed_form() {
    let computed = ComputedLetterSpacing::Calc(CalcLengthPercentage {
        percent: -15.0,
        px: 10.0,
    });
    let lifted = lift_letter_spacing(computed);
    assert_eq!(
        lifted,
        LetterSpacingValue::Calc(LengthPercentageCalc {
            percent: -15.0,
            px: 10.0,
            em: 0.0,
            ch: 0.0,
        }),
    );
    assert_eq!(
        resolve_letter_spacing(lifted, ComputedLength(12.0), None, &CTX),
        computed,
    );
}

#[test]
fn tab_size_calc_resolves_em_against_font_size() {
    // Generic `em` math (not fixture-specific): `px` + `em * font-size`.
    let font_size = ComputedLength(20.0);
    assert_eq!(
        resolve_tab_size(
            TabSize::Calc(LengthPercentageCalc {
                percent: 0.0,
                px: 10.0,
                em: 0.5,
                ch: 0.0,
            }),
            font_size,
            None,
            &CTX,
        ),
        ComputedTabSize::Length(ComputedLength(20.0)),
    );
}

#[test]
fn tab_size_calc_clamps_negative_derived_to_zero() {
    // CSS Values 4 §10.7: derived negative lengths clamp to `0`
    // (spec `[0,∞]` — authored negatives are still parse-rejected).
    let font_size = ComputedLength(20.0);
    assert_eq!(
        resolve_tab_size(
            TabSize::Calc(LengthPercentageCalc {
                percent: 0.0,
                px: 10.0,
                em: -1.0,
                ch: 0.0,
            }),
            font_size,
            None,
            &CTX,
        ),
        ComputedTabSize::Length(ComputedLength(0.0)),
    );
}

#[test]
fn length_percentage_with_ch_retains_authored_factor() {
    let resolved =
        resolve_length_percentage_with_ch(Length::Ch(2.0), ComputedLength(20.0), None, &CTX);
    assert_eq!(resolved.value, ComputedLengthPercentage::Px(20.0),);
    assert_eq!(resolved.ch_factor, Some(2.0));

    let percentage =
        resolve_length_percentage_with_ch(Length::Percent(25.0), ComputedLength(20.0), None, &CTX);
    assert_eq!(percentage.value, ComputedLengthPercentage::Percent(25.0),);
    assert_eq!(percentage.ch_factor, None);
}

#[test]
fn length_r_prefixed_font_relative_units_resolve_against_root_font_size() {
    let ctx = ResolveContext::new(ComputedLength(10.0));
    assert_eq!(
        resolve_length(Length::Rex(2.5), ComputedLength(64.0), None, &ctx),
        ComputedLength(12.5), // 2.5 * 0.5 * 10 (ignore own font-size of 64px)
    );
    assert_eq!(
        resolve_length(Length::Ric(2.5), ComputedLength(64.0), None, &ctx),
        ComputedLength(25.0),
    );
}

/// CSS Values 4 §6.2 conversion table — also check `border-*-width` via `resolve_length`
/// against the same conversion in `resolve_font_size`
/// (`width: 1in` → 96px, a verification target in the issue).
#[test]
fn length_additional_absolute_units_convert_per_spec_table() {
    assert_eq!(
        resolve_length(Length::In(1.0), ComputedLength(20.0), None, &CTX),
        ComputedLength(96.0),
    );
    assert_eq!(
        resolve_length(Length::Pc(1.0), ComputedLength(20.0), None, &CTX),
        ComputedLength(96.0 / 6.0),
    );
    assert_eq!(
        resolve_length(Length::Mm(1.0), ComputedLength(20.0), None, &CTX),
        ComputedLength(96.0 / 2.54 / 10.0),
    );
    assert_eq!(
        resolve_length(Length::Q(1.0), ComputedLength(20.0), None, &CTX),
        ComputedLength(96.0 / 2.54 / 40.0),
    );
    assert_eq!(
        resolve_length(Length::Cm(1.0), ComputedLength(20.0), None, &CTX),
        ComputedLength(96.0 / 2.54),
    );
}

// -----------------------------------------------------------------
// phase 3: absolutize `<length-percentage>` (padding)
// -----------------------------------------------------------------

#[test]
fn length_percentage_absolutizes_lengths() {
    let fs = ComputedLength(20.0);
    assert_eq!(
        resolve_length_percentage(Length::Px(10.0), fs, None, &CTX),
        ComputedLengthPercentage::Px(10.0),
    );
    assert_eq!(
        resolve_length_percentage(Length::Pt(6.0), fs, None, &CTX),
        ComputedLengthPercentage::Px(8.0),
    );
    assert_eq!(
        resolve_length_percentage(Length::Em(2.0), fs, None, &CTX),
        ComputedLengthPercentage::Px(40.0),
    );
    assert_eq!(
        resolve_length_percentage(Length::Rem(0.5), fs, None, &CTX),
        ComputedLengthPercentage::Px(8.0),
    );
}

/// Exercise every newly added unit directly in `resolve_length_percentage` (`padding-*`
/// absolutization),
/// independently of the unit tests for `resolve_font_size` / `resolve_length`:
/// all three functions have independent match arms, so patch coverage must be
/// checked separately for each function.
#[test]
fn length_percentage_absolutizes_additional_units() {
    let fs = ComputedLength(20.0);
    assert_eq!(
        resolve_length_percentage(Length::Ex(2.0), fs, None, &CTX),
        ComputedLengthPercentage::Px(20.0), // 2 * 0.5 * 20
    );
    assert_eq!(
        resolve_length_percentage(Length::Ch(2.0), fs, None, &CTX),
        ComputedLengthPercentage::Px(20.0),
    );
    assert_eq!(
        resolve_length_percentage(Length::Ic(1.5), fs, None, &CTX),
        ComputedLengthPercentage::Px(30.0),
    );
    let ctx = ResolveContext::new(ComputedLength(10.0));
    assert_eq!(
        resolve_length_percentage(Length::Rex(2.0), fs, None, &ctx),
        ComputedLengthPercentage::Px(10.0), // 2 * 0.5 * 10 (ignore own 20px)
    );
    assert_eq!(
        resolve_length_percentage(Length::Rch(2.0), fs, None, &ctx),
        ComputedLengthPercentage::Px(10.0),
    );
    assert_eq!(
        resolve_length_percentage(Length::Ric(2.0), fs, None, &ctx),
        ComputedLengthPercentage::Px(20.0),
    );
    assert_eq!(
        resolve_length_percentage(Length::Cm(1.0), fs, None, &CTX),
        ComputedLengthPercentage::Px(96.0 / 2.54),
    );
    assert_eq!(
        resolve_length_percentage(Length::Mm(1.0), fs, None, &CTX),
        ComputedLengthPercentage::Px(96.0 / 2.54 / 10.0),
    );
    assert_eq!(
        resolve_length_percentage(Length::Q(1.0), fs, None, &CTX),
        ComputedLengthPercentage::Px(96.0 / 2.54 / 40.0),
    );
    assert_eq!(
        resolve_length_percentage(Length::In(1.0), fs, None, &CTX),
        ComputedLengthPercentage::Px(96.0),
    );
    assert_eq!(
        resolve_length_percentage(Length::Pc(1.0), fs, None, &CTX),
        ComputedLengthPercentage::Px(96.0 / 6.0),
    );
}

/// `padding: 1lh` — use resolved own line-height (`Some`) as the multiplier.
#[test]
fn length_percentage_lh_multiplies_own_line_height_basis() {
    let fs = ComputedLength(20.0);
    assert_eq!(
        resolve_length_percentage(Length::Lh(1.5), fs, Some(ComputedLength(24.0)), &CTX),
        ComputedLengthPercentage::Px(36.0), // 1.5 * 24
    );
}

/// `padding: 1lh` — when own line-height is `normal` and unresolved (`None`),
/// fall back to padding's spec initial value `0` (an independent implementation:
/// do not invent a ratio; see the `resolve_length_percentage` docs).
#[test]
fn length_percentage_lh_falls_back_to_zero_when_unresolvable() {
    let fs = ComputedLength(20.0);
    assert_eq!(
        resolve_length_percentage(Length::Lh(1.5), fs, None, &CTX),
        ComputedLengthPercentage::Px(0.0),
    );
}

/// `padding: 1rlh` — use tree-global `ctx.root_line_height` as the basis,
/// not own line-height.
#[test]
fn length_percentage_rlh_multiplies_root_line_height_basis() {
    let fs = ComputedLength(20.0);
    let ctx = ResolveContext::with_root_line_height(
        ComputedLength(16.0),
        Some(ComputedLength(19.2)), // root: known px, not equivalent to line-height: normal
    );
    assert_eq!(
        resolve_length_percentage(Length::Rlh(2.0), fs, Some(ComputedLength(999.0)), &ctx),
        ComputedLengthPercentage::Px(38.4), // ignore own_line_height (999); use only root
    );
}

/// Percentages in `<length-percentage>` **remain percentages** in the computed
/// layer (CSS Values 4 §5.5.1 / CSS Box 3 `padding-top` "Computed value: a
/// computed `<length-percentage>` value"). Keep the authored number unchanged;
/// do not divide by `/ 100`.
#[test]
fn length_percentage_percent_passes_through_unchanged() {
    assert_eq!(
        resolve_length_percentage(Length::Percent(50.0), ComputedLength(20.0), None, &CTX),
        ComputedLengthPercentage::Percent(50.0),
    );
}

// -----------------------------------------------------------------
// phase 3: `<length-percentage> | auto` (margin / width / height)
// -----------------------------------------------------------------

#[test]
fn length_percentage_or_auto_keeps_auto() {
    assert_eq!(
        resolve_length_percentage_or_auto(LengthOrAuto::Auto, ComputedLength(16.0), None, &CTX),
        ComputedLengthPercentageOrAuto::Auto,
    );
}

#[test]
fn length_percentage_or_auto_absolutizes_and_passes_percent() {
    let fs = ComputedLength(20.0);
    assert_eq!(
        resolve_length_percentage_or_auto(LengthOrAuto::Length(Length::Em(1.5)), fs, None, &CTX),
        ComputedLengthPercentageOrAuto::Px(30.0),
    );
    assert_eq!(
        resolve_length_percentage_or_auto(
            LengthOrAuto::Length(Length::Percent(25.0)),
            fs,
            None,
            &CTX
        ),
        ComputedLengthPercentageOrAuto::Percent(25.0),
    );
}

/// `width: 1lh` (`resolve_length_percentage_or_auto` — `width`/`height`
/// only since Finding A) —
/// A resolvable own line-height serves as the multiplier.
/// `resolve_length_percentage_or_auto` **does not** delegate `Lh`/`Rlh` to
/// `resolve_length_percentage` (the fallback differs; see the next test),
/// but their numeric results agree when line-height is resolvable.
#[test]
fn length_percentage_or_auto_lh_multiplies_own_line_height_basis() {
    let fs = ComputedLength(20.0);
    assert_eq!(
        resolve_length_percentage_or_auto(
            LengthOrAuto::Length(Length::Lh(1.5)),
            fs,
            Some(ComputedLength(24.0)),
            &CTX,
        ),
        ComputedLengthPercentageOrAuto::Px(36.0),
    );
}

/// `width: 1lh` — when own line-height is `normal` and unresolved, fall back to **`Auto`**
/// (the spec initial value for `width`/`height`, CSS Sizing 3 §3.1.1),
/// unlike `resolve_length_percentage`'s `0px` fallback. **Margin no longer
/// passes through this function** (Finding A); see
/// `resolve_margin_length_or_auto_lh_falls_back_to_zero_when_unresolvable`
/// for the corresponding margin test.
#[test]
fn length_percentage_or_auto_lh_falls_back_to_auto_when_unresolvable() {
    let fs = ComputedLength(20.0);
    assert_eq!(
        resolve_length_percentage_or_auto(LengthOrAuto::Length(Length::Lh(1.5)), fs, None, &CTX),
        ComputedLengthPercentageOrAuto::Auto,
    );
    let ctx_no_root_lh = ResolveContext::new(ComputedLength(16.0));
    assert_eq!(
        resolve_length_percentage_or_auto(
            LengthOrAuto::Length(Length::Rlh(1.0)),
            fs,
            Some(ComputedLength(999.0)), // own line-height does not affect rlh
            &ctx_no_root_lh,
        ),
        ComputedLengthPercentageOrAuto::Auto,
    );
}

/// `margin-top: 1lh` (Finding A)
/// — use a resolvable own line-height as the multiplier. Numerically
/// identical to `resolve_length_percentage_or_auto`'s answer when
/// resolvable — only the unresolvable fallback differs (next test).
#[test]
fn resolve_margin_length_or_auto_lh_multiplies_own_line_height_basis() {
    let fs = ComputedLength(20.0);
    assert_eq!(
        resolve_margin_length_or_auto(
            LengthOrAuto::Length(Length::Lh(1.5)),
            fs,
            Some(ComputedLength(24.0)),
            &CTX,
        ),
        ComputedLengthPercentageOrAuto::Px(36.0),
    );
}

/// The Finding A regression check: `margin-top: 1lh` / `1rlh` under
/// `line-height: normal` (unresolvable) must compute to **`Px(0.0)`**
/// — margin's actual spec initial (CSS Box 3 §3.1) — not `Auto`
/// (`resolve_length_percentage_or_auto`'s fallback, which is correct
/// for `width`/`height` but was wrongly shared with `margin` before this
/// fix; `Auto` triggers real taffy auto-margin layout, not a neutral
/// "unspecified" value, per `resolve_margin_length_or_auto`'s doc).
#[test]
fn resolve_margin_length_or_auto_lh_falls_back_to_zero_when_unresolvable() {
    let fs = ComputedLength(20.0);
    assert_eq!(
        resolve_margin_length_or_auto(LengthOrAuto::Length(Length::Lh(1.5)), fs, None, &CTX),
        ComputedLengthPercentageOrAuto::Px(0.0),
    );
    let ctx_no_root_lh = ResolveContext::new(ComputedLength(16.0));
    assert_eq!(
        resolve_margin_length_or_auto(
            LengthOrAuto::Length(Length::Rlh(1.0)),
            fs,
            Some(ComputedLength(999.0)), // own line-height does not affect rlh
            &ctx_no_root_lh,
        ),
        ComputedLengthPercentageOrAuto::Px(0.0),
    );
}

/// `margin: auto` itself must still pass through as `Auto` —
/// `resolve_margin_length_or_auto` only changes the `Lh`/`Rlh`
/// unresolvable fallback, not the literal `auto` keyword's own meaning.
#[test]
fn resolve_margin_length_or_auto_keeps_auto_keyword() {
    assert_eq!(
        resolve_margin_length_or_auto(LengthOrAuto::Auto, ComputedLength(16.0), None, &CTX),
        ComputedLengthPercentageOrAuto::Auto,
    );
}

/// `margin-top: 50%` — percentage still passes through un-absolutized,
/// same as `resolve_length_percentage_or_auto` (used-value layer input).
#[test]
fn resolve_margin_length_or_auto_keeps_percent() {
    assert_eq!(
        resolve_margin_length_or_auto(
            LengthOrAuto::Length(Length::Percent(50.0)),
            ComputedLength(16.0),
            None,
            &CTX,
        ),
        ComputedLengthPercentageOrAuto::Percent(50.0),
    );
}

// -----------------------------------------------------------------
// phase 3: line-height
// -----------------------------------------------------------------

#[test]
fn line_height_normal_passes_through() {
    assert_eq!(
        resolve_line_height(LineHeight::Normal, ComputedLength(20.0), None, &CTX),
        ComputedLineHeight::Normal,
    );
}

/// `<number>` remains a number in the computed layer (CSS Inline 3 §5.1
/// "Computed value: the specified keyword, a number, or a computed
/// `<length>` value").
#[test]
fn line_height_number_passes_through() {
    assert_eq!(
        resolve_line_height(LineHeight::Number(1.5), ComputedLength(20.0), None, &CTX),
        ComputedLineHeight::Number(1.5),
    );
}

/// `<percentage>` is absolutized against the **element's own** computed font-size
/// (CSS Inline 3 §5.1 "Percentages: computed relative to 1em").
/// `150% × 20px = 30px`.
#[test]
fn line_height_percent_resolves_against_own_font_size() {
    assert_eq!(
        resolve_line_height(
            LineHeight::Length(Length::Percent(150.0)),
            ComputedLength(20.0),
            None,
            &CTX
        ),
        ComputedLineHeight::Length(ComputedLength(30.0)),
    );
}

#[test]
fn line_height_length_absolutizes_font_relative_units() {
    assert_eq!(
        resolve_line_height(
            LineHeight::Length(Length::Em(1.2)),
            ComputedLength(20.0),
            None,
            &CTX
        ),
        ComputedLineHeight::Length(ComputedLength(24.0)),
    );
    assert_eq!(
        resolve_line_height(
            LineHeight::Length(Length::Px(24.0)),
            ComputedLength(20.0),
            None,
            &CTX
        ),
        ComputedLineHeight::Length(ComputedLength(24.0)),
    );
}

/// `line-height: 1lh` is self-referential (CSS Values 4 §6.1.1, spec
/// quote + `lh`/`rlh` asymmetry rationale canonically documented on
/// `resolve_line_height`). When the parent's
/// own line-height is resolvable, `lh` multiplies by it — `own
/// font_size` (the 2nd arg) and `ctx.root_line_height` are **not**
/// consulted at all for this case, only `self_reference_basis` is.
#[test]
fn line_height_lh_resolves_against_parent_self_reference_basis() {
    let parent_basis = Some(ComputedLength(24.0)); // parent's used line-height
    let ctx_with_unrelated_root = ResolveContext::with_root_line_height(
        ComputedLength(16.0),
        Some(ComputedLength(999.0)), // must not leak into `lh`'s answer
    );
    assert_eq!(
        resolve_line_height(
            LineHeight::Length(Length::Lh(1.5)),
            ComputedLength(999.0), // own font-size, irrelevant here
            parent_basis,
            &ctx_with_unrelated_root,
        ),
        ComputedLineHeight::Length(ComputedLength(36.0)),
    );
}

/// `rlh`, unlike `lh`, is **not** treated as self-referential in this
/// crate — its own definition ("the lh unit on the root element") is a
/// tree-global constant, not something that depends on the declaring
/// element (see the `resolve_line_height` docs, "`Length::Rlh` — tree-global
/// constant rather than self-reference", for why the literal "Similarly,
/// lh or rlh" spec wording is not followed for `rlh` on non-root
/// elements). So `line-height: 1rlh` on a *non-root* element reads
/// `ctx.root_line_height`, **not** `self_reference_basis` (the parent's
/// line-height) — this is the discriminating test: parent and root
/// bases are deliberately different values.
#[test]
fn line_height_rlh_resolves_against_root_not_parent_self_reference_basis() {
    let parent_basis = Some(ComputedLength(999.0)); // must not leak into `rlh`'s answer
    let ctx = ResolveContext::with_root_line_height(
        ComputedLength(16.0),
        Some(ComputedLength(24.0)), // root's used line-height
    );
    assert_eq!(
        resolve_line_height(
            LineHeight::Length(Length::Rlh(0.5)),
            ComputedLength(999.0),
            parent_basis,
            &ctx,
        ),
        ComputedLineHeight::Length(ComputedLength(12.0)), // 0.5 * 24 (root), not 0.5 * 999
    );
}

/// When the parent's own line-height is `normal` (unresolvable — no font
/// metrics in the style layer, same wall as `cap`/`rcap`) or there is no
/// parent (`self_reference_basis: None`, root element case — CSS Values
/// 4 §6.1.1's "if the element has no parent" clause reduces to `normal`
/// there too, see `SpecifiedValues::finalize_as_root` doc), `1lh` falls
/// back to `line-height`'s own spec initial value `normal` rather than
/// inventing a length (this is the per-property
/// fallback chosen for the "normal" wall, distinct from
/// `resolve_length_percentage`'s `0px` / `resolve_length_percentage_or_auto`'s
/// `Auto`, because `normal` is what "unspecified" actually means for
/// this property).
#[test]
fn line_height_lh_falls_back_to_normal_when_self_reference_basis_unresolvable() {
    assert_eq!(
        resolve_line_height(
            LineHeight::Length(Length::Lh(1.5)),
            ComputedLength(20.0),
            None,
            &CTX,
        ),
        ComputedLineHeight::Normal,
    );
}

/// Same fallback for `rlh`, but keyed off `ctx.root_line_height` instead
/// of `self_reference_basis` — this is what makes root's own
/// self-referential `1rlh` (`self_reference_basis` irrelevant there,
/// `finalize_as_root` never passes it) *and* a normal-rooted document's
/// descendants both land on `normal` without special-casing which node
/// is which.
#[test]
fn line_height_rlh_falls_back_to_normal_when_root_line_height_unresolvable() {
    assert_eq!(
        resolve_line_height(
            LineHeight::Length(Length::Rlh(1.5)),
            ComputedLength(20.0),
            Some(ComputedLength(999.0)), // parent basis must not rescue rlh
            &CTX,                        // CTX.root_line_height == None
        ),
        ComputedLineHeight::Normal,
    );
}

// -----------------------------------------------------------------
// phase 3: border
// -----------------------------------------------------------------

#[test]
fn border_absolutizes_width_and_carries_style_and_color() {
    let mut specified = SpecifiedValues::initial().border.top;
    specified.style = BorderStyle::Solid;
    let computed = resolve_border(specified, ComputedLength(20.0), None, &CTX);
    // specified `medium` = 3px (CSS Backgrounds 3 §3.3: thin/medium/thick are
    // **normatively** equivalent to 1px/3px/5px).
    assert_eq!(computed.width, ComputedLength(3.0));
    assert_eq!(computed.style, specified.style);
    assert_eq!(computed.color, specified.color);
}

#[test]
fn a_border_without_a_line_keeps_its_colour_and_loses_its_width() {
    let mut specified = SpecifiedValues::initial().border.top;
    specified.style = BorderStyle::Solid;
    let border = resolve_border(specified, ComputedLength(20.0), None, &CTX);
    assert_eq!(border.width(), ComputedLength(3.0));
    let none = border.without_line();
    assert_eq!(none.width(), ComputedLength::ZERO);
    assert_eq!(none.style(), BorderStyle::None);
    assert_eq!(none.color, border.color);
}

/// `border-*-width: 1lh` / `1rlh` — use a resolvable basis as a multiplier;
/// if the basis is `None` (`normal` is unresolved), use `resolve_length`'s
/// grammar-unreachable `Percent` arm's fallback, `0px`.
#[test]
fn border_width_resolves_lh_and_rlh() {
    let mut specified = SpecifiedValues::initial().border.top;
    specified.style = BorderStyle::Solid;
    specified.width = Length::Lh(2.0);
    assert_eq!(
        resolve_border(
            specified,
            ComputedLength(20.0),
            Some(ComputedLength(10.0)),
            &CTX
        )
        .width,
        ComputedLength(20.0), // 2 * 10
    );
    // `own_line_height: None` (normal is unresolved) → 0px.
    assert_eq!(
        resolve_border(specified, ComputedLength(20.0), None, &CTX).width,
        ComputedLength::ZERO,
    );

    let mut rlh_specified = specified;
    rlh_specified.width = Length::Rlh(1.5);
    let ctx =
        ResolveContext::with_root_line_height(ComputedLength(16.0), Some(ComputedLength(20.0)));
    assert_eq!(
        // own_line_height (999) does not affect `rlh`; use only `ctx.root_line_height`.
        resolve_border(
            rlh_specified,
            ComputedLength(20.0),
            Some(ComputedLength(999.0)),
            &ctx
        )
        .width,
        ComputedLength(30.0), // 1.5 * 20
    );
}

/// Pin the body of the `ComputedBorder::width()` / `::style()` accessors.
/// The test above is in the same module,
/// so it directly accesses `pub(crate)` fields and does not call the accessor
/// bodies. A doctest calls them from outside the crate (in the `ComputedBorder`
/// type docs), but this repo's toolchain (pinned stable;
/// `cargo llvm-cov` without `--doctests`) does not measure doctest coverage.
/// This test executes the accessor bodies so they appear in coverage.
#[test]
fn computed_border_accessors_read_the_gated_fields() {
    let mut specified = SpecifiedValues::initial().border.top;
    specified.style = BorderStyle::Solid;
    let computed = resolve_border(specified, ComputedLength(20.0), None, &CTX);
    assert_eq!(computed.width(), ComputedLength(3.0));
    assert_eq!(computed.style(), BorderStyle::Solid);
}

/// CSS Backgrounds 3 §3.3 "Computed value: … zero if the border style is
/// none or hidden" — style gating is required at the **computed layer**.
#[test]
fn border_width_is_zero_when_style_is_none_or_hidden() {
    let mut b = SpecifiedValues::initial().border.top;
    b.width = Length::Px(5.0);
    for style in [BorderStyle::None, BorderStyle::Hidden] {
        b.style = style;
        assert_eq!(
            resolve_border(b, ComputedLength(20.0), None, &CTX).width,
            ComputedLength::ZERO,
        );
    }
    b.style = BorderStyle::Solid;
    assert_eq!(
        resolve_border(b, ComputedLength(20.0), None, &CTX).width,
        ComputedLength(5.0),
    );
}

#[test]
fn border_em_width_resolves_against_own_font_size() {
    let mut specified = SpecifiedValues::initial().border.top;
    specified.width = Length::Em(0.5);
    specified.style = BorderStyle::Solid;
    let computed = resolve_border(specified, ComputedLength(20.0), None, &CTX);
    assert_eq!(computed.width, ComputedLength(10.0));
}

/// CSS Values 4 §6.2 defines `1pc = 16px`. Border width remains zero
/// when the border style is `none`, as required by CSS Backgrounds and Borders.
#[test]
fn border_pc_width_is_absolutized_and_still_gated_by_style() {
    let mut specified = SpecifiedValues::initial().border.top;
    specified.width = Length::Pc(1.0);
    specified.style = BorderStyle::Solid;
    assert_eq!(
        resolve_border(specified, ComputedLength(20.0), None, &CTX).width,
        ComputedLength(16.0),
    );
    specified.style = BorderStyle::None;
    assert_eq!(
        resolve_border(specified, ComputedLength(20.0), None, &CTX).width,
        ComputedLength::ZERO,
    );
}

#[test]
fn border_radius_and_box_shadow_resolve_all_length_components() {
    let radius = BorderRadius {
        top_left: Length::Em(1.0).into(),
        top_right: Length::Rem(0.5).into(),
        bottom_right: Length::Pt(12.0).into(),
        bottom_left: Length::Lh(2.0).into(),
    };
    assert_eq!(
        resolve_border_radius(
            radius,
            ComputedLength(20.0),
            Some(ComputedLength(10.0)),
            &CTX,
        ),
        ComputedBorderRadius {
            top_left: ComputedLengthPercentage::Px(20.0).into(),
            top_right: ComputedLengthPercentage::Px(8.0).into(),
            bottom_right: ComputedLengthPercentage::Px(16.0).into(),
            bottom_left: ComputedLengthPercentage::Px(20.0).into(),
        }
    );

    let shadow = BoxShadowItem {
        offset_x: Length::Em(-0.5),
        offset_y: Length::Rem(0.25),
        blur_radius: Length::Pt(3.0),
        spread_radius: Length::Lh(0.5),
        color: TextShadowColor::Resolved(CssColor::BLACK),
        inset: true,
    };
    assert_eq!(
        resolve_box_shadow_item(
            shadow,
            ComputedLength(20.0),
            Some(ComputedLength(10.0)),
            &CTX,
        ),
        ComputedBoxShadowItem {
            offset_x: ComputedLength(-10.0),
            offset_y: ComputedLength(4.0),
            blur_radius: ComputedLength(4.0),
            spread_radius: ComputedLength(5.0),
            color: TextShadowColor::Resolved(CssColor::BLACK),
            inset: true,
        }
    );
}

/// `resolve_css_position` absolutizes each axis's `Length` payload while
/// preserving the `Start`/`End` edge — `Em`/`Percent` cover both the
/// px-absolutization branch and the pass-through-percentage branch,
/// `End` covers the edge-relative offset case (`CssPositionOffset` doc's
/// "why two variants" section).
#[test]
fn resolve_css_position_absolutizes_each_offset() {
    let specified = CssPosition {
        horizontal: CssPositionOffset::Start(Length::Em(1.0)),
        vertical: CssPositionOffset::End(Length::Percent(30.0)),
    };
    assert_eq!(
        resolve_css_position(specified, ComputedLength(16.0), None, &CTX),
        ComputedCssPosition {
            horizontal: ComputedCssPositionOffset::Start(ComputedLengthPercentage::Px(16.0)),
            vertical: ComputedCssPositionOffset::End(ComputedLengthPercentage::Percent(30.0)),
        }
    );
}

/// `resolve_background_size` absolutizes each axis via the same
/// `<length-percentage> | auto` shape `width`/`height` use, and passes
/// `cover`/`contain` straight through untouched.
#[test]
fn resolve_background_size_absolutizes_explicit_axes_and_passes_keywords_through() {
    let explicit = BackgroundSize::Explicit {
        width: LengthOrAuto::Length(Length::Em(2.0)),
        height: LengthOrAuto::Auto,
    };
    assert_eq!(
        resolve_background_size(explicit, ComputedLength(16.0), None, &CTX),
        ComputedBackgroundSize::Explicit {
            width: ComputedLengthPercentageOrAuto::Px(32.0),
            height: ComputedLengthPercentageOrAuto::Auto,
        }
    );
    assert_eq!(
        resolve_background_size(BackgroundSize::Cover, ComputedLength(16.0), None, &CTX),
        ComputedBackgroundSize::Cover
    );
    assert_eq!(
        resolve_background_size(BackgroundSize::Contain, ComputedLength(16.0), None, &CTX),
        ComputedBackgroundSize::Contain
    );
}

#[test]
fn outline_resolution_gates_width_for_none_and_preserves_visible_styles() {
    let mut outline = Outline {
        width: Length::Em(2.0),
        style: OutlineStyle::None,
        color: OutlineColor::Resolved(CssColor::BLACK),
    };
    let computed = resolve_outline(outline, ComputedLength(20.0), None, &CTX);
    assert_eq!(computed.width(), ComputedLength::ZERO);
    assert_eq!(computed.style(), OutlineStyle::None);
    assert_eq!(computed.color, outline.color);

    outline.style = OutlineStyle::Solid;
    let computed = resolve_outline(outline, ComputedLength(20.0), None, &CTX);
    assert_eq!(computed.width(), ComputedLength(40.0));
    assert_eq!(computed.style(), OutlineStyle::Solid);
    assert_eq!(computed.color, outline.color);

    outline.style = OutlineStyle::Auto;
    let computed = resolve_outline(outline, ComputedLength(20.0), None, &CTX);
    assert_eq!(computed.width(), ComputedLength(40.0));
    assert_eq!(computed.style(), OutlineStyle::Auto);
    assert_eq!(computed.color, outline.color);
}

// -----------------------------------------------------------------
// Losslessness / fixed-point property of lifting (computed → specified)
// -----------------------------------------------------------------

/// `Px` is a fixed point of absolutization, so lifting and absolutizing again
/// is the identity. Changing the basis font-size must not change the result.
#[test]
fn lift_font_size_is_fixed_point_under_absolutization() {
    let inherited = ComputedLength(24.0);
    let lifted = lift_font_size(inherited);
    assert_eq!(lifted, Length::Px(24.0));
    assert_eq!(
        resolve_font_size(lifted, ComputedLength(16.0), None, &CTX),
        inherited,
    );
    // Unchanged with a different basis (no double application).
    assert_eq!(
        resolve_font_size(
            lifted,
            ComputedLength(100.0),
            None,
            &ResolveContext::new(ComputedLength(100.0)),
        ),
        inherited,
    );
}

/// `line-height: 150%` is absolutized on the **declaring element**, and the child
/// inherits that length (CSS Inline 3 §5.1). **Do not resolve it again** against
/// the child's font-size. This test catches a mistaken "re-resolve percentages" fix.
/// This is a regression check.
#[test]
fn lift_line_height_length_does_not_re_resolve_percentage_in_child() {
    let declared = resolve_line_height(
        LineHeight::Length(Length::Percent(150.0)),
        ComputedLength(20.0),
        None,
        &CTX,
    );
    assert_eq!(declared, ComputedLineHeight::Length(ComputedLength(30.0)));

    let lifted = lift_line_height(declared);
    assert_eq!(lifted, LineHeight::Length(Length::Px(30.0)));

    // Even if the child's font-size is 10px, this does not become 15px.
    let child = resolve_line_height(lifted, ComputedLength(10.0), None, &CTX);
    assert_eq!(child, ComputedLineHeight::Length(ComputedLength(30.0)));
}

/// Lifting also passes `<number>` through, allowing the child's own font-size to apply.
#[test]
fn lift_line_height_number_and_normal_pass_through() {
    assert_eq!(
        lift_line_height(ComputedLineHeight::Number(1.5)),
        LineHeight::Number(1.5),
    );
    assert_eq!(
        lift_line_height(ComputedLineHeight::Normal),
        LineHeight::Normal,
    );
    // Number stays a number rather than resolving against the child's font-size.
    assert_eq!(
        resolve_line_height(
            lift_line_height(ComputedLineHeight::Number(1.5)),
            ComputedLength(10.0),
            None,
            &CTX
        ),
        ComputedLineHeight::Number(1.5),
    );
}

// -----------------------------------------------------------------
// text-indent computed-value resolution and inheritance
// -----------------------------------------------------------------

#[test]
fn lift_length_percentage_px_is_a_fixed_point() {
    assert_eq!(
        lift_length_percentage(ComputedLengthPercentage::Px(40.0)),
        Length::Px(40.0),
    );
}

#[test]
fn lift_length_percentage_percent_remains_unresolved() {
    assert_eq!(
        lift_length_percentage(ComputedLengthPercentage::Percent(10.0)),
        Length::Percent(10.0),
    );
}

#[test]
fn lift_text_indent_calc_does_not_reapply_parent_em_in_child() {
    let lifted = lift_text_indent(ComputedTextIndent::Calc(CalcLengthPercentage {
        percent: 50.0,
        px: 60.0,
    }));
    assert_eq!(
        lifted,
        TextIndentLength::Calc(LengthPercentageCalc {
            percent: 50.0,
            px: 60.0,
            em: 0.0,
            ch: 0.0,
        }),
    );
}

#[test]
fn resolve_text_indent_calc_collapses_pure_length_and_preserves_mixed_calc() {
    assert_eq!(
        resolve_text_indent_calc(
            LengthPercentageCalc {
                percent: 0.0,
                px: 10.0,
                em: 0.5,
                ch: 0.0,
            },
            ComputedLength(40.0),
        ),
        ComputedTextIndent::Px(30.0),
    );
    assert_eq!(
        resolve_text_indent_calc(
            LengthPercentageCalc {
                percent: 50.0,
                px: 60.0,
                em: 0.0,
                ch: 0.0,
            },
            ComputedLength(40.0),
        ),
        ComputedTextIndent::Calc(CalcLengthPercentage {
            percent: 50.0,
            px: 60.0,
        }),
    );
}

/// `resolve_flex_basis`'s `FlexBasisValue::Content` arm — pass-through
/// keyword, no `<length-percentage>` machinery involved
/// ([`resolve_flex_basis`] docs: "content passes through as a keyword"
/// note).
#[test]
fn resolve_flex_basis_content_is_pass_through_keyword() {
    assert_eq!(
        resolve_flex_basis(FlexBasisValue::Content, ComputedLength(20.0), None, &CTX),
        ComputedFlexBasis::Content,
    );
}

/// `resolve_flex_basis`'s nested `Lh`/`Rlh`-unresolvable fallback
/// (delegated to [`resolve_length_percentage_or_auto`], sibling to
/// `border_width_resolves_lh_and_rlh` above) — `own_line_height: None`
/// (unresolved because it is `normal`) makes `1lh` fall back to `Auto`, as in the
/// `<'width'>` shape [`resolve_flex_basis`]'s doc says it reuses.
#[test]
fn resolve_flex_basis_falls_back_to_auto_when_lh_unresolvable() {
    assert_eq!(
        resolve_flex_basis(
            FlexBasisValue::Length(Length::Lh(2.0)),
            ComputedLength(20.0),
            None,
            &CTX
        ),
        ComputedFlexBasis::Auto,
    );
}

/// [`resolve_vertical_align`]'s bare-keyword variants are pass-through
/// — no length payload, nothing for phase 3 to absolutize (same shape
/// as [`resolve_flex_basis`]'s `Content` arm above).
#[test]
fn resolve_vertical_align_keywords_are_pass_through() {
    for va in [
        VerticalAlign::Baseline,
        VerticalAlign::Sub,
        VerticalAlign::Super,
        VerticalAlign::Middle,
        VerticalAlign::TextTop,
        VerticalAlign::TextBottom,
        VerticalAlign::Top,
        VerticalAlign::Bottom,
    ] {
        assert_eq!(
            resolve_vertical_align(va, ComputedLength(20.0), None, &CTX),
            va,
        );
    }
}

/// `<length>` absolutizes against the declaring node's own `font-size`
/// (`em`) — the same basis `letter-spacing`/`word-spacing` use via
/// [`resolve_length_or_normal`].
#[test]
fn resolve_vertical_align_length_absolutizes_em() {
    assert_eq!(
        resolve_vertical_align(
            VerticalAlign::Length(Length::Em(2.0)),
            ComputedLength(10.0),
            None,
            &CTX
        ),
        VerticalAlign::Length(Length::Px(20.0)),
    );
}

/// §10.8.1 spec verbatim ("Raise (positive value) or lower (negative
/// value)") — negative `<length>` absolutizes without a non-negative
/// filter, same as `letter-spacing`/`margin-*`.
#[test]
fn resolve_vertical_align_length_preserves_negative_sign() {
    assert_eq!(
        resolve_vertical_align(
            VerticalAlign::Length(Length::Px(-6.0)),
            ComputedLength(10.0),
            None,
            &CTX
        ),
        VerticalAlign::Length(Length::Px(-6.0)),
    );
}

/// `own_line_height: None` (unresolved `normal`) makes `1lh` fall back
/// to `Px(0.0)` — same [`resolve_length`] "Finding B" fallback
/// `resolve_flex_basis_falls_back_to_auto_when_lh_unresolvable` above
/// pins, but landing on the *benign* side of that doc's distinction:
/// `0px` shift here matches CSS 2.1 §10.8.1's own spec verbatim for
/// `<length>` ("The value `0cm` means the same as `baseline`."), not an
/// arbitrary "no better option" value (`resolve_length` doc's "Finding
/// B" section).
#[test]
fn resolve_vertical_align_length_falls_back_to_zero_when_lh_unresolvable() {
    assert_eq!(
        resolve_vertical_align(
            VerticalAlign::Length(Length::Lh(2.0)),
            ComputedLength(20.0),
            None,
            &CTX
        ),
        VerticalAlign::Length(Length::Px(0.0)),
    );
}

#[test]
fn resolve_vertical_align_percentage_resolves_against_own_line_height() {
    // 50% of 20px = 10px; 100% = basis itself.
    assert_eq!(
        resolve_vertical_align(
            VerticalAlign::Length(Length::Percent(50.0)),
            ComputedLength(16.0),
            Some(ComputedLength(20.0)),
            &CTX
        ),
        VerticalAlign::Length(Length::Px(10.0)),
    );
    assert_eq!(
        resolve_vertical_align(
            VerticalAlign::Length(Length::Percent(100.0)),
            ComputedLength(16.0),
            Some(ComputedLength(30.0)),
            &CTX
        ),
        VerticalAlign::Length(Length::Px(30.0)),
    );
}

#[test]
fn resolve_vertical_align_percentage_falls_back_to_zero_when_line_height_normal() {
    // `line-height: normal` → `own_line_height: None` → spec-deviation
    // fallback to 0px (baseline-equivalent), pinned as documented deviation
    // (`VerticalAlign` doc + `resolve_vertical_align` doc).
    assert_eq!(
        resolve_vertical_align(
            VerticalAlign::Length(Length::Percent(50.0)),
            ComputedLength(16.0),
            None,
            &CTX
        ),
        VerticalAlign::Length(Length::Px(0.0)),
    );
    // 0% with fallback is still 0 and spec-equivalent to baseline.
    assert_eq!(
        resolve_vertical_align(
            VerticalAlign::Length(Length::Percent(0.0)),
            ComputedLength(16.0),
            None,
            &CTX
        ),
        VerticalAlign::Length(Length::Px(0.0)),
    );
    // negative percentage preserves sign when basis is resolvable,
    // falls back to 0 when not.
    assert_eq!(
        resolve_vertical_align(
            VerticalAlign::Length(Length::Percent(-40.0)),
            ComputedLength(16.0),
            Some(ComputedLength(20.0)),
            &CTX
        ),
        VerticalAlign::Length(Length::Px(-8.0)),
    );
    assert_eq!(
        resolve_vertical_align(
            VerticalAlign::Length(Length::Percent(-40.0)),
            ComputedLength(16.0),
            None,
            &CTX
        ),
        VerticalAlign::Length(Length::Px(0.0)),
    );
}

#[test]
fn resolve_vertical_align_mixed_calc_uses_own_line_height() {
    let cases = [
        (50.0, 25.0, Some(ComputedLength(100.0)), 75.0),
        (75.0, -30.0, Some(ComputedLength(100.0)), 45.0),
        (0.0, 40.0, Some(ComputedLength(100.0)), 40.0),
        (-10.0, 40.0, Some(ComputedLength(100.0)), 30.0),
        // Keep the absolute term when the percentage basis is unavailable.
        (50.0, 25.0, None, 25.0),
    ];
    for (percent, px, line_height, expected_px) in cases {
        assert_eq!(
            resolve_vertical_align(
                VerticalAlign::Calc(CalcLengthPercentage { percent, px }),
                ComputedLength(16.0),
                line_height,
                &CTX,
            ),
            VerticalAlign::Length(Length::Px(expected_px)),
        );
    }
}

// -----------------------------------------------------------------
// specified initial → computed initial (detect drift per function)
// -----------------------------------------------------------------

/// Pass specified-layer initial values separately through each absolutization
/// function and check they become the spec's computed initial values (padding /
/// margin = 0px, width / height = auto, border-width = 0px, line-height = normal,
/// font-size = 16px).
///
/// The aggregate check (all of `SpecifiedValues::finalize`) lives under `crate::specified` // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests (#[test]-item doc) — rustdoc-blind, confirmed by deliberately breaking a link
/// in `initial_specified_finalizes_to_initial_computed`. This test
/// isolates **which function broke** through per-function checks.
#[test]
fn initial_length_fields_absolutize_to_spec_initials() {
    let initial = SpecifiedValues::initial();
    let fs = ComputedLength(INITIAL_FONT_SIZE_PX);

    assert_eq!(initial.padding, Sides::all(Length::Px(0.0)));
    assert_eq!(
        resolve_length_percentage(initial.padding.top, fs, None, &CTX),
        ComputedLengthPercentage::Px(0.0),
    );
    assert_eq!(
        initial.text_indent,
        TextIndentLength::Length(Length::Px(0.0)),
    );
    assert_eq!(
        resolve_length_percentage(Length::Px(0.0), fs, None, &CTX),
        ComputedLengthPercentage::Px(0.0),
    );
    assert_eq!(
        resolve_length_percentage_or_auto(initial.margin.top, fs, None, &CTX),
        ComputedLengthPercentageOrAuto::Px(0.0),
    );
    assert_eq!(
        resolve_length_percentage_or_auto(initial.width, fs, None, &CTX),
        ComputedLengthPercentageOrAuto::Auto,
    );
    assert_eq!(
        resolve_length_percentage_or_auto(initial.height, fs, None, &CTX),
        ComputedLengthPercentageOrAuto::Auto,
    );
    // CSS Backgrounds 3 §3.3 "Computed value: … zero if the border style is
    // none or hidden" — initial style is `none`, so computed width is 0px
    // (style gating reduces the specified `medium` = 3px).
    assert_eq!(initial.border.left.width, Length::Px(3.0));
    assert_eq!(
        resolve_border(initial.border.left, fs, None, &CTX).width,
        ComputedLength::ZERO,
    );
    assert_eq!(
        resolve_line_height(initial.line_height, fs, None, &CTX),
        ComputedLineHeight::Normal,
    );
    assert_eq!(
        resolve_font_size(initial.font_size, fs, None, &CTX),
        ComputedLength(INITIAL_FONT_SIZE_PX),
    );
}

#[test]
fn text_underline_offset_percentage_stays_relative() {
    // CSS Text Decoration 4 §2.8: percentages inherit as relative values, so
    // the declaring element's font size must not be baked in.
    assert_eq!(
        resolve_text_underline_offset(
            TextUnderlineOffset::Length(Length::Percent(25.0)),
            ComputedLength(20.0),
            Some(ComputedLength(40.0)),
            &CTX,
        ),
        ComputedTextUnderlineOffset::Percent(25.0),
    );
}

#[test]
fn text_underline_offset_calc_resolves_em_and_retains_mixed_percentages() {
    let font_size = ComputedLength(16.0);
    let cases = [
        (
            LengthPercentageCalc {
                percent: 0.0,
                px: -8.0,
                em: 2.0,
                ch: 0.0,
            },
            ComputedTextUnderlineOffset::Length(ComputedLength(24.0)),
        ),
        (
            LengthPercentageCalc {
                percent: -50.0,
                px: 0.0,
                em: 2.0,
                ch: 0.0,
            },
            ComputedTextUnderlineOffset::Calc(CalcLengthPercentage {
                percent: -50.0,
                px: 32.0,
            }),
        ),
        (
            LengthPercentageCalc {
                percent: 200.0,
                px: -8.0,
                em: 0.0,
                ch: 0.0,
            },
            ComputedTextUnderlineOffset::Calc(CalcLengthPercentage {
                percent: 200.0,
                px: -8.0,
            }),
        ),
        (
            LengthPercentageCalc {
                percent: 200.0,
                px: 0.0,
                em: -0.5,
                ch: 0.0,
            },
            ComputedTextUnderlineOffset::Calc(CalcLengthPercentage {
                percent: 200.0,
                px: -8.0,
            }),
        ),
    ];
    for (specified, expected) in cases {
        assert_eq!(
            resolve_text_underline_offset(
                TextUnderlineOffset::Calc(specified),
                font_size,
                None,
                &CTX,
            ),
            expected,
        );
    }
}

#[test]
fn text_decoration_thickness_preserves_keywords_and_resolves_lengths() {
    assert_eq!(
        resolve_text_decoration_thickness(
            TextDecorationThickness::Auto,
            ComputedLength(20.0),
            None,
            &CTX,
        ),
        ComputedTextDecorationThickness::Auto,
    );
    assert_eq!(
        resolve_text_decoration_thickness(
            TextDecorationThickness::FromFont,
            ComputedLength(20.0),
            None,
            &CTX,
        ),
        ComputedTextDecorationThickness::FromFont,
    );

    let computed = resolve_text_decoration_thickness(
        TextDecorationThickness::Length(Length::Em(0.5)),
        ComputedLength(20.0),
        None,
        &CTX,
    );
    assert_eq!(
        computed,
        ComputedTextDecorationThickness::Length(ComputedLength(10.0))
    );
}

#[test]
fn circular_corner_constructor_preserves_each_value_until_axis_resolution() {
    use ComputedLengthPercentage::{Percent, Px};
    let [tl, tr, br, bl] = std::hint::black_box([Percent(25.0), Px(10.0), Percent(40.0), Px(0.0)]);
    let radius = ComputedBorderRadius::corners(tl, tr, br, bl);
    assert_eq!(
        radius.used(200.0, 100.0),
        [[50.0, 25.0], [10.0, 10.0], [80.0, 40.0], [0.0, 0.0]]
    );
}
