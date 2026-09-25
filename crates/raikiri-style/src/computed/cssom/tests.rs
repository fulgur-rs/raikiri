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
    assert_eq!(ComputedProperty::from_name("color"), None);
    assert_eq!(ComputedProperty::from_name(""), None);
}

#[test]
fn every_property_serializes_its_initial_value() {
    let initial = ComputedValues::initial();
    let mut ch_advance = |_: &ChFontKey| -> f32 { panic!("initial values have no ch lengths") };
    for &property in ComputedProperty::ALL {
        assert!(
            property.serialize(&initial, &mut ch_advance).is_some(),
            "{property:?}"
        );
    }
}

#[test]
fn text_decoration_inset_measures_ch_lengths_through_the_callback() {
    let mut computed = ComputedValues::initial();
    let font = ChFontKey {
        family: computed.font_family.clone(),
        size: computed.font_size,
        weight: computed.font_weight,
        style: computed.font_style,
    };
    computed.text_decoration_inset = crate::resolve::ComputedTextDecorationInset::Lengths {
        start: crate::resolve::ComputedLength(1.0),
        end: crate::resolve::ComputedLength(4.0),
    };
    computed.text_decoration_inset_start_ch = Some(crate::ChLengthProvenance {
        factor: 2.0,
        font: font.clone(),
    });
    computed.text_decoration_inset_end_ch = Some(crate::ChLengthProvenance { factor: 1.0, font });
    let mut calls = 0;
    let mut ch_advance = |_: &ChFontKey| {
        calls += 1;
        1.5
    };
    assert_eq!(
        ComputedProperty::TextDecorationInset.serialize(&computed, &mut ch_advance),
        Some("3px 1.5px".to_owned())
    );
    assert_eq!(calls, 2);
}

#[test]
fn text_decoration_inset_uses_shared_css_number_formatting() {
    let mut computed = ComputedValues::initial();
    let mut ch_advance = |_: &ChFontKey| -> f32 { panic!("no ch lengths") };
    computed.text_decoration_inset = crate::resolve::ComputedTextDecorationInset::Lengths {
        start: crate::resolve::ComputedLength(1.2345678),
        end: crate::resolve::ComputedLength(-0.0),
    };
    assert_eq!(
        ComputedProperty::TextDecorationInset.serialize(&computed, &mut ch_advance),
        Some("1.23457px 0px".to_owned())
    );
    computed.text_decoration_inset = crate::resolve::ComputedTextDecorationInset::Lengths {
        start: crate::resolve::ComputedLength(2.5),
        end: crate::resolve::ComputedLength(2.5),
    };
    assert_eq!(
        ComputedProperty::TextDecorationInset.serialize(&computed, &mut ch_advance),
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
fn ch_spacing_and_indent_are_measured_through_the_callback() {
    let mut computed = ComputedValues::initial();
    computed.letter_spacing_computed = crate::resolve::ComputedLetterSpacing::Px(8.0);
    computed.letter_spacing_ch_factor = Some(2.0);
    computed.word_spacing_computed = crate::resolve::ComputedLetterSpacing::Px(4.0);
    computed.word_spacing_ch_factor = Some(1.0);
    computed.text_indent = crate::resolve::ComputedTextIndent::Px(12.0);
    computed.text_indent_ch_factor = Some(3.0);
    let declaring_font = ChFontKey {
        family: computed.font_family.clone(),
        size: crate::resolve::ComputedLength(40.0),
        weight: computed.font_weight,
        style: computed.font_style,
    };
    computed.text_indent_ch_font = Some(declaring_font.clone());
    let own_size = computed.font_size;
    // The own font measures 5px per ch; the declaring font 9px.
    let mut ch_advance = |font: &ChFontKey| {
        if *font == declaring_font {
            9.0
        } else {
            assert_eq!(font.size, own_size);
            5.0
        }
    };
    assert_eq!(
        ComputedProperty::LetterSpacing.serialize(&computed, &mut ch_advance),
        Some("10px".to_owned())
    );
    assert_eq!(
        ComputedProperty::WordSpacing.serialize(&computed, &mut ch_advance),
        Some("5px".to_owned())
    );
    assert_eq!(
        ComputedProperty::TextIndent.serialize(&computed, &mut ch_advance),
        Some("27px".to_owned())
    );
    // A zero `ch` letter-spacing still reads back as `normal`.
    computed.letter_spacing_ch_factor = Some(0.0);
    assert_eq!(
        ComputedProperty::LetterSpacing.serialize(&computed, &mut ch_advance),
        Some("normal".to_owned())
    );
}
