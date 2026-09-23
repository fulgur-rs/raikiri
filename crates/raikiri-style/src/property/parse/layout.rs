//! Layout property parsers: display, float/clear, flexbox, grid, alignment,
//! multi-column, fragmentation breaks, tables and lists.

use std::sync::Arc;

use cssparser::{ParseError, Parser, Token};
use smol_str::SmolStr;

use crate::property::types::*;

use super::common::*;

/// `flex-direction: row | row-reverse | column | column-reverse` を parse
/// する (CSS Flexible Box Layout Module Level 1 §5.1
/// <https://www.w3.org/TR/css-flexbox-1/#flex-direction-property>)。
/// [`parse_display`] と同じ single-ident ASCII case-insensitive idiom。
pub(super) fn parse_flex_direction(input: &mut Parser<'_, '_>) -> Option<FlexDirectionValue> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "row" => Some(FlexDirectionValue::Row),
        "row-reverse" => Some(FlexDirectionValue::RowReverse),
        "column" => Some(FlexDirectionValue::Column),
        "column-reverse" => Some(FlexDirectionValue::ColumnReverse),
        _ => None,
    }
}

/// `flex-wrap: nowrap | wrap | wrap-reverse` を parse する (CSS Flexible Box
/// Layout Module Level 1 §5.2
/// <https://www.w3.org/TR/css-flexbox-1/#flex-wrap-property>)。
pub(super) fn parse_flex_wrap(input: &mut Parser<'_, '_>) -> Option<FlexWrapValue> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "nowrap" => Some(FlexWrapValue::NoWrap),
        "wrap" => Some(FlexWrapValue::Wrap),
        "wrap-reverse" => Some(FlexWrapValue::WrapReverse),
        _ => None,
    }
}

/// `<number [0,∞]>` を parse する — `flex-grow` / `flex-shrink` 共有 helper。
///
/// # Non-negative **と** finite の両方を parse 時に enforce する
///
/// [`parse_line_height`](super::text::parse_line_height) の `<number>` branch と同じ `[0,∞]` non-negative
/// check に加え、本 helper は **finiteness も** enforce する
/// ([`PropertyValue::FlexGrow`] doc 参照)。これは本 crate の他の length 系
/// helper (`parse_length_value` 等) とは非対称な判断で、理由は明示しておく:
/// `padding`/`width` 等の length は raikiri-dom 側の
/// `sanitize_taffy`/`sanitize_taffy_layout` という **sink 境界の guard** を
/// 必ず経由してから taffy に渡る (「guard は sink 境界に置く」という本 crate
/// 全体の設計方針、`crates/raikiri-dom/src/layout.rs` の非有限 f32 guard 節
/// 参照) ため parse 層では素通しでよい。一方 `flex-grow`/`flex-shrink` は
/// raikiri-dom の `bridge_flex` が `taffy::Style::flex_grow`/`flex_shrink`
/// (共に生 `f32`) へ **無変換で直接 copy する** — 途中に絶対化/sink guard の
/// 通過点が無いため、`+Inf` を防ぐ唯一の場所が本 parse-time check になる。
/// `<number [0,∞]>` 自体の f64→f32 変換 (cssparser tokenizer 側) は巨大な
/// literal (`flex-grow: 1e40`) で `+Inf` を produce しうる。`NaN` についても
/// 同じ `is_finite()` check が引き続き弾くが、`expect_number_stable`
/// (module doc「Numeric-token NaN stabilization」節参照) が zero-mantissa
/// huge-exponent literal (`flex-grow: 0e999`) 由来の `NaN` を acquisition
/// 時点で既に訂正するため、通常の parse ではこの check が `NaN` を実際に
/// 弾く場面はもう無い — `+Inf` に対する必須の check という位置づけ。
pub(crate) fn parse_nonneg_finite_number(input: &mut Parser<'_, '_>) -> Option<f32> {
    let n = expect_number_stable(input).ok()?;
    (n.is_finite() && n >= 0.0).then_some(n)
}

/// [`parse_nonneg_finite_number`] の `Result` 版 — `try_parse` closure 用
/// ([`parse_padding_side_res`](super::box_model::parse_padding_side_res) と同じ wrapper pattern)。range/finite check
/// の失敗も `Err` として返すため、`try_parse` が呼び出し側で自動的に
/// rewind する (捕捉した Number token を別 branch へ fall through させない
/// — [`parse_line_height`](super::text::parse_line_height) doc の「Number token を commit した後は必ず
/// ここで確定させる」節と同じ懸念を、check 自体を closure 内に置くことで
/// 構造的に回避する)。
fn parse_nonneg_finite_number_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<f32, ParseError<'i, ()>> {
    parse_nonneg_finite_number(input).ok_or_else(|| input.new_custom_error(()))
}

/// `flex-basis: content | <'width'>` を parse する (CSS Flexible Box Layout
/// Module Level 1 §7.2.3
/// <https://www.w3.org/TR/css-flexbox-1/#flex-basis-property>)。
///
/// [`parse_width`](super::box_model::parse_width) と同じ 3-branch shape (`auto` → `content` →
/// `<length-percentage [0,∞]>`) に `content` branch を追加したもの —
/// [`FlexBasisValue`] doc 参照。
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
    // `<'width'>` reuse: CSS Sizing 3 §3.1.1 の `[0,∞]` non-negative
    // constraint (`parse_width` と同 pattern)。
    (length.payload() >= 0.0).then_some(FlexBasisValue::Length(length))
}

/// [`parse_flex_basis`] の `Result` 版 ([`parse_padding_side_res`](super::box_model::parse_padding_side_res) と同じ
/// wrapper pattern、[`parse_flex_shorthand`] の `try_parse` 用)。
fn parse_flex_basis_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<FlexBasisValue, ParseError<'i, ()>> {
    parse_flex_basis(input).ok_or_else(|| input.new_custom_error(()))
}

