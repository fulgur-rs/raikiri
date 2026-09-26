//! Tests for the text and font property parsers in `parse/text.rs`.

use super::*;

#[test]
fn line_break_anywhere_parses_as_a_cascadable_keyword() {
    assert_eq!(
        parse_entire("anywhere", "line-break"),
        Some(PropertyValue::LineBreak(LineBreak::Anywhere)),
    );
}

#[test]
fn font_size_parse_px() {
    assert_eq!(
        parse("16px", "font-size"),
        Some(PropertyValue::FontSize(Length::Px(16.0)))
    );
}

/// CSS Fonts 4 §2.5 の grammar `<length-percentage [0,∞]>` は font-relative
/// unit と percentage を含む。cascade の phase 2 (絶対化) が入ったので、
/// これらを parse 段で drop しなくなった。
#[test]
fn font_size_accepts_font_relative_and_percentage() {
    assert_eq!(
        parse("1.5em", "font-size"),
        Some(PropertyValue::FontSize(Length::Em(1.5)))
    );
    assert_eq!(
        parse("2rem", "font-size"),
        Some(PropertyValue::FontSize(Length::Rem(2.0)))
    );
    assert_eq!(
        parse("12pt", "font-size"),
        Some(PropertyValue::FontSize(Length::Pt(12.0)))
    );
    assert_eq!(
        parse("150%", "font-size"),
        Some(PropertyValue::FontSize(Length::Percent(150.0)))
    );
}

/// `math` は spec-valid だが未実装
/// (MathML scaling algorithm 未対応) として drop。
/// `<absolute-size>` / `<relative-size>` は受理済み —
/// 別 test (`font_size_accepts_absolute_size_keywords` /
/// `font_size_accepts_relative_size_keywords`) 参照。
#[test]
fn font_size_rejects_math_keyword() {
    assert_eq!(parse("math", "font-size"), None);
}

/// CSS Fonts 4 §2.5.1 <https://www.w3.org/TR/css-fonts-4/#absolute-size-mapping>
/// の scaling-factor table 全 8 keyword。`medium` = raikiri の固定基準
/// (16px) そのもの、他は table の分数を掛けたもの
/// (`resolve_relative_weight` 前例に倣い浮動小数 literal ではなく分数式で
/// 期待値を書く — 丸め誤差の議論を spec 引用だけで閉じるため)。
#[test]
fn font_size_accepts_absolute_size_keywords() {
    const MEDIUM: f32 = 16.0;
    let cases: &[(&str, f32)] = &[
        ("xx-small", MEDIUM * (3.0 / 5.0)),
        ("x-small", MEDIUM * (3.0 / 4.0)),
        ("small", MEDIUM * (8.0 / 9.0)),
        ("medium", MEDIUM),
        ("large", MEDIUM * (6.0 / 5.0)),
        ("x-large", MEDIUM * (3.0 / 2.0)),
        ("xx-large", MEDIUM * (2.0 / 1.0)),
        ("xxx-large", MEDIUM * (3.0 / 1.0)),
    ];
    for (keyword, px) in cases {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            parse(keyword, "font-size"),
            Some(PropertyValue::FontSize(Length::Px(*px))),
            "keyword = {keyword}"
        );
    }
}

/// CSS Values 3 §3.1 "Pre-defined Keywords": keyword は ASCII
/// case-insensitive。sibling `font_weight_keyword_case_insensitive` と同 pattern。
#[test]
fn font_size_absolute_size_keyword_case_insensitive() {
    assert_eq!(
        parse("MEDIUM", "font-size"),
        Some(PropertyValue::FontSize(Length::Px(16.0)))
    );
    assert_eq!(
        parse("Large", "font-size"),
        Some(PropertyValue::FontSize(Length::Px(16.0 * (6.0 / 5.0))))
    );
}

/// `<relative-size>` (`larger` / `smaller`) は parse 段では解決せず
/// `PropertyValue::FontSizeRelative` をそのまま返す — 解決 (親の
/// computed font-size に対する read-modify-write) は
/// `crate::cascade` の責務 (`bolder` / `lighter` と同型)。 // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests (#[test]-item doc) — rustdoc-blind, confirmed via わざと壊して確かめる
#[test]
fn font_size_accepts_relative_size_keywords() {
    assert_eq!(
        parse("larger", "font-size"),
        Some(PropertyValue::FontSizeRelative(RelativeFontSize::Larger))
    );
    assert_eq!(
        parse("smaller", "font-size"),
        Some(PropertyValue::FontSizeRelative(RelativeFontSize::Smaller))
    );
    assert_eq!(
        parse("LARGER", "font-size"),
        Some(PropertyValue::FontSizeRelative(RelativeFontSize::Larger))
    );
}

/// `font-size: 12px` と `font-size: larger` は同じ property を競合する
/// (`PropertyValue::FontSizeRelative` doc 参照) — 別 key だと両方が
/// cascade で「勝つ」事態が起き spec (1 property = 1 winner) と食い違う。
#[test]
fn font_size_relative_shares_property_key_with_font_size() {
    assert_eq!(
        PropertyValue::FontSize(Length::Px(12.0)).key(),
        PropertyKey::FontSize
    );
    assert_eq!(
        PropertyValue::FontSizeRelative(RelativeFontSize::Larger).key(),
        PropertyKey::FontSize
    );
}

#[test]
fn font_size_rejects_negative() {
    // spec grammar `[0,∞]`: 負値は全 unit で drop (px だけではない)。
    assert_eq!(parse("-10px", "font-size"), None);
    assert_eq!(parse("-0.5px", "font-size"), None);
    assert_eq!(parse("-1em", "font-size"), None);
    assert_eq!(parse("-2rem", "font-size"), None);
    assert_eq!(parse("-12pt", "font-size"), None);
    assert_eq!(parse("-50%", "font-size"), None);
    // 追加した unit も `Length::payload` 経由で同じ
    // non-negative check を通ることを pin。
    assert_eq!(parse("-1ex", "font-size"), None);
    assert_eq!(parse("-1cm", "font-size"), None);
}

#[test]
fn font_size_accepts_additional_units() {
    // CSS Fonts 4 §2.5 `<length-percentage [0,∞]>` —
    // 追加した font-relative / absolute unit も `font-size` 上で受理される
    // (`parse_length_value` の dispatch に mode 差は無い)。
    assert_eq!(
        parse("2ex", "font-size"),
        Some(PropertyValue::FontSize(Length::Ex(2.0)))
    );
    assert_eq!(
        parse("1cm", "font-size"),
        Some(PropertyValue::FontSize(Length::Cm(1.0)))
    );
}

#[test]
fn font_size_accepts_lh_and_rlh() {
    // CSS Fonts 4's `font-size` grammar
    // (`<absolute-size> | <relative-size> | <length-percentage [0,∞]>`)
    // has no carve-out excluding `lh`/`rlh` from `<length-percentage>`'s
    // `<length>` component (CSS Values 4 §6.1.1) — `font-size: 1lh` /
    // `font-size: 1rlh` are spec-valid and must survive parsing so the
    // cascade can pick them as a winner (dropping at parse time, as this
    // crate previously did, can change *which
    // declaration wins* the cascade — a stronger effect than an
    // incorrectly-resolved value). Resolution against the parent's used
    // line-height is `crate::resolve::resolve_font_size`'s concern, not
    // this parser's — pinned by that module's tests, not here.
    assert_eq!(
        parse("1lh", "font-size"),
        Some(PropertyValue::FontSize(Length::Lh(1.0)))
    );
    assert_eq!(
        parse("1rlh", "font-size"),
        Some(PropertyValue::FontSize(Length::Rlh(1.0)))
    );
}

#[test]
fn font_size_rejects_negative_lh_and_rlh() {
    // The grammar's `[0,∞]` non-negative constraint (`parse_font_size`
    // doc "Non-negative constraint" 節) applies to `lh`/`rlh` the same as
    // every other `Length` variant — `Length::payload` reads their inner
    // `f32` generically, so this falls out of the existing post-filter
    // without a dedicated branch.
    assert_eq!(parse("-1lh", "font-size"), None);
    assert_eq!(parse("-1rlh", "font-size"), None);
}

#[test]
fn font_size_accepts_zero() {
    // spec `[0,∞]` の閉区間下端。`0px` は Dimension arm、bare `0` は
    // CSS Values 3 §5 unitless-zero clause の Number arm を通し、
    // parse_font_size の非負 Px post-filter を pass。
    assert_eq!(
        parse("0px", "font-size"),
        Some(PropertyValue::FontSize(Length::Px(0.0)))
    );
    assert_eq!(
        parse("0", "font-size"),
        Some(PropertyValue::FontSize(Length::Px(0.0)))
    );
}

