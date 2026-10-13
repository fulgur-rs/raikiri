use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use cssparser::{ParseError, Parser, ParserInput, Token};
use smol_str::SmolStr;

use crate::computed::{CustomPropertyEnvironment, empty_custom_properties};
use crate::property::{
    CssWideKeyword, CustomProperty, DeferredValue, FontFeatureSettings, FontVariationSettings,
    LengthOrAuto, MAX_DEFERRED_VALUE_NESTING_DEPTH, MAX_SUBSTITUTED_VALUE_BYTES, PropertyKey,
    PropertyValue, RelativeCalcLengthPercentage, RelativeCalcUnit, VerticalAlign,
    is_custom_property_name, parse_property_grammar, parse_value, product_operator,
    property_key_for_name, sum_operator,
};

use super::candidate::CustomCandidates;
use super::collect::{RankedDecl, beats};

/// Resolve a deferred declaration for either the element or page cascade.
///
/// `pub(crate)` is limited to the sibling page pipeline; no external API is
/// added. Both callers therefore share the same invalid-at-computed-value-time
/// and shorthand projection behavior.
pub(crate) fn resolve_deferred_value(
    deferred: &DeferredValue,
    custom_properties: &CustomPropertyEnvironment,
) -> Option<PropertyValue> {
    #[cfg(test)]
    tests::note_resolution(deferred);
    let substituted = substitute_vars(&deferred.value, &mut |name| custom_properties.get(name), 0)?;
    resolve_substituted_value(deferred, &substituted)?.project(deferred)
}

/// [`resolve_deferred_value`] for the page context, whose lengths are
/// resolved without a viewport: a value that uses a viewport-percentage
/// length after substitution is invalid at computed-value time, as the same
/// value written directly is dropped at parse time.
pub(crate) fn resolve_page_deferred_value(
    deferred: &DeferredValue,
    custom_properties: &CustomPropertyEnvironment,
) -> Option<PropertyValue> {
    let substituted = substitute_vars(&deferred.value, &mut |name| custom_properties.get(name), 0)?;
    if crate::property::has_viewport_length_in(&substituted) {
        return None;
    }
    resolve_substituted_value(deferred, &substituted)?.project(deferred)
}

// The lifetime fixes the custom-property environment for an entire winner drain,
// including any origin/layer rollback. Parsed shorthands never escape that drain.
pub(super) struct DeferredResolver<'a> {
    environment: &'a CustomPropertyEnvironment,
    shorthands: HashMap<(SmolStr, SmolStr), Option<ResolvedValue>>,
}

impl<'a> DeferredResolver<'a> {
    pub(super) fn new(environment: &'a CustomPropertyEnvironment) -> Self {
        Self {
            environment,
            shorthands: HashMap::new(),
        }
    }

    // Rollback inspects every candidate, including arbitrarily many losers.
    // Only actual winners and selected fallbacks may retain parsed payloads.
    pub(super) fn inspect(&self, deferred: &DeferredValue) -> Option<PropertyValue> {
        let source = (deferred.property.clone(), deferred.value.clone());
        if let Some(value) = self.shorthands.get(&source) {
            return value.as_ref()?.clone().project(deferred);
        }
        resolve_deferred_value(deferred, self.environment)
    }

    pub(super) fn resolve(&mut self, deferred: &DeferredValue) -> Option<PropertyValue> {
        if !property_key_for_name(&deferred.property).is_some_and(|key| {
            matches!(
                crate::rule::classify(key),
                crate::rule::KeyClass::Shorthand { .. }
            )
        }) {
            return resolve_deferred_value(deferred, self.environment);
        }
        let source = (deferred.property.clone(), deferred.value.clone());
        if let Some(value) = self.shorthands.get(&source) {
            return value.as_ref()?.clone().project(deferred);
        }
        #[cfg(test)]
        tests::note_resolution(deferred);
        let value = substitute_vars(&deferred.value, &mut |name| self.environment.get(name), 0)
            .and_then(|substituted| resolve_substituted_value(deferred, &substituted));
        // Missing variables are negative cache entries too.
        let projected = value
            .as_ref()
            .cloned()
            .and_then(|value| value.project(deferred));
        // Allocation failure falls back to uncached resolution, without changing
        // CSS validity or making the cache an additional cascade failure mode.
        if self.shorthands.try_reserve(1).is_ok() {
            #[cfg(test)]
            tests::note_cache_size(self.shorthands.len() + 1);
            self.shorthands.insert(source, value);
        }
        projected
    }
}

#[derive(Clone)]
enum ResolvedValue {
    Parsed(PropertyValue),
    Calc(RelativeCalcLengthPercentage),
    RadiusKeyword(CssWideKeyword),
}

impl ResolvedValue {
    fn project(self, deferred: &DeferredValue) -> Option<PropertyValue> {
        match self {
            Self::Parsed(value) => project_deferred_value(value, deferred.key),
            Self::Calc(value) => calc_length_percentage_value(deferred.key, value),
            Self::RadiusKeyword(keyword) => Some(PropertyValue::Deferred(DeferredValue {
                property: radius_corner_property(deferred.key)?.into(),
                value: keyword.as_css_str().into(),
                key: deferred.key,
            })),
        }
    }
}

