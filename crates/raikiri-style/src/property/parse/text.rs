//! Font and text property parsers, including `text-decoration`, `text-shadow`
//! and the `font` shorthand.

use std::collections::BTreeMap;
use std::sync::Arc;

use cssparser::{ParseError, Parser, ParserInput, Token};
use smol_str::SmolStr;

use crate::Atom;
use crate::property::types::*;

use super::color::*;
use super::common::*;

/// `font-family: <family-name>#` を parse する。
///
/// comma-separated な family-name の list。各 family-name は quoted string
/// (`"Times New Roman"`) か、unquoted identifier の連続 (`Times New Roman` =
/// 3 ident が空白区切りで 1 family、CSS4 で有効) のいずれか。
///
/// 末尾で comma が続かなければ loop を止め、残り input (`!important` 等) は
/// 手を付けずに downstream (caller の `parse_important` / `expect_exhausted`)
/// に委ねる — `!` を garbage として拒否しないための Finding 3 対応。
pub(super) fn parse_font_family(input: &mut Parser<'_, '_>) -> Option<Vec<Atom>> {
    let mut families = Vec::new();
    loop {
        // Try quoted string first (e.g. "Times New Roman")
        let family = if let Ok(s) = input.try_parse(|i| i.expect_string().cloned()) {
            Atom::from(s.as_ref())
        } else if let Ok(first) = input.try_parse(|i| i.expect_ident().cloned()) {
            // Unquoted ident sequence: `Times New Roman` = 3 idents joined by space
            let mut buf = first.as_ref().to_string();
            while let Ok(next) = input.try_parse(|i| i.expect_ident().cloned()) {
                buf.push(' ');
                buf.push_str(next.as_ref());
            }
            Atom::from(buf.as_str())
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
        .try_parse(|i| parse_text_indent_calc(i).ok_or_else(|| i.new_custom_error::<(), ()>(())))
        .ok()
        .or_else(|| parse_length_value(input, true).map(TextIndentLength::Length))
}

fn parse_text_indent_calc(input: &mut Parser<'_, '_>) -> Option<TextIndentLength> {
    input
        .try_parse(|i| -> Result<TextIndentLength, ParseError<'_, ()>> {
            match i.next()?.clone() {
                Token::Function(name) if name.eq_ignore_ascii_case("calc") => {}
                token => return Err(i.new_unexpected_token_error(token)),
            }
            i.parse_nested_block(parse_text_indent_calc_terms)
        })
        .ok()
}

fn parse_text_indent_calc_terms<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<TextIndentLength, ParseError<'i, ()>> {
    let value = parse_text_indent_calc_sum(input)?;
    if value.em == 0.0 {
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
) -> Result<LengthPercentageCalc, ParseError<'i, ()>> {
    let mut value = parse_text_indent_calc_term(input)?;
    while !input.is_exhausted() {
        let sign = match input.next()?.clone() {
            Token::Delim('+') => 1.0,
            Token::Delim('-') => -1.0,
            token => return Err(input.new_unexpected_token_error(token)),
        };
        let term = parse_text_indent_calc_term(input)?;
        value.percent += sign * term.percent;
        value.px += sign * term.px;
        value.em += sign * term.em;
        if !value.percent.is_finite() || !value.px.is_finite() || !value.em.is_finite() {
            return Err(input.new_custom_error(()));
        }
    }
    Ok(value)
}

fn parse_text_indent_calc_term<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<LengthPercentageCalc, ParseError<'i, ()>> {
    let start = input.state();
    if matches!(input.next()?.clone(), Token::ParenthesisBlock) {
        return input.parse_nested_block(parse_text_indent_calc_sum);
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
        },
        Length::Pt(v) => LengthPercentageCalc {
            percent: 0.0,
            px: v * (96.0 / 72.0),
            em: 0.0,
        },
        Length::Cm(v) => LengthPercentageCalc {
            percent: 0.0,
            px: v * (96.0 / 2.54),
            em: 0.0,
        },
        Length::Mm(v) => LengthPercentageCalc {
            percent: 0.0,
            px: v * (96.0 / 25.4),
            em: 0.0,
        },
        Length::Q(v) => LengthPercentageCalc {
            percent: 0.0,
            px: v * (96.0 / 101.6),
            em: 0.0,
        },
        Length::In(v) => LengthPercentageCalc {
            percent: 0.0,
            px: v * 96.0,
            em: 0.0,
        },
        Length::Pc(v) => LengthPercentageCalc {
            percent: 0.0,
            px: v * 16.0,
            em: 0.0,
        },
        Length::Em(em) => LengthPercentageCalc {
            percent: 0.0,
            px: 0.0,
            em,
        },
        Length::Percent(percent) => LengthPercentageCalc {
            percent,
            px: 0.0,
            em: 0.0,
        },
        _ => return Err(input.new_custom_error(())),
    };
    if value.percent.is_finite() && value.px.is_finite() && value.em.is_finite() {
        Ok(value)
    } else {
        Err(input.new_custom_error(()))
    }
}

/// `font-size: <absolute-size> | <relative-size> | <length-percentage [0,∞]> |
/// math` を parse する。
///
/// Grammar (CSS Fonts 4 §2.5 "Font size: the font-size property"
/// <https://www.w3.org/TR/css-fonts-4/#font-size-prop>):
/// `<absolute-size> | <relative-size> | <length-percentage [0,∞]> | math`。
///
/// - `<absolute-size>` (`xx-small` … `xxx-large`、`medium`) — [`parse_font_size_keyword`]
///   が §2.5.1 の scaling-factor table を `medium` = 16px 基準で解決し、
///   [`PropertyValue::FontSize`] (`Length::Px`) を返す。
/// - `<relative-size>` (`larger` / `smaller`) — 継承先依存のため
///   [`PropertyValue::FontSizeRelative`] を返し、解決は
///   [`crate::cascade::apply_value`] / [`crate::cascade::resolve_against_inherited`]
///   に委ねる (詳細は同 variant の doc)。
/// - `<length-percentage [0,∞]>` — 本関数の後半、[`parse_length_value`] 経由。
/// - `math` — 未実装 (MathML scaling algorithm が丸ごと未対応) として
///   `None` に落とす。
///
/// # ident 分岐を先に `try_parse` する理由
///
/// `<absolute-size>` / `<relative-size>` / `math` はいずれも単一 ident token。
/// [`parse_margin_side`](super::box_model::parse_margin_side) の `auto` 分岐と同じ pattern — [`parse_length_value`]
/// は内部で `input.next()` を unconditional に消費するため、ident 分岐は
/// checkpoint 経由の rewind (`try_parse`) で先に試す必要がある。
///
/// Relative and percentage lengths are resolved by the cascade using the
/// parent and root font-size bases required by CSS Values 4.
///
/// # Non-negative constraint
///
/// grammar の `[0,∞]` を parse-time enforce する。[`parse_padding_side`](super::box_model::parse_padding_side) /
/// [`parse_width`](super::box_model::parse_width) と同じ [`Length::payload`] 経由の全 [`Length`] variant check
/// — `-5px` だけでなく `-50%` / `-1em` も drop する。`<absolute-size>` /
/// `<relative-size>` は grammar 上そもそも符号を持たないので本 constraint の
/// 対象外 (ident 分岐は `parse_length_value` に達する前に return する)。
///
/// # `lh` / `rlh` は受理し、親基準で解決する
///
/// [`Length::Lh`] doc の「自己参照」節: CSS Values 4 §6.1.1 は `lh`/`rlh` が
/// `line-height` **または font-\* property** の値として、それが指す要素自身に
/// 使われたときは親 (または「親が無ければ initial values」) の line-height /
/// font metrics を基準にする、と規定する。`font-size` はまさにその
/// font-\* property であり、grammar 上 `lh`/`rlh` を排除する根拠は無い
/// (CSS Fonts 4 の `font-size` grammar `<absolute-size> | <relative-size> |
/// <length-percentage [0,∞]>` の `<length-percentage>` は `<length>` を含み、
/// CSS Values 4 §6.1.1 の `<length>` production は `lh`/`rlh` を除外しない)。
///
/// 当初は、この解決 (「親の computed line-height」を
/// font-size 解決の基準として渡す) が `line-height`
/// (`finalize`/`finalize_as_root` が既に持つ `parent: &ComputedValues` を
/// そのまま使える) より高コストに見えたため drop していたが、実際に実装した
/// ところコストは局所的だった — [`crate::resolve::resolve_font_size`] の
/// `self_reference_basis` 引数、および [`crate::specified::SpecifiedValues::finalize`]
/// 内の 2, 3 行の並べ替えで足りる (`parent` は本関数の呼び出しに入る前に
/// tree walk で既に確定済みのため、cross-node な phase 順序の変更は不要 —
/// [`mod@crate::resolve`] module doc の「想定される 4 段階」節参照)。
pub(crate) fn parse_font_size(input: &mut Parser<'_, '_>) -> Option<PropertyValue> {
    if let Ok(ident) = input.try_parse(|i| i.expect_ident().cloned()) {
        return parse_font_size_keyword(&ident);
    }
    let length = parse_length_value(input, true)?;
    (length.payload() >= 0.0).then_some(PropertyValue::FontSize(length))
}