/// `flex: none | [ <'flex-grow'> <'flex-shrink'>? || <'flex-basis'> ]` を
/// parse する (CSS Flexible Box Layout Module Level 1 §7.1 "The flex
/// Shorthand" <https://www.w3.org/TR/css-flexbox-1/#flex-property>)。
///
/// [`FlexShorthand`] doc の "Omitted-component defaults" 節が説明する
/// shorthand-local default (grow=1 / shrink=1 / basis=0px、longhand 自身の
/// initial とは異なる) をここで適用する。
///
/// # `none` — exclusive keyword
///
/// spec §7.1 "The keyword none expands to 0 0 auto." — 他の component と
/// 共存しない (grammar top-level alternative)。
///
/// # Component order と unitless-zero ambiguity
///
/// grammar は `<flex-grow> <flex-shrink>?` の group と `<flex-basis>` の 2
/// group を `||` (any order、どちらか 1 つ以上) で combine する。本関数は
/// 最大 3 回のループで両 group を試す —
/// **`<flex-grow>` group を毎回先に試す**ことで、spec 本文の以下の
/// disambiguation 規則をそのまま実現する (verbatim):
///
/// > A unitless zero that is not already preceded by two flex factors must
/// > be interpreted as a flex factor. To avoid misinterpretation or invalid
/// > declarations, authors must specify a zero `<'flex-basis'>` component
/// > with a unit or precede it by two flex factors.
///
/// `grow` が未確定な間は bare `0` を常に number (flex factor) として先取り
/// consume するため、`flex: 0` は `grow=0` (`<'flex-basis'>` ではない) に
/// なる。`grow`/`shrink` が両方確定した**後**の bare `0` は
/// [`parse_flex_basis`] の unitless-zero clause 経由で `flex-basis` として
/// 解釈される (`flex: 2 3 0` → `basis: Length(Px(0.0))`)。
///
/// `<flex-shrink>` は grammar 上 `<flex-grow>` に直接後続する成分であり
/// (独立した `||` alternative ではない)、本関数もそれに合わせて `grow` を
/// 得た**直後**にのみ `shrink` を試す。
///
/// 5 個目以降の leftover token は本関数では consume せず、[`parse_padding_shorthand`](super::box_model::parse_padding_shorthand)
/// 等と同じく caller (`rule.rs::DeclParser`) の `expect_exhausted` が
/// declaration ごと drop する。
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
            // `<flex-shrink>` は `<flex-grow>` に直接後続する成分 — 独立
            // alternative としては試さない (上記 doc 参照)。
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
    // grammar 上どちらかの group が最低 1 つは要る — 0-value form
    // (`flex:` に何も続かない) は invalid。
    if grow.is_none() && basis.is_none() {
        return None;
    }
    Some(FlexShorthand {
        // shorthand-local default (`FlexShorthand` doc 参照) — longhand
        // 自身の initial (grow=0 / basis=auto) とは異なる。
        grow: grow.unwrap_or(1.0),
        shrink: shrink.unwrap_or(1.0),
        basis: basis.unwrap_or(FlexBasisValue::Length(Length::Px(0.0))),
    })
}

/// `order: <integer>` を parse する (CSS Flexible Box Layout Module Level 1
/// §4.2 "Display Order: the order property"
/// <https://www.w3.org/TR/css-flexbox-1/#order-property>、
/// [`PropertyValue::Order`] doc 参照)。
///
/// `expect_integer` 直接呼び出し — [`parse_z_index`](super::box_model::parse_z_index) と同じ pattern。spec
/// value grammar は `<integer>` のみ (符号付き、range 制限なし)。
pub(super) fn parse_order(input: &mut Parser<'_, '_>) -> Option<i32> {
    input.try_parse(|i| i.expect_integer()).ok()
}

/// [`parse_flex_direction`] の `Result` 版 ([`parse_padding_side_res`](super::box_model::parse_padding_side_res) と
/// 同じ wrapper pattern、[`parse_flex_flow`] の `try_parse` 用)。
fn parse_flex_direction_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<FlexDirectionValue, ParseError<'i, ()>> {
    parse_flex_direction(input).ok_or_else(|| input.new_custom_error(()))
}

/// [`parse_flex_wrap`] の `Result` 版 ([`parse_flex_direction_res`] と同じ
/// wrapper pattern、[`parse_flex_flow`] の `try_parse` 用)。
fn parse_flex_wrap_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<FlexWrapValue, ParseError<'i, ()>> {
    parse_flex_wrap(input).ok_or_else(|| input.new_custom_error(()))
}

/// `flex-flow: <'flex-direction'> || <'flex-wrap'>` を parse する (CSS
/// Flexible Box Layout Module Level 1 §5.3 "Flex Direction and Wrap: the
/// flex-flow shorthand"
/// <https://www.w3.org/TR/css-flexbox-1/#flex-flow-property>、
/// [`FlexFlow`] doc 参照)。
///
/// `||` (any-order、each component at most once、at least 1 必須) —
/// 最大 2 回のループで両 component を試す。省略成分は対応 longhand の
/// initial (direction=row / wrap=nowrap) を適用する。
/// leftover token は本関数では consume せず、caller
/// (`rule.rs::DeclParser`) の `expect_exhausted` が declaration ごと drop
/// する ([`parse_flex_shorthand`] と同じ contract)。
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

/// `justify-content` / `align-content` 共有 parser
/// ([`ContentAlignmentValue`] doc の scope carving 節参照 — `safe`/`unsafe`
/// prefix、`<baseline-position>`、justify-content 独自の `left`/`right` は
/// 単一 ident しか consume しない本関数の shape 上、自然に unmatched (2-token
/// 列や非対応 ident は `_ => None`) になる)。
pub(super) fn parse_content_alignment(input: &mut Parser<'_, '_>) -> Option<ContentAlignmentValue> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "normal" => Some(ContentAlignmentValue::Normal),
        "stretch" => Some(ContentAlignmentValue::Stretch),
        "space-between" => Some(ContentAlignmentValue::SpaceBetween),
        "space-evenly" => Some(ContentAlignmentValue::SpaceEvenly),
        "space-around" => Some(ContentAlignmentValue::SpaceAround),
        "center" => Some(ContentAlignmentValue::Center),
        "start" => Some(ContentAlignmentValue::Start),
        "end" => Some(ContentAlignmentValue::End),
        "flex-start" => Some(ContentAlignmentValue::FlexStart),
        "flex-end" => Some(ContentAlignmentValue::FlexEnd),
        _ => None,
    }
}

