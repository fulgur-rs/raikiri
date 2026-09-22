use cssparser::{CowRcStr, ParseError, Parser, ParserInput, ToCss as _, Token};

use super::calc_serialize::{
    CalcNode, CalcUnitKind, parse_calc_or_plain, serialize_calc_node, serialize_calc_node_as_angle,
};
use super::parse::{channel_to_u8, parse_color, parse_color_float};
use super::types::*;

/// Serializes a parsed [`PropertyValue`] back to canonical CSS text, for
/// WPT `test_valid_value` assertions that expect a specific serialization
/// rather than an echo of the input. Returns `None` for any variant this
/// crate cannot yet serialize (or, for `Width`/`Height`/`Min*`/`Max*`,
/// cannot *correctly* serialize with their current parsed representation —
/// see the Tier2 Phase 1 plan's Global Constraints). Callers should fall
/// back to echoing the raw input string when this returns `None`.
pub fn serialize_value(value: &PropertyValue) -> Option<String> {
    match value {
        PropertyValue::FontSize(l)
        | PropertyValue::PaddingTop(l)
        | PropertyValue::PaddingRight(l)
        | PropertyValue::PaddingBottom(l)
        | PropertyValue::PaddingLeft(l)
        | PropertyValue::BorderTopWidth(l)
        | PropertyValue::BorderRightWidth(l)
        | PropertyValue::BorderBottomWidth(l)
        | PropertyValue::BorderLeftWidth(l)
        | PropertyValue::BorderRadiusTopLeft(l)
        | PropertyValue::BorderRadiusTopRight(l)
        | PropertyValue::BorderRadiusBottomRight(l)
        | PropertyValue::BorderRadiusBottomLeft(l)
        | PropertyValue::OutlineWidth(l)
        | PropertyValue::OutlineOffset(l) => Some(serialize_length(l)),

        PropertyValue::Top(v)
        | PropertyValue::Right(v)
        | PropertyValue::Bottom(v)
        | PropertyValue::Left(v)
        | PropertyValue::MarginTop(v)
        | PropertyValue::MarginRight(v)
        | PropertyValue::MarginBottom(v)
        | PropertyValue::MarginLeft(v) => Some(serialize_length_or_auto(v)),

        PropertyValue::Padding(sides) => Some(serialize_sides(sides, serialize_length)),
        PropertyValue::BorderWidth(sides) => Some(serialize_sides(sides, serialize_length)),
        PropertyValue::Margin(sides) => Some(serialize_sides(sides, serialize_length_or_auto)),

        PropertyValue::PaddingInline(pair) | PropertyValue::PaddingBlock(pair) => {
            Some(serialize_start_end(pair, serialize_length))
        }
        PropertyValue::MarginInline(pair) | PropertyValue::MarginBlock(pair) => {
            Some(serialize_start_end(pair, serialize_length_or_auto))
        }

        // Width/Height/MinWidth/MinHeight/MaxWidth/MaxHeight all wrap
        // LengthOrAuto too, but parse_width/parse_min_size/parse_max_size
        // fold `auto`/`min-content`/`max-content`/bare `fit-content` into
        // the same LengthOrAuto::Auto, discarding which keyword was
        // written. Serializing Auto as "auto" here would silently turn a
        // currently-correct echoed "none"/"min-content"/"max-content" into
        // a wrong "auto" — deliberately left unserialized (falls back to
        // echo) until those three parse functions preserve the keyword.
        PropertyValue::Width(_)
        | PropertyValue::Height(_)
        | PropertyValue::MinWidth(_)
        | PropertyValue::MinHeight(_)
        | PropertyValue::MaxWidth(_)
        | PropertyValue::MaxHeight(_) => None,

        // `color`/`background-color`/`border-*-color`/`text-decoration-color`/
        // `outline-color` (including the `border-color` shorthand here) are
        // deliberately absent even though they carry `CssColor`/
        // `CurrentColor`-wrapping payloads: see `serialize_color_value`,
        // which serializes them from raw CSS text instead, because the
        // parsed value alone cannot distinguish keyword syntax from
        // legacy-functional syntax from modern-functional syntax (all three
        // can produce the same `CssColor`).
        PropertyValue::BorderColor(_) => None,

        PropertyValue::TextDecorationInset(inset) => Some(match inset {
            TextDecorationInset::Auto => "auto".to_owned(),
            TextDecorationInset::Lengths { start, end } => {
                let start = serialize_length(start);
                let end = serialize_length(end);
                if start == end {
                    start
                } else {
                    format!("{start} {end}")
                }
            }
        }),

        PropertyValue::TextUnderlineOffset(value) => Some(serialize_length_or_auto(value)),

        PropertyValue::HangingPunctuation(value) => Some(match value {
            HangingPunctuation::None => "none".to_owned(),
            HangingPunctuation::First => "first".to_owned(),
        }),

        _ => None,
    }
}

