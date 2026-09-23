//! Font and text property parsers, including `text-decoration`, `text-shadow`
//! and the `font` shorthand.

use std::sync::Arc;

use cssparser::{ParseError, Parser, Token};

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
/// covers all three components, returning [`TextIndentValue`].
///
/// No non-negative filter, unlike [`parse_padding_side`](super::box_model::parse_padding_side) — the spec places no
/// `[0,∞]` restriction on this grammar (negative indents are valid, sibling
/// [`parse_margin_side`](super::box_model::parse_margin_side) applies the same "no filter" treatment for the same
/// reason its own grammar allows negative values).
pub(super) fn parse_text_indent(input: &mut Parser<'_, '_>) -> Option<TextIndentValue> {
    // CSS Text 3 §8.1 grammar: `<length-percentage> && hanging? && each-line?`
    // Order-independent, but at least the length component must be present.
    // We collect optional hanging/each-line idents and one length-percentage,
    // in any order, then ensure no extra tokens.
    let mut length: Option<Length> = None;
    let mut hanging = false;
    let mut each_line = false;
    loop {
        // Try length-percentage (allow_percentage true)
        if length.is_none()
            && let Ok(l) = input.try_parse(|i| {
                parse_length_value(i, true).ok_or_else(|| i.new_custom_error::<(), ()>(()))
            })
        {
            length = Some(l);
            continue;
        }
        // Try hanging
        if !hanging
            && input
                .try_parse(|i| i.expect_ident_matching("hanging"))
                .is_ok()
        {
            hanging = true;
            continue;
        }
        // Try each-line
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

/// `letter-spacing: normal | <length>` / `word-spacing: normal | <length>`
/// を parse する。両 property は grammar が完全に同型 (CSS Text 3 §7.2
/// <https://www.w3.org/TR/css-text-3/#letter-spacing-property> / §7.1
/// <https://www.w3.org/TR/css-text-3/#word-spacing-property>) なので 1
/// 関数を共有する ([`LengthOrNormal`] doc の reuse pattern 節参照)。
///
/// # Ordering
///
/// `normal` (`try_parse` + `expect_ident_matching`) → [`parse_length_value`]
/// (`allow_percentage = false`) — [`parse_line_height`] と同じ 2-branch
/// shape だが、`<number>` branch が無い (grammar 自体に `<number>`
/// alternative が無いため、CSS Values 3 §5 の number-vs-length
/// disambiguation は本 property には適用されない — bare `0` はそのまま
/// [`parse_length_value`] の unitless-zero clause 経由で `Length::Px(0.0)`
/// になる)。
///
/// # Percentage は非対応
///
/// 両 property とも spec が "Percentages: N/A" (word-spacing) /
/// "Percentages: n/a" (letter-spacing) と明記する — `allow_percentage =
/// false` により `5%` は `_ => None` (Percentage token に対する
/// `allow_percentage` guard 不成立) で drop される。
///
/// # Negative length は許容 (non-negative filter を掛けない)
///
/// [`parse_line_height`] / [`parse_font_size`] 等の `[0,∞]` callers とは
/// 異なり、本関数は [`Length::payload`] による `>= 0.0` post-filter を
/// **意図的に行わない**。CSS Text 3 §7.2 (letter-spacing) / §7.1
/// (word-spacing) がいずれも "Values may be negative, but there may be
/// implementation-dependent limits." と明記するため — spec 自身が sign を
/// 制限していない ([`LengthOrAuto`] を使う `margin-*` と同じ扱い、`padding`
/// / `border-width` の non-negative constraint とは対照的)。
///
/// # Non-goals
///
/// - **(b) 非対応**: CSS-wide keyword は未実装 (将来対応)、silent drop
///   (5 keyword の一覧・理由は [`PropertyValue`] doc の「CSS-wide keyword」節
///   が canonical)。
/// - **(b) 非対応**: `calc()` / `var()` は未実装 (css-variables-and-math)、
///   silent drop
/// - **(a) spec-invalid → drop**: `<percentage>`、`auto` 等 spec-invalid
///   keyword は spec grammar 違反、drop
pub(crate) fn parse_letter_or_word_spacing(input: &mut Parser<'_, '_>) -> Option<LengthOrNormal> {
    // 1. `normal` keyword — spec initial value、"Computes to zero"。
    if input
        .try_parse(|i| i.expect_ident_matching("normal"))
        .is_ok()
    {
        return Some(LengthOrNormal::Normal);
    }
    // 2. `<length-percentage>` — CSS Text 4 adds percentage support
    //    (WPT letter-spacing-valid expects 120% / -10%). Sign not restricted.
    parse_length_value(input, true).map(LengthOrNormal::Length)
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
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "normal" => Some(FontVariantCaps::Normal),
        "small-caps" => Some(FontVariantCaps::SmallCaps),
        "all-small-caps" => Some(FontVariantCaps::AllSmallCaps),
        "petite-caps" => Some(FontVariantCaps::PetiteCaps),
        "all-petite-caps" => Some(FontVariantCaps::AllPetiteCaps),
        "unicase" => Some(FontVariantCaps::Unicase),
        "titling-caps" => Some(FontVariantCaps::TitlingCaps),
        _ => None,
    }
}

/// Parse the CSS Text `text-transform` grammar.
///
/// The optional case keyword and width keywords may appear in any order. At
/// most one case keyword and each width keyword are accepted.
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
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "normal" => Some(WordBreak::Normal),
        "keep-all" => Some(WordBreak::KeepAll),
        "break-all" => Some(WordBreak::BreakAll),
        "manual" => Some(WordBreak::Manual),
        "auto-phrase" => Some(WordBreak::AutoPhrase),
        "break-word" => Some(WordBreak::BreakWord),
        _ => None,
    }
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
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "normal" => Some(OverflowWrap::Normal),
        "break-word" => Some(OverflowWrap::BreakWord),
        "anywhere" => Some(OverflowWrap::Anywhere),
        _ => None,
    }
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
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "normal" => Some(WhiteSpace::Normal),
        "pre" => Some(WhiteSpace::Pre),
        "nowrap" => Some(WhiteSpace::Nowrap),
        "pre-wrap" => Some(WhiteSpace::PreWrap),
        "pre-line" => Some(WhiteSpace::PreLine),
        "break-spaces" => Some(WhiteSpace::BreakSpaces),
        _ => None,
    }
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
/// Parses `text-wrap: wrap | nowrap` (subset, CSS Text 4 §5).
pub(super) fn parse_text_wrap_mode(input: &mut Parser<'_, '_>) -> Option<TextWrapMode> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "wrap" => Some(TextWrapMode::Wrap),
        "nowrap" => Some(TextWrapMode::Nowrap),
        _ => None,
    }
}