/// [`parse_content_alignment`] の `Result` 版 ([`parse_padding_side_res`](super::box_model::parse_padding_side_res) と
/// 同じ wrapper pattern、[`parse_place_content_shorthand`] の `try_parse` 用)。
fn parse_content_alignment_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<ContentAlignmentValue, ParseError<'i, ()>> {
    parse_content_alignment(input).ok_or_else(|| input.new_custom_error(()))
}

/// `align-items` parser ([`SelfAlignmentValue`] doc の scope carving 節
/// 参照)。`align-self` (`auto` を追加で受理する) は [`parse_align_self`] が
/// 本関数を再利用する。
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

/// [`parse_self_alignment`] の `Result` 版 ([`parse_padding_side_res`](super::box_model::parse_padding_side_res) と
/// 同じ wrapper pattern、[`parse_place_items_shorthand`] の `try_parse` 用)。
fn parse_self_alignment_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<SelfAlignmentValue, ParseError<'i, ()>> {
    parse_self_alignment(input).ok_or_else(|| input.new_custom_error(()))
}

/// `align-self: auto | …` parser — `auto` branch を先に試し
/// (`try_parse` checkpoint、[`parse_width`](super::box_model::parse_width) の "Order of alternatives" 節と
/// 同じ rationale)、それ以外は [`parse_self_alignment`] (= `align-items` と
/// 同じ keyword set) に delegate する。
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

/// [`parse_align_self`] の `Result` 版 ([`parse_padding_side_res`](super::box_model::parse_padding_side_res) と同じ
/// wrapper pattern、[`parse_place_self_shorthand`] の `try_parse` 用)。
fn parse_align_self_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<AlignSelfValue, ParseError<'i, ()>> {
    parse_align_self(input).ok_or_else(|| input.new_custom_error(()))
}

/// `row-gap` / `column-gap`: `normal | <length-percentage [0,∞]>` 共有
/// parser (CSS Box Alignment Module Level 3 §8.1
/// <https://www.w3.org/TR/css-align-3/#propdef-row-gap>)。
///
/// [`parse_letter_or_word_spacing`](super::text::parse_letter_or_word_spacing) と shape は同じ (`normal` branch →
/// length branch) だが、**percentage を受理する**点が異なる
/// (`allow_percentage = true`、letter-spacing/word-spacing は "Percentages:
/// N/A" だが gap は `<length-percentage>`)。non-negative constraint は
/// [`parse_padding_side`](super::box_model::parse_padding_side) と同 pattern。
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

/// [`parse_gap_value`] の `Result` 版 ([`parse_padding_side_res`](super::box_model::parse_padding_side_res) と同じ
/// wrapper pattern、[`parse_gap_shorthand`] の `try_parse` 用)。
fn parse_gap_value_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<LengthOrNormal, ParseError<'i, ()>> {
    parse_gap_value(input).ok_or_else(|| input.new_custom_error(()))
}

/// `gap: <'row-gap'> <'column-gap'>?` shorthand を parse する (CSS Box
/// Alignment Module Level 3 §8.2
/// <https://www.w3.org/TR/css-align-3/#propdef-gap>)。第 2 成分省略時は
/// spec 本文通り第 1 成分をそのまま copy する ([`GapShorthand`] doc 参照)。
pub(crate) fn parse_gap_shorthand(input: &mut Parser<'_, '_>) -> Option<GapShorthand> {
    let row = parse_gap_value(input)?;
    let column = input.try_parse(parse_gap_value_res).unwrap_or(row);
    Some(GapShorthand { row, column })
}

/// `place-content: <'align-content'> <'justify-content'>?` shorthand を
/// parse する (CSS Box Alignment Module Level 3 §5.2
/// <https://www.w3.org/TR/css-align-3/#propdef-place-content>)。第 2 成分
/// 省略時は spec 本文通り第 1 成分をそのまま copy する
/// ([`PlaceContentShorthand`] doc 参照 — `<baseline-position>` 例外分岐は
/// [`ContentAlignmentValue`] が同 variant を持たないため本 crate では
/// 到達不能)。
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

/// `<custom-ident>` 除外リスト — [`GridLineValue`] 系 production と
/// `<line-names>` (§7.2.2) の両方で使う ([`is_reserved_custom_ident`] に
/// `span` / `auto` を追加除外)。
///
/// CSS Grid Layout Module Level 1 §8.3 verbatim: "In all the above
/// productions, the `<custom-ident>` additionally excludes the keywords
/// `span` and `auto`"、および §7.2.2 verbatim: "A line name cannot be span
/// or auto, i.e. the `<custom-ident>` in the `<line-names>` production
/// excludes the keywords span and auto." — §7.2.2 自身がこの除外を明示的に
/// `<line-names>` production に適用すると述べている ([`is_reserved_counter_name`](super::content::is_reserved_counter_name)
/// が base list に `none` を足す precedent と同じ pattern)。
pub(crate) fn is_reserved_grid_line_name(ident: &str) -> bool {
    is_reserved_custom_ident(ident)
        || matches!(ident.to_ascii_lowercase().as_str(), "span" | "auto")
}

/// [`GridLineValue`] 系 production 内の `<custom-ident>` を parse する
/// ([`is_reserved_grid_line_name`] の追加除外を適用する点のみ
/// [`parse_custom_ident`] と異なる)。
fn parse_grid_custom_ident(input: &mut Parser<'_, '_>) -> Option<SmolStr> {
    let ident = input.expect_ident().ok()?.clone();
    if is_reserved_grid_line_name(&ident) {
        None
    } else {
        Some(SmolStr::new(ident.as_ref()))
    }
}

/// [`parse_grid_custom_ident`] の `Result` 版 ([`parse_padding_side_res`](super::box_model::parse_padding_side_res) と
/// 同じ wrapper pattern、`&&`/`||` combinator 内の `try_parse` 用)。
fn parse_grid_custom_ident_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<SmolStr, ParseError<'i, ()>> {
    parse_grid_custom_ident(input).ok_or_else(|| input.new_custom_error(()))
}

