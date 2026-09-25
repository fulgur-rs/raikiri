use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use cssparser::{Parser, ParserInput, Token};
use smol_str::SmolStr;

use crate::computed::CustomPropertyEnvironment;
use crate::property::{
    CalcLengthPercentage, CustomProperty, DeferredValue, FontVariationSettings, LengthOrAuto,
    MAX_DEFERRED_VALUE_NESTING_DEPTH, MAX_SUBSTITUTED_VALUE_BYTES, PropertyKey, PropertyValue,
    VerticalAlign, is_custom_property_name, parse_value,
};

use super::collect::{CustomCascadedDecl, RankedDecl, beats, cascade_rank};

/// Resolve a deferred declaration for either the element or page cascade.
///
/// `pub(crate)` is limited to the sibling page pipeline; no external API is
/// added. Both callers therefore share the same invalid-at-computed-value-time
/// and shorthand projection behavior.
pub(crate) fn resolve_deferred_value(
    deferred: &DeferredValue,
    custom_properties: &CustomPropertyEnvironment,
) -> Option<PropertyValue> {
    let substituted = substitute_vars(&deferred.value, &mut |name| custom_properties.get(name), 0)?;
    if deferred.key == PropertyKey::HyphenateLimitChars {
        // The integer components need to know whether a fractional number came
        // from a math function: calc results are rounded, direct fractional
        // tokens are invalid. Parse the substituted token stream before the
        // generic math simplifier erases that distinction.
        let mut input = ParserInput::new(substituted.as_ref());
        let mut parser = Parser::new(&mut input);
        let value = parse_value(&deferred.property, &mut parser)?;
        parser.expect_exhausted().ok()?;
        return project_deferred_value(value, deferred.key);
    }
    if deferred.key == PropertyKey::TextUnderlineOffset {
        // Preserve the substituted calc's em and percentage terms for the
        // property-specific computed-value resolver.
        let mut input = ParserInput::new(substituted.as_ref());
        let mut parser = Parser::new(&mut input);
        let value = parse_value(&deferred.property, &mut parser)?;
        parser.expect_exhausted().ok()?;
        return project_deferred_value(value, deferred.key);
    }
    if deferred.key == PropertyKey::TextShadow {
        // The text-shadow parser must see the substituted calc source: generic
        // simplification would erase mixed em/px terms, calculated-blur
        // provenance, or percentages that cancel to zero.
        let mut input = ParserInput::new(substituted.as_ref());
        let mut parser = Parser::new(&mut input);
        let value = parse_value(&deferred.property, &mut parser)?;
        parser.expect_exhausted().ok()?;
        return project_deferred_value(value, deferred.key);
    }
    let simplified = match simplify_math_functions(&substituted) {
        Some(value) => value,
        None => {
            let value = parse_simple_calc_length_percentage(&substituted)?;
            return calc_length_percentage_value(deferred.key, value);
        }
    };
    let mut input = ParserInput::new(simplified.as_ref());
    let mut parser = Parser::new(&mut input);
    let value = parse_value(&deferred.property, &mut parser)?;
    parser.expect_exhausted().ok()?;
    project_deferred_value(value, deferred.key)
}

/// Wrap a mixed-unit `calc()` in the value of the property it was declared
/// for. Sizing, inset, and `vertical-align` values can carry one; for any
/// other property the declaration is invalid at computed-value time.
fn calc_length_percentage_value(
    key: PropertyKey,
    value: CalcLengthPercentage,
) -> Option<PropertyValue> {
    let calc = LengthOrAuto::Calc(value);
    Some(match key {
        PropertyKey::Width => PropertyValue::Width(calc),
        PropertyKey::Height => PropertyValue::Height(calc),
        PropertyKey::MaxWidth => PropertyValue::MaxWidth(calc),
        PropertyKey::MaxHeight => PropertyValue::MaxHeight(calc),
        PropertyKey::MinWidth => PropertyValue::MinWidth(calc),
        PropertyKey::MinHeight => PropertyValue::MinHeight(calc),
        PropertyKey::MinBlockSize => PropertyValue::MinBlockSize(calc),
        PropertyKey::Top => PropertyValue::Top(calc),
        PropertyKey::Right => PropertyValue::Right(calc),
        PropertyKey::Bottom => PropertyValue::Bottom(calc),
        PropertyKey::Left => PropertyValue::Left(calc),
        PropertyKey::VerticalAlign => PropertyValue::VerticalAlign(VerticalAlign::Calc(value)),
        _ => return None,
    })
}

/// Parse mixed-unit calc forms that need a used containing-block basis.
pub(crate) fn parse_simple_calc_length_percentage(input: &str) -> Option<CalcLengthPercentage> {
    let trimmed = input.trim();
    let open = trimmed.find('(')?;
    if !trimmed[..open].eq_ignore_ascii_case("calc") || !trimmed.ends_with(')') {
        return None;
    }
    let inner = &trimmed[open + 1..trimmed.len() - 1];
    let mut depth = 0usize;
    let mut operator = None;
    for (index, byte) in inner.as_bytes().iter().copied().enumerate() {
        match byte {
            b'(' => depth = depth.checked_add(1)?,
            b')' => depth = depth.checked_sub(1)?,
            b'+' | b'-' if depth == 0 && index > 0 => {
                operator = Some((index, byte));
                break;
            }
            _ => {}
        }
    }
    let (left, right, sign) = match operator {
        Some((index, op)) => (&inner[..index], &inner[index + 1..], op),
        None => (inner, "", b'+'),
    };
    let parse_term = |term: &str| -> Option<(f32, f32)> {
        let value = MathParser::new(term.trim()).parse()?;
        if !value.number.is_finite() {
            return None;
        }
        match value.unit.as_deref()? {
            "%" => Some((value.number, 0.0)),
            "px" => Some((0.0, value.number)),
            _ => None,
        }
    };
    let (left_percent, left_px) = parse_term(left)?;
    let (right_percent, right_px) = if right.trim().is_empty() {
        (0.0, 0.0)
    } else {
        parse_term(right)?
    };
    let sign = if sign == b'-' { -1.0 } else { 1.0 };
    let result = CalcLengthPercentage {
        percent: left_percent + sign * right_percent,
        px: left_px + sign * right_px,
    };
    (result.percent.is_finite() && result.px.is_finite()).then_some(result)
}

