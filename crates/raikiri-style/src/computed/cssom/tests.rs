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
    computed.text_decoration_inset_start_ch = Some(crate::ChLengthProvenance { factor: 2.0, font });
    let mut calls = 0;
    let mut ch_advance = |_: &ChFontKey| {
        calls += 1;
        1.5
    };
    assert_eq!(
        ComputedProperty::TextDecorationInset.serialize(&computed, &mut ch_advance),
        Some("3px 4px".to_owned())
    );
    assert_eq!(calls, 1);
}
