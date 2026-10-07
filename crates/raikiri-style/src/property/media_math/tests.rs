use super::*;

#[test]
fn valid_functions_keep_their_numeric_types() {
    for value in [
        "clamp(1px, 2em, 3px)",
        "clamp(none, 2em, none)",
        "min(1px)",
        "max(1px, 2em)",
        "abs(-1px)",
        "round(nearest, 1px, 2em)",
        "mod(1px, 2em)",
        "rem(1px, 2em)",
        "hypot(1px, 2em)",
        "calc(1px * 1em / 1rem)",
        "calc(1px * (2 + 1))",
        "calc(1px /**/ + /**/ 2em)",
    ] {
        assert!(
            math_value_has_type(value, MediaNumericType::Length),
            "{value}"
        );
    }
    for value in [
        "round(1.5)",
        "round(up, 1.5)",
        "sign(1px)",
        "pow(2, 3)",
        "sqrt(4)",
        "exp(1)",
        "log(2)",
        "log(2, 10)",
        "sin(1deg)",
        "cos(1)",
        "tan(1rad)",
        "calc(1px / 2em)",
        "calc(1s / 2ms)",
        "calc(1Hz / 2kHz)",
        "calc(1dpi / 2dppx)",
        "calc(1fr / 2fr)",
        "calc(1% / 2%)",
        "calc(asin(1) / acos(1))",
        "calc(atan(1) / atan2(1px, 2em))",
    ] {
        assert!(
            math_value_has_type(value, MediaNumericType::Number),
            "{value}"
        );
    }
}

#[test]
fn malformed_functions_and_wrong_types_remain_invalid() {
    for value in [
        "clamp(1px)",
        "clamp(1px, 2px)",
        "clamp(1px, 2px, 3px, 4px)",
        "clamp(1px, none, 2px)",
        "clamp(1px, 2deg, 3px)",
        "min()",
        "min(1px,)",
        "min(1px,,2px)",
        "calc(1px, 2px)",
        "calc(1px + 1deg)",
        "calc(- (1px))",
        "calc(1px/**/+/**/2px)",
        "calc(1px +/**/2px)",
        "calc(1px + - 2px)",
        "calc(1unknown)",
        "round(1px)",
        "round(sideways, 1px, 2px)",
        "mod(1px)",
        "sign(1px, 2px)",
        "pow(1px, 2)",
        "sqrt(1px)",
        "log(1px)",
        "log(1,2,3)",
        "exp(1,2)",
        "sin(1px)",
        "asin(1px)",
        "atan2(1px, 2deg)",
        "mystery(1px)",
        "var(--length)",
    ] {
        assert!(
            !math_value_has_type(value, MediaNumericType::Length),
            "{value}"
        );
        assert!(
            !math_value_has_type(value, MediaNumericType::Number),
            "{value}"
        );
    }
    assert!(!math_value_has_type(
        "calc(1px * 1px)",
        MediaNumericType::Length
    ));
    assert!(!math_value_has_type("asin(1)", MediaNumericType::Number));
}

#[test]
fn grammar_storage_and_nesting_are_bounded() {
    assert!(math_value_has_type(
        &format!("min({})", vec!["1px"; 128].join(",")),
        MediaNumericType::Length
    ));
    assert!(!math_value_has_type(
        &format!("min({})", vec!["1px"; 129].join(",")),
        MediaNumericType::Length
    ));
    assert!(!math_value_has_type(
        &format!("{}1px{}", "calc(".repeat(130), ")".repeat(130)),
        MediaNumericType::Length
    ));
}