/// `<line-names> = '[' <custom-ident>* ']'` (CSS Grid Layout Module Level 1
/// §7.2.2 "Naming Grid Lines: the `[<custom-ident>*]` syntax"
/// <https://www.w3.org/TR/css-grid-1/#named-lines>) を parse する。
///
/// `<line-names>` の `<custom-ident>` は [`is_reserved_grid_line_name`] の
/// 追加除外 (`span`/`auto`) を**受ける** — §7.2.2 verbatim: "A line name
/// cannot be span or auto, i.e. the `<custom-ident>` in the `<line-names>`
/// production excludes the keywords span and auto." (この除外は §8.3 の
/// `<grid-line>` production 群だけでなく、§7.2.2 自身が明示する)。
///
/// `[` が見つからなければ `None` (呼び出し元は
/// [`parse_line_names_or_empty`] 経由で空 `Vec` へ fallback)。空 `[]` は
/// `Some(vec![])`。
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

/// [`GridTrackList::line_names`] / [`GridTrackRepeat::line_names`] の各
/// interleave slot を埋める helper — `<line-names>?` (省略可) を空 `Vec` に
/// 正規化する。
fn parse_line_names_or_empty(input: &mut Parser<'_, '_>) -> Vec<SmolStr> {
    parse_line_names(input).unwrap_or_default()
}

/// `<flex [0,∞]>` — the `fr` unit (CSS Grid Layout Module Level 1 §7.2.4
/// "Flexible Lengths: the fr unit"
/// <https://www.w3.org/TR/css-grid-1/#fr-unit>) を parse する。
///
/// non-negative **と** finite を parse 時に enforce する —
/// [`parse_nonneg_finite_number`] doc と同じ理由 (raikiri-dom bridge が
/// taffy `MaxTrackSizingFunction::fr` へ無変換で copy する、sink guard を
/// 経由しない経路)。
///
/// token 取得は `next_numeric_stable` 経由 (module doc 冒頭「Numeric-token
/// NaN stabilization」節参照) — zero-mantissa/huge-exponent literal
/// (`grid-template-columns: 0e999fr`) を cssparser tokenizer が `NaN` に
/// collapse する artifact をここで訂正済のため、spec-correct な `Flex(0.0)`
/// に解決される。
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

/// `<track-breadth> = <length-percentage [0,∞]> | <flex [0,∞]> | min-content
/// | max-content | auto` を parse する (CSS Grid Layout Module Level 1 §7.2.1
/// "Track Sizes"
/// <https://www.w3.org/TR/css-grid-1/#valdef-grid-template-columns-track-breadth>)。
///
/// `<length-percentage>` branch は必ず最後に試す —
/// [`parse_length_value`] は失敗時も token を consume する ([`parse_flex_basis`]
/// doc の同 rationale 参照) ため、それ以降に別 alternative を試せない。
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

/// `<inflexible-breadth> = <length-percentage [0,∞]> | min-content |
/// max-content | auto` を parse する (CSS Grid Layout Module Level 1 §7.2.1
/// <https://www.w3.org/TR/css-grid-1/#valdef-grid-template-columns-inflexible-breadth>)。
/// [`parse_track_breadth`] と同型だが `<flex>` branch を持たない。
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

/// `minmax( <inflexible-breadth>, <track-breadth> )` を parse する (CSS Grid
/// Layout Module Level 1 §7.2.1
/// <https://www.w3.org/TR/css-grid-1/#funcdef-grid-template-columns-minmax>)。
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

/// `fit-content( <length-percentage [0,∞]> )` を parse する (CSS Grid Layout
/// Module Level 1 §7.2.1
/// <https://www.w3.org/TR/css-grid-1/#funcdef-grid-template-columns-fit-content>)。
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

/// `<track-size>` を parse する ([`GridTrackSize`] doc 参照)。3 alternative
/// を順に試す (`minmax()` → `fit-content()` → bare `<track-breadth>`) —
/// 前 2 つは固有 function name で判別できるため順序に意味は薄いが、
/// bare `<track-breadth>` は必ず最後 ([`parse_track_breadth`] doc の
/// consume-on-failure 注記参照)。
fn parse_track_size(input: &mut Parser<'_, '_>) -> Option<GridTrackSize> {
    if let Ok((min, max)) = input.try_parse(parse_grid_minmax_res) {
        return Some(GridTrackSize::MinMax(min, max));
    }
    if let Ok(limit) = input.try_parse(parse_grid_fit_content_res) {
        return Some(GridTrackSize::FitContent(limit));
    }
    parse_track_breadth(input).map(GridTrackSize::Breadth)
}

/// [`GridTrackBreadth`] が `<fixed-breadth>` (= `<length-percentage>`のみ)
/// かどうか — [`grid_track_size_is_fixed`] の component。
fn grid_track_breadth_is_fixed(b: &GridTrackBreadth) -> bool {
    matches!(b, GridTrackBreadth::Length(_))
}

/// [`GridInflexibleBreadth`] が `<fixed-breadth>` かどうか —
/// [`grid_track_size_is_fixed`] の component。
fn grid_inflexible_breadth_is_fixed(b: &GridInflexibleBreadth) -> bool {
    matches!(b, GridInflexibleBreadth::Length(_))
}

/// [`GridTrackSize`] が `<fixed-size>` 制約 (CSS Grid Layout Module Level 1
/// §7.2.1 `<fixed-size> = <fixed-breadth> | minmax( <fixed-breadth>,
/// <track-breadth> ) | minmax( <inflexible-breadth>, <fixed-breadth> )`) を
/// 満たすかどうか — [`GridTrackSize`] doc の "fixed-size 制約" 節参照。
///
/// `minmax()` は min/max のどちらか一方が `<fixed-breadth>` であれば足りる
/// (spec 上両方が fixed である必要はない — `minmax(100px, 1fr)` は valid
/// `<fixed-size>`)。`fit-content()` は `<fixed-size>` の grammar に
/// alternative が無いため常に `false`。
pub(crate) fn grid_track_size_is_fixed(t: &GridTrackSize) -> bool {
    match t {
        GridTrackSize::Breadth(b) => grid_track_breadth_is_fixed(b),
        GridTrackSize::MinMax(min, max) => {
            grid_inflexible_breadth_is_fixed(min) || grid_track_breadth_is_fixed(max)
        }
        GridTrackSize::FitContent(_) => false,
    }
}