pub(crate) fn project_deferred_value(
    value: PropertyValue,
    key: crate::property::PropertyKey,
) -> Option<PropertyValue> {
    if value.key() == key {
        return Some(value);
    }
    Some(match value {
        PropertyValue::Padding(sides) => match key {
            crate::property::PropertyKey::PaddingTop => PropertyValue::PaddingTop(sides.top),
            crate::property::PropertyKey::PaddingRight => PropertyValue::PaddingRight(sides.right),
            crate::property::PropertyKey::PaddingBottom => {
                PropertyValue::PaddingBottom(sides.bottom)
            }
            crate::property::PropertyKey::PaddingLeft => PropertyValue::PaddingLeft(sides.left),
            _ => return None,
        },
        PropertyValue::Margin(sides) => match key {
            crate::property::PropertyKey::MarginTop => PropertyValue::MarginTop(sides.top),
            crate::property::PropertyKey::MarginRight => PropertyValue::MarginRight(sides.right),
            crate::property::PropertyKey::MarginBottom => PropertyValue::MarginBottom(sides.bottom),
            crate::property::PropertyKey::MarginLeft => PropertyValue::MarginLeft(sides.left),
            _ => return None,
        },
        // `margin-inline`/`padding-inline`/`margin-block`/`padding-block`
        // logical 2-value shorthand — same projection shape as
        // `Margin`/`Padding` above, fanning out to the 2 physical longhands
        // `crate::rule::expand_deferred`'s key table pairs each shorthand
        // key with (`crate::property::PropertyValue::MarginInline` doc's
        // physical-mapping rationale covers *why* these specific longhands).
        PropertyValue::MarginInline(pair) => match key {
            crate::property::PropertyKey::MarginLeft => PropertyValue::MarginLeft(pair.start),
            crate::property::PropertyKey::MarginRight => PropertyValue::MarginRight(pair.end),
            _ => return None,
        },
        PropertyValue::MarginBlock(pair) => match key {
            crate::property::PropertyKey::MarginTop => PropertyValue::MarginTop(pair.start),
            crate::property::PropertyKey::MarginBottom => PropertyValue::MarginBottom(pair.end),
            _ => return None,
        },
        PropertyValue::PaddingInline(pair) => match key {
            crate::property::PropertyKey::PaddingLeft => PropertyValue::PaddingLeft(pair.start),
            crate::property::PropertyKey::PaddingRight => PropertyValue::PaddingRight(pair.end),
            _ => return None,
        },
        PropertyValue::PaddingBlock(pair) => match key {
            crate::property::PropertyKey::PaddingTop => PropertyValue::PaddingTop(pair.start),
            crate::property::PropertyKey::PaddingBottom => PropertyValue::PaddingBottom(pair.end),
            _ => return None,
        },
        PropertyValue::Outline(outline) => match key {
            crate::property::PropertyKey::OutlineWidth => {
                PropertyValue::OutlineWidth(outline.width)
            }
            crate::property::PropertyKey::OutlineStyle => {
                PropertyValue::OutlineStyle(outline.style)
            }
            crate::property::PropertyKey::OutlineColor => {
                PropertyValue::OutlineColor(outline.color)
            }
            _ => return None,
        },
        PropertyValue::Border(sides) => match key {
            crate::property::PropertyKey::BorderTopWidth => {
                PropertyValue::BorderTopWidth(sides.top.width)
            }
            crate::property::PropertyKey::BorderRightWidth => {
                PropertyValue::BorderRightWidth(sides.right.width)
            }
            crate::property::PropertyKey::BorderBottomWidth => {
                PropertyValue::BorderBottomWidth(sides.bottom.width)
            }
            crate::property::PropertyKey::BorderLeftWidth => {
                PropertyValue::BorderLeftWidth(sides.left.width)
            }
            crate::property::PropertyKey::BorderTopStyle => {
                PropertyValue::BorderTopStyle(sides.top.style)
            }
            crate::property::PropertyKey::BorderRightStyle => {
                PropertyValue::BorderRightStyle(sides.right.style)
            }
            crate::property::PropertyKey::BorderBottomStyle => {
                PropertyValue::BorderBottomStyle(sides.bottom.style)
            }
            crate::property::PropertyKey::BorderLeftStyle => {
                PropertyValue::BorderLeftStyle(sides.left.style)
            }
            crate::property::PropertyKey::BorderTopColor => {
                PropertyValue::BorderTopColor(sides.top.color)
            }
            crate::property::PropertyKey::BorderRightColor => {
                PropertyValue::BorderRightColor(sides.right.color)
            }
            crate::property::PropertyKey::BorderBottomColor => {
                PropertyValue::BorderBottomColor(sides.bottom.color)
            }
            crate::property::PropertyKey::BorderLeftColor => {
                PropertyValue::BorderLeftColor(sides.left.color)
            }
            _ => return None,
        },
        // `border-style` / `border-width` / `border-color` shorthands fan
        // out to their 4 side longhands by key (same shape as `Border`
        // above; reached via `var()`/re-cascade paths that bypass rule.rs
        // expansion).
        // cov:ignore: shorthand projection is defensive; normal rule expansion covers this path.
        PropertyValue::BorderStyle(sides) => match key {
            crate::property::PropertyKey::BorderTopStyle => {
                PropertyValue::BorderTopStyle(sides.top)
            }
            crate::property::PropertyKey::BorderRightStyle => {
                PropertyValue::BorderRightStyle(sides.right)
            }
            crate::property::PropertyKey::BorderBottomStyle => {
                PropertyValue::BorderBottomStyle(sides.bottom)
            }
            crate::property::PropertyKey::BorderLeftStyle => {
                PropertyValue::BorderLeftStyle(sides.left)
            }
            _ => return None,
        },
        // cov:ignore: shorthand projection is defensive; normal rule expansion covers this path.
        PropertyValue::BorderWidth(sides) => match key {
            crate::property::PropertyKey::BorderTopWidth => {
                PropertyValue::BorderTopWidth(sides.top)
            }
            crate::property::PropertyKey::BorderRightWidth => {
                PropertyValue::BorderRightWidth(sides.right)
            }
            crate::property::PropertyKey::BorderBottomWidth => {
                PropertyValue::BorderBottomWidth(sides.bottom)
            }
            crate::property::PropertyKey::BorderLeftWidth => {
                PropertyValue::BorderLeftWidth(sides.left)
            }
            _ => return None,
        },
        // cov:ignore: shorthand projection is defensive; normal rule expansion covers this path.
        PropertyValue::BorderColor(sides) => match key {
            crate::property::PropertyKey::BorderTopColor => {
                PropertyValue::BorderTopColor(sides.top)
            }
            crate::property::PropertyKey::BorderRightColor => {
                PropertyValue::BorderRightColor(sides.right)
            }
            crate::property::PropertyKey::BorderBottomColor => {
                PropertyValue::BorderBottomColor(sides.bottom)
            }
            crate::property::PropertyKey::BorderLeftColor => {
                PropertyValue::BorderLeftColor(sides.left)
            }
            _ => return None,
        },
        // cov:ignore: shorthand projection is defensive; normal rule expansion covers this path.
        PropertyValue::Overflow(pair) => match key {
            crate::property::PropertyKey::OverflowX => PropertyValue::OverflowX(pair.x),
            crate::property::PropertyKey::OverflowY => PropertyValue::OverflowY(pair.y),
            _ => return None,
        },
        // cov:ignore: shorthand projection is defensive; normal rule expansion covers this path.
        PropertyValue::TextDecoration(shorthand) => match key {
            crate::property::PropertyKey::TextDecorationLine => {
                PropertyValue::TextDecorationLine(shorthand.line)
            }
            crate::property::PropertyKey::TextDecorationThickness => {
                PropertyValue::TextDecorationThickness(shorthand.thickness)
            }
            crate::property::PropertyKey::TextDecorationStyle => {
                PropertyValue::TextDecorationStyle(shorthand.style)
            }
            crate::property::PropertyKey::TextDecorationColor => {
                PropertyValue::TextDecorationColor(shorthand.color)
            }
            _ => return None,
        },
        // cov:ignore: shorthand projection is defensive; normal rule expansion covers this path.
        PropertyValue::TextEmphasis(shorthand) => match key {
            crate::property::PropertyKey::TextEmphasisStyle => {
                PropertyValue::TextEmphasisStyle(shorthand.style)
            }
            crate::property::PropertyKey::TextEmphasisColor => {
                PropertyValue::TextEmphasisColor(shorthand.color)
            }
            _ => return None,
        },
        // cov:ignore: shorthand projection is defensive; normal rule expansion covers this path.
        PropertyValue::Flex(shorthand) => match key {
            crate::property::PropertyKey::FlexGrow => PropertyValue::FlexGrow(shorthand.grow),
            crate::property::PropertyKey::FlexShrink => PropertyValue::FlexShrink(shorthand.shrink),
            crate::property::PropertyKey::FlexBasis => PropertyValue::FlexBasis(shorthand.basis),
            _ => return None,
        },
        // cov:ignore: shorthand projection is defensive; normal rule expansion covers this path.
        PropertyValue::FlexFlow(shorthand) => match key {
            crate::property::PropertyKey::FlexDirection => {
                PropertyValue::FlexDirection(shorthand.direction)
            }
            crate::property::PropertyKey::FlexWrap => PropertyValue::FlexWrap(shorthand.wrap),
            _ => return None,
        },
        // cov:ignore: shorthand projection is defensive; normal rule expansion covers this path.
        PropertyValue::Columns(shorthand) => match key {
            crate::property::PropertyKey::ColumnWidth => {
                PropertyValue::ColumnWidth(shorthand.width)
            }
            crate::property::PropertyKey::ColumnCount => {
                PropertyValue::ColumnCount(shorthand.count)
            }
            _ => return None,
        },
        PropertyValue::Gap(shorthand) => match key {
            crate::property::PropertyKey::RowGap => PropertyValue::RowGap(shorthand.row),
            crate::property::PropertyKey::ColumnGap => PropertyValue::ColumnGap(shorthand.column),
            _ => return None,
        },
        PropertyValue::PlaceContent(shorthand) => match key {
            crate::property::PropertyKey::AlignContent => {
                PropertyValue::AlignContent(shorthand.align)
            }
            crate::property::PropertyKey::JustifyContent => {
                PropertyValue::JustifyContent(shorthand.justify)
            }
            _ => return None,
        },
        PropertyValue::GridRow(shorthand) => match key {
            crate::property::PropertyKey::GridRowStart => {
                PropertyValue::GridRowStart(shorthand.start)
            }
            crate::property::PropertyKey::GridRowEnd => PropertyValue::GridRowEnd(shorthand.end),
            _ => return None,
        },
        PropertyValue::GridColumn(shorthand) => match key {
            crate::property::PropertyKey::GridColumnStart => {
                PropertyValue::GridColumnStart(shorthand.start)
            }
            crate::property::PropertyKey::GridColumnEnd => {
                PropertyValue::GridColumnEnd(shorthand.end)
            }
            _ => return None,
        },
        PropertyValue::PlaceItems(shorthand) => match key {
            crate::property::PropertyKey::AlignItems => PropertyValue::AlignItems(shorthand.align),
            crate::property::PropertyKey::JustifyItems => {
                PropertyValue::JustifyItems(shorthand.justify)
            }
            _ => return None,
        },
        PropertyValue::PlaceSelf(shorthand) => match key {
            crate::property::PropertyKey::AlignSelf => PropertyValue::AlignSelf(shorthand.align),
            crate::property::PropertyKey::JustifySelf => {
                PropertyValue::JustifySelf(shorthand.justify)
            }
            _ => return None,
        },
        PropertyValue::Font(shorthand) => match key {
            crate::property::PropertyKey::FontStyle => PropertyValue::FontStyle(shorthand.style),
            crate::property::PropertyKey::FontVariantCaps => {
                PropertyValue::FontVariantCaps(shorthand.variant)
            }
            crate::property::PropertyKey::FontWeight => PropertyValue::FontWeight(shorthand.weight),
            crate::property::PropertyKey::FontSize => match shorthand.size {
                crate::property::FontShorthandSize::Absolute(length) => {
                    PropertyValue::FontSize(length)
                }
                crate::property::FontShorthandSize::Relative(relative) => {
                    PropertyValue::FontSizeRelative(relative)
                }
            },
            crate::property::PropertyKey::LineHeight => {
                PropertyValue::LineHeight(shorthand.line_height)
            }
            crate::property::PropertyKey::FontFamily => PropertyValue::FontFamily(shorthand.family),
            crate::property::PropertyKey::FontKerning => {
                PropertyValue::FontKerning(crate::property::FontKerning::Auto)
            }
            crate::property::PropertyKey::FontLanguageOverride => {
                PropertyValue::FontLanguageOverride(crate::property::FontLanguageOverride::Normal)
            }
            crate::property::PropertyKey::FontOpticalSizing => {
                PropertyValue::FontOpticalSizing(crate::property::FontOpticalSizing::Auto)
            }
            crate::property::PropertyKey::FontVariantEastAsian => {
                PropertyValue::FontVariantEastAsian(crate::property::FontVariantEastAsian::initial())
            }
            crate::property::PropertyKey::FontVariantEmoji => {
                PropertyValue::FontVariantEmoji(crate::property::FontVariantEmoji::Normal)
            }
            crate::property::PropertyKey::FontVariantLigatures => {
                PropertyValue::FontVariantLigatures(crate::property::FontVariantLigatures::Normal)
            }
            crate::property::PropertyKey::FontVariantNumeric => {
                PropertyValue::FontVariantNumeric(crate::property::FontVariantNumeric::initial())
            }
            crate::property::PropertyKey::FontVariantPosition => {
                PropertyValue::FontVariantPosition(crate::property::FontVariantPosition::Normal)
            }
            crate::property::PropertyKey::FontVariationSettings => {
                PropertyValue::FontVariationSettings(FontVariationSettings::Normal)
            }
            _ => return None,
        },
        PropertyValue::Background(shorthand) => match key {
            crate::property::PropertyKey::BackgroundColor => {
                PropertyValue::BackgroundColor(shorthand.color)
            }
            crate::property::PropertyKey::BackgroundImage => {
                PropertyValue::BackgroundImage(shorthand.image)
            }
            crate::property::PropertyKey::BackgroundRepeat => {
                PropertyValue::BackgroundRepeat(shorthand.repeat)
            }
            crate::property::PropertyKey::BackgroundAttachment => {
                PropertyValue::BackgroundAttachment(shorthand.attachment)
            }
            crate::property::PropertyKey::BackgroundPosition => {
                PropertyValue::BackgroundPosition(shorthand.position)
            }
            crate::property::PropertyKey::BackgroundSize => {
                PropertyValue::BackgroundSize(shorthand.size)
            }
            crate::property::PropertyKey::BackgroundClip => {
                PropertyValue::BackgroundClip(shorthand.clip)
            }
            crate::property::PropertyKey::BackgroundOrigin => {
                PropertyValue::BackgroundOrigin(shorthand.origin)
            }
            _ => return None,
        },
        _ => return None,
    })
}

pub(crate) fn simplify_math_functions(input: &str) -> Option<SmolStr> {
    simplify_math_functions_at_depth(input, 0)
}

fn push_bounded(output: &mut String, fragment: &str) -> Option<()> {
    let new_len = output.len().checked_add(fragment.len())?;
    if new_len > MAX_SUBSTITUTED_VALUE_BYTES {
        return None;
    }
    output.push_str(fragment);
    Some(())
}

const MATH_FUNCTIONS: [&str; 4] = ["calc", "min", "max", "clamp"];

/// Find named CSS functions by walking cssparser's component-value tokens.
/// `Parser::next` skips a function body, so recurse into every block to keep
/// nested `var()`/math functions visible while preserving their byte offsets.
pub(crate) fn find_function_tokens(
    input: &str,
    names: &[&str],
) -> Option<Vec<(SmolStr, usize, usize, usize)>> {
    let mut parser_input = ParserInput::new(input);
    let mut parser = Parser::new(&mut parser_input);
    find_function_tokens_in_parser(&mut parser, names, 0)
}

pub(crate) fn find_function_tokens_in_parser<'i, 't>(
    parser: &mut Parser<'i, 't>,
    names: &[&str],
    depth: usize,
) -> Option<Vec<(SmolStr, usize, usize, usize)>> {
    if depth > MAX_DEFERRED_VALUE_NESTING_DEPTH {
        return None;
    }
    let mut found = Vec::new();
    loop {
        let token_start = parser.position().byte_index();
        let token = match parser.next() {
            Ok(token) => token.clone(),
            Err(_) => break,
        };
        match token {
            Token::Function(name) => {
                let open = parser.position().byte_index().checked_sub(1)?;
                let is_target = names.iter().any(|wanted| name.eq_ignore_ascii_case(wanted));
                let decoded_name: SmolStr = name.as_ref().into();
                let mut nested_found = parser
                    .parse_nested_block(|nested| {
                        find_function_tokens_in_parser(nested, names, depth.saturating_add(1))
                            .ok_or_else(|| nested.new_custom_error::<(), ()>(()))
                    })
                    .ok()?;
                if is_target {
                    let close = parser.position().byte_index().checked_sub(1)?;
                    found.push((decoded_name, token_start, open, close));
                }
                found.append(&mut nested_found);
            }
            Token::ParenthesisBlock | Token::SquareBracketBlock | Token::CurlyBracketBlock => {
                let mut nested_found = parser
                    .parse_nested_block(|nested| {
                        find_function_tokens_in_parser(nested, names, depth.saturating_add(1))
                            .ok_or_else(|| nested.new_custom_error::<(), ()>(()))
                    })
                    .ok()?;
                found.append(&mut nested_found);
            }
            _ => {}
        }
    }
    Some(found)
}

pub(crate) fn needs_css_token_separator(left: &str, right: &str) -> bool {
    let left = left
        .as_bytes()
        .iter()
        .rev()
        .find(|byte| !byte.is_ascii_whitespace())
        .copied();
    let right = right
        .as_bytes()
        .iter()
        .find(|byte| !byte.is_ascii_whitespace())
        .copied();
    let (Some(left), Some(right)) = (left, right) else {
        return false;
    };
    if left.is_ascii_alphanumeric()
        && (right.is_ascii_alphanumeric() || matches!(right, b'.' | b'%'))
    {
        return true;
    }
    if (left.is_ascii_alphabetic() || matches!(left, b'-' | b'_' | b'\\') || left >= 0x80)
        && (right.is_ascii_alphanumeric() || matches!(right, b'-' | b'_' | b'\\') || right >= 0x80)
    {
        return true;
    }
    if right == b'('
        && (left.is_ascii_alphabetic() || matches!(left, b'-' | b'_' | b'\\') || left >= 0x80)
    {
        return true;
    }
    if left.is_ascii_digit() && (matches!(right, b'-' | b'_' | b'\\') || right >= 0x80) {
        return true;
    }
    if matches!(left, b'+' | b'-') && (right.is_ascii_digit() || right == b'.') {
        return true;
    }
    if left == b'.' && right.is_ascii_digit() {
        return true;
    }
    matches!(left, b'#' | b'@')
        && (right.is_ascii_alphanumeric() || matches!(right, b'-' | b'_' | b'\\') || right >= 0x80)
}

