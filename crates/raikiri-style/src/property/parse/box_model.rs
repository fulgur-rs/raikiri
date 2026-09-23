//! Box-model property parsers: margin, padding, border, sizing, overflow,
//! positioning, border-radius, box-shadow and outline.

use cssparser::{ParseError, Parser};
use smol_str::SmolStr;

use crate::property::types::*;

use super::color::*;
use super::common::*;
use super::text::*;

/// `border-*-color` の value parser — `currentcolor` keyword を先取りしてから
/// 既存 [`parse_color`] に委譲する。
///
/// CSS Backgrounds 3 §3.1 <https://www.w3.org/TR/css-backgrounds-3/#border-color>
/// の border-*-color grammar は `<color>` そのもの、`<color>` production は
/// CSS Color 3 §4.4 <https://www.w3.org/TR/css-color-3/#currentColor-def>
/// `currentcolor` keyword を含む。しかし本 crate の [`parse_color`] は
/// cssparser の `parse_named_color` (RGB triple mapping)
/// 経由のため `currentcolor` は named-color table 未収載として `None` 側に
/// 落ちる — 本 helper が Ident 段で先取りする必要がある。resolution 委譲の
/// rationale は [`BorderColor`] enum doc 参照 (paint scope 責務)。
///
pub(super) fn parse_border_color(input: &mut Parser<'_, '_>) -> Option<BorderColor> {
    // `expect_ident_matching` は ASCII case-insensitive (cssparser 慣行、
    // sibling `parse_margin_side` line 1892 と同 shape の keyword intercept)。
    // 失敗時 `try_parse` が rewind、続く `parse_color` が Ident (named /
    // transparent) / Hash / Function の全 alternative を担当。
    if input
        .try_parse(|i| i.expect_ident_matching("currentcolor"))
        .is_ok()
    {
        return Some(BorderColor::CurrentColor);
    }
    parse_color(input).map(BorderColor::Resolved)
}

/// `<length-percentage> | auto` の共通 parser — margin longhand 1 side 分。
///
/// grammar reference: CSS Box 3 §3.1
/// <https://www.w3.org/TR/css-box-3/#margin-physical> "Value:
/// `<length-percentage> | auto`"。
///
/// # Order of alternative
///
/// `auto` ident branch を **先に** try_parse する — [`parse_length_value`] は内部で
/// `input.next()` を unconditional に消費 (fail 時も token を戻さない) するため、
/// naive な "try length first, then auto" だと `margin: auto` の `auto` ident
/// が length parser で drop され後段の auto match が届かない。try_parse で
/// checkpoint 経由の rewind を確保する (sibling: [`parse_content_list_items`](super::content::parse_content_list_items) の
/// bare `<string>` literal 分岐と同 pattern)。
///
/// `expect_ident_matching` is ASCII case-insensitive, so `AUTO` and `Auto`
/// are accepted as well.
pub(crate) fn parse_margin_side(input: &mut Parser<'_, '_>) -> Option<LengthOrAuto> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(LengthOrAuto::Auto);
    }
    parse_length_value(input, true).map(LengthOrAuto::Length)
}

/// `margin: <'margin-top'>{1,4}` shorthand — 1-4 value expansion 実装。
///
/// grammar reference: CSS Box 3 §3.2
/// <https://www.w3.org/TR/css-box-3/#margin-shorthand>。
///
/// # Expansion rules (spec verbatim, §3.2)
///
/// "If there is only one component value, it applies to all sides. If there
/// are two values, the top and bottom margins are set to the first value and
/// the right and left margins are set to the second. If there are three
/// values, the top is set to the first value, the left and right are set to
/// the second, and the bottom is set to the third. If there are four values
/// they apply to the top, right, bottom, and left, respectively."
///
/// # Trailing garbage handling
///
/// 5+ value (`margin: 10px 20px 30px 40px 50px`) は本 helper では 4 value 消費
/// して残り 1 token を unconsumed で return する。caller の
/// [`mod@crate::rule`] の `DeclParser` の
/// [`cssparser::DeclarationParser::parse_value`]
/// impl が `expect_exhausted` で余剰 token を
/// 検知して declaration ごと drop する (既存 [`parse_font_family`] 系と同じ
/// 責務分担、`rejects_extra_length_after_font_size` 系 test で pattern を pin)。
pub(super) fn parse_margin_shorthand(input: &mut Parser<'_, '_>) -> Option<Sides<LengthOrAuto>> {
    let v1 = parse_margin_side(input)?;
    // 2nd value 不在 → 1 value case: 全 4 side に spread (§3.2 "If there is only
    // one component value, it applies to all sides")。
    let Some(v2) = input.try_parse(|i| parse_margin_side(i).ok_or(())).ok() else {
        return Some(Sides::all(v1));
    };
    // 3rd 不在 → 2 value case: top/bottom = 1st, right/left = 2nd。
    let Some(v3) = input.try_parse(|i| parse_margin_side(i).ok_or(())).ok() else {
        return Some(Sides {
            top: v1,
            right: v2,
            bottom: v1,
            left: v2,
        });
    };
    // 4th 不在 → 3 value case: top = 1st, right/left = 2nd, bottom = 3rd。
    let Some(v4) = input.try_parse(|i| parse_margin_side(i).ok_or(())).ok() else {
        return Some(Sides {
            top: v1,
            right: v2,
            bottom: v3,
            left: v2,
        });
    };
    // 4 values: clockwise from top (top, right, bottom, left)。5th 以降は
    // 本 helper では消費せず、caller の `expect_exhausted` で drop される
    // (property.rs test `margin_shorthand_leaves_extra_values_for_caller_exhausted_check`
    //  で parse_value 単体挙動、rule.rs test `margin_shorthand_five_values_declaration_dropped`
    //  で end-to-end drop を pin)。
    Some(Sides {
        top: v1,
        right: v2,
        bottom: v3,
        left: v4,
    })
}

/// `margin-inline: <'margin-top'>{1,2}` / `margin-block: <'margin-top'>{1,2}`
/// shorthand を [`StartEnd<LengthOrAuto>`] に expand する — 1-2 value
/// expansion。
///
/// grammar reference: CSS Logical Properties and Values 1 §4.2
/// <https://www.w3.org/TR/css-logical-1/#propdef-margin-inline>: "The first
/// value represents the start edge style, and the second value represents
/// the end edge style. If only one value is given, it applies to both the
/// start and end edges." — `margin-block` の propdef も同じ文言・同じ
/// grammar (`<'margin-top'>{1,2}`) を共有するため、この 1 helper を
/// `margin-inline`/`margin-block` 両方の parser arm で共用する
/// ([`PropertyValue::MarginInline`] doc 参照 — 物理 axis (left/right か
/// top/bottom か) を決めるのは呼び出し側が選ぶ `PropertyValue` variant で
/// あり、この関数自体は axis を知らない)。
///
/// # Robustness
///
/// 3 個目以降の value は本 helper では consume せず leftover として残す →
/// caller (`rule.rs::DeclParser`) の `expect_exhausted` が declaration
/// ごと drop する ([`parse_margin_shorthand`] の "Trailing garbage
/// handling" 節と同型)。
pub(super) fn parse_margin_logical_shorthand(
    input: &mut Parser<'_, '_>,
) -> Option<StartEnd<LengthOrAuto>> {
    let start = parse_margin_side(input)?;
    // 2nd value 不在 → 1 value case: start/end 両方に spread (spec "If only
    // one value is given, it applies to both the start and end edges")。
    let Some(end) = input.try_parse(|i| parse_margin_side(i).ok_or(())).ok() else {
        return Some(StartEnd::both(start));
    };
    Some(StartEnd { start, end })
}