/// `repeat()` の repetition count (`<integer [1,∞]>` / `auto-fill` /
/// `auto-fit`) を parse する ([`GridRepeatCount`] doc 参照)。
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

/// `repeat( <count>, [ <line-names>? <track-size> ]+ <line-names>? )` を
/// parse する ([`GridTrackRepeat`] doc 参照)。count が
/// [`GridRepeatCount::Count`] か [`GridRepeatCount::AutoFill`]/
/// [`GridRepeatCount::AutoFit`] かで inner track が `<track-size>` /
/// `<fixed-size>` のどちらの grammar に従うべきかが変わる
/// ([`GridTrackRepeat`] doc の "許可される count と `<fixed-size>` 制約"
/// 節参照) が、本関数は grammar 上共通の shape (full `<track-size>`) で
/// parse し、`<fixed-size>` 制約は [`parse_grid_template_tracks`] が
/// track list 全体を見て post-validate する。
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

/// [`parse_grid_repeat_res`] の `Option` 版 —
/// [`parse_grid_track_list`] の alternation 用。
pub(crate) fn parse_grid_repeat(input: &mut Parser<'_, '_>) -> Option<GridTrackRepeat> {
    input.try_parse(parse_grid_repeat_res).ok()
}

/// `<track-list>` / `<auto-track-list>` の component 列 (`[ <line-names>?
/// [ <track-size> | <track-repeat> ] ]+ <line-names>?`) を parse する。
/// `<fixed-size>` 制約の検査は行わない ([`grid_track_list_obeys_auto_repeat_constraint`]
/// が呼び出し元 [`parse_grid_template_tracks`] で担う)。
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

/// [`parse_grid_track_list`] が返した [`GridTrackList`] が spec の 2 制約を
/// 満たすかどうかを検査する (post-parse validation、
/// [`GridTrackRepeat`] doc の "許可される count と `<fixed-size>` 制約" 節参照):
///
/// 1. CSS Grid Layout Module Level 1 §7.2.3.1 verbatim: "It can only appear
///    once in the track list" — auto-fill/auto-fit repeat は track list 中
///    に高々 1 つ。
/// 2. §7.2.3.1 verbatim: "Automatic repetitions (auto-fill or auto-fit)
///    cannot be combined with fully intrinsic or flexible sizes" —
///    auto-repeat が存在する場合、track list 中の**他の全 track**
///    (bare track と他の repeat() の中身の両方、auto-repeat 自身の中身も
///    含む) が [`grid_track_size_is_fixed`] を満たす必要がある。
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

/// `grid-template-columns` / `grid-template-rows`: `none | <track-list> |
/// <auto-track-list>` を parse する (CSS Grid Layout Module Level 1 §7.2
/// <https://www.w3.org/TR/css-grid-1/#track-sizing>、[`GridTemplateTracks`]
/// doc 参照)。
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

/// `grid-auto-columns` / `grid-auto-rows`: `<track-size>+` を parse する
/// (CSS Grid Layout Module Level 1 §7.6
/// <https://www.w3.org/TR/css-grid-1/#propdef-grid-auto-columns>)。
/// `<track-list>` と異なり `repeat()` も `<line-names>` interleaving も
/// grammar に含まれない — bare `<track-size>` の並びのみ。
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

/// `grid-auto-flow: [ row | column ] || dense` を parse する (CSS Grid
/// Layout Module Level 1 §7.7
/// <https://www.w3.org/TR/css-grid-1/#propdef-grid-auto-flow>)。
///
/// `dense` 単独 (axis 省略) は [`GridAutoFlowValue::RowDense`] に畳む —
/// axis 省略時の default が `row` であるため ([`GridAutoFlowValue`] doc の
/// 同型注記参照)。両 group とも順序自由 (`||`) なので最大 2 回のループで
/// 両方を試す。
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

/// `span <integer [1,∞]> || <custom-ident>` (`span` ident は呼び出し元が
/// 既に consume 済み) を parse する — [`parse_grid_line`] の tail helper。
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
        // `span` の直後に何も続かない — grammar 上 `span &&
        // [ <integer> || <custom-ident> ]` の右辺 group が必須のため invalid。
        (None, None) => None,
    }
}

/// `<grid-line>` を parse する ([`GridLineValue`] doc 参照)。
///
/// `[ [ <integer> ] && <custom-ident>? ]` alternative は order-free
/// (`&&`) なので、`<integer>` を最大 2 回のループで先に/後に両方試す —
/// **`<integer>` が一度も現れなければ** (`number.is_none()`)、それは
/// この alternative ではなく別の top-level alternative `<custom-ident>`
/// (単独) にマッチしたことを意味し、[`GridLineValue::Named`]
/// (bare-ident、shorthand omission-copy 規則の対象) を返す —
/// [`GridLineValue::NamedLine`] (`<integer>` 併記、対象外) とは
/// [`GridLineShorthand`] doc の spec verbatim 引用が要求する区別
/// ([`parse_grid_line_shorthand`] 参照)。
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
        // `0` は spec verbatim "Negative integers or zero are invalid" —
        // named の有無に関わらず reject。
        (Some(0), _) => None,
        (Some(n), Some(name)) => Some(GridLineValue::NamedLine(name, n)),
        (Some(n), None) => Some(GridLineValue::Line(n)),
        (None, Some(name)) => Some(GridLineValue::Named(name)),
        (None, None) => None,
    }
}

