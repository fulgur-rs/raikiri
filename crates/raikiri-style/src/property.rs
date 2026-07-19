//! CSS property value 型と per-property parser。
//!
//! M1.4 では color / font-family / font-size / font-weight の 4 property のみ。
//! 認識できない property name / invalid value は `parse_value` が `None` を返す
//! (spec 準拠の silent drop、caller である rule.rs で declaration ごと drop)。
//!
//! `parse_value` は rule.rs の `DeclParser::parse_value` から呼ばれる。

use cssparser::color::{clamp_unit_f32, parse_hash_color, parse_named_color};
use cssparser::{ParseError, Parser, Token};
use smol_str::SmolStr;

use crate::Atom;

/// RGBA color (0-255 per channel、`a` は 255 = fully opaque)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CssColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl CssColor {
    /// Opaque black — `<color>` initial value に相当。
    pub const BLACK: Self = Self {
        r: 0,
        g: 0,
        b: 0,
        a: 255,
    };
}

/// CSS length。M1.4 では pixel (`<length>` = px リテラル) のみ。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Length {
    /// Absolute pixel length。
    Px(f32),
}

/// `display` property の value。M1.4a scope では `block` / `inline` のみ。
///
/// spec §M1.4a Non-goals: `table*`, `flex`, `grid`, `none` 等は M6+。
/// (raikiri-spike-m1.22)
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayValue {
    Block,
    Inline,
}

/// M1.4 でサポートする property の resolved value。
///
/// 認識できない property (`background-color` / `margin` / ...) や invalid value
/// (`font-size: 1em` — em 未対応) は parser 段で `None` に落として rule から
/// silently 除外される。
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum PropertyValue {
    /// `color: <color>` — inherited、initial: black。
    Color(CssColor),
    /// `font-family: <family-name>#` — inherited、initial: `[Atom::from("serif")]`。
    FontFamily(Vec<Atom>),
    /// `font-size: <length>` — inherited、initial: 16px。
    FontSize(Length),
    /// `font-weight: <integer>` — inherited、initial: 400。
    FontWeight(u16),
    /// `display: <block-or-inline>` — non-inherited、initial: inline
    /// (spec §M1.4a、raikiri-spike-m1.22)。
    Display(DisplayValue),
    /// `counter-reset: [ <counter-name> <integer>? ]+ | none` —
    /// non-inherited、initial: empty list (CSS Lists 3 §3)。
    /// missing integer は 0 に default (spec default)。M5 pre-work (raikiri-spike-s85)。
    CounterReset(Vec<(SmolStr, i32)>),
    /// `counter-increment: [ <counter-name> <integer>? ]+ | none` —
    /// non-inherited、initial: empty list (CSS Lists 3 §3)。
    /// missing integer は 1 に default (spec default)。M5 pre-work (raikiri-spike-s85)。
    CounterIncrement(Vec<(SmolStr, i32)>),
    /// `counter-set: [ <counter-name> <integer>? ]+ | none` —
    /// non-inherited、initial: empty list (CSS Lists 3 §3)。
    /// missing integer は 0 に default (spec default)。M5 pre-work (raikiri-spike-s85)。
    CounterSet(Vec<(SmolStr, i32)>),
}

/// Property key (cascade で "同一 property を勝ち取る" ための discriminant)。
///
/// cascade.rs の per-node winner selection、および page.rs の
/// [`cascade_page`](crate::page::cascade_page) が [`PageCascadeResult`] の
/// map key に使う。`PropertyValue` の variant tag を stateless に抜き出したもので
/// 追加情報を持たないため public に露出する (raikiri-spike-m4.1、[`PageCascadeResult`]
/// が `pub` 型を要求するため — clippy `private_interfaces` 対応)。
///
/// [`PageCascadeResult`]: crate::page::PageCascadeResult
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PropertyKey {
    Color,
    FontFamily,
    FontSize,
    FontWeight,
    Display,
    CounterReset,
    CounterIncrement,
    CounterSet,
}

impl PropertyValue {
    /// この value が属する property key を返す。
    ///
    /// cascade winner selection で "同一 property を勝ち取る" ための discriminant として、
    /// また `@page` cascade 結果 map の key として使う。
    pub fn key(&self) -> PropertyKey {
        match self {
            PropertyValue::Color(_) => PropertyKey::Color,
            PropertyValue::FontFamily(_) => PropertyKey::FontFamily,
            PropertyValue::FontSize(_) => PropertyKey::FontSize,
            PropertyValue::FontWeight(_) => PropertyKey::FontWeight,
            PropertyValue::Display(_) => PropertyKey::Display,
            PropertyValue::CounterReset(_) => PropertyKey::CounterReset,
            PropertyValue::CounterIncrement(_) => PropertyKey::CounterIncrement,
            PropertyValue::CounterSet(_) => PropertyKey::CounterSet,
        }
    }
}

