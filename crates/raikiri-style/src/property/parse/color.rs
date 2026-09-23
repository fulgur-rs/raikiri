//! `<color>` parsing, plus color-space conversion and interpolation math.

use cssparser::color::parse_named_color;
use cssparser::{ParseError, Parser, Token};

use crate::property::types::*;

use super::common::*;

/// `<color>` を parse する。
///
/// cssparser 0.37 は (0.36 までと異なり) 汎用 `Color` enum / `Color::parse` を
/// 提供しない — それは別 crate `cssparser-color` 側に移った。ここでは
/// 各 form の parse を自前 (独立実装) で組み立て、hex / named / rgb() /
/// color() / lab() / lch() / oklab() / oklch() / color-mix() をカバーする:
///
/// - **Hex** (`#rgb` / `#rgba` / `#rrggbb` / `#rrggbbaa`) は
///   [`CssColor::from_hex`] を呼び出す — CSS Color 4 §5.2 準拠の 独立実装 実装。
///   `Token::Hash` / `Token::IDHash` の payload は leading `#` を含まないため
///   そのまま渡す。
/// - **Named color** は `parse_named_color` (140+ CSS Color L3 keyword table を
///   再実装しない方針のため cssparser の table を暫定利用)。
/// - **`rgb()` / `rgba()` function form** は [`parse_rgb_function`] で
///   `parse_nested_block` 経由の手動 parse。
///
/// `transparent` keyword は CSS Color 4 §6.3 "The transparent keyword"
/// <https://www.w3.org/TR/css-color-4/#transparent-color> で
/// `rgba(0, 0, 0, 0)` の shorthand と規定される — `parse_named_color` の
/// (r, g, b) は alpha を返さないため、Ident arm 手前で明示 branch して
/// [`CssColor::TRANSPARENT`] を返す。
///
/// `from` を先頭に置く CSS Color 5 の relative color syntax は、origin と
/// channel/math grammar を検証する。cascade context を持たないため、解決値は
/// origin color の bounded approximation を保持し、`var()` origin は deferred
/// value として後段へ渡す。
///
/// Modern color syntax の `none`（missing component）は構文上受理し、bounded
/// model では一時的に zero component として扱う。missing-component の
/// carry-forward と computed-value serialization は後段の未実装範囲である。
/// Lab/OKLab の lightness が black/white boundary にある場合は conversion 側で
/// a/b や chroma にかかわらず boundary color へ固定する。それ以外の
/// out-of-gamut は 8-bit sRGB への bounded approximation であり、CSS Color 4 の
/// 完全な gamut mapping は未対応である。`color-mix()` の interpolation では、
/// Lab-family の座標を sRGB へ先に clip せず、指定空間での計算後にだけ変換する。
pub(crate) fn parse_color(input: &mut Parser<'_, '_>) -> Option<CssColor> {
    parse_color_float(input, 0).map(ParsedColor::to_css_color)
}

/// CSS system-color keywords are context-dependent at computed-value time.
/// The parser has no document/UA color context, so preserve their syntactic
/// validity and use a deterministic black/white approximation for the bounded
/// `CssColor` model.
fn parse_system_color(name: &str) -> Option<CssColor> {
    let is_system = matches!(
        name.to_ascii_lowercase().as_str(),
        "activetext"
            | "buttonborder"
            | "buttonface"
            | "buttontext"
            | "canvas"
            | "canvastext"
            | "field"
            | "fieldtext"
            | "graytext"
            | "highlight"
            | "highlighttext"
            | "linktext"
            | "mark"
            | "marktext"
            | "visitedtext"
            | "selecteditem"
            | "selecteditemtext"
            | "accentcolor"
            | "accentcolortext"
    );
    if !is_system {
        return None;
    }
    let light = matches!(
        name.to_ascii_lowercase().as_str(),
        "buttonface" | "canvas" | "field" | "highlighttext" | "marktext" | "selecteditemtext"
    );
    Some(if light {
        CssColor {
            r: 255,
            g: 255,
            b: 255,
            a: 255,
        }
    } else {
        CssColor::BLACK
    })
}

fn parse_alpha_color_function<'i>(
    input: &mut Parser<'i, '_>,
    color_mix_depth: usize,
) -> Result<ParsedColor, ParseError<'i, ()>> {
    input.expect_ident_matching("from")?;
    let origin = parse_relative_origin(input, color_mix_depth)?;
    let alpha = if input.try_parse(|i| i.expect_delim('/')).is_ok() {
        if input
            .try_parse(|i| i.expect_ident_matching("alpha"))
            .is_ok()
        {
            origin.alpha
        } else if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
            0.0
        } else if let Ok(percentage) = input.try_parse(|i| expect_percentage_stable(i)) {
            percentage.clamp(0.0, 1.0)
        } else if let Ok(number) = input.try_parse(|i| expect_number_stable(i)) {
            number.clamp(0.0, 1.0)
        } else {
            // The remaining valid form is a relative math expression such as
            // `calc(alpha * 0.5)`. Validate it with the alpha-only channel
            // set; evaluation is deferred to the context-aware cascade.
            parse_relative_component(input, RelativeColorKind::Alpha, 0)?;
            origin.alpha
        }
    } else {
        origin.alpha
    };
    Ok(ParsedColor::from_coordinates(
        ParsedColorSpace::Srgb,
        origin.to_srgb(),
        alpha,
    ))
}

fn parse_light_dark_function<'i>(
    input: &mut Parser<'i, '_>,
    color_mix_depth: usize,
) -> Result<ParsedColor, ParseError<'i, ()>> {
    let light =
        parse_color_float(input, color_mix_depth).ok_or_else(|| input.new_custom_error(()))?;
    input.expect_comma()?;
    let _dark =
        parse_color_float(input, color_mix_depth).ok_or_else(|| input.new_custom_error(()))?;
    // Color-scheme selection is a computed-value concern. Use the light branch
    // in this context-free specified-value parser.
    Ok(light)
}

fn parse_contrast_color_function<'i>(
    input: &mut Parser<'i, '_>,
    color_mix_depth: usize,
) -> Result<ParsedColor, ParseError<'i, ()>> {
    let origin =
        parse_color_float(input, color_mix_depth).ok_or_else(|| input.new_custom_error(()))?;
    let [r, g, b] = origin.to_srgb().map(|component| component.clamp(0.0, 1.0));
    // Relative luminance is sufficient for the parser's deterministic
    // two-candidate fallback. The contrast-color grammar itself is validated
    // independently of this approximation.
    let luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    Ok(ParsedColor::from_css_color(if luminance > 0.5 {
        CssColor::BLACK
    } else {
        CssColor {
            r: 255,
            g: 255,
            b: 255,
            a: 255,
        }
    }))
}

fn is_color_layers_blend_mode(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "normal"
            | "multiply"
            | "screen"
            | "overlay"
            | "darken"
            | "lighten"
            | "color-dodge"
            | "color-burn"
            | "hard-light"
            | "soft-light"
            | "difference"
            | "exclusion"
            | "hue"
            | "saturation"
            | "color"
            | "luminosity"
    )
}

fn composite_color_layers(bottom: ParsedColor, top: ParsedColor) -> ParsedColor {
    let bottom_rgb = bottom.to_srgb();
    let top_rgb = top.to_srgb();
    let bottom_alpha = bottom.alpha.clamp(0.0, 1.0);
    let top_alpha = top.alpha.clamp(0.0, 1.0);
    let alpha = top_alpha + bottom_alpha * (1.0 - top_alpha);
    if alpha == 0.0 {
        return ParsedColor::from_coordinates(ParsedColorSpace::Srgb, [0.0; 3], 0.0);
    }
    let rgb = std::array::from_fn(|index| {
        (top_rgb[index] * top_alpha + bottom_rgb[index] * bottom_alpha * (1.0 - top_alpha)) / alpha
    });
    ParsedColor::from_coordinates(ParsedColorSpace::Srgb, rgb, alpha)
}

fn parse_color_layers_function<'i>(
    input: &mut Parser<'i, '_>,
    color_mix_depth: usize,
) -> Result<ParsedColor, ParseError<'i, ()>> {
    // An optional blend mode is followed by a comma. Trying the complete
    // optional prefix atomically lets a color keyword such as `red` remain
    // the first layer when it is not a blend mode.
    let _has_blend_mode = input
        .try_parse(|i| {
            let name = i.expect_ident()?.clone();
            i.expect_comma()?;
            if is_color_layers_blend_mode(name.as_ref()) {
                Ok(())
            } else {
                Err(i.new_custom_error::<(), ()>(()))
            }
        })
        .is_ok();
    let mut result =
        parse_color_float(input, color_mix_depth).ok_or_else(|| input.new_custom_error(()))?;
    while input.try_parse(|i| i.expect_comma()).is_ok() {
        let layer =
            parse_color_float(input, color_mix_depth).ok_or_else(|| input.new_custom_error(()))?;
        result = composite_color_layers(result, layer);
    }
    Ok(result)
}

pub(crate) fn parse_color_float(
    input: &mut Parser<'_, '_>,
    color_mix_depth: usize,
) -> Option<ParsedColor> {
    let token = input.next().ok()?.clone();
    match token {
        Token::Hash(ref value) | Token::IDHash(ref value) => {
            CssColor::from_hex(value).map(ParsedColor::from_css_color)
        }
        Token::Ident(ref name) if name.eq_ignore_ascii_case("transparent") => {
            Some(ParsedColor::from_css_color(CssColor::TRANSPARENT))
        }
        Token::Ident(ref name) if name.eq_ignore_ascii_case("currentcolor") => {
            Some(ParsedColor::from_css_color(CssColor {
                r: 0,
                g: 0,
                b: 0,
                a: 255,
            }))
        }
        Token::Ident(ref name) => {
            if let Some(color) = parse_system_color(name) {
                Some(ParsedColor::from_css_color(color))
            } else {
                let (r, g, b) = parse_named_color(name).ok()?;
                Some(ParsedColor::from_css_color(CssColor { r, g, b, a: 255 }))
            }
        }
        Token::Function(ref name)
            if name.eq_ignore_ascii_case("rgb") || name.eq_ignore_ascii_case("rgba") =>
        {
            input
                .parse_nested_block(|nested| {
                    parse_rgb_function_or_relative(nested, color_mix_depth)
                })
                .ok()
        }
        Token::Function(ref name) if name.eq_ignore_ascii_case("color") => input
            .parse_nested_block(|nested| parse_color_function_or_relative(nested, color_mix_depth))
            .ok(),
        Token::Function(ref name) if name.eq_ignore_ascii_case("lab") => input
            .parse_nested_block(|i| {
                parse_lab_function_or_relative(i, LabFunction::Lab, color_mix_depth)
            })
            .ok(),
        Token::Function(ref name) if name.eq_ignore_ascii_case("lch") => input
            .parse_nested_block(|i| {
                parse_lab_function_or_relative(i, LabFunction::Lch, color_mix_depth)
            })
            .ok(),
        Token::Function(ref name) if name.eq_ignore_ascii_case("oklab") => input
            .parse_nested_block(|i| {
                parse_lab_function_or_relative(i, LabFunction::Oklab, color_mix_depth)
            })
            .ok(),
        Token::Function(ref name) if name.eq_ignore_ascii_case("oklch") => input
            .parse_nested_block(|i| {
                parse_lab_function_or_relative(i, LabFunction::Oklch, color_mix_depth)
            })
            .ok(),
        Token::Function(ref name)
            if name.eq_ignore_ascii_case("hsl") || name.eq_ignore_ascii_case("hsla") =>
        {
            input
                .parse_nested_block(|i| parse_hsl_function_or_relative(i, color_mix_depth))
                .ok()
        }
        Token::Function(ref name) if name.eq_ignore_ascii_case("hwb") => input
            .parse_nested_block(|i| parse_hwb_function_or_relative(i, color_mix_depth))
            .ok(),
        Token::Function(ref name) if name.eq_ignore_ascii_case("alpha") => input
            .parse_nested_block(|nested| parse_alpha_color_function(nested, color_mix_depth))
            .ok(),
        Token::Function(ref name) if name.eq_ignore_ascii_case("light-dark") => input
            .parse_nested_block(|nested| parse_light_dark_function(nested, color_mix_depth))
            .ok(),
        Token::Function(ref name) if name.eq_ignore_ascii_case("contrast-color") => input
            .parse_nested_block(|nested| parse_contrast_color_function(nested, color_mix_depth))
            .ok(),
        Token::Function(ref name) if name.eq_ignore_ascii_case("color-layers") => {
            if color_mix_depth >= MAX_COLOR_MIX_NESTING_DEPTH {
                None
            } else {
                input
                    .parse_nested_block(|nested| {
                        parse_color_layers_function(nested, color_mix_depth.saturating_add(1))
                    })
                    .ok()
            }
        }
        Token::Function(ref name) if name.eq_ignore_ascii_case("color-mix") => {
            if color_mix_depth >= MAX_COLOR_MIX_NESTING_DEPTH {
                None
            } else {
                input
                    .parse_nested_block(|nested| {
                        parse_color_mix_function(nested, color_mix_depth.saturating_add(1))
                    })
                    .ok()
            }
        }
        _ => None,
    }
}

