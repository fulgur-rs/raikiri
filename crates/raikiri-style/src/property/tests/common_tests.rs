//! Tests for the shared value helpers in `parse/common.rs`.

use super::*;

// ── parse_length_value helper ────────────────────
//
// Shared fixture exercising the helper directly, without going through the property
// dispatcher (`parse_value`): directly verify parsing of five sample units
// (`px` / `em` / `rem` / `%` / `pt`). Upstream `parse_font_size` tests already exist
// (`font_size_parse_px`, etc.); those verify the px-only post-filter separately.

fn parse_length(source: &str, allow_percentage: bool) -> Option<Length> {
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    parse_length_value(&mut parser, allow_percentage)
}

#[test]
fn parse_length_value_accepts_px() {
    assert_eq!(parse_length("10px", false), Some(Length::Px(10.0)));
    // Accept px in length-percentage mode as well (independent of mode).
    assert_eq!(parse_length("10px", true), Some(Length::Px(10.0)));
}

#[test]
fn parse_length_value_accepts_em() {
    // CSS Values 4 §6.1.1 em (https://www.w3.org/TR/css-values-4/#em):
    // Preserve authored `1.2em` unchanged as Length::Em(1.2) (resolution is downstream).
    assert_eq!(parse_length("1.2em", false), Some(Length::Em(1.2)));
}

#[test]
fn parse_length_value_accepts_rem() {
    // CSS Values 4 §6.1.1 rem (https://www.w3.org/TR/css-values-4/#rem):
    // Store the authored value in Length::Rem, relative to the root element's font size.
    assert_eq!(parse_length("1rem", false), Some(Length::Rem(1.0)));
}

#[test]
fn parse_length_value_accepts_pt() {
    // CSS Values 4 §6.2 absolute lengths (https://www.w3.org/TR/css-values-4/#absolute-lengths):
    // 1pt = 1/72in and 1in = 96px; resolution converts 12pt to about 16px.
    assert_eq!(parse_length("12pt", false), Some(Length::Pt(12.0)));
}

#[test]
fn parse_length_value_accepts_percentage_when_allowed() {
    // CSS Values 4 §5.5 (https://www.w3.org/TR/css-values-4/#percentages):
    // Accept only in `<length-percentage>` mode. Multiply cssparser's `unit_value = 0.5`
    // by 100.0 to recover the authored `50` and store Length::Percent(50.0).
    assert_eq!(parse_length("50%", true), Some(Length::Percent(50.0)));
}

#[test]
fn parse_length_value_extreme_percentage_saturates_to_f32_max_not_inf() {
    // The cssparser tokenizer parses `1e40%` as
    // `unit_value = 1e40 / 100.0 = 1e38` (within the finite f32 range, `3.4028235e38`),
    // but this helper's `× 100.0` to recover the authored number
    // overflowed f32 and produced +Inf (before the fix).
    //
    // CSS Values 4 §5 "Range Checking and Precision for Numeric Types"
    // <https://www.w3.org/TR/css-values-4/#numeric-types>:
    // "When a value cannot be explicitly supported due to
    // range/precision limitations, it must be converted to the closest
    // value supported by the implementation" — non-finite values are not allowed;
    // check a finite value near f32::MAX while preserving its sign.
    assert_eq!(parse_length("1e40%", true), Some(Length::Percent(f32::MAX)));
    // Also check sign preservation (negative overflow approaches -f32::MAX).
    assert_eq!(
        parse_length("-1e40%", true),
        Some(Length::Percent(-f32::MAX))
    );
}