/// Property name + Parser から `PropertyValue` を produce。
/// 認識できない name / invalid value は `None`。
pub(crate) fn parse_value(name: &str, input: &mut Parser<'_, '_>) -> Option<PropertyValue> {
    // ascii-lowercase 比較で property name を dispatch。
    let normalized_name = name.to_ascii_lowercase();
    match normalized_name.as_str() {
        "color" => parse_color(input).map(PropertyValue::Color),
        "font-family" => parse_font_family(input).map(PropertyValue::FontFamily),
        "font-size" => parse_font_size(input).map(PropertyValue::FontSize),
        "font-weight" => parse_font_weight(input).map(PropertyValue::FontWeight),
        "display" => parse_display(input).map(PropertyValue::Display),
        // CSS Lists 3 §3 counter properties (raikiri-spike-s85、M5 pre-work)。
        // spec default: reset = 0、increment = 1、set = 0。
        "counter-reset" => parse_counter_property(input, 0).map(PropertyValue::CounterReset),
        "counter-increment" => {
            parse_counter_property(input, 1).map(PropertyValue::CounterIncrement)
        }
        "counter-set" => parse_counter_property(input, 0).map(PropertyValue::CounterSet),
        _ => None,
    }
}

/// `<color>` を parse する。
///
/// cssparser 0.37 は (0.36 までと異なり) 汎用 `Color` enum / `Color::parse` を
/// 提供しない — それは別 crate `cssparser-color` 側に移った。ここでは
/// `cssparser::color` に残っている building block (`parse_hash_color` /
/// `parse_named_color`) と、`rgb()` / `rgba()` function の手動 parse で
/// hex / named / rgb() の 3 形式をカバーする (m1.4 scope)。
fn parse_color(input: &mut Parser<'_, '_>) -> Option<CssColor> {
    let token = input.next().ok()?.clone();
    match token {
        Token::Hash(ref value) | Token::IDHash(ref value) => {
            let (r, g, b, alpha) = parse_hash_color(value.as_bytes()).ok()?;
            Some(CssColor {
                r,
                g,
                b,
                a: clamp_unit_f32(alpha),
            })
        }
        Token::Ident(ref name) => {
            let (r, g, b) = parse_named_color(name).ok()?;
            Some(CssColor { r, g, b, a: 255 })
        }
        Token::Function(ref name)
            if name.eq_ignore_ascii_case("rgb") || name.eq_ignore_ascii_case("rgba") =>
        {
            input.parse_nested_block(parse_rgb_function).ok()
        }
        _ => None,
    }
}

/// `rgb( <integer> , <integer> , <integer> [, <number>]? )` の中身 (関数呼び出しの
/// 括弧内) を parse する。`parse_nested_block` の caller 側で `rgb(` / `rgba(` の
/// function token は既に consume 済み。
fn parse_rgb_function<'i>(input: &mut Parser<'i, '_>) -> Result<CssColor, ParseError<'i, ()>> {
    let r = clamp_channel(input.expect_integer()?);
    input.expect_comma()?;
    let g = clamp_channel(input.expect_integer()?);
    input.expect_comma()?;
    let b = clamp_channel(input.expect_integer()?);
    let a = if input.try_parse(|input| input.expect_comma()).is_ok() {
        clamp_unit_f32(input.expect_number()?)
    } else {
        255
    };
    Ok(CssColor { r, g, b, a })
}

fn clamp_channel(value: i32) -> u8 {
    value.clamp(0, 255) as u8
}