#[derive(Clone, Copy)]
enum LabFunction {
    Lab,
    Lch,
    Oklab,
    Oklch,
}

#[derive(Clone, Copy)]
enum RelativeColorKind {
    Rgb,
    Alpha,
    Hsl,
    Hwb,
    Lab,
    Lch,
    Oklab,
    Oklch,
    ColorRgb,
    ColorXyz,
}

fn parse_rgb_function_or_relative<'i>(
    input: &mut Parser<'i, '_>,
    color_mix_depth: usize,
) -> Result<ParsedColor, ParseError<'i, ()>> {
    let start = input.state();
    if input.try_parse(|i| i.expect_ident_matching("from")).is_ok() {
        parse_relative_color_after_from(input, RelativeColorKind::Rgb, color_mix_depth)
    } else {
        input.reset(&start);
        parse_rgb_function(input)
    }
}

fn parse_hsl_function_or_relative<'i>(
    input: &mut Parser<'i, '_>,
    color_mix_depth: usize,
) -> Result<ParsedColor, ParseError<'i, ()>> {
    let start = input.state();
    if input.try_parse(|i| i.expect_ident_matching("from")).is_ok() {
        parse_relative_color_after_from(input, RelativeColorKind::Hsl, color_mix_depth)
    } else {
        input.reset(&start);
        parse_hsl_function(input)
    }
}

fn parse_hwb_function_or_relative<'i>(
    input: &mut Parser<'i, '_>,
    color_mix_depth: usize,
) -> Result<ParsedColor, ParseError<'i, ()>> {
    let start = input.state();
    if input.try_parse(|i| i.expect_ident_matching("from")).is_ok() {
        parse_relative_color_after_from(input, RelativeColorKind::Hwb, color_mix_depth)
    } else {
        input.reset(&start);
        parse_hwb_function(input)
    }
}

fn parse_lab_function_or_relative<'i>(
    input: &mut Parser<'i, '_>,
    function: LabFunction,
    color_mix_depth: usize,
) -> Result<ParsedColor, ParseError<'i, ()>> {
    let start = input.state();
    let relative_kind = match function {
        LabFunction::Lab => RelativeColorKind::Lab,
        LabFunction::Lch => RelativeColorKind::Lch,
        LabFunction::Oklab => RelativeColorKind::Oklab,
        LabFunction::Oklch => RelativeColorKind::Oklch,
    };
    if input.try_parse(|i| i.expect_ident_matching("from")).is_ok() {
        parse_relative_color_after_from(input, relative_kind, color_mix_depth)
    } else {
        input.reset(&start);
        parse_lab_function(input, function)
    }
}

fn parse_color_function_or_relative<'i>(
    input: &mut Parser<'i, '_>,
    color_mix_depth: usize,
) -> Result<ParsedColor, ParseError<'i, ()>> {
    let start = input.state();
    if input.try_parse(|i| i.expect_ident_matching("from")).is_ok() {
        parse_relative_color_after_from(input, RelativeColorKind::ColorRgb, color_mix_depth)
    } else {
        input.reset(&start);
        parse_color_function(input)
    }
}

fn relative_kind_for_color_space(name: &str) -> Option<RelativeColorKind> {
    match name.to_ascii_lowercase().as_str() {
        "srgb" | "srgb-linear" | "a98-rgb" | "display-p3" | "display-p3-linear" | "rec2020"
        | "prophoto-rgb" => Some(RelativeColorKind::ColorRgb),
        "xyz" | "xyz-d50" | "xyz-d65" => Some(RelativeColorKind::ColorXyz),
        _ => None,
    }
}

fn relative_ident_allowed(kind: RelativeColorKind, name: &str) -> bool {
    match kind {
        RelativeColorKind::Rgb | RelativeColorKind::ColorRgb => {
            matches!(name, "r" | "g" | "b" | "alpha")
        }
        RelativeColorKind::Alpha => name == "alpha",
        RelativeColorKind::Hsl => matches!(name, "h" | "s" | "l" | "alpha"),
        RelativeColorKind::Hwb => matches!(name, "h" | "w" | "b" | "alpha"),
        RelativeColorKind::Lab | RelativeColorKind::Oklab => {
            matches!(name, "l" | "a" | "b" | "alpha")
        }
        RelativeColorKind::Lch | RelativeColorKind::Oklch => {
            matches!(name, "l" | "c" | "h" | "alpha")
        }
        RelativeColorKind::ColorXyz => matches!(name, "x" | "y" | "z" | "alpha"),
    }
}

fn relative_component_is_hue(kind: RelativeColorKind, index: usize) -> bool {
    match kind {
        RelativeColorKind::Hsl | RelativeColorKind::Hwb => index == 0,
        RelativeColorKind::Lch | RelativeColorKind::Oklch => index == 2,
        _ => false,
    }
}

pub(crate) fn relative_math_function(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "calc"
            | "min"
            | "max"
            | "clamp"
            | "sign"
            | "abs"
            | "round"
            | "mod"
            | "rem"
            | "pow"
            | "sqrt"
            | "hypot"
            | "log"
            | "exp"
            | "sin"
            | "cos"
            | "tan"
            | "asin"
            | "acos"
            | "atan"
            | "atan2"
    )
}

fn relative_ident_math_type(
    kind: RelativeColorKind,
    _index: usize,
    name: &str,
) -> Option<ColorMathType> {
    if relative_ident_allowed(kind, name) {
        // Relative channel identifiers are exposed as unitless channel values.
        // A percentage or angle can still be produced by multiplying/dividing
        // them inside calc(), but it cannot be added directly to a number.
        Some(ColorMathType::Number)
    } else {
        None
    }
}

fn relative_math_expression_type<'i>(
    input: &mut Parser<'i, '_>,
    kind: RelativeColorKind,
    index: usize,
    allow_comma: bool,
) -> Result<ColorMathType, ParseError<'i, ()>> {
    let mut terms = Vec::new();
    loop {
        let token = match input.next() {
            Ok(token) => token.clone(),
            Err(_) => break,
        };
        match token {
            Token::Number { .. } => terms.push(MathTerm::Value(ColorMathType::Number)),
            Token::Percentage { .. } => terms.push(MathTerm::Value(ColorMathType::Percentage)),
            Token::Dimension { ref unit, .. } => {
                terms.push(MathTerm::Value(color_math_dimension_type(unit.as_ref())))
            }
            Token::Ident(ref name) if is_math_constant(name.as_ref()) => {
                terms.push(MathTerm::Value(ColorMathType::Number));
            }
            Token::Ident(ref name) => terms.push(MathTerm::Value(
                relative_ident_math_type(kind, index, name.as_ref())
                    .unwrap_or(ColorMathType::Invalid),
            )),
            Token::Function(ref name) => {
                let name_lower = name.as_ref().to_ascii_lowercase();
                let function_type = if name_lower == "var" || name_lower == "env" {
                    input.parse_nested_block(|nested| {
                        consume_math_component_values(nested)?;
                        Ok::<_, ParseError<'_, ()>>(ColorMathType::Unknown)
                    })?
                } else {
                    let nested_allows_comma = matches!(
                        name_lower.as_str(),
                        "min" | "max" | "clamp" | "round" | "mod" | "rem" | "atan2"
                    );
                    let nested = input
                        .parse_nested_block(|nested| {
                            relative_math_expression_type(nested, kind, index, nested_allows_comma)
                        })
                        .unwrap_or(ColorMathType::Invalid);
                    if !relative_math_function(name_lower.as_ref()) {
                        ColorMathType::Invalid
                    } else {
                        match name_lower.as_str() {
                            "calc" | "min" | "max" | "clamp" | "abs" | "round" | "mod" | "rem" => {
                                nested
                            }
                            "sign" | "pow" | "sqrt" | "hypot" | "log" | "exp" | "sin" | "cos"
                            | "tan" | "asin" | "acos" | "atan" | "atan2" => {
                                if nested == ColorMathType::Invalid {
                                    ColorMathType::Invalid
                                } else {
                                    ColorMathType::Number
                                }
                            }
                            _ => ColorMathType::Invalid,
                        }
                    }
                };
                terms.push(MathTerm::Value(function_type));
            }
            Token::ParenthesisBlock => {
                let nested = input
                    .parse_nested_block(|nested| {
                        relative_math_expression_type(nested, kind, index, false)
                    })
                    .unwrap_or(ColorMathType::Invalid);
                terms.push(MathTerm::Value(nested));
            }
            Token::Delim('+' | '-' | '*' | '/') => {
                if let Token::Delim(operator) = token {
                    terms.push(MathTerm::Operator(operator));
                }
            }
            Token::Comma if allow_comma => terms.push(MathTerm::Comma),
            Token::WhiteSpace(_) | Token::Comment(_) => {}
            _ => terms.push(MathTerm::Value(ColorMathType::Invalid)),
        }
    }
    Ok(MathTermsParser::new(terms).parse_all(allow_comma))
}