/// `width: auto | <length-percentage [0,∞]>` を parse する。
///
/// grammar reference: CSS Sizing 3 §3.1.1
/// <https://www.w3.org/TR/css-sizing-3/#preferred-size-properties> "Value:
/// `auto | <length-percentage [0,∞]> | min-content | max-content |
/// fit-content(<length-percentage>)`"、"Initial: auto"、"Inherited: no"。
///
/// # 非対応 (spec-valid、将来対応)
///
/// `min-content` / `max-content` / `fit-content()` は intrinsic sizing keyword
/// で未実装 — 本 helper では受理せず自然に `None` に落ちる (`auto` ident
/// 分岐で `expect_ident_matching("auto")` が fail、続く `parse_length_value` が
/// keyword / function token を Dimension / Percentage arm fall-through で drop)。
/// 負値 (`width: -10px`) は spec grammar `[0,∞]` violation として drop する。
///
/// # Order of alternatives
///
/// [`parse_margin_side`] と同 pattern の "auto ident branch 先行 try_parse":
/// [`parse_length_value`] は内部で `input.next()` を unconditional に消費する
/// (fail 時も token を戻さない) ため、naive な "try length first, then auto"
/// だと `width: auto` の `auto` ident が length parser で drop され後段の auto
/// match が届かない。`try_parse` で checkpoint 経由の rewind を確保する。
///
/// # Non-negative constraint
///
/// [`parse_padding_side`] と同 pattern の全 [`Length`] variant OR-pattern check —
/// spec `[0,∞]` の closed interval を parse-time enforce (Verification #4:
/// `width: -10px` → `None` → declaration drop)。padding と shape は同じだが
/// `auto` keyword 分岐が先行する (padding は `auto` を受理しない grammar
/// `<length-percentage [0,∞]>` のみ)。
pub(super) fn parse_width(input: &mut Parser<'_, '_>) -> Option<LengthOrAuto> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(LengthOrAuto::Auto);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("min-content"))
        .is_ok()
    {
        return Some(LengthOrAuto::Auto);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("max-content"))
        .is_ok()
    {
        return Some(LengthOrAuto::Auto);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("fit-content"))
        .is_ok()
    {
        let _ = input.try_parse(|i| {
            i.expect_parenthesis_block()?;
            i.parse_nested_block(|nested| {
                parse_length_value(nested, true)
                    .ok_or_else(|| nested.new_custom_error::<_, ()>(()))?;
                Ok::<_, cssparser::ParseError<'_, ()>>(())
            })
        });
        return Some(LengthOrAuto::Auto);
    }
    if input
        .try_parse(|i| {
            i.expect_function_matching("fit-content")?;
            i.parse_nested_block(|nested| {
                parse_length_value(nested, true)
                    .ok_or_else(|| nested.new_custom_error::<_, ()>(()))?;
                nested.expect_exhausted()?;
                Ok::<_, cssparser::ParseError<'_, ()>>(())
            })
        })
        .is_ok()
    {
        return Some(LengthOrAuto::Auto);
    }
    let length = parse_length_value(input, true)?;
    (length.payload() >= 0.0).then_some(LengthOrAuto::Length(length))
}

/// `padding-{top,right,bottom,left}` の single-side value を parse する。
///
/// grammar: `<length-percentage [0,∞]>` (CSS Box 3 §4.1
/// <https://www.w3.org/TR/css-box-3/#padding-physical>)。spec verbatim:
/// "Negative values for padding properties are invalid." — 負値は grammar 違反
/// として declaration ごと drop する。
///
/// # 実装 note
///
/// 1. [`parse_length_value`] を `allow_percentage=true` で呼ぶ (grammar が
///    `<length-percentage>`)。dimension 未対応 unit / `auto` keyword / non-numeric
///    token は同 helper が `None` に落とす (font-size 経路と同 pattern)。
/// 2. 全 [`Length`] variant の payload ([`Length::payload`] 経由) に対し
///    `>= 0.0` を確認、負値は `None` 返し (`Percent(-10.0)` = `-10%` も含む —
///    Verification #5 で pin)。
///
/// # Sibling pattern
///
/// [`parse_font_size`] の `<length-percentage>` 分岐 (ident 分岐で `None` に
/// なった後の tail) と同形 — どちらも `allow_percentage=true` で
/// [`parse_length_value`] を呼び、[`Length::payload`] で全 [`Length`] variant の
/// payload を抽出して `>= 0.0` を post-filter する (tail 部分の body は
/// identical)。`parse_font_size` は後に `<absolute-size>` /
/// `<relative-size>` / `math` の ident 分岐 (`parse_font_size_keyword`) が
/// 前段に付いたため関数全体としては同形ではなくなったが、この tail 部分の
/// ロジックは identical。
///
/// 両者が非対称だった時期 (font-size が `<length>` px-only scope で、padding
/// だけが `<length-percentage>` の 5 variant を受けていた頃) の記述は
/// font-relative unit 対応で解消済み。
pub(super) fn parse_padding_side(input: &mut Parser<'_, '_>) -> Option<Length> {
    let length = parse_length_value(input, true)?;
    // spec (CSS Box 3) §4.1: "Negative values for padding properties are invalid."。
    (length.payload() >= 0.0).then_some(length)
}

/// `padding: <'padding-top'>{1,4}` shorthand を [`Sides<Length>`] に expand する。
///
/// CSS Box 3 §4.2 <https://www.w3.org/TR/css-box-3/#padding-shorthand> の
/// 1-4 value expansion (逐語引用ではないので `verbatim` 表記は使わない):
///
/// - 1 value: all 4 sides = value
/// - 2 values: top/bottom = 1st, left/right = 2nd
/// - 3 values: top = 1st, left/right = 2nd, bottom = 3rd
/// - 4 values: top / right / bottom / left (clockwise from top)
///
/// # Robustness
///
/// - 5 個目以降の value は本関数では consume せず leftover として残す →
///   caller (`rule.rs::DeclParser`) の `expect_exhausted` が declaration
///   ごと drop する (`padding: 1px 2px 3px 4px 5px` → invalid, drop)。
/// - 0 value (input が empty) は 1st `parse_padding_side` が `None` を返し
///   全体 `None` propagate。
/// - 各 value の non-negative constraint は [`parse_padding_side`] が個別に
///   enforce (負値混じり `padding: 10px -5px` → 2nd で `None`、全体 drop)。
pub(super) fn parse_padding_shorthand(input: &mut Parser<'_, '_>) -> Option<Sides<Length>> {
    // 1st value 必須。無ければ全体 drop (0-value form は grammar 違反)。
    let v1 = parse_padding_side(input)?;
    // 2-4 value は sequential `try_parse` で optional 取得。`try_parse` は
    // 失敗時に parser position を rewind するため、前段 None 時にも下段の
    // try_parse は同 token を再 read → 同 fail、guard 不要 (自然 short-circuit)。
    let v2 = input.try_parse(parse_padding_side_res).ok();
    let v3 = input.try_parse(parse_padding_side_res).ok();
    let v4 = input.try_parse(parse_padding_side_res).ok();
    // spec (CSS Box 3) §4.2 1-4 value expansion (code, not a spec quote):
    let sides = match (v2, v3, v4) {
        (None, _, _) => Sides::all(v1),
        (Some(h), None, _) => Sides {
            top: v1,
            right: h,
            bottom: v1,
            left: h,
        },
        (Some(h), Some(b), None) => Sides {
            top: v1,
            right: h,
            bottom: b,
            left: h,
        },
        (Some(r), Some(b), Some(l)) => Sides {
            top: v1,
            right: r,
            bottom: b,
            left: l,
        },
    };
    Some(sides)
}