#[test]
fn font_family_parse_comma_list() {
    let got = parse(r#"Arial, "Times New Roman", serif"#, "font-family");
    let expected = Some(PropertyValue::FontFamily(Arc::new(vec![
        FontFamilyName::named("Arial"),
        FontFamilyName::named("Times New Roman"),
        FontFamilyName::generic("serif"),
    ])));
    assert_eq!(got, expected);
}

#[test]
fn font_family_preserves_quoted_generic_keyword_as_named_family() {
    let got = parse(r#""serif""#, "font-family");
    assert_eq!(
        got,
        Some(PropertyValue::FontFamily(Arc::new(vec![
            FontFamilyName::named("serif"),
        ])))
    );
}

#[test]
fn font_family_unquoted_multi_word_single_family() {
    // CSS4: unquoted multi-word family name = ident sequence joined by space。
    let got = parse("Times New Roman", "font-family");
    let expected = Some(PropertyValue::FontFamily(Arc::new(vec![
        FontFamilyName::named("Times New Roman"),
    ])));
    assert_eq!(got, expected);
}

/// `initial_font_family()` は呼び出しごとに独立した call site でも
/// **同一** underlying `Vec` allocation を指す (`Arc::ptr_eq` = true) —
/// `OnceLock` 経由の shared slot であることの直接 pin。
///
/// この check は cascade level の test (`mod@crate::cascade` の
/// `initial_font_family_shares_arc_slot_across_independent_cascade_runs`
/// 等) では**代替できない** — `font-family` は inherited なので、単一
/// document 内の兄弟 element は `SpecifiedValues::inherit_from` の
/// 「親の Arc を bump」経路で共有される。これは同 document 内で
/// `initial_font_family()` が実質 1 回しか呼ばれないことを意味し、
/// ここで `OnceLock` を外して per-call `Arc::new(..)` に戻す regression を
/// 混入させても、その cascade level test は green のままになる
/// (実際に perturbation で確認済み)。
/// 本 test は `initial_font_family()` を直接 2 回呼ぶことで、この
/// inheritance-sharing の死角を回避する。
#[test]
fn initial_font_family_shares_arc_slot_across_calls() {
    assert!(Arc::ptr_eq(&initial_font_family(), &initial_font_family()));
}

/// `font-weight` の parse 期待値を組み立てる test-local helper。
fn fw(w: f32) -> Option<PropertyValue> {
    Some(PropertyValue::FontWeight(FontWeightValue::Absolute(w)))
}

#[test]
fn font_weight_parse_integer() {
    assert_eq!(parse("400", "font-weight"), fw(400.0));
    assert_eq!(parse("700", "font-weight"), fw(700.0));
}

#[test]
fn font_weight_parse_keyword_normal() {
    // CSS Fonts 4 §2.2: normal = 400。
    assert_eq!(parse("normal", "font-weight"), fw(400.0));
}

#[test]
fn font_weight_parse_keyword_bold() {
    // CSS Fonts 4 §2.2: bold = 700。
    assert_eq!(parse("bold", "font-weight"), fw(700.0));
}

#[test]
fn font_weight_keyword_case_insensitive() {
    // CSS Values 3 §3.1 "Pre-defined Keywords": keyword は ASCII
    // case-insensitive で照合する。
    assert_eq!(parse("NORMAL", "font-weight"), fw(400.0));
    assert_eq!(parse("Bold", "font-weight"), fw(700.0));
}

#[test]
fn font_weight_accepts_full_spec_range() {
    // CSS Fonts 4 §2.2 `<font-weight-absolute> = [ normal | bold |
    // <number [1,1000]> ]`。旧実装は `[100, 900]` に絞っていたが spec は
    // `[1, 1000]`。
    assert_eq!(parse("1", "font-weight"), fw(1.0));
    assert_eq!(parse("1000", "font-weight"), fw(1000.0));
    assert_eq!(parse("50", "font-weight"), fw(50.0));
    // 旧 range の両端も当然 valid のまま (regression guard)。
    assert_eq!(parse("100", "font-weight"), fw(100.0));
    assert_eq!(parse("900", "font-weight"), fw(900.0));
}

#[test]
fn font_weight_rejects_out_of_range_number() {
    // spec-invalid — spec grammar。§2.2 "Only values greater than or
    // equal to 1, and less than or equal to 1000, are valid, and all other
    // values are invalid"。
    assert_eq!(parse("0", "font-weight"), None);
    assert_eq!(parse("1001", "font-weight"), None);
    assert_eq!(parse("-100", "font-weight"), None);
    // 範囲判定は **丸める前の指定値** に対して行う: 丸めれば範囲内に入る
    // 値でも spec 上は invalid。
    assert_eq!(parse("0.6", "font-weight"), None);
    assert_eq!(parse("1000.4", "font-weight"), None);
    // 非有限値。`1e400` は f32 に収まらず ±inf に overflow するため、
    // `value <= 1000.0` (または `>= 1.0`) が成立せず reject される
    // (§2.2 "all other values are invalid" と一致) — これは genuine
    // magnitude overflow であり、`next_numeric_stable` が訂正する
    // zero-mantissa/huge-mantissa 由来の `NaN` collapse とは別の hazard
    // class (`parse_font_weight` doc参照)。`nan` / `inf` は `<number>`
    // production ではなく Ident token なので keyword arm にも該当せず
    // reject される — cssparser tokenizer の `NaN` artifact とは無関係。
    assert_eq!(parse("1e400", "font-weight"), None);
    assert_eq!(parse("-1e400", "font-weight"), None);
    assert_eq!(parse("nan", "font-weight"), None);
    assert_eq!(parse("inf", "font-weight"), None);
}

#[test]
fn font_weight_mirror_huge_mantissa_tiny_exponent_resolves_inside_valid_range() {
    // Same mirror-collapse class as
    // `parse_length_value_mirror_huge_mantissa_tiny_exponent_resolves_correctly`
    // — a mantissa long enough to itself overflow to `+Infinity` during
    // cssparser's digit-by-digit accumulation (`5` followed by 400
    // zeros), multiplied by a sufficiently negative exponent's
    // `10^exponent` (which underflows to `0.0`), collapses to
    // `Infinity * 0.0` = `NaN` in the tokenizer's own two-step
    // computation — even though the true value, `5 * 10^400 * 10^-399`
    // = `50`, lands squarely inside `<font-weight-absolute>`'s valid
    // `[1,1000]` range (CSS Fonts 4 §2.2). `next_numeric_stable`
    // (which `parse_font_weight` acquires its token through) recovers
    // this before the range guard ever runs, so this legitimate value
    // resolves to `FontWeightValue::Absolute(50.0)` instead of being
    // silently dropped.
    let digits = format!("5{}", "0".repeat(400)); // 5 * 10^400, overflows f64 digit accumulation to +Infinity
    let source = format!("{digits}e-399");
    assert_eq!(
        parse(&source, "font-weight"),
        Some(PropertyValue::FontWeight(FontWeightValue::Absolute(50.0)))
    );
}

#[test]
fn font_weight_computed_preserves_fractional_precision() {
    // **spec 準拠 pin。** §2.2 の computed value は "a number" であり、
    // §2.2.2 "Missing weights" <https://www.w3.org/TR/css-fonts-4/#missing-weights>
    // は "Fractional weights are valid" と明言する。旧実装 (computed side が
    // `u16`) は parse 時に round-half-away-from-zero で整数化しており、
    // これは spec 沈黙点の選択ではなく表現上の制約による既知 divergence
    // だった。payload / `ComputedValues.font_weight`
    // を `f32` に格上げしたことで丸め自体が不要になり、本 test はその
    // 解消を check する — もはや丸めていないことの regression guard。
    // 全て 2 進数で厳密表現可能な小数 (`.5` / `.25`) — parse 側と期待値の
    // 独立な文字列→f32 変換が bit-for-bit 一致することを保証でき、
    // 丸め誤差を懸念せず `assert_eq!` で直接比較できる。
    assert_eq!(parse("100.5", "font-weight"), fw(100.5));
    assert_eq!(parse("250.75", "font-weight"), fw(250.75));
    assert_eq!(parse("399.5", "font-weight"), fw(399.5));
    assert_eq!(parse("999.5", "font-weight"), fw(999.5));
}

#[test]
fn font_weight_wpt_font_weight_computed_150_25() {
    // WPT css/css-fonts/parsing/font-weight-computed.html:
    // `test_computed_value('font-weight', '150.25')` — 2-arg 形は
    // computed === specified を check する。parse 結果 (specified-equivalent
    // な `PropertyValue`) がそのまま `150.25` を保持することを確認する。
    // cascade を経由した computed 側の同値 check は
    // `crate::cascade::tests::font_weight_wpt_font_weight_computed_150_25`。
    assert_eq!(parse("150.25", "font-weight"), fw(150.25));
}

#[test]
fn font_weight_accepts_scientific_notation_number() {
    // `int_value` matcher から `value` (f32) 参照に変えた副次効果。
    // `1e3` は CSS Values 3 の `<number>` production として spec-valid
    // なので受理が正しい。
    assert_eq!(parse("1e3", "font-weight"), fw(1000.0));
}

#[test]
fn font_weight_parses_relative_keywords_as_sentinels() {
    // CSS Fonts 4 §2.2: `bolder` / `lighter` は継承値依存の relative
    // weight。parse 段では解けないので sentinel variant を返し、cascade が
    // 親の computed weight から解決する。
    assert_eq!(
        parse("bolder", "font-weight"),
        Some(PropertyValue::FontWeight(FontWeightValue::Bolder))
    );
    assert_eq!(
        parse("lighter", "font-weight"),
        Some(PropertyValue::FontWeight(FontWeightValue::Lighter))
    );
    // CSS Values 3 §3.1: relative keyword も ASCII case-insensitive。
    assert_eq!(
        parse("BOLDER", "font-weight"),
        Some(PropertyValue::FontWeight(FontWeightValue::Bolder))
    );
    assert_eq!(
        parse("Lighter", "font-weight"),
        Some(PropertyValue::FontWeight(FontWeightValue::Lighter))
    );
}

#[test]
fn font_weight_rejects_unknown_ident() {
    // spec-invalid keyword → declaration drop。
    assert_eq!(parse("normal-ish", "font-weight"), None);
    assert_eq!(parse("super-bold", "font-weight"), None);
}

// ── line-height (CSS Inline 3 §5.1) ────────────────
//
// Verification 5/6/7 の spec-derived: grammar `normal |
// <number [0,∞]> | <length-percentage [0,∞]>` — 4 accept branch + negative
// reject + Number vs Length variant distinction を check する。
//
// sibling: parse_display (keyword accept)、parse_font_size (Length
// post-filter for non-negative)、parse_length_value (unit dispatch)。

#[test]
fn line_height_parse_normal_keyword() {
    // Verification 5.1: `line-height: normal` → LineHeight::Normal
    assert_eq!(
        parse("normal", "line-height"),
        Some(PropertyValue::LineHeight(LineHeight::Normal))
    );
}

#[test]
fn line_height_parse_bare_number() {
    // Verification 5.2 + 6: `line-height: 1.5` (bare number, no unit) →
    // LineHeight::Number(1.5)。Token::Number arm を通り Length branch には
    // 落ちない (Number vs Length distinction load-bearing、下流 special
    // behavior "specified value inherit" のための variant tag)。
    assert_eq!(
        parse("1.5", "line-height"),
        Some(PropertyValue::LineHeight(LineHeight::Number(1.5)))
    );
}

#[test]
fn line_height_parse_length_px() {
    // Verification 5.3: `line-height: 24px` → LineHeight::Length(Px(24.0))
    assert_eq!(
        parse("24px", "line-height"),
        Some(PropertyValue::LineHeight(LineHeight::Length(Length::Px(
            24.0
        ))))
    );
}

#[test]
fn line_height_parse_length_percentage() {
    // Verification 5.4: `line-height: 150%` → LineHeight::Length(Percent(150.0))
    // parse_length_value(allow_percentage=true) が Percent branch を有効化。
    assert_eq!(
        parse("150%", "line-height"),
        Some(PropertyValue::LineHeight(LineHeight::Length(
            Length::Percent(150.0)
        )))
    );
}

#[test]
fn line_height_number_vs_em_are_distinct_variants() {
    // Verification 6: `1.5` (unitless) と `1.5em` (dimensioned) は同じ scalar
    // でも別 variant に mapping (Token::Number vs Token::Dimension で分岐)。
    // spec §5.1 unitless number は child が specified value を inherit する
    // special behavior、Length variant は通常 resolve — 下流が区別する必要。
    let number = parse("1.5", "line-height");
    let length_em = parse("1.5em", "line-height");
    assert_eq!(
        number,
        Some(PropertyValue::LineHeight(LineHeight::Number(1.5)))
    );
    assert_eq!(
        length_em,
        Some(PropertyValue::LineHeight(LineHeight::Length(Length::Em(
            1.5
        ))))
    );
    assert_ne!(number, length_em, "Number and Length must be distinct");
}

#[test]
fn line_height_accepts_length_em_rem_pt() {
    // 5 unit sample の length-percentage branch smoke — parse_length_value
    // helper との integration を check (em/rem/pt helper 経由)。
    assert_eq!(
        parse("1.2em", "line-height"),
        Some(PropertyValue::LineHeight(LineHeight::Length(Length::Em(
            1.2
        ))))
    );
    assert_eq!(
        parse("1rem", "line-height"),
        Some(PropertyValue::LineHeight(LineHeight::Length(Length::Rem(
            1.0
        ))))
    );
    assert_eq!(
        parse("12pt", "line-height"),
        Some(PropertyValue::LineHeight(LineHeight::Length(Length::Pt(
            12.0
        ))))
    );
}

#[test]
fn line_height_accepts_zero_number_and_length() {
    // spec `[0,∞]`: 0 は境界の valid value。
    // CSS Values 3 §5 <https://www.w3.org/TR/css-values-3/#lengths> clause 2:
    // "if a 0 could be parsed as either a `<number>` or a `<length>` in a
    // property (such as line-height), it must parse as a `<number>`" —
    // parse_line_height は expect_number branch を parse_length_value より
    // 先に試すため、bare `0` は LineHeight::Number(0.0) として確定 (unitless-zero
    // clause の Length 経路が導入した Px(0.0) route ではない)。
    assert_eq!(
        parse("0", "line-height"),
        Some(PropertyValue::LineHeight(LineHeight::Number(0.0)))
    );
    assert_eq!(
        parse("0px", "line-height"),
        Some(PropertyValue::LineHeight(LineHeight::Length(Length::Px(
            0.0
        ))))
    );
}

#[test]
fn line_height_rejects_negative_number() {
    // Verification 7: `<number [0,∞]>` — 負値は spec grammar 違反 → drop。
    assert_eq!(parse("-1.5", "line-height"), None);
}

#[test]
fn line_height_rejects_negative_number_with_trailing_length() {
    // Regression: Number branch は
    // Token::Number を commit した後 fallthrough すべきでない。fallthrough
    // していた旧実装では `-0.5 20px` が Length branch で `20px` を拾い
    // silently accept されていた (spec-invalid → 本来 declaration drop)。
    // 現行: Number 到達 = 確定、`[0,∞]` 違反は declaration drop、
    // 後続 token は expect_exhausted なくとも parse_length_value 側で拾わない。
    assert_eq!(parse("-0.5 20px", "line-height"), None);
    // 対称: negative number + em / % も同じく drop。
    assert_eq!(parse("-0.5 1em", "line-height"), None);
    assert_eq!(parse("-1.0 50%", "line-height"), None);
}

#[test]
fn line_height_rejects_negative_length() {
    // Verification 7: `<length-percentage [0,∞]>` — 負 length は drop。
    assert_eq!(parse("-10px", "line-height"), None);
    assert_eq!(parse("-1em", "line-height"), None);
}

#[test]
fn line_height_rejects_negative_percentage() {
    // Verification 7: 負 percentage も spec `[0,∞]` 違反 → drop。
    assert_eq!(parse("-50%", "line-height"), None);
}

#[test]
fn line_height_rejects_unknown_keyword() {
    // spec grammar 外の ident (`auto` / `medium` 等) は (a) spec-invalid、
    // silent drop。CSS-wide keyword は別 test
    // (`line_height_rejects_css_wide_keyword`) — (a) ではなく (b) の
    // 非対応なので混同しないこと。
    assert_eq!(parse("auto", "line-height"), None);
    assert_eq!(parse("medium", "line-height"), None);
}

#[test]
fn line_height_rejects_css_wide_keyword() {
    // (b) 非対応 — CSS-wide keyword は未実装 (将来対応)、silent drop。
    // canonical: PropertyValue doc「CSS-wide keyword」節。
    assert_eq!(parse("inherit", "line-height"), None);
    assert_eq!(parse("initial", "line-height"), None);
    assert_eq!(parse("unset", "line-height"), None);
    assert_eq!(parse("revert", "line-height"), None);
    assert_eq!(parse("revert-layer", "line-height"), None);
}

#[test]
fn line_height_rejects_unsupported_unit() {
    // parse_length_value が silent drop する unit (`vw` / `cap` 等、
    // 現状未対応) は helper 側で `None` →
    // line-height parse も declaration drop。`ch` / `lh` / `rlh` は
    // それぞれ受理側へ移った
    // (`line_height_accepts_ch` / `line_height_accepts_lh` 参照)。
    assert_eq!(parse("10vw", "line-height"), None);
    assert_eq!(parse("10cap", "line-height"), None);
}

#[test]
fn line_height_accepts_lh() {
    // CSS Values 4 §6.1.1 `lh`/`rlh`.
    // `line-height` itself is a valid context for `lh`/`rlh` at parse
    // time (unlike `font-size`, which `parse_font_size` post-filters —
    // see that function's doc for why) — the self-reference resolve
    // basis is handled downstream in `crate::resolve::resolve_line_height`.
    assert_eq!(
        parse("10lh", "line-height"),
        Some(PropertyValue::LineHeight(LineHeight::Length(Length::Lh(
            10.0
        ))))
    );
    assert_eq!(
        parse("1rlh", "line-height"),
        Some(PropertyValue::LineHeight(LineHeight::Length(Length::Rlh(
            1.0
        ))))
    );
}

#[test]
fn line_height_accepts_ch() {
    assert_eq!(
        parse("2ch", "line-height"),
        Some(PropertyValue::LineHeight(LineHeight::Length(Length::Ch(
            2.0
        ))))
    );
}

#[test]
fn line_height_normal_is_case_insensitive() {
    // CSS spec: keyword ident は ASCII case-insensitive
    // (expect_ident_matching が case-insensitive)。
    assert_eq!(
        parse("NORMAL", "line-height"),
        Some(PropertyValue::LineHeight(LineHeight::Normal))
    );
    assert_eq!(
        parse("Normal", "line-height"),
        Some(PropertyValue::LineHeight(LineHeight::Normal))
    );
}

#[test]
fn line_height_key_maps_to_line_height_property_key() {
    // PropertyValue::LineHeight → PropertyKey::LineHeight (cascade winner 選択の
    // discriminant integrity、既存 sibling font_size / display と同じ pattern)。
    let v = PropertyValue::LineHeight(LineHeight::Normal);
    assert_eq!(v.key(), PropertyKey::LineHeight);
    let v = PropertyValue::LineHeight(LineHeight::Number(1.5));
    assert_eq!(v.key(), PropertyKey::LineHeight);
    let v = PropertyValue::LineHeight(LineHeight::Length(Length::Px(24.0)));
    assert_eq!(v.key(), PropertyKey::LineHeight);
}

// ── text-align (CSS Text 3 §6.1) ──
//
// Value grammar (§6.1 spec verbatim):
//   start | end | left | right | center | justify | match-parent | justify-all
// Initial: start / Inherited: yes / spec 上 shorthand (text-align-all +
// text-align-last、単一 field で保持 = (b)
// 非対応)。inheritance test は cascade.rs 側 (parent → child コピー、display
// non-inherited との対比)。

#[test]
fn text_align_parse_all_eight_keywords() {
    // Verification 5: 8 keyword が全て正しく TextAlign variant にマップされる。
    // 1 test で全 arm coverage (patch coverage 100% 目標)。
    assert_eq!(
        parse("start", "text-align"),
        Some(PropertyValue::TextAlign(TextAlign::Start))
    );
    assert_eq!(
        parse("end", "text-align"),
        Some(PropertyValue::TextAlign(TextAlign::End))
    );
    assert_eq!(
        parse("left", "text-align"),
        Some(PropertyValue::TextAlign(TextAlign::Left))
    );
    assert_eq!(
        parse("right", "text-align"),
        Some(PropertyValue::TextAlign(TextAlign::Right))
    );
    assert_eq!(
        parse("center", "text-align"),
        Some(PropertyValue::TextAlign(TextAlign::Center))
    );
    assert_eq!(
        parse("justify", "text-align"),
        Some(PropertyValue::TextAlign(TextAlign::Justify))
    );
    assert_eq!(
        parse("match-parent", "text-align"),
        Some(PropertyValue::TextAlign(TextAlign::MatchParent))
    );
    assert_eq!(
        parse("-internal-center", "text-align"),
        Some(PropertyValue::TextAlign(TextAlign::InternalCenter))
    );
    assert_eq!(
        parse("justify-all", "text-align"),
        Some(PropertyValue::TextAlign(TextAlign::JustifyAll))
    );
}

#[test]
fn text_align_is_case_insensitive() {
    // Verification 6: CSS spec 慣行 — property value keyword は ASCII case-insensitive。
    assert_eq!(
        parse("CENTER", "text-align"),
        Some(PropertyValue::TextAlign(TextAlign::Center))
    );
    assert_eq!(
        parse("Justify-All", "text-align"),
        Some(PropertyValue::TextAlign(TextAlign::JustifyAll))
    );
    assert_eq!(
        parse("Match-Parent", "text-align"),
        Some(PropertyValue::TextAlign(TextAlign::MatchParent))
    );
}

#[test]
fn text_align_rejects_unknown_keyword() {
    // spec §6.1 grammar に含まれない keyword は silent drop (spec-invalid)。
    // `middle` は typo/俗称、`text-align` spec に存在しない。
    assert_eq!(parse("middle", "text-align"), None);
    assert_eq!(parse("baseline", "text-align"), None);
    assert_eq!(parse("top", "text-align"), None);
}

#[test]
fn text_align_rejects_css_wide_keyword() {
    // (b) 非対応 — CSS-wide keyword は未実装 (将来対応)、silent drop。5 keyword
    // の一覧・理由は `PropertyValue` doc の「CSS-wide keyword」節が canonical。
    assert_eq!(
        parse("inherit", "text-align"),
        Some(PropertyValue::TextAlign(TextAlign::Inherit))
    );
    assert_eq!(parse("initial", "text-align"), None);
    assert_eq!(parse("unset", "text-align"), None);
    assert_eq!(parse("revert", "text-align"), None);
    assert_eq!(parse("revert-layer", "text-align"), None);
}

#[test]
fn text_align_rejects_string_value() {
    // CSS Text 3 §6.1 grammar は 8 keyword のみ、`<string>` value は本 crate
    // が引用する level では未定義 → spec-invalid、silent drop。
    // (Text 4 draft では tabular-data character alignment 用に `<string>` が
    // 検討されているが本 crate は Text 3 pin。expect_ident が String token を
    // reject する経路で `None` を返す。)
    assert_eq!(parse(r#""." "#, "text-align"), None);
}

#[test]
fn text_align_rejects_non_ident() {
    // Number / dimension token は expect_ident で reject。
    assert_eq!(parse("16px", "text-align"), None);
    assert_eq!(parse("100", "text-align"), None);
}

#[test]
fn text_align_key_maps_to_text_align_property_key() {
    // PropertyValue::TextAlign → PropertyKey::TextAlign (cascade winner 選択の
    // discriminant integrity、既存 sibling counter-* / content / string-set /
    // position と同じ pattern)。
    let v = PropertyValue::TextAlign(TextAlign::Start);
    assert_eq!(v.key(), PropertyKey::TextAlign);
    let v = PropertyValue::TextAlign(TextAlign::Center);
    assert_eq!(v.key(), PropertyKey::TextAlign);
}

// ── hanging-punctuation (CSS Text 3 §8.2.1) ──

#[test]
fn hanging_punctuation_parse_implemented_subset() {
    assert_eq!(
        parse_entire("none", "hanging-punctuation"),
        Some(PropertyValue::HangingPunctuation(HangingPunctuation::None))
    );
    assert_eq!(
        parse_entire("first", "hanging-punctuation"),
        Some(PropertyValue::HangingPunctuation(HangingPunctuation::First))
    );
}

#[test]
fn hanging_punctuation_is_case_insensitive_and_rejects_deferred_values() {
    assert_eq!(
        parse_entire("FIRST", "hanging-punctuation"),
        Some(PropertyValue::HangingPunctuation(HangingPunctuation::First))
    );
    assert_eq!(parse_entire("last", "hanging-punctuation"), None);
    assert_eq!(parse_entire("allow-end", "hanging-punctuation"), None);
    assert_eq!(parse_entire("none first", "hanging-punctuation"), None);
}

#[test]
fn hanging_punctuation_key_maps_to_property_key() {
    assert_eq!(
        PropertyValue::HangingPunctuation(HangingPunctuation::None).key(),
        PropertyKey::HangingPunctuation
    );
    assert_eq!(
        PropertyValue::HangingPunctuation(HangingPunctuation::First).key(),
        PropertyKey::HangingPunctuation
    );
}

#[test]
fn hanging_punctuation_serializes_keywords() {
    assert_eq!(
        serialize_value(&PropertyValue::HangingPunctuation(HangingPunctuation::None)),
        Some("none".to_owned())
    );
    assert_eq!(
        serialize_value(&PropertyValue::HangingPunctuation(
            HangingPunctuation::First
        )),
        Some("first".to_owned())
    );
}

// ── text-autospace (CSS Text 4) ──

#[test] // cov:ignore: cfg(test)-only property tests have no lcov source record on the pinned coverage run.
// cov:ignore: cfg(test)-only property tests have no lcov source record on the pinned coverage run.
fn text_autospace_parses_keyword_forms() {
    assert_eq!(
        parse_entire("normal", "text-autospace"),
        Some(PropertyValue::TextAutospace(TextAutospace::Normal))
    );
    assert_eq!(
        parse_entire("auto", "text-autospace"),
        Some(PropertyValue::TextAutospace(TextAutospace::Auto))
    );
    assert_eq!(
        parse_entire("no-autospace", "text-autospace"),
        Some(PropertyValue::TextAutospace(TextAutospace::NoAutospace))
    );
    assert_eq!(
        serialize_value(&PropertyValue::TextAutospace(TextAutospace::Normal)),
        Some("normal".to_owned())
    );
    assert_eq!(
        serialize_value(&PropertyValue::TextAutospace(TextAutospace::Auto)),
        Some("auto".to_owned())
    );
    assert_eq!(
        serialize_value(&PropertyValue::TextAutospace(TextAutospace::NoAutospace)),
        Some("no-autospace".to_owned())
    );
}

#[test] // cov:ignore: cfg(test)-only property tests have no lcov source record on the pinned coverage run.
// cov:ignore: cfg(test)-only property tests have no lcov source record on the pinned coverage run.
fn text_autospace_parses_and_canonicalizes_explicit_forms() {
    let value = parse_entire(
        "punctuation ideograph-alpha ideograph-numeric replace",
        "text-autospace",
    );
    assert_eq!(
        value,
        Some(PropertyValue::TextAutospace(TextAutospace::Custom {
            ideograph_alpha: true,
            ideograph_numeric: true,
            punctuation: true,
            mode: TextAutospaceMode::Replace,
        }))
    );
    assert_eq!(
        serialize_value(&value.expect("valid text-autospace")),
        Some("ideograph-alpha ideograph-numeric punctuation replace".to_owned())
    );
    assert_eq!(
        serialize_value(&PropertyValue::TextAutospace(TextAutospace::Custom {
            ideograph_alpha: false,
            ideograph_numeric: false,
            punctuation: false,
            mode: TextAutospaceMode::None,
        })),
        Some(String::new())
    );
    assert_eq!(
        serialize_value(&PropertyValue::TextAutospace(TextAutospace::Custom {
            ideograph_alpha: false,
            ideograph_numeric: false,
            punctuation: false,
            mode: TextAutospaceMode::Insert,
        })),
        Some("insert".to_owned())
    );
}

#[test] // cov:ignore: cfg(test)-only property tests have no lcov source record on the pinned coverage run.
// cov:ignore: cfg(test)-only property tests have no lcov source record on the pinned coverage run.
fn text_autospace_rejects_duplicate_or_mixed_keyword_forms() {
    for source in [
        "normal ideograph-alpha",
        "auto insert",
        "no-autospace punctuation",
        "ideograph-alpha ideograph-alpha",
        "insert replace",
        "punctuation unknown",
        "ideograph-alpha 1px",
        // Not part of the current grammar; `no-autospace` replaced it.
        "none",
    ] {
        assert_eq!(parse_entire(source, "text-autospace"), None, "{source}");
    }
}

#[test]
fn supported_property_name_registry_is_case_insensitive() {
    assert!(is_supported_property_name("text-autospace"));
    assert!(is_supported_property_name("TEXT-AUTOSPACE"));
    assert!(!is_supported_property_name("not-a-property"));
}

#[test] // cov:ignore: cfg(test)-only property tests have no lcov source record on the pinned coverage run.
// cov:ignore: cfg(test)-only property tests have no lcov source record on the pinned coverage run.
fn text_autospace_key_maps_to_property_key() {
    assert_eq!(
        PropertyValue::TextAutospace(TextAutospace::Normal).key(),
        PropertyKey::TextAutospace
    );
}

// ── text-spacing-trim (CSS Text 4) ──

#[test] // cov:ignore: cfg(test)-only property tests have no lcov source record on the pinned coverage run.
// cov:ignore: cfg(test)-only property tests have no lcov source record on the pinned coverage run.
fn text_spacing_trim_parses_and_serializes_supported_keywords() {
    let cases = [
        ("auto", TextSpacingTrim::Auto),
        ("normal", TextSpacingTrim::Normal),
        ("space-all", TextSpacingTrim::SpaceAll),
        ("trim-both", TextSpacingTrim::TrimBoth),
        ("trim-all", TextSpacingTrim::TrimAll),
        ("trim-start", TextSpacingTrim::TrimStart),
        ("space-first", TextSpacingTrim::SpaceFirst),
    ];
    for (source, expected) in cases {
        let value = PropertyValue::TextSpacingTrim(expected);
        assert_eq!(
            parse_entire(source, "text-spacing-trim"),
            Some(value.clone())
        );
        assert_eq!(serialize_value(&value).as_deref(), Some(source));
    }
    assert_eq!(
        parse_entire("TRIM-START", "text-spacing-trim"),
        Some(PropertyValue::TextSpacingTrim(TextSpacingTrim::TrimStart))
    );
}

#[test] // cov:ignore: cfg(test)-only property tests have no lcov source record on the pinned coverage run.
// cov:ignore: cfg(test)-only property tests have no lcov source record on the pinned coverage run.
fn text_spacing_trim_rejects_unsupported_or_compound_values() {
    for source in [
        "",
        "none",
        "inherit",
        "unknown",
        "trim-start space-first",
        "normal auto",
        "space-all 1px",
    ] {
        assert_eq!(parse_entire(source, "text-spacing-trim"), None, "{source}");
    }
}

#[test] // cov:ignore: cfg(test)-only property tests have no lcov source record on the pinned coverage run.
// cov:ignore: cfg(test)-only property tests have no lcov source record on the pinned coverage run.
fn text_spacing_trim_is_registered_and_maps_to_its_property_key() {
    assert!(is_supported_property_name("text-spacing-trim"));
    assert!(is_supported_property_name("TEXT-SPACING-TRIM"));
    assert_eq!(
        PropertyValue::TextSpacingTrim(TextSpacingTrim::Normal).key(),
        PropertyKey::TextSpacingTrim
    );
}

// ── text-spacing (CSS Text 4) ──

#[test] // cov:ignore: cfg(test)-only property tests have no lcov source record on the pinned coverage run.
// cov:ignore: cfg(test)-only property tests have no lcov source record on the pinned coverage run.
fn text_spacing_parses_the_pinned_wpt_values_into_longhands() {
    let normal = TextSpacingShorthand {
        trim: TextSpacingTrim::Normal,
        autospace: TextAutospace::Normal,
    };
    let no_autospace = TextSpacingShorthand {
        trim: TextSpacingTrim::Normal,
        autospace: TextAutospace::NoAutospace,
    };
    let trim_start = TextSpacingShorthand {
        trim: TextSpacingTrim::TrimStart,
        autospace: TextAutospace::Normal,
    };
    let space_all = TextSpacingShorthand {
        trim: TextSpacingTrim::SpaceAll,
        autospace: TextAutospace::Normal,
    };
    let none = TextSpacingShorthand {
        trim: TextSpacingTrim::SpaceAll,
        autospace: TextAutospace::NoAutospace,
    };
    let trim_start_no_autospace = TextSpacingShorthand {
        trim: TextSpacingTrim::TrimStart,
        autospace: TextAutospace::NoAutospace,
    };
    let auto = TextSpacingShorthand {
        trim: TextSpacingTrim::Auto,
        autospace: TextAutospace::Auto,
    };
    let cases = [
        ("initial", normal),
        ("normal", normal),
        ("none", none),
        ("auto", auto),
        ("no-autospace", no_autospace),
        ("trim-start", trim_start),
        ("space-all", space_all),
        ("normal normal", normal),
        ("normal trim-start", trim_start),
        ("no-autospace normal", no_autospace),
        ("no-autospace space-all", none),
        ("no-autospace trim-start", trim_start_no_autospace),
        ("trim-start normal", trim_start),
        ("normal no-autospace", no_autospace),
        ("space-all no-autospace", none),
        ("trim-start no-autospace", trim_start_no_autospace),
    ];
    for (source, expected) in cases {
        assert_eq!(
            parse_entire(source, "text-spacing"),
            Some(PropertyValue::TextSpacingShorthand(expected)),
            "{source}"
        );
    }
}

#[test] // cov:ignore: cfg(test)-only property tests have no lcov source record on the pinned coverage run.
// cov:ignore: cfg(test)-only property tests have no lcov source record on the pinned coverage run.
fn text_spacing_reuses_full_autospace_component_grammar_and_rejects_invalid_values() {
    assert_eq!(
        parse_entire(
            "trim-start ideograph-alpha punctuation insert",
            "text-spacing"
        ),
        Some(PropertyValue::TextSpacingShorthand(TextSpacingShorthand {
            trim: TextSpacingTrim::TrimStart,
            autospace: TextAutospace::Custom {
                ideograph_alpha: true,
                ideograph_numeric: false,
                punctuation: true,
                mode: TextAutospaceMode::Insert,
            },
        }))
    );
    for source in [
        "",
        "inherit",
        "unknown",
        "none trim-start",
        "trim-start space-all",
        "trim-start trim-start",
        "no-autospace punctuation",
        "trim-start no-autospace punctuation",
    ] {
        assert_eq!(parse_entire(source, "text-spacing"), None, "{source}");
    }
}

#[test] // cov:ignore: cfg(test)-only property tests have no lcov source record on the pinned coverage run.
// cov:ignore: cfg(test)-only property tests have no lcov source record on the pinned coverage run.
fn text_spacing_bounds_components_before_trying_permutations() {
    assert_eq!(
        parse_entire(
            "trim-all ideograph-alpha ideograph-numeric punctuation replace",
            "text-spacing"
        ),
        Some(PropertyValue::TextSpacingShorthand(TextSpacingShorthand {
            trim: TextSpacingTrim::TrimAll,
            autospace: TextAutospace::Custom {
                ideograph_alpha: true,
                ideograph_numeric: true,
                punctuation: true,
                mode: TextAutospaceMode::Replace,
            },
        }))
    );

    let repeated_normal = vec!["normal"; 9_000].join(" ");
    assert_eq!(parse_entire(&repeated_normal, "text-spacing"), None);
}

#[test] // cov:ignore: cfg(test)-only property tests have no lcov source record on the pinned coverage run.
// cov:ignore: cfg(test)-only property tests have no lcov source record on the pinned coverage run.
fn text_spacing_is_registered_and_maps_to_its_shorthand_key() {
    assert!(is_supported_property_name("text-spacing"));
    assert!(is_supported_property_name("TEXT-SPACING"));
    assert_eq!(
        PropertyValue::TextSpacingShorthand(TextSpacingShorthand {
            trim: TextSpacingTrim::Normal,
            autospace: TextAutospace::Normal,
        })
        .key(),
        PropertyKey::TextSpacing
    );
}

// ── text-indent (CSS Text 3 §8.1) ──
//
// Full value grammar: `<length-percentage> && hanging? && each-line?` —
// this crate preserves both simple values and additive calc terms.
// Initial: 0 / Applies to: block containers / Inherited: yes /
// Percentages: refers to block container's own inline-axis inner size /
// Computed value: computed <length-percentage> value, plus any
// specified keywords.

#[test]
fn text_indent_parse_px() {
    assert_eq!(
        parse("20px", "text-indent"),
        Some(PropertyValue::TextIndent(TextIndentValue {
            length: TextIndentLength::Length(Length::Px(20.0)),
            hanging: false,
            each_line: false
        }))
    );
}

#[test]
fn text_indent_parse_percentage() {
    assert_eq!(
        parse("10%", "text-indent"),
        Some(PropertyValue::TextIndent(TextIndentValue {
            length: TextIndentLength::Length(Length::Percent(10.0)),
            hanging: false,
            each_line: false
        }))
    );
}

#[test]
fn text_indent_parse_em() {
    assert_eq!(
        parse("2em", "text-indent"),
        Some(PropertyValue::TextIndent(TextIndentValue {
            length: TextIndentLength::Length(Length::Em(2.0)),
            hanging: false,
            each_line: false
        }))
    );
}

#[test]
fn text_indent_accepts_negative_length() {
    // CSS Text 3 §8.1 places no `[0,∞]` restriction on this grammar
    // (unlike `padding-top` — `PropertyValue::TextIndent` doc).
    assert_eq!(
        parse("-2em", "text-indent"),
        Some(PropertyValue::TextIndent(TextIndentValue {
            length: TextIndentLength::Length(Length::Em(-2.0)),
            hanging: false,
            each_line: false
        }))
    );
}

#[test]
fn text_indent_accepts_zero() {
    // CSS Values 3 §5 unitless-zero clause — bare `0` is a valid `<length>`.
    assert_eq!(
        parse("0", "text-indent"),
        Some(PropertyValue::TextIndent(TextIndentValue {
            length: TextIndentLength::Length(Length::Px(0.0)),
            hanging: false,
            each_line: false
        }))
    );
}

#[test]
fn text_indent_parse_hanging() {
    // CSS Text 3 §8.1 `hanging` keyword is kept, not dropped.
    assert_eq!(
        parse("2em hanging", "text-indent"),
        Some(PropertyValue::TextIndent(TextIndentValue {
            length: TextIndentLength::Length(Length::Em(2.0)),
            hanging: true,
            each_line: false,
        }))
    );
}

#[test]
fn text_indent_parse_each_line() {
    assert_eq!(
        parse("each-line 2em", "text-indent"),
        Some(PropertyValue::TextIndent(TextIndentValue {
            length: TextIndentLength::Length(Length::Em(2.0)),
            hanging: false,
            each_line: true,
        }))
    );
}

#[test]
fn text_indent_parse_hanging_each_line_combined() {
    // Order-independent `&&`: flags may precede the length.
    assert_eq!(
        parse("hanging each-line 2em", "text-indent"),
        Some(PropertyValue::TextIndent(TextIndentValue {
            length: TextIndentLength::Length(Length::Em(2.0)),
            hanging: true,
            each_line: true,
        }))
    );
}

#[test]
fn text_wrap_parse_nowrap() {
    assert_eq!(
        parse("nowrap", "text-wrap"),
        Some(PropertyValue::TextWrapShorthand(TextWrapShorthand {
            mode: TextWrapMode::Nowrap,
            style: TextWrapStyle::Auto,
        }))
    );
}

#[test]
fn text_wrap_parse_wrap() {
    assert_eq!(
        parse("wrap", "text-wrap"),
        Some(PropertyValue::TextWrapShorthand(TextWrapShorthand {
            mode: TextWrapMode::Wrap,
            style: TextWrapStyle::Auto,
        }))
    );
}

#[test]
fn text_wrap_mode_longhand_parses_wrap_and_nowrap() {
    assert_eq!(
        parse("wrap", "text-wrap-mode"),
        Some(PropertyValue::TextWrap(TextWrapMode::Wrap))
    );
    assert_eq!(
        parse("nowrap", "text-wrap-mode"),
        Some(PropertyValue::TextWrap(TextWrapMode::Nowrap))
    );
}

#[test]
fn text_wrap_style_longhand_parses_all_metadata_keywords() {
    assert_eq!(
        parse("auto", "text-wrap-style"),
        Some(PropertyValue::TextWrapStyle(TextWrapStyle::Auto))
    );
    assert_eq!(
        parse("balance", "text-wrap-style"),
        Some(PropertyValue::TextWrapStyle(TextWrapStyle::Balance))
    );
    assert_eq!(
        parse("pretty", "text-wrap-style"),
        Some(PropertyValue::TextWrapStyle(TextWrapStyle::Pretty))
    );
    assert_eq!(
        parse("stable", "text-wrap-style"),
        Some(PropertyValue::TextWrapStyle(TextWrapStyle::Stable))
    );
    assert_eq!(
        parse("balance", "text-wrap-style").map(|value| value.key()),
        Some(PropertyKey::TextWrapStyle)
    );
}

#[test]
fn text_wrap_style_rejects_mode_keyword() {
    assert_eq!(parse("nowrap", "text-wrap-style"), None);
}

#[test]
fn text_wrap_style_only_shorthand_defaults_mode_to_wrap() {
    assert_eq!(
        parse("balance", "text-wrap"),
        Some(PropertyValue::TextWrapShorthand(TextWrapShorthand {
            mode: TextWrapMode::Wrap,
            style: TextWrapStyle::Balance,
        }))
    );
}

#[test]
fn text_wrap_shorthand_accepts_mode_and_style_in_either_order() {
    let expected = PropertyValue::TextWrapShorthand(TextWrapShorthand {
        mode: TextWrapMode::Nowrap,
        style: TextWrapStyle::Stable,
    });
    assert_eq!(parse("nowrap stable", "text-wrap"), Some(expected.clone()));
    assert_eq!(parse("stable nowrap", "text-wrap"), Some(expected));
}

#[test]
fn text_wrap_shorthand_maps_to_text_wrap_key() {
    assert_eq!(
        parse("balance", "text-wrap").map(|value| value.key()),
        Some(PropertyKey::TextWrap)
    );
}

#[test]
fn text_wrap_shorthand_rejects_duplicate_components() {
    assert_eq!(parse("wrap nowrap", "text-wrap"), None);
    assert_eq!(parse("balance stable", "text-wrap"), None);
}

#[test]
fn text_indent_rejects_unsupported_unit() {
    // `cap` (CSS Values 4 §6.1.1) is not implemented — dropped by
    // `parse_length_value`'s `Token::Dimension` fall-through, same as the
    // `margin_side_rejects_unsupported_unit` sibling.
    assert_eq!(parse("1cap", "text-indent"), None);
}

#[test]
fn text_indent_rejects_auto() {
    // Unlike `margin` / `width`, `text-indent`'s grammar has no `auto`
    // alternative — the `hanging`/`each-line` keywords are the only
    // idents the full grammar accepts. `parse_length_value` reads one
    // token via `input.next()` and only has match arms for
    // `Token::Dimension` / `Token::Percentage` / a zero `Token::Number` —
    // an `Ident` token (`auto` included) matches none of them and falls
    // through to the trailing `_ => None`.
    assert_eq!(parse("auto", "text-indent"), None);
}

#[test]
fn text_indent_key_maps_to_text_indent_property_key() {
    let v = PropertyValue::TextIndent(TextIndentValue {
        length: TextIndentLength::Length(Length::Px(20.0)),
        hanging: false,
        each_line: false,
    });
    assert_eq!(v.key(), PropertyKey::TextIndent);
}

// ── direction (CSS Writing Modes 4 §2.1) ──
//
// Value grammar (§2.1 spec verbatim): ltr | rtl
// Initial: ltr / Inherited: yes / Computed value: specified value。

#[test]
fn direction_parse_both_keywords() {
    assert_eq!(
        parse("ltr", "direction"),
        Some(PropertyValue::Direction(Direction::Ltr))
    );
    assert_eq!(
        parse("rtl", "direction"),
        Some(PropertyValue::Direction(Direction::Rtl))
    );
}

#[test]
fn direction_is_case_insensitive() {
    assert_eq!(
        parse("LTR", "direction"),
        Some(PropertyValue::Direction(Direction::Ltr))
    );
    assert_eq!(
        parse("Rtl", "direction"),
        Some(PropertyValue::Direction(Direction::Rtl))
    );
}

#[test]
fn direction_rejects_unknown_keyword() {
    // 旧 draft 相当の `auto` は現行 §2.1 grammar に無い — 実 spec-invalid。
    assert_eq!(parse("auto", "direction"), None);
    assert_eq!(parse("horizontal-tb", "direction"), None);
}

#[test]
fn direction_rejects_css_wide_keyword() {
    // (b) 非対応 — CSS-wide keyword は未実装 (将来対応)、silent drop。5 keyword
    // の一覧・理由は `PropertyValue` doc の「CSS-wide keyword」節が canonical。
    assert_eq!(parse("inherit", "direction"), None);
    assert_eq!(parse("initial", "direction"), None);
    assert_eq!(parse("unset", "direction"), None);
    assert_eq!(parse("revert", "direction"), None);
    assert_eq!(parse("revert-layer", "direction"), None);
}

#[test]
fn direction_rejects_non_ident() {
    assert_eq!(parse("16px", "direction"), None);
    assert_eq!(parse(r#""ltr""#, "direction"), None);
}

#[test]
fn direction_key_maps_to_direction_property_key() {
    let v = PropertyValue::Direction(Direction::Ltr);
    assert_eq!(v.key(), PropertyKey::Direction);
    let v = PropertyValue::Direction(Direction::Rtl);
    assert_eq!(v.key(), PropertyKey::Direction);
}

// ── writing-mode (CSS Writing Modes 4 §3.2) ──
//
// Value grammar (§3.2 spec verbatim): horizontal-tb | vertical-rl |
// vertical-lr | sideways-rl | sideways-lr
// Initial: horizontal-tb / Inherited: yes / Computed value: specified
// value — this crate deliberately diverges from the last clause for the
// 4 non-`horizontal-tb` keywords (`WritingMode` doc's Non-goal section,
// `resolve_writing_mode` tests below).

#[test] // cov:ignore: unit-test module is not emitted as an lcov source record.
// cov:ignore: unit-test module is not emitted as an lcov source record.
fn ruby_position_parses_all_keywords_and_maps_key() {
    assert_eq!(
        parse("over", "ruby-position"),
        Some(PropertyValue::RubyPosition(RubyPosition::Over))
    );
    assert_eq!(
        parse("under", "ruby-position"),
        Some(PropertyValue::RubyPosition(RubyPosition::Under))
    );
    assert_eq!(
        parse("inter-character", "ruby-position"),
        Some(PropertyValue::RubyPosition(RubyPosition::InterCharacter))
    );
    assert_eq!(
        PropertyValue::RubyPosition(RubyPosition::Under).key(),
        PropertyKey::RubyPosition
    );
    assert_eq!(parse("sideways", "ruby-position"), None);
}

#[test]
fn writing_mode_parse_all_five_keywords() {
    // All 5 keywords parse successfully (`Some`, not `None`) — this
    // crate accepts the full spec grammar even though the 4
    // non-`horizontal-tb` keywords later collapse at the computed layer
    // (`resolve_writing_mode`, not this parser).
    assert_eq!(
        parse("horizontal-tb", "writing-mode"),
        Some(PropertyValue::WritingMode(WritingMode::HorizontalTb))
    );
    assert_eq!(
        parse("vertical-rl", "writing-mode"),
        Some(PropertyValue::WritingMode(WritingMode::VerticalRl))
    );
    assert_eq!(
        parse("vertical-lr", "writing-mode"),
        Some(PropertyValue::WritingMode(WritingMode::VerticalLr))
    );
    assert_eq!(
        parse("sideways-rl", "writing-mode"),
        Some(PropertyValue::WritingMode(WritingMode::SidewaysRl))
    );
    assert_eq!(
        parse("sideways-lr", "writing-mode"),
        Some(PropertyValue::WritingMode(WritingMode::SidewaysLr))
    );
}

#[test]
fn writing_mode_is_case_insensitive() {
    assert_eq!(
        parse("HORIZONTAL-TB", "writing-mode"),
        Some(PropertyValue::WritingMode(WritingMode::HorizontalTb))
    );
    assert_eq!(
        parse("Vertical-Rl", "writing-mode"),
        Some(PropertyValue::WritingMode(WritingMode::VerticalRl))
    );
    assert_eq!(
        parse("SIDEWAYS-LR", "writing-mode"),
        Some(PropertyValue::WritingMode(WritingMode::SidewaysLr))
    );
}

#[test]
fn writing_mode_rejects_unknown_keyword() {
    assert_eq!(parse("auto", "writing-mode"), None);
    // `ltr`/`rtl` are `direction`'s keywords, not `writing-mode`'s.
    assert_eq!(parse("ltr", "writing-mode"), None);
}

#[test]
fn writing_mode_rejects_css_wide_keyword() {
    // (b) 非対応 — CSS-wide keyword は未実装 (将来対応)、silent drop。5
    // keyword の一覧・理由は `PropertyValue` doc の「CSS-wide keyword」節が
    // canonical。
    assert_eq!(parse("inherit", "writing-mode"), None);
    assert_eq!(parse("initial", "writing-mode"), None);
    assert_eq!(parse("unset", "writing-mode"), None);
    assert_eq!(parse("revert", "writing-mode"), None);
    assert_eq!(parse("revert-layer", "writing-mode"), None);
}

#[test]
fn writing_mode_rejects_non_ident() {
    assert_eq!(parse("16px", "writing-mode"), None);
    assert_eq!(parse(r#""vertical-rl""#, "writing-mode"), None);
}

#[test]
fn writing_mode_key_maps_to_writing_mode_property_key() {
    let v = PropertyValue::WritingMode(WritingMode::HorizontalTb);
    assert_eq!(v.key(), PropertyKey::WritingMode);
    let v = PropertyValue::WritingMode(WritingMode::VerticalRl);
    assert_eq!(v.key(), PropertyKey::WritingMode);
}

/// [`resolve_writing_mode`]'s whole reason to exist — every one of the 5
/// spec keywords collapses to [`WritingMode::HorizontalTb`], not just the
/// 4 non-horizontal ones (identity for `HorizontalTb` itself is also
/// pinned so a future refactor can't "fix" this into a no-op passthrough
/// without a test noticing).
///
/// Future work: vertical writing-mode 実装時に本 collapse を
/// 削除し、本 test を revert/rewrite すること。
#[test]
fn resolve_writing_mode_collapses_all_five_keywords_to_horizontal_tb() {
    for specified in [
        WritingMode::HorizontalTb,
        WritingMode::VerticalRl,
        WritingMode::VerticalLr,
        WritingMode::SidewaysRl,
        WritingMode::SidewaysLr,
    ] {
        assert_eq!(resolve_writing_mode(specified), WritingMode::HorizontalTb);
    }
}

// ── text-decoration-line (CSS Text Decoration Module Level 3 §2.1) ──

#[test]
fn text_decoration_line_parses_none() {
    assert_eq!(
        parse("none", "text-decoration-line"),
        Some(PropertyValue::TextDecorationLine(TextDecorationLine::NONE))
    );
}

#[test]
fn text_decoration_line_parses_each_single_keyword() {
    assert_eq!(
        parse("underline", "text-decoration-line"),
        Some(PropertyValue::TextDecorationLine(
            TextDecorationLine::UNDERLINE
        ))
    );
    assert_eq!(
        parse("overline", "text-decoration-line"),
        Some(PropertyValue::TextDecorationLine(
            TextDecorationLine::OVERLINE
        ))
    );
    assert_eq!(
        parse("line-through", "text-decoration-line"),
        Some(PropertyValue::TextDecorationLine(
            TextDecorationLine::LINE_THROUGH
        ))
    );
    assert_eq!(
        parse("blink", "text-decoration-line"),
        Some(PropertyValue::TextDecorationLine(TextDecorationLine::BLINK))
    );
}

#[test]
fn text_decoration_line_parses_combination_in_any_order() {
    // `||` grammar: order-independent. Both orderings of the same pair
    // must produce the same flag set.
    assert_eq!(
        parse("underline overline", "text-decoration-line"),
        Some(PropertyValue::TextDecorationLine(TextDecorationLine {
            underline: true,
            overline: true,
            line_through: false,
            blink: false,
            spelling_error: false,
            grammar_error: false,
        }))
    );
    assert_eq!(
        parse("overline underline", "text-decoration-line"),
        Some(PropertyValue::TextDecorationLine(TextDecorationLine {
            underline: true,
            overline: true,
            line_through: false,
            blink: false,
            spelling_error: false,
            grammar_error: false,
        }))
    );
}

#[test]
fn text_decoration_line_parses_spelling_and_grammar_error_alone() {
    // CSS Text Decoration 4 §2.1: `spelling-error` / `grammar-error`
    // are top-level alternatives, each accepted only on its own.
    assert_eq!(
        parse("spelling-error", "text-decoration-line"),
        Some(PropertyValue::TextDecorationLine(
            TextDecorationLine::SPELLING_ERROR
        ))
    );
    assert_eq!(
        parse("grammar-error", "text-decoration-line"),
        Some(PropertyValue::TextDecorationLine(
            TextDecorationLine::GRAMMAR_ERROR
        ))
    );
}

#[test]
fn text_decoration_line_rejects_spelling_error_combined() {
    // Same grammar: neither combines with the `||` group nor with
    // each other — leftover makes the caller drop the declaration.
    assert_eq!(
        parse_entire("underline spelling-error", "text-decoration-line"),
        None
    );
    assert_eq!(
        parse_entire("spelling-error underline", "text-decoration-line"),
        None
    );
    assert_eq!(
        parse_entire("spelling-error grammar-error", "text-decoration-line"),
        None
    );
}

#[test]
fn text_decoration_line_parses_all_four_combined() {
    assert_eq!(
        parse(
            "underline overline line-through blink",
            "text-decoration-line"
        ),
        Some(PropertyValue::TextDecorationLine(TextDecorationLine {
            underline: true,
            overline: true,
            line_through: true,
            blink: true,
            spelling_error: false,
            grammar_error: false,
        }))
    );
}

#[test]
fn text_decoration_line_is_case_insensitive() {
    assert_eq!(
        parse("NONE", "text-decoration-line"),
        Some(PropertyValue::TextDecorationLine(TextDecorationLine::NONE))
    );
    assert_eq!(
        parse("Underline", "text-decoration-line"),
        Some(PropertyValue::TextDecorationLine(
            TextDecorationLine::UNDERLINE
        ))
    );
}

#[test]
fn text_decoration_line_two_underlines_leaves_leftover_for_caller_exhausted_check() {
    // Each `||` component at most once (CSS Values 4 §2.2). The 2nd
    // `underline` is left unconsumed by `parse_text_decoration_line`
    // (its flag is already set) — `parse_border_shorthand`'s sibling
    // `border_shorthand_two_widths_leaves_leftover_for_caller_exhausted_check`
    // test pattern: the helper itself still returns `Some` (1st token
    // consumed), and rejection is the caller's (`rule.rs`'s
    // `DeclParser::parse_value`'s `expect_exhausted`) responsibility —
    // pinned end-to-end by `rule.rs`'s
    // `text_decoration_line_duplicate_and_none_combination_declarations_dropped`
    // sibling test.
    let mut input = ParserInput::new("underline underline");
    let mut parser = Parser::new(&mut input);
    assert_eq!(
        parse_text_decoration_line(&mut parser),
        Some(TextDecorationLine::UNDERLINE)
    );
    assert!(!parser.is_exhausted());
}

#[test]
fn text_decoration_line_none_combined_with_a_keyword_leaves_leftover() {
    // `none | [ ... ]` — `none` is a separate top-level alternative, not
    // a member of the `||` combination, so it cannot co-occur with the
    // other keywords in either order. Same "helper returns `Some`,
    // leftover is the caller's `expect_exhausted` responsibility" shape
    // as the duplicate-keyword sibling test above.
    let mut input = ParserInput::new("none underline");
    let mut parser = Parser::new(&mut input);
    assert_eq!(
        parse_text_decoration_line(&mut parser),
        Some(TextDecorationLine::NONE)
    );
    assert!(!parser.is_exhausted());

    let mut input = ParserInput::new("underline none");
    let mut parser = Parser::new(&mut input);
    assert_eq!(
        parse_text_decoration_line(&mut parser),
        Some(TextDecorationLine::UNDERLINE)
    );
    assert!(!parser.is_exhausted());
}

#[test]
fn text_decoration_line_rejects_unknown_keyword() {
    assert_eq!(parse("bogus", "text-decoration-line"), None);
}

#[test]
fn text_decoration_line_rejects_css_wide_keyword() {
    // (b) not supported — CSS-wide keyword is unimplemented (future work),
    // silent drop (`PropertyValue` doc's "CSS-wide keyword" section is canonical).
    for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(kw, "text-decoration-line"), None);
    }
}

#[test]
fn text_decoration_line_rejects_non_ident() {
    assert_eq!(parse("16px", "text-decoration-line"), None);
    assert_eq!(parse(r#""underline""#, "text-decoration-line"), None);
}

#[test]
fn text_decoration_line_serializes_all_keyword_combinations() {
    for mask in 0_u8..16 {
        let value = TextDecorationLine {
            underline: mask & 1 != 0,
            overline: mask & 2 != 0,
            line_through: mask & 4 != 0,
            blink: mask & 8 != 0,
            spelling_error: false,
            grammar_error: false,
        };
        let mut components = Vec::new();
        for (bit, component) in [
            (1_u8, "underline"),
            (2_u8, "overline"),
            (4_u8, "line-through"),
            (8_u8, "blink"),
        ] {
            if mask & bit != 0 {
                components.push(component);
            }
        }
        let expected = if components.is_empty() {
            "none".to_owned()
        } else {
            components.join(" ")
        };
        assert_eq!(
            serialize_value(&PropertyValue::TextDecorationLine(value)),
            Some(expected),
            "mask {mask}"
        );
    }

    assert_eq!(
        serialize_value(&PropertyValue::TextDecorationLine(
            TextDecorationLine::SPELLING_ERROR
        )),
        Some("spelling-error".to_owned())
    );
    assert_eq!(
        serialize_value(&PropertyValue::TextDecorationLine(
            TextDecorationLine::GRAMMAR_ERROR
        )),
        Some("grammar-error".to_owned())
    );
}

// ── text-decoration-style (CSS Text Decoration Module Level 3 §2.2) ──

#[test]
fn text_decoration_style_parses_all_five_keywords() {
    for (kw, expected) in [
        ("solid", TextDecorationStyle::Solid),
        ("double", TextDecorationStyle::Double),
        ("dotted", TextDecorationStyle::Dotted),
        ("dashed", TextDecorationStyle::Dashed),
        ("wavy", TextDecorationStyle::Wavy),
    ] {
        assert_eq!(
            parse(kw, "text-decoration-style"),
            Some(PropertyValue::TextDecorationStyle(expected))
        );
        assert_eq!(
            serialize_value(&PropertyValue::TextDecorationStyle(expected)),
            Some(kw.to_owned())
        );
    }
}

#[test]
fn text_decoration_style_is_case_insensitive() {
    assert_eq!(
        parse("WAVY", "text-decoration-style"),
        Some(PropertyValue::TextDecorationStyle(
            TextDecorationStyle::Wavy
        ))
    );
}

#[test]
fn text_decoration_style_rejects_unknown_keyword() {
    // `underline` is a `text-decoration-line` keyword, not a
    // `text-decoration-style` one — the two properties' keyword sets are
    // disjoint.
    assert_eq!(parse("underline", "text-decoration-style"), None);
    assert_eq!(parse("bogus", "text-decoration-style"), None);
}

// ── text-decoration-color (CSS Text Decoration Module Level 3 §2.3) ──

#[test]
fn text_decoration_color_parses_currentcolor() {
    assert_eq!(
        parse("currentcolor", "text-decoration-color"),
        Some(PropertyValue::TextDecorationColor(
            TextDecorationColor::CurrentColor
        ))
    );
    assert_eq!(
        parse("CurrentColor", "text-decoration-color"),
        Some(PropertyValue::TextDecorationColor(
            TextDecorationColor::CurrentColor
        ))
    );
}

#[test]
fn text_decoration_color_parses_resolved_color() {
    assert_eq!(
        parse("red", "text-decoration-color"),
        Some(PropertyValue::TextDecorationColor(
            TextDecorationColor::Resolved(CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255,
            })
        ))
    );
    assert_eq!(
        parse("#00ff00", "text-decoration-color"),
        Some(PropertyValue::TextDecorationColor(
            TextDecorationColor::Resolved(CssColor {
                r: 0,
                g: 255,
                b: 0,
                a: 255,
            })
        ))
    );
}

#[test]
fn text_decoration_color_rejects_unknown_ident() {
    assert_eq!(parse("bogus", "text-decoration-color"), None);
}

// ── text-decoration shorthand (CSS Text Decoration Module Level 3
// §2.4) ──
//
// `<'text-decoration-line'> || <'text-decoration-style'> ||
// <'text-decoration-color'>`. Omitted components fill with their
// longhand's initial value (verbatim: "Omitted values are set to their
// initial values.").

#[test]
fn text_decoration_shorthand_line_only_fills_the_rest_with_initial() {
    assert_eq!(
        parse("underline", "text-decoration"),
        Some(PropertyValue::TextDecoration(TextDecorationShorthand {
            line: TextDecorationLine::UNDERLINE,
            style: TextDecorationStyle::Solid,
            color: TextDecorationColor::CurrentColor,
            thickness: TextDecorationThickness::Auto,
        }))
    );
}

#[test]
fn text_decoration_shorthand_style_only_fills_the_rest_with_initial() {
    // A bare style keyword is a spec-valid shorthand value under `||`
    // (`text-decoration: wavy;`) — this was previously unreachable
    // (pre-longhand-decomposition `text-decoration` only accepted
    // `none`/`underline`).
    assert_eq!(
        parse("wavy", "text-decoration"),
        Some(PropertyValue::TextDecoration(TextDecorationShorthand {
            line: TextDecorationLine::NONE,
            style: TextDecorationStyle::Wavy,
            color: TextDecorationColor::CurrentColor,
            thickness: TextDecorationThickness::Auto,
        }))
    );
}

#[test]
fn text_decoration_shorthand_color_only_fills_the_rest_with_initial() {
    // Likewise a bare color (`text-decoration: red;`) was rejected
    // wholesale pre-decomposition; `||` makes it valid on its own.
    assert_eq!(
        parse("red", "text-decoration"),
        Some(PropertyValue::TextDecoration(TextDecorationShorthand {
            line: TextDecorationLine::NONE,
            style: TextDecorationStyle::Solid,
            color: TextDecorationColor::Resolved(CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255,
            }),
            thickness: TextDecorationThickness::Auto,
        }))
    );
}

#[test]
fn text_decoration_shorthand_parses_all_four_in_any_order() {
    let expected = Some(PropertyValue::TextDecoration(TextDecorationShorthand {
        line: TextDecorationLine::UNDERLINE,
        style: TextDecorationStyle::Wavy,
        color: TextDecorationColor::Resolved(CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255,
        }),
        thickness: TextDecorationThickness::Auto,
    }));
    assert_eq!(parse("underline wavy red", "text-decoration"), expected);
    assert_eq!(parse("red wavy underline", "text-decoration"), expected);
    assert_eq!(parse("wavy red underline", "text-decoration"), expected);
}

#[test]
fn text_decoration_shorthand_line_combination_plus_style_and_color() {
    assert_eq!(
        parse("underline overline wavy red", "text-decoration"),
        Some(PropertyValue::TextDecoration(TextDecorationShorthand {
            line: TextDecorationLine {
                underline: true,
                overline: true,
                line_through: false,
                blink: false,
                spelling_error: false,
                grammar_error: false,
            },
            style: TextDecorationStyle::Wavy,
            color: TextDecorationColor::Resolved(CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255,
            }),
            thickness: TextDecorationThickness::Auto,
        }))
    );
}

#[test]
fn text_decoration_shorthand_thickness_only_fills_the_rest_with_initial() {
    assert_eq!(
        parse("from-font", "text-decoration"),
        Some(PropertyValue::TextDecoration(TextDecorationShorthand {
            line: TextDecorationLine::NONE,
            style: TextDecorationStyle::Solid,
            color: TextDecorationColor::CurrentColor,
            thickness: TextDecorationThickness::FromFont,
        }))
    );
}

#[test]
fn text_decoration_shorthand_thickness_combines_with_other_components() {
    // ED §2.6: thickness participates in the `||` loop like the other
    // 3 components (WPT `text-decoration-shorthand.html` maps
    // `overline from-font dotted green` to all 4 longhands).
    assert_eq!(
        parse("overline from-font dotted green", "text-decoration"),
        Some(PropertyValue::TextDecoration(TextDecorationShorthand {
            line: TextDecorationLine::OVERLINE,
            style: TextDecorationStyle::Dotted,
            color: TextDecorationColor::Resolved(CssColor {
                r: 0,
                g: 128,
                b: 0,
                a: 255,
            }),
            thickness: TextDecorationThickness::FromFont,
        }))
    );
}

#[test]
fn text_decoration_shorthand_length_thickness_combines() {
    assert_eq!(
        parse("line-through 20px", "text-decoration"),
        Some(PropertyValue::TextDecoration(TextDecorationShorthand {
            line: TextDecorationLine::LINE_THROUGH,
            style: TextDecorationStyle::Solid,
            color: TextDecorationColor::CurrentColor,
            thickness: TextDecorationThickness::Length(Length::Px(20.0)),
        }))
    );
}

// ── text-decoration-skip-ink (ED §2.10.4) ──

#[test]
fn text_decoration_skip_ink_parses_all_three_keywords() {
    assert_eq!(
        parse("auto", "text-decoration-skip-ink"),
        Some(PropertyValue::TextDecorationSkipInk(
            TextDecorationSkipInk::Auto
        ))
    );
    assert_eq!(
        parse("none", "text-decoration-skip-ink"),
        Some(PropertyValue::TextDecorationSkipInk(
            TextDecorationSkipInk::None
        ))
    );
    assert_eq!(
        parse("all", "text-decoration-skip-ink"),
        Some(PropertyValue::TextDecorationSkipInk(
            TextDecorationSkipInk::All
        ))
    );
    assert_eq!(
        serialize_value(&PropertyValue::TextDecorationSkipInk(
            TextDecorationSkipInk::Auto
        )),
        Some("auto".to_owned())
    );
    assert_eq!(
        serialize_value(&PropertyValue::TextDecorationSkipInk(
            TextDecorationSkipInk::None
        )),
        Some("none".to_owned())
    );
    assert_eq!(
        serialize_value(&PropertyValue::TextDecorationSkipInk(
            TextDecorationSkipInk::All
        )),
        Some("all".to_owned())
    );
    assert_eq!(parse_entire("auto none", "text-decoration-skip-ink"), None);
    assert_eq!(parse_entire("bogus", "text-decoration-skip-ink"), None);
}

#[test]
fn text_decoration_skip_ink_key_maps_to_property_key() {
    let v = PropertyValue::TextDecorationSkipInk(TextDecorationSkipInk::Auto);
    assert_eq!(v.key(), PropertyKey::TextDecorationSkipInk);
}

// ── text-decoration-skip-spaces (ED §2.10.3) ──

#[test]
fn text_decoration_skip_spaces_parses_all_forms() {
    assert_eq!(
        parse("none", "text-decoration-skip-spaces"),
        Some(PropertyValue::TextDecorationSkipSpaces(
            TextDecorationSkipSpaces::None
        ))
    );
    assert_eq!(
        parse("all", "text-decoration-skip-spaces"),
        Some(PropertyValue::TextDecorationSkipSpaces(
            TextDecorationSkipSpaces::All
        ))
    );
    assert_eq!(
        parse("start", "text-decoration-skip-spaces"),
        Some(PropertyValue::TextDecorationSkipSpaces(
            TextDecorationSkipSpaces::Start
        ))
    );
    assert_eq!(
        parse("end", "text-decoration-skip-spaces"),
        Some(PropertyValue::TextDecorationSkipSpaces(
            TextDecorationSkipSpaces::End
        ))
    );
    // `||` order-independence, both orders map to `StartEnd`.
    assert_eq!(
        parse("start end", "text-decoration-skip-spaces"),
        Some(PropertyValue::TextDecorationSkipSpaces(
            TextDecorationSkipSpaces::StartEnd
        ))
    );
    assert_eq!(
        parse("end start", "text-decoration-skip-spaces"),
        Some(PropertyValue::TextDecorationSkipSpaces(
            TextDecorationSkipSpaces::StartEnd
        ))
    );
    assert_eq!(
        parse_entire("none start", "text-decoration-skip-spaces"),
        None
    );
    assert_eq!(
        parse_entire("start start", "text-decoration-skip-spaces"),
        None
    );
    assert_eq!(parse_entire("bogus", "text-decoration-skip-spaces"), None);
    for (value, expected) in [
        (TextDecorationSkipSpaces::None, "none"),
        (TextDecorationSkipSpaces::All, "all"),
        (TextDecorationSkipSpaces::Start, "start"),
        (TextDecorationSkipSpaces::End, "end"),
        (TextDecorationSkipSpaces::StartEnd, "start end"),
    ] {
        assert_eq!(
            serialize_value(&PropertyValue::TextDecorationSkipSpaces(value)),
            Some(expected.to_owned())
        );
    }
}

#[test]
fn text_decoration_skip_spaces_key_maps_to_property_key() {
    let v = PropertyValue::TextDecorationSkipSpaces(TextDecorationSkipSpaces::StartEnd);
    assert_eq!(v.key(), PropertyKey::TextDecorationSkipSpaces);
}

// ── text-decoration-thickness (ED §2.4.1) ──

#[test]
fn text_decoration_thickness_parses_all_forms() {
    assert_eq!(
        parse("auto", "text-decoration-thickness"),
        Some(PropertyValue::TextDecorationThickness(
            TextDecorationThickness::Auto
        ))
    );
    assert_eq!(
        parse("from-font", "text-decoration-thickness"),
        Some(PropertyValue::TextDecorationThickness(
            TextDecorationThickness::FromFont
        ))
    );
    assert_eq!(
        parse("3em", "text-decoration-thickness"),
        Some(PropertyValue::TextDecorationThickness(
            TextDecorationThickness::Length(Length::Em(3.0))
        ))
    );
    assert_eq!(
        parse("50%", "text-decoration-thickness"),
        Some(PropertyValue::TextDecorationThickness(
            TextDecorationThickness::Length(Length::Percent(50.0))
        ))
    );
    // `<line-width>` keywords are out of scope (dropped).
    assert_eq!(parse_entire("thin", "text-decoration-thickness"), None);
    assert_eq!(parse_entire("medium", "text-decoration-thickness"), None);
}

#[test]
fn text_decoration_thickness_key_maps_to_property_key() {
    let v = PropertyValue::TextDecorationThickness(TextDecorationThickness::Auto);
    assert_eq!(v.key(), PropertyKey::TextDecorationThickness);
}

// ── text-decoration-inset (ED §2.9.1) ──

#[test]
fn text_decoration_inset_parses_auto_and_one_or_two_lengths() {
    assert_eq!(
        parse("auto", "text-decoration-inset"),
        Some(PropertyValue::TextDecorationInset(
            TextDecorationInset::Auto
        ))
    );
    assert_eq!(
        parse("-1em", "text-decoration-inset"),
        Some(PropertyValue::TextDecorationInset(
            TextDecorationInset::Lengths {
                start: Length::Em(-1.0),
                end: Length::Em(-1.0),
            }
        ))
    );
    assert_eq!(
        parse("1px 2px", "text-decoration-inset"),
        Some(PropertyValue::TextDecorationInset(
            TextDecorationInset::Lengths {
                start: Length::Px(1.0),
                end: Length::Px(2.0),
            }
        ))
    );
    // `<percentage>` is rejected (WPT ground truth over ED grammar).
    assert_eq!(parse_entire("10%", "text-decoration-inset"), None);
    assert_eq!(parse_entire("none", "text-decoration-inset"), None);
    assert_eq!(parse_entire("auto auto", "text-decoration-inset"), None);
    assert_eq!(
        serialize_value(&parse_entire("0", "text-decoration-inset").unwrap()),
        Some("0px".to_owned())
    );
    assert_eq!(
        serialize_value(&parse_entire("0px 0px", "text-decoration-inset").unwrap()),
        Some("0px".to_owned())
    );
    assert_eq!(
        serialize_value(&parse_entire("-1ch -1ch", "text-decoration-inset").unwrap()),
        Some("-1ch".to_owned())
    );
}

#[test]
fn text_decoration_inset_key_maps_to_property_key() {
    let v = PropertyValue::TextDecorationInset(TextDecorationInset::Auto);
    assert_eq!(v.key(), PropertyKey::TextDecorationInset);
}

// ── text-emphasis-position (ED §3.4) ──

#[test]
fn text_emphasis_position_parses_auto_and_axis_combinations() {
    assert_eq!(
        parse("auto", "text-emphasis-position"),
        Some(PropertyValue::TextEmphasisPosition(
            TextEmphasisPosition::Auto
        ))
    );
    assert_eq!(
        parse("over", "text-emphasis-position"),
        Some(PropertyValue::TextEmphasisPosition(
            TextEmphasisPosition::Position {
                vertical: TextEmphasisVEdge::Over,
                horizontal: None,
            }
        ))
    );
    assert_eq!(
        parse("right under", "text-emphasis-position"),
        Some(PropertyValue::TextEmphasisPosition(
            TextEmphasisPosition::Position {
                vertical: TextEmphasisVEdge::Under,
                horizontal: Some(TextEmphasisHEdge::Right),
            }
        ))
    );
    // vertical is required; doubled axes rejected.
    assert_eq!(parse_entire("left", "text-emphasis-position"), None);
    assert_eq!(
        parse_entire("left over right", "text-emphasis-position"),
        None
    );
    assert_eq!(
        parse_entire("under right over", "text-emphasis-position"),
        None
    );
}

#[test]
fn text_emphasis_position_key_maps_to_property_key() {
    let v = PropertyValue::TextEmphasisPosition(TextEmphasisPosition::Auto);
    assert_eq!(v.key(), PropertyKey::TextEmphasisPosition);
}

// ── text-emphasis-style (CSS Text Decoration 4) ──

#[test]
fn text_emphasis_style_parses_and_serializes_pinned_computed_values() {
    let cases = [
        ("none", "none"),
        ("dot", "dot"),
        ("filled circle", "circle"),
        ("filled", "circle"),
        ("open", "open circle"),
        ("double-circle", "double-circle"),
        ("triangle", "triangle"),
        ("open sesame", "open sesame"),
        ("\"*\"", "\"*\""),
        ("circle filled", "circle"),
        ("sesame open", "open sesame"),
    ];
    for (source, expected) in cases {
        let value = parse_entire(source, "text-emphasis-style")
            .unwrap_or_else(|| panic!("text-emphasis-style should parse {source:?}"));
        assert_eq!(
            serialize_value(&value).as_deref(),
            Some(expected),
            "source: {source:?}",
        );
    }
}

#[test]
fn text_emphasis_style_preserves_fill_only_default_shape_for_resolution() {
    assert_eq!(
        parse_entire("filled", "text-emphasis-style"),
        Some(PropertyValue::TextEmphasisStyle(
            TextEmphasisStyle::DefaultShape {
                fill: TextEmphasisFill::Filled,
            }
        )),
    );
    assert_eq!(
        parse_entire("open", "text-emphasis-style"),
        Some(PropertyValue::TextEmphasisStyle(
            TextEmphasisStyle::DefaultShape {
                fill: TextEmphasisFill::Open,
            }
        )),
    );
}

#[test]
fn text_emphasis_style_rejects_mixed_or_duplicate_components() {
    for source in [
        "none circle",
        "open filled circle",
        "dot circle",
        "\"*\" circle",
    ] {
        assert_eq!(
            parse_entire(source, "text-emphasis-style"),
            None,
            "source: {source:?}",
        );
    }
}

#[test]
fn text_emphasis_style_key_maps_names_and_deferred_values() {
    let value = parse_entire("dot", "text-emphasis-style").unwrap();
    assert_eq!(value.key(), PropertyKey::TextEmphasisStyle);
    assert!(is_supported_property_name("text-emphasis-style"));
    let deferred = parse_entire("var(--mark)", "text-emphasis-style").unwrap();
    assert_eq!(deferred.key(), PropertyKey::TextEmphasisStyle);
}

#[test]
fn text_emphasis_color_parses_currentcolor_and_named_colors() {
    assert_eq!(
        parse_entire("currentColor", "text-emphasis-color"),
        Some(PropertyValue::TextEmphasisColor(
            TextDecorationColor::CurrentColor
        )),
    );
    assert_eq!(
        parse_entire("red", "text-emphasis-color"),
        Some(PropertyValue::TextEmphasisColor(
            TextDecorationColor::Resolved(CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255,
            })
        )),
    );
    let deferred = parse_entire("var(--mark-color)", "text-emphasis-color").unwrap();
    assert_eq!(deferred.key(), PropertyKey::TextEmphasisColor);
}

#[test]
fn text_emphasis_shorthand_parses_style_and_color_components() {
    let dot = TextEmphasisStyle::Shape {
        fill: TextEmphasisFill::Filled,
        shape: TextEmphasisShape::Dot,
    };
    let open_sesame = TextEmphasisStyle::Shape {
        fill: TextEmphasisFill::Open,
        shape: TextEmphasisShape::Sesame,
    };
    let red = TextDecorationColor::Resolved(CssColor {
        r: 255,
        g: 0,
        b: 0,
        a: 255,
    });
    let cases = [
        (
            "none",
            TextEmphasisStyle::None,
            TextDecorationColor::CurrentColor,
        ),
        ("dot", dot.clone(), TextDecorationColor::CurrentColor),
        (
            "open sesame",
            open_sesame,
            TextDecorationColor::CurrentColor,
        ),
        (
            "\"*\"",
            TextEmphasisStyle::String(SmolStr::new("*")),
            TextDecorationColor::CurrentColor,
        ),
        (
            "currentColor",
            TextEmphasisStyle::None,
            TextDecorationColor::CurrentColor,
        ),
        (
            "black",
            TextEmphasisStyle::None,
            TextDecorationColor::Resolved(CssColor::BLACK),
        ),
        ("dot red", dot.clone(), red),
        ("red dot", dot, red),
        (
            "none black",
            TextEmphasisStyle::None,
            TextDecorationColor::Resolved(CssColor::BLACK),
        ),
    ];
    for (source, style, color) in cases {
        assert_eq!(
            parse_entire(source, "text-emphasis"),
            Some(PropertyValue::TextEmphasis(TextEmphasisShorthand {
                style,
                color
            })),
            "source: {source:?}",
        );
    }
}

#[test]
fn text_emphasis_shorthand_rejects_empty_duplicate_and_mixed_components() {
    for source in [
        "",
        "dot circle",
        "open filled circle",
        "none dot",
        "\"*\" circle",
        "red blue",
    ] {
        assert_eq!(
            parse_entire(source, "text-emphasis"),
            None,
            "source: {source:?}",
        );
    }
}

#[test]
fn text_emphasis_shorthand_key_maps_names_and_deferred_values() {
    let value = parse_entire("dot red", "text-emphasis").unwrap();
    assert_eq!(value.key(), PropertyKey::TextEmphasis);
    assert!(is_supported_property_name("text-emphasis"));
    let deferred = parse_entire("var(--emphasis)", "text-emphasis").unwrap();
    assert_eq!(deferred.key(), PropertyKey::TextEmphasis);
}

// ── text-underline-position (ED §2.7) ──

#[test]
fn text_underline_position_parses_auto_and_combinations() {
    assert_eq!(
        parse("auto", "text-underline-position"),
        Some(PropertyValue::TextUnderlinePosition(
            TextUnderlinePosition::AUTO
        ))
    );
    assert_eq!(
        parse("under", "text-underline-position"),
        Some(PropertyValue::TextUnderlinePosition(
            TextUnderlinePosition {
                under: true,
                ..TextUnderlinePosition::AUTO
            }
        ))
    );
    assert_eq!(
        parse("from-font left", "text-underline-position"),
        Some(PropertyValue::TextUnderlinePosition(
            TextUnderlinePosition {
                from_font: true,
                left: true,
                ..TextUnderlinePosition::AUTO
            }
        ))
    );
    assert_eq!(
        parse("right under", "text-underline-position"),
        Some(PropertyValue::TextUnderlinePosition(
            TextUnderlinePosition {
                under: true,
                right: true,
                ..TextUnderlinePosition::AUTO
            }
        ))
    );
    // `auto` is exclusive; `from-font`+`under` and `left`+`right`
    // never combine.
    assert_eq!(parse_entire("auto under", "text-underline-position"), None);
    assert_eq!(
        parse_entire("under from-font", "text-underline-position"),
        None
    );
    assert_eq!(parse_entire("left right", "text-underline-position"), None);
    assert_eq!(parse_entire("bogus", "text-underline-position"), None);
}

#[test]
fn text_underline_offset_parses_lengths_percentages_and_auto() {
    assert_eq!(
        parse("auto", "text-underline-offset"),
        Some(PropertyValue::TextUnderlineOffset(
            TextUnderlineOffset::Auto
        ))
    );
    assert_eq!(
        parse("11px", "text-underline-offset"),
        Some(PropertyValue::TextUnderlineOffset(
            TextUnderlineOffset::Length(Length::Px(11.0),)
        ))
    );
    assert_eq!(
        parse("10%", "text-underline-offset"),
        Some(PropertyValue::TextUnderlineOffset(
            TextUnderlineOffset::Length(Length::Percent(10.0),)
        ))
    );
    assert_eq!(
        parse("calc(2em - 8px)", "text-underline-offset"),
        Some(PropertyValue::TextUnderlineOffset(
            TextUnderlineOffset::Calc(LengthPercentageCalc {
                percent: 0.0,
                px: -8.0,
                em: 2.0,
            },)
        ))
    );
    assert_eq!(
        parse("calc(2em - 50%)", "text-underline-offset"),
        Some(PropertyValue::TextUnderlineOffset(
            TextUnderlineOffset::Calc(LengthPercentageCalc {
                percent: -50.0,
                px: 0.0,
                em: 2.0,
            },)
        ))
    );
    assert_eq!(
        parse("calc(200% - 8px)", "text-underline-offset"),
        Some(PropertyValue::TextUnderlineOffset(
            TextUnderlineOffset::Calc(LengthPercentageCalc {
                percent: 200.0,
                px: -8.0,
                em: 0.0,
            },)
        ))
    );
    assert_eq!(
        parse("calc(200% - 0.5em)", "text-underline-offset"),
        Some(PropertyValue::TextUnderlineOffset(
            TextUnderlineOffset::Calc(LengthPercentageCalc {
                percent: 200.0,
                px: 0.0,
                em: -0.5,
            },)
        ))
    );
    assert_eq!(
        serialize_value(&parse_entire("11px", "text-underline-offset").unwrap()),
        Some("11px".to_owned())
    );
    assert_eq!(
        serialize_value(&parse_entire("10%", "text-underline-offset").unwrap()),
        Some("10%".to_owned())
    );
    assert_eq!(
        serialize_value(&parse_entire("calc(2em - 8px)", "text-underline-offset").unwrap()),
        Some("calc(2em - 8px)".to_owned())
    );
    assert_eq!(
        serialize_value(&parse_entire("calc(200% - 8px)", "text-underline-offset").unwrap()),
        Some("calc(200% - 8px)".to_owned())
    );
}

#[test]
fn text_underline_offset_key_maps_to_property_key() {
    let v = PropertyValue::TextUnderlineOffset(TextUnderlineOffset::Auto);
    assert_eq!(v.key(), PropertyKey::TextUnderlineOffset);
}

#[test]
fn text_underline_position_key_maps_to_property_key() {
    let v = PropertyValue::TextUnderlinePosition(TextUnderlinePosition::AUTO);
    assert_eq!(v.key(), PropertyKey::TextUnderlinePosition);
}

#[test]
fn text_decoration_shorthand_rejects_empty_value() {
    assert_eq!(parse("", "text-decoration"), None);
}

#[test]
fn text_decoration_shorthand_two_style_components_leaves_leftover_for_caller_exhausted_check() {
    // Each `||` component at most once — a 2nd style keyword ("wavy")
    // doesn't match any unfilled slot (style already filled by "solid";
    // it isn't a line keyword or a `<color>`) so it's left unconsumed.
    // Same "helper returns `Some`, caller's `expect_exhausted` drops the
    // whole declaration" shape as `parse_border_shorthand`'s
    // `border_shorthand_two_widths_leaves_leftover_for_caller_exhausted_check`
    // — end-to-end rejection is pinned by `rule.rs`'s
    // `text_decoration_shorthand_two_style_components_declaration_dropped`.
    let mut input = ParserInput::new("solid wavy");
    let mut parser = Parser::new(&mut input);
    assert_eq!(
        parse_text_decoration_shorthand(&mut parser),
        Some(TextDecorationShorthand {
            line: TextDecorationLine::NONE,
            style: TextDecorationStyle::Solid,
            color: TextDecorationColor::CurrentColor,
            thickness: TextDecorationThickness::Auto,
        })
    );
    assert!(!parser.is_exhausted());
}

#[test]
fn text_decoration_shorthand_rejects_css_wide_keyword() {
    for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(kw, "text-decoration"), None);
    }
}

#[test]
fn text_decoration_key_maps_to_text_decoration_property_key() {
    let line = PropertyValue::TextDecorationLine(TextDecorationLine::UNDERLINE);
    assert_eq!(line.key(), PropertyKey::TextDecorationLine);
    let style = PropertyValue::TextDecorationStyle(TextDecorationStyle::Wavy);
    assert_eq!(style.key(), PropertyKey::TextDecorationStyle);
    let color = PropertyValue::TextDecorationColor(TextDecorationColor::CurrentColor);
    assert_eq!(color.key(), PropertyKey::TextDecorationColor);
    let shorthand = PropertyValue::TextDecoration(TextDecorationShorthand {
        line: TextDecorationLine::NONE,
        style: TextDecorationStyle::Solid,
        color: TextDecorationColor::CurrentColor,
        thickness: TextDecorationThickness::Auto,
    });
    assert_eq!(shorthand.key(), PropertyKey::TextDecoration);
}

// ── vertical-align (CSS 2.1 §10.8.1) ──
//
// Value grammar (`VerticalAlign` doc's "Scope carving" section):
// baseline | sub | super | middle | text-top | text-bottom | top | bottom |
// <length> | <percentage>, plus supported length / length-percentage calc().
// Initial: baseline / Inherited: no / Computed value: keyword as specified,
// length, percentage, and mixed calc() absolutized.

#[test]
fn vertical_align_parse_all_keywords() {
    assert_eq!(
        parse("baseline", "vertical-align"),
        Some(PropertyValue::VerticalAlign(VerticalAlign::Baseline))
    );
    assert_eq!(
        parse("sub", "vertical-align"),
        Some(PropertyValue::VerticalAlign(VerticalAlign::Sub))
    );
    assert_eq!(
        parse("super", "vertical-align"),
        Some(PropertyValue::VerticalAlign(VerticalAlign::Super))
    );
    assert_eq!(
        parse("middle", "vertical-align"),
        Some(PropertyValue::VerticalAlign(VerticalAlign::Middle))
    );
    assert_eq!(
        parse("text-top", "vertical-align"),
        Some(PropertyValue::VerticalAlign(VerticalAlign::TextTop))
    );
    assert_eq!(
        parse("text-bottom", "vertical-align"),
        Some(PropertyValue::VerticalAlign(VerticalAlign::TextBottom))
    );
    assert_eq!(
        parse("top", "vertical-align"),
        Some(PropertyValue::VerticalAlign(VerticalAlign::Top))
    );
    assert_eq!(
        parse("bottom", "vertical-align"),
        Some(PropertyValue::VerticalAlign(VerticalAlign::Bottom))
    );
}

#[test]
fn vertical_align_is_case_insensitive() {
    assert_eq!(
        parse("BASELINE", "vertical-align"),
        Some(PropertyValue::VerticalAlign(VerticalAlign::Baseline))
    );
    assert_eq!(
        parse("Sub", "vertical-align"),
        Some(PropertyValue::VerticalAlign(VerticalAlign::Sub))
    );
    assert_eq!(
        parse("SUPER", "vertical-align"),
        Some(PropertyValue::VerticalAlign(VerticalAlign::Super))
    );
    assert_eq!(
        parse("Middle", "vertical-align"),
        Some(PropertyValue::VerticalAlign(VerticalAlign::Middle))
    );
    assert_eq!(
        parse("TEXT-TOP", "vertical-align"),
        Some(PropertyValue::VerticalAlign(VerticalAlign::TextTop))
    );
    assert_eq!(
        parse("Text-Bottom", "vertical-align"),
        Some(PropertyValue::VerticalAlign(VerticalAlign::TextBottom))
    );
}

#[test]
fn vertical_align_parse_length() {
    assert_eq!(
        parse("10px", "vertical-align"),
        Some(PropertyValue::VerticalAlign(VerticalAlign::Length(
            Length::Px(10.0)
        )))
    );
    assert_eq!(
        parse("1.5em", "vertical-align"),
        Some(PropertyValue::VerticalAlign(VerticalAlign::Length(
            Length::Em(1.5)
        )))
    );
}

#[test]
fn vertical_align_parse_length_allows_negative() {
    // §10.8.1 spec verbatim: "Raise (positive value) or lower
    // (negative value) the box by this distance." — no non-negative
    // filter, same as `letter-spacing`/`margin-*`.
    assert_eq!(
        parse("-4px", "vertical-align"),
        Some(PropertyValue::VerticalAlign(VerticalAlign::Length(
            Length::Px(-4.0)
        )))
    );
}

#[test]
fn vertical_align_rejects_unknown_keywords() {
    for kw in ["sideways", "unknown"] {
        assert_eq!(parse(kw, "vertical-align"), None);
    }
}

#[test]
fn vertical_align_rejects_unknown_keyword() {
    assert_eq!(parse("bogus", "vertical-align"), None);
}

#[test]
fn vertical_align_rejects_css_wide_keyword() {
    // (b) not supported — CSS-wide keyword is unimplemented (future work),
    // silent drop (`PropertyValue` doc's "CSS-wide keyword" section is canonical).
    for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(kw, "vertical-align"), None);
    }
}