/// `<absolute-size>` / `<relative-size>` / `math` の ident 部分を parse する
/// ([`parse_font_size`] の helper)。
///
/// # `<absolute-size>` scaling-factor table
///
/// CSS Fonts 4 §2.5.1 "Absolute Size Keyword Mapping Table"
/// <https://www.w3.org/TR/css-fonts-4/#absolute-size-mapping> の表をそのまま
/// 写す (`resolve_relative_weight` の "算術式で書いてはいけない"
/// 方針と同じ理由 — 分数のまま持つことで丸め誤差の議論を spec 引用だけで
/// 閉じられる)。`medium` は raikiri の固定基準
/// ([`crate::computed::INITIAL_FONT_SIZE_PX`] = 16px、
/// [`crate::specified::SpecifiedValues::initial`] doc 参照) を再利用する:
///
/// | keyword | xx-small | x-small | small | medium | large | x-large | xx-large | xxx-large |
/// |---|---|---|---|---|---|---|---|---|
/// | factor | 3/5 | 3/4 | 8/9 | 1 | 6/5 | 3/2 | 2/1 | 3/1 |
///
/// 同 §の "an UA applying these guidelines should nevertheless avoid creating
/// font sizes of less than 9 device pixels per EM unit" は "should" (RFC 2119
/// 弱勧告)。本 table の最小値は `xx-small` = `16 * 3/5 = 9.6px` で、9px の
/// 下限を上回るため clamp は不要 (実装しない理由は「未対応」ではなく
/// 「`medium` = 16px 基準ではこの guideline を最初から満たす」こと)。
///
/// # `<relative-size>`
///
/// [`RelativeFontSize`] doc 参照。
///
/// # `math`
///
/// 未実装 (spec-valid だが対応外)。
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
        // `math` はここに落ちる (spec-valid だが未対応)。
        // 未知 ident も同じく drop。
        _ => return None,
    };
    Some(PropertyValue::FontSize(Length::Px(px)))
}

/// `line-height: normal | <number> | <length-percentage>` を parse する。
///
/// Grammar: CSS Inline 3 §5.1 "Line Spacing: the line-height property"
/// (<https://www.w3.org/TR/css-inline-3/#line-height-property>) — value
/// alternative は 3 branch:
///
/// 1. `normal` keyword → [`LineHeight::Normal`]
/// 2. `<number [0,∞]>` bare number (Token::Number、unit なし) → [`LineHeight::Number`]
/// 3. `<length-percentage [0,∞]>` → [`LineHeight::Length`] with reused Length variant
///
/// # Number vs Length grammar distinction
///
/// spec は `<number>` と `<length-percentage>` を別 alternative として持つため
/// 1 token レベルで区別が要る (Token::Number = unitless / Token::Dimension =
/// unit-bearing / Token::Percentage)。unitless `1.5` と dimensioned `1.5em` を
/// 別 variant に mapping することで、下流 (paint) が unitless number の
/// spec special behavior "specified value を child が inherit する"
/// (§5.1 "When a child element inherits a computed value...") と、length の
/// 通常 resolve context を区別できる。
///
/// # Ordering
///
/// `normal` (`try_parse` + `expect_ident_matching`) → bare number
/// (`try_parse(|i| expect_number_stable(i))` — Dimension/Percentage に対しては rewind
/// して失敗) → [`parse_length_value`] (`allow_percentage = true`)。この順で
/// `1.5` は Number branch、`1.5em` / `1.5px` / `150%` は Length branch に確定分岐。
///
/// # Non-negative
///
/// spec `[0,∞]` により全 branch で negative reject:
/// - Number branch: `n >= 0.0` guard、負なら `None` = declaration drop
/// - Length branch: 全 payload の inner f32 に `>= 0.0` guard、負なら drop
///
/// spec-invalid → drop: spec grammar が range を parse-time
/// で制約するため、reject 自体が spec 準拠。
///
/// # Non-goals
///
/// - **(b) 非対応**: CSS-wide keyword は未実装 (将来対応)、silent drop
///   (5 keyword の一覧・理由は [`PropertyValue`] doc の「CSS-wide keyword」節
///   が canonical)。
/// - **(b) 非対応**: `calc()` / `var()` は未実装 (css-variables-and-math)、
///   silent drop
/// - **(a) spec-invalid → drop**: `<number>` / `<length-percentage>` の負値、
///   `auto` / `medium` 等 spec-invalid keyword は spec grammar 違反、drop
pub(super) fn parse_line_height(input: &mut Parser<'_, '_>) -> Option<LineHeight> {
    // 1. `normal` keyword — spec initial value。
    if input
        .try_parse(|i| i.expect_ident_matching("normal"))
        .is_ok()
    {
        return Some(LineHeight::Normal);
    }
    // 2. bare `<number [0,∞]>` — Token::Number (unit なし)。
    //    Dimension (`1.5em`) / Percentage (`150%`) に対しては `expect_number` が
    //    Err を返し `try_parse` が rewind するため、Length branch へフォールスルー。
    //    Number token を commit した後は必ずここで確定させる (accept か drop):
    //    `try_parse` は `Ok` の path で cursor を戻さないため、外側 `&& n >= 0.0`
    //    で reject すると consumed cursor のまま Length branch に落ち、
    //    `line-height: -0.5 20px` が `20px` として silently accept される
    //    (spec-invalid CSS を通す correctness bug)。
    if let Ok(n) = input.try_parse(|i| expect_number_stable(i)) {
        // spec `<number [0,∞]>` 違反 → declaration drop (Length branch へ落とさない)。
        return (n >= 0.0).then_some(LineHeight::Number(n));
    }
    // 3. `<length-percentage [0,∞]>` — helper で全 unit + `%` を受理、
    //    negative は post-filter で drop (helper 自体は sign check しない仕様、
    //    parse_length_value doc "Sign / range" 参照)。
    let l = parse_length_value(input, true)?;
    // spec `[0,∞]`: 負値は grammar 違反 → declaration drop。
    (l.payload() >= 0.0).then_some(LineHeight::Length(l))
}