fn parse_relative_component<'i>(
    input: &mut Parser<'i, '_>,
    kind: RelativeColorKind,
    index: usize,
) -> Result<(), ParseError<'i, ()>> {
    let token = input.next()?.clone();
    let is_hue = relative_component_is_hue(kind, index);
    match token {
        Token::Ident(ref name) if name.eq_ignore_ascii_case("none") => Ok(()),
        Token::Ident(ref name) if relative_ident_allowed(kind, name.as_ref()) => Ok(()),
        Token::Number { .. } => Ok(()),
        Token::Percentage { .. } if !is_hue => Ok(()),
        Token::Dimension { ref unit, .. } if is_angle_unit(unit.as_ref()) && is_hue => Ok(()),
        Token::Function(ref name)
            if relative_math_function(name.as_ref())
                || name.eq_ignore_ascii_case("var")
                || name.eq_ignore_ascii_case("env") =>
        {
            let name_lower = name.as_ref().to_ascii_lowercase();
            let value = if name_lower == "var" || name_lower == "env" {
                input.parse_nested_block(|nested| {
                    consume_math_component_values(nested)?;
                    Ok::<_, ParseError<'_, ()>>(ColorMathType::Unknown)
                })?
            } else {
                let allows_comma = matches!(
                    name_lower.as_str(),
                    "min" | "max" | "clamp" | "round" | "mod" | "rem" | "atan2"
                );
                input
                    .parse_nested_block(|nested| {
                        relative_math_expression_type(nested, kind, index, allows_comma)
                    })
                    .map_err(|_| input.new_custom_error(()))?
            };
            let allowed = matches!(value, ColorMathType::Unknown)
                || if is_hue {
                    matches!(value, ColorMathType::Number | ColorMathType::Angle)
                } else {
                    matches!(value, ColorMathType::Number | ColorMathType::Percentage)
                };
            if allowed {
                Ok(())
            } else {
                Err(input.new_custom_error(()))
            }
        }
        _ => Err(input.new_custom_error(())),
    }
}

fn parse_relative_var_origin<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<ParsedColor, ParseError<'i, ()>> {
    let token = input.next()?.clone();
    let Token::Function(ref name) = token else {
        return Err(input.new_custom_error(()));
    };
    if !name.eq_ignore_ascii_case("var") {
        return Err(input.new_custom_error(()));
    }
    input.parse_nested_block(|nested| {
        let custom_name = nested.expect_ident()?.clone();
        if !custom_name.as_ref().starts_with("--") {
            return Err(nested.new_custom_error(()));
        }
        if nested.try_parse(|i| i.expect_comma()).is_ok() {
            // The fallback is a component-value list. Its eventual color
            // validity is context-dependent, but it must not be empty syntax.
            let fallback_start = nested.state();
            consume_math_component_values(nested)?;
            if nested.state().position() == fallback_start.position() {
                return Err(nested.new_custom_error(()));
            }
        }
        nested
            .expect_exhausted()
            .map_err(|_| nested.new_custom_error(()))?;
        Ok(())
    })?;
    Ok(ParsedColor::from_css_color(CssColor::BLACK))
}

fn parse_relative_origin<'i>(
    input: &mut Parser<'i, '_>,
    color_mix_depth: usize,
) -> Result<ParsedColor, ParseError<'i, ()>> {
    let origin_start = input.state();
    if let Some(origin) = parse_color_float(input, color_mix_depth) {
        Ok(origin)
    } else {
        input.reset(&origin_start);
        parse_relative_var_origin(input)
    }
}

fn parse_relative_color_after_from<'i>(
    input: &mut Parser<'i, '_>,
    kind: RelativeColorKind,
    color_mix_depth: usize,
) -> Result<ParsedColor, ParseError<'i, ()>> {
    let origin = parse_relative_origin(input, color_mix_depth)?;
    let kind = if matches!(kind, RelativeColorKind::ColorRgb) {
        let target = input.expect_ident()?.clone();
        relative_kind_for_color_space(target.as_ref()).ok_or_else(|| input.new_custom_error(()))?
    } else {
        kind
    };
    for index in 0..3 {
        parse_relative_component(input, kind, index)?;
    }
    if input.try_parse(|i| i.expect_delim('/')).is_ok() {
        parse_relative_component(input, kind, 3)?;
    }
    // The context-free style model cannot retain a relative expression or
    // resolve currentColor/variables. Returning the origin preserves the
    // existing bounded CssColor representation while the grammar above does
    // the important parse-time validation.
    Ok(origin)
}

/// `<color-space>` (CSS Color 4 §13.2 "Color Space for Interpolation"
/// <https://www.w3.org/TR/css-color-4/#color-interpolation-method>)。
/// `color-mix()`では、bounded sRGB modelへ変換できる interpolation-space
/// identifiers も構文上受理する。`hsl`/`hwb` は円筒座標で補間して sRGB へ戻し、
/// wide-gamut identifiers は既存の bounded sRGB fallback を使う。Gradient
/// callers は CSS Images 側の実装範囲を保つため、`hsl`/`hwb` と wide-gamut
/// spaces を別途拒否する。
///
/// `color-mix()` (本 type の元々の用途、[`parse_mix_color_space`]) と
/// CSS Images 4 gradient function 群の `in <color-space>
/// <hue-interpolation-method>?` 節 ([`GradientColorInterpolation`]、
/// [`parse_gradient_color_interpolation`](super::visual::parse_gradient_color_interpolation)) で共有する — 両 host syntax が
/// 同じ exported `<color-space>` production を参照するため、1 つの enum で
/// 両方を賄う。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MixColorSpace {
    /// `srgb` — gamma-encoded sRGB。CSS の legacy default interpolation
    /// space。
    Srgb,
    /// `srgb-linear` — linear-light sRGB。
    SrgbLinear,
    /// `hsl` — cylindrical HSL coordinates with hue interpolation。
    Hsl,
    /// `hwb` — cylindrical HWB coordinates with hue interpolation。
    Hwb,
    /// `lab` — CIE Lab (rectangular)。
    Lab,
    /// `lch` — CIE LCH (polar、[`HueInterpolationMethod`] を受理)。
    Lch,
    /// `oklab` — Oklab (rectangular)。
    Oklab,
    /// `oklch` — Oklch (polar、[`HueInterpolationMethod`] を受理)。
    Oklch,
}