fn resolve_substituted_value(deferred: &DeferredValue, substituted: &str) -> Option<ResolvedValue> {
    if deferred.key == PropertyKey::HyphenateLimitChars {
        // The integer components need to know whether a fractional number came
        // from a math function: calc results are rounded, direct fractional
        // tokens are invalid. Parse the substituted token stream before the
        // generic math simplifier erases that distinction.
        let mut input = ParserInput::new(substituted);
        let mut parser = Parser::new(&mut input);
        let value = parse_value(&deferred.property, &mut parser)?;
        parser.expect_exhausted().ok()?;
        return Some(ResolvedValue::Parsed(value));
    }
    if deferred.key == PropertyKey::TextUnderlineOffset {
        // Preserve the substituted calc's em and percentage terms for the
        // property-specific computed-value resolver.
        let mut input = ParserInput::new(substituted);
        let mut parser = Parser::new(&mut input);
        let value = parse_value(&deferred.property, &mut parser)?;
        parser.expect_exhausted().ok()?;
        return Some(ResolvedValue::Parsed(value));
    }
    if deferred.key == PropertyKey::TextShadow {
        // The text-shadow parser must see the substituted calc source: generic
        // simplification would erase mixed em/px terms, calculated-blur
        // provenance, or percentages that cancel to zero.
        let mut input = ParserInput::new(substituted);
        let mut parser = Parser::new(&mut input);
        let value = parse_value(&deferred.property, &mut parser)?;
        parser.expect_exhausted().ok()?;
        return Some(ResolvedValue::Parsed(value));
    }
    if let Some(simplified) = simplify_math_functions(substituted) {
        if deferred.property.as_str() == "border-radius"
            && let Some(keyword) = css_wide_keyword_source(&simplified)
            && radius_corner_property(deferred.key).is_some()
        {
            return Some(ResolvedValue::RadiusKeyword(keyword));
        }
        let mut input = ParserInput::new(simplified.as_ref());
        let mut parser = Parser::new(&mut input);
        if let Some(value) = parse_value(&deferred.property, &mut parser)
            && parser.expect_exhausted().is_ok()
        {
            return Some(ResolvedValue::Parsed(value));
        }
    }
    // The math functions are left as written when one cannot be folded to a
    // single dimension, such as `calc(10% + 1px)`, or when the folded value
    // is outside the property's range, such as `calc(1em - 1em - 10px)`
    // folding to `-10px`: a math function is clamped to the range rather than
    // invalid (CSS Values 4 "Range Checking",
    // <https://www.w3.org/TR/css-values-4/#calc-range>). Properties whose
    // grammar keeps a `calc()`, such as the margin and padding sides, read it
    // themselves.
    let mut input = ParserInput::new(substituted);
    let mut parser = Parser::new(&mut input);
    if let Some(value) =
        parse_property_grammar(&deferred.property.to_ascii_lowercase(), &mut parser)
        && parser.expect_exhausted().is_ok()
    {
        return Some(ResolvedValue::Parsed(value));
    }
    // A single sizing, inset, or `vertical-align` calc is kept for the
    // computed-value resolver, which clamps sizes.
    parse_relative_calc_length_percentage(substituted).map(ResolvedValue::Calc)
}

/// The CSS-wide keyword `source` holds as its only component value.
fn css_wide_keyword_source(source: &str) -> Option<CssWideKeyword> {
    let mut input = ParserInput::new(source);
    Parser::new(&mut input)
        .parse_entirely(|parser| {
            let ident = parser.expect_ident()?.clone();
            CssWideKeyword::from_css_ident(&ident)
                .ok_or_else(|| parser.new_custom_error::<_, ()>(()))
        })
        .ok()
}

/// Wrap a mixed-unit `calc()` in the value of the property it was declared
/// for. Sizing, inset, and `vertical-align` values can carry one; for any
/// other property the declaration is invalid at computed-value time.
/// Font-relative and viewport terms are kept for computed-value time
/// ([`LengthOrAuto::RelativeCalc`]); `vertical-align` has no such
/// representation, so a calc with those terms is invalid there.
fn calc_length_percentage_value(
    key: PropertyKey,
    value: RelativeCalcLengthPercentage,
) -> Option<PropertyValue> {
    let absolute = value.as_absolute();
    let calc = match absolute {
        Some(absolute) => LengthOrAuto::Calc(absolute),
        None => LengthOrAuto::RelativeCalc(value),
    };
    Some(match key {
        PropertyKey::Width => PropertyValue::Width(calc),
        PropertyKey::Height => PropertyValue::Height(calc),
        PropertyKey::MaxWidth => PropertyValue::MaxWidth(calc),
        PropertyKey::MaxHeight => PropertyValue::MaxHeight(calc),
        PropertyKey::MinWidth => PropertyValue::MinWidth(calc),
        PropertyKey::MinHeight => PropertyValue::MinHeight(calc),
        PropertyKey::MinBlockSize => PropertyValue::MinBlockSize(calc),
        PropertyKey::InlineSize => PropertyValue::InlineSize(calc),
        PropertyKey::BlockSize => PropertyValue::BlockSize(calc),
        PropertyKey::Top => PropertyValue::Top(calc),
        PropertyKey::Right => PropertyValue::Right(calc),
        PropertyKey::Bottom => PropertyValue::Bottom(calc),
        PropertyKey::Left => PropertyValue::Left(calc),
        PropertyKey::VerticalAlign => PropertyValue::VerticalAlign(VerticalAlign::Calc(absolute?)),
        _ => return None,
    })
}

/// Parse a `calc()` whose terms mix units that cannot be combined before
/// computed-value time (percentages, font-relative and viewport lengths) into
/// one coefficient per basis.
///
/// The body is a linear sum: `+` / `-` between terms, and `*` / `/` only with
/// a plain number on one side (CSS Values 4 "Type Checking",
/// <https://www.w3.org/TR/css-values-4/#calc-type-checking>). Nested
/// parentheses and nested `calc()` are accepted; other math functions are not.
pub(crate) fn parse_relative_calc_length_percentage(
    input: &str,
) -> Option<RelativeCalcLengthPercentage> {
    let mut parser_input = ParserInput::new(input);
    let mut parser = Parser::new(&mut parser_input);
    // A `calc(` left open at the end of input is closed there (CSS Syntax 3
    // §5.4.9); the tokenizer closes it the same way.
    parser
        .parse_entirely(|parser| {
            parse_relative_calc_function(parser).ok_or_else(|| parser.new_custom_error::<_, ()>(()))
        })
        .ok()
}