/// `grid-row` / `grid-column`: `<grid-line> [ / <grid-line> ]?` shorthand を
/// parse する ([`GridLineShorthand`] doc 参照)。
pub(crate) fn parse_grid_line_shorthand(input: &mut Parser<'_, '_>) -> Option<GridLineShorthand> {
    let start = parse_grid_line(input)?;
    if input.try_parse(|i| i.expect_delim('/')).is_ok() {
        let end = parse_grid_line(input)?;
        return Some(GridLineShorthand { start, end });
    }
    // spec 本文 verbatim (`GridLineShorthand` doc 引用): 第 2 成分省略時、
    // 第 1 成分が `<custom-ident>` (= `GridLineValue::Named`、`<integer>`
    // 併記なしの bare 形のみ) なら第 2 成分にもその名前を copy、それ以外は
    // `auto`。
    let end = match &start {
        GridLineValue::Named(name) => GridLineValue::Named(name.clone()),
        _ => GridLineValue::Auto,
    };
    Some(GridLineShorthand { start, end })
}

/// `grid-template-areas` の 1 `<string>` を cell token 列へ分解する。
///
/// CSS Grid Layout Module Level 1 §7.3 verbatim tokenization 規則
/// (<https://www.w3.org/TR/css-grid-1/#grid-template-areas-property>):
/// "Tokenize the string into a list of the following tokens, using
/// longest-match semantics": ident code point の並び (named cell) / `.` の
/// 並び (null cell) / whitespace (無視、トークン化されない) / それ以外
/// (trash token → invalid)。
///
/// `None` を返すのは trash token を検出した場合のみ (spec verbatim: "A
/// trash token is a syntax error, and makes the declaration invalid.")。
///
/// whitespace 判定は `char::is_whitespace()` (Unicode `White_Space`
/// property、`U+3000` 等の非 ASCII whitespace も含む) ではなく CSS Syntax 3
/// の whitespace 定義 (<https://www.w3.org/TR/css-syntax-3/#whitespace>)
/// verbatim: "A newline, U+0009 CHARACTER TABULATION, or U+0020 SPACE" を
/// 直接使う — newline は同 spec の input preprocessing
/// (<https://www.w3.org/TR/css-syntax-3/#input-preprocessing>) で
/// U+000D/U+000C が U+000A に正規化された後の定義なので、ここでは `'\n'`
/// のみを見ればよい (CR/FF は stylesheet 全体の tokenize 前処理で
/// 消えている前提)。
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

/// CSS Syntax 3 の "ident code point" (letter / digit / `-` / `_` /
/// non-ASCII) を近似する classifier — [`tokenize_grid_area_row`] の
/// named-cell token 分解専用。escape sequence (`\XX`) は考慮しない —
/// `<string>` token の value は cssparser が既に unescape した literal
/// character 列であり、`grid-template-areas` の string tokenization
/// (§7.3) 自体は再度 CSS syntax としての escape 解釈を行わない (spec の
/// 定義がそのまま code point 単位の分類であるため)。
fn is_grid_area_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-' || !c.is_ascii()
}

/// [`tokenize_grid_area_row`] 済みの row 列から [`GridTemplateAreas`] を
/// 構築する — named area の bounding box を算出し、spec §7.3 verbatim の
/// "If a named grid area spans multiple grid cells, but those cells do not
/// form a single filled-in rectangle, the declaration is invalid." を検査
/// する。
///
/// `rows` は呼び出し元 ([`parse_grid_template_areas`]) が非空を保証する
/// (空なら呼び出し元が先に `None` を返す)。
fn build_grid_template_areas(
    rows: Vec<Vec<Option<SmolStr>>>,
    row_strings: Vec<SmolStr>,
) -> Option<GridTemplateAreas> {
    let row_count = rows.len();
    let column_count = rows[0].len();
    // spec 本文 verbatim: "All strings must define the same number of cell
    // tokens ... and at least one cell token, or else the declaration is
    // invalid."
    if column_count == 0 || rows.iter().any(|r| r.len() != column_count) {
        return None;
    }
    // (name, row_min, row_max, col_min, col_max) — 初出順、線形 scan
    // (area 名の種類数は現実的に小さいため HashMap を持ち込まない)。
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

/// `grid-template-areas: none | <string>+` を parse する (CSS Grid Layout
/// Module Level 1 §7.3
/// <https://www.w3.org/TR/css-grid-1/#grid-template-areas-property>、
/// [`GridTemplateAreasValue`] doc 参照)。
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

/// `place-items: <'align-items'> <'justify-items'>?` shorthand を parse
/// する (CSS Box Alignment Module Level 3 §7.3
/// <https://www.w3.org/TR/css-align-3/#propdef-place-items>)。第 2 成分
/// 省略時は spec 本文通り第 1 成分をそのまま copy する
/// ([`PlaceItemsShorthand`] doc 参照)。
pub(crate) fn parse_place_items_shorthand(
    input: &mut Parser<'_, '_>,
) -> Option<PlaceItemsShorthand> {
    let align = parse_self_alignment(input)?;
    let justify = input.try_parse(parse_self_alignment_res).unwrap_or(align);
    Some(PlaceItemsShorthand { align, justify })
}

/// `place-self: <'align-self'> <'justify-self'>?` shorthand を parse する
/// (CSS Box Alignment Module Level 3 §6.3
/// <https://www.w3.org/TR/css-align-3/#propdef-place-self>)。第 2 成分
/// 省略時の copy 規則は [`parse_place_items_shorthand`] と同じ。
pub(crate) fn parse_place_self_shorthand(input: &mut Parser<'_, '_>) -> Option<PlaceSelfShorthand> {
    let align = parse_align_self(input)?;
    let justify = input.try_parse(parse_align_self_res).unwrap_or(align);
    Some(PlaceSelfShorthand { align, justify })
}

/// `float: <ident>` を parse する (CSS2 §9.5.1
/// <https://www.w3.org/TR/CSS2/visuren.html#propdef-float>, [`FloatValue`]
/// doc 参照)。
///
/// Value grammar: `left | right | none` (`inherit` は上記 "CSS-wide
/// keyword (canonical)" 節により未対応)。ASCII case-insensitive matching
/// は sibling `parse_word_break` と同 flavor。
pub(super) fn parse_float(input: &mut Parser<'_, '_>) -> Option<FloatValue> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "none" => Some(FloatValue::None),
        "left" => Some(FloatValue::Left),
        "right" => Some(FloatValue::Right),
        "inline-start" => Some(FloatValue::InlineStart),
        "inline-end" => Some(FloatValue::InlineEnd),
        "footnote" => Some(FloatValue::Footnote),
        _ => None,
    }
}

