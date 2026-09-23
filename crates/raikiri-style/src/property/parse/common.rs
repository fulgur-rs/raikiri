//! Numeric, length and identifier helpers shared by several property parser domains.

use cssparser::{BasicParseError, ParseError, Parser, Token};
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

/// `<length>` / `<length-percentage>` の共通 parser。1 token を consume する。
///
/// Grammar reference: CSS Values 4 §6 <https://www.w3.org/TR/css-values-4/#lengths>
/// / §5.5 <https://www.w3.org/TR/css-values-4/#percentages>.
///
/// # Mode selector
///
/// `allow_percentage` は `%` (`Token::Percentage`) token の受理有無のみを
/// 分岐する — dimension unit (`px` 等) の受理集合は分岐に依存しない (下の
/// `Token::Dimension` match arm 参照、両 mode で同一集合を受理する)。
/// - `false` → `<length>` mode: `%` を受理しない。
/// - `true` → `<length-percentage>` mode: `%` も受理する。
///
/// The `Token::Dimension` match below is the source of truth for supported
/// units; unsupported units are rejected.
///
/// # Unitless zero
///
/// CSS Values 3 §5 "Distance Units: the `<length>` type"
/// <https://www.w3.org/TR/css-values-3/#lengths> verbatim: "For zero lengths
/// the unit identifier is optional (i.e. can be syntactically represented as the
/// `<number>` 0)." — bare `0` (Token::Number, value == 0.0) を [`Length::Px`]
/// `(0.0)` として受理する (mode 非依存: `<length>` / `<length-percentage>` 両方)。
/// 非零 unitless number (`5`, `-1` etc.) は grammar 上 `<length>` にならないため
/// 引き続き drop する (`== 0.0` guard で判定)。
///
/// 同 spec §5 clause 2: "if a 0 could be parsed as either a `<number>` or a
/// `<length>` in a property (such as line-height), it must parse as a `<number>`"
/// — [`parse_line_height`](super::text::parse_line_height) は本 helper より先に `expect_number` branch を試すため
/// 該当分岐は `LineHeight::Number(0.0)` を返し、本 helper 経由の `Length::Px(0.0)`
/// には落ちない (spec-required disambiguation)。
///
/// # Sign / range
///
/// 本 helper は sign / range check を行わない — property ごとに要件が異なるため
/// (padding は non-negative、margin は negative 許容、etc.)。caller 側で
/// post-filter する ([`parse_font_size`](super::text::parse_font_size) は **全 [`Length`] variant** の payload に
/// 対して `>= 0.0` を確認する)。
///
/// # `allow_percentage=true` の caller
///
/// forward-provisioning として導入した mode だが、現在は 7 caller が使用する:
/// [`parse_margin_side`](super::box_model::parse_margin_side) / [`parse_padding_side`](super::box_model::parse_padding_side) / [`parse_width`](super::box_model::parse_width) /
/// [`parse_height`](super::box_model::parse_height) / [`parse_line_height`](super::text::parse_line_height) / [`parse_font_size`](super::text::parse_font_size) /
/// [`parse_text_indent`](super::text::parse_text_indent)。いずれも
/// grammar が spec で `<length-percentage>` を含む
/// (`font-size` は元は `<length>` 限定だったが後に拡張)。共通 helper 化により
/// 重複 dimension unit dispatch を回避している。
///
/// `allow_percentage=false` (= `<length>` mode) の caller は
/// [`parse_border_width_side`](super::box_model::parse_border_width_side) / [`parse_letter_or_word_spacing`](super::text::parse_letter_or_word_spacing) —
/// 前者は CSS Backgrounds 3 §3.3 の `<line-width>` grammar が `<percentage>`
/// を含まないため、後者は CSS Text 3 §7.1/§7.2 の `letter-spacing` /
/// `word-spacing` grammar が共に "Percentages: N/A" と明記するため。
///
/// # Percentage overflow
///
/// `Token::Percentage.unit_value` は f64→f32 変換済 (cssparser 0.37
/// tokenizer が `value / 100.0` を emit) だが、[`Length::Percent`] は
/// authored number (`50%` → `50.0`) を保持する設計のため、本 helper 側で
/// `unit_value * 100.0` の逆変換を行う。`unit_value` 自体が f32 有限範囲に
/// 収まっていても (例 `1e40%` → cssparser 側は `1e38` で有限)、この
/// ×100.0 の逆変換それ自体が f32 overflow を起こしうる (`1e38 * 100.0` は
/// f32 の有限範囲 `3.4028235e38` を超えて `+Inf`)。CSS Values 4 §5 "Range
/// Checking and Precision for Numeric Types"
/// <https://www.w3.org/TR/css-values-4/#numeric-types> の "it must be
/// converted to the closest value supported by the implementation" に従い、
/// `±Inf` になった場合のみ、符号を保持しつつ `f32::MAX` へ寄せる。
///
/// 既存の sink-guard precedent (「guard は sink 境界に
/// 置く、parse/resolve 層には置かない」) はここには適用しない —
/// 本件は guard ではなく変換の正確さの問題
/// (specified 層の値そのものが CSS Values 4 §5 の要求から外れている)
/// であり、precedent とは別軸。`raikiri-dom::layout::sanitize_finite`
/// (resolve 後の geometry に対する sink guard) は本変更後も引き続き必要。
///
/// **`NaN` はこの saturation の対象外**。`is_finite()` は `NaN` に対しても
/// `false` を返すため、当初の実装は `NaN` も `±f32::MAX` へ saturate して
/// いたが、それは誤り: 例えば `0e999%` は cssparser の tokenizer が計算する
/// `0.0 * 10^999` (`f64::powf` が `+Inf` を返す) の中間結果としては `NaN`
/// になるが、**真の数学的値は 0** の入力であり、"closest value" は
/// `f32::MAX` ではなく `0.0` である。この関数が呼ぶ `next_numeric_stable`
/// (module doc 冒頭「Numeric-token NaN stabilization」節参照) がまさに
/// この class を token 取得の時点で訂正するため、通常の parse では
/// `unit_value` がこの arm に `NaN` のまま届くことはもう無く、`0e999%` は
/// この saturation 分岐を経由せずそのまま `Length::Percent(0.0)` になる。
/// それでも `NaN` を saturate 対象から除外する条件分岐 (`is_infinite()`
/// 限定) 自体は defense-in-depth として残す —
/// `sanitize_finite` (`raikiri-dom/src/layout.rs`) が `NaN` を既に `0.0`
/// として扱う既存の sink 契約と整合するため、`next_numeric_stable` の
/// recovery が (再 parse 失敗などで) 効かなかった残余の `NaN` も
/// `f32::MAX` へ寄せず無変換で通す。`is_infinite()` の saturation 自体は
/// `1e40%` のような正真正銘の magnitude overflow に対して引き続き
/// 必要 (module doc の「同じ collapse は逆方向にも起こりうる」とは別の、
/// 通常の overflow class)。
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
            // `ex`/`ch`/`ic` の real-metric variant は
            // style 層に font metrics が無いため常に spec fallback を使う
            // (`Length::Ex` / `Length::Ch` / `Length::Ic` の doc 参照)。
            "ex" => Some(Length::Ex(*value)),
            "rex" => Some(Length::Rex(*value)),
            "ch" => Some(Length::Ch(*value)),
            "rch" => Some(Length::Rch(*value)),
            "ic" => Some(Length::Ic(*value)),
            "ric" => Some(Length::Ric(*value)),
            // Additional absolute units (CSS Values 4 §6.2).
            // `unit` は `to_ascii_lowercase()` 済 —
            // `Q` トークンも `"q"` として届く。
            "cm" => Some(Length::Cm(*value)),
            "mm" => Some(Length::Mm(*value)),
            "q" => Some(Length::Q(*value)),
            "in" => Some(Length::In(*value)),
            "pc" => Some(Length::Pc(*value)),
            // `lh` / `rlh` (CSS Values 4 §6.1.1).
            // Accepted generally here for every consumer, `font-size` included
            // (moved out of `parse_font_size`'s former
            // post-filter — see that function's doc "`lh` / `rlh` は受理し、
            // 親基準で解決する" section for the self-reference resolution).
            "lh" => Some(Length::Lh(*value)),
            "rlh" => Some(Length::Rlh(*value)),
            // (b) 非対応 — viewport-relative unit (`vw`/`vh`/…) と
            // `cap`/`rcap` は未対応、silent drop。両者とも specified 層だけ
            // では正しく resolve できない (viewport size / font ascent が
            // style 層に存在しない) ため follow-up task へ切り出し済。
            //
            // この arm はそれ以外の全 unrecognized unit (例:
            // container-query unit `cqw`/`cqh`/`cqi`/`cqb`/`cqmin`/`cqmax` —
            // CSS Contain 3 §6 <https://www.w3.org/TR/css-contain-3/#container-lengths>、
            // container size も viewport size 同様 style 層に存在しない)
            // も等しく drop する。本 comment が「未対応 unit の一覧」の
            // canonical source になった以上、この一覧を書き足す形の
            // 重複記述はしないこと。
            _ => None,
        },
        Token::Percentage { unit_value, .. } if allow_percentage => {
            // authored-number 逆変換 + overflow saturation: 上の
            // "# Percentage overflow" section 参照。
            let percent = *unit_value * 100.0;
            Some(Length::Percent(if percent.is_infinite() {
                f32::MAX.copysign(percent)
            } else {
                percent
            }))
        }
        // CSS Values 3 §5 unitless-zero clause (doc "# Unitless zero" 参照)。
        Token::Number { value, .. } if *value == 0.0 => Some(Length::Px(0.0)),
        _ => None,
    }
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
/// unrestricted-sign shape [`parse_letter_or_word_spacing`](super::text::parse_letter_or_word_spacing) already uses for
/// `letter-spacing` / `word-spacing`, unlike this module's sibling
/// [`parse_non_negative_length`] (`size`'s `<length>` alternative, which the
/// spec instead states is `[0,∞]`).
pub(crate) fn parse_length_allow_negative(input: &mut Parser<'_, '_>) -> Option<Length> {
    parse_length_value(input, false)
}