/// Serializes a numeric CSS dimension (`10px`, `1.5em`) using the exact
/// number-formatting algorithm `cssparser`'s own tokenizer uses for
/// `Token::Dimension`/`Token::Percentage` (shortest round-tripping decimal
/// via `dtoa_short`, integers printed without a decimal point). Building a
/// `Token` and calling its `to_css_string()` reuses that algorithm instead
/// of reimplementing CSS number serialization here.
fn serialize_dimension(value: f32, unit: &str) -> String {
    let int_value = if value.fract() == 0.0 {
        Some(value as i32)
    } else {
        None
    };
    Token::Dimension {
        has_sign: false,
        value,
        int_value,
        unit: CowRcStr::from(unit),
    }
    .to_css_string()
}

fn serialize_percentage(value: f32) -> String {
    let int_value = if value.fract() == 0.0 {
        Some(value as i32)
    } else {
        None
    };
    Token::Percentage {
        has_sign: false,
        unit_value: value / 100.0,
        int_value,
    }
    .to_css_string()
}

/// Serializes a [`Length`] back to CSS text (`Length::Px(10.0)` -> `"10px"`).
pub(crate) fn serialize_length(length: &Length) -> String {
    match *length {
        Length::Px(v) => serialize_dimension(v, "px"),
        Length::Em(v) => serialize_dimension(v, "em"),
        Length::Rem(v) => serialize_dimension(v, "rem"),
        Length::Percent(v) => serialize_percentage(v),
        Length::Pt(v) => serialize_dimension(v, "pt"),
        Length::Ex(v) => serialize_dimension(v, "ex"),
        Length::Rex(v) => serialize_dimension(v, "rex"),
        Length::Ch(v) => serialize_dimension(v, "ch"),
        Length::Rch(v) => serialize_dimension(v, "rch"),
        Length::Ic(v) => serialize_dimension(v, "ic"),
        Length::Ric(v) => serialize_dimension(v, "ric"),
        Length::Cm(v) => serialize_dimension(v, "cm"),
        Length::Mm(v) => serialize_dimension(v, "mm"),
        Length::Q(v) => serialize_dimension(v, "q"),
        Length::In(v) => serialize_dimension(v, "in"),
        Length::Pc(v) => serialize_dimension(v, "pc"),
        Length::Lh(v) => serialize_dimension(v, "lh"),
        Length::Rlh(v) => serialize_dimension(v, "rlh"),
    }
}

/// `LengthOrAuto::Calc` is never constructed by any parser in this file
/// today (verified: no `LengthOrAuto::Calc(` or `::Calc(` construction site
/// exists in `property.rs`). This keeps the match exhaustive and gives a
/// spec-plausible `calc()` serialization if that ever changes, rather than
/// a `match` arm that would need revisiting the moment it does.
fn serialize_calc_length_percentage(calc: &CalcLengthPercentage) -> String {
    if calc.px == 0.0 {
        return format!("calc({})", serialize_percentage(calc.percent));
    }
    if calc.percent == 0.0 {
        return format!("calc({})", serialize_dimension(calc.px, "px"));
    }
    if calc.px >= 0.0 {
        format!(
            "calc({} + {})",
            serialize_percentage(calc.percent),
            serialize_dimension(calc.px, "px")
        )
    } else {
        format!(
            "calc({} - {})",
            serialize_percentage(calc.percent),
            serialize_dimension(-calc.px, "px")
        )
    }
}

