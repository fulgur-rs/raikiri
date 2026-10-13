use super::serialize_position_value;

fn position(raw: &str) -> Option<String> {
    serialize_position_value("background-position", raw)
}

#[test]
fn one_component_gains_the_implied_center() {
    assert_eq!(position("1px").as_deref(), Some("1px center"));
    assert_eq!(position("LEFT").as_deref(), Some("left center"));
    assert_eq!(position("bottom").as_deref(), Some("center bottom"));
    assert_eq!(position("center").as_deref(), Some("center center"));
}

#[test]
fn two_components_put_the_horizontal_axis_first() {
    assert_eq!(position("center left").as_deref(), Some("left center"));
    assert_eq!(position("top left").as_deref(), Some("left top"));
    assert_eq!(position("bottom center").as_deref(), Some("center bottom"));
    assert_eq!(position("5% top").as_deref(), Some("5% top"));
    assert_eq!(position("left 10px").as_deref(), Some("left 10px"));
    assert_eq!(position("right 9%").as_deref(), Some("right 9%"));
    assert_eq!(
        position("calc(1px + 2%) -3%").as_deref(),
        Some("calc(1px + 2%) -3%")
    );
}

#[test]
fn keyword_offsets_stay_with_their_keyword() {
    assert_eq!(
        position("center right 7%").as_deref(),
        Some("right 7% center")
    );
    assert_eq!(
        position("top 15px center").as_deref(),
        Some("center top 15px")
    );
    assert_eq!(
        position("bottom 16% left").as_deref(),
        Some("left bottom 16%")
    );
    assert_eq!(
        position("top 17px right 18px").as_deref(),
        Some("right 18px top 17px")
    );
    assert_eq!(
        position("bottom right 19%").as_deref(),
        Some("right 19% bottom")
    );
    assert_eq!(
        position("left 10px center").as_deref(),
        Some("left 10px center")
    );
}

#[test]
fn unrecognized_text_and_other_properties_are_left_alone() {
    assert_eq!(position("1px, 2px"), None);
    assert_eq!(position("middle"), None);
    assert_eq!(position("1px 2px 3px"), None);
    assert_eq!(serialize_position_value("width", "left"), None);
}

#[test]
fn radial_gradient_positions_are_canonicalized_in_place() {
    let image = |raw| serialize_position_value("background-image", raw);
    assert_eq!(
        image("radial-gradient(at bottom 10% right 20%, red, blue)").as_deref(),
        Some("radial-gradient(at right 20% bottom 10%, red, blue)")
    );
    assert_eq!(
        image("repeating-radial-gradient(circle at top center, red, blue)").as_deref(),
        Some("repeating-radial-gradient(circle at center top, red, blue)")
    );
    // Already canonical, or no position: nothing to rewrite.
    assert_eq!(image("radial-gradient(at left bottom, red, blue)"), None);
    assert_eq!(image("radial-gradient(red, blue)"), None);
    assert_eq!(image("linear-gradient(to top, red, blue)"), None);
}

#[test]
fn object_position_and_odd_gradient_arguments() {
    assert_eq!(
        serialize_position_value("object-position", "top").as_deref(),
        Some("center top")
    );
    assert_eq!(position("#fff"), None);
    let image = |raw| serialize_position_value("background-image", raw);
    // A sized shape with a math function before `at`.
    assert_eq!(
        image("radial-gradient(calc(10px) at top left, red, blue)").as_deref(),
        Some("radial-gradient(calc(10px) at left top, red, blue)")
    );
    // A position with a math function offset.
    assert_eq!(
        image("radial-gradient(at top calc(1px + 2%), red, blue)").as_deref(),
        Some("radial-gradient(at calc(1px + 2%) top, red, blue)")
    );
    // `at` without a position, or with one that is not a position.
    assert_eq!(image("radial-gradient(at, red, blue)"), None);
    assert_eq!(image("radial-gradient(at middle, red, blue)"), None);
}

#[test]
fn substitution_functions_are_left_alone() {
    assert_eq!(position("var(--p)"), None);
    assert_eq!(position("left VAR(--y)"), None);
    assert_eq!(position("env(--x) top"), None);
    assert_eq!(
        serialize_position_value(
            "background-image",
            "radial-gradient(at var(--p), red, blue)"
        ),
        None
    );
}