/// `<hue-interpolation-method>` (CSS Color 4 §13.2
/// <https://www.w3.org/TR/css-color-4/#color-interpolation-method>) —
/// `[ shorter | longer | increasing | decreasing ] hue`。polar な
/// [`MixColorSpace`] (`Lch`/`Oklch`) に対してのみ意味を持ち、それ以外では
/// caller が reject する ([`parse_color_mix_function`]、
/// [`parse_gradient_color_interpolation`](super::visual::parse_gradient_color_interpolation))。`color-mix()` と gradient の
/// `<color-interpolation-method>` で共有する理由は [`MixColorSpace`] と同じ。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HueInterpolationMethod {
    /// `shorter hue` — 短い方の弧で補間する。省略時のこの production 自体の
    /// default。
    Shorter,
    /// `longer hue` — 長い方の弧で補間する。
    Longer,
    /// `increasing hue` — hue 角度が単調増加する方向で補間する。
    Increasing,
    /// `decreasing hue` — hue 角度が単調減少する方向で補間する。
    Decreasing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ParsedColorSpace {
    Srgb,
    SrgbLinear,
    Lab,
    Lch,
    Oklab,
    Oklch,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum LightnessBoundary {
    Lower,
    Upper,
}

impl LightnessBoundary {
    fn from_coordinates(space: ParsedColorSpace, lightness: f32) -> Option<Self> {
        match space {
            ParsedColorSpace::Lab | ParsedColorSpace::Lch if lightness <= 0.0 => Some(Self::Lower),
            ParsedColorSpace::Lab | ParsedColorSpace::Lch if lightness >= 100.0 => {
                Some(Self::Upper)
            }
            ParsedColorSpace::Oklab | ParsedColorSpace::Oklch if lightness <= 0.0 => {
                Some(Self::Lower)
            }
            ParsedColorSpace::Oklab | ParsedColorSpace::Oklch if lightness >= 1.0 => {
                Some(Self::Upper)
            }
            _ => None,
        }
    }

    fn srgb_coordinates(self) -> [f32; 3] {
        match self {
            Self::Lower => [0.0; 3],
            Self::Upper => [1.0; 3],
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ColorCoordinates {
    pub(crate) first: f32,
    pub(crate) second: f32,
    pub(crate) third: f32,
    pub(crate) alpha: f32,
    pub(crate) polar_hue_missing: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct ParsedColor {
    pub(crate) coordinates: [f32; 3],
    pub(crate) alpha: f32,
    pub(crate) space: ParsedColorSpace,
    pub(crate) lightness_boundary: Option<LightnessBoundary>,
    pub(crate) polar_hue_missing: bool,
}

impl ParsedColor {
    fn from_css_color(color: CssColor) -> Self {
        Self::from_coordinates(
            ParsedColorSpace::Srgb,
            [
                f32::from(color.r) / 255.0,
                f32::from(color.g) / 255.0,
                f32::from(color.b) / 255.0,
            ],
            f32::from(color.a) / 255.0,
        )
    }

    pub(crate) fn from_coordinates(
        space: ParsedColorSpace,
        coordinates: [f32; 3],
        alpha: f32,
    ) -> Self {
        Self {
            coordinates,
            alpha,
            space,
            lightness_boundary: None,
            polar_hue_missing: false,
        }
    }

    fn from_lab_coordinates(space: ParsedColorSpace, coordinates: [f32; 3], alpha: f32) -> Self {
        Self {
            coordinates,
            alpha,
            space,
            lightness_boundary: LightnessBoundary::from_coordinates(space, coordinates[0]),
            polar_hue_missing: false,
        }
    }

    fn from_interpolation_coordinates_with_boundary(
        space: ParsedColorSpace,
        coordinates: [f32; 3],
        alpha: f32,
        lightness_boundary: Option<LightnessBoundary>,
        polar_hue_missing: bool,
    ) -> Self {
        // A generated lightness value outside its nominal range is still raw
        // interpolation data. Only caller-supplied endpoint provenance may
        // request the CSS boundary mapping.
        let parsed = Self {
            coordinates,
            alpha,
            space,
            lightness_boundary,
            polar_hue_missing,
        };
        if lightness_boundary.is_some()
            || matches!(
                space,
                ParsedColorSpace::Lab
                    | ParsedColorSpace::Lch
                    | ParsedColorSpace::Oklab
                    | ParsedColorSpace::Oklch
            )
        {
            return parsed;
        }
        // Preserve the legacy float-sRGB path for in-gamut values; retain
        // interpolation coordinates only when final sRGB conversion is out
        // of gamut, so nested mixes do not clip them early.
        let srgb = parsed.to_srgb_for_interpolation();
        if srgb.iter().all(|component| (0.0..=1.0).contains(component)) {
            Self::from_coordinates(ParsedColorSpace::Srgb, srgb, alpha)
        } else {
            parsed
        }
    }

    pub(crate) fn to_css_color(self) -> CssColor {
        rgb_f32_to_css_color(self.to_srgb(), self.alpha)
    }

    pub(crate) fn to_srgb(self) -> [f32; 3] {
        // Boundary provenance requests CSS Color's fixed black/white mapping;
        // retained interpolation coordinates otherwise use unbounded conversion.
        if let Some(boundary) = self.lightness_boundary {
            return boundary.srgb_coordinates();
        }
        match self.space {
            ParsedColorSpace::Srgb => self.coordinates,
            ParsedColorSpace::SrgbLinear => self.coordinates.map(srgb_encode),
            ParsedColorSpace::Lab => lab_to_srgb_unbounded(self.coordinates),
            ParsedColorSpace::Lch => lab_to_srgb_unbounded(polar_to_rectangular(self.coordinates)),
            ParsedColorSpace::Oklab => oklab_to_srgb_unbounded(self.coordinates),
            ParsedColorSpace::Oklch => {
                oklab_to_srgb_unbounded(polar_to_rectangular(self.coordinates))
            }
        }
    }

    fn to_srgb_for_interpolation(self) -> [f32; 3] {
        match self.space {
            ParsedColorSpace::Srgb => self.coordinates,
            ParsedColorSpace::SrgbLinear => self.coordinates.map(srgb_encode),
            ParsedColorSpace::Lab => lab_to_srgb_unbounded(self.coordinates),
            ParsedColorSpace::Lch => lab_to_srgb_unbounded(polar_to_rectangular(self.coordinates)),
            ParsedColorSpace::Oklab => oklab_to_srgb_unbounded(self.coordinates),
            ParsedColorSpace::Oklch => {
                oklab_to_srgb_unbounded(polar_to_rectangular(self.coordinates))
            }
        }
    }

    pub(crate) fn to_srgb_linear(self) -> [f32; 3] {
        match self.space {
            ParsedColorSpace::Srgb => self.coordinates.map(srgb_decode),
            ParsedColorSpace::SrgbLinear => self.coordinates,
            ParsedColorSpace::Lab => lab_to_srgb_linear(self.coordinates),
            ParsedColorSpace::Lch => lab_to_srgb_linear(polar_to_rectangular(self.coordinates)),
            ParsedColorSpace::Oklab => oklab_to_srgb_linear(self.coordinates),
            ParsedColorSpace::Oklch => oklab_to_srgb_linear(polar_to_rectangular(self.coordinates)),
        }
    }

    pub(crate) fn to_lab(self) -> [f32; 3] {
        match self.space {
            ParsedColorSpace::Srgb => srgb_to_lab(self.coordinates),
            ParsedColorSpace::SrgbLinear => linear_srgb_to_lab(self.coordinates),
            ParsedColorSpace::Lab => self.coordinates,
            ParsedColorSpace::Lch => polar_to_rectangular(self.coordinates),
            ParsedColorSpace::Oklab => linear_srgb_to_lab(oklab_to_srgb_linear(self.coordinates)),
            ParsedColorSpace::Oklch => {
                linear_srgb_to_lab(oklab_to_srgb_linear(polar_to_rectangular(self.coordinates)))
            }
        }
    }

    pub(crate) fn to_oklab(self) -> [f32; 3] {
        match self.space {
            ParsedColorSpace::Srgb => srgb_to_oklab(self.coordinates),
            ParsedColorSpace::SrgbLinear => linear_srgb_to_oklab(self.coordinates),
            ParsedColorSpace::Lab => linear_srgb_to_oklab(lab_to_srgb_linear(self.coordinates)),
            ParsedColorSpace::Lch => {
                linear_srgb_to_oklab(lab_to_srgb_linear(polar_to_rectangular(self.coordinates)))
            }
            ParsedColorSpace::Oklab => self.coordinates,
            ParsedColorSpace::Oklch => polar_to_rectangular(self.coordinates),
        }
    }

    fn to_coordinates(self, space: MixColorSpace) -> ColorCoordinates {
        let (coordinates, polar_hue_missing) = match (self.space, space) {
            (ParsedColorSpace::Srgb, MixColorSpace::Srgb)
            | (ParsedColorSpace::SrgbLinear, MixColorSpace::SrgbLinear)
            | (ParsedColorSpace::Lab, MixColorSpace::Lab)
            | (ParsedColorSpace::Oklab, MixColorSpace::Oklab) => (self.coordinates, false),
            (ParsedColorSpace::Lch, MixColorSpace::Lch) => (
                normalize_polar(self.coordinates, false, self.polar_hue_missing),
                self.polar_hue_missing,
            ),
            (ParsedColorSpace::Oklch, MixColorSpace::Oklch) => (
                normalize_polar(self.coordinates, true, self.polar_hue_missing),
                self.polar_hue_missing,
            ),
            (_, MixColorSpace::Hsl) => {
                let coordinates = srgb_to_hsl(self.to_srgb_for_interpolation());
                (coordinates, coordinates[1] == 0.0)
            }
            (_, MixColorSpace::Hwb) => {
                let coordinates = srgb_to_hwb(self.to_srgb_for_interpolation());
                (coordinates, coordinates[1] + coordinates[2] >= 1.0)
            }
            (_, MixColorSpace::Srgb) => (self.to_srgb_for_interpolation(), false),
            (_, MixColorSpace::SrgbLinear) => (self.to_srgb_linear(), false),
            (_, MixColorSpace::Lab) => (self.to_lab(), false),
            (_, MixColorSpace::Lch) => {
                let coordinates = rectangular_to_polar(self.to_lab(), false);
                (coordinates, polar_hue_missing_for_space(false, coordinates))
            }
            (_, MixColorSpace::Oklab) => (self.to_oklab(), false),
            (_, MixColorSpace::Oklch) => {
                let coordinates = rectangular_to_polar(self.to_oklab(), true);
                (coordinates, polar_hue_missing_for_space(true, coordinates))
            }
        };
        let [first, second, third] = coordinates;
        ColorCoordinates {
            first,
            second,
            third,
            alpha: self.alpha,
            polar_hue_missing,
        }
    }
}

impl From<MixColorSpace> for ParsedColorSpace {
    fn from(space: MixColorSpace) -> Self {
        match space {
            MixColorSpace::Srgb => Self::Srgb,
            MixColorSpace::SrgbLinear => Self::SrgbLinear,
            MixColorSpace::Hsl | MixColorSpace::Hwb => Self::Srgb,
            MixColorSpace::Lab => Self::Lab,
            MixColorSpace::Lch => Self::Lch,
            MixColorSpace::Oklab => Self::Oklab,
            MixColorSpace::Oklch => Self::Oklch,
        }
    }
}

fn polar_to_rectangular([lightness, chroma, hue]: [f32; 3]) -> [f32; 3] {
    [lightness, chroma * hue.cos(), chroma * hue.sin()]
}

fn polar_hue_missing_for_space(is_oklab: bool, coordinates: [f32; 3]) -> bool {
    let chroma_threshold = if is_oklab { 0.000004 } else { 0.0015 };
    coordinates[1] <= chroma_threshold
}

fn normalize_polar(
    [lightness, chroma, hue]: [f32; 3],
    is_oklab: bool,
    polar_hue_missing: bool,
) -> [f32; 3] {
    let chroma_threshold = if is_oklab { 0.000004 } else { 0.0015 };
    if chroma <= chroma_threshold {
        [
            lightness,
            if polar_hue_missing { 0.0 } else { chroma },
            if polar_hue_missing {
                0.0
            } else {
                hue.rem_euclid(std::f32::consts::TAU)
            },
        ]
    } else {
        [lightness, chroma, hue.rem_euclid(std::f32::consts::TAU)]
    }
}

fn rectangular_to_polar([lightness, a, b]: [f32; 3], is_oklab: bool) -> [f32; 3] {
    let chroma = a.hypot(b);
    let chroma_threshold = if is_oklab { 0.000004 } else { 0.0015 };
    if chroma <= chroma_threshold {
        [lightness, 0.0, 0.0]
    } else {
        [
            lightness,
            chroma,
            b.atan2(a).rem_euclid(std::f32::consts::TAU),
        ]
    }
}

fn parse_color_function<'i>(input: &mut Parser<'i, '_>) -> Result<ParsedColor, ParseError<'i, ()>> {
    // Bounded CSS Color 4 support: the two sRGB spaces are representable by
    // CssColor without widening the public value model. Wide-gamut predefined
    // spaces remain a follow-up because this parser stores only 8-bit sRGB.
    let color_space = input.expect_ident()?.clone();
    let first = parse_color_component(input, 1.0)?;
    let second = parse_color_component(input, 1.0)?;
    let third = parse_color_component(input, 1.0)?;
    let alpha = parse_optional_modern_alpha(input)?;

    let (space, coordinates) = match color_space.as_ref().to_ascii_lowercase().as_str() {
        // CSS Color 4 §10.1 says out-of-gamut `color()` components are valid
        // and retained for intermediate computations. Keep authored `srgb`
        // coordinates raw; generated color-mix() values use the same path.
        "srgb" => (ParsedColorSpace::Srgb, [first, second, third]),
        // CSS Color 4 §10.1 also retains out-of-gamut linear-sRGB
        // components. Keep them in their declared space so srgb-linear
        // interpolation sees the raw coordinates; serialization converts and
        // bounds them only at the final sRGB/u8 sink.
        "srgb-linear" => (ParsedColorSpace::SrgbLinear, [first, second, third]),
        // For css-color parsing coverage (WPT), accept other predefined
        // spaces as srgb fallback (treat coordinates as srgb). This allows
        // `none` vectors in those spaces to count as valid without
        // widening the public value model to 8-bit gamut for those spaces.
        "a98-rgb" | "display-p3" | "display-p3-linear" | "rec2020" | "prophoto-rgb" | "xyz"
        | "xyz-d50" | "xyz-d65" => (ParsedColorSpace::Srgb, [first, second, third]),
        _ => return Err(input.new_custom_error(())),
    };
    Ok(ParsedColor::from_coordinates(space, coordinates, alpha))
}

fn parse_lab_function<'i>(
    input: &mut Parser<'i, '_>,
    function: LabFunction,
) -> Result<ParsedColor, ParseError<'i, ()>> {
    let is_oklab = matches!(function, LabFunction::Oklab | LabFunction::Oklch);
    let lightness = parse_lightness(input, is_oklab)?;
    let component_scale = match function {
        LabFunction::Lab => 125.0,
        LabFunction::Lch => 150.0,
        LabFunction::Oklab | LabFunction::Oklch => 0.4,
    };
    let mut second = parse_color_component(input, component_scale)?;
    let third = if matches!(function, LabFunction::Lch | LabFunction::Oklch) {
        second = second.max(0.0);
        parse_hue(input)?
    } else {
        parse_color_component(input, component_scale)?
    };
    let alpha = parse_optional_modern_alpha(input)?;

    let (space, coordinates) = match function {
        LabFunction::Lab => (ParsedColorSpace::Lab, [lightness, second, third]),
        LabFunction::Lch => (ParsedColorSpace::Lch, [lightness, second, third]),
        LabFunction::Oklab => (ParsedColorSpace::Oklab, [lightness, second, third]),
        LabFunction::Oklch => (ParsedColorSpace::Oklch, [lightness, second, third]),
    };
    Ok(ParsedColor::from_lab_coordinates(space, coordinates, alpha))
}

fn parse_hsl_function<'i>(input: &mut Parser<'i, '_>) -> Result<ParsedColor, ParseError<'i, ()>> {
    // CSS Color 4 keeps the legacy comma form alongside the modern
    // space-separated form. The legacy grammar requires percentage
    // saturation/lightness and does not permit `none`.
    let hue_is_none = input.try_parse(|i| i.expect_ident_matching("none")).is_ok();
    let hue = if hue_is_none { 0.0 } else { parse_hue(input)? };
    if input.try_parse(|i| i.expect_comma()).is_ok() {
        if hue_is_none {
            return Err(input.new_custom_error(()));
        }
        let saturation = expect_percentage_stable(input)?.clamp(0.0, 1.0);
        input.expect_comma()?;
        let lightness = expect_percentage_stable(input)?.clamp(0.0, 1.0);
        let alpha = if input.try_parse(|i| i.expect_comma()).is_ok() {
            parse_alpha_value(input)?
        } else {
            1.0
        };
        return Ok(ParsedColor::from_coordinates(
            ParsedColorSpace::Srgb,
            hsl_to_srgb(hue, saturation, lightness),
            alpha,
        ));
    }

    let saturation = parse_hsl_percentage_or_number(input)?;
    let lightness = parse_hsl_percentage_or_number(input)?;
    let alpha = parse_optional_modern_alpha(input)?;
    Ok(ParsedColor::from_coordinates(
        ParsedColorSpace::Srgb,
        hsl_to_srgb(hue, saturation, lightness),
        alpha,
    ))
}

fn parse_hsl_percentage_or_number<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<f32, ParseError<'i, ()>> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        Ok(0.0)
    } else if let Ok((kind, value)) =
        input.try_parse(|i| parse_color_math_value(i, ColorMathContext::NumberOrPercentage))
    {
        Ok(if kind == ColorMathType::Percentage {
            value.clamp(0.0, 1.0)
        } else {
            (value / 100.0).clamp(0.0, 1.0)
        })
    } else if let Ok(pct) = input.try_parse(|i| expect_percentage_stable(i)) {
        Ok(pct.clamp(0.0, 1.0))
    } else {
        Ok((expect_number_stable(input)? / 100.0).clamp(0.0, 1.0))
    }
}

fn hsl_to_srgb(hue: f32, saturation: f32, lightness: f32) -> [f32; 3] {
    if saturation == 0.0 {
        return [lightness; 3];
    }
    let q = if lightness < 0.5 {
        lightness * (1.0 + saturation)
    } else {
        lightness + saturation - lightness * saturation
    };
    let p = 2.0 * lightness - q;
    let hue_to_channel = |mut t: f32| {
        t = t.rem_euclid(1.0);
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 1.0 / 2.0 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    [
        hue_to_channel(hue / (2.0 * std::f32::consts::PI) + 1.0 / 3.0),
        hue_to_channel(hue / (2.0 * std::f32::consts::PI)),
        hue_to_channel(hue / (2.0 * std::f32::consts::PI) - 1.0 / 3.0),
    ]
}

fn srgb_to_hsl(rgb: [f32; 3]) -> [f32; 3] {
    let [r, g, b] = rgb.map(|component| component.clamp(0.0, 1.0));
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let lightness = (max + min) * 0.5;
    if delta == 0.0 {
        return [0.0, 0.0, lightness];
    }
    let saturation = delta / (1.0 - (2.0 * lightness - 1.0).abs());
    let hue_degrees = if max == r {
        60.0 * ((g - b) / delta).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / delta + 2.0)
    } else {
        60.0 * ((r - g) / delta + 4.0)
    };
    [hue_degrees.to_radians(), saturation, lightness]
}

fn srgb_to_hwb(rgb: [f32; 3]) -> [f32; 3] {
    let rgb = rgb.map(|component| component.clamp(0.0, 1.0));
    let [hue, _, _] = srgb_to_hsl(rgb);
    [
        hue,
        rgb[0].min(rgb[1]).min(rgb[2]),
        1.0 - rgb[0].max(rgb[1]).max(rgb[2]),
    ]
}

fn hwb_to_srgb(hue: f32, whiteness: f32, blackness: f32) -> [f32; 3] {
    let whiteness = whiteness.clamp(0.0, 1.0);
    let blackness = blackness.clamp(0.0, 1.0);
    if whiteness + blackness >= 1.0 {
        let gray = whiteness / (whiteness + blackness);
        [gray; 3]
    } else {
        let scale = 1.0 - whiteness - blackness;
        hsl_to_srgb(hue, 1.0, 0.5).map(|channel| channel * scale + whiteness)
    }
}

fn parse_hwb_function<'i>(input: &mut Parser<'i, '_>) -> Result<ParsedColor, ParseError<'i, ()>> {
    let hue = parse_hue(input)?;
    let whiteness = parse_hsl_percentage_or_number(input)?;
    let blackness = parse_hsl_percentage_or_number(input)?;
    let alpha = parse_optional_modern_alpha(input)?;
    Ok(ParsedColor::from_coordinates(
        ParsedColorSpace::Srgb,
        hwb_to_srgb(hue, whiteness, blackness),
        alpha,
    ))
}

/// Parse one `<color-stop>` with its optional percentage in either order.
///
/// CSS Color 4 permits `<percentage> <color>` as well as the existing
/// `<color> <percentage>` spelling, but a stop may contain only one percentage.
fn parse_color_mix_stop<'i>(
    input: &mut Parser<'i, '_>,
    color_mix_depth: usize,
) -> Result<(ParsedColor, Option<f32>), ParseError<'i, ()>> {
    let leading_percentage = input.try_parse(parse_mix_percentage).ok();
    let color =
        parse_color_float(input, color_mix_depth).ok_or_else(|| input.new_custom_error(()))?;
    let trailing_percentage = input.try_parse(parse_mix_percentage).ok();
    if leading_percentage.is_some() && trailing_percentage.is_some() {
        return Err(input.new_custom_error(()));
    }
    Ok((color, leading_percentage.or(trailing_percentage)))
}