/// Serializes a [`LengthOrAuto`] back to CSS text. Callers must confirm
/// `Auto` is unambiguous for the property they are serializing before
/// using this — see the `Width`/`Height`/`Min*`/`Max*` exclusion in
/// [`serialize_value`].
pub(crate) fn serialize_length_or_auto(value: &LengthOrAuto) -> String {
    match value {
        LengthOrAuto::Length(l) => serialize_length(l),
        LengthOrAuto::Auto => "auto".to_owned(),
        LengthOrAuto::Calc(calc) => serialize_calc_length_percentage(calc),
    }
}

/// `<alpha-value>` serialization for legacy `rgba()`: finds the shortest
/// decimal that re-quantizes, via [`channel_to_u8`], back to the same u8.
/// `channel_to_u8` already discarded the original alpha precision when
/// parsing, so naive `alpha as f32 / 255.0` division does not reproduce
/// clean expected decimals (e.g. u8 128 -> 0.50196..., not "0.5"). Any u8
/// alpha's rounding bucket under `channel_to_u8` spans ~1/255 (~0.00392),
/// wider than a 3-decimal-place grid step (0.001), so a grid search up to 3
/// decimal places always finds a match.
pub(crate) fn serialize_alpha_channel(alpha: u8) -> String {
    let exact = f64::from(alpha) / 255.0;
    for digits in 0..=3 {
        let scale = 10f64.powi(digits);
        let rounded = (exact * scale).round() / scale;
        if channel_to_u8(rounded as f32) == alpha {
            return serialize_number(rounded as f32);
        }
    }
    unreachable!("a 3-decimal-place grid always round-trips through channel_to_u8")
}

pub(crate) fn serialize_number(value: f32) -> String {
    let int_value = if value.fract() == 0.0 {
        Some(value as i32)
    } else {
        None
    };
    Token::Number {
        has_sign: false,
        value,
        int_value,
    }
    .to_css_string()
}

/// Serializes a [`CssColor`] as legacy `rgb()`/`rgba()` notation. CSS Color 4
/// §5.1 "The RGB functions" makes `rgb()`/`rgba()` aliases sharing one
/// grammar, and legacy syntax always serializes via comma-separated
/// `rgb()`/`rgba()` (§"Serializing color values").
pub(crate) fn serialize_css_color(color: &CssColor) -> String {
    if color.a == 255 {
        format!("rgb({}, {}, {})", color.r, color.g, color.b)
    } else {
        format!(
            "rgba({}, {}, {}, {})",
            color.r,
            color.g,
            color.b,
            serialize_alpha_channel(color.a)
        )
    }
}

fn parse_color_entirely(raw_value: &str) -> Option<CssColor> {
    let mut input = ParserInput::new(raw_value);
    let mut parser = Parser::new(&mut input);
    parser
        .parse_entirely(|i| -> Result<CssColor, ParseError<'_, ()>> {
            parse_color(i).ok_or_else(|| i.new_custom_error(()))
        })
        .ok()
}