/// [`parse_padding_side`] の `Result` 版 — `try_parse` は closure 内で
/// `Result` を要求するため wrapper 化。
pub(super) fn parse_padding_side_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<Length, ParseError<'i, ()>> {
    parse_padding_side(input).ok_or_else(|| input.new_custom_error(()))
}

/// `padding-inline: <'padding-top'>{1,2}` / `padding-block: <'padding-top'>{1,2}`
/// shorthand を [`StartEnd<Length>`] に expand する — 1-2 value expansion。
///
/// grammar reference: CSS Logical Properties and Values 1 §4.4
/// <https://www.w3.org/TR/css-logical-1/#propdef-padding-inline> — same
/// "first value = start, second value = end, one value spreads to both"
/// text as [`parse_margin_logical_shorthand`] documents in full for the
/// margin sibling; `padding-block`'s propdef shares the same grammar, so
/// this one helper backs both parser arms (axis choice is the caller's,
/// via which `PropertyValue` variant wraps the result).
///
/// # Robustness
///
/// 1st value の non-negative constraint 違反は [`parse_padding_side`] の
/// `?` propagation でそのまま `None` になる。2nd value 位置の違反は
/// `try_parse` が rewind するため **1-value form の `Some` として扱われ**、
/// 違反した token は unconsumed のまま残る → caller
/// (`rule.rs::DeclParser`) の `expect_exhausted` がその leftover を検知して
/// declaration ごと drop する ([`parse_padding_shorthand`] の "Robustness"
/// 節と同型。
pub(super) fn parse_padding_logical_shorthand(
    input: &mut Parser<'_, '_>,
) -> Option<StartEnd<Length>> {
    let start = parse_padding_side(input)?;
    let end = input.try_parse(parse_padding_side_res).ok();
    Some(match end {
        None => StartEnd::both(start),
        Some(end) => StartEnd { start, end },
    })
}

/// `border-width` の `medium` keyword (= spec 上の initial value) に対応する
/// px 値。
///
/// CSS Backgrounds 3 §3.3 "Line Thickness: the border-width properties"
/// (<https://www.w3.org/TR/css-backgrounds-3/#border-width>) 本文 verbatim:
/// "The thin, medium, and thick keywords are equivalent to 1px, 3px, and 5px,
/// respectively." — `font-size` の `medium` (UA 裁量、
/// [`crate::computed::INITIAL_FONT_SIZE_PX`] 参照) とは異なり、こちらは
/// **spec が規範的に定める厳密値**であり、raikiri の選択ではない。
///
/// The implementation uses one shared constant for the `medium` keyword and
/// the omitted shorthand width. The initial border value reuses the same
/// constant.
pub(crate) const BORDER_WIDTH_MEDIUM_PX: f32 = 3.0;

/// `border-{top,right,bottom,left}-width` の single-side value を parse する。
///
/// Grammar: `<line-width>` = `<length [0,∞]> | thin | medium | thick`
/// (CSS Backgrounds 3 §3.3 <https://www.w3.org/TR/css-backgrounds-3/#border-width>)。
/// **`<percentage>` は含まれない** — padding とは違う (
/// `parse_length_value(input, false)` = `<length>` mode を渡す)。
///
/// # Keyword mapping (spec 規定値)
///
/// spec §3.3 は 3 keyword を normative に規定する — verbatim: "The thin,
/// medium, and thick keywords are equivalent to 1px, 3px, and 5px,
/// respectively." 対応表:
/// - `thin`   → `Length::Px(1.0)`
/// - `medium` → `Length::Px(3.0)` (initial value)
/// - `thick`  → `Length::Px(5.0)`
///
/// UA 裁量ではなく spec 規定の equivalence なので、独立実装の制約下でも
/// そのまま採用できる (Chromium / Firefox / WebKit の実装とも一致)。
///
/// # Sign / range
///
/// spec `<length [0,∞]>` の non-negative 制約は本 helper が enforce する
/// (負値 → `None` = declaration drop)。sibling [`parse_padding_side`] と同じ
/// post-filter pattern だが、`Length::Percent` variant は生成されない
/// (`allow_percentage=false` により Percentage token 自体が reject される)。
///
/// # Non-goals
///
/// - **(a) spec-invalid → drop**: 負値 (`-1px`)、未知 keyword (`fat` 等)、
///   spec-invalid unit (`%` は grammar に含まれない → drop)。
/// - **(b) 非対応**: CSS-wide keyword は未実装 (将来対応)、silent drop
///   (5 keyword の一覧・理由は [`PropertyValue`] doc の「CSS-wide keyword」節
///   が canonical)。
/// - **(b) 非対応**: `calc()` / `var()` は未実装 (将来対応)、silent drop。
pub(super) fn parse_border_width_side(input: &mut Parser<'_, '_>) -> Option<Length> {
    // 1. keyword branch (thin / medium / thick) を先に try — `parse_length_value`
    //    は unconditional に token を consume するため、`try_parse` で rewind を
    //    確保する必要がある (sibling `parse_margin_side` の `auto` branch と同
    //    pattern)。
    let keyword = input.try_parse(|i| -> Result<Length, ParseError<'_, ()>> {
        let ident = i.expect_ident()?.clone();
        match ident.to_ascii_lowercase().as_str() {
            "thin" => Ok(Length::Px(1.0)),
            "medium" => Ok(Length::Px(BORDER_WIDTH_MEDIUM_PX)),
            "thick" => Ok(Length::Px(5.0)),
            _ => Err(i.new_custom_error(())),
        }
    });
    if let Ok(l) = keyword {
        return Some(l);
    }
    // 2. `<length [0,∞]>` — allow_percentage=false で `<length>` mode
    //    (Percentage token は reject される、`<percentage>` は grammar 外)。
    let length = parse_length_value(input, false)?;
    // spec `<length [0,∞]>` の non-negative constraint — `Length::payload` は
    // `Percent` も含む全 variant に対して定義されているが、`Percent` は
    // `allow_percentage=false` により本関数へは到達し得ない (unreachable、
    // dead value であって dead code ではない — helper 自体は border-width
    // 専用ではないため分岐を割ることはしない)。
    (length.payload() >= 0.0).then_some(length)
}

/// [`parse_border_width_side`] の `Result` 版 — `try_parse` は closure 内で
/// `Result` を要求するため wrapper 化 ([`parse_padding_side_res`] と同 pattern)。
fn parse_border_width_side_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<Length, ParseError<'i, ()>> {
    parse_border_width_side(input).ok_or_else(|| input.new_custom_error(()))
}