/// `clear: <ident>` を parse する (CSS2 §9.5.2
/// <https://www.w3.org/TR/CSS2/visuren.html#propdef-clear>, [`ClearValue`]
/// doc 参照)。
///
/// Value grammar: `none | left | right | both` (`inherit` は上記
/// "CSS-wide keyword (canonical)" 節により未対応)。ASCII case-insensitive
/// matching は sibling `parse_float` と同 flavor。
pub(super) fn parse_clear(input: &mut Parser<'_, '_>) -> Option<ClearValue> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "none" => Some(ClearValue::None),
        "left" => Some(ClearValue::Left),
        "right" => Some(ClearValue::Right),
        "both" => Some(ClearValue::Both),
        "inline-start" => Some(ClearValue::InlineStart),
        "inline-end" => Some(ClearValue::InlineEnd),
        _ => None,
    }
}

/// `table-layout: <ident>` を parse する (CSS Tables 3 §4
/// <https://www.w3.org/TR/css-tables-3/#table-layout-property>,
/// [`TableLayoutValue`] doc 参照)。
///
/// Value grammar: `auto | fixed`。ASCII case-insensitive matching は
/// sibling [`parse_unicode_bidi`](super::text::parse_unicode_bidi) と同 flavor、余剰 token
/// (`table-layout: auto fixed` 等) は caller (`rule.rs::DeclParser`) の
/// `expect_exhausted` が drop する。
pub(super) fn parse_table_layout(input: &mut Parser<'_, '_>) -> Option<TableLayoutValue> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "auto" => Some(TableLayoutValue::Auto),
        "fixed" => Some(TableLayoutValue::Fixed),
        _ => None,
    }
}

/// `border-collapse: <ident>` を parse する (CSS Tables 3 §6
/// <https://www.w3.org/TR/css-tables-3/#border-collapse-property>,
/// [`BorderCollapseValue`] doc 参照)。
///
/// Value grammar: `collapse | separate`。matching 規則は sibling
/// [`parse_table_layout`] と同じ。
pub(super) fn parse_border_collapse(input: &mut Parser<'_, '_>) -> Option<BorderCollapseValue> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "collapse" => Some(BorderCollapseValue::Collapse),
        "separate" => Some(BorderCollapseValue::Separate),
        _ => None,
    }
}

/// `border-spacing: <length>{1,2}` を parse する (CSS Tables 3 §6.1
/// <https://www.w3.org/TR/css-tables-3/#border-spacing-property>,
/// [`BorderSpacingValue`] doc 参照)。
///
/// 各成分は [`parse_non_negative_length`] (`<length [0,∞]>`、
/// `<percentage>` alternative なし — spec の Percentages: N/A と
/// "Negative lengths are illegal" を共に enforce)。unitless `0` は
/// [`parse_length_value`] の CSS Values 3 §5 unitless-zero clause で
/// `Px(0.0)` として受理される (WPT computed の `"0"` → `"0px"` case)。
/// 第 2 成分省略時は第 1 成分を copy する (spec 本文 +
/// [`GapShorthand`] と同型)。3 成分以上・bare non-zero number・`%` は
/// caller (`rule.rs::DeclParser`) の `expect_exhausted` / 各成分の `None`
/// で drop する。
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

/// `caption-side: <ident>` を parse する (CSS Tables 3 §7
/// <https://www.w3.org/TR/css-tables-3/#caption-side-property>,
/// [`CaptionSideValue`] doc 参照)。
///
/// Value grammar: `top | bottom`。ASCII case-insensitive matching は
/// sibling [`parse_table_layout`] と同 flavor、余剰 token
/// (`caption-side: top bottom` 等) は caller (`rule.rs::DeclParser`) の
/// `expect_exhausted` が drop する。
pub(super) fn parse_caption_side(input: &mut Parser<'_, '_>) -> Option<CaptionSideValue> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "top" => Some(CaptionSideValue::Top),
        "bottom" => Some(CaptionSideValue::Bottom),
        _ => None,
    }
}

/// `empty-cells: <ident>` を parse する (CSS Tables 3 §8
/// <https://www.w3.org/TR/css-tables-3/#empty-cells-property>,
/// [`EmptyCellsValue`] doc 参照)。
///
/// Value grammar: `show | hide`。matching 規則は sibling
/// [`parse_caption_side`] と同じ。
pub(super) fn parse_empty_cells(input: &mut Parser<'_, '_>) -> Option<EmptyCellsValue> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "show" => Some(EmptyCellsValue::Show),
        "hide" => Some(EmptyCellsValue::Hide),
        _ => None,
    }
}

pub(super) fn parse_display(input: &mut Parser<'_, '_>) -> Option<DisplayValue> {
    // sibling multi-keyword idiom (parse_string_fetch / parse_content_part /
    // parse_content_text_keyword) に揃える。ASCII case-insensitive matching は
    // to_ascii_lowercase() 経由 (parse-time allocation は一 declaration 一回)。
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "block" => Some(DisplayValue::Block),
        "inline" => Some(DisplayValue::Inline),
        "inline-block" => Some(DisplayValue::InlineBlock),
        "flow-root" => Some(DisplayValue::FlowRoot),
        "none" => Some(DisplayValue::None),
        // The current layout bridge models the outer display type as block,
        // so inline-flex/inline-grid share the corresponding formatting
        // context until inline-level shrink-to-fit support is added.
        "flex" => Some(DisplayValue::Flex),
        "inline-flex" => Some(DisplayValue::InlineFlex),
        "grid" => Some(DisplayValue::Grid),
        "inline-grid" => Some(DisplayValue::InlineGrid),
        "list-item" => Some(DisplayValue::ListItem),
        "contents" => Some(DisplayValue::Contents),
        "table" => Some(DisplayValue::Table),
        "inline-table" => Some(DisplayValue::InlineTable),
        "table-row-group" => Some(DisplayValue::TableRowGroup),
        "table-header-group" => Some(DisplayValue::TableHeaderGroup),
        "table-footer-group" => Some(DisplayValue::TableFooterGroup),
        "table-row" => Some(DisplayValue::TableRow),
        "table-column-group" => Some(DisplayValue::TableColumnGroup),
        "table-column" => Some(DisplayValue::TableColumn),
        "table-cell" => Some(DisplayValue::TableCell),
        "table-caption" => Some(DisplayValue::TableCaption),
        _ => None,
    }
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
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "inside" => Some(ListStylePosition::Inside),
        "outside" => Some(ListStylePosition::Outside),
        _ => None,
    }
}

