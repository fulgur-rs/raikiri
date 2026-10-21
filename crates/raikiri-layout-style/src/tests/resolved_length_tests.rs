use super::*;
use taffy::{Dimension, LengthPercentage as Lp, LengthPercentageAuto as Lpa};

#[test]
fn resolved_pixels_preserve_finite_used_values_and_input_policy() {
    for (input, authored, resolved) in [
        (20_000_000.0, 10_000_000.0, 20_000_000.0),
        (-20_000_000.0, -10_000_000.0, -20_000_000.0),
        (0.0, 0.0, 0.0),
        (-0.0, -0.0, -0.0),
        (f32::NAN, 0.0, 0.0),
        (f32::INFINITY, 10_000_000.0, 10_000_000.0),
        (f32::NEG_INFINITY, -10_000_000.0, -10_000_000.0),
    ] {
        for (value, expected) in [
            (LengthValue::computed(L::Px(input)).unwrap(), authored),
            (LengthValue::resolved(input), resolved),
        ] {
            assert_eq!(value.dimension(), Dimension::length(expected));
            assert_eq!(value.auto(false), Lpa::length(expected));
            assert_eq!(value.auto(true), Lpa::length(expected));
            for basis in [PercentBasis::Taffy, PercentBasis::Definite(200.0)] {
                assert_eq!(value.lp(basis), Lp::length(expected));
            }
            if input == 0.0 {
                assert_eq!(
                    value.dimension().into_raw().value().to_bits(),
                    expected.to_bits()
                );
            }
        }
    }
}

#[test]
fn resolved_owned_and_borrowed_values_keep_physical_axes_until_projection() {
    let cv = ComputedValues::initial();
    let prepared = PreparedStyle::new(&cv, StyleMetadata::default(), &[]).unwrap();
    let owned = [
        OwnedPropertyOverride::length(LengthProperty::Width, L::Px(20_000_000.0), true).unwrap(),
        OwnedPropertyOverride::length(LengthProperty::Height, L::Px(30_000_000.0), true).unwrap(),
        OwnedPropertyOverride::length(LengthProperty::MinWidth, L::Px(40_000_000.0), true).unwrap(),
    ];
    let borrowed = owned.each_ref().map(OwnedPropertyOverride::as_borrowed);
    let direct = [
        PropertyOverride::length(LengthProperty::Width, LengthValue::resolved(20_000_000.0))
            .unwrap(),
        PropertyOverride::length(LengthProperty::Height, LengthValue::resolved(30_000_000.0))
            .unwrap(),
        PropertyOverride::length(
            LengthProperty::MinWidth,
            LengthValue::resolved(40_000_000.0),
        )
        .unwrap(),
    ];
    let mut legacy = taffy::Style::<raikiri_style::SmolStr>::default();
    let _undo: Vec<_> = owned
        .iter()
        .map(|entry| entry.apply_legacy_sizing(&mut legacy).unwrap())
        .collect();
    assert_eq!(legacy.size.width, Dimension::length(20_000_000.0));
    assert_eq!(legacy.size.height, Dimension::length(30_000_000.0));
    assert_eq!(legacy.min_size.width, Lpa::length(40_000_000.0));
    for overrides in [&borrowed[..], &direct[..]] {
        let context = ViewContext {
            layout_wm: raikiri_style::property::WritingMode::VerticalRl,
            percent_basis: PercentBasis::Definite(200.0),
            ..ViewContext::default()
        };
        let view = LayoutStyleView::new(&prepared, context, &[], overrides);
        assert_eq!(view.size().width, Dimension::length(30_000_000.0));
        assert_eq!(view.size().height, Dimension::length(20_000_000.0));
        assert_eq!(view.min_size().height, Lpa::length(40_000_000.0));
    }
    let edges = [PropertyOverride::length(
        LengthProperty::PaddingTop,
        LengthValue::resolved(20_000_000.0),
    )
    .unwrap()];
    let view = LayoutStyleView::new(
        &prepared,
        ViewContext {
            percent_basis: PercentBasis::Definite(200.0),
            ..ViewContext::default()
        },
        &[],
        &edges,
    );
    assert_eq!(view.padding().top, Lp::length(20_000_000.0));
}
