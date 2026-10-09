use super::*;
use crate::cascade::test_support::*;
use crate::cascade::{apply_value, cascade};
use crate::property::{CssColor, Length, OutlineColor, OutlineStyle, PropertyKey, VerticalAlign};
use crate::resolve::{
    ComputedLength, ComputedLengthPercentage, ComputedLengthPercentageOrAuto, ComputedLineHeight,
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
fn var_in_font_shorthand_resets_font_feature_settings() {
    use crate::property::FontFeatureSettings;
    let mut doc = TestDoc::new();
    let parent = doc.push_element(0, "p", Some("font-feature-settings: \"sinf\""));
    let child = doc.push_element(parent, "div", Some("--f: 20px serif; font: var(--f)"));
    let later = doc.push_element(
        parent,
        "em",
        Some("--f: 20px serif; font: var(--f); font-feature-settings: \"smcp\""),
    );
    let tree = build_rule_tree(&doc);
    let result = cascade(&doc, &tree).expect("cascade Ok");
    assert_ne!(
        result.computed[parent].font_feature_settings,
        FontFeatureSettings::Normal
    );
    assert_eq!(
        result.computed[child].font_feature_settings,
        FontFeatureSettings::Normal
    );
    assert_ne!(
        result.computed[later].font_feature_settings,
        FontFeatureSettings::Normal
    );
}

#[test]
fn var_in_font_shorthand_projects_each_deferred_longhand() {
    // Sibling of `var_in_background_shorthand_projects_each_deferred_longhand`:
    // sibling — `font: var(--f)` expands 6 grammar longhands plus 10
    // reset-only subproperties after substitution.
    use crate::property::{
        FontKerning, FontLanguageOverride, FontOpticalSizing, FontStyle, FontVariantCaps,
        FontVariantEastAsian, FontVariantEastAsianWidth, FontVariantEmoji, FontVariantLigatures,
        FontVariantNumeric, FontVariantPosition, FontVariationSetting, FontVariationSettings,
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
            "border-top",
            "1px solid red",
            vec![
                PropertyKey::BorderTopWidth,
                PropertyKey::BorderTopStyle,
                PropertyKey::BorderTopColor,
            ],
        ),
        (
            "border-top",
            "inherit",
            vec![
                PropertyKey::BorderTopWidth,
                PropertyKey::BorderTopStyle,
                PropertyKey::BorderTopColor,
            ],
        ),
        (
            "border-right",
            "1px solid red",
            vec![
                PropertyKey::BorderRightWidth,
                PropertyKey::BorderRightStyle,
                PropertyKey::BorderRightColor,
            ],
        ),
        (
            "border-right",
            "inherit",
            vec![
                PropertyKey::BorderRightWidth,
                PropertyKey::BorderRightStyle,
                PropertyKey::BorderRightColor,
            ],
        ),
        (
            "border-bottom",
            "1px solid red",
            vec![
                PropertyKey::BorderBottomWidth,
                PropertyKey::BorderBottomStyle,
                PropertyKey::BorderBottomColor,
            ],
        ),
        (
            "border-bottom",
            "inherit",
            vec![
                PropertyKey::BorderBottomWidth,
                PropertyKey::BorderBottomStyle,
                PropertyKey::BorderBottomColor,
            ],
        ),
        (
            "border-left",
            "1px solid red",
            vec![
                PropertyKey::BorderLeftWidth,
                PropertyKey::BorderLeftStyle,
                PropertyKey::BorderLeftColor,
            ],
        ),
        (
            "border-left",
            "inherit",
            vec![
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
                PropertyKey::FontFeatureSettings,
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
                PropertyKey::FontFeatureSettings,
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
fn list_style_defaulting_marker_cannot_project_to_an_unrelated_property() {
    assert_eq!(
        project_deferred_value(
            PropertyValue::Deferred(DeferredValue {
                property: "list-style".into(),
                value: "inherit".into(),
                key: PropertyKey::ListStyle,
            }),
            PropertyKey::Color
        ),
        None
    );
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
fn important_var_value_keeps_its_whole_text() {
    // `Parser::next` skips the unread body of a `var(` token before returning
    // the `!` that follows it, so the value must end at that `!` rather than
    // where the parser stood before reading it.
    let cv = cascade_doc(
        "p { --accent: red; color: var(--accent) !important } p { color: blue }",
        "p",
        None,
    );
    assert_eq!(cv.color, RED);
}

#[test]
fn inline_important_var_value_resolves() {
    let cv = cascade_doc(
        "",
        "p",
        Some("--accent: red; color: var(--accent) !important"),
    );
    assert_eq!(cv.color, RED);
}

#[test]
fn important_var_in_shorthand_resolves_every_longhand() {
    let cv = cascade_doc(
        "p { --gap: 3px; margin: auto var(--gap) !important }",
        "p",
        None,
    );
    assert_eq!(cv.margin.top, ComputedLengthPercentageOrAuto::Auto);
    assert_eq!(cv.margin.right, ComputedLengthPercentageOrAuto::Px(3.0));
    assert_eq!(cv.margin.bottom, ComputedLengthPercentageOrAuto::Auto);
    assert_eq!(cv.margin.left, ComputedLengthPercentageOrAuto::Px(3.0));
}

#[test]
fn important_var_in_logical_shorthand_resolves() {
    let cv = cascade_doc(
        "p { --gap: 3px; padding-block: var(--gap) !important }",
        "p",
        None,
    );
    assert_eq!(cv.padding.top, ComputedLengthPercentage::Px(3.0));
    assert_eq!(cv.padding.bottom, ComputedLengthPercentage::Px(3.0));
}

#[test]
fn important_math_with_var_resolves() {
    let cv = cascade_doc(
        "p { --w: 10px; width: calc(var(--w) + 5px) !important }",
        "p",
        None,
    );
    assert_eq!(cv.width, ComputedLengthPercentageOrAuto::Px(15.0));
}

#[test]
fn empty_unterminated_var_is_invalid_not_a_panic() {
    for style in ["width: var(", "width: calc(1px + var("] {
        let cv = cascade_doc("", "p", Some(style));
        assert_eq!(cv.width, ComputedLengthPercentageOrAuto::Auto, "{style}");
    }
}

#[test]
fn var_left_open_at_the_end_of_input_is_closed_there() {
    // CSS Syntax 3 returns a function the input ends inside of, so a style
    // attribute ending in `var(--x` reads as `var(--x)`.
    let cv = cascade_doc("", "p", Some("--x: 7px; width: var(--x"));
    assert_eq!(cv.width, ComputedLengthPercentageOrAuto::Px(7.0));
    let cv = cascade_doc("", "p", Some("--x: 7px; width: calc(var(--x) + 1px"));
    assert_eq!(cv.width, ComputedLengthPercentageOrAuto::Px(8.0));
}

#[test]
fn var_after_another_component_keeps_the_separator() {
    let cv = cascade_doc("", "p", Some("--x: 2px; margin: 1px var(--x)"));
    assert_eq!(cv.margin.top, ComputedLengthPercentageOrAuto::Px(1.0));
    assert_eq!(cv.margin.right, ComputedLengthPercentageOrAuto::Px(2.0));
    assert_eq!(cv.margin.bottom, ComputedLengthPercentageOrAuto::Px(1.0));
    assert_eq!(cv.margin.left, ComputedLengthPercentageOrAuto::Px(2.0));

    let cv = cascade_doc("", "p", Some("--gap: 5px; padding: 0 var(--gap) 3px"));
    assert_eq!(cv.padding.right, ComputedLengthPercentage::Px(5.0));
    assert_eq!(cv.padding.bottom, ComputedLengthPercentage::Px(3.0));
}

#[test]
fn border_shorthand_with_a_trailing_var_color_resolves() {
    let cv = cascade_doc("", "p", Some("--c: red; border: 1px solid var(--c)"));
    assert_eq!(cv.border.top.width, ComputedLength(1.0));
    assert_eq!(cv.border.left.style, crate::property::BorderStyle::Solid);
    assert_eq!(
        cv.border.bottom.color,
        crate::property::BorderColor::Resolved(RED)
    );
}

#[test]
fn math_after_another_component_keeps_the_separator() {
    let cv = cascade_doc("", "p", Some("--x: 2px; margin: 1px calc(var(--x) + 1px)"));
    assert_eq!(cv.margin.top, ComputedLengthPercentageOrAuto::Px(1.0));
    assert_eq!(cv.margin.right, ComputedLengthPercentageOrAuto::Px(3.0));
}

const CALC_100_PERCENT_MINUS_20PX: ComputedLengthPercentageOrAuto =
    ComputedLengthPercentageOrAuto::Calc(crate::property::CalcLengthPercentage {
        percent: 100.0,
        px: -20.0,
    });

#[test]
fn comment_before_a_mixed_calc_substitution_keeps_it_valid() {
    for width in ["/**/var(--w)", "/* note */ var(--w)", "var(--w) /* note */"] {
        let style = format!("--w: calc(100% - 20px); width: {width}");
        let cv = cascade_doc("", "div", Some(&style));
        assert_eq!(cv.width, CALC_100_PERCENT_MINUS_20PX, "{style}");
    }
    let cv = cascade_doc(
        "",
        "div",
        Some("--w: /**/ calc(100% - 20px) /**/ !important; width: var(--w)"),
    );
    assert_eq!(cv.width, CALC_100_PERCENT_MINUS_20PX);
}

#[test]
fn mixed_calc_left_open_at_the_end_of_input_is_closed_there() {
    for style in [
        "width: calc(100% - 20px",
        "--x: 20px; width: calc(100% - var(--x)",
    ] {
        let cv = cascade_doc("", "div", Some(style));
        assert_eq!(cv.width, CALC_100_PERCENT_MINUS_20PX, "{style}");
    }
}

#[test]
fn important_custom_property_keeps_its_function_value() {
    let cv = cascade_doc(
        "p { --c: rgb(255, 0, 0) !important; color: var(--c) }",
        "p",
        None,
    );
    assert_eq!(cv.color, RED);
    assert_eq!(
        cv.resolved_custom_property("--c").as_deref(),
        Some("rgb(255, 0, 0)")
    );
}

#[test]
fn comment_next_to_a_substitution_keeps_the_tokens_apart() {
    for style in [
        "--c: 2px; margin: 1px/**/var(--c)",
        "--a: 1px; --b: 2px; margin: var(--a)/**/var(--b)",
    ] {
        let cv = cascade_doc("", "p", Some(style));
        assert_eq!(
            cv.margin.top,
            ComputedLengthPercentageOrAuto::Px(1.0),
            "{style}"
        );
        assert_eq!(
            cv.margin.right,
            ComputedLengthPercentageOrAuto::Px(2.0),
            "{style}"
        );
    }
}

#[test]
fn var_after_a_calc_operator_resolves() {
    let cv = cascade_doc("", "p", Some("--x: 5px; margin-top: calc(10px + var(--x))"));
    assert_eq!(cv.margin.top, ComputedLengthPercentageOrAuto::Px(15.0));
}

#[test]
fn custom_property_value_excludes_surrounding_comments() {
    // CSS Syntax 3 keeps a custom property's original text from its first to
    // its last token; comments produce no tokens.
    for style in [
        "--c: /* lead */ a /* trail */",
        "--c: /* lead */ a /* trail */ !important",
        "--c: a /* left open",
    ] {
        let cv = cascade_doc("", "p", Some(style));
        assert_eq!(
            cv.resolved_custom_property("--c").as_deref(),
            Some("a"),
            "{style}"
        );
    }
    let cv = cascade_doc("", "p", Some("--c: a /* kept */ b"));
    assert_eq!(
        cv.resolved_custom_property("--c").as_deref(),
        Some("a /* kept */ b")
    );
}

#[test]
fn trimming_keeps_strings_and_escapes_whole() {
    for (input, trimmed) in [
        ("  /* a */ x /* b */ ", "x"),
        ("\"/* kept */\" /* c */", "\"/* kept */\""),
        ("'left open /* x */", "'left open /* x */"),
        ("a\\  ", "a\\ "),
        ("a\\", "a\\"),
        ("x /* left open", "x"),
        ("\u{3000}é\u{a0} ", "\u{3000}é\u{a0}"),
        (" /**/ ", ""),
        ("", ""),
    ] {
        assert_eq!(
            trim_css_whitespace_and_comments(input),
            trimmed,
            "{input:?}"
        );
    }
}

#[test]
fn custom_property_value_keeps_non_css_whitespace() {
    // Only space, tab and the newlines are CSS whitespace; U+3000 and U+00A0
    // are ident code points.
    for (style, value) in [
        ("--c: x\u{3000}", "x\u{3000}"),
        ("--c: x\u{a0} !important", "x\u{a0}"),
        ("--c: \u{3000}x", "\u{3000}x"),
    ] {
        let cv = cascade_doc("", "p", Some(style));
        assert_eq!(
            cv.resolved_custom_property("--c").as_deref(),
            Some(value),
            "{style:?}"
        );
    }
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
        (PropertyKey::InlineSize, PropertyValue::InlineSize(calc)),
        (PropertyKey::BlockSize, PropertyValue::BlockSize(calc)),
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
fn parse_simple_calc_length_percentage_closes_a_calc_left_open_at_the_end() {
    assert_eq!(
        parse_simple_calc_length_percentage("calc(50%"),
        Some(CalcLengthPercentage {
            percent: 50.0,
            px: 0.0
        })
    );
    assert_eq!(
        parse_simple_calc_length_percentage("calc(50%) + (10px"),
        None
    );
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

#[test]
fn radius_inheritance_substitution_retains_shorthand_type_before_expansion() {
    let environment = CustomPropertyEnvironment::from_map(HashMap::new());
    let deferred = DeferredValue {
        property: "border-radius".into(),
        value: "var(--missing, inherit)".into(),
        key: PropertyKey::BorderRadius,
    };
    assert_eq!(
        resolve_deferred_value(&deferred, &environment),
        Some(PropertyValue::BorderRadiusInherit)
    );
    assert_eq!(
        project_deferred_value(
            PropertyValue::BorderRadius(crate::property::BorderRadius::elliptical(
                [Length::Px(5.0); 4],
                [Length::Px(5.0); 4]
            )),
            PropertyKey::Color
        ),
        None
    );
}