/// `tab-size: <number [0,∞]> | <length [0,∞]>` を parse する (CSS Text
/// Module Level 3 §4.2 "Tab Character Size: the tab-size property"
/// <https://www.w3.org/TR/css-text-3/#tab-size-property>)。
///
/// # Ordering
///
/// [`parse_line_height`] と同じ 2-branch shape (bare `<number>` を先に試し、
/// Dimension/Percentage には rewind して Length branch へ) から `normal`
/// branch を除いたもの — tab-size の grammar に `normal` alternative は無い。
/// `<length>` 側は `allow_percentage = false`
/// ([`parse_length_value`] — spec propdef "Percentages: N/A" が根拠、
/// [`TabSize::Length`] doc 参照)。
///
/// # Non-negative
///
/// spec `[0,∞]` (両 branch) — 全 branch で negative reject:
/// - Number branch: `n >= 0.0` guard、負なら `None` = declaration drop
/// - Length branch: payload の inner f32 に `>= 0.0` guard、負なら drop
///
/// [`parse_line_height`] doc の「Number token を commit した後は必ずここで
/// 確定させる」節と同じ懸念がここにも当てはまる — `try_parse` は `Ok` の
/// path で cursor を戻さないため、Number branch を通った後に外側で reject
/// すると `tab-size: -1 20px` が `20px` として silently accept されてしまう
/// (spec-invalid CSS を通す correctness bug)。
///
/// # Non-goals
///
/// - **(b) 非対応**: CSS-wide keyword は未実装 (将来対応)、silent drop
///   (5 keyword の一覧・理由は [`PropertyValue`] doc の「CSS-wide keyword」節
///   が canonical)。
/// - **(b) 非対応**: `calc()` / `var()` は未実装、silent drop。
/// - **(a) spec-invalid → drop**: `<number>` / `<length>` の負値、`auto` 等
///   spec-invalid keyword、percentage は spec grammar 違反、drop。
pub(super) fn parse_tab_size(input: &mut Parser<'_, '_>) -> Option<TabSize> {
    // 1. bare `<number [0,∞]>` — Token::Number (unit なし)。Dimension
    //    (`4px`) に対しては `expect_number` が Err を返し `try_parse` が
    //    rewind するため、Length branch へフォールスルー。
    if let Ok(n) = input.try_parse(|i| expect_number_stable(i)) {
        // spec `[0,∞]` 違反 → declaration drop (Length branch へ落とさない、
        // 上記 doc 節参照)。
        return (n >= 0.0).then_some(TabSize::Number(n));
    }
    // 2. `<length [0,∞]>` — percentage 非対応 (allow_percentage = false)。
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
    if let Some(parsed) = parse_text_indent_calc(input) {
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
    if let Some(parsed) = parse_text_indent_calc(input) {
        return Some(match parsed {
            TextIndentLength::Length(length) => LetterSpacingValue::Length(length),
            TextIndentLength::Calc(calc) => LetterSpacingValue::Calc(calc),
        });
    }
    parse_length_value(input, true).map(LetterSpacingValue::Length)
}

/// `font-weight: <font-weight-absolute> | bolder | lighter` を parse する。
///
/// CSS Fonts 4 §2.2 "Font weight: the font-weight property"
/// <https://www.w3.org/TR/css-fonts-4/#font-weight-prop>:
///
/// ```text
/// <font-weight-absolute> = [ normal | bold | <number [1,1000]> ]
/// ```
///
/// - `normal` = 400 / `bold` = 700 (spec §2.2 の keyword 定義)
/// - `bolder` / `lighter` は継承値依存の relative weight。parse 段では解けない
///   ため sentinel variant ([`FontWeightValue::Bolder`] /
///   [`FontWeightValue::Lighter`]) で保持し、[`crate::cascade::apply_value`]
///   が親の computed weight から解決する。
///
/// ASCII case-insensitive matching は CSS Values 3 §3.1 "Pre-defined Keywords"
/// <https://www.w3.org/TR/css-values-3/#keywords> 準拠 (sibling
/// `parse_display` / `parse_content_*` と同 convention)。
///
/// # Range (spec grammar)
///
/// spec §2.2: "Only values greater than or equal to 1, and less than or equal
/// to 1000, are valid, and all other values are invalid"。したがって `0` /
/// `1001` / `-100` の reject は **spec grammar そのもの** であり、stricter
/// policy ではない。範囲判定は **丸める前の指定値** に対して行う (spec の
/// "values" は author が書いた `<number>` を指すため、`0.6` や `1000.4` は
/// 丸めれば範囲内になるが invalid)。
///
/// Fractional values are preserved as `f32`; relative-weight resolution uses
/// the unrounded computed value.
///
pub(crate) fn parse_font_weight(input: &mut Parser<'_, '_>) -> Option<FontWeightValue> {
    match &next_numeric_stable(input).ok()? {
        // `<number [1,1000]>`。`value` field (f32) を見るので `1e3` のような
        // scientific notation や fractional もそのまま受理される (どちらも
        // CSS Values 3 の `<number>` production として spec-valid)。fraction は
        // 丸めずそのまま computed value まで運ぶ (上記 doc 参照)。
        // token 取得は `next_numeric_stable` 経由 (module doc「Numeric-token
        // NaN stabilization」節参照) — zero-mantissa/huge-exponent
        // (`0e999`) と huge-mantissa/underflowing-exponent (例:
        // `50` に等しい `5` + 400 zeros + `e-399`) の両方の cssparser
        // tokenizer artifact をここで訂正済のため、この範囲判定が実際に
        // `NaN` を見ることはもう無い。±inf (`1e400` 等、真正の magnitude
        // overflow) は依然どちらか片方の比較が false になり reject される
        // (§2.2 "all other values are invalid" と一致)。
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

/// `font-style: <ident>` を parse する (CSS Fonts 4 §2.4
/// <https://www.w3.org/TR/css-fonts-4/#font-style-prop>)。
///
/// Value grammar (§2.4, full property grammar): `normal | italic | left |
/// right | oblique <angle [-90deg,90deg]>?`。本 parser は `normal` /
/// `italic` / bare `oblique` の 3 keyword のみ受理する ([`FontStyle`] doc の
/// Scope carving 節参照) — `oblique` に続く `<angle>` 引数と `left` / `right`
/// は spec-valid だが未実装のため、他の未知 ident と同じく silent drop =
/// `None` とする。`oblique <angle>` (例: `oblique 14deg`) はこの関数自体は
/// `oblique` の ident だけを consume して成功で返るが、後続の `<angle>`
/// token が未消費のまま残るため、宣言全体が caller ([`mod@crate::rule`] の
/// `DeclParser`) の exhaustive-consumption check で drop される
/// ([`parse_text_transform`] doc の「case keyword が先」ケースと同 mechanism)。
/// ASCII case-insensitive で ident を比較する (sibling [`parse_direction`]
/// と同 flavor)。
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

/// Parse CSS Fonts 4 `font-variation-settings` for computed-value CSSOM exposure.
///
/// Duplicate tags keep their last value. A `BTreeMap` produces the spec's
/// ascending tag order without quadratic duplicate searches.
pub(super) fn parse_font_variation_settings(
    input: &mut Parser<'_, '_>,
) -> Option<FontVariationSettings> {
    if let Ok(ident) = input.try_parse(|input| input.expect_ident_cloned()) {
        return ident
            .eq_ignore_ascii_case("normal")
            .then_some(FontVariationSettings::Normal);
    }

    let mut settings = BTreeMap::new();
    loop {
        let tag = {
            let tag = input.expect_string().ok()?;
            if tag.len() != 4 || !tag.bytes().all(|byte| (0x20..=0x7e).contains(&byte)) {
                return None;
            }
            SmolStr::new(tag.as_ref())
        };
        let value = expect_number_stable(input).ok()?;
        settings.insert(tag, value);

        if input.try_parse(|input| input.expect_comma()).is_err() {
            break;
        }
    }

    if settings.is_empty() {
        return None;
    }
    Some(FontVariationSettings::Settings(
        settings
            .into_iter()
            .map(|(tag, value)| FontVariationSetting { tag, value })
            .collect(),
    ))
}

pub(super) fn parse_font_style(input: &mut Parser<'_, '_>) -> Option<FontStyle> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "normal" => Some(FontStyle::Normal),
        "italic" => Some(FontStyle::Italic),
        "oblique" => Some(FontStyle::Oblique),
        _ => None,
    }
}

/// `font-variant-caps: <ident>` を parse する (CSS Fonts Module Level 3 §6.6
/// <https://www.w3.org/TR/css-fonts-3/#font-variant-caps-prop>)。
///
/// Value grammar (§6.6, full property grammar): `normal | small-caps |
/// all-small-caps | petite-caps | all-petite-caps | unicase |
/// titling-caps`。本 parser はこの 7 keyword 全てを受理する
/// ([`FontVariantCaps`] doc の「7 keyword の意味」節参照)。それ以外の ident は
/// spec-invalid = 未知 ident として silent drop = `None` とする。
/// ASCII case-insensitive で ident を比較する (sibling [`parse_font_style`]
/// と同 flavor)。
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

/// `word-break: <ident>` を parse する (CSS Text 3 §5.1
/// <https://www.w3.org/TR/css-text-3/#word-break-property>)。
///
/// Value grammar (§5.1, full property grammar): `normal | keep-all |
/// break-all | break-word`。本 parser は `normal` / `keep-all` /
/// `break-all` の 3 keyword のみ受理する ([`WordBreak`] doc の Scope
/// carving 節参照) — 4th keyword `break-word` (deprecated,
/// `word-break: normal` + `overflow-wrap: anywhere` の compound 相当) は
/// spec-valid だが未実装のため、他の未知 ident と同じく silent drop =
/// `None` とする。ASCII case-insensitive で ident を比較する (sibling
/// `parse_font_style` と同 flavor)。
pub(super) fn parse_word_break(input: &mut Parser<'_, '_>) -> Option<WordBreak> {
    WordBreak::from_css_ident(input.expect_ident().ok()?)
}

/// `overflow-wrap: <ident>` (`word-wrap` legacy alias 名でも呼ばれる、
/// [`OverflowWrap`] doc の「legacy alias」節参照) を parse する (CSS Text 3
/// §5.4 <https://www.w3.org/TR/css-text-3/#overflow-wrap-property>)。
///
/// Value grammar (§5.4): `normal | break-word | anywhere` — 3 keyword とも
/// 受理する (`WordBreak` の deprecated `break-word` とは異なり、
/// `overflow-wrap` 自身の `break-word` は deprecated ではない spec-valid
/// keyword、[`OverflowWrap`] doc 参照)。ASCII case-insensitive で ident を
/// 比較する (sibling `parse_word_break` と同 flavor)。
pub(super) fn parse_overflow_wrap(input: &mut Parser<'_, '_>) -> Option<OverflowWrap> {
    OverflowWrap::from_css_ident(input.expect_ident().ok()?)
}

/// `white-space: <ident>` を parse する (CSS Text 3 §3
/// <https://www.w3.org/TR/css-text-3/#white-space-property>)。
///
/// Value grammar (§3, full property grammar): `normal | pre | nowrap |
/// pre-wrap | break-spaces | pre-line`。本 parser は `normal` / `pre` /
/// `nowrap` / `pre-wrap` / `pre-line` の 5 keyword のみ受理する
/// ([`WhiteSpace`] doc の Scope carving 節参照) — 6th keyword
/// `break-spaces` は spec-valid だが未実装のため、他の未知 ident と同じく
/// silent drop = `None` とする。ASCII case-insensitive で ident を比較する
/// (sibling `parse_word_break` と同 flavor)。
pub(super) fn parse_white_space(input: &mut Parser<'_, '_>) -> Option<WhiteSpace> {
    WhiteSpace::from_css_ident(input.expect_ident().ok()?)
}

/// Parses one `white-space-collapse` keyword (CSS Text 4:
/// <https://www.w3.org/TR/css-text-4/#propdef-white-space-collapse>).
/// Property identifiers are matched ASCII case-insensitively.
pub(super) fn parse_white_space_collapse(input: &mut Parser<'_, '_>) -> Option<WhiteSpaceCollapse> {
    WhiteSpaceCollapse::from_css_ident(input.expect_ident().ok()?)
}

/// `hyphens: <ident>` を parse する (CSS Text 3 §5.3
/// <https://www.w3.org/TR/css-text-3/#hyphens-property>)。
///
/// Value grammar (§5.3): `none | manual | auto` — 3 keyword とも受理する。
/// `auto` は `manual` に collapse せず、parse 段では別 keyword として
/// そのまま [`Hyphens::Auto`] を返す ([`Hyphens`] doc の「Downstream
/// handoff」節 — 両者の扱いの一致は downstream consumer 側の実装判断であり、
/// この parser の責務ではない)。ASCII case-insensitive で ident を比較する
/// (sibling `parse_word_break` と同 flavor)。
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

/// `text-autospace: normal | <autospace> | auto` を parse する
/// (CSS Text 4 §6.2.1 <https://drafts.csswg.org/css-text-4/#text-autospace-property>)。
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
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "start" => Some(TextAlignAll::Start),
        "end" => Some(TextAlignAll::End),
        "left" => Some(TextAlignAll::Left),
        "right" => Some(TextAlignAll::Right),
        "center" => Some(TextAlignAll::Center),
        "justify" => Some(TextAlignAll::Justify),
        "match-parent" => Some(TextAlignAll::MatchParent),
        _ => None,
    }
}