#[test]
fn vertical_align_parse_percentage() {
    // `<percentage>` is now implemented — absolutization is
    // `resolve_vertical_align` responsibility (own line-height basis,
    // `normal` fallback pinned there).
    assert_eq!(
        parse("50%", "vertical-align"),
        Some(PropertyValue::VerticalAlign(VerticalAlign::Length(
            Length::Percent(50.0)
        )))
    );
    assert_eq!(
        parse("-20%", "vertical-align"),
        Some(PropertyValue::VerticalAlign(VerticalAlign::Length(
            Length::Percent(-20.0)
        )))
    );
    assert_eq!(
        parse("0%", "vertical-align"),
        Some(PropertyValue::VerticalAlign(VerticalAlign::Length(
            Length::Percent(0.0)
        )))
    );
}

#[test]
fn vertical_align_parse_accepts_wpt_calc_expressions() {
    for expression in [
        "calc(50px)",
        "calc(50%)",
        "calc(25px + 50%)",
        "calc(150% / 2 - 30px)",
        "calc(40px + 10% - 20% / 2)",
        "calc(40px - 10%)",
    ] {
        assert!(
            parse_entire(expression, "vertical-align").is_some(),
            "{expression}"
        );
    }
}

#[test]
fn vertical_align_key_maps_to_vertical_align_property_key() {
    for va in [
        VerticalAlign::Baseline,
        VerticalAlign::Sub,
        VerticalAlign::Super,
        VerticalAlign::Middle,
        VerticalAlign::TextTop,
        VerticalAlign::TextBottom,
        VerticalAlign::Top,
        VerticalAlign::Bottom,
        VerticalAlign::Length(Length::Px(3.0)),
        VerticalAlign::Calc(CalcLengthPercentage {
            percent: 50.0,
            px: 3.0,
        }),
    ] {
        assert_eq!(
            PropertyValue::VerticalAlign(va).key(),
            PropertyKey::VerticalAlign
        );
    }
}