/// Consume one `calc()` function from `input` and read it like
/// [`parse_relative_calc_length_percentage`]. The parser is left where it
/// was when the next component value is not such a `calc()`.
pub(crate) fn parse_relative_calc_function(
    input: &mut Parser<'_, '_>,
) -> Option<RelativeCalcLengthPercentage> {
    input
        .try_parse(|parser| {
            if !parser.expect_function()?.eq_ignore_ascii_case("calc") {
                return Err(parser.new_custom_error(()));
            }
            match parser.parse_nested_block(|nested| evaluate_linear_sum(nested, 1))? {
                LinearValue::Length(terms) => terms
                    .into_value()
                    .ok_or_else(|| parser.new_custom_error(())),
                LinearValue::Number(_) => Err(parser.new_custom_error(())),
            }
        })
        .ok()
}

/// A sub-expression of a linear `calc()`: a plain number, or a sum of
/// length-percentage terms.
#[derive(Clone, Copy)]
enum LinearValue {
    Number(f32),
    Length(LinearTerms),
}

/// Length-percentage terms of a linear `calc()` with one coefficient per
/// relative basis. Unlike [`RelativeCalcLengthPercentage`] it has room for
/// every basis, so terms that cancel later in the expression do not count
/// against that type's limit.
#[derive(Clone, Copy, Default)]
struct LinearTerms {
    percent: f32,
    px: f32,
    relative: [f32; RelativeCalcUnit::ALL.len()],
}

impl LinearTerms {
    fn from_unit(value: f32, unit: &str) -> Option<Self> {
        let single = RelativeCalcLengthPercentage::from_unit(value, unit)?;
        let mut terms = Self {
            percent: single.percent,
            px: single.px,
            ..Self::default()
        };
        for (unit, coefficient) in single.relative_terms() {
            terms.relative[unit as usize] = coefficient;
        }
        Some(terms)
    }

    fn coefficients_mut(&mut self) -> impl Iterator<Item = &mut f32> {
        [&mut self.percent, &mut self.px]
            .into_iter()
            .chain(self.relative.iter_mut())
    }

    /// Add `sign * other` to every coefficient.
    fn add_scaled(mut self, mut other: Self, sign: f32) -> Option<Self> {
        for (left, right) in self.coefficients_mut().zip(other.coefficients_mut()) {
            *left += sign * *right;
            if !left.is_finite() {
                return None;
            }
        }
        Some(self)
    }

    /// Multiply every coefficient by `factor`.
    fn scale(mut self, factor: f32) -> Option<Self> {
        for coefficient in self.coefficients_mut() {
            *coefficient *= factor;
            if !coefficient.is_finite() {
                return None;
            }
        }
        Some(self)
    }

    /// The compact value, or `None` when more relative bases remain than it
    /// can hold.
    fn into_value(self) -> Option<RelativeCalcLengthPercentage> {
        let mut value = RelativeCalcLengthPercentage::default();
        value.percent = self.percent;
        value.px = self.px;
        for (unit, coefficient) in RelativeCalcUnit::ALL.into_iter().zip(self.relative) {
            if coefficient != 0.0 {
                value = value.add_scaled(
                    RelativeCalcLengthPercentage::relative(unit, coefficient)?,
                    1.0,
                )?;
            }
        }
        Some(value)
    }
}

fn radius_corner_property(key: PropertyKey) -> Option<&'static str> {
    Some(match key {
        PropertyKey::BorderRadiusTopLeft => "border-top-left-radius",
        PropertyKey::BorderRadiusTopRight => "border-top-right-radius",
        PropertyKey::BorderRadiusBottomRight => "border-bottom-right-radius",
        PropertyKey::BorderRadiusBottomLeft => "border-bottom-left-radius",
        _ => return None,
    })
}

/// The property name of a border-image longhand key.
fn border_image_longhand_name(key: PropertyKey) -> Option<&'static str> {
    Some(match key {
        PropertyKey::BorderImageSource => "border-image-source",
        PropertyKey::BorderImageSlice => "border-image-slice",
        PropertyKey::BorderImageWidth => "border-image-width",
        PropertyKey::BorderImageOutset => "border-image-outset",
        PropertyKey::BorderImageRepeat => "border-image-repeat",
        _ => return None,
    })
}

/// The `key` longhand of a `border-image` shorthand value.
fn border_image_longhand(
    key: PropertyKey,
    shorthand: &crate::property::BorderImageShorthand,
) -> Option<PropertyValue> {
    Some(match key {
        PropertyKey::BorderImageSource => {
            PropertyValue::BorderImageSource(shorthand.source.clone())
        }
        PropertyKey::BorderImageSlice => PropertyValue::BorderImageSlice(shorthand.slice),
        PropertyKey::BorderImageWidth => PropertyValue::BorderImageWidth(shorthand.width),
        PropertyKey::BorderImageOutset => PropertyValue::BorderImageOutset(shorthand.outset),
        PropertyKey::BorderImageRepeat => PropertyValue::BorderImageRepeat(shorthand.repeat),
        _ => return None,
    })
}