/// `border-{top,right,bottom,left}-style` の single-side value を parse する。
///
/// Grammar: `<line-style>` = `none | hidden | dotted | dashed | solid | double
/// | groove | ridge | inset | outset` (CSS Backgrounds 3 §3.2
/// <https://www.w3.org/TR/css-backgrounds-3/#border-style>)。
/// ASCII case-insensitive で ident と照合 (sibling
/// [`parse_display`](super::layout::parse_display) / [`parse_text_align`] と同 flavor)。
///
/// # Non-goals
///
/// - **(a) spec-invalid → drop**: 未知 keyword (`wavy` 等) は silent drop。
/// - **(b) 非対応**: CSS-wide keyword は未実装 (将来対応)、silent drop
///   (5 keyword の一覧・理由は [`PropertyValue`] doc の「CSS-wide keyword」節
///   が canonical)。
pub(super) fn parse_border_style_side(input: &mut Parser<'_, '_>) -> Option<BorderStyle> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "none" => Some(BorderStyle::None),
        "hidden" => Some(BorderStyle::Hidden),
        "dotted" => Some(BorderStyle::Dotted),
        "dashed" => Some(BorderStyle::Dashed),
        "solid" => Some(BorderStyle::Solid),
        "double" => Some(BorderStyle::Double),
        "groove" => Some(BorderStyle::Groove),
        "ridge" => Some(BorderStyle::Ridge),
        "inset" => Some(BorderStyle::Inset),
        "outset" => Some(BorderStyle::Outset),
        _ => None,
    }
}

/// `parse_border_style_side` の `Result` 版 (`try_parse` 用)。
fn parse_border_style_side_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<BorderStyle, ParseError<'i, ()>> {
    parse_border_style_side(input).ok_or_else(|| input.new_custom_error(()))
}

/// `parse_border_color` の `Result` 版 (`try_parse` 用)。
fn parse_border_color_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<BorderColor, ParseError<'i, ()>> {
    parse_border_color(input).ok_or_else(|| input.new_custom_error(()))
}

/// `border-style: <line-style>{1,4}` shorthand (CSS Backgrounds 3 §3.4)。
/// 1-4 value expansion は [`parse_padding_shorthand`] と同型 (1 → all、
/// 2 → vertical/horizontal、3 → top/horizontal/bottom、4 → clockwise)。
/// 5 value 以降は caller の `expect_exhausted` が drop (padding precedent)。
pub(super) fn parse_border_style_shorthand(
    input: &mut Parser<'_, '_>,
) -> Option<Sides<BorderStyle>> {
    let v1 = parse_border_style_side(input)?;
    let v2 = input.try_parse(parse_border_style_side_res).ok();
    let v3 = input.try_parse(parse_border_style_side_res).ok();
    let v4 = input.try_parse(parse_border_style_side_res).ok();
    let sides = match (v2, v3, v4) {
        (None, _, _) => Sides::all(v1),
        (Some(h), None, _) => Sides {
            top: v1,
            right: h,
            bottom: v1,
            left: h,
        },
        (Some(h), Some(b), None) => Sides {
            top: v1,
            right: h,
            bottom: b,
            left: h,
        },
        (Some(r), Some(b), Some(l)) => Sides {
            top: v1,
            right: r,
            bottom: b,
            left: l,
        },
    };
    Some(sides)
}

/// `border-width: <line-width>{1,4}` shorthand (CSS Backgrounds 3 §3.4)。
/// 各 value の grammar は [`parse_border_width_side`] (thin/medium/thick +
/// 非負 `<length>`)、1-4 expansion は [`parse_padding_shorthand`] と同型。
pub(super) fn parse_border_width_shorthand(input: &mut Parser<'_, '_>) -> Option<Sides<Length>> {
    let v1 = parse_border_width_side(input)?;
    let v2 = input.try_parse(parse_border_width_side_res).ok();
    let v3 = input.try_parse(parse_border_width_side_res).ok();
    let v4 = input.try_parse(parse_border_width_side_res).ok();
    let sides = match (v2, v3, v4) {
        (None, _, _) => Sides::all(v1),
        (Some(h), None, _) => Sides {
            top: v1,
            right: h,
            bottom: v1,
            left: h,
        },
        (Some(h), Some(b), None) => Sides {
            top: v1,
            right: h,
            bottom: b,
            left: h,
        },
        (Some(r), Some(b), Some(l)) => Sides {
            top: v1,
            right: r,
            bottom: b,
            left: l,
        },
    };
    Some(sides)
}

/// `border-color: <color>{1,4}` shorthand (CSS Backgrounds 3 §3.4)。各
/// value の grammar は [`parse_border_color`] (`currentcolor` / named /
/// hash / function)、1-4 expansion は [`parse_padding_shorthand`] と同型。
pub(super) fn parse_border_color_shorthand(
    input: &mut Parser<'_, '_>,
) -> Option<Sides<BorderColor>> {
    let v1 = parse_border_color(input)?;
    let v2 = input.try_parse(parse_border_color_res).ok();
    let v3 = input.try_parse(parse_border_color_res).ok();
    let v4 = input.try_parse(parse_border_color_res).ok();
    let sides = match (v2, v3, v4) {
        (None, _, _) => Sides::all(v1),
        (Some(h), None, _) => Sides {
            top: v1,
            right: h,
            bottom: v1,
            left: h,
        },
        (Some(h), Some(b), None) => Sides {
            top: v1,
            right: h,
            bottom: b,
            left: h,
        },
        (Some(r), Some(b), Some(l)) => Sides {
            top: v1,
            right: r,
            bottom: b,
            left: l,
        },
    };
    Some(sides)
}