/// `in <color-space> <hue-interpolation-method>?` (CSS Color 4 §13.2
/// "Color Space for Interpolation" — [`MixColorSpace`] doc参照) の共通
/// parse + validation。`<hue-interpolation-method>` は polar な
/// `<color-space>` にのみ許され、省略時は
/// [`HueInterpolationMethod::Shorter`]がdefault。`allow_hsl_hwb` は
/// `color-mix()` では true、gradient の
/// `<color-interpolation-method>` ([`parse_gradient_color_interpolation`](super::visual::parse_gradient_color_interpolation))
/// では false とし、host grammar の実装範囲を保つ。
pub(super) fn parse_color_interpolation_method<'i>(
    input: &mut Parser<'i, '_>,
    allow_hsl_hwb: bool,
) -> Result<(MixColorSpace, HueInterpolationMethod), ParseError<'i, ()>> {
    input.expect_ident_matching("in")?;
    let color_space = parse_mix_color_space(input, allow_hsl_hwb)?;
    let explicit_hue_method = input.try_parse(parse_hue_interpolation_method).ok();
    if explicit_hue_method.is_some()
        && !matches!(
            color_space,
            MixColorSpace::Hsl | MixColorSpace::Hwb | MixColorSpace::Lch | MixColorSpace::Oklch
        )
    {
        return Err(input.new_custom_error(()));
    }
    Ok((
        color_space,
        explicit_hue_method.unwrap_or(HueInterpolationMethod::Shorter),
    ))
}

fn parse_color_mix_function<'i>(
    input: &mut Parser<'i, '_>,
    color_mix_depth: usize,
) -> Result<ParsedColor, ParseError<'i, ()>> {
    // CSS Color 5 defaults an omitted interpolation method to sRGB. Keep the
    // shared helper strict for gradient callers, but make this color-mix
    // shorthand explicit here (`color-mix(red, blue)`).
    let interpolation_method = input.try_parse(|i| parse_color_interpolation_method(i, true));
    let has_interpolation_method = interpolation_method.is_ok();
    let (color_space, hue_interpolation_method) =
        interpolation_method.unwrap_or((MixColorSpace::Srgb, HueInterpolationMethod::Shorter));
    if has_interpolation_method {
        input.expect_comma()?;
    }

    // The current CSS Color 5 grammar accepts one or more color stops. Keep
    // parsing until the enclosing function block is exhausted; a comma with
    // no following stop naturally becomes an error rather than being silently
    // accepted as a trailing comma.
    let mut stops = vec![parse_color_mix_stop(input, color_mix_depth)?];
    while input.try_parse(|i| i.expect_comma()).is_ok() {
        stops.push(parse_color_mix_stop(input, color_mix_depth)?);
    }

    // A calc()-derived stop percentage is syntactically valid but cannot be
    // evaluated in this context-free parser. Use a finite placeholder for the
    // interpolation bookkeeping and keep the deferred marker so a placeholder
    // zero cannot make an otherwise valid declaration fail at parse time.
    let has_deferred_percentage = stops
        .iter()
        .any(|(_, percentage)| percentage.is_some_and(|value| !value.is_finite()));
    let specified_sum: f32 = stops
        .iter()
        .filter_map(|(_, percentage)| percentage.filter(|value| value.is_finite()))
        .sum();
    let unspecified_count = stops
        .iter()
        .filter(|(_, percentage)| percentage.is_none())
        .count();
    let has_unspecified = unspecified_count != 0;
    let mut raw_weights = Vec::with_capacity(stops.len());
    if has_unspecified && specified_sum <= 1.0 {
        let remainder = (1.0 - specified_sum) / unspecified_count as f32;
        raw_weights.extend(stops.iter().map(|(_, percentage)| match percentage {
            Some(value) if value.is_finite() => *value,
            Some(_) => 1.0,
            None => remainder,
        }));
    } else {
        raw_weights.extend(stops.iter().map(|(_, percentage)| match percentage {
            Some(value) if value.is_finite() => *value,
            Some(_) => 1.0,
            None => 0.0,
        }));
    }
    let raw_sum: f32 = raw_weights.iter().sum();
    if raw_sum == 0.0 {
        return Err(input.new_custom_error(()));
    }
    // If every stop was explicitly assigned a total below 100%, the missing
    // portion is transparent. Otherwise normalize the interpolation weights
    // to the full color contribution.
    let alpha_multiplier = if !has_deferred_percentage && !has_unspecified && specified_sum < 1.0 {
        specified_sum
    } else {
        1.0
    };
    let weights: Vec<f32> = raw_weights.iter().map(|weight| weight / raw_sum).collect();

    let first = stops[0].0;
    let mut mixed = css_color_to_coordinates(first, color_space);
    let mut accumulated_weight = weights[0];
    for ((color, _), weight) in stops.iter().skip(1).zip(weights.iter().skip(1)) {
        let next = css_color_to_coordinates(*color, color_space);
        mixed = if hue_interpolation_method == HueInterpolationMethod::Shorter {
            mix_coordinates(mixed, next, accumulated_weight, *weight, color_space)
        } else {
            mix_coordinates_with_hue(
                mixed,
                next,
                accumulated_weight,
                *weight,
                color_space,
                hue_interpolation_method,
            )
        };
        accumulated_weight += *weight;
    }

    let lightness_boundary = if stops.len() == 2 {
        let first = css_color_to_coordinates(stops[0].0, color_space);
        let second = css_color_to_coordinates(stops[1].0, color_space);
        propagated_lightness_boundary(
            stops[0].0,
            stops[1].0,
            [weights[0], weights[1]],
            color_space,
            mixed.first,
            [first.first, second.first],
        )
    } else {
        None
    };
    let (parsed_space, coordinates, polar_hue_missing) = match color_space {
        // HSL/HWB are represented as cylindrical interpolation coordinates
        // while mixing, then converted back to the bounded sRGB model.
        MixColorSpace::Hsl => (
            ParsedColorSpace::Srgb,
            hsl_to_srgb(mixed.first, mixed.second, mixed.third),
            false,
        ),
        MixColorSpace::Hwb => (
            ParsedColorSpace::Srgb,
            hwb_to_srgb(mixed.first, mixed.second, mixed.third),
            false,
        ),
        _ => (
            color_space.into(),
            [mixed.first, mixed.second, mixed.third],
            mixed.polar_hue_missing,
        ),
    };
    Ok(ParsedColor::from_interpolation_coordinates_with_boundary(
        parsed_space,
        coordinates,
        mixed.alpha * alpha_multiplier,
        lightness_boundary,
        polar_hue_missing,
    ))
}