pub(super) fn parse_hyphens(input: &mut Parser<'_, '_>) -> Option<Hyphens> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "none" => Some(Hyphens::None),
        "manual" => Some(Hyphens::Manual),
        "auto" => Some(Hyphens::Auto),
        _ => None,
    }
}

pub(super) fn parse_line_break(input: &mut Parser<'_, '_>) -> Option<LineBreak> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "auto" => Some(LineBreak::Auto),
        "loose" => Some(LineBreak::Loose),
        "normal" => Some(LineBreak::Normal),
        "strict" => Some(LineBreak::Strict),
        "anywhere" => Some(LineBreak::Anywhere),
        _ => None,
    }
}

pub(super) fn parse_text_justify(input: &mut Parser<'_, '_>) -> Option<TextJustify> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "auto" => Some(TextJustify::Auto),
        "none" => Some(TextJustify::None),
        "inter-word" => Some(TextJustify::InterWord),
        "inter-character" => Some(TextJustify::InterCharacter),
        "distribute" => Some(TextJustify::Distribute),
        _ => None,
    }
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
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "auto" => Some(TextAlignLast::Auto),
        "start" => Some(TextAlignLast::Start),
        "end" => Some(TextAlignLast::End),
        "left" => Some(TextAlignLast::Left),
        "right" => Some(TextAlignLast::Right),
        "center" => Some(TextAlignLast::Center),
        "justify" => Some(TextAlignLast::Justify),
        "match-parent" => Some(TextAlignLast::MatchParent),
        _ => None,
    }
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
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "none" => Some(TextCombineUpright::None),
        "all" => Some(TextCombineUpright::All),
        _ => None,
    }
}

pub(super) fn parse_text_orientation(input: &mut Parser<'_, '_>) -> Option<TextOrientation> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "mixed" => Some(TextOrientation::Mixed),
        "upright" => Some(TextOrientation::Upright),
        "sideways" => Some(TextOrientation::Sideways),
        _ => None,
    }
}

pub(super) fn parse_unicode_bidi(input: &mut Parser<'_, '_>) -> Option<UnicodeBidi> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "normal" => Some(UnicodeBidi::Normal),
        "embed" => Some(UnicodeBidi::Embed),
        "isolate" => Some(UnicodeBidi::Isolate),
        "bidi-override" => Some(UnicodeBidi::BidiOverride),
        "isolate-override" => Some(UnicodeBidi::IsolateOverride),
        "plaintext" => Some(UnicodeBidi::Plaintext),
        _ => None,
    }
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
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "start" => Some(TextAlign::Start),
        "end" => Some(TextAlign::End),
        "left" => Some(TextAlign::Left),
        "right" => Some(TextAlign::Right),
        "center" => Some(TextAlign::Center),
        "justify" => Some(TextAlign::Justify),
        "match-parent" => Some(TextAlign::MatchParent),
        "inherit" => Some(TextAlign::Inherit),
        "-internal-center" => Some(TextAlign::InternalCenter),
        "justify-all" => Some(TextAlign::JustifyAll),
        _ => None,
    }
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
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "ltr" => Some(Direction::Ltr),
        "rtl" => Some(Direction::Rtl),
        _ => None,
    }
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
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "horizontal-tb" => Some(WritingMode::HorizontalTb),
        "vertical-rl" => Some(WritingMode::VerticalRl),
        "vertical-lr" => Some(WritingMode::VerticalLr),
        "sideways-rl" => Some(WritingMode::SidewaysRl),
        "sideways-lr" => Some(WritingMode::SidewaysLr),
        _ => None,
    }
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
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "solid" => Some(TextDecorationStyle::Solid),
        "double" => Some(TextDecorationStyle::Double),
        "dotted" => Some(TextDecorationStyle::Dotted),
        "dashed" => Some(TextDecorationStyle::Dashed),
        "wavy" => Some(TextDecorationStyle::Wavy),
        _ => None,
    }
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
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "auto" => Some(TextDecorationSkipInk::Auto),
        "none" => Some(TextDecorationSkipInk::None),
        "all" => Some(TextDecorationSkipInk::All),
        _ => None,
    }
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
pub(super) fn parse_text_underline_offset(input: &mut Parser<'_, '_>) -> Option<LengthOrAuto> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(LengthOrAuto::Auto);
    }
    parse_length_value(input, true).map(LengthOrAuto::Length)
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