/// `font-family: <family-name>#` を parse する。
///
/// comma-separated な family-name の list。各 family-name は quoted string
/// (`"Times New Roman"`) か、unquoted identifier の連続 (`Times New Roman` =
/// 3 ident が空白区切りで 1 family、CSS4 で有効) のいずれか。
///
/// 末尾で comma が続かなければ loop を止め、残り input (`!important` 等) は
/// 手を付けずに downstream (caller の `parse_important` / `expect_exhausted`)
/// に委ねる — `!` を garbage として拒否しないための Finding 3 対応。
fn parse_font_family(input: &mut Parser<'_, '_>) -> Option<Vec<Atom>> {
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

fn parse_font_size(input: &mut Parser<'_, '_>) -> Option<Length> {
    // <length> = px リテラルのみ (m1.4 scope)。
    match input.next().ok()? {
        Token::Dimension { value, unit, .. }
            if unit.eq_ignore_ascii_case("px") && *value >= 0.0 =>
        {
            Some(Length::Px(*value))
        }
        _ => None,
    }
}

fn parse_font_weight(input: &mut Parser<'_, '_>) -> Option<u16> {
    // integer literal (100..=900) のみ、keyword は drop。
    match input.next().ok()? {
        Token::Number {
            int_value: Some(v), ..
        } if *v >= 100 && *v <= 900 => Some(*v as u16),
        _ => None,
    }
}

/// `display: <ident>` を parse する。
///
/// M1.4a scope では `block` / `inline` のみ受理、他 keyword (`flex`,
/// `grid`, `none`, `table*` 等) は silent drop (`None`)。
/// ASCII case-insensitive で ident を比較する (CSS spec 準拠)。
fn parse_display(input: &mut Parser<'_, '_>) -> Option<DisplayValue> {
    let ident = input.next().ok()?;
    match ident {
        Token::Ident(name) if name.eq_ignore_ascii_case("block") => Some(DisplayValue::Block),
        Token::Ident(name) if name.eq_ignore_ascii_case("inline") => Some(DisplayValue::Inline),
        _ => None,
    }
}

/// `counter-reset` / `counter-increment` / `counter-set` の value を parse する。
///
/// Grammar (CSS Lists 3 §3):
///   `<counter-name> = <custom-ident>` — CSS-wide keyword (inherit / initial /
///   unset / revert / revert-layer) + `default` + `none` を除く任意 ident。
///   `[ <counter-name> <integer>? ]+ | none`。
///
/// `default_number`: 各 property の spec default (reset=0、increment=1、set=0)。
///
/// `none` を top-level alternative として先に処理。以降は ident + optional
/// integer を LL(1) で peel。ident が reserved keyword、または最初の token が
/// ident でない (`counter-reset: 123 abc` 等) 場合は None を返し、rule.rs 側の
/// silent-drop で declaration が丸ごと落ちる。
///
/// 途中 ident (`chapter none`) が reserved の場合は `try_parse` の rewind で
/// 未消費のまま loop を抜け、caller の `expect_exhausted` (rule.rs)
/// が leftover token を検出して declaration を drop する。
fn parse_counter_property(
    input: &mut Parser<'_, '_>,
    default_number: i32,
) -> Option<Vec<(SmolStr, i32)>> {
    // `none` = empty list (top-level alternative)。
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(Vec::new());
    }

    let mut result = Vec::new();
    loop {
        // reserved keyword を counter-name として受理しない (spec §3、`<custom-ident>`
        // の除外リスト)。try_parse の rewind で reserved 検出時は unconsumed に戻す。
        let name = match input.try_parse(|i| -> Result<SmolStr, ParseError<'_, ()>> {
            let ident = i.expect_ident()?.clone();
            if is_reserved_counter_name(&ident) {
                Err(i.new_custom_error(()))
            } else {
                Ok(SmolStr::new(ident.as_ref()))
            }
        }) {
            Ok(name) => name,
            Err(_) => break,
        };
        // optional trailing `<integer>` (missing → property-specific default)。
        let value = input
            .try_parse(|i| i.expect_integer())
            .unwrap_or(default_number);
        result.push((name, value));
    }

    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