/// `border: <line-width> || <line-style> || <color>` shorthand を parse する。
///
/// CSS Backgrounds 3 §3.4 <https://www.w3.org/TR/css-backgrounds-3/#border-shorthands>。
/// 4 side 全てに同一 [`Border`] を配る (`Sides::all`)。
///
/// # `||` (any-order) grammar semantics
///
/// spec CSS Values 4 §2.2 "Component Value Combinators"
/// <https://www.w3.org/TR/css-values-4/#component-combinators> verbatim:
/// "A double bar (||) separates two or more options: one or more of them must
/// occur, in any order." — 本 shorthand では:
/// - each component は最大 1 回 (2 回目の同 slot ident は spec-invalid = drop)
/// - at least 1 component が必須 (0 component の empty `border:` は drop)
/// - order は自由 (`1px solid red` / `red 1px solid` / `solid 1px` 全て valid)
///
/// # Loop 実装
///
/// unfilled slot (width / style / color) を loop で peel:
/// 1. `try_parse` で order-independent に各 slot の parser を試す
/// 2. 埋まっている slot に match する token に当たったら stop (spec 準拠、caller
///    の `expect_exhausted` が leftover を drop する — 例: `border: 1px 2px` は
///    `1px` を width に置いた後 `2px` は既に埋まっている width slot に match して
///    stop、caller が leftover を検出して declaration ごと drop)
/// 3. 全 slot が埋まった or どの parser も match しなくなったら break
/// 4. 少なくとも 1 slot が埋まっていれば `Some`、0 slot なら `None`
///
/// # Initial value fill (省略成分)
///
/// spec §3.4 verbatim: "Omitted values are set to their initial values."
/// 各成分の initial:
/// - width 省略 → `Length::Px(3.0)` (medium initial)
/// - style 省略 → `BorderStyle::None` (initial、spec §3.2)
/// - color 省略 → [`BorderColor::CurrentColor`] (spec §3.1 initial、used-value
///   resolution は paint scope 責務)
///
/// # Non-goals (spec deviation 明示)
///
/// spec §3.4 では border shorthand が **border-image-* も reset** する (spec
/// verbatim: "The border shorthand also resets border-image to its initial
/// value.") が、本 crate は border-image を実装していないため
/// reset side effect を省略。
/// border-image longhand 実装時に統合する。
///
/// # Sibling pattern
///
/// [`parse_margin_shorthand`] / [`parse_padding_shorthand`] は `{1,4}`
/// multiplier (順序固定、side ごとに違う値) だが、本 shorthand は `||` (any-order、
/// side は 4 side 共通) — 別 pattern。sibling は `try_parse` 経由の rewind と
/// initial fill の点で共通 principle を持つ。
pub(crate) fn parse_border_shorthand(input: &mut Parser<'_, '_>) -> Option<Sides<Border>> {
    let mut width: Option<Length> = None;
    let mut style: Option<BorderStyle> = None;
    let mut color: Option<BorderColor> = None;

    // `||` grammar: at least 1 component 必須、each component 最大 1 回、
    // order 自由。全 slot 満了 or 未 match token 到達で break。
    //
    // 各 iteration は "unfilled slot を順に try_parse、成功したら continue、
    // どの slot にも match しなかったら break" の shape。`continue` の前に slot
    // 満了 check を置くことで、埋まっている slot に対する 2 回目 (`border: 1px
    // 2px`) は自動的に fall-through して break (caller の `expect_exhausted` が
    // 残 token を検知して declaration drop)。
    loop {
        // 全 slot 満了 → break (leftover token は caller `expect_exhausted` が drop)
        if width.is_some() && style.is_some() && color.is_some() {
            break;
        }

        // width slot (unfilled のみ試行) — keyword (thin/medium/thick) と length
        // の両方を扱う helper を direct 呼ぶ。`try_parse` で失敗時 rewind。
        // `let Ok(..) = ..` の nested-if は clippy::collapsible-if を回避するため
        // let-chain (rust 1.88+) で 1 段化。
        if width.is_none()
            && let Ok(v) = input.try_parse(parse_border_width_side_res)
        {
            width = Some(v);
            continue;
        }

        // style slot — ident が 10 keyword に match すれば埋める。`try_parse` で
        // 失敗時 rewind (width keyword `thin` / `medium` / `thick` を先に試すため
        // style keyword `none` / `solid` などとの間の ambiguity は無い、ident 集合が
        // disjoint)。
        if style.is_none()
            && let Ok(s) = input.try_parse(|i| -> Result<BorderStyle, ParseError<'_, ()>> {
                parse_border_style_side(i).ok_or_else(|| i.new_custom_error(()))
            })
        {
            style = Some(s);
            continue;
        }

        // color slot — `parse_border_color` を reuse。hex / named / rgb(a) /
        // transparent の全 alternative + `currentcolor` keyword (CSS Color 3
        // §4.4) を受理。4 longhand parse site (border-{top,right,bottom,left}-color)
        // と同じ helper を経由することで sibling convention consistency を
        // 担保。
        if color.is_none()
            && let Ok(c) = input.try_parse(|i| -> Result<BorderColor, ParseError<'_, ()>> {
                parse_border_color(i).ok_or_else(|| i.new_custom_error(()))
            })
        {
            color = Some(c);
            continue;
        }

        // どの unfilled slot にも match しなかった → 埋まっている slot に対する
        // 2 回目の指定 or 未知 token。break で loop 終了、caller の
        // `expect_exhausted` が leftover を drop する (`border: 1px 2px` →
        // `2px` は width slot 満了で本 fall-through 到達、declaration ごと drop)。
        break;
    }

    // spec `||` grammar: at least 1 component 必須。0 component (empty `border:`
    // or 未知 keyword only) は `None` = declaration drop。
    if width.is_none() && style.is_none() && color.is_none() {
        return None;
    }

    // 省略成分は spec §3.4 の initial value で埋める。
    let border = Border {
        width: width.unwrap_or(Length::Px(BORDER_WIDTH_MEDIUM_PX)), // medium
        style: style.unwrap_or(BorderStyle::None),
        // §3.1 initial "currentcolor" — used-value resolution は paint scope
        // 責務。
        color: color.unwrap_or(BorderColor::CurrentColor),
    };
    Some(Sides::all(border))
}

/// `height: <length-percentage [0,∞]> | auto` を parse する。
///
/// grammar reference: CSS Sizing 3 §3.1.1 "Preferred Size Properties"
/// <https://www.w3.org/TR/css-sizing-3/#preferred-size-properties>。value
/// grammar は `auto | <length-percentage [0,∞]> | min-content | max-content |
/// fit-content(<length-percentage>)`、initial value `auto`、Inheritance `No`。
///
/// # Scope carving
///
/// - **(a) spec-invalid → drop**: 負値 (`height: -10px`) は grammar `[0,∞]` 違反、
///   全 [`Length`] variant の payload に対し `>= 0.0` post-filter で reject
///   ([`parse_padding_side`] の非負フィルタ pattern と同 shape)。
/// - **(b) 非対応 — 未対応 sizing keyword**: `min-content` /
///   `max-content` / `fit-content(<length-percentage>)` は現状 scope
///   外、silent drop (auto ident branch から外れる他 keyword は
///   `expect_ident_matching("auto")` が失敗 → length parser の Dimension /
///   Percentage arm でも受理されず None に落ちる)。
/// - **(b) 非対応 — CSS-wide keyword**: 未実装 (将来対応)、silent drop
///   (5 keyword の一覧・理由は [`PropertyValue`] doc の「CSS-wide keyword」節
///   が canonical。同 ident 経路で他 keyword と同じく
///   落ちる。
/// - **calc() / var()**: 未実装、本 task scope 外
///   (`Token::Function` は `parse_length_value` が Dimension / Percentage 以外を
///   silent drop)。
///
/// # Order of alternative (sibling: [`parse_margin_side`])
///
/// `auto` ident branch を **先に** try_parse する — [`parse_length_value`] は内部
/// で `input.next()` を unconditional に消費するため、naive な "try length first,
/// then auto" だと `height: auto` の `auto` ident が length parser で drop され
/// 後段の auto match が届かない。`try_parse` で checkpoint 経由の rewind を
/// 確保する ([`parse_margin_side`] と同 pattern — margin の grammar `<length-
/// percentage> | auto` と同 shape を LengthOrAuto payload で共有)。
///
/// `expect_ident_matching` is ASCII case-insensitive, so `AUTO` and `Auto`
/// are accepted as well.
///
/// # Non-negative filter (sibling: [`parse_padding_side`])
///
/// spec `<length-percentage [0,∞]>` (§3.1.1) の非負制約は [`Length::payload`]
/// 経由で全 [`Length`] variant の payload に対し `>= 0.0` を確認 —
/// [`parse_padding_side`] の同名 pattern を踏襲 (`<length-percentage [0,∞]>`
/// grammar と非負フィルタが対応する sibling)。`Percent(-10.0)` = `-10%` も
/// 含めて全 variant 経由で reject する。
pub(super) fn parse_height(input: &mut Parser<'_, '_>) -> Option<LengthOrAuto> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(LengthOrAuto::Auto);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("min-content"))
        .is_ok()
    {
        return Some(LengthOrAuto::Auto);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("max-content"))
        .is_ok()
    {
        return Some(LengthOrAuto::Auto);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("fit-content"))
        .is_ok()
    {
        let _ = input.try_parse(|i| {
            i.expect_parenthesis_block()?;
            i.parse_nested_block(|nested| {
                parse_length_value(nested, true)
                    .ok_or_else(|| nested.new_custom_error::<_, ()>(()))?;
                Ok::<_, cssparser::ParseError<'_, ()>>(())
            })
        });
        return Some(LengthOrAuto::Auto);
    }
    if input
        .try_parse(|i| {
            i.expect_function_matching("fit-content")?;
            i.parse_nested_block(|nested| {
                parse_length_value(nested, true)
                    .ok_or_else(|| nested.new_custom_error::<_, ()>(()))?;
                nested.expect_exhausted()?;
                Ok::<_, cssparser::ParseError<'_, ()>>(())
            })
        })
        .is_ok()
    {
        return Some(LengthOrAuto::Auto);
    }
    let length = parse_length_value(input, true)?;
    (length.payload() >= 0.0).then_some(LengthOrAuto::Length(length))
}