pub(super) fn parse_text_align_last(input: &mut Parser<'_, '_>) -> Option<TextAlignLast> {
    TextAlignLast::from_css_ident(input.expect_ident().ok()?)
}

/// `display: <ident>` を parse する。
///
/// CSS Display 3 §2 "Box Layout Modes: the display property"
/// <https://www.w3.org/TR/css-display-3/#propdef-display>。現状受理する
/// keyword は 18 つ:
///
/// - `block` — `<display-outside>` (block flow)
/// - `inline` — `<display-outside>` (inline flow、initial value)
/// - `inline-block` — `<display-legacy>` (inline flow-root)
/// - `none` — `<display-box>` (subtree omitted from box tree)
/// - `flex` — `<display-inside>` (§2.2) keyword、outer-defaulting rule
///   により `block flex` と等価
/// - `grid` — `<display-inside>` (§2.2) keyword、outer-defaulting rule
///   により `block grid` と等価
/// - `list-item` — `<display-listitem>` keyword、outer-defaulting rule
///   により `block flow list-item` と等価。HTML Living Standard の default
///   UA stylesheet が `li` に指定する
///   (<https://html.spec.whatwg.org/multipage/rendering.html#lists>)。
///   [`DisplayValue::ListItem`] の doc が言う通り keyword acceptance のみ
///   — marker box 生成は本 crate scope 外。
/// - `contents` — `<display-box>` (§2.5)、要素自身が box を生成しない
///   ([`DisplayValue::Contents`] doc 参照)
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
/// `inline-flex` / `inline-grid` は現在の layout bridge ではそれぞれ
/// `flex` / `grid` と同じ formatting context として受理する (inline-level
/// shrink-to-fit の区別は未実装)。`flow-root` 等の他 keyword は未実装のため
/// silent drop (`None`)。ASCII case-insensitive で ident を比較する (CSS Values 3
/// §3.1 "Pre-defined Keywords" <https://www.w3.org/TR/css-values-3/#keywords>:
/// keyword は ASCII case-insensitive)。
pub(super) fn parse_text_combine_upright(input: &mut Parser<'_, '_>) -> Option<TextCombineUpright> {
    TextCombineUpright::from_css_ident(input.expect_ident().ok()?)
}