/// Serializes a single-value `<color>` property's specified value directly
/// from its raw CSS text, for the property names whose `PropertyValue`
/// payload is a bare [`CssColor`] or a `CurrentColor`-wrapping enum around
/// one: `color`, `background-color`, the four physical `border-*-color`
/// longhands, `text-decoration-color`, `outline-color`. `border-color`'s
/// `<color>{1,4}` shorthand goes through [`serialize_border_color_shorthand`]
/// instead, since it is several raw `<color>` values, not one.
///
/// CSS Color 4 "Serializing color values" gives three different
/// serializations depending on how a color was written, and the parsed
/// `PropertyValue` alone cannot tell them apart — a resolved
/// `CssColor{255,0,0,255}` could have come from `red`, `#ff0000`, or
/// `rgb(255,0,0)`, three different correct serializations:
///
/// - keyword syntax (named colors, `transparent`, `currentcolor`, system
///   colors) serializes as the keyword itself, lowercased;
/// - legacy functional syntax (hex notation, `rgb()`, `rgba()`, `hsl()`,
///   `hsla()`, `hwb()`) always canonicalizes to comma-separated
///   `rgb()`/`rgba()`;
/// - anything else (modern functional syntax: `lab()`, `lch()`, `oklab()`,
///   `oklch()`, `color()`, `color-mix()`, `color-layers()`,
///   `light-dark()`, `contrast-color()`, relative `from` forms) must
///   preserve its own functional notation, which a resolved `CssColor`'s
///   u8 sRGB triple cannot represent — deliberately returns `None` here
///   (falls back to echo) until a color-space-aware value model exists.
///
/// This re-parses `raw_value` directly rather than going through
/// [`serialize_value`], because the syntax classification above happens
/// before `parse_color` collapses everything to `CssColor` — recovering it
/// from the already-parsed `PropertyValue` is not possible without
/// changing `CssColor`/`PropertyValue`'s shape, which is public API used
/// well beyond this parser (cascade, paint, and downstream consumers).
pub fn serialize_color_value(name: &str, raw_value: &str) -> Option<String> {
    if name == "border-color" {
        return serialize_border_color_shorthand(raw_value);
    }
    if !matches!(
        name,
        "color"
            | "background-color"
            | "border-top-color"
            | "border-right-color"
            | "border-bottom-color"
            | "border-left-color"
            | "text-decoration-color"
            | "outline-color"
    ) {
        return None;
    }
    serialize_one_color(raw_value)
}

struct LabFamilySpec {
    function_name: &'static str,
    lightness_scale: f64,
    lightness_max: f64,
    second_scale: f64,
    second_clamp_min: Option<f64>,
    third_kind: ThirdComponentKind,
}

enum ThirdComponentKind {
    Cartesian,
    Angle,
}

const LAB_FAMILY_SPECS: &[LabFamilySpec] = &[
    LabFamilySpec {
        function_name: "lab",
        lightness_scale: 100.0,
        lightness_max: 100.0,
        second_scale: 125.0,
        second_clamp_min: None,
        third_kind: ThirdComponentKind::Cartesian,
    },
    LabFamilySpec {
        function_name: "oklab",
        lightness_scale: 1.0,
        lightness_max: 1.0,
        second_scale: 0.4,
        second_clamp_min: None,
        third_kind: ThirdComponentKind::Cartesian,
    },
    LabFamilySpec {
        function_name: "lch",
        lightness_scale: 100.0,
        lightness_max: 100.0,
        second_scale: 150.0,
        second_clamp_min: Some(0.0),
        third_kind: ThirdComponentKind::Angle,
    },
    LabFamilySpec {
        function_name: "oklch",
        lightness_scale: 1.0,
        lightness_max: 1.0,
        second_scale: 0.4,
        second_clamp_min: Some(0.0),
        third_kind: ThirdComponentKind::Angle,
    },
];

/// One `<lab()>`/`<lch()>`/`<oklab()>`/`<oklch()>` component: either the
/// literal `none` keyword, or a value to scale/clamp/serialize.
enum LabComponent {
    None,
    /// A bare number/percentage/angle — gets its function's scale factor
    /// and clamp range applied at serialize time.
    Plain(CalcNode),
    /// A `calc(...)`-wrapped value. Per the real corpus (e.g.
    /// `lab(200 calc(50%) 0.5)` -> `lab(100 calc(50%) 0.5)`: the bare `200`
    /// clamps to `100`, but `calc(50%)` serializes unchanged, without the
    /// lightness scale factor or clamp applied), a `calc()`-authored
    /// component is serialized as-is — no scaling, no clamping.
    Calc(CalcNode),
}

fn parse_lab_component(
    input: &mut Parser<'_, '_>,
    unit_kind: CalcUnitKind,
) -> Option<LabComponent> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(LabComponent::None);
    }
    let start = input.state();
    let is_calc = input
        .try_parse(|i| match i.next() {
            Ok(Token::Function(name)) if name.eq_ignore_ascii_case("calc") => Ok(()),
            _ => Err(()),
        })
        .is_ok();
    input.reset(&start);
    let node = parse_calc_or_plain(input, unit_kind)?;
    Some(if is_calc {
        LabComponent::Calc(node)
    } else {
        LabComponent::Plain(node)
    })
}