// ── font-style (CSS Fonts 4 §2.4) ──
//
// Value grammar (§2.4 spec verbatim, full property grammar): `normal |
// italic | left | right | oblique <angle [-90deg,90deg]>?`. This crate
// implements normal / italic / bare oblique (`FontStyle` doc's "Scope
// carving" section — the `<angle>` argument to `oblique` is out of
// scope). Initial: normal / Inherited: yes / Computed value:
// specified keyword (angle-bearing branch unreachable at this scope).

#[test]
fn font_style_parse_all_implemented_keywords() {
    assert_eq!(
        parse("normal", "font-style"),
        Some(PropertyValue::FontStyle(FontStyle::Normal))
    );
    assert_eq!(
        parse("italic", "font-style"),
        Some(PropertyValue::FontStyle(FontStyle::Italic))
    );
    assert_eq!(
        parse("oblique", "font-style"),
        Some(PropertyValue::FontStyle(FontStyle::Oblique))
    );
}

#[test]
fn font_style_is_case_insensitive() {
    assert_eq!(
        parse("NORMAL", "font-style"),
        Some(PropertyValue::FontStyle(FontStyle::Normal))
    );
    assert_eq!(
        parse("Italic", "font-style"),
        Some(PropertyValue::FontStyle(FontStyle::Italic))
    );
    assert_eq!(
        parse("OBLIQUE", "font-style"),
        Some(PropertyValue::FontStyle(FontStyle::Oblique))
    );
}