/// `<custom-ident>` (CSS Values 4 §4.2
/// <https://www.w3.org/TR/css-values-4/#custom-idents>): CSS-wide keyword と
/// `default` を除いた任意 ident。case-preserving、smol str で保持。
///
/// `none` はここでは除外しない。spec verbatim: "Specifications using
/// `<custom-ident>` must specify clearly what other keywords are excluded
/// from `<custom-ident>`, if any…" と述べるとおり、より狭い grammar
/// (`<counter-name>` 等) の追加除外は個別の predicate (例
/// [`is_reserved_counter_name`](super::content::is_reserved_counter_name)) 側の責務。[`is_reserved_custom_ident`] の
/// docstring も参照。
///
/// **呼び出し元は当初 3 箇所**: `string()` の name 引数 ([`parse_string_fn`](super::content::parse_string_fn))、
/// `target-counter()` / `target-counters()` の第 2 引数
/// ([`parse_target_counter_fn`](super::content::parse_target_counter_fn) / [`parse_target_counters_fn`](super::content::parse_target_counters_fn))。いずれも spec 上
/// `<custom-ident>` を取り `none` は valid。
///
/// 後に `pub(crate)` に広げ、`counter_style` module が
/// `<counter-style-name>` (CSS Counter Styles L3 §3
/// <https://www.w3.org/TR/css-counter-styles-3/#typedef-counter-style-name> —
/// `<custom-ident>` に `none` 追加除外を足した production、`<symbol>` の
/// `<custom-ident>` alternative 等) の base として同じ CSS-wide keyword 除外
/// list を再利用する 4 箇所目の呼び出し元になった (`is_reserved_custom_ident`
/// の list を二重管理しないため — 本 crate の drift 回避規約、
/// [`crate::page::PageCascadeResult::declarations`] doc 同旨)。
///
/// `<counter-name>` を取る `counter()` / `counters()` および counter-* property は
/// **本関数を経由しない** — [`parse_counter_name`](super::content::parse_counter_name) / [`parse_counter_property`](super::content::parse_counter_property) が
/// [`is_reserved_counter_name`](super::content::is_reserved_counter_name) で `none` を追加除外する。したがって本関数に
/// `none` 除外を足してはならない (足すと `target-counter(url(#a), none)` と
/// `string(none)` を spec に反して reject する)。
pub(crate) fn parse_custom_ident(input: &mut Parser<'_, '_>) -> Option<SmolStr> {
    let ident = input.expect_ident().ok()?.clone();
    if is_reserved_custom_ident(&ident) {
        None
    } else {
        Some(SmolStr::new(ident.as_ref()))
    }
}