pub(crate) fn simplify_math_functions_at_depth(input: &str, depth: usize) -> Option<SmolStr> {
    if depth > MAX_VARIABLE_RESOLUTION_DEPTH || input.len() > MAX_SUBSTITUTED_VALUE_BYTES {
        return None;
    }
    let mut output = String::with_capacity(input.len());
    let mut position = 0;
    let functions = find_function_tokens(input, &MATH_FUNCTIONS)?;
    for (name, name_start, open, close) in functions {
        if name_start < position {
            continue;
        }
        push_bounded(&mut output, &input[position..name_start])?;
        let inner_source = &input[open + 1..close];
        let inner = simplify_math_functions_at_depth(inner_source, depth.saturating_add(1))?;
        let evaluated = evaluate_math_function(name.as_str(), &inner)?;
        push_bounded(&mut output, &evaluated)?;
        if needs_css_token_separator(&output, &input[close + 1..]) {
            push_bounded(&mut output, " ")?;
        }
        position = close.checked_add(1)?;
    }
    push_bounded(&mut output, &input[position..])?;
    Some(output.into())
}

#[derive(Clone, Debug)]
pub(crate) struct MathValue {
    pub(crate) number: f32,
    pub(crate) unit: Option<String>,
}

pub(crate) fn evaluate_math_function(name: &str, input: &str) -> Option<String> {
    if input.len() > MAX_SUBSTITUTED_VALUE_BYTES {
        return None;
    }
    match name.to_ascii_lowercase().as_str() {
        "calc" => {
            let value = MathParser::new(input).parse()?;
            serialize_math_value(value)
        }
        "min" | "max" => {
            let arguments = split_top_level_commas(input)?;
            if arguments.is_empty() {
                return None; // cov:ignore: split_top_level_commas always returns at least one part when it succeeds.
            }
            let mut values = arguments
                .into_iter()
                .map(|argument| MathParser::new(argument).parse())
                .collect::<Option<Vec<_>>>()?;
            normalize_math_values(&mut values)?;
            let is_min = name.eq_ignore_ascii_case("min");
            let mut selected = values.remove(0);
            for value in values {
                let choose = if is_min {
                    value.number < selected.number
                } else {
                    value.number > selected.number
                };
                if choose {
                    selected = value;
                }
            }
            serialize_math_value(selected)
        }
        "clamp" => {
            let arguments = split_top_level_commas(input)?;
            if arguments.len() != 3 {
                return None;
            }
            let values = arguments
                .into_iter()
                .map(|argument| MathParser::new(argument).parse())
                .collect::<Option<Vec<_>>>()?;
            let mut values = values;
            normalize_math_values(&mut values)?;
            let selected = if values[0].number > values[2].number {
                // CSS Values 4 §10.2: when the bounds are reversed, the
                // minimum wins over the maximum.
                values[0].clone()
            } else {
                MathValue {
                    number: values[1].number.max(values[0].number).min(values[2].number),
                    unit: values[0].unit.clone(),
                }
            };
            serialize_math_value(selected)
        }
        _ => None,
    }
}

pub(crate) fn absolute_length_factor(unit: &str) -> Option<f32> {
    match unit {
        "px" => Some(1.0),
        "in" => Some(96.0),
        "cm" => Some(96.0 / 2.54),
        "mm" => Some(96.0 / 25.4),
        "q" => Some(96.0 / 101.6),
        "pt" => Some(96.0 / 72.0),
        "pc" => Some(16.0),
        _ => None,
    }
}

pub(crate) fn normalize_math_pair(left: &mut MathValue, right: &mut MathValue) -> Option<()> {
    // Additive zero vanishes regardless of unit (CSS Values 4 §10.7
    // calculation simplification): `calc(50% + 0px)` is `50%`, `calc(0em
    // + 10px)` is `10px`. Zero is zero in every unit, so the surviving
    // side keeps its own unit for downstream (possibly percentage)
    // resolution. Both-zero keeps the left side as-is.
    if left.number == 0.0 {
        left.unit = right.unit.clone();
        return Some(());
    }
    if right.number == 0.0 {
        right.unit = left.unit.clone();
        return Some(());
    }
    match (&left.unit, &right.unit) {
        (None, None) => Some(()),
        (Some(left_unit), Some(right_unit)) if left_unit == right_unit => Some(()),
        (Some(left_unit), Some(right_unit)) => {
            let left_factor = absolute_length_factor(left_unit)?;
            let right_factor = absolute_length_factor(right_unit)?;
            left.number *= left_factor;
            right.number *= right_factor;
            if !left.number.is_finite() || !right.number.is_finite() {
                return None;
            }
            left.unit = Some("px".to_owned());
            right.unit = Some("px".to_owned());
            Some(())
        }
        _ => None,
    }
}

pub(crate) fn normalize_math_values(values: &mut [MathValue]) -> Option<()> {
    if values.is_empty() {
        return None;
    }
    let first_unit = values[0].unit.clone();
    if values.iter().all(|value| value.unit == first_unit) {
        return Some(());
    }
    if !values.iter().all(|value| {
        value
            .unit
            .as_deref()
            .is_some_and(|unit| absolute_length_factor(unit).is_some())
    }) {
        return None;
    }
    for value in values {
        let factor = absolute_length_factor(value.unit.as_deref()?)?;
        value.number *= factor;
        if !value.number.is_finite() {
            return None;
        }
        value.unit = Some("px".to_owned());
    }
    Some(())
}

pub(crate) fn serialize_math_value(value: MathValue) -> Option<String> {
    if !value.number.is_finite() {
        return None;
    }
    let number = if value.number == 0.0 {
        "0".to_owned()
    } else {
        value.number.to_string()
    };
    let unit = value.unit.as_deref().unwrap_or("");
    if number.len().checked_add(unit.len())? > MAX_SUBSTITUTED_VALUE_BYTES {
        return None;
    }
    Some(format!("{}{}", number, unit))
}

pub(crate) struct MathParser<'a> {
    input: &'a str,
    position: usize,
    nesting_depth: usize,
}

impl<'a> MathParser<'a> {
    pub(crate) fn new(input: &'a str) -> Self {
        Self {
            input,
            position: 0,
            nesting_depth: 0,
        }
    }

    pub(crate) fn parse(mut self) -> Option<MathValue> {
        let value = self.parse_sum()?;
        self.skip_whitespace()?;
        (self.position == self.input.len()).then_some(value)
    }

    fn parse_sum(&mut self) -> Option<MathValue> {
        let mut value = self.parse_product()?;
        loop {
            self.skip_whitespace()?;
            let Some(&operator) = self.input.as_bytes().get(self.position) else {
                return Some(value);
            };
            if operator != b'+' && operator != b'-' {
                return Some(value);
            }
            let operator_position = self.position;
            if !self.has_css_whitespace_before(operator_position)
                || !self.has_css_whitespace_after(operator_position + 1)
            {
                return None;
            }
            self.position += 1;
            let mut right = self.parse_product()?;
            normalize_math_pair(&mut value, &mut right)?;
            value.number = if operator == b'+' {
                value.number + right.number
            } else {
                value.number - right.number
            };
            if !value.number.is_finite() {
                return None;
            }
        }
    }

    fn parse_product(&mut self) -> Option<MathValue> {
        let mut value = self.parse_primary()?;
        loop {
            self.skip_whitespace()?;
            let operator = match self.input.as_bytes().get(self.position) {
                Some(b'*') | Some(b'/') => self.input.as_bytes()[self.position],
                _ => return Some(value),
            };
            self.position += 1;
            let right = self.parse_primary()?;
            match operator {
                b'*' if value.unit.is_none() => {
                    value = MathValue {
                        number: value.number * right.number,
                        unit: right.unit,
                    }
                }
                b'*' if right.unit.is_none() => value.number *= right.number,
                b'/' if right.unit.is_none() && right.number != 0.0 => value.number /= right.number,
                _ => return None,
            }
            if !value.number.is_finite() {
                return None;
            }
        }
    }

    fn parse_primary(&mut self) -> Option<MathValue> {
        self.skip_whitespace()?;
        if self.input.as_bytes().get(self.position) == Some(&b'(') {
            if self.nesting_depth >= MAX_VARIABLE_RESOLUTION_DEPTH {
                return None;
            }
            self.position += 1;
            self.nesting_depth += 1;
            let value = self.parse_sum();
            self.nesting_depth -= 1;
            let value = value?;
            self.skip_whitespace()?;
            if self.input.as_bytes().get(self.position) != Some(&b')') {
                return None;
            }
            self.position += 1;
            return Some(value);
        }
        self.parse_number()
    }

    fn parse_number(&mut self) -> Option<MathValue> {
        let start = self.position;
        if matches!(
            self.input.as_bytes().get(self.position),
            Some(b'+') | Some(b'-')
        ) {
            self.position += 1;
        }
        let digits_start = self.position;
        while self
            .input
            .as_bytes()
            .get(self.position)
            .is_some_and(u8::is_ascii_digit)
        {
            self.position += 1;
        }
        if self.input.as_bytes().get(self.position) == Some(&b'.') {
            self.position += 1;
            while self
                .input
                .as_bytes()
                .get(self.position)
                .is_some_and(u8::is_ascii_digit)
            {
                self.position += 1;
            }
        }
        if self.position == digits_start
            || (self.position == digits_start + 1
                && self.input.as_bytes().get(digits_start) == Some(&b'.'))
        {
            return None;
        }
        if matches!(
            self.input.as_bytes().get(self.position),
            Some(b'e') | Some(b'E')
        ) {
            let exponent_start = self.position;
            self.position += 1;
            if matches!(
                self.input.as_bytes().get(self.position),
                Some(b'+') | Some(b'-')
            ) {
                self.position += 1;
            }
            let exponent_digits = self.position;
            while self
                .input
                .as_bytes()
                .get(self.position)
                .is_some_and(u8::is_ascii_digit)
            {
                self.position += 1;
            }
            if self.position == exponent_digits {
                self.position = exponent_start;
            }
        }
        let number = self.input[start..self.position].parse::<f32>().ok()?;
        let unit_start = self.position;
        if self.input.as_bytes().get(self.position) == Some(&b'%') {
            self.position += 1;
        } else {
            while self
                .input
                .as_bytes()
                .get(self.position)
                .is_some_and(u8::is_ascii_alphabetic)
            {
                self.position += 1;
            }
        }
        let unit = (unit_start != self.position)
            .then(|| self.input[unit_start..self.position].to_ascii_lowercase());
        Some(MathValue { number, unit })
    }

    fn skip_whitespace(&mut self) -> Option<()> {
        loop {
            while self
                .input
                .as_bytes()
                .get(self.position)
                .is_some_and(u8::is_ascii_whitespace)
            {
                self.position += 1;
            }
            if self.input.as_bytes().get(self.position..self.position + 2) == Some(b"/*") {
                self.position = skip_css_comment(self.input, self.position)?;
                continue;
            }
            return Some(());
        }
    }

    pub(crate) fn has_css_whitespace_before(&self, position: usize) -> bool {
        if position == 0 {
            return false;
        }
        if self.input.as_bytes()[position - 1].is_ascii_whitespace() {
            return true;
        }
        self.input[..position].ends_with("*/")
    }

    fn has_css_whitespace_after(&self, position: usize) -> bool {
        self.input
            .as_bytes()
            .get(position)
            .is_some_and(u8::is_ascii_whitespace)
            || self.input.as_bytes().get(position..position + 2) == Some(b"/*")
    }
}