/// `orphans` / `widows: <integer>` の value を parse する (CSS Fragmentation
/// Module Level 3 §3.3 "Breaks Between Lines: orphans, widows"
/// <https://www.w3.org/TR/css-break-3/#widows-orphans>).
///
/// `expect_integer` 直接呼び出しは [`parse_z_index`](super::box_model::parse_z_index) / [`parse_counter_property`](super::content::parse_counter_property)
/// と同じ pattern。この 2 property は `<integer>` を **正の値のみ**に制限する
/// spec 独自の制約を持つ点が sibling と異なる: "Only positive integers are
/// allowed as values of orphans and widows. Negative values and zero are
/// invalid and must cause the declaration to be ignored." — 0 以下は
/// spec-invalid として `None` (declaration が丸ごと drop される、
/// 他の out-of-range `<integer>`/`<length>` value と同じ扱い)。
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

/// `break-before: <ident>` / `break-after: <ident>` を parse する (CSS
/// Fragmentation Module Level 3 §3.1
/// <https://www.w3.org/TR/css-break-3/#break-between>)。
///
/// この crate の scope で受理する 4 keyword ([`BreakBetween`] doc の Scope
/// carving 節参照): `auto` / `avoid` / `avoid-page` / `page`。propdef の
/// 残り 8 keyword (`left` / `right` / `recto` / `verso` / `avoid-column` /
/// `column` / `avoid-region` / `region`) と、現行 spec grammar に無い
/// `always` / `all` は他の未知 ident と同じく silent drop (`None`)。ASCII
/// case-insensitive で ident を比較する ([`parse_word_break`](super::text::parse_word_break) 等 sibling と
/// 同 flavor)。
pub(super) fn parse_break_between(input: &mut Parser<'_, '_>) -> Option<BreakBetween> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "auto" => Some(BreakBetween::Auto),
        "avoid" => Some(BreakBetween::Avoid),
        "avoid-page" => Some(BreakBetween::AvoidPage),
        "page" => Some(BreakBetween::Page),
        _ => None,
    }
}

/// `break-inside: <ident>` を parse する (CSS Fragmentation Module Level 3
/// §3.2 <https://www.w3.org/TR/css-break-3/#break-within>)。
///
/// この crate の scope で受理する 3 keyword ([`BreakInside`] doc の Scope
/// carving 節参照): `auto` / `avoid` / `avoid-page`。propdef の残り 2
/// keyword (`avoid-column` / `avoid-region`) は他の未知 ident と同じく
/// silent drop (`None`)。ASCII case-insensitive で ident を比較する
/// ([`parse_break_between`] と同 flavor)。
pub(super) fn parse_break_inside(input: &mut Parser<'_, '_>) -> Option<BreakInside> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "auto" => Some(BreakInside::Auto),
        "avoid" => Some(BreakInside::Avoid),
        "avoid-page" => Some(BreakInside::AvoidPage),
        _ => None,
    }
}

/// `page-break-before: <ident>` / `page-break-after: <ident>` — CSS2.1
/// legacy shorthand for `break-before` / `break-after` — を parse し、
/// [`BreakBetween`] へ remap する (CSS Fragmentation Module Level 3 §3.4
/// <https://www.w3.org/TR/css-break-3/#page-break-properties>,
/// [`BreakBetween`] doc の「legacy shorthand」節の mapping table 参照)。
///
/// CSS2.1 自身の `page-break-before` / `page-break-after` propdef grammar
/// (verbatim, <https://www.w3.org/TR/CSS2/page.html#propdef-page-break-before>)
/// は `auto | always | avoid | left | right`。本 parser はそのうち
/// `auto` / `avoid` / `always` の 3 keyword のみ受理する — `left` /
/// `right` は `break-before`/`break-after` 側で未実装 ([`BreakBetween`] doc
/// の Scope carving 節) の値へ remap されるため、この legacy shorthand
/// 経由でも同じく受理しない。`always` は spec の mapping table どおり
/// [`BreakBetween::Page`] へ remap する (identity ではない — `auto` /
/// `avoid` は identity)。
pub(super) fn parse_legacy_page_break_between(input: &mut Parser<'_, '_>) -> Option<BreakBetween> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "auto" => Some(BreakBetween::Auto),
        "avoid" => Some(BreakBetween::Avoid),
        "always" => Some(BreakBetween::Page),
        _ => None,
    }
}

/// `page-break-inside: <ident>` — CSS2.1 legacy shorthand for
/// `break-inside` — を parse する (CSS Fragmentation Module Level 3 §3.4,
/// [`BreakInside`] doc の「legacy shorthand」節参照)。
///
/// CSS2.1 自身の `page-break-inside` propdef grammar (verbatim,
/// <https://www.w3.org/TR/CSS2/page.html#propdef-page-break-inside>) は
/// `avoid | auto` のみ (`always` / `left` / `right` は無い) — この 2
/// keyword を [`BreakInside`] へ identity mapping する。`break-inside`
/// 自身が持つ `avoid-page` は CSS2.1 の `page-break-inside` grammar には
/// 無いため受理しない。
pub(super) fn parse_legacy_page_break_inside(input: &mut Parser<'_, '_>) -> Option<BreakInside> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "auto" => Some(BreakInside::Auto),
        "avoid" => Some(BreakInside::Avoid),
        _ => None,
    }
}