fn parse_hue_interpolation_method<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<HueInterpolationMethod, ParseError<'i, ()>> {
    let name = input.expect_ident()?.clone();
    let method = match name.as_ref().to_ascii_lowercase().as_str() {
        "shorter" => HueInterpolationMethod::Shorter,
        "longer" => HueInterpolationMethod::Longer,
        "increasing" => HueInterpolationMethod::Increasing,
        "decreasing" => HueInterpolationMethod::Decreasing,
        _ => return Err(input.new_custom_error(())),
    };
    input.expect_ident_matching("hue")?;
    Ok(method)
}

fn parse_mix_color_space<'i>(
    input: &mut Parser<'i, '_>,
    allow_hsl_hwb: bool,
) -> Result<MixColorSpace, ParseError<'i, ()>> {
    let name = input.expect_ident()?.clone();
    let name = name.as_ref().to_ascii_lowercase();
    if allow_hsl_hwb && name == "hsl" {
        return Ok(MixColorSpace::Hsl);
    }
    if allow_hsl_hwb && name == "hwb" {
        return Ok(MixColorSpace::Hwb);
    }
    match name.as_str() {
        "srgb" => Ok(MixColorSpace::Srgb),
        "srgb-linear" => Ok(MixColorSpace::SrgbLinear),
        "lab" => Ok(MixColorSpace::Lab),
        "lch" => Ok(MixColorSpace::Lch),
        "oklab" => Ok(MixColorSpace::Oklab),
        "oklch" => Ok(MixColorSpace::Oklch),
        // The bounded color model stores the result as sRGB. Accept the
        // remaining CSS Color 4 interpolation-space identifiers for syntax
        // coverage and use the same fallback as `parse_color_function` for
        // wide-gamut endpoint spaces.
        "a98-rgb" | "display-p3" | "display-p3-linear" | "rec2020" | "prophoto-rgb" | "xyz"
        | "xyz-d50" | "xyz-d65" => Ok(MixColorSpace::Srgb),
        _ => Err(input.new_custom_error(())),
    }
}

fn parse_mix_percentage<'i>(input: &mut Parser<'i, '_>) -> Result<f32, ParseError<'i, ()>> {
    if input
        .try_parse(|i| parse_color_math_value(i, ColorMathContext::Percentage))
        .is_ok()
    {
        // The exact weight is deferred. The caller treats NaN as a valid
        // syntactic percentage and substitutes a bookkeeping weight.
        return Ok(f32::NAN);
    }
    let percentage = expect_percentage_stable(input)?;
    if (0.0..=1.0).contains(&percentage) {
        Ok(percentage)
    } else {
        Err(input.new_custom_error(()))
    }
}

fn propagated_lightness_boundary(
    color_one: ParsedColor,
    color_two: ParsedColor,
    weights: [f32; 2],
    space: MixColorSpace,
    mixed_lightness: f32,
    endpoint_lightness: [f32; 2],
) -> Option<LightnessBoundary> {
    let [weight_one, weight_two] = weights;
    let [first_lightness, second_lightness] = endpoint_lightness;
    // Do not infer provenance from a generated result's lightness. It can be
    // outside the nominal range while still requiring raw interpolation.
    if weight_one == 1.0 && weight_two == 0.0 {
        return boundary_for_interpolation_space(
            space,
            color_one.space,
            color_one.lightness_boundary,
            mixed_lightness,
        );
    }
    if weight_one == 0.0 && weight_two == 1.0 {
        return boundary_for_interpolation_space(
            space,
            color_two.space,
            color_two.lightness_boundary,
            mixed_lightness,
        );
    }
    let effective_one = color_one.alpha * weight_one;
    let effective_two = color_two.alpha * weight_two;
    match (effective_one == 0.0, effective_two == 0.0) {
        (false, true) => boundary_for_interpolation_space(
            space,
            color_one.space,
            color_one.lightness_boundary,
            mixed_lightness,
        ),
        (true, false) => boundary_for_interpolation_space(
            space,
            color_two.space,
            color_two.lightness_boundary,
            mixed_lightness,
        ),
        (false, false) => {
            let boundary = match (color_one.lightness_boundary, color_two.lightness_boundary) {
                (Some(first), Some(second))
                    if first == second
                        && boundary_endpoint_is_structural(
                            color_one,
                            space,
                            first,
                            first_lightness,
                        )
                        && boundary_endpoint_is_structural(
                            color_two,
                            space,
                            first,
                            second_lightness,
                        ) =>
                {
                    Some(first)
                }
                (Some(boundary), None)
                    if boundary_endpoint_is_structural(
                        color_one,
                        space,
                        boundary,
                        first_lightness,
                    ) && boundary_endpoint_is_structural(
                        color_two,
                        space,
                        boundary,
                        second_lightness,
                    ) =>
                {
                    Some(boundary)
                }
                (None, Some(boundary))
                    if boundary_endpoint_is_structural(
                        color_one,
                        space,
                        boundary,
                        first_lightness,
                    ) && boundary_endpoint_is_structural(
                        color_two,
                        space,
                        boundary,
                        second_lightness,
                    ) =>
                {
                    Some(boundary)
                }
                _ => None,
            };
            boundary
                .filter(|boundary| lightness_boundary_matches(space, *boundary, mixed_lightness))
        }
        (true, true) => None,
    }
}

fn boundary_endpoint_is_structural(
    color: ParsedColor,
    space: MixColorSpace,
    boundary: LightnessBoundary,
    converted_lightness: f32,
) -> bool {
    if color.lightness_boundary.is_some() {
        return boundary_for_interpolation_space(
            space,
            color.space,
            Some(boundary),
            converted_lightness,
        ) == Some(boundary);
    }

    let target_is_lab = matches!(space, MixColorSpace::Lab | MixColorSpace::Lch);
    let target_is_oklab = matches!(space, MixColorSpace::Oklab | MixColorSpace::Oklch);
    let source_is_lab = matches!(color.space, ParsedColorSpace::Lab | ParsedColorSpace::Lch);
    let source_is_oklab = matches!(
        color.space,
        ParsedColorSpace::Oklab | ParsedColorSpace::Oklch
    );
    if source_is_lab && target_is_lab {
        let expected = match boundary {
            LightnessBoundary::Lower => 0.0,
            LightnessBoundary::Upper => 100.0,
        };
        let chroma = if matches!(color.space, ParsedColorSpace::Lch) {
            color.coordinates[1]
        } else {
            color.coordinates[1].hypot(color.coordinates[2])
        };
        return color.coordinates[0] == expected
            && chroma <= 0.0015
            && lightness_boundary_matches(space, boundary, converted_lightness);
    }
    if source_is_oklab && target_is_oklab {
        let expected = match boundary {
            LightnessBoundary::Lower => 0.0,
            LightnessBoundary::Upper => 1.0,
        };
        let chroma = if matches!(color.space, ParsedColorSpace::Oklch) {
            color.coordinates[1]
        } else {
            color.coordinates[1].hypot(color.coordinates[2])
        };
        return color.coordinates[0] == expected
            && chroma <= 0.000004
            && lightness_boundary_matches(space, boundary, converted_lightness);
    }
    if !matches!(
        color.space,
        ParsedColorSpace::Srgb | ParsedColorSpace::SrgbLinear
    ) {
        return false;
    }
    let expected = match boundary {
        LightnessBoundary::Lower => 0.0,
        LightnessBoundary::Upper => 1.0,
    };
    color
        .coordinates
        .iter()
        .all(|component| *component == expected)
        && lightness_boundary_matches(space, boundary, converted_lightness)
}

fn boundary_for_interpolation_space(
    space: MixColorSpace,
    source_space: ParsedColorSpace,
    boundary: Option<LightnessBoundary>,
    mixed_lightness: f32,
) -> Option<LightnessBoundary> {
    let source_is_lab = matches!(source_space, ParsedColorSpace::Lab | ParsedColorSpace::Lch);
    let source_is_oklab = matches!(
        source_space,
        ParsedColorSpace::Oklab | ParsedColorSpace::Oklch
    );
    let target_is_lab = matches!(space, MixColorSpace::Lab | MixColorSpace::Lch);
    let target_is_oklab = matches!(space, MixColorSpace::Oklab | MixColorSpace::Oklch);
    if !(source_is_lab && target_is_lab || source_is_oklab && target_is_oklab) {
        None
    } else {
        boundary.filter(|boundary| lightness_boundary_matches(space, *boundary, mixed_lightness))
    }
}

pub(crate) fn lightness_boundary_matches(
    space: MixColorSpace,
    boundary: LightnessBoundary,
    lightness: f32,
) -> bool {
    let expected = match (space, boundary) {
        (MixColorSpace::Lab | MixColorSpace::Lch, LightnessBoundary::Lower) => 0.0,
        (MixColorSpace::Lab | MixColorSpace::Lch, LightnessBoundary::Upper) => 100.0,
        (MixColorSpace::Oklab | MixColorSpace::Oklch, LightnessBoundary::Lower) => 0.0,
        (MixColorSpace::Oklab | MixColorSpace::Oklch, LightnessBoundary::Upper) => 1.0,
        (
            MixColorSpace::Srgb
            | MixColorSpace::SrgbLinear
            | MixColorSpace::Hsl
            | MixColorSpace::Hwb,
            _,
        ) => return false,
    };
    (lightness - expected).abs() <= 4.0 * f32::EPSILON * expected.abs().max(1.0)
}

fn parse_color_component<'i>(
    input: &mut Parser<'i, '_>,
    percentage_scale: f32,
) -> Result<f32, ParseError<'i, ()>> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Ok(0.0);
    }
    if let Ok((_, value)) =
        input.try_parse(|i| parse_color_math_value(i, ColorMathContext::NumberOrPercentage))
    {
        return Ok(value * percentage_scale);
    }
    if let Ok(percentage) = input.try_parse(|i| expect_percentage_stable(i)) {
        return Ok(percentage * percentage_scale);
    }
    Ok(expect_number_stable(input)?)
}