#[test]
fn font_style_rejects_unimplemented_keywords() {
    // (b) not supported — the `left` / `right` slant-direction keywords
    // are spec-valid but unimplemented (`FontStyle` doc's "Scope
    // carving" section), not (a) spec-invalid.
    assert_eq!(parse("left", "font-style"), None);
    assert_eq!(parse("right", "font-style"), None);
}

// `oblique <angle>` (e.g. `oblique 14deg`) is not exercised by this
// module's `parse()` helper — it calls `parse_value` directly, which
// only runs the property-specific parser and does not perform the
// exhaustive-consumption check that drops a declaration with unconsumed
// trailing tokens. `parse_font_style` alone happily returns
// `Some(FontStyle::Oblique)` after consuming just the `oblique` ident,
// leaving `14deg` unread — the rejection of `oblique <angle>` as a
// whole declaration only happens one layer up, in
// `crate::rule`'s `DeclParser`. See
// `rule::tests::rejects_font_style_oblique_with_angle` (mirrors
// `rejects_extra_length_after_font_size`) for that test.

#[test]
fn font_style_rejects_unknown_keyword() {
    assert_eq!(parse("bogus", "font-style"), None);
}

#[test]
fn font_style_rejects_css_wide_keyword() {
    // (b) not supported — CSS-wide keyword is unimplemented (future work),
    // silent drop (`PropertyValue` doc's "CSS-wide keyword" section is canonical).
    for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(kw, "font-style"), None);
    }
}

#[test]
fn font_style_rejects_non_ident() {
    assert_eq!(parse("16px", "font-style"), None);
    assert_eq!(parse(r#""italic""#, "font-style"), None);
}

#[test]
fn font_kerning_parses_serializes_and_maps_its_property_key() {
    for (input, expected) in [
        ("auto", FontKerning::Auto),
        ("normal", FontKerning::Normal),
        ("none", FontKerning::None),
    ] {
        let value = PropertyValue::FontKerning(expected);
        assert_eq!(parse(input, "font-kerning"), Some(value.clone()));
        assert_eq!(serialize_value(&value), Some(input.to_owned()));
        assert_eq!(value.key(), PropertyKey::FontKerning);
    }
}

#[test]
fn font_kerning_is_case_insensitive_and_rejects_unknown_keywords() {
    assert_eq!(
        parse("NORMAL", "font-kerning"),
        Some(PropertyValue::FontKerning(FontKerning::Normal))
    );
    for input in ["bogus", "italic", "inherit", "1"] {
        assert_eq!(parse(input, "font-kerning"), None);
    }
}

#[test]
fn font_optical_sizing_parses_serializes_and_maps_its_property_key() {
    for (input, expected) in [
        ("auto", FontOpticalSizing::Auto),
        ("none", FontOpticalSizing::None),
    ] {
        let value = PropertyValue::FontOpticalSizing(expected);
        assert_eq!(parse(input, "font-optical-sizing"), Some(value.clone()));
        assert_eq!(serialize_value(&value), Some(input.to_owned()));
        assert_eq!(value.key(), PropertyKey::FontOpticalSizing);
    }
}

#[test]
fn font_optical_sizing_is_case_insensitive_and_rejects_unknown_values() {
    assert_eq!(
        parse("AUTO", "font-optical-sizing"),
        Some(PropertyValue::FontOpticalSizing(FontOpticalSizing::Auto))
    );
    for input in ["normal", "on", "inherit", "1"] {
        assert_eq!(parse(input, "font-optical-sizing"), None);
    }
}

#[test]
fn font_variant_emoji_parses_serializes_and_maps_its_property_key() {
    for (input, expected) in [
        ("normal", FontVariantEmoji::Normal),
        ("text", FontVariantEmoji::Text),
        ("emoji", FontVariantEmoji::Emoji),
        ("unicode", FontVariantEmoji::Unicode),
    ] {
        let value = PropertyValue::FontVariantEmoji(expected);
        assert_eq!(parse(input, "font-variant-emoji"), Some(value.clone()));
        assert_eq!(serialize_value(&value), Some(input.to_owned()));
        assert_eq!(value.key(), PropertyKey::FontVariantEmoji);
    }
}

#[test]
fn font_variant_emoji_is_case_insensitive_and_rejects_unknown_values() {
    assert_eq!(
        parse("UNICODE", "font-variant-emoji"),
        Some(PropertyValue::FontVariantEmoji(FontVariantEmoji::Unicode))
    );
    for input in ["none", "bogus", "inherit", "1"] {
        assert_eq!(parse(input, "font-variant-emoji"), None);
    }
    assert_eq!(parse_entire("text emoji", "font-variant-emoji"), None);
}

#[test]
fn font_language_override_parses_serializes_and_maps_its_property_key() {
    for (input, expected, serialized) in [
        ("normal", FontLanguageOverride::Normal, "normal"),
        (
            "\"KSW\"",
            FontLanguageOverride::String("KSW".into()),
            "\"KSW\"",
        ),
        (
            "\"ENG \"",
            FontLanguageOverride::String("ENG".into()),
            "\"ENG\"",
        ),
        (
            "\"en  \"",
            FontLanguageOverride::String("en".into()),
            "\"en\"",
        ),
        (
            "\" en \"",
            FontLanguageOverride::String(" en".into()),
            "\" en\"",
        ),
    ] {
        let value = PropertyValue::FontLanguageOverride(expected);
        assert_eq!(
            parse_entire(input, "font-language-override"),
            Some(value.clone())
        );
        assert_eq!(serialize_value(&value), Some(serialized.to_owned()));
        assert_eq!(value.key(), PropertyKey::FontLanguageOverride);
    }
}

#[test]
fn font_language_override_accepts_case_insensitive_normal_and_rejects_invalid_values() {
    assert_eq!(
        parse("NORMAL", "font-language-override"),
        Some(PropertyValue::FontLanguageOverride(
            FontLanguageOverride::Normal
        ))
    );
    for input in ["none", "foo", "inherit", "1"] {
        assert_eq!(parse(input, "font-language-override"), None);
    }
    assert_eq!(
        parse_entire("\"KSW\" \"ENG\"", "font-language-override"),
        None
    );
}

#[test]
fn font_variant_ligatures_parses_and_serializes_the_ten_individual_keywords() {
    for (input, expected, serialized) in [
        ("normal", FontVariantLigatures::Normal, "normal"),
        ("none", FontVariantLigatures::None, "none"),
        (
            "common-ligatures",
            FontVariantLigatures::CommonLigatures,
            "common-ligatures",
        ),
        (
            "no-common-ligatures",
            FontVariantLigatures::NoCommonLigatures,
            "no-common-ligatures",
        ),
        (
            "discretionary-ligatures",
            FontVariantLigatures::DiscretionaryLigatures,
            "discretionary-ligatures",
        ),
        (
            "no-discretionary-ligatures",
            FontVariantLigatures::NoDiscretionaryLigatures,
            "no-discretionary-ligatures",
        ),
        (
            "historical-ligatures",
            FontVariantLigatures::HistoricalLigatures,
            "historical-ligatures",
        ),
        (
            "no-historical-ligatures",
            FontVariantLigatures::NoHistoricalLigatures,
            "no-historical-ligatures",
        ),
        ("contextual", FontVariantLigatures::Contextual, "contextual"),
        (
            "no-contextual",
            FontVariantLigatures::NoContextual,
            "no-contextual",
        ),
    ] {
        let value = PropertyValue::FontVariantLigatures(expected);
        assert_eq!(
            parse_entire(input, "font-variant-ligatures"),
            Some(value.clone())
        );
        assert_eq!(serialize_value(&value), Some(serialized.to_owned()));
        assert_eq!(value.key(), PropertyKey::FontVariantLigatures);
    }
}

#[test]
fn font_variant_ligatures_accepts_case_insensitive_keywords_and_rejects_unknown_values() {
    assert_eq!(
        parse("NO-COMMON-LIGATURES", "font-variant-ligatures"),
        Some(PropertyValue::FontVariantLigatures(
            FontVariantLigatures::NoCommonLigatures
        ))
    );
    for input in ["bogus", "inherit", "1"] {
        assert_eq!(parse(input, "font-variant-ligatures"), None, "{input}");
    }
    assert_eq!(parse_entire("normal none", "font-variant-ligatures"), None);
}

#[test]
fn font_variant_position_parses_and_serializes_the_three_computed_keywords() {
    for (input, expected) in [
        ("normal", FontVariantPosition::Normal),
        ("sub", FontVariantPosition::Sub),
        ("super", FontVariantPosition::Super),
    ] {
        let value = parse(input, "font-variant-position");
        assert_eq!(value, Some(PropertyValue::FontVariantPosition(expected)));
        assert_eq!(serialize_value(&value.unwrap()), Some(input.to_owned()));
    }
    for input in ["none", "bogus", "inherit", "1"] {
        assert_eq!(parse(input, "font-variant-position"), None, "{input}");
    }
    assert_eq!(parse_entire("normal sub", "font-variant-position"), None);
}

#[test]
fn font_palette_parses_and_serializes_pinned_keywords_and_dashed_identifier() {
    for (input, expected) in [
        ("normal", FontPaletteValue::Normal),
        ("light", FontPaletteValue::Light),
        ("dark", FontPaletteValue::Dark),
        (
            "--pitchfork",
            FontPaletteValue::Palette("--pitchfork".into()),
        ),
    ] {
        let value = parse(input, "font-palette").expect("parse font-palette value");
        assert_eq!(value, PropertyValue::FontPalette(expected));
        assert_eq!(value.key(), PropertyKey::FontPalette);
        assert_eq!(serialize_value(&value), Some(input.to_owned()));
    }
    for input in ["none", "--", "foo", "palette-mix(dark, --pitchfork)"] {
        assert_eq!(parse(input, "font-palette"), None, "{input}");
    }
    assert_eq!(parse_entire("normal light", "font-palette"), None);
}

#[test]
fn font_variant_numeric_parses_and_serializes_the_pinned_value_set() {
    let cases = [
        ("normal", FontVariantNumeric::initial()),
        (
            "lining-nums",
            FontVariantNumeric {
                lining_nums: true,
                ..FontVariantNumeric::initial()
            },
        ),
        (
            "oldstyle-nums",
            FontVariantNumeric {
                oldstyle_nums: true,
                ..FontVariantNumeric::initial()
            },
        ),
        (
            "proportional-nums",
            FontVariantNumeric {
                proportional_nums: true,
                ..FontVariantNumeric::initial()
            },
        ),
        (
            "tabular-nums",
            FontVariantNumeric {
                tabular_nums: true,
                ..FontVariantNumeric::initial()
            },
        ),
        (
            "diagonal-fractions",
            FontVariantNumeric {
                diagonal_fractions: true,
                ..FontVariantNumeric::initial()
            },
        ),
        (
            "stacked-fractions",
            FontVariantNumeric {
                stacked_fractions: true,
                ..FontVariantNumeric::initial()
            },
        ),
        (
            "ordinal",
            FontVariantNumeric {
                ordinal: true,
                ..FontVariantNumeric::initial()
            },
        ),
        (
            "slashed-zero",
            FontVariantNumeric {
                slashed_zero: true,
                ..FontVariantNumeric::initial()
            },
        ),
        (
            "oldstyle-nums tabular-nums diagonal-fractions",
            FontVariantNumeric {
                oldstyle_nums: true,
                tabular_nums: true,
                diagonal_fractions: true,
                ..FontVariantNumeric::initial()
            },
        ),
        (
            "lining-nums proportional-nums stacked-fractions ordinal slashed-zero",
            FontVariantNumeric {
                lining_nums: true,
                proportional_nums: true,
                stacked_fractions: true,
                ordinal: true,
                slashed_zero: true,
                ..FontVariantNumeric::initial()
            },
        ),
    ];

    for (input, expected) in cases {
        let value = parse(input, "font-variant-numeric").expect("parse font-variant-numeric");
        assert_eq!(value, PropertyValue::FontVariantNumeric(expected));
        assert_eq!(value.key(), PropertyKey::FontVariantNumeric);
        assert_eq!(serialize_value(&value), Some(input.to_owned()));
    }

    for input in [
        "lining-nums oldstyle-nums",
        "proportional-nums tabular-nums",
        "diagonal-fractions stacked-fractions",
        "lining-nums lining-nums",
        "normal ordinal",
        "ordinal normal",
        "none",
    ] {
        assert_eq!(parse_entire(input, "font-variant-numeric"), None, "{input}");
    }
}

#[test]
fn font_variant_east_asian_parses_and_serializes_the_pinned_value_set() {
    let cases = [
        ("normal", FontVariantEastAsian::initial()),
        (
            "jis78",
            FontVariantEastAsian {
                variant: Some(FontVariantEastAsianVariant::Jis78),
                ..FontVariantEastAsian::initial()
            },
        ),
        (
            "jis83",
            FontVariantEastAsian {
                variant: Some(FontVariantEastAsianVariant::Jis83),
                ..FontVariantEastAsian::initial()
            },
        ),
        (
            "jis90",
            FontVariantEastAsian {
                variant: Some(FontVariantEastAsianVariant::Jis90),
                ..FontVariantEastAsian::initial()
            },
        ),
        (
            "jis04",
            FontVariantEastAsian {
                variant: Some(FontVariantEastAsianVariant::Jis04),
                ..FontVariantEastAsian::initial()
            },
        ),
        (
            "simplified",
            FontVariantEastAsian {
                variant: Some(FontVariantEastAsianVariant::Simplified),
                ..FontVariantEastAsian::initial()
            },
        ),
        (
            "traditional",
            FontVariantEastAsian {
                variant: Some(FontVariantEastAsianVariant::Traditional),
                ..FontVariantEastAsian::initial()
            },
        ),
        (
            "full-width",
            FontVariantEastAsian {
                width: Some(FontVariantEastAsianWidth::FullWidth),
                ..FontVariantEastAsian::initial()
            },
        ),
        (
            "proportional-width",
            FontVariantEastAsian {
                width: Some(FontVariantEastAsianWidth::ProportionalWidth),
                ..FontVariantEastAsian::initial()
            },
        ),
        (
            "ruby",
            FontVariantEastAsian {
                ruby: true,
                ..FontVariantEastAsian::initial()
            },
        ),
        (
            "jis78 proportional-width",
            FontVariantEastAsian {
                variant: Some(FontVariantEastAsianVariant::Jis78),
                width: Some(FontVariantEastAsianWidth::ProportionalWidth),
                ruby: false,
            },
        ),
        (
            "simplified full-width ruby",
            FontVariantEastAsian {
                variant: Some(FontVariantEastAsianVariant::Simplified),
                width: Some(FontVariantEastAsianWidth::FullWidth),
                ruby: true,
            },
        ),
    ];

    for (input, expected) in cases {
        let value = parse(input, "font-variant-east-asian").expect("parse font-variant-east-asian");
        assert_eq!(value, PropertyValue::FontVariantEastAsian(expected));
        assert_eq!(value.key(), PropertyKey::FontVariantEastAsian);
        assert_eq!(serialize_value(&value), Some(input.to_owned()));
    }

    for input in [
        "jis78 jis83",
        "jis78 simplified",
        "full-width proportional-width",
        "jis78 jis78",
        "ruby ruby",
        "normal ruby",
        "ruby normal",
        "none",
    ] {
        assert_eq!(
            parse_entire(input, "font-variant-east-asian"),
            None,
            "{input}"
        );
    }
}

#[test]
fn font_synthesis_parses_serializes_and_maps_the_pinned_values() {
    let value = |weight, style, small_caps, position| FontSynthesisValue {
        weight,
        style,
        small_caps,
        position,
    };
    let cases = [
        ("none", FontSynthesisValue::none()),
        (
            "weight",
            value(true, FontSynthesisStyle::None, false, false),
        ),
        (
            "style",
            value(false, FontSynthesisStyle::Auto, false, false),
        ),
        (
            "oblique-only",
            value(false, FontSynthesisStyle::ObliqueOnly, false, false),
        ),
        (
            "small-caps",
            value(false, FontSynthesisStyle::None, true, false),
        ),
        (
            "position",
            value(false, FontSynthesisStyle::None, false, true),
        ),
        (
            "small-caps position",
            value(false, FontSynthesisStyle::None, true, true),
        ),
        (
            "style small-caps",
            value(false, FontSynthesisStyle::Auto, true, false),
        ),
        (
            "style position",
            value(false, FontSynthesisStyle::Auto, false, true),
        ),
        (
            "style small-caps position",
            value(false, FontSynthesisStyle::Auto, true, true),
        ),
        (
            "oblique-only small-caps",
            value(false, FontSynthesisStyle::ObliqueOnly, true, false),
        ),
        (
            "oblique-only position",
            value(false, FontSynthesisStyle::ObliqueOnly, false, true),
        ),
        (
            "oblique-only small-caps position",
            value(false, FontSynthesisStyle::ObliqueOnly, true, true),
        ),
        (
            "weight small-caps",
            value(true, FontSynthesisStyle::None, true, false),
        ),
        (
            "weight style",
            value(true, FontSynthesisStyle::Auto, false, false),
        ),
        (
            "weight oblique-only",
            value(true, FontSynthesisStyle::ObliqueOnly, false, false),
        ),
        (
            "weight position",
            value(true, FontSynthesisStyle::None, false, true),
        ),
        (
            "weight style small-caps",
            value(true, FontSynthesisStyle::Auto, true, false),
        ),
        (
            "weight style small-caps position",
            FontSynthesisValue::initial(),
        ),
        (
            "weight oblique-only small-caps",
            value(true, FontSynthesisStyle::ObliqueOnly, true, false),
        ),
        (
            "weight oblique-only small-caps position",
            value(true, FontSynthesisStyle::ObliqueOnly, true, true),
        ),
    ];

    for (input, expected) in cases {
        let parsed = parse_entire(input, "font-synthesis")
            .unwrap_or_else(|| panic!("expected valid font-synthesis `{input}`"));
        let value = PropertyValue::FontSynthesis(expected);
        assert_eq!(parsed, value, "{input}");
        assert_eq!(serialize_value(&parsed), Some(input.to_owned()), "{input}");
        assert_eq!(parsed.key(), PropertyKey::FontSynthesis, "{input}");
    }
}

#[test]
fn font_synthesis_is_case_insensitive_and_rejects_invalid_or_duplicate_components() {
    assert_eq!(
        parse_entire("WEIGHT ObLiQuE-OnLy", "font-synthesis"),
        Some(PropertyValue::FontSynthesis(FontSynthesisValue {
            weight: true,
            style: FontSynthesisStyle::ObliqueOnly,
            small_caps: false,
            position: false,
        }))
    );
    for input in [
        "",
        "bogus",
        "none weight",
        "weight weight",
        "style oblique-only",
        "style style",
        "small-caps small-caps",
        "position position",
        "position none",
    ] {
        assert_eq!(parse_entire(input, "font-synthesis"), None, "{input}");
    }
}

#[test]
fn font_style_key_maps_to_font_style_property_key() {
    let v = PropertyValue::FontStyle(FontStyle::Normal);
    assert_eq!(v.key(), PropertyKey::FontStyle);
    let v = PropertyValue::FontStyle(FontStyle::Italic);
    assert_eq!(v.key(), PropertyKey::FontStyle);
    let v = PropertyValue::FontStyle(FontStyle::Oblique);
    assert_eq!(v.key(), PropertyKey::FontStyle);
}

// ── font-variant-caps (CSS Fonts Module Level 3 §6.6) ──
//
// Value grammar (§6.6 spec verbatim, full property grammar): `normal |
// small-caps | all-small-caps | petite-caps | all-petite-caps | unicase
// | titling-caps`. This crate implements all 7 keywords
// (`FontVariantCaps` doc's "7 keyword の意味" section). Initial: normal /
// Inherited: yes / Computed value: specified keyword.

#[test]
fn font_variant_caps_parse_all_seven_keywords() {
    assert_eq!(
        parse("normal", "font-variant-caps"),
        Some(PropertyValue::FontVariantCaps(FontVariantCaps::Normal))
    );
    assert_eq!(
        parse("small-caps", "font-variant-caps"),
        Some(PropertyValue::FontVariantCaps(FontVariantCaps::SmallCaps))
    );
    assert_eq!(
        parse("all-small-caps", "font-variant-caps"),
        Some(PropertyValue::FontVariantCaps(
            FontVariantCaps::AllSmallCaps
        ))
    );
    assert_eq!(
        parse("petite-caps", "font-variant-caps"),
        Some(PropertyValue::FontVariantCaps(FontVariantCaps::PetiteCaps))
    );
    assert_eq!(
        parse("all-petite-caps", "font-variant-caps"),
        Some(PropertyValue::FontVariantCaps(
            FontVariantCaps::AllPetiteCaps
        ))
    );
    assert_eq!(
        parse("unicase", "font-variant-caps"),
        Some(PropertyValue::FontVariantCaps(FontVariantCaps::Unicase))
    );
    assert_eq!(
        parse("titling-caps", "font-variant-caps"),
        Some(PropertyValue::FontVariantCaps(FontVariantCaps::TitlingCaps))
    );
}

#[test]
fn font_variant_caps_is_case_insensitive() {
    assert_eq!(
        parse("NORMAL", "font-variant-caps"),
        Some(PropertyValue::FontVariantCaps(FontVariantCaps::Normal))
    );
    assert_eq!(
        parse("Small-Caps", "font-variant-caps"),
        Some(PropertyValue::FontVariantCaps(FontVariantCaps::SmallCaps))
    );
    assert_eq!(
        parse("ALL-SMALL-CAPS", "font-variant-caps"),
        Some(PropertyValue::FontVariantCaps(
            FontVariantCaps::AllSmallCaps
        ))
    );
    assert_eq!(
        parse("Titling-Caps", "font-variant-caps"),
        Some(PropertyValue::FontVariantCaps(FontVariantCaps::TitlingCaps))
    );
}

#[test]
fn font_variant_caps_rejects_unknown_keyword() {
    // (a) spec-invalid — not one of the 7 keywords in the property
    // grammar.
    assert_eq!(parse("bogus", "font-variant-caps"), None);
}

#[test]
fn font_variant_caps_rejects_css_wide_keyword() {
    // (b) not supported — CSS-wide keyword is unimplemented (future work),
    // silent drop (`PropertyValue` doc's "CSS-wide keyword" section is canonical).
    for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(kw, "font-variant-caps"), None);
    }
}