/// `min-width` / `min-height` / `min-block-size: auto | <length-percentage [0,∞]> | min-content | max-content | fit-content` を parse する。
///
/// Grammar reference: CSS Sizing 3 §4 "Minimum Size Properties"
/// <https://www.w3.org/TR/css-sizing-3/#min-size-properties>。initial value
/// `auto`、Inheritance `No`。sibling [`parse_max_size`] (CSS Sizing 3 §5) と
/// 同 shape で、`none` keyword 分岐が `auto` に置き換わる点だけが異なる
/// (min の initial は `auto`、`none` は max-only grammar)。
/// 非負制約 (`[0,∞]` → [`Length::payload`] post-filter) と intrinsic keyword の
/// Auto placeholder mapping は sibling と同一。
pub(super) fn parse_max_size(input: &mut Parser<'_, '_>) -> Option<LengthOrAuto> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(LengthOrAuto::Auto);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("min-content"))
        .is_ok()
    {
        return Some(LengthOrAuto::Auto);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("max-content"))
        .is_ok()
    {
        return Some(LengthOrAuto::Auto);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("fit-content"))
        .is_ok()
    {
        let _ = input.try_parse(|i| {
            i.expect_parenthesis_block()?;
            i.parse_nested_block(|nested| {
                parse_length_value(nested, true)
                    .ok_or_else(|| nested.new_custom_error::<_, ()>(()))?;
                Ok::<_, cssparser::ParseError<'_, ()>>(())
            })
        });
        return Some(LengthOrAuto::Auto);
    }
    if input
        .try_parse(|i| {
            i.expect_function_matching("fit-content")?;
            i.parse_nested_block(|nested| {
                parse_length_value(nested, true)
                    .ok_or_else(|| nested.new_custom_error::<_, ()>(()))?;
                nested.expect_exhausted()?;
                Ok::<_, cssparser::ParseError<'_, ()>>(())
            })
        })
        .is_ok()
    {
        return Some(LengthOrAuto::Auto);
    }
    let length = parse_length_value(input, true)?;
    (length.payload() >= 0.0).then_some(LengthOrAuto::Length(length))
}

/// `min-width` / `min-height: auto | <length-percentage [0,∞]> | min-content | max-content | fit-content` を parse する.
///
/// Grammar reference: CSS Sizing 3 §4 "Minimum Size Properties"
/// <https://www.w3.org/TR/css-sizing-3/#min-size-properties>。initial value
/// `auto`、Inheritance `No`。sibling `parse_max_size` (CSS Sizing 3 §5) と
/// 同 shape で、`none` keyword 分岐が `auto` に置き換わる点だけが異なる
/// (min の initial は `auto`、`none` は max-only grammar)。
/// 非負制約 (`[0,∞]` → `Length::payload` post-filter) と intrinsic keyword の
/// Auto placeholder mapping は sibling と同一。
pub(super) fn parse_min_size(input: &mut Parser<'_, '_>) -> Option<LengthOrAuto> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(LengthOrAuto::Auto);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("min-content"))
        .is_ok()
    {
        return Some(LengthOrAuto::Auto);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("max-content"))
        .is_ok()
    {
        return Some(LengthOrAuto::Auto);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("fit-content"))
        .is_ok()
    {
        let _ = input.try_parse(|i| {
            i.expect_parenthesis_block()?;
            i.parse_nested_block(|nested| {
                parse_length_value(nested, true)
                    .ok_or_else(|| nested.new_custom_error::<_, ()>(()))?;
                Ok::<_, cssparser::ParseError<'_, ()>>(())
            })
        });
        return Some(LengthOrAuto::Auto);
    }
    if input
        .try_parse(|i| {
            i.expect_function_matching("fit-content")?;
            i.parse_nested_block(|nested| {
                parse_length_value(nested, true)
                    .ok_or_else(|| nested.new_custom_error::<_, ()>(()))?;
                nested.expect_exhausted()?;
                Ok::<_, cssparser::ParseError<'_, ()>>(())
            })
        })
        .is_ok()
    {
        return Some(LengthOrAuto::Auto);
    }
    let length = parse_length_value(input, true)?;
    (length.payload() >= 0.0).then_some(LengthOrAuto::Length(length))
}

/// `box-sizing: <ident>` を parse する
/// (CSS Sizing 3 §3.3 <https://www.w3.org/TR/css-sizing-3/#box-sizing>)。
///
/// Spec value grammar (§3.3): `content-box | border-box`。ASCII
/// case-insensitive で ident を比較する (CSS Values 3 §3.1 "Pre-defined
/// Keywords"、sibling [`parse_display`](super::layout::parse_display) / [`parse_text_align`] と同 flavor)。
///
/// # Scope carving ([`BoxSizing`] doc-comment に詳述)
///
/// - **(b) 非対応**: CSS-wide keyword は未実装 (将来対応)、silent drop
///   (5 keyword の一覧・理由は [`PropertyValue`] doc の「CSS-wide keyword」節
///   が canonical)。
/// - **(a) spec-invalid**: 他 keyword (`padding-box` — CSS-UI 3 draft 相当
///   だが css-sizing-3 では削除、`margin-box` 等) は silent drop = `None`。
pub(super) fn parse_box_sizing(input: &mut Parser<'_, '_>) -> Option<BoxSizing> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "content-box" => Some(BoxSizing::ContentBox),
        "border-box" => Some(BoxSizing::BorderBox),
        _ => None,
    }
}

/// `overflow-x` / `overflow-y: <ident>` を parse する (
/// CSS Overflow 3 §3.1 <https://www.w3.org/TR/css-overflow-3/#overflow-properties>)。
///
/// Spec value grammar (§3.1): `visible | hidden | clip | scroll | auto`。
/// ASCII case-insensitive で ident を比較する (sibling [`parse_box_sizing`] /
/// [`parse_direction`] と同 flavor)。
///
/// # Scope carving ([`OverflowValue`] doc-comment に詳述)
///
/// - **(a) spec-invalid**: 上記 5 keyword 以外の ident は silent drop = `None`。
/// - **(b) 非対応**: CSS-wide keyword は未実装 (将来対応)、silent drop
///   (5 keyword の一覧・理由は [`PropertyValue`] doc の「CSS-wide keyword」節
///   が canonical)。
pub(super) fn parse_overflow_value(input: &mut Parser<'_, '_>) -> Option<OverflowValue> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "visible" => Some(OverflowValue::Visible),
        "hidden" => Some(OverflowValue::Hidden),
        "clip" => Some(OverflowValue::Clip),
        "scroll" => Some(OverflowValue::Scroll),
        "auto" => Some(OverflowValue::Auto),
        _ => None,
    }
}

