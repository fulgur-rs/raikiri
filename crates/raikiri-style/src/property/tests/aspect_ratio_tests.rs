use super::*;

fn aspect_ratio(source: &str) -> Option<AspectRatioValue> {
    match parse_entire(source, "aspect-ratio") {
        Some(PropertyValue::AspectRatio(value)) => Some(value),
        Some(other) => panic!("expected PropertyValue::AspectRatio, got {other:?}"),
        None => None,
    }
}

fn ratio(width: f32, height: f32) -> AspectRatioValue {
    AspectRatioValue {
        auto: false,
        ratio: Some((width, height)),
    }
}

#[test]
fn aspect_ratio_accepts_auto_a_ratio_and_both_in_either_order() {
    assert_eq!(aspect_ratio("auto"), Some(AspectRatioValue::AUTO));
    assert_eq!(aspect_ratio("AUTO"), Some(AspectRatioValue::AUTO));
    assert_eq!(aspect_ratio("16 / 9"), Some(ratio(16.0, 9.0)));
    assert_eq!(aspect_ratio("16/9"), Some(ratio(16.0, 9.0)));
    assert_eq!(aspect_ratio("1.5/ 2"), Some(ratio(1.5, 2.0)));
    let auto_and_ratio = AspectRatioValue {
        auto: true,
        ratio: Some((4.0, 3.0)),
    };
    assert_eq!(aspect_ratio("auto 4 / 3"), Some(auto_and_ratio));
    assert_eq!(aspect_ratio("4 / 3 auto"), Some(auto_and_ratio));
}

#[test]
fn aspect_ratio_takes_a_lone_number_as_a_ratio_to_one() {
    assert_eq!(aspect_ratio("2"), Some(ratio(2.0, 1.0)));
    assert_eq!(aspect_ratio("0.5"), Some(ratio(0.5, 1.0)));
    assert_eq!(aspect_ratio("auto 3"), {
        Some(AspectRatioValue {
            auto: true,
            ratio: Some((3.0, 1.0)),
        })
    });
}

#[test]
fn aspect_ratio_keeps_a_degenerate_ratio_for_the_computed_value() {
    assert_eq!(aspect_ratio("0 / 1"), Some(ratio(0.0, 1.0)));
    assert_eq!(aspect_ratio("1 / 0"), Some(ratio(1.0, 0.0)));
    assert_eq!(aspect_ratio("0"), Some(ratio(0.0, 1.0)));
}

#[test]
fn aspect_ratio_rejects_other_values() {
    for source in [
        "",
        "none",
        "-1",
        "1 / -1",
        "auto auto",
        "auto 1 / 2 auto",
        "1 /",
        "/ 2",
        "1 2",
        "1 / 2 / 3",
        "1px",
        "50%",
        "1 / auto",
        "1, 2",
    ] {
        assert_eq!(aspect_ratio(source), None, "{source:?}");
    }
}

#[test]
fn aspect_ratio_key_maps_to_aspect_ratio_property_key() {
    let value = PropertyValue::AspectRatio(AspectRatioValue::AUTO);
    assert_eq!(value.key(), PropertyKey::AspectRatio);
    assert_eq!(
        property_key_for_name("aspect-ratio"),
        Some(PropertyKey::AspectRatio)
    );
    assert!(supported_property_names().contains(&"aspect-ratio"));
}

#[test]
fn the_preferred_ratio_follows_auto_and_the_natural_ratio() {
    let auto = AspectRatioValue::AUTO;
    assert_eq!(auto.preferred(None), None);
    assert_eq!(auto.preferred(Some(2.0)), Some(2.0));

    // A ratio alone replaces the natural ratio.
    assert_eq!(ratio(4.0, 2.0).preferred(None), Some(2.0));
    assert_eq!(ratio(4.0, 1.0).preferred(Some(2.0)), Some(4.0));

    // With `auto` the natural ratio wins and the ratio stands in for a box
    // without one.
    let auto_ratio = AspectRatioValue {
        auto: true,
        ratio: Some((4.0, 1.0)),
    };
    assert_eq!(auto_ratio.preferred(None), Some(4.0));
    assert_eq!(auto_ratio.preferred(Some(2.0)), Some(2.0));
}

#[test]
fn a_degenerate_ratio_behaves_as_auto() {
    for degenerate in [ratio(0.0, 1.0), ratio(1.0, 0.0), ratio(0.0, 0.0)] {
        assert_eq!(degenerate.preferred(None), None);
        assert_eq!(degenerate.preferred(Some(2.0)), Some(2.0));
    }
    // So does a ratio that overflows to infinity or underflows to zero.
    assert_eq!(ratio(f32::MAX, f32::MIN_POSITIVE).preferred(None), None);
    assert_eq!(
        ratio(f32::MIN_POSITIVE, f32::MAX).preferred(Some(3.0)),
        Some(3.0)
    );
    // An unusable natural ratio is no ratio.
    assert_eq!(AspectRatioValue::AUTO.preferred(Some(0.0)), None);
    assert_eq!(AspectRatioValue::AUTO.preferred(Some(f32::NAN)), None);
    assert_eq!(ratio(2.0, 1.0).preferred(Some(f32::INFINITY)), Some(2.0));
}

#[test]
fn the_natural_ratio_and_an_auto_ratio_relate_the_content_box() {
    let auto = AspectRatioValue::AUTO;
    assert!(auto.relates_content_box(Some(2.0)));
    // `auto` alone leaves nothing to relate without a natural ratio.
    assert!(!auto.relates_content_box(None));
    assert!(!auto.relates_content_box(Some(0.0)));

    // A ratio given with `auto` relates the content box, with or without a
    // natural ratio.
    let auto_ratio = AspectRatioValue {
        auto: true,
        ratio: Some((4.0, 1.0)),
    };
    assert!(auto_ratio.relates_content_box(None));
    assert!(auto_ratio.relates_content_box(Some(2.0)));

    // A ratio alone relates the box `box-sizing` names, a natural ratio
    // notwithstanding.
    assert!(!ratio(4.0, 1.0).relates_content_box(None));
    assert!(!ratio(4.0, 1.0).relates_content_box(Some(2.0)));
}

#[test]
fn a_degenerate_ratio_relates_the_content_box_of_the_natural_ratio() {
    for degenerate in [ratio(0.0, 1.0), ratio(1.0, 0.0), ratio(0.0, 0.0)] {
        assert!(degenerate.relates_content_box(Some(2.0)));
        assert!(!degenerate.relates_content_box(None));
    }
    // The same for a ratio that overflows to infinity.
    assert!(ratio(f32::MAX, f32::MIN_POSITIVE).relates_content_box(Some(3.0)));
}
