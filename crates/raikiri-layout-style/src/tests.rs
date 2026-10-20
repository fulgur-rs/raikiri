use super::*;
use raikiri_style::{ComputedValues, property::CalcLengthPercentage};

#[test]
fn calc_respects_property_range() {
    let value = CalcLengthPercentage { px: -10.0, percent: 25.0 };
    let signed = AlignedCalc::new(value, false);
    let positive = AlignedCalc::new(value, true);
    assert_eq!((signed.resolve(0.0), signed.resolve(100.0)), (-10.0, 15.0));
    assert_eq!((positive.resolve(0.0), positive.resolve(100.0)), (0.0, 15.0));
    assert!(std::mem::align_of::<AlignedCalc>() >= 8);
}

#[test]
fn prepared_style_rejects_wrong_bindings() {
    let mut cv = ComputedValues::initial();
    let expr = CalcLengthPercentage { px: 2.0, percent: 10.0 };
    cv.width = raikiri_style::ComputedLengthPercentageOrAuto::Calc(expr);
    let metadata = StyleMetadata::default();
    assert!(PreparedStyle::new(&cv, metadata, &[]).is_err());
    let payload = AlignedCalc::new(expr, true);
    let binding = PreparedCalc { slot: CalcSlot::Property(LengthProperty::Width), payload: &payload };
    assert!(PreparedStyle::new(&cv, metadata, &[binding]).is_ok());
    assert!(PreparedStyle::new(&cv, metadata, &[binding, binding]).is_err());
    let signed = AlignedCalc::new(expr, false);
    assert!(PreparedStyle::new(&cv, metadata, &[PreparedCalc { payload: &signed, ..binding }]).is_err());
    let wrong = AlignedCalc::new(CalcLengthPercentage { px: 3.0, ..expr }, true);
    assert!(PreparedStyle::new(&cv, metadata, &[PreparedCalc { payload: &wrong, ..binding }]).is_err());
}