/// Tries `preferred_unit_kind` first, then falls back to a plain number —
/// covers every lab-family component's grammar, which always accepts
/// `<number>` in addition to its own preferred unit (percentage for L/a/b/
/// C/alpha, angle for hue).
fn parse_lab_component_preferring(
    input: &mut Parser<'_, '_>,
    preferred_unit_kind: CalcUnitKind,
) -> Option<LabComponent> {
    let start = input.state();
    if let Some(component) = parse_lab_component(input, preferred_unit_kind) {
        return Some(component);
    }
    input.reset(&start);
    parse_lab_component(input, CalcUnitKind::Number)
}

/// Scales and clamps a parsed component per the given scale/clamp range,
/// returning its final serialized text (either `none`, a folded/reordered
/// `calc(...)`, or a plain number). Only a fully-resolved constant
/// Number/Percentage/Angle gets the scale factor and clamp applied — an
/// unresolved (font-relative) calc() expression can't be scaled here since
/// its real value isn't known until used-value time; the real corpus never
/// asks this module to scale+reorder in the same value.
fn serialize_lab_component(
    component: &LabComponent,
    scale: f64,
    clamp_min: Option<f64>,
    clamp_max: Option<f64>,
    is_angle: bool,
) -> String {
    let node = match component {
        LabComponent::None => return "none".to_owned(),
        LabComponent::Calc(node) if is_angle => return serialize_calc_node_as_angle(node),
        LabComponent::Calc(node) => return serialize_calc_node(node),
        LabComponent::Plain(node) => node,
    };
    // A hue's bare `<number>` (no `deg` suffix, e.g. `lch(10 20 -700)`)
    // still needs the [0, 360) normalization a `CalcNode::Angle` gets —
    // the grammar treats a unitless hue as degrees.
    let scaled = match node {
        CalcNode::Percentage(v) => Some(v / 100.0 * scale),
        CalcNode::Number(v) if is_angle => Some(v.rem_euclid(360.0)),
        CalcNode::Number(v) => Some(*v),
        CalcNode::Angle(v) => Some(v.rem_euclid(360.0)),
        _ => None,
    };
    let Some(mut value) = scaled else {
        return serialize_calc_node(node);
    };
    if let Some(min) = clamp_min {
        value = value.max(min);
    }
    if let Some(max) = clamp_max {
        value = value.min(max);
    }
    serialize_calc_node(&CalcNode::Number(value))
        .trim_start_matches("calc(")
        .trim_end_matches(')')
        .to_owned()
}

