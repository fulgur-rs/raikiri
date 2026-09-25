//! Tests for property value serialization in `serialize.rs`.

use super::*;

#[test]
fn serialize_length_formats_each_unit() {
    assert_eq!(serialize_length(&Length::Px(10.0)), "10px");
    assert_eq!(serialize_length(&Length::Em(1.5)), "1.5em");
    assert_eq!(serialize_length(&Length::Rem(1.0)), "1rem");
    assert_eq!(serialize_length(&Length::Percent(50.0)), "50%");
    assert_eq!(serialize_length(&Length::Pt(12.0)), "12pt");
    assert_eq!(serialize_length(&Length::Ex(1.0)), "1ex");
    assert_eq!(serialize_length(&Length::Rex(1.0)), "1rex");
    assert_eq!(serialize_length(&Length::Ch(1.0)), "1ch");
    assert_eq!(serialize_length(&Length::Rch(1.0)), "1rch");
    assert_eq!(serialize_length(&Length::Ic(1.0)), "1ic");
    assert_eq!(serialize_length(&Length::Ric(1.0)), "1ric");
    assert_eq!(serialize_length(&Length::Cm(1.0)), "1cm");
    assert_eq!(serialize_length(&Length::Mm(1.0)), "1mm");
    assert_eq!(serialize_length(&Length::Q(1.0)), "1q");
    assert_eq!(serialize_length(&Length::In(1.0)), "1in");
    assert_eq!(serialize_length(&Length::Pc(1.0)), "1pc");
    assert_eq!(serialize_length(&Length::Lh(1.0)), "1lh");
    assert_eq!(serialize_length(&Length::Rlh(1.0)), "1rlh");
}

#[test]
fn serialize_length_formats_negative_and_fractional_values() {
    assert_eq!(serialize_length(&Length::Px(-5.0)), "-5px");
    assert_eq!(serialize_length(&Length::Px(0.0)), "0px");
    assert_eq!(serialize_length(&Length::Percent(33.333332)), "33.3333%");
}

#[test]
fn serialize_length_or_auto_formats_length_and_auto() {
    assert_eq!(
        serialize_length_or_auto(&LengthOrAuto::Length(Length::Px(10.0))),
        "10px"
    );
    assert_eq!(serialize_length_or_auto(&LengthOrAuto::Auto), "auto");
}

#[test]
fn serialize_value_covers_simple_length_variants() {
    assert_eq!(
        serialize_value(&PropertyValue::PaddingTop(Length::Px(10.0))),
        Some("10px".to_owned())
    );
    assert_eq!(
        serialize_value(&PropertyValue::OutlineWidth(Length::Px(2.0))),
        Some("2px".to_owned())
    );
}

#[test]
fn serialize_value_covers_safe_length_or_auto_variants() {
    assert_eq!(
        serialize_value(&PropertyValue::Top(LengthOrAuto::Auto)),
        Some("auto".to_owned())
    );
    assert_eq!(
        serialize_value(&PropertyValue::MarginLeft(LengthOrAuto::Length(
            Length::Percent(10.0)
        ))),
        Some("10%".to_owned())
    );
}

#[test]
fn serialize_value_excludes_width_height_min_max_variants() {
    assert_eq!(
        serialize_value(&PropertyValue::Width(LengthOrAuto::Auto)),
        None
    );
    assert_eq!(
        serialize_value(&PropertyValue::Height(LengthOrAuto::Auto)),
        None
    );
    assert_eq!(
        serialize_value(&PropertyValue::MinWidth(LengthOrAuto::Auto)),
        None
    );
    assert_eq!(
        serialize_value(&PropertyValue::MinHeight(LengthOrAuto::Auto)),
        None
    );
    assert_eq!(
        serialize_value(&PropertyValue::MaxWidth(LengthOrAuto::Auto)),
        None
    );
    assert_eq!(
        serialize_value(&PropertyValue::MaxHeight(LengthOrAuto::Auto)),
        None
    );
}

