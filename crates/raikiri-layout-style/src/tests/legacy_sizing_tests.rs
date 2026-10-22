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
fn legacy_sizing_membership_matches_the_accepted_properties() {
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
        assert!(entry.is_legacy_sizing(), "{property:?}");
        assert!(entry.apply_legacy_sizing(&mut Style::default()).is_ok());
    }
    for property in [MarginLeft, PaddingTop, Top, FlexBasis, ColumnGap] {
        let entry = OwnedPropertyOverride::length(property, L::Px(3.0), true).unwrap();
        assert!(!entry.is_legacy_sizing(), "{property:?}");
    }
    assert!(OwnedPropertyOverride::flex_shrink(0.0).is_legacy_sizing());
    assert!(OwnedPropertyOverride::aspect_ratio(Some(2.0)).is_legacy_sizing());
    assert!(
        !OwnedPropertyOverride::clear(raikiri_style::property::ClearValue::Both).is_legacy_sizing()
    );
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
            .apply_transposed_legacy_sizing(&mut style, false)
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
            .apply_transposed_legacy_sizing(&mut style, false)
            .unwrap();
        assert_eq!(style.aspect_ratio, ratio.map(|r| 1.0 / r));
        undo.restore(&mut style);
        assert_eq!(style.aspect_ratio, None);
    }
}

#[test]
fn transposed_legacy_sizing_rejects_unrelated_entries_without_mutation() {
    let mut style = Style::<raikiri_style::SmolStr> {
        aspect_ratio: Some(3.0),
        ..Style::default()
    };
    for entry in [
        OwnedPropertyOverride::length(LengthProperty::MarginLeft, L::Px(7.0), true).unwrap(),
        OwnedPropertyOverride::clear(raikiri_style::property::ClearValue::None),
    ] {
        assert!(matches!(
            entry.apply_transposed_legacy_sizing(&mut style, false),
            Err(OverrideError::UnsupportedLegacySizing)
        ));
        assert_eq!(style.aspect_ratio, Some(3.0));
        assert!(style.size.width.is_auto());
        assert!(style.size.height.is_auto());
        assert_eq!(style.flex_shrink, 1.0);
        assert_eq!(style.margin.left, Lpa::length(0.0));
    }
}

#[test]
fn scope_projection_restores_border_and_shrink_fields() {
    use taffy::LengthPercentage as Lp;
    let mut style = Style::<raikiri_style::SmolStr> {
        border: taffy::Rect {
            top: Lp::length(1.0),
            right: Lp::length(2.0),
            bottom: Lp::length(3.0),
            left: Lp::length(4.0),
        },
        flex_shrink: 0.0,
        ..Style::default()
    };
    let original = style.border;
    let mut undos = Vec::new();
    for (property, px) in [
        (LengthProperty::BorderTop, 11.0),
        (LengthProperty::BorderRight, 12.0),
        (LengthProperty::BorderBottom, 13.0),
        (LengthProperty::BorderLeft, 14.0),
    ] {
        undos.push(
            OwnedPropertyOverride::length(property, L::Px(px), true)
                .unwrap()
                .apply_legacy_sizing(&mut style)
                .unwrap(),
        );
    }
    undos.push(
        OwnedPropertyOverride::flex_shrink(1.0)
            .apply_legacy_sizing(&mut style)
            .unwrap(),
    );
    assert_eq!(
        style.border,
        taffy::Rect {
            top: Lp::length(11.0),
            right: Lp::length(12.0),
            bottom: Lp::length(13.0),
            left: Lp::length(14.0)
        }
    );
    assert_eq!(style.flex_shrink, 1.0);
    style.margin.left = Lpa::length(91.0);
    for undo in undos.into_iter().rev() {
        undo.restore(&mut style);
    }
    assert_eq!(style.border, original);
    assert_eq!(style.flex_shrink, 0.0);
    assert_eq!(style.margin.left, Lpa::length(91.0));
}

#[test]
fn scope_projection_keeps_border_calc_owner_identity() {
    use taffy::LengthPercentage as Lp;
    let mut style = Style::<raikiri_style::SmolStr>::default();
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
    let undo = entry.apply_legacy_sizing(&mut style).unwrap();
    drop(entry);
    assert_eq!(style.border.left, Lp::calc(owner.pointer()));
    assert!(owner.non_negative());
    undo.restore(&mut style);
    assert_eq!(style.border.left, Lp::length(0.0));
}

#[test]
fn scope_projection_transposes_all_edges() {
    use taffy::LengthPercentage as Lp;
    for right_to_left in [false, true] {
        for property in [
            LengthProperty::BorderTop,
            LengthProperty::BorderRight,
            LengthProperty::BorderBottom,
            LengthProperty::BorderLeft,
        ] {
            let mut style = Style::<raikiri_style::SmolStr>::default();
            let original = style.border;
            let undo = OwnedPropertyOverride::length(property, L::Px(17.0), true)
                .unwrap()
                .apply_transposed_legacy_sizing(&mut style, right_to_left)
                .unwrap();
            let edge = match property {
                LengthProperty::BorderTop => style.border.left,
                LengthProperty::BorderBottom => style.border.right,
                LengthProperty::BorderLeft if right_to_left => style.border.bottom,
                LengthProperty::BorderRight if right_to_left => style.border.top,
                LengthProperty::BorderLeft => style.border.top,
                LengthProperty::BorderRight => style.border.bottom,
                _ => unreachable!(),
            };
            assert_eq!(edge, Lp::length(17.0));
            undo.restore(&mut style);
            assert_eq!(style.border, original);
        }
        let mut style = Style::<raikiri_style::SmolStr>::default();
        let undo = OwnedPropertyOverride::flex_shrink(0.0)
            .apply_transposed_legacy_sizing(&mut style, right_to_left)
            .unwrap();
        assert_eq!(style.flex_shrink, 0.0);
        undo.restore(&mut style);
        assert_eq!(style.flex_shrink, 1.0);
    }
}