pub(super) fn parse_text_orientation(input: &mut Parser<'_, '_>) -> Option<TextOrientation> {
    TextOrientation::from_css_ident(input.expect_ident().ok()?)
}

pub(super) fn parse_unicode_bidi(input: &mut Parser<'_, '_>) -> Option<UnicodeBidi> {
    UnicodeBidi::from_css_ident(input.expect_ident().ok()?)
}

/// `text-align: <ident>` を parse する
/// (CSS Text 3 §6.1 <https://www.w3.org/TR/css-text-3/#text-align-property>)。
///
/// Spec value grammar (§6.1): `start | end | left | right | center | justify |
/// match-parent | justify-all`。加えて inherited property の CSS-wide `inherit`
/// と HTML UA-only `-internal-center` を内部 cascade 用に受理する。ASCII
/// case-insensitive で ident を比較する
/// (CSS spec 慣行、sibling [`parse_string_fetch`](super::content::parse_string_fetch) / [`parse_content_part`](super::content::parse_content_part) /
/// [`parse_content_text_keyword`](super::content::parse_content_text_keyword) と同 flavor)。
///
/// # Scope carving ([`TextAlign`] doc-comment に詳述)
///
/// - **(b) 非対応**: `<string>` value は silent drop。CSS Text 3
///   §6.1 の grammar には無く、CSS Text 4 §7.1
///   <https://www.w3.org/TR/css-text-4/#text-align-property> で追加された
///   alternative (semantics は同 §7.2 "Character-based Alignment in a Table
///   Column")。
/// - **(b) 非対応**: CSS-wide keyword は未実装 (将来対応)、silent drop
///   (5 keyword の一覧・理由は [`PropertyValue`] doc の「CSS-wide keyword」節
///   が canonical)。
/// - **(a) spec-invalid**: 未知 keyword (`middle` 等) は silent drop = `None`。
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

/// `direction: <ident>` を parse する
/// (CSS Writing Modes 4 §2.1 <https://www.w3.org/TR/css-writing-modes-4/#direction>)。
///
/// Spec value grammar (§2.1): `ltr | rtl`。ASCII case-insensitive で ident を
/// 比較する (sibling [`parse_text_align`] と同 flavor)。
///
/// # Scope carving ([`Direction`] doc-comment に詳述)
///
/// - **部分対応**: CSS-wide `inherit` は computed cascade で親の値へ解決する。
///   他の CSS-wide keyword (`initial` / `unset` / `revert` / `revert-layer`) は
///   未実装で silent drop。
/// - **(a) spec-invalid**: `ltr` / `rtl` 以外の ident は silent drop = `None`。
pub(super) fn parse_direction(input: &mut Parser<'_, '_>) -> Option<Direction> {
    Direction::from_css_ident(input.expect_ident().ok()?)
}