#[test]
fn font_variant_caps_rejects_non_ident() {
    assert_eq!(parse("16px", "font-variant-caps"), None);
    assert_eq!(parse(r#""small-caps""#, "font-variant-caps"), None);
}

#[test]
fn font_variant_caps_key_maps_to_font_variant_caps_property_key() {
    for value in [
        FontVariantCaps::Normal,
        FontVariantCaps::SmallCaps,
        FontVariantCaps::AllSmallCaps,
        FontVariantCaps::PetiteCaps,
        FontVariantCaps::AllPetiteCaps,
        FontVariantCaps::Unicase,
        FontVariantCaps::TitlingCaps,
    ] {
        let v = PropertyValue::FontVariantCaps(value);
        assert_eq!(v.key(), PropertyKey::FontVariantCaps);
    }
}

#[test]
fn font_variant_caps_shorthand_name_has_no_dispatch_arm() {
    // `font-variant` (the shorthand, not `font-variant-caps`) resets
    // longhands this crate doesn't have (`FontVariantCaps` doc's
    // "Scope carving" section) — it is unrecognized like any other
    // unknown property name, not silently mapped to `-caps`.
    assert_eq!(parse("small-caps", "font-variant"), None);
}

// ── text-transform (CSS Text Module Level 4) ──
//
// Value grammar (spec verbatim): `none | [capitalize | uppercase |
// lowercase] || full-width || full-size-kana | math-auto`. Initial: none /
// Inherited: yes / Computed value: specified keyword.

#[test]
fn text_transform_parse_all_four_keywords() {
    assert_eq!(
        parse("none", "text-transform"),
        Some(PropertyValue::TextTransform(TextTransform::None))
    );
    assert_eq!(
        parse("capitalize", "text-transform"),
        Some(PropertyValue::TextTransform(TextTransform::Capitalize))
    );
    assert_eq!(
        parse("uppercase", "text-transform"),
        Some(PropertyValue::TextTransform(TextTransform::Uppercase))
    );
    assert_eq!(
        parse("lowercase", "text-transform"),
        Some(PropertyValue::TextTransform(TextTransform::Lowercase))
    );
}

#[test]
fn text_transform_is_case_insensitive() {
    assert_eq!(
        parse("NONE", "text-transform"),
        Some(PropertyValue::TextTransform(TextTransform::None))
    );
    assert_eq!(
        parse("Uppercase", "text-transform"),
        Some(PropertyValue::TextTransform(TextTransform::Uppercase))
    );
}

#[test]
fn text_transform_parses_math_auto_as_separate_keyword() {
    assert_eq!(
        parse_entire("math-auto", "text-transform"),
        Some(PropertyValue::TextTransform(TextTransform::MathAuto)),
    );
    assert_eq!(
        parse_entire("MATH-AUTO", "text-transform"),
        Some(PropertyValue::TextTransform(TextTransform::MathAuto)),
    );
}

#[test]
fn text_transform_rejects_math_auto_combinations() {
    for input in [
        "math-auto uppercase",
        "uppercase math-auto",
        "math-auto full-width",
        "full-size-kana math-auto",
        "math-auto none",
    ] {
        assert_eq!(parse_entire(input, "text-transform"), None, "{input}");
    }
}

#[test]
fn text_transform_accepts_width_keywords_and_combinations() {
    assert_eq!(
        parse("full-width", "text-transform"),
        Some(PropertyValue::TextTransform(TextTransform::FullWidth))
    );
    assert_eq!(
        parse("full-size-kana", "text-transform"),
        Some(PropertyValue::TextTransform(TextTransform::FullSizeKana))
    );
    assert_eq!(
        parse("full-width full-size-kana lowercase", "text-transform"),
        Some(PropertyValue::TextTransform(
            TextTransform::LowercaseFullWidthFullSizeKana
        ))
    );
    assert_eq!(
        parse("full-size-kana capitalize", "text-transform"),
        Some(PropertyValue::TextTransform(
            TextTransform::CapitalizeFullSizeKana
        ))
    );
    assert_eq!(
        parse("full-width uppercase", "text-transform"),
        Some(PropertyValue::TextTransform(
            TextTransform::UppercaseFullWidth
        ))
    );
    assert_eq!(parse("uppercase uppercase", "text-transform"), None);
    assert_eq!(parse("full-width full-width", "text-transform"), None);
}

#[test]
fn text_transform_rejects_unknown_keyword() {
    assert_eq!(parse("bogus", "text-transform"), None);
}

#[test]
fn text_transform_rejects_css_wide_keyword() {
    // (b) not supported — CSS-wide keyword is unimplemented (future work),
    // silent drop (`PropertyValue` doc's "CSS-wide keyword" section is canonical).
    for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(kw, "text-transform"), None);
    }
}

#[test]
fn text_transform_rejects_non_ident() {
    assert_eq!(parse("16px", "text-transform"), None);
    assert_eq!(parse(r#""uppercase""#, "text-transform"), None);
}

#[test]
fn text_transform_key_maps_to_text_transform_property_key() {
    let v = PropertyValue::TextTransform(TextTransform::None);
    assert_eq!(v.key(), PropertyKey::TextTransform);
    let v = PropertyValue::TextTransform(TextTransform::Capitalize);
    assert_eq!(v.key(), PropertyKey::TextTransform);
    let v = PropertyValue::TextTransform(TextTransform::MathAuto);
    assert_eq!(v.key(), PropertyKey::TextTransform);
}

// ── word-break (CSS Text 3 §5.1) ──
//
// Value grammar (§5.1 spec verbatim, full property grammar): `normal |
// keep-all | break-all | break-word`. This crate implements only
// normal / keep-all / break-all (`WordBreak` doc's "Scope carving"
// section) — the 4th, deprecated `break-word` keyword is excluded.
// Initial: normal / Inherited: yes / Computed value: specified keyword.

#[test]
fn word_break_parse_all_implemented_keywords() {
    assert_eq!(
        parse("normal", "word-break"),
        Some(PropertyValue::WordBreak(WordBreak::Normal))
    );
    assert_eq!(
        parse("keep-all", "word-break"),
        Some(PropertyValue::WordBreak(WordBreak::KeepAll))
    );
    assert_eq!(
        parse("break-all", "word-break"),
        Some(PropertyValue::WordBreak(WordBreak::BreakAll))
    );
}

#[test]
fn word_break_is_case_insensitive() {
    assert_eq!(
        parse("NORMAL", "word-break"),
        Some(PropertyValue::WordBreak(WordBreak::Normal))
    );
    assert_eq!(
        parse("Keep-All", "word-break"),
        Some(PropertyValue::WordBreak(WordBreak::KeepAll))
    );
    assert_eq!(
        parse("BREAK-ALL", "word-break"),
        Some(PropertyValue::WordBreak(WordBreak::BreakAll))
    );
}

#[test]
fn word_break_accepts_break_word_and_level4_keywords() {
    assert_eq!(
        parse("break-word", "word-break"),
        Some(PropertyValue::WordBreak(WordBreak::BreakWord))
    );
    assert_eq!(
        parse("manual", "word-break"),
        Some(PropertyValue::WordBreak(WordBreak::Manual))
    );
    assert_eq!(
        parse("auto-phrase", "word-break"),
        Some(PropertyValue::WordBreak(WordBreak::AutoPhrase))
    );
}

#[test]
fn word_break_rejects_unknown_keyword() {
    assert_eq!(parse("bogus", "word-break"), None);
}

#[test]
fn word_break_rejects_css_wide_keyword() {
    // (b) not supported — CSS-wide keyword is unimplemented (future work),
    // silent drop (`PropertyValue` doc's "CSS-wide keyword" section is canonical).
    for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(kw, "word-break"), None);
    }
}

#[test]
fn word_break_rejects_non_ident() {
    assert_eq!(parse("16px", "word-break"), None);
    assert_eq!(parse(r#""normal""#, "word-break"), None);
}

#[test]
fn word_break_key_maps_to_word_break_property_key() {
    let v = PropertyValue::WordBreak(WordBreak::Normal);
    assert_eq!(v.key(), PropertyKey::WordBreak);
    let v = PropertyValue::WordBreak(WordBreak::KeepAll);
    assert_eq!(v.key(), PropertyKey::WordBreak);
    let v = PropertyValue::WordBreak(WordBreak::BreakAll);
    assert_eq!(v.key(), PropertyKey::WordBreak);
}

// ── overflow-wrap / word-wrap legacy alias (CSS Text 3 §5.4) ──
//
// Value grammar (§5.4 spec verbatim): `normal | break-word | anywhere`.
// All 3 keywords are implemented — unlike `word-break`'s deprecated
// `break-word`, `overflow-wrap: break-word` is not deprecated
// (`OverflowWrap` doc). `word-wrap` is the spec-mandated legacy name
// alias and must parse identically. Initial: normal / Inherited: yes /
// Computed value: specified keyword.

#[test]
fn overflow_wrap_parse_all_keywords() {
    assert_eq!(
        parse("normal", "overflow-wrap"),
        Some(PropertyValue::OverflowWrap(OverflowWrap::Normal))
    );
    assert_eq!(
        parse("break-word", "overflow-wrap"),
        Some(PropertyValue::OverflowWrap(OverflowWrap::BreakWord))
    );
    assert_eq!(
        parse("anywhere", "overflow-wrap"),
        Some(PropertyValue::OverflowWrap(OverflowWrap::Anywhere))
    );
}

#[test]
fn overflow_wrap_is_case_insensitive() {
    assert_eq!(
        parse("NORMAL", "overflow-wrap"),
        Some(PropertyValue::OverflowWrap(OverflowWrap::Normal))
    );
    assert_eq!(
        parse("Break-Word", "overflow-wrap"),
        Some(PropertyValue::OverflowWrap(OverflowWrap::BreakWord))
    );
    assert_eq!(
        parse("ANYWHERE", "overflow-wrap"),
        Some(PropertyValue::OverflowWrap(OverflowWrap::Anywhere))
    );
}

#[test]
fn word_wrap_legacy_alias_parses_identically_to_overflow_wrap() {
    // CSS Text 3 §5.4 verbatim: "For legacy reasons, UAs must treat
    // word-wrap as a legacy name alias of the overflow-wrap property."
    for (kw, expected) in [
        ("normal", OverflowWrap::Normal),
        ("break-word", OverflowWrap::BreakWord),
        ("anywhere", OverflowWrap::Anywhere),
    ] {
        assert_eq!(
            parse(kw, "word-wrap"),
            Some(PropertyValue::OverflowWrap(expected))
        );
        assert_eq!(parse(kw, "word-wrap"), parse(kw, "overflow-wrap"));
    }
}

#[test]
fn overflow_wrap_rejects_unknown_keyword() {
    assert_eq!(parse("bogus", "overflow-wrap"), None);
    assert_eq!(parse("bogus", "word-wrap"), None);
}

#[test]
fn overflow_wrap_rejects_css_wide_keyword() {
    // (b) not supported — CSS-wide keyword is unimplemented (future work),
    // silent drop (`PropertyValue` doc's "CSS-wide keyword" section is canonical).
    for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(kw, "overflow-wrap"), None);
        assert_eq!(parse(kw, "word-wrap"), None);
    }
}

#[test]
fn overflow_wrap_rejects_non_ident() {
    assert_eq!(parse("16px", "overflow-wrap"), None);
    assert_eq!(parse(r#""normal""#, "overflow-wrap"), None);
}

#[test]
fn overflow_wrap_key_maps_to_overflow_wrap_property_key() {
    // `word-wrap` and `overflow-wrap` share one `PropertyKey` — the
    // legacy alias cascades as one property, not two independently
    // winning ones (`OverflowWrap` doc's "legacy alias" section).
    let v = PropertyValue::OverflowWrap(OverflowWrap::Normal);
    assert_eq!(v.key(), PropertyKey::OverflowWrap);
    let v = PropertyValue::OverflowWrap(OverflowWrap::BreakWord);
    assert_eq!(v.key(), PropertyKey::OverflowWrap);
    let v = PropertyValue::OverflowWrap(OverflowWrap::Anywhere);
    assert_eq!(v.key(), PropertyKey::OverflowWrap);
}

// ── letter-spacing / word-spacing computed values ──
//
// Both are inherited and initially `normal`. `word-spacing` uses the same
// CSS Text 4 length-percentage/calc representation as `letter-spacing`.

#[test]
fn letter_spacing_parse_normal_keyword() {
    assert_eq!(
        parse("normal", "letter-spacing"),
        Some(PropertyValue::LetterSpacing(LetterSpacingValue::Normal))
    );
}

#[test]
fn word_spacing_parse_normal_keyword() {
    assert_eq!(
        parse("normal", "word-spacing"),
        Some(PropertyValue::WordSpacing(WordSpacingValue::Normal))
    );
}

#[test]
fn letter_spacing_is_case_insensitive_normal() {
    assert_eq!(
        parse("NORMAL", "letter-spacing"),
        Some(PropertyValue::LetterSpacing(LetterSpacingValue::Normal))
    );
    assert_eq!(
        parse("Normal", "word-spacing"),
        Some(PropertyValue::WordSpacing(WordSpacingValue::Normal))
    );
}

#[test]
fn letter_spacing_parse_length_px() {
    assert_eq!(
        parse("2px", "letter-spacing"),
        Some(PropertyValue::LetterSpacing(LetterSpacingValue::Length(
            Length::Px(2.0)
        )))
    );
}

#[test]
fn word_spacing_parse_length_px() {
    assert_eq!(
        parse("4px", "word-spacing"),
        Some(PropertyValue::WordSpacing(WordSpacingValue::Length(
            Length::Px(4.0)
        )))
    );
}

#[test]
fn letter_spacing_accepts_length_em_rem_pt() {
    assert_eq!(
        parse("0.1em", "letter-spacing"),
        Some(PropertyValue::LetterSpacing(LetterSpacingValue::Length(
            Length::Em(0.1)
        )))
    );
    assert_eq!(
        parse("1rem", "letter-spacing"),
        Some(PropertyValue::LetterSpacing(LetterSpacingValue::Length(
            Length::Rem(1.0)
        )))
    );
    assert_eq!(
        parse("2pt", "letter-spacing"),
        Some(PropertyValue::LetterSpacing(LetterSpacingValue::Length(
            Length::Pt(2.0)
        )))
    );
}

// Spec verbatim (both §7.1 and §7.2): "Values may be negative, but there
// may be implementation-dependent limits." — unlike `line-height` /
// `font-size` / `padding` etc., this property does NOT reject negative
// lengths at parse time. `letter-spacing` and `word-spacing` use separate
// parser entry points.
#[test]
fn letter_spacing_accepts_negative_length() {
    assert_eq!(
        parse("-2px", "letter-spacing"),
        Some(PropertyValue::LetterSpacing(LetterSpacingValue::Length(
            Length::Px(-2.0)
        )))
    );
    assert_eq!(
        parse("-0.05em", "letter-spacing"),
        Some(PropertyValue::LetterSpacing(LetterSpacingValue::Length(
            Length::Em(-0.05)
        )))
    );
}

#[test]
fn word_spacing_accepts_negative_length() {
    assert_eq!(
        parse("-1px", "word-spacing"),
        Some(PropertyValue::WordSpacing(WordSpacingValue::Length(
            Length::Px(-1.0)
        )))
    );
}

// Bare `0` has no `<number>` grammar alternative to disambiguate against
// here (unlike `line-height`) — it goes through `parse_length_value`'s
// unitless-zero clause and becomes `Length::Px(0.0)`, not `Normal`.
#[test]
fn letter_spacing_unitless_zero_is_length_not_normal() {
    assert_eq!(
        parse("0", "letter-spacing"),
        Some(PropertyValue::LetterSpacing(LetterSpacingValue::Length(
            Length::Px(0.0)
        )))
    );
}

// The pinned computed-value case exercises percentages. Keep the parser's
// accepted value explicit here, independent of the separate calc tests below.
#[test]
fn letter_spacing_accepts_percentage() {
    assert_eq!(
        parse("5%", "letter-spacing"),
        Some(PropertyValue::LetterSpacing(LetterSpacingValue::Length(
            Length::Percent(5.0)
        )))
    );
}

#[test]
fn letter_spacing_parses_em_calc_for_computed_resolution() {
    assert_eq!(
        parse("calc(10px - 0.5em)", "letter-spacing"),
        Some(PropertyValue::LetterSpacing(LetterSpacingValue::Calc(
            LengthPercentageCalc {
                percent: 0.0,
                px: 10.0,
                em: -0.5,
            }
        )))
    );
}

#[test]
fn letter_spacing_simplifies_percentage_calc() {
    assert_eq!(
        parse("calc(10% - 20%)", "letter-spacing"),
        Some(PropertyValue::LetterSpacing(LetterSpacingValue::Length(
            Length::Percent(-10.0)
        )))
    );
}

#[test]
fn letter_spacing_parses_parenthesized_mixed_calc() {
    assert_eq!(
        parse("calc(10px - (5% + 10%))", "letter-spacing"),
        Some(PropertyValue::LetterSpacing(LetterSpacingValue::Calc(
            LengthPercentageCalc {
                percent: -15.0,
                px: 10.0,
                em: 0.0,
            }
        )))
    );
}

#[test]
fn word_spacing_accepts_percentage() {
    assert_eq!(
        parse("5%", "word-spacing"),
        Some(PropertyValue::WordSpacing(WordSpacingValue::Length(
            Length::Percent(5.0)
        )))
    );
}

#[test]
fn word_spacing_preserves_mixed_length_percentage_calcs() {
    assert_eq!(
        parse("calc(10px - 0.5em)", "word-spacing"),
        Some(PropertyValue::WordSpacing(WordSpacingValue::Calc(
            LengthPercentageCalc {
                percent: 0.0,
                px: 10.0,
                em: -0.5,
            }
        )))
    );
    assert_eq!(
        parse("calc(10% - 20%)", "word-spacing"),
        Some(PropertyValue::WordSpacing(WordSpacingValue::Length(
            Length::Percent(-10.0)
        )))
    );
    assert_eq!(
        parse("calc(10px - (5% + 10%))", "word-spacing"),
        Some(PropertyValue::WordSpacing(WordSpacingValue::Calc(
            LengthPercentageCalc {
                percent: -15.0,
                px: 10.0,
                em: 0.0,
            }
        )))
    );
}

#[test]
fn letter_spacing_rejects_unknown_keyword() {
    assert_eq!(parse("bogus", "letter-spacing"), None);
    assert_eq!(parse("auto", "letter-spacing"), None);
}

#[test]
fn word_spacing_rejects_unknown_keyword() {
    assert_eq!(parse("bogus", "word-spacing"), None);
}

#[test]
fn letter_spacing_rejects_css_wide_keyword() {
    // (b) not supported — CSS-wide keyword is unimplemented (future work),
    // silent drop (`PropertyValue` doc's "CSS-wide keyword" section is canonical).
    for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(kw, "letter-spacing"), None);
    }
}

