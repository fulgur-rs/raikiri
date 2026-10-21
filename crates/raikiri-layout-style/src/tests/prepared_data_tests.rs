use super::*;
use std::sync::Arc;

fn width_values(px: f32) -> ComputedValues {
    let mut values = ComputedValues::initial();
    values.width = L::Calc(CalcLengthPercentage { px, percent: 25.0 });
    values
}

fn data(values: &ComputedValues) -> Arc<PreparedData> {
    Arc::new(
        PreparedData::prepare_with(values, |_, expression, range| {
            Arc::new(AlignedCalc::new(expression, range))
        })
        .unwrap(),
    )
}

#[test]
fn from_data_keeps_calc_owners() {
    let values = width_values(5.0);
    let owner = Arc::new(AlignedCalc::new(
        CalcLengthPercentage {
            px: 5.0,
            percent: 25.0,
        },
        true,
    ));
    let weak = Arc::downgrade(&owner);
    let data = Arc::new(
        PreparedData::prepare_with(&values, |slot, _, range| {
            assert_eq!(slot, CalcSlot::Property(LengthProperty::Width));
            assert!(range);
            owner.clone()
        })
        .unwrap(),
    );
    let prepared =
        PreparedStyle::from_data(&values, StyleMetadata::default(), data.clone()).unwrap();
    let view = LayoutStyleView::new(&prepared, ViewContext::default(), &[], &[]);
    assert_eq!(
        view.size().width.into_raw().calc_value(),
        Arc::as_ptr(&owner).cast()
    );
    drop(owner);
    drop(data);
    let retained = weak.upgrade().unwrap();
    assert_eq!(
        (retained.resolve(0.0), retained.resolve(100.0)),
        (5.0, 30.0)
    );
    drop(retained);
    drop(prepared);
    assert!(weak.upgrade().is_none());
}

#[test]
fn from_data_rejects_wrong_values() {
    let values = width_values(5.0);
    let data = data(&values);
    let wrong = width_values(6.0);
    assert!(matches!(
        PreparedStyle::from_data(&wrong, StyleMetadata::default(), data.clone()),
        Err(PrepareError::Mismatch(CalcSlot::Property(
            LengthProperty::Width
        )))
    ));
    let initial = ComputedValues::initial();
    assert!(matches!(
        PreparedStyle::from_data(&initial, StyleMetadata::default(), data),
        Err(PrepareError::Unexpected)
    ));
    let empty = self::data(&initial);
    assert!(matches!(
        PreparedStyle::from_data(&values, StyleMetadata::default(), empty),
        Err(PrepareError::Missing(CalcSlot::Property(
            LengthProperty::Width
        )))
    ));
}

#[test]
fn from_shared_data_keeps_values() {
    let values = Arc::new(width_values(5.0));
    let weak = Arc::downgrade(&values);
    let data = data(&values);
    let prepared =
        PreparedStyle::from_shared_data(values.clone(), StyleMetadata::default(), data.clone())
            .unwrap();
    drop(values);
    drop(data);
    assert!(weak.upgrade().is_some());
    let payload = prepared.payload(CalcSlot::Property(LengthProperty::Width));
    assert_eq!((payload.resolve(0.0), payload.resolve(100.0)), (5.0, 30.0));
    drop(prepared);
    assert!(weak.upgrade().is_none());
}

#[test]
fn shared_data_binding_rejects_another_expression() {
    let values = width_values(5.0);
    assert!(matches!(
        PreparedStyle::from_shared_data(
            Arc::new(width_values(6.0)),
            StyleMetadata::default(),
            data(&values),
        ),
        Err(PrepareError::Mismatch(CalcSlot::Property(
            LengthProperty::Width
        )))
    ));
}

#[test]
fn data_does_not_retain_computed_values() {
    let values = Arc::new(width_values(5.0));
    let weak_values = Arc::downgrade(&values);
    let owner = Arc::new(AlignedCalc::new(
        CalcLengthPercentage {
            px: 5.0,
            percent: 25.0,
        },
        true,
    ));
    let weak_owner = Arc::downgrade(&owner);
    let data = PreparedData::prepare_with(&values, |_, _, _| owner.clone()).unwrap();
    drop(values);
    drop(owner);
    assert!(weak_values.upgrade().is_none());
    assert_eq!(weak_owner.upgrade().unwrap().resolve(100.0), 30.0);
    drop(data);
    assert!(weak_owner.upgrade().is_none());
}

#[test]
fn data_binding_checks_exact_zero_and_nan_bits() {
    for (correct, wrong) in [
        (-0.0, 0.0),
        (f32::from_bits(0x7fc0_0001), f32::from_bits(0x7fc0_0002)),
    ] {
        let values = width_values(correct);
        let data = data(&values);
        assert!(PreparedStyle::from_data(&values, StyleMetadata::default(), data.clone()).is_ok());
        let wrong = width_values(wrong);
        assert!(matches!(
            PreparedStyle::from_data(&wrong, StyleMetadata::default(), data),
            Err(PrepareError::Mismatch(CalcSlot::Property(
                LengthProperty::Width
            )))
        ));
    }
}

#[test]
fn data_factory_rejects_wrong_range_and_expression_bits() {
    let values = width_values(-0.0);
    for owner in [
        AlignedCalc::new(
            CalcLengthPercentage {
                px: -0.0,
                percent: 25.0,
            },
            false,
        ),
        AlignedCalc::new(
            CalcLengthPercentage {
                px: 0.0,
                percent: 25.0,
            },
            true,
        ),
    ] {
        let owner = Arc::new(owner);
        assert!(matches!(
            PreparedData::prepare_with(&values, |_, _, _| owner.clone()),
            Err(PrepareError::Mismatch(CalcSlot::Property(
                LengthProperty::Width
            )))
        ));
    }
}

#[test]
fn shared_data_does_not_share_box_metadata() {
    let values = width_values(5.0);
    let data = data(&values);
    let visible =
        PreparedStyle::from_data(&values, StyleMetadata::default(), data.clone()).unwrap();
    let hidden = PreparedStyle::from_data(
        &values,
        StyleMetadata {
            flags: StyleFlags::HIDDEN,
            ..StyleMetadata::default()
        },
        data,
    )
    .unwrap();
    let visible = LayoutStyleView::new(&visible, ViewContext::default(), &[], &[]);
    let hidden = LayoutStyleView::new(&hidden, ViewContext::default(), &[], &[]);
    assert_eq!(
        visible.box_generation_mode(),
        taffy::BoxGenerationMode::Normal
    );
    assert_eq!(hidden.box_generation_mode(), taffy::BoxGenerationMode::None);
    assert_eq!(
        visible.size().width.into_raw().calc_value(),
        hidden.size().width.into_raw().calc_value()
    );
}