/// `writing-mode: <ident>` を parse する
/// (CSS Writing Modes 4 §3.2 <https://www.w3.org/TR/css-writing-modes-4/#propdef-writing-mode>)。
///
/// Spec value grammar (§3.2): `horizontal-tb | vertical-rl | vertical-lr |
/// sideways-rl | sideways-lr`。ASCII case-insensitive で ident を比較する
/// (sibling [`parse_direction`] と同 flavor)。5 keyword とも spec 通り
/// 受理する — `vertical-rl` 以降 4 keyword の computed value normalization は
/// 本関数の責務ではなく [`resolve_writing_mode`] が担う ([`WritingMode`] doc の
/// Scope carving 節参照)。
///
/// # Scope carving ([`WritingMode`] doc-comment に詳述)
///
/// - **(b) 非対応**: CSS-wide keyword は未実装 (将来対応)、silent drop
///   (5 keyword の一覧・理由は [`PropertyValue`] doc の「CSS-wide keyword」節
///   が canonical)。
/// - **(a) spec-invalid**: 上記 5 keyword 以外の ident は silent drop = `None`。
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
/// blink ]` を parse する (CSS Text Decoration Module Level 3 §2.1
/// <https://www.w3.org/TR/css-text-decor-3/#text-decoration-line-property>)。
///
/// # top-level alternative (`none` vs. `||` combination)
///
/// grammar は `none | [ ... ]` — `none` は他 4 keyword と併記不可能な
/// **別 alternative** (`none underline` は spec-invalid) であり、`none` 自体が
/// `||` combination の一員ではない。よって `none` を最初に単独で試し、
/// 一致すれば即 return する。
///
/// # `||` (any-order, each-at-most-once) loop
///
/// `none` に一致しなければ、[`parse_border_shorthand`](super::box_model::parse_border_shorthand) の per-slot
/// `try_parse` loop と同じ shape で 4 keyword を順不同・重複無しに peel する
/// (詳細な rationale は同関数 doc 参照)。4 keyword の ident 集合は互いに
/// disjoint (border shorthand の width/style/color 3 slot が disjoint なのと
/// 同じ理由 — 単純に別々の語)。
///
/// - unfilled flag (未 true の bool field) のみ試行
/// - 埋まっている flag に対する 2 回目の同一 keyword は、その flag の
///   `try_parse` を試さない (falls through) ので match せず loop を抜ける —
///   caller ([`mod@crate::rule`] の `DeclParser`) の `expect_exhausted` が
///   leftover token を検知して declaration ごと drop する
///   (`text-decoration-line: underline underline` は 0 decl になる)
/// - 4 flag とも埋まった、またはどの keyword にも match しなくなったら break
/// - 1 個も flag が立たなければ (`none` でもなく、`||` combination も 0 個)
///   `None` — spec `||` grammar の "one or more of them must occur" 違反
pub(crate) fn parse_text_decoration_line(input: &mut Parser<'_, '_>) -> Option<TextDecorationLine> {
    // top-level alternative: `none`。`||` combination とは併記不可 (上記 doc)。
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(TextDecorationLine::NONE);
    }
    // top-level alternative: `spelling-error` / `grammar-error`。互いに
    // 排他、かつ `||` group とも併記不可 — 単独 ident の場合のみ受理し、
    // 後続 token が残れば caller の `expect_exhausted` が落とす
    // (spec grammar `none | [ ... ] | spelling-error | grammar-error`)。
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
        // `none` は上で既に処理済み — ここに来るのは 0 keyword しか
        // match しなかった場合のみ (未知 ident、または value 自体が空)。
        return None;
    }
    Some(line)
}

/// `text-decoration-style: solid | double | dotted | dashed | wavy` を
/// parse する (CSS Text Decoration Module Level 3 §2.2
/// <https://www.w3.org/TR/css-text-decor-3/#text-decoration-style-property>)。
/// ASCII case-insensitive で ident を比較する (sibling
/// [`parse_border_style_side`](super::box_model::parse_border_style_side) と同 flavor)。
pub(super) fn parse_text_decoration_style(
    input: &mut Parser<'_, '_>,
) -> Option<TextDecorationStyle> {
    TextDecorationStyle::from_css_ident(input.expect_ident().ok()?)
}

/// `text-decoration-color: <color>` を parse する (CSS Text Decoration Module
/// Level 3 §2.3
/// <https://www.w3.org/TR/css-text-decor-3/#text-decoration-color-property>)。
///
/// [`parse_border_color`](super::box_model::parse_border_color) と同型 — `currentcolor` keyword (CSS Color 3 §4.4)
/// を先取りしてから [`parse_color`] (hex / named / `rgb(a)` / `transparent`)
/// に委譲する。独立した helper にしてあるのは、両 property が異なる
/// payload 型 ([`TextDecorationColor`] / [`BorderColor`]) を持つため —
/// [`parse_border_color`](super::box_model::parse_border_color) 自体は border-*-color 専用のまま変更しない。
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
/// <'text-decoration-color'>` shorthand を parse する (CSS Text Decoration
/// Module Level 3 §2.4
/// <https://www.w3.org/TR/css-text-decor-3/#text-decoration-property>)。
///
/// [`parse_border_shorthand`](super::box_model::parse_border_shorthand) と同じ 3-slot `||` loop (line / style / color)
/// — 詳細な rationale・loop 構造・initial value fill の判断根拠は同関数 doc
/// 参照。3 slot の ident/token 集合は互いに disjoint: line keyword
/// (`none`/`underline`/`overline`/`line-through`/`blink`) と style keyword
/// (`solid`/`double`/`dotted`/`dashed`/`wavy`) はどちらも named CSS color
/// ではなく ([`parse_named_color`](cssparser::color::parse_named_color) のテーブルに無い)、[`parse_color`] の
/// Ident 分岐に誤って吸われることはない。
///
/// # Initial value fill (省略成分)
///
/// spec §2.4 verbatim: "Omitted values are set to their initial values."
/// - line 省略 → [`TextDecorationLine::NONE`] (§2.1 initial)
/// - style 省略 → [`TextDecorationStyle::Solid`] (§2.2 initial)
/// - color 省略 → [`TextDecorationColor::CurrentColor`] (§2.3 initial)
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

    // spec `||` grammar: at least 1 component 必須。0 component は `None` =
    // declaration drop (`parse_border_shorthand` と同じ判断)。
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

/// `text-decoration-skip-ink: auto | none | all` を parse する
/// (ED §2.10.4 <https://drafts.csswg.org/css-text-decor-4/#text-decoration-skip-ink-property>)。
/// ASCII case-insensitive で ident を比較する (sibling [`parse_text_decoration_style`]
/// と同 flavor)。
pub(super) fn parse_text_decoration_skip_ink(
    input: &mut Parser<'_, '_>,
) -> Option<TextDecorationSkipInk> {
    TextDecorationSkipInk::from_css_ident(input.expect_ident().ok()?)
}

/// `text-decoration-skip-spaces: none | all | [ start || end ]` を parse する
/// (ED §2.10.3 <https://drafts.csswg.org/css-text-decor-4/#text-decoration-skip-spaces-property>)。
/// `none` / `all` は単独 (top-level alternative)、`start` / `end` は `||`
/// loop で各最大 1 回 ([`parse_text_decoration_line`] の 4-keyword loop と同形)。
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

/// `text-decoration-thickness: auto | from-font | <length-percentage>` を parse する
/// (ED §2.4.1 <https://drafts.csswg.org/css-text-decor-4/#text-decoration-thickness-property>)。
/// `<line-width>` (`thin`/`medium`/`thick`) は scope 外のため drop
/// ([`TextDecorationThickness`] doc 参照)。`<length-percentage>` は
/// [`parse_length_value`] (`allow_percentage=true`) に委譲し、sign check
/// はしない (spec grammar に range 制限なし)。
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

/// `text-underline-offset: auto | <length-percentage>` を parse する。
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

/// `text-decoration-inset: <length>{1,2} | auto` を parse する
/// (ED §2.9.1 <https://drafts.csswg.org/css-text-decor-4/#text-decoration-inset-property>)。
/// ED grammar は `<length-percentage>` だが WPT が `%` を reject するため
/// `allow_percentage=false` ([`TextDecorationInset`] doc 参照)。負値・
/// `calc()` は受理する (`calc` は [`parse_value`](super::parse_value) の deferred 経路が
/// 事前に `DeferredValue` 化し、`純粋 [`Length`] のみ
/// ここに届く)。1 値目の場合は 2 値目を 1 値目に複製する
/// (margin/padding の 2-value 規則と同型)。
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

