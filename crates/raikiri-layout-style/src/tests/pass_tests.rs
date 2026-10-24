use super::*;
use raikiri_style::property::ClearValue;

#[test]
fn clear_override_has_pass_and_scope_precedence() {
    let mut values = ComputedValues::initial();
    values.clear = ClearValue::Left;
    let prepared = PreparedStyle::prepare(&values, StyleMetadata::default());
    let pass = [OwnedPropertyOverride::clear(ClearValue::Both)];
    let scoped = [OwnedPropertyOverride::clear(ClearValue::None)];
    let base = LayoutStyleView::with_owned_overrides(&prepared, ViewContext::default(), &[], &[]);
    assert_eq!(base.clear(), taffy::Clear::Left);
    let view = LayoutStyleView::with_owned_overrides(&prepared, ViewContext::default(), &pass, &[]);
    assert_eq!(view.clear(), taffy::Clear::Both);
    let view =
        LayoutStyleView::with_owned_overrides(&prepared, ViewContext::default(), &pass, &scoped);
    assert_eq!(view.clear(), taffy::Clear::None);
}

#[test]
fn every_physical_override_reaches_its_getter() {
    use taffy::{Dimension, LengthPercentage as Lp, LengthPercentageAuto as Lpa};
    let values = ComputedValues::initial();
    let prepared = PreparedStyle::prepare(&values, StyleMetadata::default());
    let mut entries = Vec::new();
    for property in LengthProperty::ALL {
        if let Ok(entry) = OwnedPropertyOverride::length(*property, L::Px(17.0), true) {
            entries.push(entry);
        }
    }
    entries.extend([
        OwnedPropertyOverride::aspect_ratio(Some(2.0)),
        OwnedPropertyOverride::aspect_ratio(None),
        OwnedPropertyOverride::overflow(Axis::X, raikiri_style::property::OverflowValue::Hidden),
        OwnedPropertyOverride::overflow(Axis::Y, raikiri_style::property::OverflowValue::Scroll),
        OwnedPropertyOverride::clear(ClearValue::Both),
        OwnedPropertyOverride::flex_shrink(0.5),
        OwnedPropertyOverride::align_self(Some(taffy::AlignSelf::START)),
        OwnedPropertyOverride::justify_self(Some(taffy::AlignSelf::END)),
    ]);
    let mut pass = Vec::new();
    for entry in entries {
        entry.install(&mut pass);
    }
    // Each property keeps its last entry only.
    assert_eq!(pass.len(), LengthProperty::ALL.len() - 3 + 7);
    let view = LayoutStyleView::with_owned_overrides(&prepared, ViewContext::default(), &pass, &[]);
    let dimension = Dimension::length(17.0);
    let length = Lp::length(17.0);
    let length_or_auto = Lpa::length(17.0);
    assert_eq!(
        view.size(),
        taffy::Size {
            width: dimension,
            height: dimension
        }
    );
    assert_eq!(
        view.min_size(),
        taffy::Size {
            width: length_or_auto,
            height: length_or_auto
        }
    );
    assert_eq!(
        view.max_size(),
        taffy::Size {
            width: length_or_auto,
            height: length_or_auto
        }
    );
    fn sides<T: Copy>(side: T) -> taffy::Rect<T> {
        taffy::Rect {
            left: side,
            right: side,
            top: side,
            bottom: side,
        }
    }
    assert_eq!(view.margin(), sides(length_or_auto));
    assert_eq!(view.padding(), sides(length));
    assert_eq!(view.border(), sides(length));
    assert_eq!(view.inset(), sides(length_or_auto));
    assert_eq!(view.flex_basis(), dimension);
    assert_eq!(
        view.gap(),
        taffy::Size {
            width: length,
            height: length
        }
    );
    assert_eq!(view.aspect_ratio(), None);
    assert_eq!(
        view.overflow(),
        taffy::Point {
            x: taffy::Overflow::Hidden,
            y: taffy::Overflow::Scroll
        }
    );
    assert_eq!(view.clear(), taffy::Clear::Both);
    assert_eq!(view.flex_shrink(), 0.5);
    assert_eq!(view.align_self(), Some(taffy::AlignSelf::START));
    assert_eq!(view.bridge_justify_self(), Some(taffy::AlignSelf::END));
}

#[test]
fn flex_shrink_getter_preserves_the_override_value() {
    let values = ComputedValues::initial();
    let prepared = PreparedStyle::prepare(&values, StyleMetadata::default());
    for value in [-1.0, f32::NAN, f32::INFINITY, 1.0e8] {
        let entries = [OwnedPropertyOverride::flex_shrink(value)];
        let view =
            LayoutStyleView::with_owned_overrides(&prepared, ViewContext::default(), &entries, &[]);
        assert_eq!(view.flex_shrink().to_bits(), value.to_bits());
    }
}

#[test]
fn clearance_covers_each_authored_side() {
    for (value, expected) in [
        (ClearValue::None, taffy::Clear::None),
        (ClearValue::Left, taffy::Clear::Left),
        (ClearValue::Right, taffy::Clear::Right),
        (ClearValue::Both, taffy::Clear::Both),
    ] {
        let mut values = ComputedValues::initial();
        values.clear = value;
        let prepared = PreparedStyle::prepare(&values, StyleMetadata::default());
        let view =
            LayoutStyleView::with_owned_overrides(&prepared, ViewContext::default(), &[], &[]);
        assert_eq!(view.clear(), expected);
    }
}