/// `overflow: <'overflow-block'>{1,2}` shorthand — 1-2 value expansion。
///
/// grammar reference: CSS Overflow 3 §3.1
/// <https://www.w3.org/TR/css-overflow-3/#overflow-properties>。
///
/// # Expansion rule (spec verbatim, §3.1)
///
/// "The overflow property is a shorthand property that sets the specified
/// values of overflow-x and overflow-y in that order. If the second value is
/// omitted, it is copied from the first."
///
/// [`parse_margin_shorthand`] と同じ try_parse 積み上げ pattern の 2-value
/// 版 (1-4 value ではなく 1-2 value であること以外は同型)。
///
/// # Trailing garbage handling
///
/// 3rd value (`overflow: hidden scroll auto`) は本 helper では 2 value 消費
/// して残り 1 token を unconsumed で return する。caller の
/// [`mod@crate::rule`] の `DeclParser` の
/// [`cssparser::DeclarationParser::parse_value`] impl が `expect_exhausted`
/// で余剰 token を検知して declaration ごと drop する
/// ([`parse_margin_shorthand`] doc の「Trailing garbage handling」節と同じ
/// 責務分担)。
pub(super) fn parse_overflow_shorthand(input: &mut Parser<'_, '_>) -> Option<OverflowXY> {
    let v1 = parse_overflow_value(input)?;
    // 2nd value 不在 → 1 value case: 両 axis に spread (§3.1 "If the second
    // value is omitted, it is copied from the first.")。
    let Some(v2) = input.try_parse(|i| parse_overflow_value(i).ok_or(())).ok() else {
        return Some(OverflowXY::both(v1));
    };
    Some(OverflowXY { x: v1, y: v2 })
}

/// `position: static | sticky | running(<custom-ident>)` を parse する
/// (CSS GCPM 3 §1.2.1 <https://www.w3.org/TR/css-gcpm-3/#running-syntax> および
/// CSS Positioned Layout Module Level 3 §3
/// <https://www.w3.org/TR/css-position-3/#sticky-pos>)。
///
/// 現状 scope:
/// - `static` — [`PositionValue::Static`]、`inherit_from` の初期状態と一致するため
///   apply_value が no-op でも問題ない。cascade winner selection では
///   先行 `running(...)` を上書き suppress する identity 用途
///   (standalone-static test だけでは実効性が問えない点に注意)。
/// - `sticky` — [`PositionValue::Sticky`]、CSS Positioned Layout §3。
///   現状は parse のみ受理し `apply_value` は `Static` 同様に no-op (将来の
///   layout 連携まで保持する)。
/// - `running(<custom-ident>)` — [`PositionValue::Running`]、apply_value が
///   1-item `RunningTemplate` を computed.running_templates に seed する。
/// - 他 keyword (`relative` / `absolute` / `fixed`) は未実装、silent drop = `None`。
///
/// `<custom-ident>` の除外は string-set と同じ規約:
/// [`is_reserved_custom_ident`] (CSS-wide keyword + `default`) に加えて
/// `none` を弾く。`none` は position property の他 spec-defined keyword
/// では無いが、custom-ident としては予約 alternative の慣行を残しつつ、
/// runtime resolve で `element(none)` 参照を誤って matching させないためのガード
/// (string-set の `none` reject と同じ扱い)。
pub(super) fn parse_position(input: &mut Parser<'_, '_>) -> Option<PositionValue> {
    // `static` は現状 scope で受理する keyword の一つ。
    if input
        .try_parse(|i| i.expect_ident_matching("static"))
        .is_ok()
    {
        return Some(PositionValue::Static);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("relative"))
        .is_ok()
    {
        return Some(PositionValue::Relative);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("absolute"))
        .is_ok()
    {
        return Some(PositionValue::Absolute);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("fixed"))
        .is_ok()
    {
        return Some(PositionValue::Fixed);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("sticky"))
        .is_ok()
    {
        return Some(PositionValue::Sticky);
    }
    // `running(<custom-ident>)`。function name は ASCII case-insensitive、
    // 中身の custom-ident は case-preserving で SmolStr に格納。
    let running = input.try_parse(|i| -> Result<SmolStr, ParseError<'_, ()>> {
        let fn_name = i.expect_function()?.clone();
        if !fn_name.eq_ignore_ascii_case("running") {
            return Err(i.new_custom_error(()));
        }
        i.parse_nested_block(|inner| -> Result<SmolStr, ParseError<'_, ()>> {
            let ident = inner.expect_ident()?.clone();
            if is_reserved_custom_ident(&ident) || ident.eq_ignore_ascii_case("none") {
                return Err(inner.new_custom_error(()));
            }
            Ok(SmolStr::new(ident.as_ref()))
        })
    });
    running.ok().map(PositionValue::Running)
}

/// `top` / `right` / `bottom` / `left: auto | <length-percentage>` を parse する
/// (CSS Positioned Layout Module Level 3 §3).
pub(super) fn parse_inset(input: &mut Parser<'_, '_>) -> Option<LengthOrAuto> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(LengthOrAuto::Auto);
    }
    let length = parse_length_value(input, false)?;
    Some(LengthOrAuto::Length(length))
}

/// `z-index: auto | <integer>` を parse する (CSS2 §9.9.1
/// <https://www.w3.org/TR/CSS2/visuren.html#z-index>, [`ZIndexValue`] doc
/// 参照)。
///
/// `auto` ident branch を先に try_parse する — [`parse_margin_side`] と同じ
/// order-of-alternative 理由 (同関数 doc 参照)、ここでは the two branches
/// (`auto` ident と integer token) の token kind が既に不連続なので必須では
/// ないが、既存 sibling と同じ並びに揃える。
///
/// integer 本体は [`parse_counter_property`](super::content::parse_counter_property) の `<integer>` 抽出と同じ
/// `expect_integer` 直接呼び出し — CSS Values 3 §4.2 "Integers: the
/// `<integer>` type" により符号付き (負値含む) を許容し、range 制限は無い。
pub(super) fn parse_z_index(input: &mut Parser<'_, '_>) -> Option<ZIndexValue> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(ZIndexValue::Auto);
    }
    input
        .try_parse(|i| i.expect_integer())
        .ok()
        .map(ZIndexValue::Integer)
}

/// `border-radius` の 1--4 個の circular `<length>` を四隅へ展開する。
///
/// CSS Backgrounds and Borders 3 §5 の shorthand expansion に従い、値は
/// top-left, top-right, bottom-right, bottom-left の順で解釈する。
/// percentage は受理するが、slash 以降の楕円形指定と負値は受理しない。
pub(super) fn parse_border_radius(input: &mut Parser<'_, '_>) -> Option<BorderRadius> {
    let first = parse_non_negative_length_percentage(input)?;
    let second = input
        .try_parse(parse_non_negative_length_percentage_res)
        .ok();
    let third = input
        .try_parse(parse_non_negative_length_percentage_res)
        .ok();
    let fourth = input
        .try_parse(parse_non_negative_length_percentage_res)
        .ok();

    Some(match (second, third, fourth) {
        (None, _, _) => BorderRadius {
            top_left: first,
            top_right: first,
            bottom_right: first,
            bottom_left: first,
        },
        (Some(opposite), None, _) => BorderRadius {
            top_left: first,
            top_right: opposite,
            bottom_right: first,
            bottom_left: opposite,
        },
        (Some(horizontal), Some(bottom), None) => BorderRadius {
            top_left: first,
            top_right: horizontal,
            bottom_right: bottom,
            bottom_left: horizontal,
        },
        (Some(top_right), Some(bottom_right), Some(bottom_left)) => BorderRadius {
            top_left: first,
            top_right,
            bottom_right,
            bottom_left,
        },
    })
}