#[test]
fn word_spacing_rejects_css_wide_keyword() {
    for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(kw, "word-spacing"), None);
    }
}

#[test]
fn letter_spacing_rejects_non_length_non_ident() {
    assert_eq!(parse(r#""2px""#, "letter-spacing"), None);
}

#[test]
fn letter_spacing_key_maps_to_letter_spacing_property_key() {
    let v = PropertyValue::LetterSpacing(LetterSpacingValue::Normal);
    assert_eq!(v.key(), PropertyKey::LetterSpacing);
    let v = PropertyValue::LetterSpacing(LetterSpacingValue::Length(Length::Px(2.0)));
    assert_eq!(v.key(), PropertyKey::LetterSpacing);
}

// ── tab-size (CSS Text Module Level 3 §4.2) ──
//
// Value grammar: `<number [0,∞]> | <length [0,∞]>`. Initial: `8`.
// Inherited: yes. Percentages: N/A. Shares `parse_line_height`'s
// Number-then-Length ordering (minus the `normal` branch tab-size's
// grammar doesn't have) — see `parse_tab_size` doc.

#[test]
fn tab_size_parse_bare_number() {
    assert_eq!(
        parse("4", "tab-size"),
        Some(PropertyValue::TabSize(TabSize::Number(4.0)))
    );
}

#[test]
fn tab_size_parse_length_px() {
    assert_eq!(
        parse("32px", "tab-size"),
        Some(PropertyValue::TabSize(TabSize::Length(Length::Px(32.0))))
    );
}

#[test]
fn tab_size_accepts_length_em_rem_pt() {
    assert_eq!(
        parse("2em", "tab-size"),
        Some(PropertyValue::TabSize(TabSize::Length(Length::Em(2.0))))
    );
    assert_eq!(
        parse("1rem", "tab-size"),
        Some(PropertyValue::TabSize(TabSize::Length(Length::Rem(1.0))))
    );
    assert_eq!(
        parse("6pt", "tab-size"),
        Some(PropertyValue::TabSize(TabSize::Length(Length::Pt(6.0))))
    );
}

#[test]
fn tab_size_number_vs_em_are_distinct_variants() {
    // Same load-bearing Number-vs-Length distinction as `line-height`
    // (`line_height_number_vs_em_are_distinct_variants`) — unitless `4`
    // and dimensioned `4em` map to different variants even though the
    // scalar matches.
    let number = parse("4", "tab-size");
    let length_em = parse("4em", "tab-size");
    assert_eq!(number, Some(PropertyValue::TabSize(TabSize::Number(4.0))));
    assert_eq!(
        length_em,
        Some(PropertyValue::TabSize(TabSize::Length(Length::Em(4.0))))
    );
    assert_ne!(number, length_em, "Number and Length must be distinct");
}

#[test]
fn tab_size_accepts_zero_number_and_length() {
    // spec `[0,∞]`: 0 is a valid boundary value. Same CSS Values 3 §5
    // "0 could be parsed as either a `<number>` or a `<length>`... must
    // parse as a `<number>`" clause `line-height` exercises
    // (`line_height_accepts_zero_number_and_length`) — bare `0` commits
    // to the Number branch before the Length branch is ever tried.
    assert_eq!(
        parse("0", "tab-size"),
        Some(PropertyValue::TabSize(TabSize::Number(0.0)))
    );
    assert_eq!(
        parse("0px", "tab-size"),
        Some(PropertyValue::TabSize(TabSize::Length(Length::Px(0.0))))
    );
}

#[test]
fn tab_size_rejects_negative_number() {
    // spec verbatim: "Negative values are not allowed."
    assert_eq!(parse("-4", "tab-size"), None);
}

#[test]
fn tab_size_rejects_negative_length() {
    assert_eq!(parse("-10px", "tab-size"), None);
    assert_eq!(parse("-1em", "tab-size"), None);
}

#[test]
fn tab_size_rejects_negative_number_with_trailing_length() {
    // Regression guard mirroring
    // `line_height_rejects_negative_number_with_trailing_length`: the
    // Number branch must not fall through to the Length branch once a
    // Token::Number has been consumed via `try_parse`'s `Ok` path
    // (which does not rewind). A fallthrough implementation would let
    // `tab-size: -1 20px` silently accept `20px`.
    assert_eq!(parse("-1 20px", "tab-size"), None);
    assert_eq!(parse("-1 2em", "tab-size"), None);
}

#[test]
fn tab_size_rejects_percentage() {
    // spec propdef: "Percentages: N/A".
    assert_eq!(parse("50%", "tab-size"), None);
}

#[test]
fn tab_size_parses_additive_length_calc() {
    // Same additive `px` + `em` path as `letter-spacing` /
    // `word-spacing` (`letter_spacing_parses_em_calc_for_computed_resolution`
    // sibling) — deferred until computed font-size resolution.
    assert_eq!(
        parse("calc(10px + 0.5em)", "tab-size"),
        Some(PropertyValue::TabSize(TabSize::Calc(
            LengthPercentageCalc {
                percent: 0.0,
                px: 10.0,
                em: 0.5,
            }
        )))
    );
    assert_eq!(
        parse("calc(10px - 0.5em)", "tab-size"),
        Some(PropertyValue::TabSize(TabSize::Calc(
            LengthPercentageCalc {
                percent: 0.0,
                px: 10.0,
                em: -0.5,
            }
        )))
    );
}

#[test]
fn tab_size_collapses_single_value_calc_to_length() {
    // `parse_text_indent_calc_terms` folds a pure-`px` sum to `Length`,
    // so it takes the same non-negative filter as a plain length.
    assert_eq!(
        parse("calc(10px)", "tab-size"),
        Some(PropertyValue::TabSize(TabSize::Length(Length::Px(10.0))))
    );
}

#[test]
fn tab_size_rejects_percentage_calc_terms() {
    // spec propdef "Percentages: N/A" — percentage inside `calc()` is
    // also invalid (same guard as the `parse_value` deferred path).
    assert_eq!(parse("calc(10px + 5%)", "tab-size"), None);
    assert_eq!(parse("calc(50%)", "tab-size"), None);
}

#[test]
fn tab_size_rejects_unknown_keyword() {
    assert_eq!(parse("bogus", "tab-size"), None);
    assert_eq!(parse("auto", "tab-size"), None);
    assert_eq!(parse("normal", "tab-size"), None);
}

#[test]
fn tab_size_rejects_css_wide_keyword() {
    // (b) not supported — CSS-wide keyword is unimplemented (future
    // work), silent drop (`PropertyValue` doc's "CSS-wide keyword"
    // section is canonical).
    for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(kw, "tab-size"), None);
    }
}

#[test]
fn tab_size_rejects_non_length_non_ident() {
    assert_eq!(parse(r#""4""#, "tab-size"), None);
}

#[test]
fn tab_size_key_maps_to_tab_size_property_key() {
    let v = PropertyValue::TabSize(TabSize::Number(4.0));
    assert_eq!(v.key(), PropertyKey::TabSize);
    let v = PropertyValue::TabSize(TabSize::Length(Length::Px(32.0)));
    assert_eq!(v.key(), PropertyKey::TabSize);
}

#[test]
fn word_spacing_key_maps_to_word_spacing_property_key() {
    let v = PropertyValue::WordSpacing(WordSpacingValue::Normal);
    assert_eq!(v.key(), PropertyKey::WordSpacing);
    let v = PropertyValue::WordSpacing(WordSpacingValue::Length(Length::Px(2.0)));
    assert_eq!(v.key(), PropertyKey::WordSpacing);
}

// ── white-space (CSS Text Module Level 3 §3) ──
//
// Value grammar (§3 spec verbatim, full property grammar): `normal | pre
// | nowrap | pre-wrap | break-spaces | pre-line`. This crate implements
// only normal / pre / nowrap / pre-wrap / pre-line (`WhiteSpace` doc's
// "Scope carving" section) — the 6th keyword `break-spaces` is excluded.
// Initial: normal / Inherited: yes / Computed value: specified keyword.

#[test]
fn white_space_parse_all_implemented_keywords() {
    assert_eq!(
        parse("normal", "white-space"),
        Some(PropertyValue::WhiteSpace(WhiteSpace::Normal))
    );
    assert_eq!(
        parse("pre", "white-space"),
        Some(PropertyValue::WhiteSpace(WhiteSpace::Pre))
    );
    assert_eq!(
        parse("nowrap", "white-space"),
        Some(PropertyValue::WhiteSpace(WhiteSpace::Nowrap))
    );
    assert_eq!(
        parse("pre-wrap", "white-space"),
        Some(PropertyValue::WhiteSpace(WhiteSpace::PreWrap))
    );
    assert_eq!(
        parse("pre-line", "white-space"),
        Some(PropertyValue::WhiteSpace(WhiteSpace::PreLine))
    );
}

#[test]
fn white_space_is_case_insensitive() {
    assert_eq!(
        parse("NORMAL", "white-space"),
        Some(PropertyValue::WhiteSpace(WhiteSpace::Normal))
    );
    assert_eq!(
        parse("Pre-Wrap", "white-space"),
        Some(PropertyValue::WhiteSpace(WhiteSpace::PreWrap))
    );
    assert_eq!(
        parse("PRE-LINE", "white-space"),
        Some(PropertyValue::WhiteSpace(WhiteSpace::PreLine))
    );
}

#[test]
fn white_space_accepts_break_spaces() {
    assert_eq!(
        parse("break-spaces", "white-space"),
        Some(PropertyValue::WhiteSpace(WhiteSpace::BreakSpaces))
    );
}

#[test]
fn white_space_rejects_unknown_keyword() {
    assert_eq!(parse("bogus", "white-space"), None);
}

#[test]
fn white_space_rejects_css_wide_keyword() {
    // (b) not supported — CSS-wide keyword is unimplemented (future work),
    // silent drop (`PropertyValue` doc's "CSS-wide keyword" section is canonical).
    for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(kw, "white-space"), None);
    }
}

