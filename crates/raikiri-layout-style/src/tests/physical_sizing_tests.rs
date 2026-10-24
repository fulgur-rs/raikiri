use super::*;
use crate::values::{CalcRef, Dimension, LengthPercentage as Lp, LengthPercentageAuto as Lpa};

#[test]
fn physical_sizing_replaces_only_the_affected_fields() {
    let mut style = BoxStyle {
        aspect_ratio: Some(2.0),
        ..BoxStyle::default()
    };
    style.margin.left = Lpa::length(91.0);
    for (property, px) in [
        (LengthProperty::Width, 11.0),
        (LengthProperty::Height, 12.0),
        (LengthProperty::MinWidth, 13.0),
        (LengthProperty::MinHeight, 14.0),
        (LengthProperty::MaxWidth, 15.0),
        (LengthProperty::MaxHeight, 16.0),
    ] {
        OwnedPropertyOverride::length(property, L::Px(px), true)
            .unwrap()
            .apply_physical_sizing(&mut style)
            .unwrap();
    }
    assert_eq!(style.size.width, Dimension::length(11.0));
    assert_eq!(style.size.height, Dimension::length(12.0));
    assert_eq!(style.min_size.width, Lpa::length(13.0));
    assert_eq!(style.min_size.height, Lpa::length(14.0));
    assert_eq!(style.max_size.width, Lpa::length(15.0));
    assert_eq!(style.max_size.height, Lpa::length(16.0));
    assert_eq!(style.aspect_ratio, Some(2.0));

    OwnedPropertyOverride::length(LengthProperty::Width, L::Px(77.0), true)
        .unwrap()
        .apply_physical_sizing(&mut style)
        .unwrap();
    assert_eq!(style.size.width, Dimension::length(77.0));
    assert_eq!(style.size.height, Dimension::length(12.0));
    OwnedPropertyOverride::aspect_ratio(None)
        .apply_physical_sizing(&mut style)
        .unwrap();
    assert_eq!(style.aspect_ratio, None);
    assert_eq!(style.margin.left, Lpa::length(91.0));
}

#[test]
fn physical_sizing_keeps_percentage_and_calc_owner_identity() {
    let mut style = BoxStyle::default();
    let percent =
        OwnedPropertyOverride::length(LengthProperty::Height, L::Percent(25.0), false).unwrap();
    percent.apply_physical_sizing(&mut style).unwrap();
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
    calc.apply_physical_sizing(&mut style).unwrap();
    assert_eq!(style.min_size.width, Lpa::calc(CalcRef::new(owner)));
    assert!(owner.non_negative());
}

#[test]
fn physical_sizing_rejects_unrelated_properties_before_mutation() {
    let mut style = BoxStyle::default();
    style.margin.left = Lpa::length(91.0);
    let entry =
        OwnedPropertyOverride::length(LengthProperty::MarginLeft, L::Px(3.0), true).unwrap();
    assert!(matches!(
        entry.apply_physical_sizing(&mut style),
        Err(OverrideError::UnsupportedPhysicalSizing)
    ));
    assert_eq!(style.margin.left, Lpa::length(91.0));
    assert_eq!(style.size.width, Dimension::auto());
}

#[test]
fn physical_sizing_membership_matches_the_accepted_properties() {
    use LengthProperty::*;
    for property in [
        Width,
        Height,
        MinWidth,
        MinHeight,
        MaxWidth,
        MaxHeight,
        BorderTop,
        BorderRight,
        BorderBottom,
        BorderLeft,
    ] {
        let entry = OwnedPropertyOverride::length(property, L::Px(3.0), true).unwrap();
        assert!(entry.is_physical_sizing(), "{property:?}");
        assert!(
            entry
                .apply_physical_sizing(&mut BoxStyle::default())
                .is_ok()
        );
    }
    for property in [MarginLeft, PaddingTop, Top, FlexBasis, ColumnGap] {
        let entry = OwnedPropertyOverride::length(property, L::Px(3.0), true).unwrap();
        assert!(!entry.is_physical_sizing(), "{property:?}");
        assert!(
            entry
                .apply_physical_sizing(&mut BoxStyle::default())
                .is_err()
        );
    }
    assert!(OwnedPropertyOverride::flex_shrink(0.0).is_physical_sizing());
    assert!(OwnedPropertyOverride::aspect_ratio(Some(2.0)).is_physical_sizing());
    let clear = OwnedPropertyOverride::clear(raikiri_style::property::ClearValue::Both);
    assert!(!clear.is_physical_sizing());
    assert!(
        clear
            .apply_physical_sizing(&mut BoxStyle::default())
            .is_err()
    );
}

#[test]
fn physical_sizing_sets_border_fields_and_ignores_shrink() {
    let mut style = BoxStyle {
        border: Edges {
            top: Lp::length(1.0),
            right: Lp::length(2.0),
            bottom: Lp::length(3.0),
            left: Lp::length(4.0),
        },
        ..BoxStyle::default()
    };
    style.margin.left = Lpa::length(91.0);
    for (property, px) in [
        (LengthProperty::BorderTop, 11.0),
        (LengthProperty::BorderRight, 12.0),
        (LengthProperty::BorderBottom, 13.0),
        (LengthProperty::BorderLeft, 14.0),
    ] {
        OwnedPropertyOverride::length(property, L::Px(px), true)
            .unwrap()
            .apply_physical_sizing(&mut style)
            .unwrap();
    }
    let before_shrink = style.clone();
    OwnedPropertyOverride::flex_shrink(1.0)
        .apply_physical_sizing(&mut style)
        .unwrap();
    // A box style holds no flex properties, so the shrink factor changes nothing.
    assert_eq!(style, before_shrink);
    assert_eq!(
        style.border,
        Edges {
            top: Lp::length(11.0),
            right: Lp::length(12.0),
            bottom: Lp::length(13.0),
            left: Lp::length(14.0)
        }
    );
    assert_eq!(style.margin.left, Lpa::length(91.0));
}

#[test]
fn physical_sizing_keeps_border_calc_owner_identity() {
    let mut style = BoxStyle::default();
    let entry = OwnedPropertyOverride::length(
        LengthProperty::BorderLeft,
        L::Calc(CalcLengthPercentage {
            px: -10.0,
            percent: 25.0,
        }),
        false,
    )
    .unwrap();
    let owner = entry.calc_owner().unwrap().clone();
    entry.apply_physical_sizing(&mut style).unwrap();
    drop(entry);
    assert_eq!(style.border.left, Lp::calc(CalcRef::new(&owner)));
    assert!(owner.non_negative());
}
