use super::*;
use std::sync::Arc;

const EXPRESSION: CalcLengthPercentage = CalcLengthPercentage {
    px: 5.0,
    percent: 25.0,
};

fn width_calc() -> ComputedValues {
    let mut values = ComputedValues::initial();
    values.width = L::Calc(EXPRESSION);
    values
}

/// The handle `view` returns for its calc width.
fn width_handle(view: &LayoutStyleView<'_>) -> *const () {
    view.size().width.into_raw().calc_value()
}

#[test]
fn resolves_the_handles_of_every_kind_of_binding() {
    let values = width_calc();
    let payload = AlignedCalc::new(EXPRESSION, true);
    let bindings = [PreparedCalc {
        slot: CalcSlot::Property(LengthProperty::Width),
        payload: &payload,
    }];
    let borrowed = PreparedStyle::new(&values, StyleMetadata::default(), &bindings).unwrap();
    let owned = PreparedStyle::prepare(&values, StyleMetadata::default());
    let data = Arc::new(
        PreparedData::prepare_with(&values, |_, expression, range| {
            Arc::new(AlignedCalc::new(expression, range))
        })
        .unwrap(),
    );
    let shared = PreparedStyle::from_data(&values, StyleMetadata::default(), data).unwrap();
    for prepared in [&borrowed, &owned, &shared] {
        let view = LayoutStyleView::new(prepared, ViewContext::default(), &[], &[]);
        let handle = width_handle(&view);
        assert_eq!(view.resolve_calc_handle(handle, 0.0), Some(5.0));
        assert_eq!(view.resolve_calc_handle(handle, 100.0), Some(30.0));
    }
}

#[test]
fn resolves_the_handle_of_an_override() {
    let values = ComputedValues::initial();
    let prepared = PreparedStyle::prepare(&values, StyleMetadata::default());
    let payload = AlignedCalc::new(EXPRESSION, true);
    let pass =
        [PropertyOverride::length(LengthProperty::Width, LengthValue::calc(&payload)).unwrap()];
    let view = LayoutStyleView::new(&prepared, ViewContext::default(), &pass, &[]);
    let handle = width_handle(&view);
    assert_eq!(view.resolve_calc_handle(handle, 200.0), Some(55.0));
}

#[test]
fn finds_prepared_payloads_past_overrides_of_other_properties() {
    // The override of the top margin brings a payload of its own; the width
    // still resolves through the prepared one, and the margin through the
    // override.
    let values = width_calc();
    let prepared = PreparedStyle::prepare(&values, StyleMetadata::default());
    let margin = AlignedCalc::new(
        CalcLengthPercentage {
            px: 1.0,
            percent: 0.0,
        },
        false,
    );
    let pass = [
        PropertyOverride::length(LengthProperty::MarginTop, LengthValue::calc(&margin)).unwrap(),
        PropertyOverride::aspect_ratio(Some(2.0)),
    ];
    let view = LayoutStyleView::new(&prepared, ViewContext::default(), &pass, &[]);
    assert_eq!(
        view.resolve_calc_handle(width_handle(&view), 100.0),
        Some(30.0)
    );
    let margin_handle = view.margin().top.into_raw().calc_value();
    assert_eq!(view.resolve_calc_handle(margin_handle, 100.0), Some(1.0));
}

#[test]
fn a_foreign_address_resolves_to_nothing() {
    let values = width_calc();
    let prepared = PreparedStyle::prepare(&values, StyleMetadata::default());
    let view = LayoutStyleView::new(&prepared, ViewContext::default(), &[], &[]);
    let stranger = AlignedCalc::new(EXPRESSION, true);
    assert_eq!(view.resolve_calc_handle(stranger.pointer(), 100.0), None);
    assert_eq!(view.resolve_calc_handle(std::ptr::null(), 100.0), None);
}
