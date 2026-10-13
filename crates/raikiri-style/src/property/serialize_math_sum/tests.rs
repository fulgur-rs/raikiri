use super::serialize_math_sums;

fn sums(raw: &str) -> Option<String> {
    serialize_math_sums(raw)
}

#[test]
fn terms_sort_with_the_percentage_first_then_by_unit() {
    assert_eq!(
        sums("calc(10px + 0.5em) calc(10px - 0.5em)").as_deref(),
        Some("calc(0.5em + 10px) calc(-0.5em + 10px)")
    );
    assert_eq!(
        sums("calc(1vw + 2em + 10%)").as_deref(),
        Some("calc(10% + 2em + 1vw)")
    );
}

#[test]
fn like_units_combine_and_absolute_lengths_become_px() {
    assert_eq!(
        sums("calc(1em + 2em + 1in)").as_deref(),
        Some("calc(3em + 96px)")
    );
    assert_eq!(sums("calc(1ex + 2px)").as_deref(), Some("calc(1ex + 2px)"));
    assert_eq!(sums("calc(10px + 20px)").as_deref(), Some("calc(30px)"));
}

#[test]
fn canceled_terms_stay_as_zero() {
    assert_eq!(
        sums("calc(1em - 1em + 10px)").as_deref(),
        Some("calc(0em + 10px)")
    );
    assert_eq!(
        sums("calc(100% - 100% + 1em)").as_deref(),
        Some("calc(0% + 1em)")
    );
    assert_eq!(sums("calc(1em - 1em)").as_deref(), Some("calc(0em)"));
}

#[test]
fn products_and_nested_sums_fold() {
    assert_eq!(
        sums("calc(2 * (1em - 3px) / 4)").as_deref(),
        Some("calc(0.5em - 1.5px)")
    );
    assert_eq!(
        sums("calc(calc(1em + 1px) * 2)").as_deref(),
        Some("calc(2em + 2px)")
    );
}

#[test]
fn text_around_the_sums_is_kept() {
    assert_eq!(
        sums("auto calc(1px + 1em) auto").as_deref(),
        Some("auto calc(1em + 1px) auto")
    );
}

#[test]
fn sums_inside_other_functions_are_rewritten() {
    assert_eq!(
        sums("translateX(calc(10px + 0.5em)) rotate(1deg)").as_deref(),
        Some("translateX(calc(0.5em + 10px)) rotate(1deg)")
    );
    // Math functions other than min() / max() / clamp() keep their
    // arguments as written.
    assert_eq!(sums("round(calc(1px + 1em), 5px)"), None);
    assert_eq!(sums("abs(calc(1px + 1em))"), None);
}

#[test]
fn units_that_cancel_in_a_product_drop_out() {
    assert_eq!(sums("calc(1px / 1px * 1em)").as_deref(), Some("calc(1em)"));
    assert_eq!(sums("calc(1em * 2px / 1px)").as_deref(), Some("calc(2em)"));
    assert_eq!(sums("calc(1in / 1px * 1em)").as_deref(), Some("calc(96em)"));
    assert_eq!(
        sums("calc(10% / 5% * 1em + 1px)").as_deref(),
        Some("calc(2em + 1px)")
    );
    assert_eq!(
        sums("calc(1px * 1px / 1px + 1em)").as_deref(),
        Some("calc(1em + 1px)")
    );
    // Units that only cancel at computed-value time, a product left with a
    // squared unit, and a sum of several terms are not folded.
    for raw in [
        "calc(1em / 1rem * 1px)",
        "calc(1px * 1px)",
        "calc(1px / 1em)",
        "calc((1px + 1em) / 1px)",
        "calc((1px + 1em) * 1px)",
        "calc(1px / 0px * 1em)",
    ] {
        assert_eq!(sums(raw), None, "{raw}");
    }
}

#[test]
fn comparison_functions_keep_canonical_calculations_as_arguments() {
    assert_eq!(
        sums("min(calc(10px + 0.5em), 20px)").as_deref(),
        Some("min(0.5em + 10px, 20px)")
    );
    assert_eq!(
        sums("MAX(calc(10px + 0.5em),1in)").as_deref(),
        Some("max(0.5em + 10px, 96px)")
    );
    assert_eq!(
        sums("min(calc(1px + 1em), 5%)").as_deref(),
        Some("min(1em + 1px, 5%)")
    );
    assert_eq!(
        sums("clamp(1em, calc(10px + 1em), 50%)").as_deref(),
        Some("clamp(1em, 1em + 10px, 50%)")
    );
    assert_eq!(
        sums("min(1px, max(calc(2em + 1em), 1px))").as_deref(),
        Some("min(1px, max(3em, 1px))")
    );
    // A calc() whose whole calculation is a comparison function reads back
    // as that function.
    assert_eq!(
        sums("calc(min(1px, 1em))").as_deref(),
        Some("min(1px, 1em)")
    );
    assert_eq!(
        sums("translateX(min(calc(1px + 1em), 2px))").as_deref(),
        Some("translateX(min(1em + 1px, 2px))")
    );
}

#[test]
fn comparison_function_arguments_of_one_unit_combine() {
    assert_eq!(
        sums("min(10px, 1em, 20px, 2em)").as_deref(),
        Some("min(10px, 1em)")
    );
    assert_eq!(
        sums("max(10px, 1em, 20px, 2em)").as_deref(),
        Some("max(20px, 2em)")
    );
    assert_eq!(sums("min(10px, 1in)").as_deref(), Some("calc(10px)"));
    assert_eq!(
        sums("clamp(10px, 50px, 20px)").as_deref(),
        Some("calc(20px)")
    );
    assert_eq!(
        sums("calc(max(1em, 2em) + 1px)").as_deref(),
        Some("calc(2em + 1px)")
    );
    assert_eq!(
        sums("calc(max(1em, 2px) + 1px)"),
        None,
        "a comparison function left inside a sum is not folded"
    );
}

#[test]
fn numbers_parse_as_the_property_parsers_read_them() {
    assert_eq!(
        sums("calc(0e999px + 1em)").as_deref(),
        Some("calc(1em + 0px)")
    );
}

#[test]
fn every_length_unit_takes_part() {
    assert_eq!(
        sums("calc(10px + 1ch)").as_deref(),
        Some("calc(1ch + 10px)")
    );
    assert_eq!(
        sums("calc(1vw + 1ric + 1lh)").as_deref(),
        Some("calc(1lh + 1ric + 1vw)")
    );
}

#[test]
fn values_it_cannot_fold_are_left_alone() {
    for raw in [
        "10px",
        "calc(1 + 2)",
        "calc(1px + 2)",
        "calc(1px * 2px)",
        "calc(1px / 0)",
        "calc(1px +1em)",
        "calc(var(--x) + 1px)",
        "min(1, 2)",
        "min(1px, 2)",
        "clamp(1px, 2px)",
        "clamp(none, 1px, 2em)",
        "calc(1deg + 1em)",
        "calc(1px + 1cap)",
        "var(--x, calc(1px + 1em))",
    ] {
        assert_eq!(sums(raw), None, "{raw}");
    }
}