fn parse_lightness<'i>(
    input: &mut Parser<'i, '_>,
    is_oklab: bool,
) -> Result<f32, ParseError<'i, ()>> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Ok(0.0);
    }
    let value = if let Ok((kind, value)) =
        input.try_parse(|i| parse_color_math_value(i, ColorMathContext::NumberOrPercentage))
    {
        if kind == ColorMathType::Percentage && !is_oklab {
            value * 100.0
        } else {
            value
        }
    } else if let Ok(percentage) = input.try_parse(|i| expect_percentage_stable(i)) {
        if is_oklab {
            percentage
        } else {
            percentage * 100.0
        }
    } else {
        expect_number_stable(input)?
    };
    Ok(if is_oklab {
        value.clamp(0.0, 1.0)
    } else {
        value.clamp(0.0, 100.0)
    })
}

pub(crate) fn is_angle_unit(unit: &str) -> bool {
    matches!(
        unit.to_ascii_lowercase().as_str(),
        "deg" | "grad" | "rad" | "turn"
    )
}

fn parse_hue<'i>(input: &mut Parser<'i, '_>) -> Result<f32, ParseError<'i, ()>> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Ok(0.0);
    }
    if input
        .try_parse(|i| parse_color_math_value(i, ColorMathContext::NumberOrAngle))
        .is_ok()
    {
        return Ok(0.0);
    }
    match next_numeric_stable(input)? {
        Token::Number { value, .. } => {
            let hue = value.rem_euclid(360.0).to_radians();
            if hue.is_finite() {
                Ok(hue)
            } else {
                Err(input.new_custom_error(()))
            }
        }
        Token::Dimension {
            value, ref unit, ..
        } => {
            let hue = match unit.to_ascii_lowercase().as_str() {
                "deg" => value.rem_euclid(360.0).to_radians(),
                "grad" => (value.rem_euclid(400.0) * 0.9).to_radians(),
                "rad" => value.rem_euclid(std::f32::consts::TAU),
                "turn" => (value.rem_euclid(1.0) * 360.0).to_radians(),
                _ => return Err(input.new_custom_error(())),
            };
            if hue.is_finite() {
                Ok(hue)
            } else {
                Err(input.new_custom_error(()))
            }
        }
        token => Err(input.new_unexpected_token_error(token)),
    }
}

fn parse_optional_modern_alpha<'i>(input: &mut Parser<'i, '_>) -> Result<f32, ParseError<'i, ()>> {
    if input.try_parse(|i| i.expect_delim('/')).is_err() {
        return Ok(1.0);
    }
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Ok(0.0);
    }
    if let Ok((_, value)) =
        input.try_parse(|i| parse_color_math_value(i, ColorMathContext::NumberOrPercentage))
    {
        return Ok(value.clamp(0.0, 1.0));
    }
    if let Ok(percentage) = input.try_parse(|i| expect_percentage_stable(i)) {
        return Ok(percentage.clamp(0.0, 1.0));
    }
    Ok(expect_number_stable(input)?.clamp(0.0, 1.0))
}

fn rgb_f32_to_css_color(rgb: [f32; 3], alpha: f32) -> CssColor {
    CssColor {
        r: channel_to_u8(rgb[0]),
        g: channel_to_u8(rgb[1]),
        b: channel_to_u8(rgb[2]),
        a: channel_to_u8(alpha),
    }
}

pub(crate) fn channel_to_u8(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

// Keep transfer functions unclamped while colors are being converted for
// interpolation. The final `rgb_f32_to_css_color` conversion performs the
// destination sRGB gamut bound and 8-bit serialization.
pub(crate) fn srgb_encode(value: f32) -> f32 {
    let sign = if value.is_sign_negative() { -1.0 } else { 1.0 };
    let magnitude = value.abs();
    let encoded = if magnitude <= 0.0031308 {
        magnitude * 12.92
    } else {
        1.055 * magnitude.powf(1.0 / 2.4) - 0.055
    };
    sign * encoded
}

pub(crate) fn srgb_decode(value: f32) -> f32 {
    let sign = if value.is_sign_negative() { -1.0 } else { 1.0 };
    let magnitude = value.abs();
    let decoded = if magnitude <= 0.04045 {
        magnitude / 12.92
    } else {
        ((magnitude + 0.055) / 1.055).powf(2.4)
    };
    sign * decoded
}

// CSS Color conversion matrices are kept at their published precision; the
// runtime representation remains f32 until the final 8-bit property value.
fn lab_to_srgb_unbounded(coordinates: [f32; 3]) -> [f32; 3] {
    lab_to_srgb_linear(coordinates).map(srgb_encode)
}

#[allow(clippy::excessive_precision)]
fn lab_to_srgb_linear([lightness, a, b]: [f32; 3]) -> [f32; 3] {
    let fy = (lightness + 16.0) / 116.0;
    let fx = fy + a / 500.0;
    let fz = fy - b / 200.0;
    let epsilon = 216.0 / 24389.0;
    let kappa = 24389.0 / 27.0;
    let f_inv = |value: f32| {
        let cube = value * value * value;
        if cube > epsilon {
            cube
        } else {
            (116.0 * value - 16.0) / kappa
        }
    };
    let xyz_d50 = [
        0.9642956764 * f_inv(fx),
        f_inv(fy),
        0.8251046025 * f_inv(fz),
    ];
    let xyz_d65 = [
        0.9554734527 * xyz_d50[0] - 0.0230985369 * xyz_d50[1] + 0.0632593087 * xyz_d50[2],
        -0.0283697070 * xyz_d50[0] + 1.0099954580 * xyz_d50[1] + 0.0210413990 * xyz_d50[2],
        0.0123140017 * xyz_d50[0] - 0.0205076964 * xyz_d50[1] + 1.3303659366 * xyz_d50[2],
    ];
    [
        3.2409699 * xyz_d65[0] - 1.5373832 * xyz_d65[1] - 0.4986108 * xyz_d65[2],
        -0.9692436 * xyz_d65[0] + 1.8759675 * xyz_d65[1] + 0.0415551 * xyz_d65[2],
        0.0556301 * xyz_d65[0] - 0.2039769 * xyz_d65[1] + 1.0569715 * xyz_d65[2],
    ]
}

fn oklab_to_srgb_unbounded(coordinates: [f32; 3]) -> [f32; 3] {
    oklab_to_srgb_linear(coordinates).map(srgb_encode)
}

#[allow(clippy::excessive_precision)]
fn oklab_to_srgb_linear([lightness, a, b]: [f32; 3]) -> [f32; 3] {
    let l = lightness + 0.3963377774 * a + 0.2158037573 * b;
    let m = lightness - 0.1055613458 * a - 0.0638541728 * b;
    let s = lightness - 0.0894841775 * a - 1.2914855480 * b;
    let l = l * l * l;
    let m = m * m * m;
    let s = s * s * s;
    [
        4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
        -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
        -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s,
    ]
}

pub(crate) fn css_color_to_coordinates(
    color: ParsedColor,
    space: MixColorSpace,
) -> ColorCoordinates {
    color.to_coordinates(space)
}

pub(crate) fn mix_coordinates(
    first: ColorCoordinates,
    second: ColorCoordinates,
    weight_one: f32,
    weight_two: f32,
    space: MixColorSpace,
) -> ColorCoordinates {
    mix_coordinates_with_hue(
        first,
        second,
        weight_one,
        weight_two,
        space,
        HueInterpolationMethod::Shorter,
    )
}

fn mix_coordinates_with_hue(
    first: ColorCoordinates,
    second: ColorCoordinates,
    weight_one: f32,
    weight_two: f32,
    space: MixColorSpace,
    hue_interpolation_method: HueInterpolationMethod,
) -> ColorCoordinates {
    let alpha_one = first.alpha * weight_one;
    let alpha_two = second.alpha * weight_two;
    let alpha = alpha_one + alpha_two;
    let component = |one: f32, two: f32| {
        if alpha == 0.0 {
            0.0
        } else {
            let weighted_one = if alpha_one == 0.0 {
                0.0
            } else {
                one * alpha_one
            };
            let weighted_two = if alpha_two == 0.0 {
                0.0
            } else {
                two * alpha_two
            };
            (weighted_one + weighted_two) / alpha
        }
    };
    let (second_component, third_component) = if matches!(
        space,
        MixColorSpace::Hsl | MixColorSpace::Hwb | MixColorSpace::Lch | MixColorSpace::Oklch
    ) {
        let hue = if first.polar_hue_missing != second.polar_hue_missing {
            if first.polar_hue_missing {
                second.third
            } else {
                first.third
            }
        } else if weight_one == 0.0 {
            second.third
        } else if weight_two == 0.0 {
            first.third
        } else {
            interpolate_hue(
                first.third,
                second.third,
                weight_two,
                hue_interpolation_method,
            )
        };
        (component(first.second, second.second), hue)
    } else {
        (
            component(first.second, second.second),
            component(first.third, second.third),
        )
    };
    let first_component = component(first.first, second.first);
    let polar_hue_missing = matches!(
        space,
        MixColorSpace::Hsl | MixColorSpace::Hwb | MixColorSpace::Lch | MixColorSpace::Oklch
    ) && first.polar_hue_missing
        && second.polar_hue_missing;
    ColorCoordinates {
        first: first_component,
        second: second_component,
        third: third_component,
        alpha,
        polar_hue_missing,
    }
}

fn interpolate_hue(first: f32, second: f32, progress: f32, method: HueInterpolationMethod) -> f32 {
    match method {
        // CSS Color 4 §13.5: keep theta2 - theta1 in `[-180, 180]`, retaining
        // the authored direction when the difference is exactly a half-turn.
        // Adjust the delta, as the legacy shorter path did, so wrapped results
        // keep their existing internal representative.
        HueInterpolationMethod::Shorter => {
            let mut delta = second - first;
            if delta > std::f32::consts::PI {
                delta -= std::f32::consts::TAU;
            } else if delta < -std::f32::consts::PI {
                delta += std::f32::consts::TAU;
            }
            first + delta * progress
        }
        // CSS Color 4 §13.5: keep theta2 - theta1 in (-360, -180] or
        // [180, 360), preferring a positive full turn when the angles match.
        HueInterpolationMethod::Longer => {
            let mut first = first;
            let mut second = second;
            let delta = second - first;
            if 0.0 < delta && delta < std::f32::consts::PI {
                first += std::f32::consts::TAU;
            } else if -std::f32::consts::PI < delta && delta <= 0.0 {
                second += std::f32::consts::TAU;
            }
            first + (second - first) * progress
        }
        // CSS Color 4 §13.5: theta2 - theta1 in [0, 360).
        HueInterpolationMethod::Increasing => {
            let mut second = second;
            if second < first {
                second += std::f32::consts::TAU;
            }
            first + (second - first) * progress
        }
        // CSS Color 4 §13.5: theta2 - theta1 in (-360, 0].
        HueInterpolationMethod::Decreasing => {
            let mut first = first;
            if first < second {
                first += std::f32::consts::TAU;
            }
            first + (second - first) * progress
        }
    }
}

fn srgb_to_lab(rgb: [f32; 3]) -> [f32; 3] {
    linear_srgb_to_lab(rgb.map(srgb_decode))
}

#[allow(clippy::excessive_precision)]
fn linear_srgb_to_lab(rgb: [f32; 3]) -> [f32; 3] {
    let xyz_d65 = [
        0.4123908 * rgb[0] + 0.3575843 * rgb[1] + 0.1804808 * rgb[2],
        0.2126390 * rgb[0] + 0.7151687 * rgb[1] + 0.0721923 * rgb[2],
        0.0193308 * rgb[0] + 0.1191948 * rgb[1] + 0.9505322 * rgb[2],
    ];
    let xyz_d50 = [
        1.0479298208 * xyz_d65[0] + 0.0229467933 * xyz_d65[1] - 0.0501922295 * xyz_d65[2],
        0.0296278157 * xyz_d65[0] + 0.9904344846 * xyz_d65[1] - 0.0170738250 * xyz_d65[2],
        -0.0092430582 * xyz_d65[0] + 0.0150551449 * xyz_d65[1] + 0.7518742814 * xyz_d65[2],
    ];
    let epsilon = 216.0 / 24389.0;
    let kappa = 24389.0 / 27.0;
    let f = |value: f32| {
        if value > epsilon {
            value.cbrt()
        } else {
            (kappa * value + 16.0) / 116.0
        }
    };
    let fx = f(xyz_d50[0] / 0.9642956764);
    let fy = f(xyz_d50[1]);
    let fz = f(xyz_d50[2] / 0.8251046025);
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

fn srgb_to_oklab(rgb: [f32; 3]) -> [f32; 3] {
    linear_srgb_to_oklab(rgb.map(srgb_decode))
}

#[allow(clippy::excessive_precision)]
fn linear_srgb_to_oklab(rgb: [f32; 3]) -> [f32; 3] {
    let l = 0.41222147 * rgb[0] + 0.53633254 * rgb[1] + 0.05144599 * rgb[2];
    let m = 0.21190350 * rgb[0] + 0.68069955 * rgb[1] + 0.10739696 * rgb[2];
    let s = 0.08830246 * rgb[0] + 0.28171884 * rgb[1] + 0.62997870 * rgb[2];
    let l = l.cbrt();
    let m = m.cbrt();
    let s = s.cbrt();
    [
        0.21045426 * l + 0.79361779 * m - 0.00407205 * s,
        1.97799850 * l - 2.42859221 * m + 0.45059371 * s,
        0.02590404 * l + 0.78277177 * m - 0.80867577 * s,
    ]
}

/// `rgb()` / `rgba()` legacy comma syntax の中身 (関数呼び出しの括弧内) を
/// parse する。`parse_nested_block` の caller 側で `rgb(` / `rgba(` の function
/// token は既に consume 済み。`rgb` / `rgba` の function name は spec 上 alias
/// (CSS Color 4 §5.1: "rgb() and rgba() are now aliases for each other")
/// — alpha 省略は両者で許容し、name-based branching は行わない。
///
/// # Grammar (CSS Color 4 §5.1)
///
/// <https://www.w3.org/TR/css-color-4/#rgb-functions>
///
/// ```text
/// legacy-rgb-syntax  = rgb(  <legacy-rgb-channel>#{3} , <alpha-value>? )
/// legacy-rgba-syntax = rgba( <legacy-rgb-channel>#{3} , <alpha-value>? )
/// legacy-rgb-channel = <number> | <percentage>
/// alpha-value        = <number> | <percentage>
/// ```
///
/// legacy form の 3 channel は **all-number** or **all-percentage** の同一種で
/// なければならず、mix (`rgb(255, 50%, 0)`) は spec-invalid (§5.1:
/// "In the legacy form, the color channels can only be either all `<number>`s
/// or all `<percentage>`s — mixing types isn't allowed.")。
///
/// # Clamping
///
/// §5.1: "Values outside these ranges are not invalid, but are clamped to the
/// ranges defined here at parsed-value time" — 負値 / >255 (number) や
/// 100% 超も spec-valid、clamp only。
///
/// - `<number>` 0..=255 → `clamp_channel` で `i32.clamp(0, 255)` を 0..=1 に
///   正規化
/// - `<percentage>` 0%..=100% → `expect_percentage` は `0%`→0.0 / `100%`→1.0
///   の unit_value を返すため、そのまま normalized f32 として保持
/// - `<alpha-value>` は `<number>` 0..=1 または `<percentage>` 0%..=100% —
///   どちらも clamp 後に normalized f32 として保持
///
/// Legacy parser は color-mix() の endpoint を保持できるよう normalized f32 を返し、
/// property value へ落とす時だけ [`rgb_f32_to_css_color`] で u8 化する。
///
/// # Notes
///
/// - Modern (space + slash) syntax `rgb(R G B / A)` is parsed before the
///   legacy comma form. The two forms are not mixed.
/// - Fractional channels and position-aware `calc()` values are retained as
///   deferred syntax; the bounded parser uses zero placeholders until computed
///   value resolution is available.
/// - Alpha and channel `none` values are accepted syntactically and represented
///   as zero in the bounded model; missing-value carry-forward remains a later
///   computed-value concern.
pub(super) fn parse_rgb_function<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<ParsedColor, ParseError<'i, ()>> {
    // Try modern space-separated syntax first (CSS Color 4): `rgb(R G B [/ A])`
    // where each channel may be `none`, `<number>`, or `<percentage>`.
    // Modern syntax is space-separated, legacy is comma-separated.
    // We attempt modern via try_parse so legacy remains intact on failure.
    if let Ok(color) = input.try_parse(parse_modern_rgb_function) {
        return Ok(color);
    }
    // Legacy comma-separated path: `rgb(R, G, B [, A])` where all channels
    // share the same type (all numbers or all percentages).
    let (r, is_pct) = if let Ok(pct) = input.try_parse(|i| expect_percentage_stable(i)) {
        (pct.clamp(0.0, 1.0), true)
    } else if let Ok((kind, value)) =
        input.try_parse(|i| parse_color_math_value(i, ColorMathContext::NumberOrPercentage))
    {
        (
            if kind == ColorMathType::Percentage {
                value.clamp(0.0, 1.0)
            } else {
                clamp_rgb_number(value)
            },
            kind == ColorMathType::Percentage,
        )
    } else {
        (clamp_rgb_number(expect_number_stable(input)?), false)
    };
    input.expect_comma()?;
    let g = parse_rgb_channel(input, is_pct)?;
    input.expect_comma()?;
    let b = parse_rgb_channel(input, is_pct)?;
    // 4 番目 comma がある場合のみ alpha を parse。無ければ opaque (a=255)。
    // `rgba(...)` name 側で alpha 必須にしない (spec §5.1 alias 規定)。
    let a = if input.try_parse(|i| i.expect_comma()).is_ok() {
        parse_alpha_value(input)?
    } else {
        1.0
    };
    Ok(ParsedColor::from_coordinates(
        ParsedColorSpace::Srgb,
        [r, g, b],
        a,
    ))
}

fn parse_modern_channel<'i>(input: &mut Parser<'i, '_>) -> Result<f32, ParseError<'i, ()>> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Ok(0.0);
    }
    if let Ok((_, value)) =
        input.try_parse(|i| parse_color_math_value(i, ColorMathContext::NumberOrPercentage))
    {
        return Ok(value.clamp(0.0, 1.0));
    }
    if let Ok(pct) = input.try_parse(|i| expect_percentage_stable(i)) {
        return Ok(pct.clamp(0.0, 1.0));
    }
    // Modern number can be integer or float; use clamp_channel for ints and
    // float clamp for numbers.
    let n = expect_number_stable(input)?;
    Ok((n / 255.0).clamp(0.0, 1.0))
}