/// `text-emphasis-position: auto | ([ over | under ] && [ right | left ]?)` を parse する
/// (ED §3.4 <https://drafts.csswg.org/css-text-decor-4/#text-emphasis-position-property>)。
/// `auto` 単独、それ以外は vertical 必須 (`over`/`under`) +
/// horizontal 任意 (`right`/`left`)、順序自由・各最大 1 回。
///
/// `auto` 以外の位置に `auto` が来たら leftover として caller が drop する
/// ([`parse_font_style`] の oblique-angle 取扱いと同 mechanism)。
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

/// `text-underline-position: auto | [ from-font | under ] || [ left | right ]` を parse する
/// (ED §2.7 <https://drafts.csswg.org/css-text-decor-4/#text-underline-position-property>)。
/// 3 slot (`from-font` / `under` / horizontal) の `||` loop +
/// post-check: `from-font` と `under` の併記は reject
/// (`under from-font` invalid)、`left`+`right` 併記も reject
/// (`left right` invalid、horizontal slot が 1 個のため
/// loop 段階で保証)、`auto` 単独 (leftover は caller が drop)。
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

/// `vertical-align: <ident> | <length-percentage>` を parse する (CSS 2.1 §10.8.1
/// <https://www.w3.org/TR/CSS21/visudet.html#propdef-vertical-align>)。
/// `calc()` functions are handled earlier by `parse_value`'s shared math path;
/// this helper receives their simplified value or a plain length/percentage.
///
/// Ident は ASCII case-insensitive で比較する (sibling [`parse_direction`]
/// / [`parse_text_decoration_style`] と同 flavor)。ident 側を先に
/// `try_parse` で試し、ident token でなければ (= dimension/number token
/// の可能性があれば) `<length>` として再挑戦する — [`parse_flex_basis`](super::layout::parse_flex_basis)
/// / [`parse_letter_spacing`] と同じ「keyword 群 → length
/// フォールバック」構造。
///
/// # Scope carving ([`VerticalAlign`] doc-comment に詳述)
///
/// - **(b) 非対応**: `top` / `bottom` keyword は silent drop = `None` —
///   [`VerticalAlign`] doc 参照 (inline formatting context 依存)。
/// - **(b) 非対応**: CSS-wide keyword は未実装 (将来対応)、silent drop
///   (5 keyword の一覧・理由は [`PropertyValue`] doc の「CSS-wide keyword」節
///   が canonical)。
/// - **(a) spec-invalid**: `baseline` / `sub` / `super` / `middle` /
///   `text-top` / `text-bottom` 以外の ident、および `<length>` /
///   `<percentage>` grammar に合わない token は silent drop = `None`。
/// - `<length>` / `<percentage>` に non-negative filter は掛けない — spec が
///   "Raise (positive value) or lower (negative value)" と明示的に負値を
///   許容する ([`parse_letter_spacing`] と同じ判断、
///   `padding`/`border-width` の non-negative constraint とは対照的)。
///   percentage の解決は [`crate::resolve::resolve_vertical_align`] が
///   `used_line_height_length` 基準で行い、`normal` 時は `0px` fallback
///   (同関数 doc 参照)。
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