#[test]
fn serialize_value_collapses_sides_by_css_box_model_rules() {
    assert_eq!(
        serialize_value(&PropertyValue::Padding(Sides::all(Length::Px(10.0)))),
        Some("10px".to_owned())
    );
    assert_eq!(
        serialize_value(&PropertyValue::Padding(Sides {
            top: Length::Px(1.0),
            right: Length::Px(2.0),
            bottom: Length::Px(1.0),
            left: Length::Px(2.0),
        })),
        Some("1px 2px".to_owned())
    );
    assert_eq!(
        serialize_value(&PropertyValue::Padding(Sides {
            top: Length::Px(1.0),
            right: Length::Px(2.0),
            bottom: Length::Px(3.0),
            left: Length::Px(2.0),
        })),
        Some("1px 2px 3px".to_owned())
    );
    assert_eq!(
        serialize_value(&PropertyValue::Padding(Sides {
            top: Length::Px(1.0),
            right: Length::Px(2.0),
            bottom: Length::Px(3.0),
            left: Length::Px(4.0),
        })),
        Some("1px 2px 3px 4px".to_owned())
    );
}

#[test]
fn serialize_value_collapses_start_end_pairs() {
    assert_eq!(
        serialize_value(&PropertyValue::PaddingInline(StartEnd {
            start: Length::Px(5.0),
            end: Length::Px(5.0),
        })),
        Some("5px".to_owned())
    );
    assert_eq!(
        serialize_value(&PropertyValue::PaddingInline(StartEnd {
            start: Length::Px(5.0),
            end: Length::Px(10.0),
        })),
        Some("5px 10px".to_owned())
    );
    assert_eq!(
        serialize_value(&PropertyValue::MarginBlock(StartEnd {
            start: LengthOrAuto::Auto,
            end: LengthOrAuto::Length(Length::Px(10.0)),
        })),
        Some("auto 10px".to_owned())
    );
}

#[test]
fn serialize_alpha_channel_round_trips_every_u8_through_channel_to_u8() {
    for alpha in 0u8..=255 {
        let serialized = serialize_alpha_channel(alpha);
        let reparsed: f32 = serialized
            .parse()
            .unwrap_or_else(|_| panic!("{serialized:?} should parse as a number"));
        assert_eq!(
            channel_to_u8(reparsed),
            alpha,
            "alpha {alpha} serialized as {serialized:?}, which does not round-trip"
        );
    }
}

#[test]
fn serialize_number_formats_plain_decimals_without_a_leading_dot() {
    assert_eq!(serialize_number(0.5), "0.5");
    assert_eq!(serialize_number(1.0), "1");
    assert_eq!(serialize_number(0.0), "0");
}

#[test]
fn serialize_css_color_formats_opaque_and_translucent() {
    assert_eq!(
        serialize_css_color(&CssColor {
            r: 34,
            g: 51,
            b: 68,
            a: 255
        }),
        "rgb(34, 51, 68)"
    );
    assert_eq!(
        serialize_css_color(&CssColor {
            r: 2,
            g: 3,
            b: 4,
            a: 128
        }),
        "rgba(2, 3, 4, 0.5)"
    );
    assert_eq!(
        serialize_css_color(&CssColor::TRANSPARENT),
        "rgba(0, 0, 0, 0)"
    );
}

#[test]
fn serialize_color_value_echoes_keyword_syntax_lowercased() {
    assert_eq!(
        serialize_color_value("color", "currentColor"),
        Some("currentcolor".to_owned())
    );
    assert_eq!(
        serialize_color_value("color", "transparent"),
        Some("transparent".to_owned())
    );
    assert_eq!(
        serialize_color_value("background-color", "teal"),
        Some("teal".to_owned())
    );
    assert_eq!(
        serialize_color_value("border-top-color", "red"),
        Some("red".to_owned())
    );
}

#[test]
fn serialize_color_value_canonicalizes_legacy_functional_syntax() {
    assert_eq!(
        serialize_color_value("color", "#234"),
        Some("rgb(34, 51, 68)".to_owned())
    );
    assert_eq!(
        serialize_color_value("color", "rgb(100%, 0%, 0%)"),
        Some("rgb(255, 0, 0)".to_owned())
    );
    assert_eq!(
        serialize_color_value("color", "hsl(120, 100%, 50%)"),
        Some("rgb(0, 255, 0)".to_owned())
    );
    assert_eq!(
        serialize_color_value("color", "hsla(120, 100%, 50%, 0.25)"),
        Some("rgba(0, 255, 0, 0.25)".to_owned())
    );
    assert_eq!(
        serialize_color_value("text-decoration-color", "rgba(2, 3, 4, 50%)"),
        Some("rgba(2, 3, 4, 0.5)".to_owned())
    );
}

