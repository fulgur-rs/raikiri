use super::*;
use taffy::{Dimension, LengthPercentageAuto as Lpa, Style};

#[test]
fn legacy_sizing_restores_only_affected_fields_in_reverse_order() {
    let mut style = Style::<raikiri_style::SmolStr> {
        size: taffy::Size {
            width: Dimension::length(101.0),
            height: Dimension::length(102.0),
        },
        min_size: taffy::Size {
            width: Lpa::length(103.0),
            height: Lpa::length(104.0),
        },
        max_size: taffy::Size {
            width: Lpa::length(105.0),
            height: Lpa::length(106.0),
        },
        aspect_ratio: Some(2.0),
        ..Style::default()
    };
    let mut undo = Vec::new();
    for (property, px) in [
        (LengthProperty::Width, 11.0),
        (LengthProperty::Height, 12.0),
        (LengthProperty::MinWidth, 13.0),
        (LengthProperty::MinHeight, 14.0),
        (LengthProperty::MaxWidth, 15.0),
        (LengthProperty::MaxHeight, 16.0),
    ] {
        undo.push(
            OwnedPropertyOverride::length(property, L::Px(px), true)
                .unwrap()
                .apply_legacy_sizing(&mut style)
                .unwrap(),
        );
    }
    assert_eq!(style.size.width, Dimension::length(11.0));
    assert_eq!(style.size.height, Dimension::length(12.0));
    assert_eq!(style.min_size.width, Lpa::length(13.0));
    assert_eq!(style.min_size.height, Lpa::length(14.0));
    assert_eq!(style.max_size.width, Lpa::length(15.0));
    assert_eq!(style.max_size.height, Lpa::length(16.0));

    let nested = OwnedPropertyOverride::length(LengthProperty::Width, L::Px(77.0), true)
        .unwrap()
        .apply_legacy_sizing(&mut style)
        .unwrap();
    assert_eq!(style.size.width, Dimension::length(77.0));
    nested.restore(&mut style);
    assert_eq!(style.size.width, Dimension::length(11.0));
    let ratio = OwnedPropertyOverride::aspect_ratio(None)
        .apply_legacy_sizing(&mut style)
        .unwrap();
    assert_eq!(style.aspect_ratio, None);
    style.margin.left = Lpa::length(91.0);
    ratio.restore(&mut style);
    for saved in undo.into_iter().rev() {
        saved.restore(&mut style);
    }
    assert_eq!(style.size.width, Dimension::length(101.0));
    assert_eq!(style.size.height, Dimension::length(102.0));
    assert_eq!(style.min_size.width, Lpa::length(103.0));
    assert_eq!(style.min_size.height, Lpa::length(104.0));
    assert_eq!(style.max_size.width, Lpa::length(105.0));
    assert_eq!(style.max_size.height, Lpa::length(106.0));
    assert_eq!(style.aspect_ratio, Some(2.0));
    assert_eq!(style.margin.left, Lpa::length(91.0));
}

#[test]
fn legacy_sizing_keeps_percentage_and_calc_owner_identity() {
    let mut style = Style::<raikiri_style::SmolStr>::default();
    let percent =
        OwnedPropertyOverride::length(LengthProperty::Height, L::Percent(25.0), false).unwrap();
    let percent_undo = percent.apply_legacy_sizing(&mut style).unwrap();
    assert_eq!(style.size.height, Dimension::percent(0.25));
    let calc = OwnedPropertyOverride::length(
        LengthProperty::MinWidth,
        L::Calc(CalcLengthPercentage {
            px: -10.0,
            percent: 25.0,
        }),
        false,
    )
    .unwrap();
    let owner = calc.calc_owner().unwrap();
    let calc_undo = calc.apply_legacy_sizing(&mut style).unwrap();
    assert_eq!(style.min_size.width, Lpa::calc(owner.pointer()));
    assert!(owner.non_negative());
    calc_undo.restore(&mut style);
    percent_undo.restore(&mut style);
    assert_eq!(style.min_size.width, Lpa::auto());
    assert_eq!(style.size.height, Dimension::auto());
}

#[test]
fn legacy_sizing_rejects_unrelated_properties_before_mutation() {
    let mut style = Style::<raikiri_style::SmolStr>::default();
    style.margin.left = Lpa::length(91.0);
    let entry =
        OwnedPropertyOverride::length(LengthProperty::MarginLeft, L::Px(3.0), true).unwrap();
    assert!(matches!(
        entry.apply_legacy_sizing(&mut style),
        Err(OverrideError::UnsupportedLegacySizing)
    ));
    assert_eq!(style.margin.left, Lpa::length(91.0));
    assert_eq!(style.size.width, Dimension::auto());
}

#[test]
fn transposed_legacy_sizing_swaps_physical_targets_and_undoes_only_the_target() {
    let mut style = Style::<raikiri_style::SmolStr>::default();
    style.padding.left = taffy::LengthPercentage::length(73.0);
    for (property, expected) in [
        (LengthProperty::Width, 11.0),
        (LengthProperty::Height, 12.0),
        (LengthProperty::MinWidth, 13.0),
        (LengthProperty::MinHeight, 14.0),
        (LengthProperty::MaxWidth, 15.0),
        (LengthProperty::MaxHeight, 16.0),
    ] {
        let undo = OwnedPropertyOverride::length(property, L::Px(expected), true)
            .unwrap()
            .apply_transposed_legacy_sizing(&mut style)
            .unwrap();
        match property {
            LengthProperty::Width => assert_eq!(style.size.height, Dimension::length(11.0)),
            LengthProperty::Height => assert_eq!(style.size.width, Dimension::length(12.0)),
            LengthProperty::MinWidth => assert_eq!(style.min_size.height, Lpa::length(13.0)),
            LengthProperty::MinHeight => assert_eq!(style.min_size.width, Lpa::length(14.0)),
            LengthProperty::MaxWidth => assert_eq!(style.max_size.height, Lpa::length(15.0)),
            LengthProperty::MaxHeight => assert_eq!(style.max_size.width, Lpa::length(16.0)),
            _ => unreachable!(),
        }
        undo.restore(&mut style);
        assert_eq!(style.size, taffy::Size::auto());
        assert_eq!(style.min_size, taffy::Size::auto());
        assert_eq!(style.max_size, taffy::Size::auto());
        assert_eq!(style.padding.left, taffy::LengthPercentage::length(73.0));
    }
    for ratio in [None, Some(3.0), Some(0.0)] {
        let undo = OwnedPropertyOverride::aspect_ratio(ratio)
            .apply_transposed_legacy_sizing(&mut style)
            .unwrap();
        assert_eq!(style.aspect_ratio, ratio.map(|r| 1.0 / r));
        undo.restore(&mut style);
        assert_eq!(style.aspect_ratio, None);
    }
}