pub(crate) fn project_deferred_value(
    value: PropertyValue,
    key: crate::property::PropertyKey,
) -> Option<PropertyValue> {
    if let PropertyValue::Deferred(marker) = &value
        && marker.property.as_str() == "list-style"
        && crate::property::CssWideKeyword::from_css_ident(marker.value.as_str()).is_some()
        && matches!(
            key,
            PropertyKey::ListStyleType
                | PropertyKey::ListStylePosition
                | PropertyKey::ListStyleImage
        )
    {
        let mut marker = marker.clone();
        marker.key = key;
        marker.property = match key {
            PropertyKey::ListStyleType => "list-style-type",
            PropertyKey::ListStylePosition => "list-style-position",
            _ => "list-style-image",
        }
        .into();
        return Some(PropertyValue::Deferred(marker));
    }
    if let PropertyValue::Deferred(marker) = &value
        && marker.property.as_str() == "column-rule"
        && marker.css_wide_keyword().is_some()
        && matches!(
            key,
            PropertyKey::ColumnRuleWidth
                | PropertyKey::ColumnRuleStyle
                | PropertyKey::ColumnRuleColor
        )
    {
        let mut marker = marker.clone();
        marker.key = key;
        marker.property = match key {
            PropertyKey::ColumnRuleWidth => "column-rule-width",
            PropertyKey::ColumnRuleStyle => "column-rule-style",
            _ => "column-rule-color",
        }
        .into();
        return Some(PropertyValue::Deferred(marker));
    }
    if let PropertyValue::Deferred(marker) = &value
        && marker.property.as_str() == "border-image"
        && marker.css_wide_keyword().is_some()
        && let Some(property) = border_image_longhand_name(key)
    {
        let mut marker = marker.clone();
        marker.key = key;
        marker.property = property.into();
        return Some(PropertyValue::Deferred(marker));
    }
    if value.key() == key {
        return Some(value);
    }
    Some(match value {
        PropertyValue::BorderRadius(radius) => match key {
            PropertyKey::BorderRadiusTopLeft => PropertyValue::BorderRadiusTopLeft(radius.top_left),
            PropertyKey::BorderRadiusTopRight => {
                PropertyValue::BorderRadiusTopRight(radius.top_right)
            }
            PropertyKey::BorderRadiusBottomRight => {
                PropertyValue::BorderRadiusBottomRight(radius.bottom_right)
            }
            PropertyKey::BorderRadiusBottomLeft => {
                PropertyValue::BorderRadiusBottomLeft(radius.bottom_left)
            }
            _ => return None,
        },
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
        // `crate::rule::classify` pairs each shorthand key with
        // (`crate::property::PropertyValue::MarginInline` doc's
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
            // `border` also resets every border-image longhand (CSS
            // Backgrounds 3 §3.4).
            key => border_image_longhand(key, &crate::property::BorderImageShorthand::initial())?,
        },
        PropertyValue::BorderImage(shorthand) => border_image_longhand(key, &shorthand)?,
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
        PropertyValue::BorderTop(border) => match key {
            crate::property::PropertyKey::BorderTopWidth => {
                PropertyValue::BorderTopWidth(border.width)
            }
            crate::property::PropertyKey::BorderTopStyle => {
                PropertyValue::BorderTopStyle(border.style)
            }
            crate::property::PropertyKey::BorderTopColor => {
                PropertyValue::BorderTopColor(border.color)
            }
            _ => return None,
        },
        // cov:ignore: shorthand projection is defensive; normal rule expansion covers this path.
        PropertyValue::BorderRight(border) => match key {
            crate::property::PropertyKey::BorderRightWidth => {
                PropertyValue::BorderRightWidth(border.width)
            }
            crate::property::PropertyKey::BorderRightStyle => {
                PropertyValue::BorderRightStyle(border.style)
            }
            crate::property::PropertyKey::BorderRightColor => {
                PropertyValue::BorderRightColor(border.color)
            }
            _ => return None,
        },
        // cov:ignore: shorthand projection is defensive; normal rule expansion covers this path.
        PropertyValue::BorderBottom(border) => match key {
            crate::property::PropertyKey::BorderBottomWidth => {
                PropertyValue::BorderBottomWidth(border.width)
            }
            crate::property::PropertyKey::BorderBottomStyle => {
                PropertyValue::BorderBottomStyle(border.style)
            }
            crate::property::PropertyKey::BorderBottomColor => {
                PropertyValue::BorderBottomColor(border.color)
            }
            _ => return None,
        },
        // cov:ignore: shorthand projection is defensive; normal rule expansion covers this path.
        PropertyValue::BorderLeft(border) => match key {
            crate::property::PropertyKey::BorderLeftWidth => {
                PropertyValue::BorderLeftWidth(border.width)
            }
            crate::property::PropertyKey::BorderLeftStyle => {
                PropertyValue::BorderLeftStyle(border.style)
            }
            crate::property::PropertyKey::BorderLeftColor => {
                PropertyValue::BorderLeftColor(border.color)
            }
            _ => return None,
        },
        // cov:ignore: shorthand projection is defensive; normal rule expansion covers this path.
        PropertyValue::BorderCssWide(kw) => match key {
            crate::property::PropertyKey::BorderTopWidth => {
                PropertyValue::BorderTopWidthCssWide(kw)
            }
            crate::property::PropertyKey::BorderTopStyle => {
                PropertyValue::BorderTopStyleCssWide(kw)
            }
            crate::property::PropertyKey::BorderTopColor => {
                PropertyValue::BorderTopColorCssWide(kw)
            }
            crate::property::PropertyKey::BorderRightWidth => {
                PropertyValue::BorderRightWidthCssWide(kw)
            }
            crate::property::PropertyKey::BorderRightStyle => {
                PropertyValue::BorderRightStyleCssWide(kw)
            }
            crate::property::PropertyKey::BorderRightColor => {
                PropertyValue::BorderRightColorCssWide(kw)
            }
            crate::property::PropertyKey::BorderBottomWidth => {
                PropertyValue::BorderBottomWidthCssWide(kw)
            }
            crate::property::PropertyKey::BorderBottomStyle => {
                PropertyValue::BorderBottomStyleCssWide(kw)
            }
            crate::property::PropertyKey::BorderBottomColor => {
                PropertyValue::BorderBottomColorCssWide(kw)
            }
            crate::property::PropertyKey::BorderLeftWidth => {
                PropertyValue::BorderLeftWidthCssWide(kw)
            }
            crate::property::PropertyKey::BorderLeftStyle => {
                PropertyValue::BorderLeftStyleCssWide(kw)
            }
            crate::property::PropertyKey::BorderLeftColor => {
                PropertyValue::BorderLeftColorCssWide(kw)
            }
            // The border shorthand also sets the reset-only border-image
            // longhands, which take the keyword as a deferred marker.
            crate::property::PropertyKey::BorderImageSource
            | crate::property::PropertyKey::BorderImageSlice
            | crate::property::PropertyKey::BorderImageWidth
            | crate::property::PropertyKey::BorderImageOutset
            | crate::property::PropertyKey::BorderImageRepeat => {
                let property = match key {
                    crate::property::PropertyKey::BorderImageSource => "border-image-source",
                    crate::property::PropertyKey::BorderImageSlice => "border-image-slice",
                    crate::property::PropertyKey::BorderImageWidth => "border-image-width",
                    crate::property::PropertyKey::BorderImageOutset => "border-image-outset",
                    _ => "border-image-repeat",
                };
                PropertyValue::Deferred(DeferredValue {
                    key,
                    property: property.into(),
                    value: kw.as_css_str().into(),
                })
            }
            _ => return None,
        },
        // cov:ignore: shorthand projection is defensive; normal rule expansion covers this path.
        PropertyValue::BorderTopCssWide(kw) => match key {
            crate::property::PropertyKey::BorderTopWidth => {
                PropertyValue::BorderTopWidthCssWide(kw)
            }
            crate::property::PropertyKey::BorderTopStyle => {
                PropertyValue::BorderTopStyleCssWide(kw)
            }
            crate::property::PropertyKey::BorderTopColor => {
                PropertyValue::BorderTopColorCssWide(kw)
            }
            _ => return None,
        },
        // cov:ignore: shorthand projection is defensive; normal rule expansion covers this path.
        PropertyValue::BorderRightCssWide(kw) => match key {
            crate::property::PropertyKey::BorderRightWidth => {
                PropertyValue::BorderRightWidthCssWide(kw)
            }
            crate::property::PropertyKey::BorderRightStyle => {
                PropertyValue::BorderRightStyleCssWide(kw)
            }
            crate::property::PropertyKey::BorderRightColor => {
                PropertyValue::BorderRightColorCssWide(kw)
            }
            _ => return None,
        },
        // cov:ignore: shorthand projection is defensive; normal rule expansion covers this path.
        PropertyValue::BorderBottomCssWide(kw) => match key {
            crate::property::PropertyKey::BorderBottomWidth => {
                PropertyValue::BorderBottomWidthCssWide(kw)
            }
            crate::property::PropertyKey::BorderBottomStyle => {
                PropertyValue::BorderBottomStyleCssWide(kw)
            }
            crate::property::PropertyKey::BorderBottomColor => {
                PropertyValue::BorderBottomColorCssWide(kw)
            }
            _ => return None,
        },
        // cov:ignore: shorthand projection is defensive; normal rule expansion covers this path.
        PropertyValue::BorderLeftCssWide(kw) => match key {
            crate::property::PropertyKey::BorderLeftWidth => {
                PropertyValue::BorderLeftWidthCssWide(kw)
            }
            crate::property::PropertyKey::BorderLeftStyle => {
                PropertyValue::BorderLeftStyleCssWide(kw)
            }
            crate::property::PropertyKey::BorderLeftColor => {
                PropertyValue::BorderLeftColorCssWide(kw)
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
        PropertyValue::ListStyle(value) => match key {
            PropertyKey::ListStyleType => PropertyValue::ListStyleType(value.kind),
            PropertyKey::ListStylePosition => PropertyValue::ListStylePosition(value.position),
            PropertyKey::ListStyleImage => PropertyValue::ListStyleImage(value.image),
            _ => return None,
        },
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
        PropertyValue::ColumnRule(rule) => match key {
            PropertyKey::ColumnRuleWidth => PropertyValue::ColumnRuleWidth(rule.width),
            PropertyKey::ColumnRuleStyle => PropertyValue::ColumnRuleStyle(rule.style),
            PropertyKey::ColumnRuleColor => PropertyValue::ColumnRuleColor(rule.color),
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
                crate::property::FontShorthandSize::Keyword(keyword) => {
                    PropertyValue::FontSizeKeyword(keyword)
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
            crate::property::PropertyKey::FontFeatureSettings => {
                PropertyValue::FontFeatureSettings(FontFeatureSettings::Normal)
            }
            _ => return None,
        },
        PropertyValue::Background(shorthand) => match key {
            crate::property::PropertyKey::BackgroundColor => match shorthand.color_expression {
                Some(source) => {
                    PropertyValue::ContextualColor(crate::property::ContextualColor { source, key })
                }
                None => PropertyValue::BackgroundColor(shorthand.color),
            },
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

pub(crate) const MATH_FUNCTIONS: [&str; 4] = ["calc", "min", "max", "clamp"];

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
        // `next` skips whitespace and comments before the token it returns.
        // Skip them first, so `token_start` is where the token itself starts:
        // callers keep `input[..token_start]` verbatim when they replace the
        // function, and that text must keep the separator in front of it.
        parser.skip_whitespace();
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
                let mut body_end = open;
                let mut nested_found = parser
                    .parse_nested_block(|nested| {
                        let found =
                            find_function_tokens_in_parser(nested, names, depth.saturating_add(1));
                        body_end = nested.position().byte_index();
                        found.ok_or_else(|| nested.new_custom_error::<(), ()>(()))
                    })
                    .ok()?;
                if is_target {
                    // CSS Syntax 3 "consume a function"
                    // (https://www.w3.org/TR/css-syntax-3/#consume-function):
                    // reaching the end of input inside a function is a parse
                    // error that still returns the function. When no `)` was
                    // consumed after the body, the function is closed by the
                    // end of input and `close` is the position just past the
                    // last byte, where its `)` would be.
                    let end = parser.position().byte_index();
                    let close = if end > body_end { end - 1 } else { end };
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

/// Whether replacement text ending `output` needs a separator before `rest`,
/// the source text that follows the replaced function. Whitespace or a comment
/// at the start of `rest` is copied verbatim, so it already keeps the tokens
/// apart; otherwise this is [`needs_css_token_separator`].
fn needs_separator_after_replacement(output: &str, rest: &str) -> bool {
    !rest.starts_with(|c: char| c.is_ascii_whitespace())
        && !rest.starts_with("/*")
        && needs_css_token_separator(output, rest)
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
        let inner_source = input.get(open + 1..close)?;
        let inner = simplify_math_functions_at_depth(inner_source, depth.saturating_add(1))?;
        let evaluated = evaluate_math_function(name.as_str(), &inner)?;
        push_bounded(&mut output, &evaluated)?;
        // A function closed by the end of input has `close == input.len()`,
        // so the text after it is empty rather than out of range.
        if needs_separator_after_replacement(&output, input.get(close + 1..).unwrap_or("")) {
            push_bounded(&mut output, " ")?;
        }
        position = close.checked_add(1)?;
    }
    push_bounded(&mut output, input.get(position..).unwrap_or(""))?;
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
            let value = evaluate_math_arguments(input)?;
            let [value] = <[MathValue; 1]>::try_from(value).ok()?;
            serialize_math_value(value)
        }
        "min" | "max" => {
            let mut values = evaluate_math_arguments(input)?;
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
            let mut values = evaluate_math_arguments(input)?;
            if values.len() != 3 {
                return None;
            }
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

type EvalResult<'i, T> = Result<T, ParseError<'i, ()>>;

/// Evaluate the comma-separated arguments of a math function whose nested
/// math functions were already replaced by their values.
fn evaluate_math_arguments(input: &str) -> Option<Vec<MathValue>> {
    let mut parser_input = ParserInput::new(input);
    let mut parser = Parser::new(&mut parser_input);
    parser
        .parse_entirely(|parser| parser.parse_comma_separated(|argument| evaluate_sum(argument, 0)))
        .ok()
}

fn evaluate_sum<'i>(input: &mut Parser<'i, '_>, depth: usize) -> EvalResult<'i, MathValue> {
    let mut value = evaluate_product(input, depth)?;
    while let Ok(operator) = input.try_parse(sum_operator) {
        let mut right = evaluate_product(input, depth)?;
        normalize_math_pair(&mut value, &mut right).ok_or_else(|| input.new_custom_error(()))?;
        value.number = if operator == '+' {
            value.number + right.number
        } else {
            value.number - right.number
        };
        if !value.number.is_finite() {
            return Err(input.new_custom_error(()));
        }
    }
    Ok(value)
}

fn evaluate_product<'i>(input: &mut Parser<'i, '_>, depth: usize) -> EvalResult<'i, MathValue> {
    let mut value = evaluate_primary(input, depth)?;
    while let Ok(operator) = input.try_parse(product_operator) {
        let right = evaluate_primary(input, depth)?;
        match operator {
            '*' if value.unit.is_none() => {
                value = MathValue {
                    number: value.number * right.number,
                    unit: right.unit,
                }
            }
            '*' if right.unit.is_none() => value.number *= right.number,
            '/' if right.unit.is_none() && right.number != 0.0 => value.number /= right.number,
            _ => return Err(input.new_custom_error(())),
        }
        if !value.number.is_finite() {
            return Err(input.new_custom_error(()));
        }
    }
    Ok(value)
}

fn evaluate_primary<'i>(input: &mut Parser<'i, '_>, depth: usize) -> EvalResult<'i, MathValue> {
    if depth >= MAX_VARIABLE_RESOLUTION_DEPTH {
        return Err(input.new_custom_error(()));
    }
    input.skip_whitespace();
    let start = input.position();
    let value = match input.next()?.clone() {
        Token::Number { value, .. } => MathValue {
            number: value,
            unit: None,
        },
        // The token keeps only the fraction; read the source digits so that
        // `33.3%` stays exactly `33.3`.
        Token::Percentage { .. } => MathValue {
            number: input
                .slice_from(start)
                .strip_suffix('%')
                .and_then(|number| number.parse::<f32>().ok())
                .ok_or_else(|| input.new_custom_error(()))?,
            unit: Some("%".to_owned()),
        },
        Token::Dimension { value, unit, .. } => MathValue {
            number: value,
            unit: Some(unit.to_ascii_lowercase()),
        },
        Token::ParenthesisBlock => {
            return input.parse_nested_block(|nested| evaluate_sum(nested, depth + 1));
        }
        _ => return Err(input.new_custom_error(())),
    };
    if !value.number.is_finite() {
        return Err(input.new_custom_error(()));
    }
    Ok(value)
}

fn evaluate_linear_sum<'i>(
    input: &mut Parser<'i, '_>,
    depth: usize,
) -> EvalResult<'i, LinearValue> {
    let mut value = evaluate_linear_product(input, depth)?;
    while let Ok(operator) = input.try_parse(sum_operator) {
        let right = evaluate_linear_product(input, depth)?;
        let sign = if operator == '+' { 1.0 } else { -1.0 };
        value = match (value, right) {
            (LinearValue::Number(left), LinearValue::Number(right)) => Some(left + sign * right)
                .filter(|sum| sum.is_finite())
                .map(LinearValue::Number),
            (LinearValue::Length(left), LinearValue::Length(right)) => {
                left.add_scaled(right, sign).map(LinearValue::Length)
            }
            _ => None,
        }
        .ok_or_else(|| input.new_custom_error(()))?;
    }
    Ok(value)
}

fn evaluate_linear_product<'i>(
    input: &mut Parser<'i, '_>,
    depth: usize,
) -> EvalResult<'i, LinearValue> {
    let mut value = evaluate_linear_primary(input, depth)?;
    while let Ok(operator) = input.try_parse(product_operator) {
        let right = evaluate_linear_primary(input, depth)?;
        let finite = |number: f32| number.is_finite().then_some(number);
        value = match (operator, value, right) {
            ('*', LinearValue::Number(left), LinearValue::Number(right)) => {
                finite(left * right).map(LinearValue::Number)
            }
            ('*', LinearValue::Number(factor), LinearValue::Length(length))
            | ('*', LinearValue::Length(length), LinearValue::Number(factor)) => {
                length.scale(factor).map(LinearValue::Length)
            }
            ('/', LinearValue::Number(left), LinearValue::Number(right)) if right != 0.0 => {
                finite(left / right).map(LinearValue::Number)
            }
            ('/', LinearValue::Length(length), LinearValue::Number(divisor)) if divisor != 0.0 => {
                length.scale(divisor.recip()).map(LinearValue::Length)
            }
            _ => None,
        }
        .ok_or_else(|| input.new_custom_error(()))?;
    }
    Ok(value)
}

fn evaluate_linear_primary<'i>(
    input: &mut Parser<'i, '_>,
    depth: usize,
) -> EvalResult<'i, LinearValue> {
    if depth >= MAX_VARIABLE_RESOLUTION_DEPTH {
        return Err(input.new_custom_error(()));
    }
    let start = input.state();
    match input.next()?.clone() {
        Token::ParenthesisBlock => {
            return input.parse_nested_block(|nested| evaluate_linear_sum(nested, depth + 1));
        }
        Token::Function(name) if name.eq_ignore_ascii_case("calc") => {
            return input.parse_nested_block(|nested| evaluate_linear_sum(nested, depth + 1));
        }
        _ => input.reset(&start),
    }
    let MathValue { number, unit } = evaluate_primary(input, depth)?;
    match unit {
        None => Ok(LinearValue::Number(number)),
        Some(unit) => LinearTerms::from_unit(number, &unit)
            .map(LinearValue::Length)
            .ok_or_else(|| input.new_custom_error(())),
    }
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

/// Interpret literal rollback keywords before variable substitution.
pub(crate) fn custom_property_rollback(value: &str) -> super::rollback::Rollback {
    let mut input = ParserInput::new(value);
    let mut parser = Parser::new(&mut input);
    let Ok(ident) = parser.expect_ident_cloned() else {
        return super::rollback::Rollback::None;
    };
    if !parser.is_exhausted() {
        return super::rollback::Rollback::None;
    }
    if ident.eq_ignore_ascii_case("revert-layer") {
        super::rollback::Rollback::Layer
    } else if ident.eq_ignore_ascii_case("revert") {
        super::rollback::Rollback::Origin
    } else {
        super::rollback::Rollback::None
    }
}

/// Select custom-property rollback winners independently for each name.
pub(crate) fn select_custom_rollback_values<'a, P: Copy + Ord>(
    candidates: impl Iterator<
        Item = (
            &'a CustomProperty,
            P,
            crate::ruletree::Origin,
            crate::layer::LayerPosition,
            bool,
        ),
    >,
) -> HashMap<SmolStr, SmolStr> {
    let mut groups: HashMap<&SmolStr, Vec<_>> = HashMap::new();
    for candidate in candidates {
        groups.entry(&candidate.0.name).or_default().push(candidate);
    }
    groups
        .into_iter()
        .filter_map(|(name, candidates)| {
            let index = super::rollback::select_layered_winner(
                &candidates,
                |_, (value, priority, origin, layer, important)| {
                    Some((
                        *priority,
                        *origin,
                        *layer,
                        *important,
                        custom_property_rollback(&value.value),
                    ))
                },
            )?;
            Some((name.clone(), candidates[index].0.value.clone()))
        })
        .collect()
}

pub(crate) struct ResolvedCustomProperties {
    pub(crate) effective: Arc<CustomPropertyEnvironment>,
    pub(crate) local: Arc<CustomPropertyEnvironment>,
}

pub(crate) fn resolve_custom_properties(
    inherited: &Arc<CustomPropertyEnvironment>,
    candidates: CustomCandidates<'_>,
) -> ResolvedCustomProperties {
    if candidates.is_empty() {
        return ResolvedCustomProperties {
            effective: inherited.clone(),
            local: empty_custom_properties(),
        };
    }

    // The winners borrow their declarations; only the final local values are
    // cloned.
    let mut winners: HashMap<&SmolStr, (&CustomProperty, RankedDecl)> = HashMap::new();
    for (idx, (value, precedence)) in candidates.iter().enumerate() {
        let candidate = precedence.ranked(idx);
        let replace = winners
            .get(&value.name)
            .is_none_or(|(_, existing)| beats(candidate, *existing));
        if replace {
            winners.insert(&value.name, (value, candidate));
        }
    }

    let local = if winners
        .values()
        .any(|(value, _)| custom_property_rollback(&value.value) != super::rollback::Rollback::None)
    {
        select_custom_rollback_values(candidates.iter().enumerate().map(
            |(index, (value, precedence))| {
                let (priority, origin, layer, important, _) =
                    precedence.layered(index, super::rollback::Rollback::None);
                (value, priority, origin, layer, important)
            },
        ))
    } else {
        winners
            .into_iter()
            .map(|(name, (value, _))| (name.clone(), value.value.clone()))
            .collect()
    };
    resolve_custom_property_environment(inherited, &local)
}

/// Resolve one element or page context's local custom properties against a
/// shared inherited environment.
///
/// The returned pair holds the effective environment and this node's local
/// declarations separately. A `None` entry records a declared-but-invalid
/// custom property, which shadows an inherited value while allowing `var()`
/// fallback.
pub(crate) fn resolve_custom_property_environment(
    inherited: &Arc<CustomPropertyEnvironment>,
    local: &HashMap<SmolStr, SmolStr>,
) -> ResolvedCustomProperties {
    if local.is_empty() {
        return ResolvedCustomProperties {
            effective: inherited.clone(),
            local: empty_custom_properties(),
        };
    }
    let entries = resolve_local_custom_properties(inherited.as_ref(), local);
    let matches_inherited_layer = entries
        .iter()
        .all(|(name, value)| inherited.local_binding_matches(name, value));
    if matches_inherited_layer {
        return ResolvedCustomProperties {
            effective: inherited.clone(),
            local: CustomPropertyEnvironment::from_local_only(entries),
        }; // cov:ignore: expression-closing brace has no mapping; true fields are covered
    } // cov:ignore: branch-closing brace has no mapping; false path continues below
    let effective = CustomPropertyEnvironment::from_local(inherited, entries);
    ResolvedCustomProperties {
        effective: Arc::clone(&effective),
        local: effective,
    }
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
    if input.len() > MAX_SUBSTITUTED_VALUE_BYTES {
        return Err(());
    }
    let functions = find_function_tokens(input, &["var"]).ok_or(())?;
    for (_, _, open, close) in functions {
        let (name, _) = split_var_arguments(input.get(open + 1..close).ok_or(())?).ok_or(())?;
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
    if depth > MAX_VARIABLE_RESOLUTION_DEPTH || input.len() > MAX_SUBSTITUTED_VALUE_BYTES {
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
        // `close == input.len()` for a function closed by the end of input.
        let inner = input.get(open + 1..close)?;
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
        if ends_with_open_hex_escape(&replacement) {
            // A hex escape consumes one whitespace character after it (CSS
            // Syntax 3 "consume an escaped code point"). Terminate the escape
            // here, so that it does not absorb the whitespace token that
            // follows the `var()` in the source.
            push_bounded(&mut output, " ")?;
        }
        if needs_separator_after_replacement(&output, input.get(close + 1..).unwrap_or("")) {
            // Substitution preserves the original component-value token
            // boundaries. Add a separator only where reparsing the bounded
            // serialization would otherwise merge adjacent tokens.
            push_bounded(&mut output, " ")?;
        }
        position = close.checked_add(1)?;
    }
    push_bounded(&mut output, input.get(position..).unwrap_or(""))?;
    Some(output.into())
}

/// Whether `text` ends with a hex escape (a backslash and one to six hex digits)
/// that would consume a whitespace character appended after it.
fn ends_with_open_hex_escape(text: &str) -> bool {
    let bytes = text.as_bytes();
    let digits = bytes
        .iter()
        .rev()
        .take_while(|byte| byte.is_ascii_hexdigit())
        .count();
    if digits == 0 || digits > 6 {
        return false;
    }
    let before = &bytes[..bytes.len() - digits];
    // The backslash starts an escape only when it is not itself escaped.
    let backslashes = before
        .iter()
        .rev()
        .take_while(|byte| **byte == b'\\')
        .count();
    backslashes % 2 == 1
}

/// `input` without its leading and trailing CSS whitespace and comments.
///
/// Comments produce no tokens (CSS Syntax 3 §4.3.2), so a declaration's
/// original text runs from its first token to its last one (§5.4.6). Only
/// the CSS whitespace code points are trimmed: other Unicode spaces such as
/// U+00A0 and U+3000 are ident code points. A comment left open at the end of
/// input runs to the end, and strings and escapes are kept whole.
pub(crate) fn trim_css_whitespace_and_comments(input: &str) -> &str {
    let mut parser_input = ParserInput::new(input);
    let mut parser = Parser::new(&mut parser_input);
    let mut text = None::<(usize, usize)>;
    loop {
        parser.skip_whitespace();
        let start = parser.position().byte_index();
        let Ok(token) = parser.next_including_whitespace_and_comments() else {
            break;
        };
        if matches!(
            token,
            Token::Function(_)
                | Token::ParenthesisBlock
                | Token::SquareBracketBlock
                | Token::CurlyBracketBlock
        ) {
            let _ = parser.parse_nested_block(|nested| {
                while nested.next_including_whitespace_and_comments().is_ok() {}
                Ok::<_, ParseError<'_, ()>>(())
            });
        }
        let end = parser.position().byte_index();
        text = Some((text.map_or(start, |(start, _)| start), end));
    }
    text.map_or("", |(start, end)| &input[start..end])
}

/// The custom property name of a `var()` and its fallback, if any: the
/// argument text after the first top-level comma, without the whitespace
/// around it (CSS Variables 1 §3).
pub(crate) fn split_var_arguments(input: &str) -> Option<(SmolStr, Option<&str>)> {
    if input.len() > MAX_SUBSTITUTED_VALUE_BYTES {
        return None;
    }
    let mut parser_input = ParserInput::new(input);
    let mut parser = Parser::new(&mut parser_input);
    let name = parser.expect_ident_cloned().ok()?;
    if !is_custom_property_name(name.as_ref()) {
        return None;
    }
    if parser.is_exhausted() {
        return Some((name.as_ref().into(), None));
    }
    parser.expect_comma().ok()?;
    let fallback_start = parser.position().byte_index();
    while let Ok(token) = parser.next() {
        if matches!(
            token,
            Token::CloseParenthesis | Token::CloseSquareBracket | Token::CloseCurlyBracket
        ) {
            return None;
        }
    }
    let fallback = input[fallback_start..].trim_matches(is_css_whitespace_char);
    Some((name.as_ref().into(), Some(fallback)))
}

fn is_css_whitespace_char(character: char) -> bool {
    matches!(character, ' ' | '\t' | '\n' | '\x0C' | '\r')
}

#[cfg(test)]
mod tests;
