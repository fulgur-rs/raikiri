use super::*;
use std::{borrow::Cow, sync::Arc};

#[test]
fn prepared_comparisons_keep_branches_and_apply_range_after_the_expression() {
    for (source, expected) in [
        ("min(30px, calc(10px + 5%))", [10.0, 10.5, 30.0]),
        ("max(10px, 5%)", [10.0, 10.0, 50.0]),
        ("clamp(5px, calc(10px + 5%), 20px)", [10.0, 10.5, 20.0]),
        ("calc(min(-10px, 5%) + 20px)", [10.0, 10.0, 10.0]),
        ("calc(min(-10px, 5%) - 20px)", [0.0, 0.0, 0.0]),
    ] {
        let expression = comparison(source);
        let mut values = ComputedValues::initial();
        values.width = L::Math(expression);
        let prepared = PreparedStyle::prepare(&values, StyleMetadata::default());
        let view = LayoutStyleView::new(&prepared, ViewContext::default(), &[], &[]);
        let handle = view.size().width.calc_ref();
        for (basis, expected) in [0.0, 10.0, 1000.0].into_iter().zip(expected) {
            assert_eq!(handle.resolve(basis), expected, "{source} at {basis}");
        }
    }
}

#[test]
fn prepared_bindings_reject_a_different_tree_or_range() {
    let expression = comparison("min(30px, calc(10px + 5%))");
    let mut values = ComputedValues::initial();
    values.width = L::Math(expression.clone());
    for owner in [
        AlignedCalc::new(comparison("max(30px, calc(10px + 5%))"), true),
        AlignedCalc::new(expression, false),
        AlignedCalc::new(
            CalcLengthPercentage {
                px: 10.0,
                percent: 5.0,
            },
            true,
        ),
    ] {
        let bindings = [PreparedCalc {
            slot: CalcSlot::Property(LengthProperty::Width),
            payload: &owner,
        }];
        assert!(matches!(
            PreparedStyle::new(&values, StyleMetadata::default(), &bindings),
            Err(PrepareError::Mismatch(_))
        ));
    }
}

fn borrowed_query_keeps_tree_owners(
    view: &LayoutStyleView<'_>,
    expression: &Arc<raikiri_style::property::ComputedLengthMath>,
) {
    let before = Arc::strong_count(expression);
    let length = view.length(LengthProperty::Width);
    assert!(matches!(length.value, Cow::Borrowed(_)));
    assert_eq!(Arc::strong_count(expression), before);
    assert_eq!(length.dimension().calc_ref().resolve(10.0), 10.5);
    assert_eq!(Arc::strong_count(expression), before);
}

#[test]
fn borrowed_shared_and_override_queries_borrow_the_retained_tree() {
    let expression = comparison("min(30px, calc(10px + 5%))");
    let mut values = ComputedValues::initial();
    values.width = L::Math(expression.clone());
    let values = Arc::new(values);
    let borrowed = PreparedStyle::prepare(&values, StyleMetadata::default());
    let shared = PreparedStyle::prepare_shared(values.clone(), StyleMetadata::default());
    for prepared in [&borrowed, &shared] {
        let before = Arc::strong_count(&expression);
        let scalar = prepared.scalar(LengthProperty::Width);
        assert!(matches!(scalar, Cow::Borrowed(_)));
        assert_eq!(Arc::strong_count(&expression), before);
        let view = LayoutStyleView::new(prepared, ViewContext::default(), &[], &[]);
        borrowed_query_keeps_tree_owners(&view, &expression);
    }
    let payload = AlignedCalc::new(expression.clone(), true);
    for length in [
        LengthValue::calc(&payload),
        LengthValue {
            value: Cow::Owned(L::Math(expression.clone())),
            payload: Some(&payload),
            resolved: false,
        },
    ] {
        let pass = [PropertyOverride::length(LengthProperty::Width, length).unwrap()];
        let view = LayoutStyleView::new(&borrowed, ViewContext::default(), &pass, &[]);
        borrowed_query_keeps_tree_owners(&view, &expression);
    }
    let pass =
        [
            OwnedPropertyOverride::length(
                LengthProperty::Width,
                L::Math(expression.clone()),
                false,
            )
            .unwrap(),
        ];
    let view = LayoutStyleView::with_owned_overrides(&borrowed, ViewContext::default(), &pass, &[]);
    borrowed_query_keeps_tree_owners(&view, &expression);
}

#[test]
fn unprepared_math_and_wrong_override_range_are_rejected() {
    let expression = comparison("min(30px, calc(10px + 5%))");
    assert!(matches!(
        LengthValue::computed(L::Math(expression.clone())),
        Err(OverrideError::UnpreparedCalc)
    ));
    let signed = AlignedCalc::new(expression, false);
    assert!(matches!(
        PropertyOverride::length(LengthProperty::Width, LengthValue::calc(&signed)),
        Err(OverrideError::WrongRange)
    ));
}