/// `<counter-name>` = `<custom-ident>` の除外リスト (CSS Lists 3 §3 + CSS Values 4)。
///
/// CSS-wide keyword + `default` (Counter Styles L3) + `none` (top-level alternative)
/// を弾く。case-insensitive 比較。
fn is_reserved_counter_name(ident: &str) -> bool {
    matches!(
        ident.to_ascii_lowercase().as_str(),
        "inherit" | "initial" | "unset" | "revert" | "revert-layer" | "default" | "none"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use cssparser::ParserInput;

    fn parse(source: &str, name: &str) -> Option<PropertyValue> {
        let mut input = ParserInput::new(source);
        let mut parser = Parser::new(&mut input);
        parse_value(name, &mut parser)
    }

    #[test]
    fn color_parse_hex() {
        assert_eq!(
            parse("#ff0000", "color"),
            Some(PropertyValue::Color(CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255
            }))
        );
    }

    #[test]
    fn color_parse_named() {
        assert_eq!(
            parse("red", "color"),
            Some(PropertyValue::Color(CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255
            }))
        );
    }

    #[test]
    fn color_parse_rgb() {
        assert_eq!(
            parse("rgb(255, 0, 0)", "color"),
            Some(PropertyValue::Color(CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255
            }))
        );
    }

    #[test]
    fn color_parse_invalid_returns_none() {
        assert_eq!(parse("bogus", "color"), None);
        assert_eq!(parse("", "color"), None);
    }

    #[test]
    fn font_size_parse_px() {
        assert_eq!(
            parse("16px", "font-size"),
            Some(PropertyValue::FontSize(Length::Px(16.0)))
        );
    }

    #[test]
    fn font_size_rejects_em_and_keyword() {
        assert_eq!(parse("1em", "font-size"), None);
        assert_eq!(parse("medium", "font-size"), None);
    }

    #[test]
    fn font_size_rejects_negative() {
        // spec: font-size は non-negative <length> のみ。
        assert_eq!(parse("-10px", "font-size"), None);
        assert_eq!(parse("-0.5px", "font-size"), None);
    }

    #[test]
    fn font_size_accepts_zero() {
        assert_eq!(
            parse("0px", "font-size"),
            Some(PropertyValue::FontSize(Length::Px(0.0)))
        );
    }

    #[test]
    fn font_family_parse_comma_list() {
        let got = parse(r#"Arial, "Times New Roman", serif"#, "font-family");
        let expected = Some(PropertyValue::FontFamily(vec![
            Atom::from("Arial"),
            Atom::from("Times New Roman"),
            Atom::from("serif"),
        ]));
        assert_eq!(got, expected);
    }

    #[test]
    fn font_family_unquoted_multi_word_single_family() {
        // CSS4: unquoted multi-word family name = ident sequence joined by space。
        let got = parse("Times New Roman", "font-family");
        let expected = Some(PropertyValue::FontFamily(vec![Atom::from(
            "Times New Roman",
        )]));
        assert_eq!(got, expected);
    }

    #[test]
    fn font_weight_parse_integer() {
        assert_eq!(
            parse("400", "font-weight"),
            Some(PropertyValue::FontWeight(400))
        );
        assert_eq!(
            parse("700", "font-weight"),
            Some(PropertyValue::FontWeight(700))
        );
    }

    #[test]
    fn font_weight_rejects_keyword() {
        assert_eq!(parse("bold", "font-weight"), None);
        assert_eq!(parse("normal", "font-weight"), None);
    }

    #[test]
    fn unknown_property_returns_none() {
        assert_eq!(parse("100px", "margin"), None);
        assert_eq!(parse("red", "background-color"), None);
    }

    // ── Display (M1.4a、raikiri-spike-m1.22) ─────────────────────

    #[test]
    fn display_parse_block() {
        assert_eq!(
            parse("block", "display"),
            Some(PropertyValue::Display(DisplayValue::Block))
        );
    }

    #[test]
    fn display_parse_inline() {
        assert_eq!(
            parse("inline", "display"),
            Some(PropertyValue::Display(DisplayValue::Inline))
        );
    }

    #[test]
    fn display_rejects_unknown_ident() {
        // spec §M1.4a: block と inline 以外の値 (flex, grid, none, table, ...) は
        // M6+ 対応、現状は silent drop (None を返す)
        assert_eq!(parse("flex", "display"), None);
        assert_eq!(parse("grid", "display"), None);
        assert_eq!(parse("none", "display"), None);
        assert_eq!(parse("table", "display"), None);
    }

    #[test]
    fn display_rejects_non_ident() {
        assert_eq!(parse("16px", "display"), None);
        assert_eq!(parse("100", "display"), None);
    }

    #[test]
    fn display_is_case_insensitive() {
        // CSS spec: property value keyword は ASCII case-insensitive
        assert_eq!(
            parse("BLOCK", "display"),
            Some(PropertyValue::Display(DisplayValue::Block))
        );
        assert_eq!(
            parse("Inline", "display"),
            Some(PropertyValue::Display(DisplayValue::Inline))
        );
    }

    // ── counter-* (CSS Lists 3 §3、raikiri-spike-s85 M5 pre-work) ──

    fn counter_pairs(pairs: &[(&str, i32)]) -> Vec<(SmolStr, i32)> {
        pairs
            .iter()
            .map(|(name, value)| (SmolStr::new(name), *value))
            .collect()
    }

    #[test]
    fn counter_reset_single_name_defaults_to_zero() {
        // spec: reset の default は 0
        assert_eq!(
            parse("chapter", "counter-reset"),
            Some(PropertyValue::CounterReset(counter_pairs(&[(
                "chapter", 0
            )])))
        );
    }

    #[test]
    fn counter_reset_multiple_names_with_mixed_ints() {
        // 2 番目に integer が付く → 1 番目は default 0、2 番目は 3
        assert_eq!(
            parse("chapter section 3", "counter-reset"),
            Some(PropertyValue::CounterReset(counter_pairs(&[
                ("chapter", 0),
                ("section", 3)
            ])))
        );
    }

    #[test]
    fn counter_reset_none_returns_empty_vec() {
        // spec: `none` は空リストと同等 (top-level alternative)
        assert_eq!(
            parse("none", "counter-reset"),
            Some(PropertyValue::CounterReset(Vec::new()))
        );
    }

    #[test]
    fn counter_reset_rejects_number_first() {
        // 先頭が number → ident が来るまで peel できず empty → None (drop)
        // spec §3: `<counter-name> = <custom-ident>` (数値は counter-name ではない)
        assert_eq!(parse("123 abc", "counter-reset"), None);
    }

    #[test]
    fn counter_increment_single_name_defaults_to_one() {
        // spec: increment の default は 1
        assert_eq!(
            parse("chapter", "counter-increment"),
            Some(PropertyValue::CounterIncrement(counter_pairs(&[(
                "chapter", 1
            )])))
        );
    }

    #[test]
    fn counter_increment_mixed_int_and_default() {
        // `chapter 2 section` → chapter=2、section=default(1)
        assert_eq!(
            parse("chapter 2 section", "counter-increment"),
            Some(PropertyValue::CounterIncrement(counter_pairs(&[
                ("chapter", 2),
                ("section", 1)
            ])))
        );
    }

    #[test]
    fn counter_increment_accepts_negative_integer() {
        // spec §3: <integer> — negative も valid (counter を decrement する用途)
        assert_eq!(
            parse("chapter -1", "counter-increment"),
            Some(PropertyValue::CounterIncrement(counter_pairs(&[(
                "chapter", -1
            )])))
        );
    }

    #[test]
    fn counter_increment_none_returns_empty_vec() {
        assert_eq!(
            parse("none", "counter-increment"),
            Some(PropertyValue::CounterIncrement(Vec::new()))
        );
    }

    #[test]
    fn counter_set_defaults_to_zero() {
        // spec: set の default は 0
        assert_eq!(
            parse("page 5 note", "counter-set"),
            Some(PropertyValue::CounterSet(counter_pairs(&[
                ("page", 5),
                ("note", 0)
            ])))
        );
    }

    #[test]
    fn counter_set_none_returns_empty_vec() {
        assert_eq!(
            parse("none", "counter-set"),
            Some(PropertyValue::CounterSet(Vec::new()))
        );
    }

    #[test]
    fn counter_reset_is_case_insensitive_on_none() {
        // CSS spec: keyword `none` は ASCII case-insensitive
        assert_eq!(
            parse("NONE", "counter-reset"),
            Some(PropertyValue::CounterReset(Vec::new()))
        );
    }

    #[test]
    fn counter_reset_rejects_reserved_css_wide_keyword_as_name() {
        // spec §3: <counter-name> excludes CSS-wide keywords + `default`。
        // 先頭 ident が `inherit` → try_parse rewind で empty result → None。
        assert_eq!(parse("inherit", "counter-reset"), None);
        assert_eq!(parse("initial", "counter-reset"), None);
        assert_eq!(parse("unset", "counter-reset"), None);
        assert_eq!(parse("revert", "counter-reset"), None);
        assert_eq!(parse("default", "counter-reset"), None);
    }

    #[test]
    fn counter_reset_accepts_negative_integer() {
        // CSS Values 3 §5.1: <integer> は負値を含む。
        // increment だけでなく reset / set も同一 grammar。
        assert_eq!(
            parse("chapter -5", "counter-reset"),
            Some(PropertyValue::CounterReset(counter_pairs(&[(
                "chapter", -5
            )])))
        );
    }

    #[test]
    fn counter_set_accepts_negative_integer() {
        // 同上 (parity with reset/increment negative-integer coverage)。
        assert_eq!(
            parse("page -3", "counter-set"),
            Some(PropertyValue::CounterSet(counter_pairs(&[("page", -3)])))
        );
    }
}
