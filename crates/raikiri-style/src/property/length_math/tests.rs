use super::*;
use cssparser::{Parser, ParserInput};

fn specified(source: &str) -> Option<Arc<LengthMath>> {
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    let value = LengthMath::parse(&mut parser)?;
    parser.expect_exhausted().ok()?;
    Some(value)
}

fn computed(source: &str) -> Arc<ComputedLengthMath> {
    specified(source)
        .expect("valid comparison")
        .absolutize(|value| match value {
            Length::Px(px) => Some(CalcLengthPercentage { px, percent: 0.0 }),
            Length::Percent(percent) => Some(CalcLengthPercentage { px: 0.0, percent }),
            Length::Em(em) => Some(CalcLengthPercentage {
                px: em * 16.0,
                percent: 0.0,
            }),
            _ => None,
        })
        .unwrap()
}

#[test]
fn comparison_winners_change_with_the_used_percentage_basis() {
    for (source, values) in [
        ("min(30px, calc(10px + 5%))", [10.0, 10.5, 30.0]),
        ("max(10px, 5%)", [10.0, 10.0, 50.0]),
        ("clamp(5px, calc(10px + 5%), 20px)", [10.0, 10.5, 20.0]),
    ] {
        let expression = computed(source);
        for (basis, expected) in [0.0, 10.0, 1000.0].into_iter().zip(values) {
            assert_eq!(expression.resolve(basis), expected, "{source} at {basis}");
        }
        assert!(expression.has_percentage());
    }
}

#[test]
fn nested_math_keeps_negative_intermediates_and_relative_leaves() {
    for (source, basis, expected) in [
        ("calc(min(-10px, 5%) + 20px)", 100.0, 10.0),
        ("clamp(30px, 5%, 20px)", 10.0, 30.0),
        ("calc(2 * max(1em, 10%) - min(5px, 2%))", 200.0, 36.0),
        ("max(min(10px, 20%), calc(30px / 2))", 10.0, 15.0),
        ("calc(min(2, 3) * max(1px, 20%))", 100.0, 40.0),
    ] {
        assert_eq!(computed(source).resolve(basis), expected, "{source}");
    }
}

#[test]
fn percentage_in_a_losing_branch_is_still_preserved() {
    let value = computed("min(1px, calc(10px + 5%))");
    assert_eq!(value.resolve(0.0), 1.0);
    assert!(value.has_percentage());
}

#[test]
fn specified_math_serialization_keeps_relative_units_and_branches() {
    let value = specified("calc(min(1em, 10%) + 2px)").unwrap();
    let restored = specified(&value.to_css()).unwrap();
    assert_eq!(value, restored);
}

#[test]
fn math_rejects_invalid_dimensions_and_bounds_parse_resources() {
    for source in [
        "(max(10px, 5%))",
        "min(1px, 1s)",
        "min(1px, 2)",
        "min()",
        "clamp(1px, 2%)",
        "calc(min(1px, 2%) / 0)",
        "calc(min(1px, 2%) * 1px)",
    ] {
        assert!(specified(source).is_none(), "{source}");
    }
    let nested = format!(
        "{}1px{}",
        "min(1%,".repeat(crate::MAX_CSS_NESTING_DEPTH + 2),
        ")".repeat(crate::MAX_CSS_NESTING_DEPTH + 2)
    );
    assert!(specified(&nested).is_none());
    let wide = format!("min({}1px)", "1%,".repeat(5000));
    assert!(specified(&wide).is_none());
    let valid = format!("min({}1px)", "10%,".repeat(30));
    assert_eq!(computed(&valid).resolve(100.0), 1.0);
}

#[test]
fn absolutization_rejects_nonfinite_coefficients_and_lifts_without_reparsing() {
    let value = specified("max(1px, 5%)").unwrap();
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert!(
            value
                .absolutize(|_| Some(CalcLengthPercentage {
                    px: bad,
                    percent: 0.0
                }))
                .is_none()
        );
        assert!(
            value
                .absolutize(|_| Some(CalcLengthPercentage {
                    px: 0.0,
                    percent: bad
                }))
                .is_none()
        );
    }
    let original = computed("calc(min(1em, 10%) + 2px)");
    let lifted = LengthMath::from_computed(&original);
    let restored = lifted
        .absolutize(|value| match value {
            Length::Px(px) => Some(CalcLengthPercentage { px, percent: 0.0 }),
            Length::Percent(percent) => Some(CalcLengthPercentage { px: 0.0, percent }),
            _ => None,
        })
        .unwrap();
    for basis in [0.0, 10.0, 1000.0] {
        assert_eq!(original.resolve(basis), restored.resolve(basis));
    }
}

#[test]
fn computed_hash_agrees_with_equality_for_signed_zero() {
    use std::collections::hash_map::DefaultHasher;
    let positive = computed("max(0px, 5%)");
    let negative = computed("max(-0px, 5%)");
    let hash = |value: &ComputedLengthMath| {
        let mut hasher = DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    };
    assert_eq!(positive, negative);
    assert_eq!(hash(&positive), hash(&negative));
}

#[test]
fn numeric_arithmetic_comparisons_and_tree_identity_cover_all_node_kinds() {
    use std::collections::hash_map::DefaultHasher;
    for (source, expected) in [
        ("calc(max(2, 3) * min(10px, 20%))", 30.0),
        ("calc(clamp(1, 4, 3) * min(10px, 20%))", 30.0),
        ("calc((2 + 4 - 2) / 2 * min(10px, 20%))", 20.0),
        ("calc(min(10px, 20%) * 2 * 3 / 2)", 30.0),
        ("clamp(5px, calc(10px + 5%), 20px)", 15.0),
    ] {
        let value = computed(source);
        assert_eq!(value.resolve(100.0), expected, "{source}");
        assert_eq!(
            specified(source)
                .unwrap()
                .resolve_with(|length| match length {
                    Length::Px(px) => px,
                    Length::Percent(percent) => percent,
                    _ => panic!("absolute fixture"),
                }),
            expected,
            "{source}"
        );
        let duplicate = computed(source);
        let hash = |value: &ComputedLengthMath| {
            let mut hasher = DefaultHasher::new();
            value.hash(&mut hasher);
            hasher.finish()
        };
        assert_eq!(value, duplicate);
        assert_eq!(hash(&value), hash(&duplicate));
    }
    assert_ne!(computed("min(10px, 20%)"), computed("max(10px, 20%)"));
    for source in [
        "unknown(10px, 20%)",
        "calc(2 + min(10px, 20%))",
        "calc(min(10px, 20%) + 2)",
        "calc(2 / min(10px, 20%))",
        "calc(min(10px, 20%) * 1e39)",
        "calc(min(10px, 20%) / 1e-39)",
        "calc((1e38 * 1e38) * min(10px, 20%))",
        "min(1e39px, 20%)",
    ] {
        assert!(specified(source).is_none(), "{source}");
    }
}

#[test]
fn lifting_a_computed_affine_leaf_preserves_both_coefficients() {
    let original = specified("max(1px, 2%)")
        .unwrap()
        .absolutize(|_| {
            Some(CalcLengthPercentage {
                px: 3.0,
                percent: 5.0,
            })
        })
        .unwrap();
    let lifted = LengthMath::from_computed(&original);
    for basis in [0.0, 10.0, 1000.0] {
        assert_eq!(
            lifted.resolve_with(|length| match length {
                Length::Px(px) => px,
                Length::Percent(percent) => basis * percent / 100.0,
                _ => panic!("lifted leaves are absolute"),
            }),
            original.resolve(basis)
        );
    }
}