#[test]
fn serialize_color_value_returns_none_for_modern_functional_syntax() {
    // lab()/lch()/oklab()/oklch() are covered by
    // serialize_color_value_handles_lab_plain_values and friends —
    // color()/color-mix()/color-layers()/light-dark()/contrast-color()
    // remain out of scope (Phase2b's non-goals, see the design spec).
    assert_eq!(
        serialize_color_value("background-color", "color(srgb 1 0 0)"),
        None
    );
    assert_eq!(
        serialize_color_value("color", "color-mix(in srgb, red, blue)"),
        None
    );
}

#[test]
fn serialize_color_value_returns_none_for_relative_color_syntax_under_a_legacy_function_name() {
    // A `from` clause can appear inside `rgb()`/`hsl()`/etc. too (CSS
    // Color 4's relative color syntax) — its resolved value must
    // serialize back through the origin's own notation, not through
    // `rgb()`/`rgba()`, so this must stay unserialized just like the
    // other modern-syntax cases above.
    assert_eq!(
        serialize_color_value("background-color", "rgb(from contrast-color(blue) r g b)"),
        None
    );
    assert_eq!(
        serialize_color_value("color", "rgb(from alpha(from currentcolor / 0.5) r g b)"),
        None
    );
}

#[test]
fn serialize_color_value_returns_none_for_names_it_does_not_own() {
    assert_eq!(serialize_color_value("width", "10px"), None);
}

#[test]
fn serialize_value_defers_border_color_shorthand() {
    assert_eq!(
        serialize_value(&PropertyValue::BorderColor(Sides {
            top: BorderColor::CurrentColor,
            right: BorderColor::CurrentColor,
            bottom: BorderColor::CurrentColor,
            left: BorderColor::CurrentColor,
        })),
        None
    );
}

#[test]
fn serialize_color_value_collapses_border_color_shorthand_by_box_model_rules() {
    assert_eq!(
        serialize_color_value("border-color", "currentcolor"),
        Some("currentcolor".to_owned())
    );
    assert_eq!(
        serialize_color_value("border-color", "currentColor"),
        Some("currentcolor".to_owned())
    );
    assert_eq!(
        serialize_color_value("border-color", "red yellow green blue"),
        Some("red yellow green blue".to_owned())
    );
    assert_eq!(
        serialize_color_value("border-color", "red green"),
        Some("red green".to_owned())
    );
    // 3 authored components (top, right, bottom) with left implied
    // equal to right: the resolved sides are red/green/red/green,
    // which collapses further to the 2-value form since top == bottom
    // and right == left too.
    assert_eq!(
        serialize_color_value("border-color", "red green red"),
        Some("red green".to_owned())
    );
    // 3 authored components that do NOT collapse further stay 3-value.
    assert_eq!(
        serialize_color_value("border-color", "red green blue"),
        Some("red green blue".to_owned())
    );
}

#[test]
fn serialize_color_value_splits_border_color_components_paren_aware() {
    // A naive whitespace split would misread `rgb(0 0 255)`'s internal
    // spaces as component boundaries.
    assert_eq!(
        serialize_color_value("border-color", "rgb(0 0 255) red"),
        Some("rgb(0, 0, 255) red".to_owned())
    );
}

#[test]
fn serialize_color_value_returns_none_for_border_color_with_a_modern_component() {
    // lab() is now supported (Phase2b) — use a function still out of
    // scope (color-mix()) so this keeps testing "an unsupported modern
    // component anywhere in the shorthand forces None for the whole
    // value."
    assert_eq!(
        serialize_color_value("border-color", "red color-mix(in srgb, red, blue)"),
        None
    );
}

#[test]
fn serialize_color_value_handles_lab_plain_values() {
    assert_eq!(
        serialize_color_value("color", "lab(0 0 0)"),
        Some("lab(0 0 0)".to_owned())
    );
    assert_eq!(
        serialize_color_value("color", "lab(-40 0 0)"),
        Some("lab(0 0 0)".to_owned())
    );
    assert_eq!(
        serialize_color_value("color", "lab(400 0 10/50%)"),
        Some("lab(100 0 10 / 0.5)".to_owned())
    );
    assert_eq!(
        serialize_color_value("color", "lab(50% 50% -20%)"),
        Some("lab(50 62.5 -25)".to_owned())
    );
}