fn serialize_lab_family_function(spec: &LabFamilySpec, raw_value: &str) -> Option<String> {
    let mut input = ParserInput::new(raw_value);
    let mut parser = Parser::new(&mut input);

    let start = parser.state();
    let is_relative = parser
        .try_parse(|i| -> Result<(), ParseError<'_, ()>> {
            i.expect_function_matching(spec.function_name)?;
            i.parse_nested_block(|nested| -> Result<(), ParseError<'_, ()>> {
                nested
                    .try_parse(|n| n.expect_ident_matching("from"))
                    .map_err(|_| nested.new_custom_error(()))
            })
        })
        .is_ok();
    if is_relative {
        return None; // relative color syntax, e.g. `lab(from red l a b)`
    }
    parser.reset(&start);

    parser.expect_function_matching(spec.function_name).ok()?;
    parser
        .parse_nested_block(|nested| -> Result<String, ParseError<'_, ()>> {
            let lightness = parse_lab_component_preferring(nested, CalcUnitKind::Percentage)
                .ok_or_else(|| nested.new_custom_error(()))?;
            let second = parse_lab_component_preferring(nested, CalcUnitKind::Percentage)
                .ok_or_else(|| nested.new_custom_error(()))?;
            let third_unit = match spec.third_kind {
                ThirdComponentKind::Cartesian => CalcUnitKind::Percentage,
                ThirdComponentKind::Angle => CalcUnitKind::Angle,
            };
            let third = parse_lab_component_preferring(nested, third_unit)
                .ok_or_else(|| nested.new_custom_error(()))?;

            let alpha = if nested.try_parse(|i| i.expect_delim('/')).is_ok() {
                Some(
                    parse_lab_component_preferring(nested, CalcUnitKind::Percentage)
                        .ok_or_else(|| nested.new_custom_error(()))?,
                )
            } else {
                None
            };

            if !nested.is_exhausted() {
                return Err(nested.new_custom_error(()));
            }

            let is_angle_third = matches!(spec.third_kind, ThirdComponentKind::Angle);
            let lightness_text = serialize_lab_component(
                &lightness,
                spec.lightness_scale,
                Some(0.0),
                Some(spec.lightness_max),
                false,
            );
            let second_text = serialize_lab_component(
                &second,
                spec.second_scale,
                spec.second_clamp_min,
                None,
                false,
            );
            let third_scale = match spec.third_kind {
                ThirdComponentKind::Cartesian => spec.second_scale,
                ThirdComponentKind::Angle => 1.0,
            };
            let third_text =
                serialize_lab_component(&third, third_scale, None, None, is_angle_third);

            let function_name = spec.function_name;
            let mut result = format!("{function_name}({lightness_text} {second_text} {third_text}");
            if let Some(alpha) = alpha {
                let alpha_text = serialize_lab_component(&alpha, 1.0, Some(0.0), Some(1.0), false);
                if alpha_text != "1" {
                    result.push_str(&format!(" / {alpha_text}"));
                }
            }
            result.push(')');
            Ok(result)
        })
        .ok()
}

/// The single-`<color>`-value classification logic behind
/// [`serialize_color_value`], factored out so
/// [`serialize_border_color_shorthand`] can apply it to each of its
/// shorthand's 1-4 components individually.
fn serialize_one_color(raw_value: &str) -> Option<String> {
    let mut input = ParserInput::new(raw_value);
    let mut parser = Parser::new(&mut input);
    let leading_token = parser.next().ok()?.clone();

    match leading_token {
        Token::Ident(ident) => Some(ident.as_ref().to_ascii_lowercase()),
        Token::Hash(_) | Token::IDHash(_) => {
            parse_color_entirely(raw_value).map(|color| serialize_css_color(&color))
        }
        Token::Function(ref function_name)
            if matches!(
                function_name.to_ascii_lowercase().as_str(),
                "rgb" | "rgba" | "hsl" | "hsla" | "hwb"
            ) =>
        {
            // CSS Color 4's relative color syntax lets a `from` clause
            // appear inside any of these legacy function names (e.g.
            // `rgb(from contrast-color(blue) r g b)`); its resolved
            // value must serialize back using the origin's own
            // notation, which a resolved `CssColor`'s u8 sRGB triple
            // loses just like the modern-syntax cases below. Peek past
            // the function name for a leading `from` keyword before
            // treating this as ordinary legacy syntax.
            //
            // `parse_nested_block` requires its closure to consume the
            // whole block (it is `parse_entirely` under the hood) or it
            // reports that as an error — irrelevant here, since we only
            // want to peek at the leading token. Capture the answer via
            // a side effect set before that exhaustion check runs, and
            // discard the block result itself.
            let mut is_relative = false;
            let _: Result<(), ParseError<'_, ()>> = parser.parse_nested_block(|nested| {
                is_relative = nested
                    .try_parse(|i| i.expect_ident_matching("from"))
                    .is_ok();
                Ok(())
            });
            if is_relative {
                None
            } else {
                parse_color_entirely(raw_value).map(|color| serialize_css_color(&color))
            }
        }
        Token::Function(ref function_name)
            if matches!(
                function_name.to_ascii_lowercase().as_str(),
                "lab" | "lch" | "oklab" | "oklch"
            ) =>
        {
            let name = function_name.to_ascii_lowercase();
            let spec = LAB_FAMILY_SPECS
                .iter()
                .find(|spec| spec.function_name == name)?;
            serialize_lab_family_function(spec, raw_value)
        }
        _ => None,
    }
}