/// `<custom-ident>` 除外リスト (CSS Values 4 §4.2
/// <https://www.w3.org/TR/css-values-4/#custom-idents>)。
///
/// CSS-wide keyword (`inherit` / `initial` / `unset` / `revert` /
/// `revert-layer`) と `default` のみを弾く。`none` はここでは除外せず、
/// より狭い grammar (`<counter-name>` 等) の追加除外は個別の predicate
/// (例 [`is_reserved_counter_name`](super::content::is_reserved_counter_name)) で行う。case-insensitive 比較。
///
/// `pub(crate)`: `counter_style` module が
/// `<counter-style-name>` 系 production (rule name / `fallback` / `system:
/// extends`) の除外 predicate を組み立てる際にこの base list を再利用する
/// ([`parse_custom_ident`] の doc 参照)。
///
/// これは CSS Values 4 §4.2 の permanent な spec 除外規定であり、**CSS-wide
/// keyword の実装状況とは無関係** — [`PropertyValue`] doc の「CSS-wide keyword」節
/// が説明する「property value としては未実装」claim
/// とは別の話なので混同しないこと。
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

/// `<length-percentage [0,∞]>` — [`parse_length_percentage_res`](super::visual::parse_length_percentage_res) に
/// non-negative filter ([`Length::payload`] による `[0,∞]` pattern) を足した
/// もの。`radial-gradient()`のellipse 2-radii form
/// (`<length-percentage [0,∞]>{2}`、[`RadialSize::Ellipse`]) が使う。
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
