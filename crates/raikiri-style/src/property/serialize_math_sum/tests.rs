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
fn values_it_cannot_fold_are_left_alone() {
    for raw in [
        "10px",
        "calc(1 + 2)",
        "calc(1px + 2)",
        "calc(1px * 2px)",
        "calc(1px / 0)",
        "calc(1px +1em)",
        "calc(var(--x) + 1px)",
        "min(1px, 1em)",
        "calc(1deg + 1em)",
    ] {
        assert_eq!(sums(raw), None, "{raw}");
    }
}