/// Serializes `border-color`'s `<color>{1,4}` shorthand from its raw CSS
/// text. Splits it into its 1-4 authored `<color>` components using the
/// real parser (not a whitespace split — a legacy functional color like
/// `rgb(0 0 255)` contains internal spaces a naive split would misread as
/// component boundaries), classifies each component via
/// [`serialize_one_color`], expands them to the four physical positions per
/// the CSS box-model shorthand rule, and re-collapses using the same 1/2/3
/// -value rule `serialize_sides` applies to typed values — but by string
/// equality of the already-serialized components.
///
/// This can miss collapsing two components that are the same color written
/// in different syntax (e.g. `red` and `#ff0000` both resolve to the same
/// `CssColor`, but serialize to different strings and so compare unequal
/// here). No known WPT fixture exercises that case; catching it would need
/// each component's `BorderColor`-equivalent resolved value alongside its
/// serialized string, not just the string.
fn serialize_border_color_shorthand(raw_value: &str) -> Option<String> {
    let mut input = ParserInput::new(raw_value);
    let mut parser = Parser::new(&mut input);

    let mut components: Vec<String> = Vec::with_capacity(4);
    while !parser.is_exhausted() {
        let start = parser.position();
        parse_color_float(&mut parser, 0)?;
        let component = parser.slice_from(start);
        components.push(serialize_one_color(component)?);
    }

    let resolved: [&str; 4] = match components.len() {
        1 => [
            &components[0],
            &components[0],
            &components[0],
            &components[0],
        ],
        2 => [
            &components[0],
            &components[1],
            &components[0],
            &components[1],
        ],
        3 => [
            &components[0],
            &components[1],
            &components[2],
            &components[1],
        ],
        4 => [
            &components[0],
            &components[1],
            &components[2],
            &components[3],
        ],
        // 0 (empty input) or 5+ components are not valid `border-color`
        // syntax; a caller that pre-validated via `parse_value` never
        // reaches this, but `serialize_color_value` is `pub` and re-parses
        // `raw_value` independently of any such validation, so this reports
        // the mismatch as `None` rather than panicking on a malformed
        // direct call.
        _ => return None,
    };
    let [top, right, bottom, left] = resolved;

    Some(if top == right && right == bottom && bottom == left {
        top.to_owned()
    } else if top == bottom && right == left {
        format!("{top} {right}")
    } else if right == left {
        format!("{top} {right} {bottom}")
    } else {
        format!("{top} {right} {bottom} {left}")
    })
}

/// Collapses a [`Sides`] value using the CSS box-model 1-4 value
/// serialization rule (CSS Box 3 §3.2 / §4.2, the shorthand's own
/// serialization algorithm): all four equal -> one value; top==bottom and
/// left==right -> two values; left==right only -> three values; otherwise
/// four values, in top/right/bottom/left order.
pub(crate) fn serialize_sides<T: PartialEq>(
    sides: &Sides<T>,
    serialize: impl Fn(&T) -> String,
) -> String {
    let top = serialize(&sides.top);
    let right = serialize(&sides.right);
    let bottom = serialize(&sides.bottom);
    let left = serialize(&sides.left);
    if sides.top == sides.right && sides.top == sides.bottom && sides.top == sides.left {
        top
    } else if sides.top == sides.bottom && sides.right == sides.left {
        format!("{top} {right}")
    } else if sides.right == sides.left {
        format!("{top} {right} {bottom}")
    } else {
        format!("{top} {right} {bottom} {left}")
    }
}

/// Collapses a [`StartEnd`] value using the CSS Logical Properties 2-value
/// shorthand serialization rule: `start == end` -> one value, else two,
/// in start/end order.
pub(crate) fn serialize_start_end<T: PartialEq>(
    pair: &StartEnd<T>,
    serialize: impl Fn(&T) -> String,
) -> String {
    let start = serialize(&pair.start);
    let end = serialize(&pair.end);
    if pair.start == pair.end {
        start
    } else {
        format!("{start} {end}")
    }
}
