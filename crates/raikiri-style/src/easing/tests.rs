use super::is_valid_easing_function;

#[test]
fn accepts_each_easing_function_form() {
    for source in [
        "linear",
        "EASE",
        "ease-in",
        "ease-out",
        "ease-in-out",
        "step-start",
        "step-end",
        "cubic-bezier(0, 0, 1, 1)",
        "cubic-bezier(0.1, -5, 0.9, 7)",
        "steps(1)",
        "steps(2, jump-none)",
        "steps(3, start)",
        "steps(3, JUMP-BOTH)",
        "linear(0, 1)",
        "linear(0 0%, 0.5 25% 75%, 1)",
        "linear(10% 0, 1 100%)",
    ] {
        assert!(
            is_valid_easing_function(source),
            "{source:?} should be valid"
        );
    }
}

#[test]
fn rejects_malformed_easing_functions() {
    for source in [
        "",
        "bogus",
        "ease ease",
        "1",
        "cubic-bezier(2, 0, 1, 1)",
        "cubic-bezier(0, 0, -1, 1)",
        "cubic-bezier(0, 0, 1)",
        "cubic-bezier(0 0 1 1)",
        "cubic-bezier(0, 0, 1, 1, 1)",
        "steps(0)",
        "steps(1.5)",
        "steps(1, jump-none)",
        "steps(1, sideways)",
        "steps(1, 2)",
        "steps(1,)",
        "steps(1, end, end)",
        "linear(0)",
        "linear()",
        "linear(0,)",
        "linear(0, 1,)",
        "linear(0, 1, )",
        "linear(0 1, 1)",
        "linear(0% 0, 1 10% 20% 30%)",
        "linear(0, foo)",
        "other(0)",
    ] {
        assert!(
            !is_valid_easing_function(source),
            "{source:?} should be invalid"
        );
    }
}