#[test]
fn serialize_color_value_handles_oklab_scale_factors() {
    assert_eq!(
        serialize_color_value("color", "oklab(50% 50% -20%)"),
        Some("oklab(0.5 0.2 -0.08)".to_owned())
    );
}

#[test]
fn serialize_color_value_handles_lch_chroma_and_hue() {
    assert_eq!(
        serialize_color_value("color", "lch(20 -20 0)"),
        Some("lch(20 0 0)".to_owned())
    );
    assert_eq!(
        serialize_color_value("color", "lch(10 20 380deg)"),
        Some("lch(10 20 20)".to_owned())
    );
    assert_eq!(
        serialize_color_value("color", "lch(10 20 -700deg)"),
        Some("lch(10 20 20)".to_owned())
    );
    assert_eq!(
        serialize_color_value("color", "lch(10 20 1.28rad)"),
        Some("lch(10 20 73.3386)".to_owned())
    );
    // A unitless hue number still needs [0, 360) normalization — the
    // grammar treats it as degrees even without the `deg` suffix.
    assert_eq!(
        serialize_color_value("color", "lch(10 20 -700)"),
        Some("lch(10 20 20)".to_owned())
    );
    assert_eq!(
        serialize_color_value("color", "lch(0.5 -20% -20)"),
        Some("lch(0.5 0 340)".to_owned())
    );
}

#[test]
fn serialize_color_value_keeps_calc_folded_hue_unnormalized_with_its_deg_unit() {
    // Unlike a bare hue, a calc()-authored one is neither scaled nor
    // range-normalized — it folds to a plain evaluated number, keeping
    // `deg` only because an actual Angle operand (`20deg`) was present.
    assert_eq!(
        serialize_color_value(
            "color",
            "lch(calc(-50 * 3) calc(0.5 + 1) calc(-20deg * 2) / calc(-0.5 * 2))"
        ),
        Some("lch(calc(-150) calc(1.5) calc(-40deg) / calc(-1))".to_owned())
    );
    // A calc() hue with no Angle operand anywhere keeps no unit at all.
    assert_eq!(
        serialize_color_value("color", "lch(none 20 calc(0.5))"),
        Some("lch(none 20 calc(0.5))".to_owned())
    );
}

#[test]
fn serialize_color_value_handles_oklch_scale_factors() {
    assert_eq!(
        serialize_color_value("color", "oklch(20% 60% 10/0.5)"),
        Some("oklch(0.2 0.24 10 / 0.5)".to_owned())
    );
}

#[test]
fn serialize_color_value_preserves_none_in_lab_family() {
    assert_eq!(
        serialize_color_value("color", "lab(none 20 calc(0.5))"),
        Some("lab(none 20 calc(0.5))".to_owned())
    );
}

#[test]
fn serialize_color_value_folds_and_reorders_calc_in_lab_family() {
    assert_eq!(
        serialize_color_value(
            "color",
            "lab(calc(50 * 3) calc(0.5 - 1) calc(1.5) / calc(-0.5 + 1))"
        ),
        Some("lab(calc(150) calc(-0.5) calc(1.5) / calc(0.5))".to_owned())
    );
    assert_eq!(
        serialize_color_value(
            "color",
            "lab(calc(50 + (sign(1em - 10px) * 10)) 30 50 / 50%)"
        ),
        Some("lab(calc(50 + (10 * sign(1em - 10px))) 30 50 / 0.5)".to_owned())
    );
}

#[test]
fn serialize_color_value_returns_none_for_relative_lab_syntax() {
    assert_eq!(
        serialize_color_value("color", "lab(from red 50 20 10)"),
        None
    );
}

