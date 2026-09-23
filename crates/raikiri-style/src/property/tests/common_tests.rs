//! Tests for the shared value helpers in `parse/common.rs`.

use super::*;

// ── parse_length_value helper ────────────────────
//
// helper 単体を叩く共通 fixture — property dispatcher (`parse_value`) を経由せず
// 5 unit sample (`px` / `em` / `rem` / `%` / `pt`) の parse を直接 verify する。
// `parse_font_size` 経由 test は上流に既存 (`font_size_parse_px` 等)、そちらは
// px-only post-filter を verify するので分離する。

fn parse_length(source: &str, allow_percentage: bool) -> Option<Length> {
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    parse_length_value(&mut parser, allow_percentage)
}

#[test]
fn parse_length_value_accepts_px() {
    assert_eq!(parse_length("10px", false), Some(Length::Px(10.0)));
    // length-percentage mode でも px 受理 (mode 非依存)。
    assert_eq!(parse_length("10px", true), Some(Length::Px(10.0)));
}

#[test]
fn parse_length_value_accepts_em() {
    // CSS Values 4 §6.1.1 em (https://www.w3.org/TR/css-values-4/#em):
    // authored `1.2em` を Length::Em(1.2) にそのまま保持 (resolve は下流責務)。
    assert_eq!(parse_length("1.2em", false), Some(Length::Em(1.2)));
}

#[test]
fn parse_length_value_accepts_rem() {
    // CSS Values 4 §6.1.1 rem (https://www.w3.org/TR/css-values-4/#rem):
    // root element の font-size 基準、authored value を Length::Rem に格納。
    assert_eq!(parse_length("1rem", false), Some(Length::Rem(1.0)));
}

#[test]
fn parse_length_value_accepts_pt() {
    // CSS Values 4 §6.2 absolute lengths (https://www.w3.org/TR/css-values-4/#absolute-lengths):
    // 1pt = 1/72 in, 1in = 96px、resolve 側で 12pt → 16px 相当に変換。
    assert_eq!(parse_length("12pt", false), Some(Length::Pt(12.0)));
}

#[test]
fn parse_length_value_accepts_percentage_when_allowed() {
    // CSS Values 4 §5.5 (https://www.w3.org/TR/css-values-4/#percentages):
    // `<length-percentage>` mode でのみ受理。cssparser `unit_value = 0.5` を
    // × 100.0 で authored `50` に戻して Length::Percent(50.0) に格納。
    assert_eq!(parse_length("50%", true), Some(Length::Percent(50.0)));
}

#[test]
fn parse_length_value_extreme_percentage_saturates_to_f32_max_not_inf() {
    // `1e40%` は cssparser tokenizer 側で
    // `unit_value = 1e40 / 100.0 = 1e38` (f32 有限範囲 `3.4028235e38` 内)
    // になるが、authored number へ戻す本 helper の `× 100.0` 自体が
    // f32 overflow を起こし +Inf を作っていた (fix 前)。
    //
    // CSS Values 4 §5 "Range Checking and Precision for Numeric Types"
    // <https://www.w3.org/TR/css-values-4/#numeric-types>:
    // "When a value cannot be explicitly supported due to
    // range/precision limitations, it must be converted to the closest
    // value supported by the implementation" — 非有限は許容されないため、
    // 符号を保持しつつ f32::MAX に寄った有限値を check する。
    assert_eq!(parse_length("1e40%", true), Some(Length::Percent(f32::MAX)));
    // 符号保持も合わせて check (負の overflow は -f32::MAX へ)。
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
    // `<length>` mode (font-size 等) では `%` は grammar 外、None を返す。
    assert_eq!(parse_length("50%", false), None);
}

#[test]
fn parse_length_value_rejects_unsupported_unit() {
    // (b) 非対応 — viewport-relative unit / `cap` / `rcap` は本 helper で
    // 引き続き silent drop。`lh` / `rlh` は受理側へ移った
    // (下記 `parse_length_value_accepts_lh` / `_rlh` を参照)。
    assert_eq!(parse_length("10vw", false), None);
    assert_eq!(parse_length("1cap", true), None);
    // container-query unit (CSS Contain 3 §6) — `_` arm 直前 comment が
    // 挙げる `cq*` 一覧をこの assertion で check する。comment のみで
    // test 未網羅だと、将来 `cq*` 対応 arm が誤って追加されても
    // どの test も落ちず canonical comment が silent に stale 化する
    // (spec-lens follow-up として追加)。
    assert_eq!(parse_length("10cqw", false), None);
}

#[test]
fn parse_length_value_accepts_lh() {
    // https://www.w3.org/TR/css-values-4/#lh — authored value をそのまま保持。
    assert_eq!(parse_length("1.5lh", false), Some(Length::Lh(1.5)));
}

#[test]
fn parse_length_value_accepts_rlh() {
    // https://www.w3.org/TR/css-values-4/#rlh
    assert_eq!(parse_length("2rlh", false), Some(Length::Rlh(2.0)));
}

// ── 追加 font-relative unit (CSS Values 4 §6.1.1) ──

#[test]
fn parse_length_value_accepts_ex() {
    // https://www.w3.org/TR/css-values-4/#ex — authored value をそのまま保持。
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

// ── 追加 absolute unit (CSS Values 4 §6.2) ──

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
    // `Q` — unit token は `to_ascii_lowercase()` を経て `"q"` として dispatch
    // される。case-insensitivity test でも uppercase `Q` を確認する。
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
    // syntactically represented as the `<number>` 0)." — bare `0` は
    // mode 非依存で Length::Px(0.0) 受理 (両 mode 網羅で mode-independence pin)。
    assert_eq!(parse_length("0", false), Some(Length::Px(0.0)));
    assert_eq!(parse_length("0", true), Some(Length::Px(0.0)));
    // 非零 unitless number は grammar 上 length ではない — `== 0.0` guard で
    // 分岐して下段 `_ => None` fallthrough で drop。drop 経路は mode 非依存
    // (guard を通らず fallthrough する path が両 mode 共通) のため 1 mode で pin。
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
    // helper は sign check を行わない — property ごとに要件が異なるため
    // (font-size は non-negative post-filter、margin は negative 許容)。
    assert_eq!(parse_length("-5px", false), Some(Length::Px(-5.0)));
    assert_eq!(parse_length("-1em", false), Some(Length::Em(-1.0)));
}

#[test]
fn parse_length_value_unit_dispatch_case_insensitive() {
    // CSS spec: unit identifier は ASCII case-insensitive。
    assert_eq!(parse_length("10PX", false), Some(Length::Px(10.0)));
    assert_eq!(parse_length("1.5EM", false), Some(Length::Em(1.5)));
    assert_eq!(parse_length("2Rem", false), Some(Length::Rem(2.0)));
    assert_eq!(parse_length("14Pt", false), Some(Length::Pt(14.0)));
    // `unit.to_ascii_lowercase()` の dispatch key はすべて lowercase
    // (`"q"` / `"in"` 等) — uppercase 単位が正しく畳み込まれることを
    // 個別に確認する (`Q` は特に取り違えやすい)。
    assert_eq!(parse_length("10IN", false), Some(Length::In(10.0)));
    assert_eq!(parse_length("40Q", false), Some(Length::Q(40.0)));
    assert_eq!(parse_length("2CM", false), Some(Length::Cm(2.0)));
    assert_eq!(parse_length("2EX", false), Some(Length::Ex(2.0)));
    assert_eq!(parse_length("2CH", false), Some(Length::Ch(2.0)));
    assert_eq!(parse_length("2IC", false), Some(Length::Ic(2.0)));
}