fn parse_modern_alpha<'i>(input: &mut Parser<'i, '_>) -> Result<f32, ParseError<'i, ()>> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Ok(0.0);
    }
    if let Ok((_, value)) =
        input.try_parse(|i| parse_color_math_value(i, ColorMathContext::NumberOrPercentage))
    {
        return Ok(value.clamp(0.0, 1.0));
    }
    if let Ok(pct) = input.try_parse(|i| expect_percentage_stable(i)) {
        return Ok(pct.clamp(0.0, 1.0));
    }
    Ok(expect_number_stable(input)?.clamp(0.0, 1.0))
}

fn parse_modern_rgb_function<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<ParsedColor, ParseError<'i, ()>> {
    let r = parse_modern_channel(input)?;
    let g = parse_modern_channel(input)?;
    let b = parse_modern_channel(input)?;
    let a = if input.try_parse(|i| i.expect_delim('/')).is_ok() {
        parse_modern_alpha(input)?
    } else {
        1.0
    };
    Ok(ParsedColor::from_coordinates(
        ParsedColorSpace::Srgb,
        [r, g, b],
        a,
    ))
}

/// legacy rgb() の 2 番目 / 3 番目 channel を parse する。1 番目 channel で
/// 決定した `is_pct` kind に沿って `<number>` / `<percentage>` のどちらかを
/// hard-expect し、mix (`rgb(255, 50%, 0)` / `rgb(50%, 255, 0)`) は Err で
/// 弾く (spec §5.1: "mixing types isn't allowed")。
fn parse_rgb_channel<'i>(
    input: &mut Parser<'i, '_>,
    is_pct: bool,
) -> Result<f32, ParseError<'i, ()>> {
    if is_pct {
        if let Ok((_, value)) =
            input.try_parse(|i| parse_color_math_value(i, ColorMathContext::Percentage))
        {
            return Ok(value.clamp(0.0, 1.0));
        }
        Ok(expect_percentage_stable(input)?.clamp(0.0, 1.0))
    } else {
        if let Ok((_, value)) =
            input.try_parse(|i| parse_color_math_value(i, ColorMathContext::Number))
        {
            return Ok(clamp_rgb_number(value));
        }
        Ok(clamp_rgb_number(expect_number_stable(input)?))
    }
}

/// `<alpha-value>` (CSS Color 4 §5.1 grammar: `<number> | <percentage>`)。
/// `<number>` は 0..=1、`<percentage>` は 0%..=100% で、どちらも clamp 後
/// normalized f32 に mapping する (`expect_percentage` の unit_value は既に
/// 0..=1 化されているため同一 formula)。
///
/// try_parse で percentage を先行させる — `<percentage>` は Token::Percentage、
/// `<number>` は Token::Number で orthogonal だが、percentage-first は
/// [`parse_rgb_function`] の 1 番目 channel と対称の順序 (mix reject と同じ
/// pattern で読める)。
pub(super) fn parse_alpha_value<'i>(input: &mut Parser<'i, '_>) -> Result<f32, ParseError<'i, ()>> {
    if let Ok((_, value)) =
        input.try_parse(|i| parse_color_math_value(i, ColorMathContext::NumberOrPercentage))
    {
        Ok(value.clamp(0.0, 1.0))
    } else if let Ok(pct) = input.try_parse(|i| expect_percentage_stable(i)) {
        Ok(pct.clamp(0.0, 1.0))
    } else {
        Ok(expect_number_stable(input)?.clamp(0.0, 1.0))
    }
}

fn clamp_rgb_number(value: f32) -> f32 {
    (value / 255.0).clamp(0.0, 1.0)
}
