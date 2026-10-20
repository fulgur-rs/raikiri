use super::*;
use raikiri_style::ComputedLengthPercentageOrAuto as L;
use raikiri_style::{ComputedValues, property::CalcLengthPercentage};
use taffy::{CoreStyle, FlexboxContainerStyle, FlexboxItemStyle};
mod core_tests;
mod grid_tests;
mod policy_tests;
mod pr_review_tests;
mod review_tests;

#[test]
fn initial_values_match_bridge() {
    let cv = ComputedValues::initial();
    let p = PreparedStyle::new(&cv, StyleMetadata::default(), &[]).unwrap();
    let v = LayoutStyleView::new(&p, ViewContext::default(), &[], &[]);
    assert_eq!(v.box_sizing(), taffy::BoxSizing::ContentBox);
    assert_eq!(v.flex_grow(), 0.0);
    assert_eq!(v.flex_shrink(), 1.0);
    assert_eq!(v.flex_basis(), taffy::Dimension::auto());
    assert_eq!(v.gap(), taffy::Size::zero());
}

#[test]
fn scoped_overrides_distinguish_absence_and_last_wins() {
    let mut cv = ComputedValues::initial();
    cv.width = L::Px(100.0);
    let p = PreparedStyle::new(&cv, StyleMetadata::default(), &[]).unwrap();
    let length = |value| LengthValue::computed(value).unwrap();
    let pass = [
        PropertyOverride::length(LengthProperty::Width, length(L::Px(10.0))).unwrap(),
        PropertyOverride::length(LengthProperty::Width, length(L::Px(30.0))).unwrap(),
        PropertyOverride::aspect_ratio(Some(2.0)),
    ];
    let plain = LayoutStyleView::new(&p, ViewContext::default(), &pass, &[]);
    assert_eq!(plain.size().width, taffy::Dimension::length(30.0));
    assert_eq!(plain.aspect_ratio(), Some(2.0));
    let scoped = [
        PropertyOverride::length(LengthProperty::Width, length(L::Auto)).unwrap(),
        PropertyOverride::aspect_ratio(None),
    ];
    let v = LayoutStyleView::new(&p, ViewContext::default(), &pass, &scoped);
    assert_eq!(v.size().width, taffy::Dimension::auto());
    assert_eq!(v.aspect_ratio(), None);
}

#[test]
fn edge_resolution_is_not_repeated_and_axes_are_projected() {
    let mut cv = ComputedValues::initial();
    cv.width = L::Px(20.0);
    cv.height = L::Px(40.0);
    cv.padding.top = raikiri_style::ComputedLengthPercentageOrCalc::Percent(10.0);
    let p = PreparedStyle::new(&cv, StyleMetadata::default(), &[]).unwrap();
    let overrides =
        [
            PropertyOverride::length(LengthProperty::PaddingBottom, LengthValue::resolved(7.0))
                .unwrap(),
        ];
    let context = ViewContext {
        percent_basis: PercentBasis::Definite(200.0),
        ..ViewContext::default()
    };
    let v = LayoutStyleView::new(&p, context, &overrides, &[]);
    assert_eq!(v.padding().top, taffy::LengthPercentage::length(20.0));
    assert_eq!(v.padding().bottom, taffy::LengthPercentage::length(7.0));
    let context = ViewContext {
        layout_wm: raikiri_style::property::WritingMode::VerticalRl,
        ..context
    };
    let v = LayoutStyleView::new(&p, context, &overrides, &[]);
    assert_eq!(
        v.size(),
        taffy::Size {
            width: taffy::Dimension::length(40.0),
            height: taffy::Dimension::length(20.0)
        }
    );
    assert_eq!(v.padding().left, taffy::LengthPercentage::length(20.0));
    assert_eq!(v.padding().right, taffy::LengthPercentage::length(7.0));
}

#[test]
fn calc_respects_property_range() {
    let value = CalcLengthPercentage {
        px: -10.0,
        percent: 25.0,
    };
    let signed = AlignedCalc::new(value, false);
    let positive = AlignedCalc::new(value, true);
    assert_eq!((signed.resolve(0.0), signed.resolve(100.0)), (-10.0, 15.0));
    assert_eq!(
        (positive.resolve(0.0), positive.resolve(100.0)),
        (0.0, 15.0)
    );
    assert!(std::mem::align_of::<AlignedCalc>() >= 8);
}

#[test]
fn prepared_style_rejects_wrong_bindings() {
    let mut cv = ComputedValues::initial();
    let expr = CalcLengthPercentage {
        px: 2.0,
        percent: 10.0,
    };
    cv.width = raikiri_style::ComputedLengthPercentageOrAuto::Calc(expr);
    let metadata = StyleMetadata::default();
    assert!(PreparedStyle::new(&cv, metadata, &[]).is_err());
    let payload = AlignedCalc::new(expr, true);
    let binding = PreparedCalc {
        slot: CalcSlot::Property(LengthProperty::Width),
        payload: &payload,
    };
    assert!(PreparedStyle::new(&cv, metadata, &[binding]).is_ok());
    assert!(PreparedStyle::new(&cv, metadata, &[binding, binding]).is_err());
    let signed = AlignedCalc::new(expr, false);
    assert!(
        PreparedStyle::new(
            &cv,
            metadata,
            &[PreparedCalc {
                payload: &signed,
                ..binding
            }]
        )
        .is_err()
    );
    let wrong = AlignedCalc::new(CalcLengthPercentage { px: 3.0, ..expr }, true);
    assert!(
        PreparedStyle::new(
            &cv,
            metadata,
            &[PreparedCalc {
                payload: &wrong,
                ..binding
            }]
        )
        .is_err()
    );
}