#[test]
fn parse_length_value_zero_mantissa_huge_exponent_percentage_resolves_to_zero() {
    // `0e999%` is a zero-mantissa, huge-exponent literal — cssparser's
    // tokenizer computes its pre-`/100` magnitude as `0.0 *
    // f64::powf(10., 999.)`, and `10f64.powf(999.0)` is `+Inf`, so the
    // product collapses to `NaN` per IEEE 754, even though CSS Syntax
    // Level 3 §4.3.13's own formula gives an exact `0` for this input
    // (`property.rs` module doc's "Numeric-token NaN stabilization"
    // section is canonical for the mechanism). `next_numeric_stable`
    // (which `parse_length_value` acquires its token through) corrects
    // this before `parse_length_value` ever sees the `Token::Percentage`,
    // so `0e999%` resolves to the spec-correct `Length::Percent(0.0)`
    // directly — it no longer reaches the `is_infinite()`-only
    // saturation guard as `NaN` at all (that guard remains, for the
    // genuinely-different `1e40%` magnitude-overflow case pinned above).
    assert_eq!(parse_length("0e999%", true), Some(Length::Percent(0.0)));
    // Sign is preserved through the recovery (`-0.0 == 0.0` under IEEE
    // 754, so this also confirms the negative-mantissa path resolves,
    // not just that it doesn't panic).
    assert_eq!(parse_length("-0e999%", true), Some(Length::Percent(0.0)));
}

#[test]
fn parse_length_value_zero_mantissa_with_fractional_part_huge_exponent_resolves_to_zero() {
    // Same collapse as `0e999`, but with a written fractional part
    // (`numeric_token_prefix`'s `.`-branch, not exercised by the
    // integer-only `0e999`/`0e999%` cases above) — `0.0e999px` still
    // has zero mantissa (`0 + 0 * 10^-1 == 0`), so it resolves to
    // `Length::Px(0.0)` the same way.
    assert_eq!(parse_length("0.0e999px", false), Some(Length::Px(0.0)));
}

#[test]
fn parse_length_value_mirror_huge_mantissa_tiny_exponent_resolves_correctly() {
    // The `0 * Infinity` collapse this crate works around
    // (`0e999`-shaped literals, zero mantissa / huge exponent) has a
    // mirror case: a mantissa long enough to itself overflow to
    // `+Infinity` while being accumulated digit-by-digit (cssparser
    // `tokenizer.rs`'s `consume_numeric`, no exponent needed for this
    // half), multiplied by a sufficiently negative exponent's
    // `10^exponent` (which underflows to `0.0`), hits `Infinity * 0.0`
    // = `NaN` the same way — for a literal whose true value is small
    // but nonzero. `1` followed by 400 zeros, `e-400`, has true value
    // `1` (`1eN * 10^-N == 1` for any `N`); this crate's recovery does
    // not special-case which operand collapsed to `0`/`Infinity` (it
    // simply re-parses the raw token text), so it resolves this
    // mirror case for free, not just the zero-mantissa one.
    let digits = format!("1{}", "0".repeat(400)); // 10^400, overflows f64 digit accumulation to +Infinity
    let source = format!("{digits}e-400px");
    assert_eq!(parse_length(&source, false), Some(Length::Px(1.0)));
}

#[test]
fn stabilize_nan_numeric_value_is_a_no_op_for_non_nan_input() {
    // Direct unit test of the private recovery primitive's documented
    // "no-op" contract for the common case — every call site
    // (`expect_number_stable`/`next_numeric_stable`) already gates on
    // `value.is_nan()` before calling this function, so this branch
    // is otherwise never exercised through the public parsing paths
    // above (they only ever pass a `NaN` `value` in practice). The
    // `raw_number_text` argument is deliberately garbage here (it
    // would never actually be parsed, since the `!value.is_nan()`
    // early return fires first) to prove the early return, not the
    // reparse path, is what's under test.
    assert_eq!(stabilize_nan_numeric_value("not a number", 5.0), 5.0);
    assert_eq!(
        stabilize_nan_numeric_value("not a number", f32::INFINITY),
        f32::INFINITY
    );
}

#[test]
fn stabilize_nan_percentage_value_is_a_no_op_for_non_nan_input() {
    // Same contract, `Token::Percentage`'s `unit_value` counterpart —
    // see `stabilize_nan_numeric_value_is_a_no_op_for_non_nan_input`.
    assert_eq!(stabilize_nan_percentage_value("not a number", 0.5), 0.5);
    assert_eq!(
        stabilize_nan_percentage_value("not a number", f32::NEG_INFINITY),
        f32::NEG_INFINITY
    );
}

#[test]
fn parse_length_value_rejects_percentage_in_length_only_mode() {
    // In `<length>` mode (e.g. font-size), `%` is outside the grammar: return None.
    assert_eq!(parse_length("50%", false), None);
}