#[test]
fn computed_only_values_serialize_with_shared_css_number_formatting() {
    use crate::{
        ComputedLength, ComputedLetterSpacing, ComputedTabSize, ComputedTextDecorationThickness,
        ComputedTextIndent, ComputedTextUnderlineOffset,
    };
    use cssparser::ToCss as _;

    let mixed = CalcLengthPercentage {
        percent: 10.0,
        px: -2.0,
    };
    assert_eq!(ComputedLetterSpacing::Px(1.5).to_css_string(), "1.5px");
    assert_eq!(
        ComputedLetterSpacing::Percent(110.0).to_css_string(),
        "110%"
    );
    assert_eq!(
        ComputedLetterSpacing::Calc(mixed).to_css_string(),
        "calc(10% - 2px)"
    );
    assert_eq!(
        ComputedTextIndent::Px(1.2345678).to_css_string(),
        "1.23457px"
    );
    assert_eq!(ComputedTextIndent::Percent(5.0).to_css_string(), "5%");
    assert_eq!(
        ComputedTextIndent::Calc(mixed).to_css_string(),
        "calc(10% - 2px)"
    );
    assert_eq!(ComputedTextUnderlineOffset::Auto.to_css_string(), "auto");
    assert_eq!(
        ComputedTextUnderlineOffset::Length(ComputedLength(3.0)).to_css_string(),
        "3px"
    );
    assert_eq!(
        ComputedTextUnderlineOffset::Percent(10.0).to_css_string(),
        "10%"
    );
    assert_eq!(
        ComputedTextUnderlineOffset::Calc(mixed).to_css_string(),
        "calc(10% - 2px)"
    );
    assert_eq!(
        ComputedTextDecorationThickness::Auto.to_css_string(),
        "auto"
    );
    assert_eq!(
        ComputedTextDecorationThickness::FromFont.to_css_string(),
        "from-font"
    );
    assert_eq!(
        ComputedTextDecorationThickness::Length(ComputedLength(2.0)).to_css_string(),
        "2px"
    );
    assert_eq!(ComputedTabSize::Number(4.0).to_css_string(), "4");
    assert_eq!(
        ComputedTabSize::Length(ComputedLength(12.5)).to_css_string(),
        "12.5px"
    );
    assert_eq!(
        CssColor {
            r: 1,
            g: 2,
            b: 3,
            a: 255
        }
        .to_css_string(),
        "rgb(1, 2, 3)"
    );
    assert_eq!(
        CssColor {
            r: 1,
            g: 2,
            b: 3,
            a: 128
        }
        .to_css_string(),
        "rgba(1, 2, 3, 0.5)"
    );
}

#[test]
fn numbers_outside_i32_keep_their_value_instead_of_saturating() {
    assert_eq!(serialize_number(4.0), "4");
    assert_eq!(serialize_number(2.9), "2.9");
    // The largest f32 below 2^31 and i32::MIN still print as integers.
    assert_eq!(serialize_number(2147483520.0), "2147483520");
    assert_eq!(serialize_number(-2147483648.0), "-2147483648");
    // Past the i32 range every digit is kept, without clamping or exponent form.
    assert_eq!(serialize_number(2147483648.0), "2147483648");
    assert_eq!(serialize_number(3.0e9), "3000000000");
    assert_eq!(serialize_number(-3.0e9), "-3000000000");
    assert_eq!(serialize_length(&Length::Px(1.0e10)), "10000000000px");
    assert_eq!(serialize_length(&Length::Percent(3.0e9)), "3000000000%");
}

#[test]
fn percentages_format_their_own_number_without_scaling_error() {
    use cssparser::ToCss as _;

    // Formatting `value / 100` and scaling it back up rounds 1.562525 down.
    assert_eq!(serialize_length(&Length::Percent(1.562525)), "1.56253%");
    assert_eq!(serialize_length(&Length::Percent(12.5)), "12.5%");
    assert_eq!(serialize_length(&Length::Percent(-10.0)), "-10%");
    assert_eq!(
        crate::ComputedLetterSpacing::Percent(1.562525).to_css_string(),
        "1.56253%"
    );
}

#[test]
fn negative_zero_serializes_without_a_sign() {
    use cssparser::ToCss as _;

    assert_eq!(serialize_number(-0.0), "0");
    assert_eq!(serialize_length(&Length::Px(-0.0)), "0px");
    assert_eq!(serialize_length(&Length::Percent(-0.0)), "0%");
    assert_eq!(crate::ComputedLength(-0.0).to_css_string(), "0px");
}
