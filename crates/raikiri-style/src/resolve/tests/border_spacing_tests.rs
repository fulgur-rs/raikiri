use super::*;
use crate::property::BorderSpacingValue;

fn spacing(horizontal: f32, vertical: f32, scale: Option<f32>) -> ComputedBorderSpacing {
    resolve_border_spacing(
        BorderSpacingValue::new(Length::Px(horizontal), Length::Px(vertical)),
        ComputedLength(10.0),
        None,
        &CTX.with_border_width_device_scale(scale),
    )
}

#[test]
fn raster_spacing_has_zero_and_near_integer_device_boundaries() {
    for (value, expected) in [
        (0.7, 0.0),
        (0.989, 0.0),
        (0.99, 1.0),
        (1.5, 1.0),
        (4.9899, 4.0),
        (4.99, 5.0),
        (8.99, 9.0),
    ] {
        assert_eq!(spacing(value, value, Some(1.0)).horizontal.px(), expected);
        assert_eq!(spacing(value, value, None).horizontal.px(), value);
    }
    assert_eq!(spacing(0.7, 2.7, Some(2.0)).serialized(), "0.5px 2.5px");
}

#[test]
fn lifted_spacing_keeps_each_computed_component_without_snapping_again() {
    let ctx = CTX.with_border_width_device_scale(Some(1.1));
    let first = spacing(747_714.8, 4.99, Some(1.1));
    assert_eq!(first.horizontal.px(), 747_714.5);
    let mut current = first;
    for _ in 0..64 {
        current = resolve_border_spacing(
            lift_border_spacing(current),
            ComputedLength(10.0),
            None,
            &ctx,
        );
        assert_eq!(current, first);
    }
    let mut edited = lift_border_spacing(first);
    edited.vertical = Length::Px(0.7);
    let result = resolve_border_spacing(edited, ComputedLength(10.0), None, &ctx);
    assert_eq!(result.horizontal, first.horizontal);
    assert_eq!(result.vertical.px(), 0.0);
    edited.horizontal = Length::Em(0.17);
    let result = resolve_border_spacing(edited, ComputedLength(10.0), None, &ctx);
    assert_eq!(result.horizontal, spacing(1.7, 0.0, Some(1.1)).horizontal);
    assert_eq!(result.vertical.px(), 0.0);
}

#[test]
fn raster_spacing_does_not_turn_finite_lengths_into_infinity() {
    for scale in [f32::MIN_POSITIVE, f32::MAX] {
        let value = spacing(f32::MAX, 0.0, Some(scale));
        assert_eq!(value.horizontal.px(), f32::MAX);
        assert_eq!(value.vertical.px(), 0.0);
    }
    for scale in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        let ctx = ResolveContext {
            border_width_device_scale: Some(scale),
            ..CTX
        };
        let value = resolve_border_spacing(
            BorderSpacingValue::new(Length::Px(0.7), Length::Px(1.9)),
            ComputedLength(10.0),
            None,
            &ctx,
        );
        assert_eq!(value.serialized(), "0.7px 1.9px");
    }
}