#[test]
fn parse_length_value_rejects_unsupported_unit() {
    // (b) Unsupported — this helper still silently drops `cap` and `rcap`;
    // `lh`, `rlh` and the viewport-relative units have moved to the accepted
    // set (see `parse_length_value_accepts_lh` / `_rlh` below).
    assert_eq!(parse_length("10rcap", false), None);
    assert_eq!(parse_length("1cap", true), None);
    // Container-query units (CSS Contain 3 §6): check the `cq*` list cited in
    // the comment just before the `_` arm with this assertion. If the comment
    // has no test coverage, a future incorrect `cq*` support arm could be added
    // without failing any tests, silently making the canonical comment stale
    // (added as a spec-lens follow-up).
    assert_eq!(parse_length("10cqw", false), None);
}

#[test]
fn parse_length_value_accepts_lh() {
    // https://www.w3.org/TR/css-values-4/#lh — preserve the authored value unchanged.
    assert_eq!(parse_length("1.5lh", false), Some(Length::Lh(1.5)));
}

#[test]
fn parse_length_value_accepts_rlh() {
    // https://www.w3.org/TR/css-values-4/#rlh
    assert_eq!(parse_length("2rlh", false), Some(Length::Rlh(2.0)));
}

// ── Additional font-relative units (CSS Values 4 §6.1.1) ──

#[test]
fn parse_length_value_accepts_ex() {
    // https://www.w3.org/TR/css-values-4/#ex — preserve the authored value unchanged.
    assert_eq!(parse_length("2ex", false), Some(Length::Ex(2.0)));
}

#[test]
fn parse_length_value_accepts_rex() {
    // https://www.w3.org/TR/css-values-4/#rex
    assert_eq!(parse_length("2rex", false), Some(Length::Rex(2.0)));
}

#[test]
fn parse_length_value_accepts_ch() {
    // https://www.w3.org/TR/css-values-4/#ch
    assert_eq!(parse_length("3ch", false), Some(Length::Ch(3.0)));
}

#[test]
fn parse_length_value_accepts_rch() {
    // https://www.w3.org/TR/css-values-4/#rch
    assert_eq!(parse_length("3rch", false), Some(Length::Rch(3.0)));
}

#[test]
fn parse_length_value_accepts_ic() {
    // https://www.w3.org/TR/css-values-4/#ic
    assert_eq!(parse_length("1.5ic", false), Some(Length::Ic(1.5)));
}

#[test]
fn parse_length_value_accepts_ric() {
    // https://www.w3.org/TR/css-values-4/#ric
    assert_eq!(parse_length("1.5ric", false), Some(Length::Ric(1.5)));
}

// ── Additional absolute units (CSS Values 4 §6.2) ──

#[test]
fn parse_length_value_accepts_cm() {
    assert_eq!(parse_length("2cm", false), Some(Length::Cm(2.0)));
}

#[test]
fn parse_length_value_accepts_mm() {
    assert_eq!(parse_length("5mm", false), Some(Length::Mm(5.0)));
}

#[test]
fn parse_length_value_accepts_q() {
    // `Q`: the unit token passes through `to_ascii_lowercase()` and dispatches
    // as `"q"`. The case-insensitivity test also covers uppercase `Q`.
    assert_eq!(parse_length("40Q", false), Some(Length::Q(40.0)));
}

#[test]
fn parse_length_value_accepts_in() {
    assert_eq!(parse_length("1in", false), Some(Length::In(1.0)));
}

#[test]
fn parse_length_value_accepts_pc() {
    assert_eq!(parse_length("6pc", false), Some(Length::Pc(6.0)));
}

#[test]
fn parse_length_value_accepts_unitless_zero_only() {
    // CSS Values 3 §5 <https://www.w3.org/TR/css-values-3/#lengths>:
    // "For zero lengths the unit identifier is optional (i.e. can be
    // syntactically represented as the `<number>` 0)." — accept bare `0`
    // as Length::Px(0.0) in either mode (cover both to pin mode independence).
    assert_eq!(parse_length("0", false), Some(Length::Px(0.0)));
    assert_eq!(parse_length("0", true), Some(Length::Px(0.0)));
    // A nonzero unitless number is not a length in the grammar: the `== 0.0` guard
    // branches, then the `_ => None` fall-through drops it. The drop path is
    // independent of mode (both bypass the guard and fall through), so pin one mode.
    assert_eq!(parse_length("5", false), None);
    assert_eq!(parse_length("-1", false), None);
}

