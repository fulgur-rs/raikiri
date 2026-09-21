use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use cssparser::{Parser, ParserInput, Token};
use smol_str::SmolStr;

use crate::computed::CustomPropertyEnvironment;
use crate::property::{
    CalcLengthPercentage, CustomProperty, DeferredValue, MAX_DEFERRED_VALUE_NESTING_DEPTH,
    MAX_SUBSTITUTED_VALUE_BYTES, PropertyValue, is_custom_property_name, parse_value,
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
    let simplified = match simplify_math_functions(&substituted) {
        Some(value) => value,
        None => {
            let value = parse_simple_calc_length_percentage(&substituted)?;
            return Some(PropertyValue::CalcLengthPercentage {
                key: deferred.key,
                value,
            });
        }
    };
    let mut input = ParserInput::new(simplified.as_ref());
    let mut parser = Parser::new(&mut input);
    let value = parse_value(&deferred.property, &mut parser)?;
    parser.expect_exhausted().ok()?;
    project_deferred_value(value, deferred.key)
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