pub(super) fn parse_non_negative_length_percentage(input: &mut Parser<'_, '_>) -> Option<Length> {
    parse_non_negative_length_percentage_res(input).ok()
}

fn parse_length_allow_negative_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<Length, ParseError<'i, ()>> {
    parse_length_allow_negative(input).ok_or_else(|| input.new_custom_error(()))
}

/// `<length>{2,4}` の box-shadow length run を parse する。
///
/// offset-x/offset-y/spread-radius はいずれも sign 制限なしのため、
/// [`parse_shadow_length_reject_nan`]/`_res` 経由で `!is_nan()` guard を
/// 通す (同関数 doc 参照)。blur-radius (3rd slot) は既存の
/// `value.payload() >= 0.0` チェックが NaN も incidental に
/// 弾くため、追加 guard は不要 (`NaN >= 0.0` は IEEE 754 で `false`)。
pub(super) fn parse_box_shadow_lengths<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<(Length, Length, Length, Length), ParseError<'i, ()>> {
    let offset_x = parse_shadow_length_reject_nan_res(input)?;
    let offset_y = parse_shadow_length_reject_nan_res(input)?;
    // Parse the optional third slot without rewinding a negative length into
    // the fourth (spread) slot.  The grammar's third length is blur-radius,
    // which is non-negative; only the fourth spread-radius may be negative.
    let blur_radius = match input.try_parse(parse_length_allow_negative_res) {
        Ok(value) if value.payload() >= 0.0 => value,
        Ok(_) => return Err(input.new_custom_error(())),
        Err(_) => Length::Px(0.0),
    };
    let spread_radius = input
        .try_parse(parse_shadow_length_reject_nan_res)
        .unwrap_or(Length::Px(0.0));
    Ok((offset_x, offset_y, blur_radius, spread_radius))
}

fn parse_box_shadow_item(input: &mut Parser<'_, '_>) -> Option<BoxShadowItem> {
    let mut lengths: Option<(Length, Length, Length, Length)> = None;
    let mut color: Option<TextShadowColor> = None;
    let mut inset = false;

    // The optional color and `inset` keyword may occur before or after the
    // required length run.
    loop {
        let mut progressed = false;
        if lengths.is_none()
            && let Ok(value) = input.try_parse(parse_box_shadow_lengths)
        {
            lengths = Some(value);
            progressed = true;
        }
        if color.is_none()
            && let Ok(value) = input.try_parse(|i| -> Result<TextShadowColor, ParseError<'_, ()>> {
                parse_text_shadow_color(i).ok_or_else(|| i.new_custom_error(()))
            })
        {
            color = Some(value);
            progressed = true;
        }
        if !inset
            && input
                .try_parse(|i| i.expect_ident_matching("inset"))
                .is_ok()
        {
            inset = true;
            progressed = true;
        }
        if !progressed {
            break;
        }
    }

    let (offset_x, offset_y, blur_radius, spread_radius) = lengths?;
    Some(BoxShadowItem {
        offset_x,
        offset_y,
        blur_radius,
        spread_radius,
        color: color.unwrap_or(TextShadowColor::CurrentColor),
        inset,
    })
}

/// `box-shadow: none | <shadow>#` を parse する。
pub(super) fn parse_box_shadow(input: &mut Parser<'_, '_>) -> Option<Vec<BoxShadowItem>> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(Vec::new());
    }
    input
        .parse_comma_separated(|i| -> Result<BoxShadowItem, ParseError<'_, ()>> {
            parse_box_shadow_item(i).ok_or_else(|| i.new_custom_error(()))
        })
        .ok()
}

/// `outline` shorthand の width/style/color components を any-order で parse する。
pub(super) fn parse_outline(input: &mut Parser<'_, '_>) -> Option<Outline> {
    let mut width: Option<Length> = None;
    let mut style: Option<OutlineStyle> = None;
    let mut color: Option<OutlineColor> = None;

    loop {
        if width.is_some() && style.is_some() && color.is_some() {
            break;
        }
        if width.is_none()
            && let Ok(value) = input.try_parse(parse_border_width_side_res)
        {
            width = Some(value);
            continue;
        }
        if style.is_none()
            && let Ok(value) = input.try_parse(parse_outline_style_res)
        {
            style = Some(value);
            continue;
        }
        if color.is_none()
            && let Ok(value) = input.try_parse(|i| -> Result<OutlineColor, ParseError<'_, ()>> {
                parse_outline_color(i).ok_or_else(|| i.new_custom_error(()))
            })
        {
            color = Some(value);
            continue;
        }
        break;
    }

    if width.is_none() && style.is_none() && color.is_none() {
        return None;
    }
    Some(Outline {
        width: width.unwrap_or(Length::Px(BORDER_WIDTH_MEDIUM_PX)),
        style: style.unwrap_or(OutlineStyle::None),
        color: color.unwrap_or(OutlineColor::Invert),
    })
}

/// `outline-color` の value parser。CSS UI 3 §4.4 の `invert | <color>` を受理し、
/// `currentcolor` は `<color>` の keyword として専用 variant に保持する。
pub(super) fn parse_outline_color(input: &mut Parser<'_, '_>) -> Option<OutlineColor> {
    if input
        .try_parse(|i| i.expect_ident_matching("invert"))
        .is_ok()
    {
        return Some(OutlineColor::Invert);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("currentcolor"))
        .is_ok()
    {
        return Some(OutlineColor::CurrentColor);
    }
    parse_color(input).map(OutlineColor::Resolved)
}

/// Parse one `outline-style` keyword. The outline shorthand and longhand share
/// this helper so `auto` cannot accidentally become valid for border styles.
pub(super) fn parse_outline_style_side(input: &mut Parser<'_, '_>) -> Option<OutlineStyle> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "none" => Some(OutlineStyle::None),
        // Keep the existing `outline: hidden` rejection. The enum retains the
        // keyword for representation completeness, but this parser scope does
        // not accept it.
        "hidden" => None,
        "dotted" => Some(OutlineStyle::Dotted),
        "dashed" => Some(OutlineStyle::Dashed),
        "solid" => Some(OutlineStyle::Solid),
        "double" => Some(OutlineStyle::Double),
        "groove" => Some(OutlineStyle::Groove),
        "ridge" => Some(OutlineStyle::Ridge),
        "inset" => Some(OutlineStyle::Inset),
        "outset" => Some(OutlineStyle::Outset),
        "auto" => Some(OutlineStyle::Auto),
        _ => None,
    }
}

fn parse_outline_style_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<OutlineStyle, ParseError<'i, ()>> {
    parse_outline_style_side(input).ok_or_else(|| input.new_custom_error(()))
}