/// `text-shadow: <color>? && <length>{2,3}` 成分の `<color>` slot。
///
/// [`parse_border_color`](super::box_model::parse_border_color) / [`parse_text_decoration_color`] と同型 —
/// `currentcolor` keyword (CSS Color 3 §4.4) を先取りしてから
/// [`parse_color`] (hex / named / `rgb(a)` / `transparent`) に委譲する。
/// 独立した helper にしてあるのは、他 2 者と異なる payload 型
/// ([`TextShadowColor`]) を持つため。
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
                },
                TextIndentLength::Length(Length::Em(em)) => LengthPercentageCalc {
                    percent: 0.0,
                    px: 0.0,
                    em,
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

/// `text-shadow` の 1 shadow entry — `<color>? && <length>{2,3}`。
/// This calc-aware parser is separate from `parse_drop_shadow_item`, which
/// preserves the filter parser's existing plain-length behavior.
///
/// # `&&` (both-required, any-order) grammar semantics
///
/// spec CSS Values 4 §2.2 "Component Value Combinators"
/// <https://www.w3.org/TR/css-values-4/#component-combinators> verbatim: "A
/// double ampersand (&&) separates two or more components, all of which must
/// occur, in any order." — 本 grammar では:
/// - length run (`<length>{2,3}`) は必須、1 回のみ
/// - `<color>` はその自身の `?` multiplier により 0 or 1 回
/// - 両者の順序は自由 (`1px 1px red` / `red 1px 1px` 全て valid)、ただし
///   length run 自体は contiguous (`1px red 1px` のように間へ `<color>` を
///   挟むことはできない — [`parse_text_shadow_lengths_with_calc`] が 1 unit として
///   parse する)
///
/// # Loop 実装
///
/// [`parse_border_shorthand`](super::box_model::parse_border_shorthand) の `||` loop と同じ shape — unfilled slot
/// (length run / color) を loop で peel、`try_parse` で order-independent に
/// 試す。length run を先に試すのは任意の順序選択 (どちらを先に試しても
/// 結果は変わらない、`try_parse` が失敗時に必ず rewind するため)。
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

/// `text-shadow: none | <shadow>#` を parse する。
///
/// CSS Text Decoration Module Level 3 §4
/// <https://www.w3.org/TR/css-text-decor-3/#text-shadow-property>。
///
/// `none` = 空 list (top-level alternative) — [`parse_content`](super::content::parse_content) /
/// [`parse_counter_property`](super::content::parse_counter_property) と同じ shape。comma-separated list は
/// `cssparser::Parser::parse_comma_separated` に委譲 (各 item の未消費
/// leftover token は同メソッドの `parse_until_before` → `parse_entirely`
/// が自動検知して declaration ごと drop する)。
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

/// `font` shorthand の `font-size` 成分 — [`parse_font_size`] の返す
/// 2 通り ([`PropertyValue::FontSize`] / [`PropertyValue::FontSizeRelative`])
/// をそのまま運ぶ small enum。[`FontShorthand`] の payload 専用で、
/// [`ComputedValues`] / [`SpecifiedValues`] には入らず、umbrella crate からも
/// re-export しない ([`BackgroundShorthand`] と同じ扱い)。
///
/// [`ComputedValues`]: crate::computed::ComputedValues
/// [`SpecifiedValues`]: crate::specified::SpecifiedValues
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FontShorthandSize {
    /// `<length-percentage>` / absolute-size keyword — [`parse_font_size`] の
    /// [`PropertyValue::FontSize`] 側。展開先は同 variant。
    Absolute(Length),
    /// `larger` / `smaller` — [`parse_font_size`] の
    /// [`PropertyValue::FontSizeRelative`] 側。展開先は同 variant
    /// (key はどちらも [`PropertyKey::FontSize`])。
    Relative(RelativeFontSize),
}

/// `font` shorthand の specified value carrier — CSS Fonts 4 §2.1 "Font
/// shorthand: the font property"
/// (<https://www.w3.org/TR/css-fonts-4/#font-prop>)。
///
/// Spec grammar (full): `[ [ <'font-style'> || <font-variant-css2> ||
/// <'font-weight'> || <font-width-css3> ]? <'font-size'> [ / <'line-height'> ]?
/// <'font-family'># ] | <system-font>`。本実装の subset:
///
/// # Scope carving
///
/// - **Preface** (`||` 3 slot): `font-style` は [`parse_font_style`] の範囲
///   (`normal` / `italic` / bare `oblique`)、`font-weight` は
///   [`parse_font_weight`] の全範囲 (`normal` / `bold` / `bolder` /
///   `lighter` / `<number [1,1000]>`) を受理。`font-variant-css2`
///   (`normal` / `small-caps`) は [`FontVariantCaps::Normal`] /
///   [`FontVariantCaps::SmallCaps`] に畳む — CSS Fonts 3 §6.9 の `font-variant`
///   shorthand 全体ではなく CSS2 subset のみ対応 (本 crate が持つのは
///   `font-variant-caps` longhand だけで、他 sub-property が無いため)。
///   `font-width-css3` (`font-stretch`) longhand は本 crate に存在しないため
///   `normal` のみ consume して捨てる (initial と同じ値なので reset 効果は
///   observable ではない)。`normal` 以外の stretch keyword
///   (`condensed` 等)・`font-variant-css2` 外の variant 指定は preface の
///   どの slot にも match せず、後続の `font-size` parse が失敗するため
///   declaration 全体が drop される (spec-valid だが未対応 = silent drop、
///   本 crate の一般 policy)。
/// - **System fonts** (`caption` / `icon` / `menu` / `message-box` /
///   `small-caption` / `status-bar`) は受理しない — longhand への分解が
///   UA 依存で本 crate の font model に載らないため、declaration ごと drop。
/// - **`font-size`** は [`parse_font_size`] をそのまま使う (absolute-size
///   keyword / `larger` / `smaller` / `<length-percentage>` 全範囲)。
/// - **`line-height`** (`/ ...` 付きの場合のみ) は [`parse_line_height`] を
///   そのまま使う (`normal` / `<number>` / `<length-percentage>` 全範囲)。
/// - **`font-family`** は [`parse_font_family`] をそのまま使う
///   (`<family-name>#`、1 要素以上必須)。
///
/// # Initial value fill (omitted components)
///
/// CSS Fonts 4 §2.1 verbatim: "The 'font' property is a shorthand for
/// [font-style, font-variant, font-weight, font-size, line-height,
/// font-family]" — 省略成分は spec initial value で埋める
/// ([`BackgroundShorthand`] doc の同名節と同じ規則)。
/// [`parse_font_shorthand`] は省略された `style` → [`FontStyle::Normal`]、
/// `variant` → [`FontVariantCaps::Normal`]、 `weight` → `400`、
/// `line-height` → [`LineHeight::Normal`] で埋める (`size` と `family` は
/// 必須のため省略不可)。埋め値は各 standalone longhand の initial と同一
/// ([`crate::specified::SpecifiedValues::initial`] の対応 field が canonical)。
///
/// [`crate::rule::expand_shorthand_into`] が [`PropertyValue::Font`] を
/// [`PropertyValue::FontStyle`] / [`PropertyValue::FontVariantCaps`] /
/// [`PropertyValue::FontWeight`] / size ([`PropertyValue::FontSize`] /
/// [`PropertyValue::FontSizeRelative`]) / [`PropertyValue::LineHeight`] /
/// [`PropertyValue::FontFamily`] の 6 longhand に展開する —
/// margin/padding/border/outline shorthand precedent と同じ
/// "parse-time expansion, never reaches cascade" 設計 (詳細は同関数の doc)。
///
/// `#[non_exhaustive]` は付けない — sibling shorthand-only carrier
/// ([`GridLineShorthand`] / [`TextDecorationShorthand`] /
/// [`BackgroundShorthand`]) と同じ理由 (本型は [`PropertyValue::Font`] の
/// payload 専用で、[`ComputedValues`] / [`SpecifiedValues`] には入らず、
/// umbrella crate からも re-export しない)。
///
/// [`ComputedValues`]: crate::computed::ComputedValues
/// [`SpecifiedValues`]: crate::specified::SpecifiedValues
#[derive(Clone, Debug, PartialEq)]
pub struct FontShorthand {
    /// `font-style` 成分 — 省略時は [`FontStyle::Normal`] (spec initial)。
    pub style: FontStyle,
    /// `font-variant-css2` 成分 — 省略時は [`FontVariantCaps::Normal`]
    /// (spec initial)。`small-caps` は [`FontVariantCaps::SmallCaps`]。
    pub variant: FontVariantCaps,
    /// `font-weight` 成分 — 省略時は `400` (`normal`、spec initial)。
    pub weight: FontWeightValue,
    /// `font-size` 成分 (必須) — [`FontShorthandSize`] 参照。
    pub size: FontShorthandSize,
    /// `line-height` 成分 — 省略時は [`LineHeight::Normal`] (spec initial)。
    pub line_height: LineHeight,
    /// `font-family` 成分 (必須) — [`parse_font_family`] の結果を共有する
    /// `Arc` ([`PropertyValue::FontFamily`] と同じ DoS 対策 pattern)。
    pub family: Arc<Vec<Atom>>,
}

/// `font` shorthand を parse する ([`FontShorthand`] doc の grammar 節参照)。
///
/// [`parse_background_shorthand`](super::visual::parse_background_shorthand) と同じ loop 構造: preface の各 unfilled
/// slot を `try_parse` で順に試し、成功したら slot を埋めて loop 先頭に戻る。
/// preface が確定したら必須の `font-size`、任意の `/ line-height`、必須の
/// `font-family` を順に parse する。`font-size` / `font-family` のいずれかが
/// 無い場合は `None` (declaration 全体が drop される)。leftover は呼び出し元
/// (`rule.rs` の declaration parser) の `expect_exhausted` が丸ごと drop する
/// ([`parse_background_shorthand`](super::visual::parse_background_shorthand) と同じ契約)。
pub(super) fn parse_font_shorthand(input: &mut Parser<'_, '_>) -> Option<FontShorthand> {
    // System-font keyword は longhand に分解できないため先に reject
    // (`font: menu` 等が preface の `normal` 扱いで誤って受理されるのを防ぐ)。
    // `try_parse` で囲むため cursor は消費されない。
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
    // `font-stretch` longhand は本 crate に無いため `normal` のみ consume
    // して捨てる (`FontShorthand` doc の Scope carving 節参照)。
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

    // 必須の `font-size` (`parse_font_size` が `PropertyValue` を返すため
    // `FontShorthandSize` に畳む — 同関数はこの 2 variant しか返さない)。
    let size = match parse_font_size(input)? {
        PropertyValue::FontSize(length) => FontShorthandSize::Absolute(length),
        PropertyValue::FontSizeRelative(relative) => FontShorthandSize::Relative(relative),
        // cov:ignore: `parse_font_size` は上記 2 variant しか返さない
        // (同関数の 2 return path が canonical) — 到達不能。
        _ => return None,
    };

    // 任意の `/ line-height`。
    let line_height = if input.try_parse(|i| i.expect_delim('/')).is_ok() {
        parse_line_height(input)?
    } else {
        LineHeight::Normal
    };

    // 必須の `font-family` (`<family-name>#`、1 要素以上)。
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