/// `vertical-align: <ident> | <length>` を parse する (CSS 2.1 §10.8.1
/// <https://www.w3.org/TR/CSS21/visudet.html#propdef-vertical-align>)。
///
/// Ident は ASCII case-insensitive で比較する (sibling [`parse_direction`]
/// / [`parse_text_decoration_style`] と同 flavor)。ident 側を先に
/// `try_parse` で試し、ident token でなければ (= dimension/number token
/// の可能性があれば) `<length>` として再挑戦する — [`parse_flex_basis`](super::layout::parse_flex_basis)
/// / [`parse_letter_or_word_spacing`] と同じ「keyword 群 → length
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
///   許容する ([`parse_letter_or_word_spacing`] と同じ判断、
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

/// `<length>{2,3}` の contiguous run (offset-x offset-y blur-radius?) を 1
/// unit として parse する — [`TextShadowItem`] doc の「Non-negative
/// blur-radius」節参照。
///
/// offset-x/offset-y は [`parse_shadow_length_reject_nan`] 経由 — 同関数の
/// doc が説明する `!is_nan()` guard を通す (sign 制限が無いため
/// blur-radius 側の `>= 0.0` incidental filter が効かない)。
///
/// blur-radius は [`parse_non_negative_length`] で non-negative を
/// enforce、省略時は `Length::Px(0.0)` (同 doc の「各成分の初期値埋め」節)。
/// caller ([`parse_text_shadow_item`]) が本関数全体を `try_parse` で包む
/// ことで、offset-x の parse 失敗 (= 最初の token が `<color>` 等) 時に
/// x/y いずれの消費も正しく rewind される
/// ([`parse_length_value`] は token を unconditional に消費するため、
/// [`parse_margin_side`](super::box_model::parse_margin_side) doc の「Order of alternative」節と同じ理由で
/// checkpoint 経由の rewind が要る)。
pub(crate) fn parse_text_shadow_lengths<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<(Length, Length, Length), ParseError<'i, ()>> {
    let x = parse_shadow_length_reject_nan_res(input)?;
    let y = parse_shadow_length_reject_nan_res(input)?;
    let blur = input
        .try_parse(|i| -> Result<Length, ParseError<'_, ()>> {
            parse_non_negative_length(i).ok_or_else(|| i.new_custom_error(()))
        })
        .unwrap_or(Length::Px(0.0));
    Ok((x, y, blur))
}

/// `text-shadow` の 1 shadow entry — `<color>? && <length>{2,3}`。
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
///   挟むことはできない — [`parse_text_shadow_lengths`] が 1 unit として
///   parse する)
///
/// # Loop 実装
///
/// [`parse_border_shorthand`](super::box_model::parse_border_shorthand) の `||` loop と同じ shape — unfilled slot
/// (length run / color) を loop で peel、`try_parse` で order-independent に
/// 試す。length run を先に試すのは任意の順序選択 (どちらを先に試しても
/// 結果は変わらない、`try_parse` が失敗時に必ず rewind するため)。
pub(super) fn parse_text_shadow_item(input: &mut Parser<'_, '_>) -> Option<TextShadowItem> {
    let mut lengths: Option<(Length, Length, Length)> = None;
    let mut color: Option<TextShadowColor> = None;

    loop {
        if lengths.is_some() && color.is_some() {
            break;
        }
        if lengths.is_none()
            && let Ok(triple) = input.try_parse(parse_text_shadow_lengths)
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

    // length run は必須 (spec grammar 上 `<length>{2,3}` に `?` が無い) —
    // 0 slot でも `<color>` だけが埋まる可能性は無いが、`parse_border_shorthand`
    // の「at least 1 component 必須」とは違い、本 grammar では length run
    // 単独でも valid (`<color>` は完全に optional)。
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
/// が自動検知して declaration ごと drop する — [`TextShadowItem`] doc の
/// 「Non-negative blur-radius」節が挙げる `text-shadow: 1px 1px -1px`
/// (負の blur-radius は unconsumed のまま残る) のような入力はこの経路で
/// reject される)。
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