#[test]
fn parse_length_value_rejects_non_numeric_token() {
    assert_eq!(parse_length("medium", false), None);
    assert_eq!(parse_length("", false), None);
}

#[test]
fn parse_length_value_preserves_negative_sign() {
    // The helper does not check signs; requirements differ by property
    // (font-size has a nonnegative post-filter; margin accepts negatives).
    assert_eq!(parse_length("-5px", false), Some(Length::Px(-5.0)));
    assert_eq!(parse_length("-1em", false), Some(Length::Em(-1.0)));
}

#[test]
fn parse_length_value_unit_dispatch_case_insensitive() {
    // CSS spec: unit identifiers are ASCII case-insensitive.
    assert_eq!(parse_length("10PX", false), Some(Length::Px(10.0)));
    assert_eq!(parse_length("1.5EM", false), Some(Length::Em(1.5)));
    assert_eq!(parse_length("2Rem", false), Some(Length::Rem(2.0)));
    assert_eq!(parse_length("14Pt", false), Some(Length::Pt(14.0)));
    // All dispatch keys for `unit.to_ascii_lowercase()` are lowercase
    // (`"q"`, `"in"`, etc.): check that uppercase units fold correctly
    // one by one (`Q` is especially easy to get wrong).
    assert_eq!(parse_length("10IN", false), Some(Length::In(10.0)));
    assert_eq!(parse_length("40Q", false), Some(Length::Q(40.0)));
    assert_eq!(parse_length("2CM", false), Some(Length::Cm(2.0)));
    assert_eq!(parse_length("2EX", false), Some(Length::Ex(2.0)));
    assert_eq!(parse_length("2CH", false), Some(Length::Ch(2.0)));
    assert_eq!(parse_length("2IC", false), Some(Length::Ic(2.0)));
}

#[test]
fn parse_length_value_accepts_viewport_percentage_units() {
    // CSS Values 4 §6.1.2 (https://www.w3.org/TR/css-values-4/#viewport-relative-lengths).
    assert_eq!(parse_length("50vw", false), Some(Length::Vw(50.0)));
    assert_eq!(parse_length("100VH", false), Some(Length::Vh(100.0)));
    assert_eq!(parse_length("1vi", false), Some(Length::Vi(1.0)));
    assert_eq!(parse_length("2vb", false), Some(Length::Vb(2.0)));
    assert_eq!(parse_length("3vmin", false), Some(Length::Vmin(3.0)));
    assert_eq!(parse_length("4vmax", false), Some(Length::Vmax(4.0)));
    // Paged media's one viewport makes the small, large and dynamic
    // variants the plain unit.
    assert_eq!(parse_length("5svh", false), Some(Length::Vh(5.0)));
    assert_eq!(parse_length("6lvw", false), Some(Length::Vw(6.0)));
    assert_eq!(parse_length("7dvmax", false), Some(Length::Vmax(7.0)));
    assert_eq!(parse_length("8xvh", false), None);
    assert_eq!(parse_length("9cqh", false), None);
}

#[test]
fn has_viewport_length_finds_units_in_nested_blocks_and_rewinds() {
    let mut input = ParserInput::new("10px max(1px, calc(2vh + 1px)) end");
    let mut parser = Parser::new(&mut input);
    assert!(has_viewport_length(&mut parser));
    assert_eq!(
        parser.next(),
        Ok(&cssparser::Token::Dimension {
            has_sign: false,
            value: 10.0,
            int_value: Some(10),
            unit: "px".into(),
        })
    );
    let mut input = ParserInput::new("10px \"10vh\" vh calc(1em)");
    let mut parser = Parser::new(&mut input);
    assert!(!has_viewport_length(&mut parser));
}

#[test]
fn has_viewport_length_leaves_var_arguments_to_substitution() {
    let mut input = ParserInput::new("var(--m, 10vh)");
    let mut parser = Parser::new(&mut input);
    assert!(!has_viewport_length(&mut parser));
    assert!(has_viewport_length_in("calc(1px + 2vw)"));
    assert!(!has_viewport_length_in("calc(1px + 2em)"));
}