pub(crate) fn split_top_level_commas(input: &str) -> Option<Vec<&str>> {
    if input.len() > MAX_SUBSTITUTED_VALUE_BYTES {
        return None;
    }
    let bytes = input.as_bytes();
    let mut parts = Vec::new();
    let mut start = 0;
    let mut closers = Vec::new();
    let mut position = 0;
    while position < bytes.len() {
        match bytes[position] {
            b'\'' | b'"' => position = skip_css_string(input, position)?,
            b'/' if bytes.get(position + 1) == Some(&b'*') => {
                position = skip_css_comment(input, position)?
            }
            b'\\' => position = skip_css_escape(input, position)?,
            b'(' | b'[' | b'{' => {
                if closers.len() >= MAX_DEFERRED_VALUE_NESTING_DEPTH {
                    return None;
                }
                let closer = if bytes[position] == b'(' {
                    b')'
                } else if bytes[position] == b'[' {
                    b']'
                } else {
                    b'}'
                };
                closers.push(closer);
                position += 1;
            }
            b')' | b']' | b'}' if closers.last() == Some(&bytes[position]) => {
                closers.pop();
                position += 1;
            }
            b')' | b']' | b'}' if closers.is_empty() => return None,
            b',' if closers.is_empty() => {
                parts.push(input[start..position].trim());
                start = position + 1;
                position += 1;
            }
            _ => position += 1,
        }
    }
    if !closers.is_empty() {
        return None;
    }
    parts.push(input[start..].trim());
    parts.iter().all(|part| !part.is_empty()).then_some(parts)
}

// CSS Variables 1 §3.3 requires a UA-defined expansion limit to prevent
// exponential substitution blowups. The depth bound is an additional stack
// guard for the mutually recursive resolver/substituter paths.
pub(crate) const MAX_VARIABLE_RESOLUTION_DEPTH: usize = MAX_DEFERRED_VALUE_NESTING_DEPTH;

/// Maximum number of uncached custom-property expansions in one map resolve.
/// Memoization handles shared DAG branches; this second bound caps the work
/// spent on a large, mostly-unique dependency graph.
pub(crate) const MAX_VARIABLE_RESOLUTION_STEPS: usize = 16 * 1024;

pub(crate) struct VariableResolutionBudget {
    remaining: usize,
    pub(crate) exhausted: bool,
}

impl VariableResolutionBudget {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            remaining: limit,
            exhausted: false,
        }
    }

    pub(crate) fn consume(&mut self) -> bool {
        if self.remaining == 0 {
            self.exhausted = true;
            return false;
        }
        self.remaining -= 1;
        true
    }
}

pub(crate) fn resolve_custom_properties(
    inherited: &Arc<CustomPropertyEnvironment>,
    candidates: &[CustomCascadedDecl],
) -> Arc<CustomPropertyEnvironment> {
    let mut winners: HashMap<SmolStr, (CustomProperty, RankedDecl)> = HashMap::new();
    for (idx, (value, important, origin, specificity, source_order)) in
        candidates.iter().enumerate()
    {
        let candidate = RankedDecl {
            rank: cascade_rank(*origin, *important),
            specificity: *specificity,
            source_order: *source_order,
            idx,
        };
        let replace = winners
            .get(&value.name)
            .is_none_or(|(_, existing)| beats(candidate, *existing));
        if replace {
            winners.insert(value.name.clone(), (value.clone(), candidate));
        }
    }

    let local: HashMap<SmolStr, SmolStr> = winners
        .into_iter()
        .map(|(name, (value, _))| (name, value.value))
        .collect();
    resolve_custom_property_environment(inherited, &local)
}

/// Resolve one element or page context's local custom properties against a
/// shared inherited environment.
///
/// The returned environment owns only local entries. A `None` entry records a
/// declared-but-invalid custom property, which is a guaranteed-invalid value
/// and therefore shadows an inherited value while allowing `var()` fallback.
pub(crate) fn resolve_custom_property_environment(
    inherited: &Arc<CustomPropertyEnvironment>,
    local: &HashMap<SmolStr, SmolStr>,
) -> Arc<CustomPropertyEnvironment> {
    if local.is_empty() {
        return inherited.clone();
    }
    let entries = resolve_local_custom_properties(inherited.as_ref(), local);
    CustomPropertyEnvironment::from_local(inherited, entries)
}

fn resolve_local_custom_properties(
    inherited: &CustomPropertyEnvironment,
    local: &HashMap<SmolStr, SmolStr>,
) -> HashMap<SmolStr, Option<SmolStr>> {
    let names: Vec<SmolStr> = local.keys().cloned().collect();
    let mut resolver = CustomPropertyResolver {
        local,
        inherited,
        cycle_members: find_cycle_members(local),
        memo: HashMap::new(),
        resolving: Vec::new(),
        budget: VariableResolutionBudget::new(MAX_VARIABLE_RESOLUTION_STEPS),
    };
    let mut entries = HashMap::with_capacity(local.len());
    for name in names {
        entries.insert(name.clone(), resolver.resolve(&name, 0));
    }
    entries
}

pub(crate) struct CustomPropertyResolver<'a> {
    pub(crate) local: &'a HashMap<SmolStr, SmolStr>,
    pub(crate) inherited: &'a CustomPropertyEnvironment,
    pub(crate) cycle_members: HashSet<SmolStr>,
    pub(crate) memo: HashMap<(SmolStr, usize), Option<SmolStr>>,
    pub(crate) resolving: Vec<SmolStr>,
    pub(crate) budget: VariableResolutionBudget,
}

impl CustomPropertyResolver<'_> {
    pub(crate) fn resolve(&mut self, name: &str, depth: usize) -> Option<SmolStr> {
        if depth > MAX_VARIABLE_RESOLUTION_DEPTH {
            return None;
        }
        let name: SmolStr = name.into();
        let key = (name.clone(), depth);
        if let Some(value) = self.memo.get(&key) {
            return value.clone();
        }
        if !self.budget.consume() {
            return None;
        }
        if self.cycle_members.contains(&name) {
            return None;
        }
        let Some(raw) = self.local.get(&name).cloned() else {
            return self.inherited.get(&name);
        };
        if self.resolving.iter().any(|current| current == &name) {
            return None;
        }

        self.resolving.push(name.clone());
        let resolved = substitute_vars(
            &raw,
            &mut |reference| self.resolve(reference, depth.saturating_add(1)),
            depth,
        );
        self.resolving.pop();
        if !self.budget.exhausted {
            self.memo.insert(key, resolved.clone());
        }
        if self.budget.exhausted {
            None
        } else {
            resolved
        }
    }
}

pub(crate) fn find_cycle_members(local: &HashMap<SmolStr, SmolStr>) -> HashSet<SmolStr> {
    let mut dependencies: HashMap<SmolStr, Vec<SmolStr>> = HashMap::new();
    for (name, value) in local {
        let mut references = Vec::new();
        if collect_var_references(value, &mut references).is_ok() {
            references.retain(|reference| local.contains_key(reference));
            dependencies.insert(name.clone(), references);
        } else {
            dependencies.insert(name.clone(), Vec::new());
        }
    }

    // Iterative depth-first search keeps a stylesheet with a long variable
    // chain from consuming the Rust call stack while still identifying every
    // back-edge cycle in the per-element dependency graph.
    let mut states: HashMap<SmolStr, u8> = HashMap::new();
    let mut cycle_members = HashSet::new();
    for start in local.keys() {
        if states.get(start).copied().unwrap_or_default() != 0 {
            continue;
        }
        let mut stack = vec![(start.clone(), 0usize)];
        let mut path = vec![start.clone()];
        states.insert(start.clone(), 1);
        while let Some((node, next_index)) = stack.last_mut() {
            let node_name = node.clone();
            let deps = dependencies
                .get(&node_name)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            if *next_index >= deps.len() {
                states.insert(node_name, 2);
                stack.pop();
                path.pop();
                continue;
            }
            let target = deps[*next_index].clone();
            *next_index += 1;
            match states.get(&target).copied().unwrap_or_default() {
                0 => {
                    states.insert(target.clone(), 1);
                    path.push(target.clone());
                    stack.push((target, 0));
                }
                1 => {
                    if let Some(cycle_start) = path.iter().position(|name| name == &target) {
                        cycle_members.extend(path[cycle_start..].iter().cloned());
                    }
                }
                _ => {}
            }
        }
    }
    cycle_members
}

pub(crate) fn collect_var_references(input: &str, references: &mut Vec<SmolStr>) -> Result<(), ()> {
    if input.len() > MAX_SUBSTITUTED_VALUE_BYTES || !css_literals_are_well_formed(input) {
        return Err(());
    }
    let functions = find_function_tokens(input, &["var"]).ok_or(())?;
    for (_, _, open, close) in functions {
        let (name, _) = split_var_arguments(&input[open + 1..close]).ok_or(())?;
        references.push(name);
    }
    Ok(())
}

pub(crate) fn substitute_vars(
    input: &str,
    resolve: &mut impl FnMut(&str) -> Option<SmolStr>,
    depth: usize,
) -> Option<SmolStr> {
    let mut budget = VariableResolutionBudget::new(MAX_VARIABLE_RESOLUTION_STEPS);
    substitute_vars_with_budget(input, resolve, depth, &mut budget)
}

pub(crate) fn substitute_vars_with_budget(
    input: &str,
    resolve: &mut impl FnMut(&str) -> Option<SmolStr>,
    depth: usize,
    budget: &mut VariableResolutionBudget,
) -> Option<SmolStr> {
    if depth > MAX_VARIABLE_RESOLUTION_DEPTH
        || input.len() > MAX_SUBSTITUTED_VALUE_BYTES
        || !css_literals_are_well_formed(input)
    {
        return None;
    }
    let mut output = String::with_capacity(input.len());
    let mut position = 0;
    let functions = find_function_tokens(input, &["var"])?;
    for (_, name_start, open, close) in functions {
        if name_start < position {
            continue;
        }
        if !budget.consume() {
            return None;
        }
        push_bounded(&mut output, &input[position..name_start])?;
        let inner = &input[open + 1..close];
        let (name, fallback) = split_var_arguments(inner)?;
        let replacement = match resolve(name.as_str()) {
            Some(value) => value,
            None => match fallback {
                Some(fallback) => {
                    substitute_vars_with_budget(fallback, resolve, depth.saturating_add(1), budget)?
                }
                None => return None,
            },
        };
        push_bounded(&mut output, &replacement)?;
        if needs_css_token_separator(&output, &input[close + 1..]) {
            // Substitution preserves the original component-value token
            // boundaries. Add a separator only where reparsing the bounded
            // serialization would otherwise merge adjacent tokens.
            push_bounded(&mut output, " ")?;
        }
        position = close.checked_add(1)?;
    }
    push_bounded(&mut output, &input[position..])?;
    Some(output.into())
}

fn css_literals_are_well_formed(input: &str) -> bool {
    let bytes = input.as_bytes();
    let mut position = 0;
    while position < bytes.len() {
        if bytes[position] == b'\'' || bytes[position] == b'"' {
            let Some(end) = skip_css_string(input, position) else {
                return false;
            };
            position = end;
            continue;
        }
        if bytes[position] == b'/' && bytes.get(position + 1) == Some(&b'*') {
            let Some(end) = skip_css_comment(input, position) else {
                return false;
            };
            position = end;
            continue;
        }
        let Some(character) = input[position..].chars().next() else {
            return false; // cov:ignore: a valid Rust str cannot end between UTF-8 scalar values.
        };
        position += character.len_utf8();
    }
    true
}

pub(crate) fn skip_css_string(input: &str, start: usize) -> Option<usize> {
    let quote = input.as_bytes()[start];
    let bytes = input.as_bytes();
    let mut position = start + 1;
    while position < bytes.len() {
        match bytes[position] {
            b'\\' => position = skip_css_escape(input, position)?,
            byte if byte == quote => return Some(position + 1),
            _ => position += 1,
        }
    }
    None
}

pub(crate) fn skip_css_comment(input: &str, start: usize) -> Option<usize> {
    input[start + 2..]
        .find("*/")
        .map(|offset| start + 2 + offset + 2)
}

pub(crate) fn split_var_arguments(input: &str) -> Option<(SmolStr, Option<&str>)> {
    if input.len() > MAX_SUBSTITUTED_VALUE_BYTES {
        return None;
    }
    let bytes = input.as_bytes();
    let mut depth = 0usize;
    let mut position = 0;
    while position < bytes.len() {
        match bytes[position] {
            b'\'' | b'"' => position = skip_css_string(input, position)?,
            b'/' if bytes.get(position + 1) == Some(&b'*') => {
                position = skip_css_comment(input, position)?
            }
            b'(' => {
                depth += 1;
                position += 1;
            }
            b')' => {
                depth = depth.checked_sub(1)?;
                position += 1;
            }
            b'\\' => position = skip_css_escape(input, position)?,
            b',' if depth == 0 => {
                let name = parse_custom_property_reference(&input[..position])?;
                return Some((name, Some(input[position + 1..].trim())));
            }
            _ => position += 1,
        }
    }
    let name = parse_custom_property_reference(input)?;
    Some((name, None))
}

