use super::serialize_alpha_value;

fn opacity(raw: &str) -> Option<String> {
    serialize_alpha_value("opacity", raw)
}

#[test]
fn plain_values_read_back_as_numbers() {
    assert_eq!(opacity("0.5").as_deref(), Some("0.5"));
    assert_eq!(opacity("-2").as_deref(), Some("-2"));
    assert_eq!(opacity("50%").as_deref(), Some("0.5"));
    assert_eq!(opacity("-100%").as_deref(), Some("-1"));
    assert_eq!(opacity("300%").as_deref(), Some("3"));
}

#[test]
fn math_functions_fold_to_one_calc_keeping_their_type() {
    assert_eq!(opacity("calc(-50% - 50%)").as_deref(), Some("calc(-100%)"));
    assert_eq!(opacity("calc(25% * 2)").as_deref(), Some("calc(50%)"));
    assert_eq!(opacity("calc(2 * 25%)").as_deref(), Some("calc(50%)"));
    assert_eq!(opacity("calc(1 + 1)").as_deref(), Some("calc(2)"));
    assert_eq!(opacity("calc((1 + 1) / 4)").as_deref(), Some("calc(0.5)"));
    assert_eq!(
        opacity("clamp(50%, 80%, 70%)").as_deref(),
        Some("calc(70%)")
    );
    assert_eq!(opacity("min(-0.4, 0.5)").as_deref(), Some("calc(-0.4)"));
    assert_eq!(opacity("max(0, calc(0.5))").as_deref(), Some("calc(0.5)"));
}

#[test]
fn unfoldable_text_and_other_properties_are_left_alone() {
    assert_eq!(opacity("calc(50% + 1)"), None);
    assert_eq!(opacity("calc(50% * 50%)"), None);
    assert_eq!(opacity("calc(1 / 0)"), None);
    assert_eq!(opacity("calc(1e38 * 10)"), None);
    assert_eq!(opacity("calc(-1e38 * 10)"), None);
    assert_eq!(opacity("1e400"), None);
    assert_eq!(opacity("-1e400"), None);
    assert_eq!(opacity("1e400%"), None);
    assert_eq!(opacity("var(--x)"), None);
    assert_eq!(opacity("clamp(1, 2)"), None);
    assert_eq!(serialize_alpha_value("width", "50%"), None);
}

#[test]
fn mixed_types_unknown_tokens_and_deep_nesting_are_left_alone() {
    assert_eq!(opacity("min(50%, 1)"), None);
    assert_eq!(opacity("abs(1)"), None);
    assert_eq!(opacity("calc(x)"), None);
    assert_eq!(opacity("0.5 1"), None);
    assert_eq!(opacity("50% 1"), None);
    assert_eq!(opacity("none"), None);
    let nested = format!("{}1{}", "calc(".repeat(40), ")".repeat(40));
    assert_eq!(opacity(&nested), None);
    let shallow = format!("{}1{}", "calc(".repeat(4), ")".repeat(4));
    assert_eq!(opacity(&shallow).as_deref(), Some("calc(1)"));
}

#[test]
fn zero_mantissa_exponents_read_back_as_zero() {
    assert_eq!(opacity("0e999").as_deref(), Some("0"));
    assert_eq!(opacity("0e999%").as_deref(), Some("0"));
    assert_eq!(opacity("calc(0e999 + 0.5)").as_deref(), Some("calc(0.5)"));
}
