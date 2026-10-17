use super::*;

#[test]
fn from_name_is_ascii_case_insensitive_and_maps_legacy_aliases() {
    assert_eq!(
        ComputedProperty::from_name("letter-spacing"),
        Some(ComputedProperty::LetterSpacing)
    );
    assert_eq!(
        ComputedProperty::from_name("LETTER-Spacing"),
        Some(ComputedProperty::LetterSpacing)
    );
    assert_eq!(
        ComputedProperty::from_name("word-wrap"),
        Some(ComputedProperty::OverflowWrap)
    );
    assert_eq!(
        ComputedProperty::from_name("FONT-VARIATION-SETTINGS"),
        Some(ComputedProperty::FontVariationSettings)
    );
    assert_eq!(
        ComputedProperty::from_name("FONT-FEATURE-SETTINGS"),
        Some(ComputedProperty::FontFeatureSettings)
    );
    assert_eq!(
        ComputedProperty::from_name("FONT"),
        Some(ComputedProperty::Font)
    );
    assert_eq!(
        ComputedProperty::from_name("COLOR"),
        Some(ComputedProperty::Color)
    );
    assert_eq!(ComputedProperty::from_name(""), None);
}

#[test]
fn font_shorthand_serializes_computed_grammar_values() {
    let mut computed = ComputedValues::initial();
    let property = ComputedProperty::from_name("font").expect("font is a computed shorthand");

    assert_eq!(property.serialize(&computed), Some("16px serif".to_owned()));

    computed.font_style = crate::property::FontStyle::Italic;
    computed.font_variant_caps = crate::property::FontVariantCaps::SmallCaps;
    computed.font_weight = 700.0;
    computed.font_size = crate::resolve::ComputedLength(18.0);
    computed.line_height = crate::resolve::ComputedLineHeight::Number(1.5);
    computed.font_family = std::sync::Arc::new(vec![
        crate::property::FontFamilyName::named("Open Sans"),
        crate::property::FontFamilyName::generic("serif"),
    ]);
    assert_eq!(
        property.serialize(&computed),
        Some(r#"italic small-caps 700 18px/1.5 "Open Sans", serif"#.to_owned())
    );

    computed.font_variant_caps = crate::property::FontVariantCaps::Normal;
    computed.font_palette = crate::property::FontPaletteValue::Light;
    assert_eq!(
        property.serialize(&computed),
        Some(r#"italic 700 18px/1.5 "Open Sans", serif"#.to_owned())
    );

    computed.font_family =
        std::sync::Arc::new(vec![crate::property::FontFamilyName::named("serif")]);
    assert_eq!(
        property.serialize(&computed),
        Some(r#"italic 700 18px/1.5 "serif""#.to_owned())
    );

    computed.font_style = crate::property::FontStyle::Oblique;
    computed.line_height =
        crate::resolve::ComputedLineHeight::Length(crate::resolve::ComputedLength(20.0));
    computed.font_family =
        std::sync::Arc::new(vec![crate::property::FontFamilyName::generic("serif")]);
    assert_eq!(
        property.serialize(&computed),
        Some("oblique 700 18px/20px serif".to_owned())
    );

    computed.font_family = std::sync::Arc::new(Vec::new());
    assert_eq!(property.serialize(&computed), None);
}

#[test]
fn font_shorthand_serialization_rejects_unrepresentable_or_noninitial_subproperties() {
    let property = ComputedProperty::from_name("font").expect("font is a computed shorthand");

    macro_rules! assert_unserializable_with {
        ($field:ident, $value:expr) => {{
            let mut computed = ComputedValues::initial();
            computed.$field = $value;
            assert!(
                property.serialize(&computed).is_none(),
                "{} should prevent font shorthand serialization",
                stringify!($field)
            );
        }};
    }

    assert_unserializable_with!(font_kerning, crate::property::FontKerning::Normal);
    assert_unserializable_with!(
        font_optical_sizing,
        crate::property::FontOpticalSizing::None
    );
    assert_unserializable_with!(font_variant_emoji, crate::property::FontVariantEmoji::Text);
    assert_unserializable_with!(
        font_language_override,
        crate::property::FontLanguageOverride::String("SRB".into())
    );
    assert_unserializable_with!(
        font_variant_ligatures,
        crate::property::FontVariantLigatures::None
    );
    assert_unserializable_with!(
        font_variant_position,
        crate::property::FontVariantPosition::Sub
    );
    assert_unserializable_with!(
        font_variant_numeric,
        crate::property::FontVariantNumeric {
            ordinal: true,
            ..crate::property::FontVariantNumeric::initial()
        }
    );
    assert_unserializable_with!(
        font_variant_east_asian,
        crate::property::FontVariantEastAsian {
            width: Some(crate::property::FontVariantEastAsianWidth::FullWidth),
            ..crate::property::FontVariantEastAsian::initial()
        }
    );
    assert_unserializable_with!(
        font_variant_caps,
        crate::property::FontVariantCaps::AllSmallCaps
    );
    assert_unserializable_with!(
        font_variation_settings,
        crate::property::FontVariationSettings::Settings(
            vec![crate::property::FontVariationSetting {
                tag: "wght".into(),
                value: 700.0,
            }]
            .into()
        )
    );
    assert_unserializable_with!(
        font_feature_settings,
        crate::property::FontFeatureSettings::Features(
            vec![crate::property::FontFeatureSetting {
                tag: *b"sinf",
                value: 1,
            }]
            .into()
        )
    );
}

#[test]
fn font_variation_settings_serializes_the_computed_value() {
    let mut computed = ComputedValues::initial();
    computed.font_variation_settings = crate::property::FontVariationSettings::Settings(
        vec![
            crate::property::FontVariationSetting {
                tag: "wdth".into(),
                value: 90.0,
            },
            crate::property::FontVariationSetting {
                tag: "wght".into(),
                value: 700.0,
            },
        ]
        .into(),
    );
    assert_eq!(
        ComputedProperty::FontVariationSettings.serialize(&computed),
        Some("\"wdth\" 90, \"wght\" 700".to_owned())
    );
}

#[test]
fn font_feature_settings_serializes_the_canonical_computed_value() {
    use crate::property::{FontFeatureSetting, FontFeatureSettings};

    let mut computed = ComputedValues::initial();
    computed.font_feature_settings = FontFeatureSettings::Features(
        vec![
            FontFeatureSetting {
                tag: *b"kern",
                value: 0,
            },
            FontFeatureSetting {
                tag: *b"liga",
                value: 0,
            },
        ]
        .into(),
    );
    assert_eq!(
        ComputedProperty::FontFeatureSettings.serialize(&computed),
        Some(r#""kern" 0, "liga" 0"#.to_owned())
    );
}

#[test]
fn color_serializes_as_a_resolved_rgb_value() {
    let mut computed = ComputedValues::initial();

    assert_eq!(
        ComputedProperty::Color.serialize(&computed),
        Some("rgb(0, 0, 0)".to_owned())
    );

    computed.color = CssColor {
        r: 255,
        g: 0,
        b: 0,
        a: 128,
    };
    assert_eq!(
        ComputedProperty::Color.serialize(&computed),
        Some("rgba(255, 0, 0, 0.5)".to_owned())
    );
}

#[test]
fn every_property_serializes_its_initial_value() {
    let initial = ComputedValues::initial();
    for &property in ComputedProperty::ALL {
        assert!(property.serialize(&initial).is_some(), "{property:?}");
    }
}

#[test]
fn text_decoration_inset_serializes_resolved_ch_lengths() {
    let mut computed = ComputedValues::initial();
    computed.text_decoration_inset = crate::resolve::ComputedTextDecorationInset::Lengths {
        start: crate::resolve::ComputedLength(3.0),
        end: crate::resolve::ComputedLength(1.5),
    };
    assert_eq!(
        ComputedProperty::TextDecorationInset.serialize(&computed),
        Some("3px 1.5px".to_owned())
    );
}

#[test]
fn text_decoration_inset_uses_shared_css_number_formatting() {
    let mut computed = ComputedValues::initial();
    computed.text_decoration_inset = crate::resolve::ComputedTextDecorationInset::Lengths {
        start: crate::resolve::ComputedLength(1.2345678),
        end: crate::resolve::ComputedLength(-0.0),
    };
    assert_eq!(
        ComputedProperty::TextDecorationInset.serialize(&computed),
        Some("1.23457px 0px".to_owned())
    );
    computed.text_decoration_inset = crate::resolve::ComputedTextDecorationInset::Lengths {
        start: crate::resolve::ComputedLength(2.5),
        end: crate::resolve::ComputedLength(2.5),
    };
    assert_eq!(
        ComputedProperty::TextDecorationInset.serialize(&computed),
        Some("2.5px".to_owned())
    );
}

#[test]
fn computed_calc_values_fold_a_zero_term_like_cascade_does() {
    use crate::property::CalcLengthPercentage;
    use crate::resolve::{ComputedLetterSpacing, ComputedTextIndent, ComputedTextUnderlineOffset};
    use cssparser::ToCss as _;

    let percent_only = CalcLengthPercentage {
        percent: 10.0,
        px: 0.0,
    };
    let px_only = CalcLengthPercentage {
        percent: 0.0,
        px: -2.0,
    };
    let mixed = CalcLengthPercentage {
        percent: 10.0,
        px: -2.0,
    };
    assert_eq!(
        ComputedLetterSpacing::Calc(percent_only).to_css_string(),
        "10%"
    );
    assert_eq!(ComputedTextIndent::Calc(px_only).to_css_string(), "-2px");
    assert_eq!(
        ComputedTextUnderlineOffset::Calc(mixed).to_css_string(),
        "calc(10% - 2px)"
    );
}

#[test]
fn ch_spacing_and_indent_serialize_the_cascade_resolved_values() {
    let mut computed = ComputedValues::initial();
    computed.letter_spacing_computed = crate::resolve::ComputedLetterSpacing::Px(10.0);
    computed.word_spacing_computed = crate::resolve::ComputedLetterSpacing::Px(5.0);
    computed.text_indent = crate::resolve::ComputedTextIndent::Px(27.0);
    assert_eq!(
        ComputedProperty::LetterSpacing.serialize(&computed),
        Some("10px".to_owned())
    );
    assert_eq!(
        ComputedProperty::WordSpacing.serialize(&computed),
        Some("5px".to_owned())
    );
    assert_eq!(
        ComputedProperty::TextIndent.serialize(&computed),
        Some("27px".to_owned())
    );
    computed.letter_spacing_computed = crate::resolve::ComputedLetterSpacing::Px(0.0);
    assert_eq!(
        ComputedProperty::LetterSpacing.serialize(&computed),
        Some("normal".to_owned())
    );
}

#[test]
fn overflow_axes_serialize_separately() {
    let mut computed = ComputedValues::initial();
    computed.overflow = crate::property::OverflowXY {
        x: crate::property::OverflowValue::Hidden,
        y: crate::property::OverflowValue::Scroll,
    };
    let serialize = |name| {
        ComputedProperty::from_name(name)
            .unwrap()
            .serialize(&computed)
    };
    assert_eq!(serialize("overflow-x").as_deref(), Some("hidden"));
    assert_eq!(serialize("overflow-y").as_deref(), Some("scroll"));
}
