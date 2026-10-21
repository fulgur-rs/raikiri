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
fn retained_override_projection_matches_getters_for_every_physical_property() {
    use taffy::GridItemStyle;
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
    let mut style = taffy::Style::<raikiri_style::SmolStr>::default();
    for entry in entries {
        entry.apply_to_style(&mut style);
        entry.install(&mut pass);
        let view =
            LayoutStyleView::with_owned_overrides(&prepared, ViewContext::default(), &pass, &[]);
        assert_eq!(view.size(), style.size);
        assert_eq!(view.min_size(), style.min_size);
        assert_eq!(view.max_size(), style.max_size);
        assert_eq!(view.margin(), style.margin);
        assert_eq!(view.padding(), style.padding);
        assert_eq!(view.border(), style.border);
        assert_eq!(view.inset(), style.inset);
        assert_eq!(view.flex_basis(), style.flex_basis);
        assert_eq!(view.gap(), style.gap);
        assert_eq!(view.aspect_ratio(), style.aspect_ratio);
        assert_eq!(view.overflow(), style.overflow);
        assert_eq!(view.clear(), style.clear);
        assert_eq!(view.flex_shrink(), style.flex_shrink);
        assert_eq!(FlexboxItemStyle::align_self(&view), style.align_self);
        assert_eq!(view.justify_self(), style.justify_self);
    }
    assert_eq!(pass.len(), LengthProperty::ALL.len() - 3 + 7);
}

#[test]
fn flex_shrink_projection_preserves_the_getter_override_value() {
    let values = ComputedValues::initial();
    let prepared = PreparedStyle::prepare(&values, StyleMetadata::default());
    for value in [-1.0, f32::NAN, f32::INFINITY, 1.0e8] {
        let entries = [OwnedPropertyOverride::flex_shrink(value)];
        let view =
            LayoutStyleView::with_owned_overrides(&prepared, ViewContext::default(), &entries, &[]);
        let mut style = taffy::Style::default();
        entries[0].apply_to_style(&mut style);
        assert_eq!(view.flex_shrink().to_bits(), style.flex_shrink.to_bits());
    }
}

#[test]
fn clearance_projection_covers_each_authored_side() {
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
        let mut style = taffy::Style::default();
        OwnedPropertyOverride::clear(value).apply_to_style(&mut style);
        assert_eq!(style.clear, expected);
    }
}
