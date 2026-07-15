//! CSS property value 型と per-property parser。
//!
//! M1.4 では color / font-family / font-size / font-weight の 4 property のみ。
//! 認識できない property name / invalid value は `parse_value` が `None` を返す
//! (spec 準拠の silent drop、caller である rule.rs で declaration ごと drop)。
//!
//! `parse_value` は rule.rs の `DeclParser::parse_value` から呼ばれる。

use cssparser::color::{clamp_unit_f32, parse_hash_color, parse_named_color};
use cssparser::{ParseError, Parser, Token};

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
    pub const BLACK: Self = Self { r: 0, g: 0, b: 0, a: 255 };
}

/// CSS length。M1.4 では pixel (`<length>` = px リテラル) のみ。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Length {
    /// Absolute pixel length。
    Px(f32),
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
}

/// Property key (cascade で "同一 property を勝ち取る" ための discriminant)。
///
/// M1.4 の後続 task (cascade winner selection, Task 7) が使う。それまでの間
/// caller が無いため dead code — Task 7 で cascade.rs から使われるようになれば
/// この `#[allow]` は不要になる。
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum PropertyKey {
    Color,
    FontFamily,
    FontSize,
    FontWeight,
}

impl PropertyValue {
    /// Task 7 (cascade winner selection) が使う。それまでの間 caller が無い。
    #[allow(dead_code)]
    pub(crate) fn key(&self) -> PropertyKey {
        match self {
            PropertyValue::Color(_) => PropertyKey::Color,
            PropertyValue::FontFamily(_) => PropertyKey::FontFamily,
            PropertyValue::FontSize(_) => PropertyKey::FontSize,
            PropertyValue::FontWeight(_) => PropertyKey::FontWeight,
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
fn parse_rgb_function<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<CssColor, ParseError<'i, ()>> {
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

fn parse_font_family(input: &mut Parser<'_, '_>) -> Option<Vec<Atom>> {
    // comma-separated identifier or string の list。
    let mut families = Vec::new();
    loop {
        let name = match input.next().ok()? {
            Token::Ident(s) => Atom::from(s.as_ref()),
            Token::QuotedString(s) => Atom::from(s.as_ref()),
            _ => return None,
        };
        families.push(name);
        // 次が comma なら continue、EOF なら break。他 token は invalid。
        match input.next() {
            Ok(Token::Comma) => continue,
            Err(_) => break, // EOF
            _ => return None,
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
        Token::Dimension { value, unit, .. } if unit.eq_ignore_ascii_case("px") => {
            Some(Length::Px(*value))
        }
        _ => None,
    }
}

fn parse_font_weight(input: &mut Parser<'_, '_>) -> Option<u16> {
    // integer literal (100..=900) のみ、keyword は drop。
    match input.next().ok()? {
        Token::Number {
            int_value: Some(v),
            ..
        } if *v >= 100 && *v <= 900 => Some(*v as u16),
        _ => None,
    }
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
            Some(PropertyValue::Color(CssColor { r: 255, g: 0, b: 0, a: 255 }))
        );
    }

    #[test]
    fn color_parse_named() {
        assert_eq!(
            parse("red", "color"),
            Some(PropertyValue::Color(CssColor { r: 255, g: 0, b: 0, a: 255 }))
        );
    }

    #[test]
    fn color_parse_rgb() {
        assert_eq!(
            parse("rgb(255, 0, 0)", "color"),
            Some(PropertyValue::Color(CssColor { r: 255, g: 0, b: 0, a: 255 }))
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
}
