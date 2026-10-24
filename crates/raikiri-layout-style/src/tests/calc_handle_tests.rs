use super::*;
use crate::values::{CalcRef, ExpandedDimension, ExpandedLengthPercentageAuto};
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

/// The calc reference `view` returns for its width.
fn width_calc_ref(view: &LayoutStyleView<'_>) -> CalcRef {
    match view.size().width.expand() {
        ExpandedDimension::Calc(calc) => calc,
        other => panic!("the width is a calc, not {other:?}"),
    }
}

/// The calc reference `view` returns for its top margin.
fn margin_top_calc_ref(view: &LayoutStyleView<'_>) -> CalcRef {
    match view.margin().top.expand() {
        ExpandedLengthPercentageAuto::Calc(calc) => calc,
        other => panic!("the top margin is a calc, not {other:?}"),
    }
}

#[test]
fn resolves_the_references_of_every_kind_of_binding() {
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
        let calc = width_calc_ref(&view);
        assert_eq!(calc.resolve(0.0), 5.0);
        assert_eq!(calc.resolve(100.0), 30.0);
    }
}

#[test]
fn a_reference_is_the_payload_the_getter_read() {
    let values = width_calc();
    let payload = AlignedCalc::new(EXPRESSION, true);
    let bindings = [PreparedCalc {
        slot: CalcSlot::Property(LengthProperty::Width),
        payload: &payload,
    }];
    let prepared = PreparedStyle::new(&values, StyleMetadata::default(), &bindings).unwrap();
    let view = LayoutStyleView::new(&prepared, ViewContext::default(), &[], &[]);
    assert_eq!(width_calc_ref(&view), CalcRef::new(&payload));
    assert_eq!(width_calc_ref(&view).addr(), payload.pointer().addr());
}

#[test]
fn resolves_the_reference_of_an_override() {
    let values = ComputedValues::initial();
    let prepared = PreparedStyle::prepare(&values, StyleMetadata::default());
    let payload = AlignedCalc::new(EXPRESSION, true);
    let pass =
        [PropertyOverride::length(LengthProperty::Width, LengthValue::calc(&payload)).unwrap()];
    let view = LayoutStyleView::new(&prepared, ViewContext::default(), &pass, &[]);
    assert_eq!(width_calc_ref(&view).resolve(200.0), 55.0);
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
    assert_eq!(width_calc_ref(&view).resolve(100.0), 30.0);
    assert_eq!(margin_top_calc_ref(&view).resolve(100.0), 1.0);
}