fn parse_custom_property_reference(input: &str) -> Option<SmolStr> {
    let mut parser_input = ParserInput::new(input);
    let mut parser = Parser::new(&mut parser_input);
    let name = parser.expect_ident_cloned().ok()?;
    if !is_custom_property_name(name.as_ref()) {
        return None;
    }
    parser.expect_exhausted().ok()?;
    Some(name.as_ref().into())
}

fn is_css_whitespace_byte(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\x0C' | b'\r')
}

fn is_css_newline_byte(byte: u8) -> bool {
    matches!(byte, b'\n' | b'\x0C' | b'\r')
}

pub(crate) fn skip_css_escape(input: &str, start: usize) -> Option<usize> {
    let bytes = input.as_bytes();
    if bytes.get(start) != Some(&b'\\') {
        return None;
    }
    let mut position = start + 1;
    let first = *bytes.get(position)?;
    if is_css_newline_byte(first) {
        return None;
    }
    if first.is_ascii_hexdigit() {
        let mut digits = 0;
        while digits < 6 && bytes.get(position).is_some_and(u8::is_ascii_hexdigit) {
            digits += 1;
            position += 1;
        }
        if bytes
            .get(position)
            .is_some_and(|byte| is_css_whitespace_byte(*byte))
        {
            if bytes.get(position) == Some(&b'\r') && bytes.get(position + 1) == Some(&b'\n') {
                position += 2;
            } else {
                position += 1;
            }
        } // cov:ignore: this block terminator has no executable statement; both escape branches are covered above.
        return Some(position);
    }
    let character = input[position..].chars().next()?;
    Some(position + character.len_utf8())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cascade::test_support::*;
    use crate::cascade::{apply_value, cascade};
    use crate::property::{
        CssColor, Length, OutlineColor, OutlineStyle, PropertyKey, VerticalAlign,
    };
    use crate::resolve::{
        ComputedLength, ComputedLengthPercentage, ComputedLengthPercentageOrAuto,
        ComputedLineHeight,
    };
    use crate::ruletree::build_rule_tree;
    use crate::specified::SpecifiedValues;
    use crate::test_dom::TestDoc;

    #[test]
    fn var_in_margin_shorthand_preserves_later_longhand_cascade() {
        let cv = cascade_doc(
            "",
            "div",
            Some("--space: 10px; margin: var(--space); margin-left: 20px"),
        );
        assert_eq!(cv.margin.top, ComputedLengthPercentageOrAuto::Px(10.0));
        assert_eq!(cv.margin.right, ComputedLengthPercentageOrAuto::Px(10.0));
        assert_eq!(cv.margin.bottom, ComputedLengthPercentageOrAuto::Px(10.0));
        assert_eq!(cv.margin.left, ComputedLengthPercentageOrAuto::Px(20.0));
    }

    #[test]
    fn var_in_outline_shorthand_projects_each_deferred_longhand() {
        // `outline` expands before cascade; after the custom property is
        // substituted, each deferred longhand must project its component from
        // the reparsed `Outline` shorthand rather than being dropped.
        let cv = cascade_doc(
            "",
            "div",
            Some("--outline: auto 2px red; outline: var(--outline)"),
        );
        assert_eq!(cv.outline.width(), ComputedLength(2.0));
        assert_eq!(cv.outline.style(), OutlineStyle::Auto);
        assert_eq!(cv.outline.color, OutlineColor::Resolved(RED));
    }

    #[test]
    fn var_in_margin_inline_shorthand_preserves_later_longhand_cascade() {
        // Sibling of `var_in_margin_shorthand_preserves_later_longhand_cascade`
        // — `margin-inline` only fans out to `margin-left`/`margin-right`
        // (unlike `margin`'s 4-side fan-out), so the untouched block axis
        // must stay at initial (0), not at the `var()`-substituted value.
        let cv = cascade_doc(
            "",
            "div",
            Some("--space: 10px; margin-inline: var(--space); margin-left: 20px"),
        );
        assert_eq!(cv.margin.left, ComputedLengthPercentageOrAuto::Px(20.0));
        assert_eq!(cv.margin.right, ComputedLengthPercentageOrAuto::Px(10.0));
        assert_eq!(cv.margin.top, ComputedLengthPercentageOrAuto::Px(0.0));
        assert_eq!(cv.margin.bottom, ComputedLengthPercentageOrAuto::Px(0.0));
    }

    #[test]
    fn var_in_margin_block_shorthand_preserves_later_longhand_cascade() {
        // Sibling of `var_in_margin_inline_shorthand_preserves_later_longhand_cascade`
        // for the block-axis 2-value shorthand — exercises
        // `crate::rule::expand_deferred`'s `MarginBlock` arm (only the
        // inline-axis sibling was previously covered by an end-to-end
        // var() test).
        let cv = cascade_doc(
            "",
            "div",
            Some("--space: 10px; margin-block: var(--space); margin-top: 20px"),
        );
        assert_eq!(cv.margin.top, ComputedLengthPercentageOrAuto::Px(20.0));
        assert_eq!(cv.margin.bottom, ComputedLengthPercentageOrAuto::Px(10.0));
        assert_eq!(cv.margin.left, ComputedLengthPercentageOrAuto::Px(0.0));
        assert_eq!(cv.margin.right, ComputedLengthPercentageOrAuto::Px(0.0));
    }

    #[test]
    fn var_in_padding_inline_shorthand_preserves_later_longhand_cascade() {
        // Sibling of `var_in_margin_inline_shorthand_preserves_later_longhand_cascade`
        // for `padding-inline` — exercises `crate::rule::expand_deferred`'s
        // `PaddingInline` arm.
        let cv = cascade_doc(
            "",
            "div",
            Some("--space: 10px; padding-inline: var(--space); padding-left: 20px"),
        );
        assert_eq!(cv.padding.left, ComputedLengthPercentage::Px(20.0));
        assert_eq!(cv.padding.right, ComputedLengthPercentage::Px(10.0));
        assert_eq!(cv.padding.top, ComputedLengthPercentage::Px(0.0));
        assert_eq!(cv.padding.bottom, ComputedLengthPercentage::Px(0.0));
    }

    #[test]
    fn var_in_padding_block_shorthand_preserves_later_longhand_cascade() {
        // Sibling of `var_in_margin_inline_shorthand_preserves_later_longhand_cascade`
        // for `padding-block` — exercises `crate::rule::expand_deferred`'s
        // `PaddingBlock` arm, the last of the 4 logical 2-value shorthands.
        let cv = cascade_doc(
            "",
            "div",
            Some("--space: 10px; padding-block: var(--space); padding-top: 20px"),
        );
        assert_eq!(cv.padding.top, ComputedLengthPercentage::Px(20.0));
        assert_eq!(cv.padding.bottom, ComputedLengthPercentage::Px(10.0));
        assert_eq!(cv.padding.left, ComputedLengthPercentage::Px(0.0));
        assert_eq!(cv.padding.right, ComputedLengthPercentage::Px(0.0));
    }

    #[test]
    fn var_in_padding_inline_start_longhand_preserves_cascade() {
        // `property_key_for_name`'s `"padding-inline-start" =>
        // PropertyKey::PaddingLeft` arm is only exercised by the deferred
        // (`var()`) path when `deferred.property` gets re-parsed by
        // `resolve_deferred_value` — this pins that round-trip for a single
        // logical longhand (the `margin-inline`/`padding-inline` shorthand
        // var() round-trip is covered by
        // `var_in_margin_inline_shorthand_preserves_later_longhand_cascade`
        // above).
        let cv = cascade_doc(
            "",
            "div",
            Some("--space: 7px; padding-inline-start: var(--space)"),
        );
        assert_eq!(cv.padding.left, ComputedLengthPercentage::Px(7.0));
    }

    #[test]
    fn var_in_background_shorthand_projects_each_deferred_longhand() {
        // `background` expands before cascade; after the custom property is
        // substituted, each deferred longhand must project its component
        // from the reparsed `BackgroundShorthand` rather than being dropped
        // (`var_in_outline_shorthand_projects_each_deferred_longhand`
        // sibling, same `project_deferred_value` mechanism).
        use crate::property::{
            BackgroundAttachment, BackgroundRepeat, BackgroundRepeatKeyword, VisualBox,
        };
        let cv = cascade_doc(
            "",
            "div",
            Some("--bg: red round fixed border-box; background: var(--bg)"),
        );
        assert_eq!(cv.background_color, RED);
        assert_eq!(
            cv.background_repeat,
            BackgroundRepeat {
                x: BackgroundRepeatKeyword::Round,
                y: BackgroundRepeatKeyword::Round,
            }
        );
        assert_eq!(cv.background_attachment, BackgroundAttachment::Fixed);
        assert_eq!(cv.background_clip, VisualBox::BorderBox);
        assert_eq!(cv.background_origin, VisualBox::BorderBox);
    }

    #[test]
    fn var_in_font_shorthand_projects_each_deferred_longhand() {
        // `var_in_background_shorthand_projects_each_deferred_longhand` の
        // sibling — `font: var(--f)` expands 6 grammar longhands plus 9
        // reset-only subproperties after substitution.
        use crate::property::{
            FontKerning, FontLanguageOverride, FontOpticalSizing, FontStyle, FontVariantCaps,
            FontVariantEastAsian, FontVariantEastAsianWidth, FontVariantEmoji,
            FontVariantLigatures, FontVariantNumeric, FontVariantPosition, FontVariationSetting,
            FontVariationSettings,
        };
        let mut doc = TestDoc::new();
        let parent = doc.push_element(
            0,
            "p",
            Some(concat!(
                "font-variation-settings: \"wght\" 640; ",
                "font-kerning: normal; font-language-override: \"SRB\"; ",
                "font-optical-sizing: none; font-variant-east-asian: full-width; ",
                "font-variant-emoji: text; font-variant-ligatures: none; ",
                "font-variant-numeric: ordinal; font-variant-position: sub",
            )),
        );
        let child = doc.push_element(
            parent,
            "div",
            Some("--f: italic small-caps bold 20px/1.5 serif; font: var(--f)"),
        );
        let child_with_later_longhand = doc.push_element(
            parent,
            "em",
            Some(concat!(
                "--f: italic small-caps bold 20px/1.5 serif; font: var(--f); ",
                "font-kerning: normal; font-language-override: \"SRB\"; ",
                "font-optical-sizing: none; font-variant-east-asian: full-width; ",
                "font-variant-emoji: text; font-variant-ligatures: none; ",
                "font-variant-numeric: ordinal; font-variant-position: sub; ",
                "font-variation-settings: \"wght\" 700",
            )),
        );
        let tree = build_rule_tree(&doc);
        let result = cascade(&doc, &tree).expect("cascade Ok");
        let parent_cv = &result.computed[parent];
        let cv = &result.computed[child];
        let child_later_cv = &result.computed[child_with_later_longhand];

        assert_eq!(cv.font_style, FontStyle::Italic);
        assert_eq!(cv.font_variant_caps, FontVariantCaps::SmallCaps);
        assert_eq!(cv.font_weight, 700.0);
        assert_eq!(cv.font_size, ComputedLength(20.0));
        assert_eq!(cv.line_height, ComputedLineHeight::Number(1.5));
        assert_eq!(cv.font_family[0].to_string(), "serif");
        assert_eq!(parent_cv.font_kerning, FontKerning::Normal);
        assert_eq!(
            parent_cv.font_language_override,
            FontLanguageOverride::String("SRB".into())
        );
        assert_eq!(parent_cv.font_optical_sizing, FontOpticalSizing::None);
        assert_eq!(
            parent_cv.font_variant_east_asian.width,
            Some(FontVariantEastAsianWidth::FullWidth)
        );
        assert_eq!(parent_cv.font_variant_emoji, FontVariantEmoji::Text);
        assert_eq!(parent_cv.font_variant_ligatures, FontVariantLigatures::None);
        assert!(parent_cv.font_variant_numeric.ordinal);
        assert_eq!(parent_cv.font_variant_position, FontVariantPosition::Sub);
        assert_eq!(
            parent_cv.font_variation_settings,
            FontVariationSettings::Settings(vec![FontVariationSetting {
                tag: "wght".into(),
                value: 640.0,
            }])
        );

        assert_eq!(cv.font_kerning, FontKerning::Auto);
        assert_eq!(cv.font_language_override, FontLanguageOverride::Normal);
        assert_eq!(cv.font_optical_sizing, FontOpticalSizing::Auto);
        assert_eq!(cv.font_variant_east_asian, FontVariantEastAsian::initial());
        assert_eq!(cv.font_variant_emoji, FontVariantEmoji::Normal);
        assert_eq!(cv.font_variant_ligatures, FontVariantLigatures::Normal);
        assert_eq!(cv.font_variant_numeric, FontVariantNumeric::initial());
        assert_eq!(cv.font_variant_position, FontVariantPosition::Normal);
        assert_eq!(cv.font_variation_settings, FontVariationSettings::Normal);

        assert_eq!(child_later_cv.font_kerning, FontKerning::Normal);
        assert_eq!(
            child_later_cv.font_language_override,
            FontLanguageOverride::String("SRB".into())
        );
        assert_eq!(child_later_cv.font_optical_sizing, FontOpticalSizing::None);
        assert_eq!(
            child_later_cv.font_variant_east_asian.width,
            Some(FontVariantEastAsianWidth::FullWidth)
        );
        assert_eq!(child_later_cv.font_variant_emoji, FontVariantEmoji::Text);
        assert_eq!(
            child_later_cv.font_variant_ligatures,
            FontVariantLigatures::None
        );
        assert!(child_later_cv.font_variant_numeric.ordinal);
        assert_eq!(
            child_later_cv.font_variant_position,
            FontVariantPosition::Sub
        );
        assert_eq!(
            child_later_cv.font_variation_settings,
            FontVariationSettings::Settings(vec![FontVariationSetting {
                tag: "wght".into(),
                value: 700.0,
            }])
        );
    }

    #[test]
    fn deferred_projection_covers_supported_shorthands() {
        use crate::property::PropertyKey;

        fn parse_static(name: &str, source: &str) -> PropertyValue {
            let mut input = ParserInput::new(source);
            let mut parser = Parser::new(&mut input);
            let value = parse_value(name, &mut parser).expect("valid static shorthand");
            parser
                .expect_exhausted()
                .expect("static shorthand is exhaustive");
            value
        }

        let cases = vec![
            (
                "padding",
                "1px 2px 3px 4px",
                vec![
                    PropertyKey::PaddingTop,
                    PropertyKey::PaddingRight,
                    PropertyKey::PaddingBottom,
                    PropertyKey::PaddingLeft,
                ],
            ),
            (
                "margin-inline",
                "1px 2px",
                vec![PropertyKey::MarginLeft, PropertyKey::MarginRight],
            ),
            (
                "margin-block",
                "1px 2px",
                vec![PropertyKey::MarginTop, PropertyKey::MarginBottom],
            ),
            (
                "padding-inline",
                "1px 2px",
                vec![PropertyKey::PaddingLeft, PropertyKey::PaddingRight],
            ),
            (
                "padding-block",
                "1px 2px",
                vec![PropertyKey::PaddingTop, PropertyKey::PaddingBottom],
            ),
            (
                "margin",
                "1px 2px 3px 4px",
                vec![
                    PropertyKey::MarginTop,
                    PropertyKey::MarginRight,
                    PropertyKey::MarginBottom,
                    PropertyKey::MarginLeft,
                ],
            ),
            (
                "border",
                "1px solid red",
                vec![
                    PropertyKey::BorderTopWidth,
                    PropertyKey::BorderTopStyle,
                    PropertyKey::BorderTopColor,
                    PropertyKey::BorderRightWidth,
                    PropertyKey::BorderRightStyle,
                    PropertyKey::BorderRightColor,
                    PropertyKey::BorderBottomWidth,
                    PropertyKey::BorderBottomStyle,
                    PropertyKey::BorderBottomColor,
                    PropertyKey::BorderLeftWidth,
                    PropertyKey::BorderLeftStyle,
                    PropertyKey::BorderLeftColor,
                ],
            ),
            (
                "outline",
                "auto 2px red",
                vec![
                    PropertyKey::OutlineWidth,
                    PropertyKey::OutlineStyle,
                    PropertyKey::OutlineColor,
                ],
            ),
            (
                "overflow",
                "hidden scroll",
                vec![PropertyKey::OverflowX, PropertyKey::OverflowY],
            ),
            (
                "text-decoration",
                "underline wavy red",
                vec![
                    PropertyKey::TextDecorationLine,
                    PropertyKey::TextDecorationThickness,
                    PropertyKey::TextDecorationStyle,
                    PropertyKey::TextDecorationColor,
                ],
            ),
            (
                "text-emphasis",
                "dot red",
                vec![
                    PropertyKey::TextEmphasisStyle,
                    PropertyKey::TextEmphasisColor,
                ],
            ),
            (
                "flex",
                "2 3 10px",
                vec![
                    PropertyKey::FlexGrow,
                    PropertyKey::FlexShrink,
                    PropertyKey::FlexBasis,
                ],
            ),
            (
                "gap",
                "1px 2px",
                vec![PropertyKey::RowGap, PropertyKey::ColumnGap],
            ),
            (
                "place-content",
                "center space-between",
                vec![PropertyKey::AlignContent, PropertyKey::JustifyContent],
            ),
            (
                "grid-row",
                "2 / 5",
                vec![PropertyKey::GridRowStart, PropertyKey::GridRowEnd],
            ),
            (
                "grid-column",
                "main-start / main-end",
                vec![PropertyKey::GridColumnStart, PropertyKey::GridColumnEnd],
            ),
            (
                "place-items",
                "center stretch",
                vec![PropertyKey::AlignItems, PropertyKey::JustifyItems],
            ),
            (
                "place-self",
                "center stretch",
                vec![PropertyKey::AlignSelf, PropertyKey::JustifySelf],
            ),
            (
                "background",
                "url(a.png) top / cover no-repeat fixed border-box red",
                vec![
                    PropertyKey::BackgroundColor,
                    PropertyKey::BackgroundImage,
                    PropertyKey::BackgroundRepeat,
                    PropertyKey::BackgroundAttachment,
                    PropertyKey::BackgroundPosition,
                    PropertyKey::BackgroundSize,
                    PropertyKey::BackgroundClip,
                    PropertyKey::BackgroundOrigin,
                ],
            ),
            (
                "font",
                "italic small-caps bold 12px/1.5 serif",
                vec![
                    PropertyKey::FontStyle,
                    PropertyKey::FontVariantCaps,
                    PropertyKey::FontWeight,
                    PropertyKey::FontSize,
                    PropertyKey::LineHeight,
                    PropertyKey::FontFamily,
                    PropertyKey::FontKerning,
                    PropertyKey::FontLanguageOverride,
                    PropertyKey::FontOpticalSizing,
                    PropertyKey::FontVariantEastAsian,
                    PropertyKey::FontVariantEmoji,
                    PropertyKey::FontVariantLigatures,
                    PropertyKey::FontVariantNumeric,
                    PropertyKey::FontVariantPosition,
                    PropertyKey::FontVariationSettings,
                ],
            ),
            (
                "font",
                "italic larger serif",
                vec![
                    PropertyKey::FontStyle,
                    PropertyKey::FontVariantCaps,
                    PropertyKey::FontWeight,
                    PropertyKey::FontSize,
                    PropertyKey::LineHeight,
                    PropertyKey::FontFamily,
                    PropertyKey::FontKerning,
                    PropertyKey::FontLanguageOverride,
                    PropertyKey::FontOpticalSizing,
                    PropertyKey::FontVariantEastAsian,
                    PropertyKey::FontVariantEmoji,
                    PropertyKey::FontVariantLigatures,
                    PropertyKey::FontVariantNumeric,
                    PropertyKey::FontVariantPosition,
                    PropertyKey::FontVariationSettings,
                ],
            ),
        ];

        for (name, source, keys) in cases {
            let value = parse_static(name, source);
            for key in keys {
                assert!(project_deferred_value(value.clone(), key).is_some());
            }
            assert!(project_deferred_value(value, PropertyKey::Color).is_none());
        }
        assert!(project_deferred_value(PropertyValue::Color(RED), PropertyKey::Width).is_none());
    }

    #[test]
    fn direct_apply_ignores_precomputed_custom_values() {
        let initial = SpecifiedValues::initial();
        let mut specified = initial.clone();
        apply_value(
            PropertyValue::CustomProperty(CustomProperty {
                name: "--unused".into(),
                value: "red".into(),
            }),
            &mut specified,
        );
        apply_value(
            PropertyValue::Deferred(DeferredValue {
                property: "color".into(),
                value: "red".into(),
                key: PropertyKey::Color,
            }),
            &mut specified,
        );
        assert_eq!(specified, initial);
    }

    #[test]
    fn math_helpers_cover_nested_and_rejected_forms() {
        assert_eq!(
            simplify_math_functions(r#"rgb(calc(1px + 1px), 0, 0)"#),
            Some("rgb(2px, 0, 0)".into())
        );
        assert_eq!(
            simplify_math_functions(r#""calc(1px)" /* calc(2px) */"#),
            Some(r#""calc(1px)" /* calc(2px) */"#.into())
        );
        assert_eq!(
            simplify_math_functions("calc(calc(1px))"),
            Some("1px".into())
        );
        assert_eq!(
            simplify_math_functions_at_depth("calc(1px)", MAX_VARIABLE_RESOLUTION_DEPTH + 1),
            None
        );
        assert_eq!(simplify_math_functions("(calc(1px))"), Some("(1px)".into()));
        assert_eq!(simplify_math_functions("[calc(1px)]"), Some("[1px]".into()));
        assert_eq!(simplify_math_functions("{calc(1px)}"), Some("{1px}".into()));
        assert!(needs_css_token_separator("-", "a"));
        assert!(needs_css_token_separator("+", "2"));
        assert!(needs_css_token_separator(".", "2"));
        assert!(needs_css_token_separator("#", "a"));
        assert!(needs_css_token_separator("10", "--foo"));
        assert!(needs_css_token_separator("10", "_foo"));
        assert!(needs_css_token_separator("10", r"\66 oo"));
        assert!(needs_css_token_separator("10", "é"));
        assert!(needs_css_token_separator("#", "1"));
        assert_eq!(
            evaluate_math_function("calc", &"x".repeat(MAX_SUBSTITUTED_VALUE_BYTES + 1)),
            None
        );
        assert_eq!(
            evaluate_math_function("calc", "1 + 2"),
            Some("3".to_owned())
        );
        assert_eq!(evaluate_math_function("calc", "1 + 2px"), None);
        // Additive zero vanishes across units (CSS Values 4 §10.7).
        assert_eq!(
            evaluate_math_function("calc", "50% + 0px"),
            Some("50%".to_owned())
        );
        assert_eq!(
            evaluate_math_function("calc", "0px + 50%"),
            Some("50%".to_owned())
        );
        assert_eq!(
            evaluate_math_function("calc", "10px + 0"),
            Some("10px".to_owned())
        );

        assert_eq!(evaluate_math_function("min", ""), None);
        assert_eq!(evaluate_math_function("min", "1px, 2em"), None);
        assert_eq!(evaluate_math_function("clamp", "1px, 2px"), None);
        assert_eq!(evaluate_math_function("clamp", "1px, 2em, 3px"), None);
        assert_eq!(evaluate_math_function("unknown", "1"), None);
        assert_eq!(
            serialize_math_value(MathValue {
                number: f32::INFINITY,
                unit: None,
            }),
            None
        );
        assert_eq!(
            serialize_math_value(MathValue {
                number: -0.0,
                unit: None,
            }),
            Some("0".to_owned())
        );
        let mut empty_values: Vec<MathValue> = Vec::new();
        assert_eq!(normalize_math_values(&mut empty_values), None);
        let mut overflowing_pair_left = MathValue {
            number: f32::MAX,
            unit: Some("in".to_owned()),
        };
        let mut overflowing_pair_right = MathValue {
            number: 1.0,
            unit: Some("px".to_owned()),
        };
        assert_eq!(
            normalize_math_pair(&mut overflowing_pair_left, &mut overflowing_pair_right),
            None
        );
        let mut overflowing_values = vec![
            MathValue {
                number: f32::MAX,
                unit: Some("in".to_owned()),
            },
            MathValue {
                number: 1.0,
                unit: Some("px".to_owned()),
            },
        ];
        assert_eq!(normalize_math_values(&mut overflowing_values), None);
        assert_eq!(
            serialize_math_value(MathValue {
                number: 1.0,
                unit: Some("x".repeat(MAX_SUBSTITUTED_VALUE_BYTES)),
            }),
            None
        );

        assert!(MathParser::new("1px 2px").parse().is_none());
        assert!(MathParser::new("1px+2px").parse().is_none());
        assert!(MathParser::new("1px +2px").parse().is_none());
        assert!(MathParser::new("1px + 2em").parse().is_none());
        assert_eq!(
            MathParser::new("-2px").parse().map(|value| value.number),
            Some(-2.0)
        );
        assert_eq!(
            MathParser::new("5px - 2px")
                .parse()
                .map(|value| value.number),
            Some(3.0)
        );
        assert!(MathParser::new("3e38px + 3e38px").parse().is_none());
        assert!(MathParser::new("2px *").parse().is_none());
        assert_eq!(
            MathParser::new("2 * 3px").parse().map(|value| value.unit),
            Some(Some("px".to_owned()))
        );
        assert_eq!(
            MathParser::new("2px * 3").parse().map(|value| value.number),
            Some(6.0)
        );
        assert_eq!(
            MathParser::new("6px / 2").parse().map(|value| value.number),
            Some(3.0)
        );
        assert!(MathParser::new("2px * 3px").parse().is_none());
        assert!(MathParser::new("2px / 0").parse().is_none());
        assert!(MathParser::new("3e38 * 3").parse().is_none());
        assert_eq!(
            MathParser::new("(1px + 2px)")
                .parse()
                .map(|value| value.number),
            Some(3.0)
        );
        assert!(MathParser::new("(1px").parse().is_none());
        assert!(MathParser::new(".").parse().is_none());
        assert_eq!(
            MathParser::new("1.5px").parse().map(|value| value.number),
            Some(1.5)
        );
        assert_eq!(
            MathParser::new("1e-2px").parse().map(|value| value.number),
            Some(0.01)
        );
        assert!(MathParser::new("1e+px").parse().is_none());
        assert_eq!(
            MathParser::new("50%").parse().map(|value| value.unit),
            Some(Some("%".to_owned()))
        );

        assert_eq!(
            split_top_level_commas(r#""a,b", 1px"#),
            Some(vec![r#""a,b""#, "1px"])
        );
        assert_eq!(
            split_top_level_commas("1px /*,*/, 2px"),
            Some(vec!["1px /*,*/", "2px"])
        );
        assert_eq!(
            split_top_level_commas("min(1px, 2px), 3px"),
            Some(vec!["min(1px, 2px)", "3px"])
        );
        assert_eq!(
            split_top_level_commas(r"foo\,bar, baz"),
            Some(vec![r"foo\,bar", "baz"])
        );
        assert_eq!(split_top_level_commas("{a,b}, c"), Some(vec!["{a,b}", "c"]));
        assert_eq!(split_top_level_commas(")"), None);
        assert_eq!(split_top_level_commas("[a,b"), None);
        let nested = format!(
            "{}1{}",
            "(".repeat(MAX_VARIABLE_RESOLUTION_DEPTH + 1),
            ")".repeat(MAX_VARIABLE_RESOLUTION_DEPTH + 1)
        );
        assert!(MathParser::new(&nested).parse().is_none());
        let too_deep = format!(
            "{}1{}",
            "[".repeat(MAX_VARIABLE_RESOLUTION_DEPTH + 1),
            "]".repeat(MAX_VARIABLE_RESOLUTION_DEPTH + 1)
        );
        assert_eq!(split_top_level_commas(&too_deep), None);
    }

    #[test]
    fn variable_scanner_handles_literals_bounds_and_fallbacks() {
        let mut local = HashMap::from([(SmolStr::from("--a"), SmolStr::from("red"))]);
        let inherited = CustomPropertyEnvironment::from_map(HashMap::new());
        {
            let mut resolver = CustomPropertyResolver {
                local: &local,
                inherited: &inherited,
                cycle_members: HashSet::new(),
                memo: HashMap::new(),
                resolving: Vec::new(),
                budget: VariableResolutionBudget::new(MAX_VARIABLE_RESOLUTION_STEPS),
            };
            assert_eq!(resolver.resolve("--a", 0), Some("red".into()));
            assert_eq!(resolver.resolve("--a", 0), Some("red".into()));
        }
        let mut resolver = CustomPropertyResolver {
            local: &local,
            inherited: &inherited,
            cycle_members: HashSet::new(),
            memo: HashMap::new(),
            resolving: vec!["--a".into()],
            budget: VariableResolutionBudget::new(MAX_VARIABLE_RESOLUTION_STEPS),
        };
        assert_eq!(resolver.resolve("--a", 0), None);
        let branching_local = HashMap::from([
            (
                SmolStr::from("--root"),
                SmolStr::from("var(--shared) var(--shared)"),
            ),
            (SmolStr::from("--shared"), SmolStr::from("red")),
        ]);
        let branching_inherited = CustomPropertyEnvironment::from_map(HashMap::new());
        let mut resolver = CustomPropertyResolver {
            local: &branching_local,
            inherited: &branching_inherited,
            cycle_members: HashSet::new(),
            memo: HashMap::new(),
            resolving: Vec::new(),
            budget: VariableResolutionBudget::new(MAX_VARIABLE_RESOLUTION_STEPS),
        };
        assert_eq!(resolver.resolve("--root", 0), Some("red red".into()));
        assert!(resolver.memo.contains_key(&(SmolStr::from("--shared"), 1)));

        let budget_local = HashMap::from([
            (SmolStr::from("--a"), SmolStr::from("var(--b)")),
            (SmolStr::from("--b"), SmolStr::from("var(--c)")),
            (SmolStr::from("--c"), SmolStr::from("red")),
        ]);
        let mut resolver = CustomPropertyResolver {
            local: &budget_local,
            inherited: &inherited,
            cycle_members: HashSet::new(),
            memo: HashMap::new(),
            resolving: Vec::new(),
            budget: VariableResolutionBudget::new(2),
        };
        assert_eq!(resolver.resolve("--a", 0), None);
        assert!(resolver.budget.exhausted);
        local.insert("--broken".into(), "\"unterminated".into());
        assert!(find_cycle_members(&local).is_empty());

        let mut references = Vec::new();
        assert!(
            collect_var_references(
                r#""var(--ignored)" /* var(--also-ignored) */ var(--used)"#,
                &mut references
            )
            .is_ok()
        );
        assert_eq!(references, vec![SmolStr::from("--used")]);
        assert!(collect_var_references("\"unterminated", &mut Vec::new()).is_err());
        assert!(collect_var_references("/* unterminated", &mut Vec::new()).is_err());

        fn no_resolution(_: &str) -> Option<SmolStr> {
            None
        }
        assert_eq!(
            substitute_vars(
                r#""var(--ignored)" /* var(--also-ignored) */ blue"#,
                &mut no_resolution,
                0
            ),
            Some(r#""var(--ignored)" /* var(--also-ignored) */ blue"#.into())
        );
        assert_eq!(
            substitute_vars("var(--missing, blue)", &mut no_resolution, 0),
            Some("blue".into())
        );
        assert_eq!(
            substitute_vars(
                "var(--missing, var(--also-missing, red))",
                &mut no_resolution,
                0
            ),
            Some("red".into())
        );
        assert_eq!(
            substitute_vars(
                "var(--x)",
                &mut |_| Some("x".repeat(MAX_SUBSTITUTED_VALUE_BYTES + 1).into()),
                0
            ),
            None
        );
        assert_eq!(
            substitute_vars("var(--x)", &mut |_| None, MAX_VARIABLE_RESOLUTION_DEPTH + 1),
            None
        );

        assert_eq!(skip_css_string(r#""a\"b""#, 0), Some(6));
        assert_eq!(skip_css_string("\"unterminated", 0), None);
        assert_eq!(skip_css_comment("/* comment */", 0), Some(13));
        assert_eq!(skip_css_comment("/* unterminated", 0), None);
        assert_eq!(skip_css_escape("x", 0), None);
        assert_eq!(skip_css_escape("\\\n", 0), None);
        assert_eq!(skip_css_escape(r"\31 ", 0), Some(4));
        assert_eq!(skip_css_escape("\\31\r\n", 0), Some(5));
        assert_eq!(
            split_var_arguments("--x, var(--y, red)"),
            Some((SmolStr::from("--x"), Some("var(--y, red)")))
        );
        assert_eq!(
            split_var_arguments("--x, \"a,b\" /* comment */"),
            Some((SmolStr::from("--x"), Some("\"a,b\" /* comment */")))
        );
        assert_eq!(
            split_var_arguments("--x, foo(bar)"),
            Some((SmolStr::from("--x"), Some("foo(bar)")))
        );
        assert_eq!(split_var_arguments("color, red"), None);
        assert_eq!(split_var_arguments("\"--x\", red"), None);
        assert_eq!(
            split_var_arguments("--x /* comment */, red"),
            Some((SmolStr::from("--x"), Some("red")))
        );
        assert_eq!(split_var_arguments("--x(foo), red"), None);
        let (escaped_name, escaped_fallback) = split_var_arguments("--x\\,, red").unwrap();
        assert_eq!(escaped_name, "--x,");
        assert_eq!(escaped_fallback, Some("red"));
        assert_eq!(split_var_arguments("--x)"), None);
    }

    #[test]
    fn variable_substitution_preserves_number_identifier_boundary() {
        assert_eq!(
            substitute_vars("var(--n)--foo", &mut |_| Some("10".into()), 0),
            Some("10 --foo".into())
        );
        assert_eq!(
            substitute_vars("var(--missing, [foo)bar])", &mut |_| None, 0),
            Some("[foo)bar]".into())
        );
    }

    #[test]
    fn component_value_scanners_bound_nesting_and_blocks() {
        let nested = format!(
            "{}var(--x){}",
            "[".repeat(MAX_VARIABLE_RESOLUTION_DEPTH + 1),
            "]".repeat(MAX_VARIABLE_RESOLUTION_DEPTH + 1)
        );
        assert_eq!(find_function_tokens(&nested, &["var"]), None);
        assert_eq!(split_top_level_commas("[a,b], c"), Some(vec!["[a,b]", "c"]));
    }

    #[test]
    fn custom_property_substitutes_into_inherited_color() {
        let cv = cascade_doc("", "p", Some("--accent: red; color: var(--accent)"));
        assert_eq!(cv.color, RED);
    }

    #[test]
    fn var_substitutes_inside_a_nested_function() {
        let cv = cascade_doc("", "p", Some("--red: 255; color: rgb(var(--red), 0, 0)"));
        assert_eq!(cv.color, RED);
    }

    #[test]
    fn custom_property_inherits_to_child() {
        let mut doc = TestDoc::new();
        let parent = doc.push_element(0, "div", Some("--accent: blue"));
        let child = doc.push_element(parent, "p", Some("color: var(--accent, red)"));
        let tree = build_rule_tree(&doc);
        let result = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(result.computed[child].color, BLUE);
    }

    #[test]
    fn deep_custom_property_chain_keeps_persistent_environment_deltas() {
        use std::fmt::Write as _;

        const DEPTH: usize = 256;
        let mut doc = TestDoc::new();
        let mut parent = 0;
        let mut ids = Vec::with_capacity(DEPTH);
        for index in 0..DEPTH {
            let mut style = String::new();
            write!(style, "--chain-{index}: {index}px").unwrap();
            let id = doc.push_element(parent, "div", Some(&style));
            ids.push(id);
            parent = id;
        }

        let tree = build_rule_tree(&doc);
        let result = cascade(&doc, &tree).expect("cascade Ok");

        // Every environment owns exactly this element's declaration. The
        // complete chain is shared through parent Arc pointers, so retained
        // local entries are O(N), not O(N²).
        let mut total_local_entries = 0;
        for (index, id) in ids.iter().copied().enumerate() {
            let environment = &result.computed[id].custom_properties;
            assert_eq!(environment.local_entry_count(), 1);
            total_local_entries += environment.local_entry_count();
            if index > 0 {
                let parent_environment = environment
                    .parent_environment()
                    .expect("non-root custom environment has a parent");
                // cov:ignore: panic-message literal only executed on assertion
                // failure, which does not happen while this test passes.
                assert!(
                    std::sync::Arc::ptr_eq(
                        parent_environment,
                        &result.computed[ids[index - 1]].custom_properties
                    ),
                    "custom environment must share the immediate ancestor's environment"
                );
            }
        }
        assert_eq!(total_local_entries, DEPTH);
        let leaf_environment = &result.computed[*ids.last().unwrap()].custom_properties;
        assert_eq!(leaf_environment.get("--chain-0"), Some("0px".into()));
        assert_eq!(leaf_environment.get("--chain-255"), Some("255px".into()));
    }

    #[test]
    fn invalid_var_keeps_inherited_property_value() {
        let mut doc = TestDoc::new();
        let parent = doc.push_element(0, "div", Some("color: red"));
        let child = doc.push_element(parent, "p", Some("color: var(--missing)"));
        let tree = build_rule_tree(&doc);
        let result = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(result.computed[child].color, RED);
    }

    #[test]
    fn invalid_var_uses_initial_for_non_inherited_property() {
        let mut doc = TestDoc::new();
        let parent = doc.push_element(0, "div", Some("width: 20px"));
        let child = doc.push_element(parent, "p", Some("width: var(--missing)"));
        let tree = build_rule_tree(&doc);
        let result = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            result.computed[child].width,
            ComputedLengthPercentageOrAuto::Auto
        );
    }

    #[test]
    fn invalid_custom_property_overrides_inherited_value() {
        let mut doc = TestDoc::new();
        let parent = doc.push_element(0, "div", Some("--accent: red"));
        let child = doc.push_element(
            parent,
            "p",
            Some("--accent: var(--missing); color: var(--accent, blue)"),
        );
        let tree = build_rule_tree(&doc);
        let result = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(result.computed[child].color, BLUE);
    }

    #[test]
    fn missing_custom_property_uses_var_fallback() {
        let cv = cascade_doc("", "p", Some("color: var(--missing, blue)"));
        assert_eq!(cv.color, BLUE);
    }

    #[test]
    fn custom_property_rejects_top_level_bang_except_important() {
        let cv = cascade_doc(
            "",
            "p",
            Some("--accent: !not-important; color: var(--accent, blue)"),
        );
        assert_eq!(cv.color, BLUE);
    }

    #[test]
    fn custom_property_important_wins_and_is_stripped_before_substitution() {
        let cv = cascade_doc(
            "",
            "p",
            Some("--accent: red !important; --accent: blue; color: var(--accent)"),
        );
        assert_eq!(cv.color, RED);
    }

    #[test]
    fn custom_property_names_are_case_sensitive() {
        let cv = cascade_doc(
            "",
            "p",
            Some("--accent: red; --Accent: blue; color: var(--Accent)"),
        );
        assert_eq!(cv.color, BLUE);
    }

    #[test]
    fn cyclic_custom_property_uses_var_fallback() {
        let cv = cascade_doc(
            "",
            "p",
            Some("--a: var(--b); --b: var(--a); color: var(--a, blue)"),
        );
        assert_eq!(cv.color, BLUE);
    }

    #[test]
    fn fallback_references_do_not_rescue_a_custom_property_cycle() {
        let cv = cascade_doc(
            "",
            "p",
            Some(
                "--a: var(--b, red); --b: var(--a, blue); \
                 color: var(--a)",
            ),
        );
        assert_eq!(cv.color, CssColor::BLACK);
    }

    #[test]
    fn overly_deep_finite_variable_chain_is_bounded() {
        use std::fmt::Write as _;

        let mut css = String::new();
        for index in 0..256 {
            write!(css, "--v{index}: var(--v{}); ", index + 1).unwrap();
        }
        css.push_str("--v256: red; color: var(--v0)");

        let cv = cascade_doc("", "p", Some(&css));
        assert_eq!(cv.color, CssColor::BLACK);
    }

    #[test]
    fn overly_deep_nested_var_fallback_is_bounded() {
        let mut fallback = String::from("red");
        for index in 0..256 {
            fallback = format!("var(--missing-{index}, {fallback})");
        }
        let css = format!("color: {fallback}");

        let cv = cascade_doc("", "p", Some(&css));
        assert_eq!(cv.color, CssColor::BLACK);
    }

    #[test]
    fn calc_substituted_custom_property_reaches_width() {
        let cv = cascade_doc(
            "",
            "div",
            Some("--spacing: 10px; width: calc(var(--spacing) + 5px)"),
        );
        assert_eq!(cv.width, ComputedLengthPercentageOrAuto::Px(15.0));
    }

    #[test]
    fn calc_length_percentage_value_targets_sizing_inset_and_vertical_align() {
        let value = CalcLengthPercentage {
            percent: 50.0,
            px: 10.0,
        };
        let calc = LengthOrAuto::Calc(value);
        let cases = [
            (PropertyKey::Width, PropertyValue::Width(calc)),
            (PropertyKey::Height, PropertyValue::Height(calc)),
            (PropertyKey::MaxWidth, PropertyValue::MaxWidth(calc)),
            (PropertyKey::MaxHeight, PropertyValue::MaxHeight(calc)),
            (PropertyKey::MinWidth, PropertyValue::MinWidth(calc)),
            (PropertyKey::MinHeight, PropertyValue::MinHeight(calc)),
            (PropertyKey::MinBlockSize, PropertyValue::MinBlockSize(calc)),
            (PropertyKey::Top, PropertyValue::Top(calc)),
            (PropertyKey::Right, PropertyValue::Right(calc)),
            (PropertyKey::Bottom, PropertyValue::Bottom(calc)),
            (PropertyKey::Left, PropertyValue::Left(calc)),
            (
                PropertyKey::VerticalAlign,
                PropertyValue::VerticalAlign(VerticalAlign::Calc(value)),
            ),
        ];
        for (key, expected) in cases {
            assert_eq!(calc_length_percentage_value(key, value), Some(expected));
        }
        assert_eq!(
            calc_length_percentage_value(PropertyKey::Color, value),
            None
        );
    }

    #[test]
    fn vertical_align_wpt_calc_expressions_resolve_to_px() {
        let cases = [
            ("calc(50px)", 50.0),
            ("calc(50%)", 50.0),
            ("calc(25px + 50%)", 75.0),
            ("calc(150% / 2 - 30px)", 45.0),
            ("calc(40px + 10% - 20% / 2)", 40.0),
            ("calc(40px - 10%)", 30.0),
        ];
        for (expression, expected_px) in cases {
            let inline = format!("line-height: 100px; vertical-align: {expression}");
            let cv = cascade_doc("", "span", Some(&inline));
            assert_eq!(
                cv.vertical_align,
                VerticalAlign::Length(Length::Px(expected_px)),
                "{expression}" // cov:ignore: assertion diagnostic literal is formatted only on failure
            );
        }
    }

    #[test]
    fn parse_simple_calc_length_percentage_rejects_non_calc_prefix() {
        assert_eq!(parse_simple_calc_length_percentage("foo(50%)"), None);
    }

    #[test]
    fn parse_simple_calc_length_percentage_rejects_missing_closing_paren() {
        assert_eq!(parse_simple_calc_length_percentage("calc(50%"), None);
    }

    #[test]
    fn parse_simple_calc_length_percentage_handles_nested_parens() {
        assert_eq!(
            parse_simple_calc_length_percentage("calc((50%) + 10px)"),
            Some(CalcLengthPercentage {
                percent: 50.0,
                px: 10.0
            })
        );
    }

    #[test]
    fn parse_simple_calc_length_percentage_handles_single_term() {
        assert_eq!(
            parse_simple_calc_length_percentage("calc(50%)"),
            Some(CalcLengthPercentage {
                percent: 50.0,
                px: 0.0
            })
        );
    }

    #[test]
    fn parse_simple_calc_length_percentage_rejects_unknown_unit() {
        assert_eq!(
            parse_simple_calc_length_percentage("calc(50deg + 10px)"),
            None
        );
    }

    #[test]
    fn parse_simple_calc_length_percentage_rejects_non_finite_term() {
        // `1e40` overflows f32 to infinity — the parsed term's `number` is
        // no longer finite, which must be rejected rather than propagated
        // into a `CalcLengthPercentage`.
        assert_eq!(
            parse_simple_calc_length_percentage("calc(1e40% + 10px)"),
            None
        );
    }

    #[test]
    fn invalid_math_declaration_is_dropped_before_cascade() {
        let cv = cascade_doc("", "div", Some("width: 10px; width: calc(foo)"));
        assert_eq!(cv.width, ComputedLengthPercentageOrAuto::Px(10.0));
    }

    #[test]
    fn hash_prefixed_var_text_is_not_substituted() {
        let cv = cascade_doc("", "div", Some("color: red; color: #var(--missing)"));
        assert_eq!(cv.color, RED);
    }

    #[test]
    fn min_max_and_clamp_reach_width() {
        let min = cascade_doc("", "div", Some("width: min(20px, 10px)"));
        let max = cascade_doc("", "div", Some("width: max(10px, 20px)"));
        let clamp = cascade_doc("", "div", Some("width: clamp(5px, 20px, 10px)"));
        assert_eq!(min.width, ComputedLengthPercentageOrAuto::Px(10.0));
        assert_eq!(max.width, ComputedLengthPercentageOrAuto::Px(20.0));
        assert_eq!(clamp.width, ComputedLengthPercentageOrAuto::Px(10.0));
    }

    #[test]
    fn var_substitution_does_not_join_adjacent_tokens() {
        // `var(--n)px` is not a valid way to form a dimension: substitution
        // happens at token level, so the result is the two-token sequence
        // `10` + `px`, not a newly reparsed `10px` token.
        let cv = cascade_doc("", "div", Some("--n: 10; width: var(--n)px"));
        assert_eq!(cv.width, ComputedLengthPercentageOrAuto::Auto);
        assert_eq!(simplify_math_functions("calc(10)px"), Some("10 px".into()));
    }

    #[test]
    fn var_substitution_does_not_create_a_function_token() {
        let cv = cascade_doc("", "div", Some("--fn: rgb; color: var(--fn)(255, 0, 0)"));
        assert_eq!(cv.color, CssColor::BLACK);
        assert_eq!(
            substitute_vars("var(--fn)(255, 0, 0)", &mut |_| Some("rgb".into()), 0),
            Some("rgb (255, 0, 0)".into())
        );
    }

    #[test]
    fn escaped_math_function_names_are_evaluated_after_token_decoding() {
        assert_eq!(simplify_math_functions(r"c\61 lc(1px)"), Some("1px".into()));
        let cv = cascade_doc("", "div", Some(r"width: c\61 lc(1px)"));
        assert_eq!(cv.width, ComputedLengthPercentageOrAuto::Px(1.0));
    }

    #[test]
    fn css_comments_are_whitespace_inside_math_functions() {
        assert_eq!(
            simplify_math_functions("calc(1px /* comment */ + 2px)"),
            Some("3px".into())
        );
        let cv = cascade_doc("", "div", Some("width: calc(1px /* comment */ + 2px)"));
        assert_eq!(cv.width, ComputedLengthPercentageOrAuto::Px(3.0));
    }

    #[test]
    fn variable_resolution_budget_rejects_excess_expansions() {
        let mut budget = VariableResolutionBudget::new(2);
        assert!(budget.consume());
        assert!(budget.consume());
        assert!(!budget.consume());

        let mut budget = VariableResolutionBudget::new(0);
        assert_eq!(
            substitute_vars_with_budget("var(--x)", &mut |_| Some("red".into()), 0, &mut budget),
            None
        );
    }

    #[test]
    fn has_css_whitespace_before_at_start_returns_false() {
        assert!(!MathParser::new("1px").has_css_whitespace_before(0));
    }

    #[test]
    fn compatible_absolute_length_units_are_normalized_in_math() {
        let cv = cascade_doc("", "div", Some("width: calc(1in + 96px)"));
        assert_eq!(cv.width, ComputedLengthPercentageOrAuto::Px(192.0));
        assert_eq!(
            evaluate_math_function("calc", "1in + 96px"),
            Some("192px".to_owned())
        );
        assert_eq!(
            evaluate_math_function("min", "1in, 96px"),
            Some("96px".to_owned())
        );
        assert_eq!(
            evaluate_math_function("max", "1in, 96px"),
            Some("96px".to_owned())
        );
        assert_eq!(
            evaluate_math_function("clamp", "1in, 96px, 2in"),
            Some("96px".to_owned())
        );
    }

    #[test]
    fn mixed_length_percentage_math_is_preserved_until_used_value_resolution() {
        // The computed layer preserves the percentage and px terms; the
        // containing-block basis is only available in the layout bridge.
        let cv = cascade_doc("", "div", Some("width: calc(10px + 5%)"));
        assert_eq!(
            cv.width,
            ComputedLengthPercentageOrAuto::Calc(crate::property::CalcLengthPercentage {
                percent: 5.0,
                px: 10.0,
            })
        );
    }

    #[test]
    fn oversized_variable_and_math_inputs_are_rejected() {
        let oversized = "x".repeat(MAX_SUBSTITUTED_VALUE_BYTES + 1);
        assert!(collect_var_references(&oversized, &mut Vec::new()).is_err());
        assert_eq!(split_top_level_commas(&oversized), None);
        assert_eq!(split_var_arguments(&oversized), None);
    }

    #[test]
    fn clamp_min_wins_when_bounds_are_reversed() {
        let cv = cascade_doc("", "div", Some("width: clamp(20px, 0px, 10px)"));
        assert_eq!(cv.width, ComputedLengthPercentageOrAuto::Px(20.0));
    }
}