#[test]
fn white_space_rejects_non_ident() {
    assert_eq!(parse("16px", "white-space"), None);
    assert_eq!(parse(r#""pre""#, "white-space"), None);
}

#[test]
fn white_space_key_maps_to_white_space_property_key() {
    let v = PropertyValue::WhiteSpace(WhiteSpace::Normal);
    assert_eq!(v.key(), PropertyKey::WhiteSpace);
    let v = PropertyValue::WhiteSpace(WhiteSpace::Pre);
    assert_eq!(v.key(), PropertyKey::WhiteSpace);
    let v = PropertyValue::WhiteSpace(WhiteSpace::Nowrap);
    assert_eq!(v.key(), PropertyKey::WhiteSpace);
    let v = PropertyValue::WhiteSpace(WhiteSpace::PreWrap);
    assert_eq!(v.key(), PropertyKey::WhiteSpace);
    let v = PropertyValue::WhiteSpace(WhiteSpace::PreLine);
    assert_eq!(v.key(), PropertyKey::WhiteSpace);
}

// ── white-space-collapse (CSS Text 4) ──

#[test]
fn white_space_collapse_parses_all_six_keywords_and_maps_to_its_key() {
    let cases = [
        ("collapse", WhiteSpaceCollapse::Collapse),
        ("discard", WhiteSpaceCollapse::Discard),
        ("preserve", WhiteSpaceCollapse::Preserve),
        ("preserve-breaks", WhiteSpaceCollapse::PreserveBreaks),
        ("preserve-spaces", WhiteSpaceCollapse::PreserveSpaces),
        ("break-spaces", WhiteSpaceCollapse::BreakSpaces),
    ];

    for (input, expected) in cases {
        let value = parse(input, "white-space-collapse");
        assert_eq!(
            value,
            Some(PropertyValue::WhiteSpaceCollapse(expected)),
            "{input} must parse as its distinct white-space-collapse keyword",
        );
        assert_eq!(
            value.map(|value| value.key()),
            Some(PropertyKey::WhiteSpaceCollapse),
        );
    }
}

#[test]
fn white_space_collapse_is_case_insensitive() {
    assert_eq!(
        parse("PRESERVE-BREAKS", "white-space-collapse"),
        Some(PropertyValue::WhiteSpaceCollapse(
            WhiteSpaceCollapse::PreserveBreaks
        )),
    );
}

#[test]
fn white_space_collapse_rejects_invalid_values() {
    for input in [
        "bogus",
        "inherit",
        "initial",
        "unset",
        "revert",
        "revert-layer",
    ] {
        assert_eq!(parse(input, "white-space-collapse"), None, "{input}");
    }
    assert_eq!(parse("16px", "white-space-collapse"), None);
    assert_eq!(parse(r#""preserve""#, "white-space-collapse"), None);
}

// ── hyphens (CSS Text 3 §5.3) ──
//
// grammar: `none | manual | auto` (`Hyphens` doc's Scope carving section
// — this crate implements all 3 spec-valid keywords, unlike `word-break`
// or `white-space`). Initial: manual / Inherited: yes / Computed value:
// specified keyword.

#[test]
fn hyphens_parse_all_implemented_keywords() {
    assert_eq!(
        parse("none", "hyphens"),
        Some(PropertyValue::Hyphens(Hyphens::None))
    );
    assert_eq!(
        parse("manual", "hyphens"),
        Some(PropertyValue::Hyphens(Hyphens::Manual))
    );
    // `auto` parses to its own distinct variant, not `Hyphens::Manual`
    // — the collapse to soft-hyphen-only splitting is a downstream
    // consumer decision (`Hyphens` doc's "Downstream handoff" section),
    // not something this parser (or the computed value) performs.
    assert_eq!(
        parse("auto", "hyphens"),
        Some(PropertyValue::Hyphens(Hyphens::Auto))
    );
}

#[test]
fn hyphens_is_case_insensitive() {
    assert_eq!(
        parse("NONE", "hyphens"),
        Some(PropertyValue::Hyphens(Hyphens::None))
    );
    assert_eq!(
        parse("Manual", "hyphens"),
        Some(PropertyValue::Hyphens(Hyphens::Manual))
    );
    assert_eq!(
        parse("AUTO", "hyphens"),
        Some(PropertyValue::Hyphens(Hyphens::Auto))
    );
}

#[test]
fn hyphens_rejects_unknown_keyword() {
    assert_eq!(parse("bogus", "hyphens"), None);
}

#[test]
fn hyphens_rejects_css_wide_keyword() {
    // (b) not supported — CSS-wide keyword is unimplemented (future work),
    // silent drop (`PropertyValue` doc's "CSS-wide keyword" section is canonical).
    for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(kw, "hyphens"), None);
    }
}

#[test]
fn hyphens_rejects_non_ident() {
    assert_eq!(parse("16px", "hyphens"), None);
    assert_eq!(parse(r#""manual""#, "hyphens"), None);
}

#[test]
fn hyphens_key_maps_to_hyphens_property_key() {
    let v = PropertyValue::Hyphens(Hyphens::None);
    assert_eq!(v.key(), PropertyKey::Hyphens);
    let v = PropertyValue::Hyphens(Hyphens::Manual);
    assert_eq!(v.key(), PropertyKey::Hyphens);
    let v = PropertyValue::Hyphens(Hyphens::Auto);
    assert_eq!(v.key(), PropertyKey::Hyphens);
}

// ── resolve_text_align_match_parent (CSS Text 3 §6.1
// `#valdef-text-align-match-parent`) ──
//
// This is the shared resolver both `SpecifiedValues::finalize` (element
// path) and `cascade::resolve_against_inherited` (page path) funnel into
// — see the function doc for why `apply_value` itself is deliberately
// *not* a caller (the same-node winner-order hazard between `direction`
// and `text-align`).

#[test]
fn match_parent_resolves_start_against_ltr_parent_to_left() {
    assert_eq!(
        resolve_text_align_match_parent(TextAlign::MatchParent, TextAlign::Start, Direction::Ltr),
        TextAlign::Left
    );
}

#[test]
fn match_parent_resolves_start_against_rtl_parent_to_right() {
    assert_eq!(
        resolve_text_align_match_parent(TextAlign::MatchParent, TextAlign::Start, Direction::Rtl),
        TextAlign::Right
    );
}

#[test]
fn match_parent_resolves_end_against_ltr_parent_to_right() {
    assert_eq!(
        resolve_text_align_match_parent(TextAlign::MatchParent, TextAlign::End, Direction::Ltr),
        TextAlign::Right
    );
}

#[test]
fn match_parent_resolves_end_against_rtl_parent_to_left() {
    assert_eq!(
        resolve_text_align_match_parent(TextAlign::MatchParent, TextAlign::End, Direction::Rtl),
        TextAlign::Left
    );
}

#[test]
fn match_parent_copies_non_start_end_parent_value_verbatim() {
    // "behaves the same as inherit" for the non-start/end half — direction
    // plays no role.
    for parent in [
        TextAlign::Left,
        TextAlign::Right,
        TextAlign::Center,
        TextAlign::Justify,
        TextAlign::JustifyAll,
    ] {
        assert_eq!(
            resolve_text_align_match_parent(TextAlign::MatchParent, parent, Direction::Ltr),
            parent
        );
        assert_eq!(
            resolve_text_align_match_parent(TextAlign::MatchParent, parent, Direction::Rtl),
            parent
        );
    }
}

#[test]
fn non_match_parent_specified_values_pass_through_unchanged() {
    // Every other keyword's computed value is "as specified" — the parent
    // args must be ignored entirely.
    for specified in [
        TextAlign::Start,
        TextAlign::End,
        TextAlign::Left,
        TextAlign::Right,
        TextAlign::Center,
        TextAlign::Justify,
        TextAlign::Inherit,
        TextAlign::InternalCenter,
        TextAlign::JustifyAll,
    ] {
        assert_eq!(
            resolve_text_align_match_parent(specified, TextAlign::Center, Direction::Rtl),
            specified
        );
    }
}

#[test]
fn internal_center_centers_only_initial_start_parent() {
    assert_eq!(
        resolve_text_align_internal_center(TextAlign::InternalCenter, TextAlign::Start),
        TextAlign::Center
    );
    assert_eq!(
        resolve_text_align_internal_center(TextAlign::Inherit, TextAlign::Start),
        TextAlign::Start
    );
    for parent in [
        TextAlign::End,
        TextAlign::Left,
        TextAlign::Right,
        TextAlign::Center,
        TextAlign::Justify,
        TextAlign::JustifyAll,
    ] {
        assert_eq!(
            resolve_text_align_internal_center(TextAlign::InternalCenter, parent),
            parent
        );
    }
}

#[test]
fn internal_center_resolver_does_not_change_author_values() {
    assert_eq!(
        resolve_text_align_internal_center(TextAlign::Inherit, TextAlign::End),
        TextAlign::End
    );
    for specified in [
        TextAlign::Start,
        TextAlign::End,
        TextAlign::Left,
        TextAlign::Right,
        TextAlign::Center,
        TextAlign::Justify,
        TextAlign::MatchParent,
        TextAlign::JustifyAll,
    ] {
        assert_eq!(
            resolve_text_align_internal_center(specified, TextAlign::End),
            specified
        );
    }
}

// ── text-shadow (CSS Text Decoration Module Level 3 §4) ─────────

/// [`content_items`] と同じ shape の extraction helper —
/// `PropertyValue::TextShadow(Arc<Vec<..>>)` の payload を clone して返す。
fn text_shadow_items(source: &str) -> Vec<TextShadowItem> {
    match parse(source, "text-shadow") {
        Some(PropertyValue::TextShadow(v)) => (*v).clone(),
        // cov:ignore: this panic branch is unreached as long as every
        // caller passes a genuinely valid text-shadow declaration —
        // llvm-cov marks the panic-message literal "uncovered" the
        // same way it does for any other panic-only branch (same
        // false-positive class as the let-else panic branch above).
        other => panic!("expected PropertyValue::TextShadow, got {other:?}"),
    }
}

#[test]
fn text_shadow_parse_none_is_empty_list() {
    assert_eq!(text_shadow_items("none"), Vec::new());
}

#[test]
fn text_shadow_parse_preserves_mixed_em_px_calc() {
    let calc = crate::property::TextShadowLength::Calc { px: 10.0, em: 0.5 };
    assert_eq!(
        text_shadow_items("calc(0.5em + 10px) calc(0.5em + 10px) calc(0.5em + 10px)"),
        vec![TextShadowItem {
            offset_x: calc,
            offset_y: calc,
            blur_radius: calc,
            color: TextShadowColor::CurrentColor,
        }]
    );
}

#[test]
fn text_shadow_parse_rejects_percentage_calc_terms() {
    assert_eq!(parse_entire("calc(10% - 10%) 1px", "text-shadow"), None);
    assert_eq!(parse_entire("1px 1px calc(10% + 1px)", "text-shadow"), None);
}

#[test]
fn text_shadow_parse_single_offset_only_defaults_blur_and_color() {
    // `<color>` / blur-radius 省略 — `TextShadowItem` doc の「各成分の
    // 初期値埋め」節。
    assert_eq!(
        text_shadow_items("1px 2px"),
        vec![TextShadowItem {
            offset_x: crate::property::TextShadowLength::Length(Length::Px(1.0)),
            offset_y: crate::property::TextShadowLength::Length(Length::Px(2.0)),
            blur_radius: crate::property::TextShadowLength::Length(Length::Px(0.0)),
            color: TextShadowColor::CurrentColor,
        }]
    );
}

#[test]
fn text_shadow_parse_color_after_lengths() {
    assert_eq!(
        text_shadow_items("1px 2px 3px red"),
        vec![TextShadowItem {
            offset_x: crate::property::TextShadowLength::Length(Length::Px(1.0)),
            offset_y: crate::property::TextShadowLength::Length(Length::Px(2.0)),
            blur_radius: crate::property::TextShadowLength::Length(Length::Px(3.0)),
            color: TextShadowColor::Resolved(CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255,
            }),
        }]
    );
}

/// `<color>? && <length>{2,3}` の `&&` combinator — 順序は自由
/// ([`parse_text_shadow_item`] doc の「`&&` grammar semantics」節)。
/// color-before は color-after (直上 test) と同じ結果になる。
#[test]
fn text_shadow_parse_color_before_lengths_matches_color_after() {
    assert_eq!(
        text_shadow_items("red 1px 2px 3px"),
        text_shadow_items("1px 2px 3px red"),
    );
}

#[test]
fn text_shadow_parse_explicit_currentcolor_keyword() {
    assert_eq!(
        text_shadow_items("currentcolor 1px 1px"),
        vec![TextShadowItem {
            offset_x: crate::property::TextShadowLength::Length(Length::Px(1.0)),
            offset_y: crate::property::TextShadowLength::Length(Length::Px(1.0)),
            blur_radius: crate::property::TextShadowLength::Length(Length::Px(0.0)),
            color: TextShadowColor::CurrentColor,
        }]
    );
}

#[test]
fn text_shadow_parse_negative_offsets_allowed() {
    // offset-x / offset-y に non-negative 制約は無い (`TextShadowItem`
    // doc の「Non-negative blur-radius」節 — blur のみ制約対象)。
    assert_eq!(
        text_shadow_items("-1px -2px"),
        vec![TextShadowItem {
            offset_x: crate::property::TextShadowLength::Length(Length::Px(-1.0)),
            offset_y: crate::property::TextShadowLength::Length(Length::Px(-2.0)),
            blur_radius: crate::property::TextShadowLength::Length(Length::Px(0.0)),
            color: TextShadowColor::CurrentColor,
        }]
    );
}

#[test]
fn text_shadow_parse_rejects_percentage() {
    // `<length>` のみ、percentage 不可 (CSS Text Decoration Module Level
    // 3 §4 "Percentages: N/A", `TextShadowItem` doc 参照)。
    // `parse_length_value(input, false)` (`allow_percentage=false`) が
    // parse-time で拒否する — sibling precedent
    // `page_size_percentage_rejected` と同じ shape。
    assert_eq!(parse("50% 50%", "text-shadow"), None);
}

#[test]
fn text_shadow_parse_rejects_negative_blur_radius() {
    // blur-radius (3rd length) は non-negative — CSS Backgrounds 3 §6.1
    // "Drop Shadows: the box-shadow property"
    // "Negative values are invalid" (box-shadow / text-shadow 共通の
    // `<shadow>` syntax)。負の 3rd length は blur slot にマッチせず
    // unconsumed のまま残り、`parse_comma_separated` の
    // `parse_until_before` → `parse_entirely` が leftover を検知して
    // declaration ごと drop する (`parse_text_shadow` doc 参照)。
    assert_eq!(parse("1px 1px -3px", "text-shadow"), None);
}

#[test]
fn text_shadow_zero_mantissa_huge_exponent_offset_resolves_to_zero_but_preserves_infinity() {
    // `0e999` is a zero-mantissa, huge-exponent literal that
    // cssparser's tokenizer collapses to `NaN` internally (module doc's
    // "Numeric-token NaN stabilization" section), but
    // `next_numeric_stable` (which `parse_length_value` — reached via
    // `parse_shadow_length_reject_nan` → `parse_length_allow_negative`
    // — acquires its token through) corrects that before this
    // property's `!is_nan()` guard ever runs. So `0e999px` resolves to
    // the spec-correct `Length::Px(0.0)` offset, and the whole
    // declaration parses successfully — it is no longer dropped.
    assert_eq!(
        text_shadow_items("0e999px 1px"),
        vec![TextShadowItem {
            offset_x: crate::property::TextShadowLength::Length(Length::Px(0.0)),
            offset_y: crate::property::TextShadowLength::Length(Length::Px(1.0)),
            blur_radius: crate::property::TextShadowLength::Length(Length::Px(0.0)),
            color: TextShadowColor::CurrentColor,
        }]
    );
    assert_eq!(
        text_shadow_items("1px 0e999px"),
        vec![TextShadowItem {
            offset_x: crate::property::TextShadowLength::Length(Length::Px(1.0)),
            offset_y: crate::property::TextShadowLength::Length(Length::Px(0.0)),
            blur_radius: crate::property::TextShadowLength::Length(Length::Px(0.0)),
            color: TextShadowColor::CurrentColor,
        }]
    );

    // `+Inf`/`-Inf` are a *different* hazard class — ordinary `<number>`
    // magnitude overflow, a legitimate (if extreme) `<length>` per CSS
    // Values 4 §5 — and must NOT be rejected here either. Both signs
    // are checked (not just `+Inf`) because an earlier iteration of the
    // sibling `opacity` guard used `is_finite()` and wrongly dropped
    // the negative-overflow case too (`opacity_zero_mantissa_huge_exponent_resolves_to_zero_but_preserves_infinity`
    // doc参照).
    assert_eq!(
        text_shadow_items("1e40px 1px"),
        vec![TextShadowItem {
            offset_x: crate::property::TextShadowLength::Length(Length::Px(f32::INFINITY)),
            offset_y: crate::property::TextShadowLength::Length(Length::Px(1.0)),
            blur_radius: crate::property::TextShadowLength::Length(Length::Px(0.0)),
            color: TextShadowColor::CurrentColor,
        }]
    );
    assert_eq!(
        text_shadow_items("-1e40px 1px"),
        vec![TextShadowItem {
            offset_x: crate::property::TextShadowLength::Length(Length::Px(f32::NEG_INFINITY)),
            offset_y: crate::property::TextShadowLength::Length(Length::Px(1.0)),
            blur_radius: crate::property::TextShadowLength::Length(Length::Px(0.0)),
            color: TextShadowColor::CurrentColor,
        }]
    );
}

#[test]
fn text_shadow_blur_radius_zero_mantissa_huge_exponent_resolves_to_zero() {
    // Same recovery as the offset case above, for blur-radius (3rd
    // slot) — `0e999px` resolves to `Length::Px(0.0)`, which
    // `parse_non_negative_length`'s `>= 0.0` check accepts normally
    // (it is no longer `NaN`, so there is nothing for that check to
    // incidentally reject).
    assert_eq!(
        text_shadow_items("1px 1px 0e999px"),
        vec![TextShadowItem {
            offset_x: crate::property::TextShadowLength::Length(Length::Px(1.0)),
            offset_y: crate::property::TextShadowLength::Length(Length::Px(1.0)),
            blur_radius: crate::property::TextShadowLength::Length(Length::Px(0.0)),
            color: TextShadowColor::CurrentColor,
        }]
    );
}

#[test]
fn text_shadow_parse_multiple_comma_separated() {
    assert_eq!(
        text_shadow_items("1px 1px red, 2px 2px 4px blue"),
        vec![
            TextShadowItem {
                offset_x: crate::property::TextShadowLength::Length(Length::Px(1.0)),
                offset_y: crate::property::TextShadowLength::Length(Length::Px(1.0)),
                blur_radius: crate::property::TextShadowLength::Length(Length::Px(0.0)),
                color: TextShadowColor::Resolved(CssColor {
                    r: 255,
                    g: 0,
                    b: 0,
                    a: 255,
                }),
            },
            TextShadowItem {
                offset_x: crate::property::TextShadowLength::Length(Length::Px(2.0)),
                offset_y: crate::property::TextShadowLength::Length(Length::Px(2.0)),
                blur_radius: crate::property::TextShadowLength::Length(Length::Px(4.0)),
                color: TextShadowColor::Resolved(CssColor {
                    r: 0,
                    g: 0,
                    b: 255,
                    a: 255,
                }),
            },
        ]
    );
}

#[test]
fn text_shadow_parse_rejects_empty_value() {
    // length run は必須 — `<color>` 単体 (`text-shadow: red`) は
    // grammar 上 invalid。
    assert_eq!(parse("red", "text-shadow"), None);
}

#[test]
fn text_shadow_key_maps_to_text_shadow_property_key() {
    assert_eq!(
        PropertyValue::TextShadow(Arc::new(Vec::new())).key(),
        PropertyKey::TextShadow
    );
}

// ── font shorthand (CSS Fonts 4 §2.1) ──

fn expect_font(value: Option<PropertyValue>) -> FontShorthand {
    match value {
        Some(PropertyValue::Font(shorthand)) => shorthand,
        // cov:ignore: this branch only executes when a caller's
        // `parse(..., "font")` unexpectedly fails to parse or parses to
        // the wrong variant; every call site in this test module passes
        // valid `font` shorthand input, so the panic never fires while
        // the tests pass.
        other => panic!("expected PropertyValue::Font, got {other:?}"),
    }
}

#[test]
fn font_shorthand_minimal_size_and_family_fill_the_rest_with_initial_values() {
    assert_eq!(
        expect_font(parse_entire("16px serif", "font")),
        FontShorthand {
            style: FontStyle::Normal,
            variant: FontVariantCaps::Normal,
            weight: FontWeightValue::Absolute(400.0),
            size: FontShorthandSize::Absolute(Length::Px(16.0)),
            line_height: LineHeight::Normal,
            family: Arc::new(vec![crate::property::FontFamilyName::generic("serif")]),
        }
    );
}

#[test]
fn font_shorthand_full_preface_with_slash_line_height_and_family_list() {
    assert_eq!(
        expect_font(parse_entire(
            "italic small-caps bold 16px/1.5 \"Times New Roman\", serif",
            "font"
        )),
        FontShorthand {
            style: FontStyle::Italic,
            variant: FontVariantCaps::SmallCaps,
            weight: FontWeightValue::Absolute(700.0),
            size: FontShorthandSize::Absolute(Length::Px(16.0)),
            line_height: LineHeight::Number(1.5),
            family: Arc::new(vec![
                crate::property::FontFamilyName::named("Times New Roman"),
                crate::property::FontFamilyName::generic("serif"),
            ]),
        }
    );
}

#[test]
fn font_shorthand_preface_accepts_any_order() {
    let forward = expect_font(parse_entire("italic bold 16px serif", "font"));
    let backward = expect_font(parse_entire("bold italic 16px serif", "font"));
    assert_eq!(forward, backward);
    assert_eq!(forward.style, FontStyle::Italic);
    assert_eq!(forward.weight, FontWeightValue::Absolute(700.0));
}

#[test]
fn font_shorthand_numeric_weight_and_absolute_unit_size() {
    let shorthand = expect_font(parse_entire("600 14pt Georgia", "font"));
    assert_eq!(shorthand.weight, FontWeightValue::Absolute(600.0));
    assert_eq!(
        shorthand.size,
        FontShorthandSize::Absolute(Length::Pt(14.0))
    );
}

#[test]
fn font_shorthand_relative_size_keywords() {
    assert_eq!(
        expect_font(parse_entire("larger serif", "font")).size,
        FontShorthandSize::Relative(RelativeFontSize::Larger)
    );
    assert_eq!(
        expect_font(parse_entire("italic smaller serif", "font")).size,
        FontShorthandSize::Relative(RelativeFontSize::Smaller)
    );
}

#[test]
fn font_shorthand_absolute_size_keyword() {
    assert_eq!(
        expect_font(parse_entire("x-large serif", "font")).size,
        FontShorthandSize::Absolute(Length::Px(24.0))
    );
}

#[test]
fn font_shorthand_slash_line_height_length_and_percentage() {
    assert_eq!(
        expect_font(parse_entire("16px/24px serif", "font")).line_height,
        LineHeight::Length(Length::Px(24.0))
    );
    assert_eq!(
        expect_font(parse_entire("16px/150% serif", "font")).line_height,
        LineHeight::Length(Length::Percent(150.0))
    );
}

#[test]
fn font_shorthand_triple_normal_fills_three_preface_slots() {
    let shorthand = expect_font(parse_entire("normal normal normal 16px serif", "font"));
    assert_eq!(shorthand.style, FontStyle::Normal);
    assert_eq!(shorthand.variant, FontVariantCaps::Normal);
    assert_eq!(shorthand.weight, FontWeightValue::Absolute(400.0));
}

#[test]
fn font_shorthand_stretch_normal_consumed_after_full_preface() {
    // `normal` after style + variant + weight can only be the
    // `font-stretch` slot (`FontShorthand` doc's Scope carving section) —
    // consumed and dropped, longhands keep their fills.
    let shorthand = expect_font(parse_entire(
        "italic small-caps bold normal 16px serif",
        "font",
    ));
    assert_eq!(shorthand.style, FontStyle::Italic);
    assert_eq!(shorthand.variant, FontVariantCaps::SmallCaps);
    assert_eq!(shorthand.weight, FontWeightValue::Absolute(700.0));
}

#[test]
fn font_shorthand_oblique_style() {
    assert_eq!(
        expect_font(parse_entire("oblique 16px serif", "font")).style,
        FontStyle::Oblique
    );
}

#[test]
fn font_shorthand_rejects_system_font_keywords() {
    for keyword in [
        "caption",
        "icon",
        "menu",
        "message-box",
        "small-caption",
        "status-bar",
    ] {
        assert_eq!(parse_entire(keyword, "font"), None, "{keyword}");
    }
}

#[test]
fn font_shorthand_rejects_missing_size_or_family() {
    // preface only, no size/family
    assert_eq!(parse_entire("italic", "font"), None);
    // size without family
    assert_eq!(parse_entire("italic 16px", "font"), None);
    assert_eq!(parse_entire("16px", "font"), None);
    // family without size
    assert_eq!(parse_entire("serif", "font"), None);
    assert_eq!(parse_entire("italic serif", "font"), None);
    // empty
    assert_eq!(parse_entire("", "font"), None);
}

#[test]
fn font_shorthand_rejects_non_normal_font_stretch() {
    // `condensed` matches no preface slot, so the `font-size` parse runs
    // on it and fails — the whole declaration is dropped rather than
    // silently ignoring the stretch (`FontShorthand` doc's Scope
    // carving section).
    assert_eq!(parse_entire("condensed 16px serif", "font"), None);
}

#[test]
fn font_shorthand_rejects_non_css2_font_variant() {
    // `unicase` is a valid `font-variant-caps` keyword but not part of
    // CSS2's `font-variant-css2` (`normal` / `small-caps`) subset the
    // shorthand accepts — same drop shape as the stretch case above.
    assert_eq!(parse_entire("italic unicase 16px serif", "font"), None);
}

#[test]
fn font_shorthand_rejects_repeated_preface_component() {
    // `||` semantics: each preface component at most once. The 2nd
    // `italic` matches no remaining slot, so the `font-size` parse runs
    // on it and fails.
    assert_eq!(parse_entire("italic italic 16px serif", "font"), None);
}

#[test]
fn font_shorthand_rejects_slash_with_missing_or_invalid_line_height() {
    assert_eq!(parse_entire("16px/ serif", "font"), None);
    assert_eq!(parse_entire("16px/bogus serif", "font"), None);
}

#[test]
fn font_shorthand_rejects_unknown_keyword() {
    assert_eq!(parse_entire("bogus 16px serif", "font"), None);
}

#[test]
fn font_shorthand_key_maps_to_font_property_key() {
    let v = PropertyValue::Font(FontShorthand {
        style: FontStyle::Normal,
        variant: FontVariantCaps::Normal,
        weight: FontWeightValue::Absolute(400.0),
        size: FontShorthandSize::Absolute(Length::Px(16.0)),
        line_height: LineHeight::Normal,
        family: Arc::new(vec![crate::property::FontFamilyName::generic("serif")]),
    });
    assert_eq!(v.key(), PropertyKey::Font);
}

#[test]
fn hyphenate_character_parses_auto_case_insensitively() {
    assert_eq!(
        parse_entire("AUTO", "hyphenate-character"),
        Some(PropertyValue::HyphenateCharacter(HyphenateCharacter::Auto)),
    );
}

#[test]
fn hyphenate_character_preserves_nonempty_and_empty_css_strings() {
    assert_eq!(
        parse_entire("\"=\"", "hyphenate-character"),
        Some(PropertyValue::HyphenateCharacter(
            HyphenateCharacter::String("=".into())
        )),
    );
    assert_eq!(
        parse_entire("\"\"", "hyphenate-character"),
        Some(PropertyValue::HyphenateCharacter(
            HyphenateCharacter::String("".into())
        )),
    );
}

#[test]
fn hyphenate_character_decodes_escaped_unicode_string() {
    assert_eq!(
        parse_entire(r#""\1400""#, "hyphenate-character"),
        Some(PropertyValue::HyphenateCharacter(
            HyphenateCharacter::String("᐀".into())
        )),
    );
}

#[test]
fn hyphenate_character_rejects_unknown_ident_and_non_string_values() {
    for invalid in ["none", "manual", "unknown", "16px", "inherit"] {
        assert_eq!(
            parse_entire(invalid, "hyphenate-character"),
            None,
            "{invalid}"
        );
    }
}

#[test]
fn hyphenate_character_value_maps_to_its_property_key() {
    assert_eq!(
        parse_entire("auto", "hyphenate-character")
            .expect("valid hyphenate-character")
            .key(),
        PropertyKey::HyphenateCharacter,
    );
}

#[test]
fn font_variation_settings_preserves_specified_order_and_duplicates() {
    let cases = [
        ("normal", FontVariationSettings::Normal, "normal"),
        (
            "\"wght\" 0e999",
            FontVariationSettings::Settings(vec![FontVariationSetting {
                tag: SmolStr::new("wght"),
                value: 0.0,
            }]),
            "\"wght\" 0",
        ),
        (
            "\"wght\" 700",
            FontVariationSettings::Settings(vec![FontVariationSetting {
                tag: SmolStr::new("wght"),
                value: 700.0,
            }]),
            "\"wght\" 700",
        ),
        (
            "\"AB@D\" 0.5",
            FontVariationSettings::Settings(vec![FontVariationSetting {
                tag: SmolStr::new("AB@D"),
                value: 0.5,
            }]),
            "\"AB@D\" 0.5",
        ),
        (
            "\"wght\" 700, \"wght\" 500",
            FontVariationSettings::Settings(vec![
                FontVariationSetting {
                    tag: SmolStr::new("wght"),
                    value: 700.0,
                },
                FontVariationSetting {
                    tag: SmolStr::new("wght"),
                    value: 500.0,
                },
            ]),
            "\"wght\" 700, \"wght\" 500",
        ),
        (
            "\"wght\" 700, \"XHGT\" 0.7",
            FontVariationSettings::Settings(vec![
                FontVariationSetting {
                    tag: SmolStr::new("wght"),
                    value: 700.0,
                },
                FontVariationSetting {
                    tag: SmolStr::new("XHGT"),
                    value: 0.7,
                },
            ]),
            "\"wght\" 700, \"XHGT\" 0.7",
        ),
        (
            "\"wght\" 100, \"wdth\" 200",
            FontVariationSettings::Settings(vec![
                FontVariationSetting {
                    tag: SmolStr::new("wght"),
                    value: 100.0,
                },
                FontVariationSetting {
                    tag: SmolStr::new("wdth"),
                    value: 200.0,
                },
            ]),
            "\"wght\" 100, \"wdth\" 200",
        ),
        (
            "\"wght\" 100, \"wdth\" 200, \"wght\" 300, \"wdth\" 400",
            FontVariationSettings::Settings(vec![
                FontVariationSetting {
                    tag: SmolStr::new("wght"),
                    value: 100.0,
                },
                FontVariationSetting {
                    tag: SmolStr::new("wdth"),
                    value: 200.0,
                },
                FontVariationSetting {
                    tag: SmolStr::new("wght"),
                    value: 300.0,
                },
                FontVariationSetting {
                    tag: SmolStr::new("wdth"),
                    value: 400.0,
                },
            ]),
            "\"wght\" 100, \"wdth\" 200, \"wght\" 300, \"wdth\" 400",
        ),
    ];

    for (input, expected, serialized) in cases {
        let value = parse_entire(input, "font-variation-settings")
            .unwrap_or_else(|| panic!("expected valid font-variation-settings `{input}`"));
        assert_eq!(
            value,
            PropertyValue::FontVariationSettings(expected),
            "{input}"
        );
        assert_eq!(value.key(), PropertyKey::FontVariationSettings);
        assert_eq!(
            serialize_value(&value),
            Some(serialized.to_owned()),
            "{input}"
        );
    }

    for invalid in [
        "",
        "foo",
        "\"wgh\" 1",
        "\"wghtx\" 1",
        "\"wght\"",
        "\"wght\" 1,",
        "normal, \"wght\" 1",
        "\"wght\" 1 \"wdth\" 2",
    ] {
        assert_eq!(
            parse_entire(invalid, "font-variation-settings"),
            None,
            "{invalid}"
        );
    }
}

#[test] // cov:ignore: cfg(test)-only property tests have no lcov source record on the pinned coverage run.
// cov:ignore: cfg(test)-only property tests have no lcov source record on the pinned coverage run.
fn word_space_transform_preserves_the_five_computed_values() {
    let cases = [
        ("none", WordSpaceTransform::None, "none"),
        ("space", WordSpaceTransform::Space, "space"),
        (
            "ideographic-space",
            WordSpaceTransform::IdeographicSpace,
            "ideographic-space",
        ),
        (
            "space auto-phrase",
            WordSpaceTransform::SpaceAutoPhrase,
            "space auto-phrase",
        ),
        (
            "ideographic-space auto-phrase",
            WordSpaceTransform::IdeographicSpaceAutoPhrase,
            "ideographic-space auto-phrase",
        ),
        (
            "AUTO-PHRASE SPACE",
            WordSpaceTransform::SpaceAutoPhrase,
            "space auto-phrase",
        ),
    ];

    for (authored, expected, serialized) in cases {
        let parsed = parse_entire(authored, "word-space-transform")
            .unwrap_or_else(|| panic!("expected valid word-space-transform `{authored}`"));
        assert_eq!(
            parsed,
            PropertyValue::WordSpaceTransform(expected),
            "{authored}"
        );
        assert_eq!(
            serialize_value(&parsed),
            Some(serialized.to_owned()),
            "{authored}"
        );
        assert_eq!(parsed.key(), PropertyKey::WordSpaceTransform);
    }

    for invalid in [
        "",
        "auto-phrase",
        "space ideographic-space",
        "none space",
        "space auto-phrase auto-phrase",
        "space unknown",
    ] {
        assert_eq!(
            parse_entire(invalid, "word-space-transform"),
            None,
            "{invalid} should be rejected",
        );
    }
}
