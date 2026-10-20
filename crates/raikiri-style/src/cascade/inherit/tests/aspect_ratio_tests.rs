use super::*;
use crate::property::AspectRatioValue;

fn aspect_ratio(values: &ComputedValues) -> AspectRatioValue {
    values.aspect_ratio
}

fn ratio(auto: bool, width: f32, height: f32) -> AspectRatioValue {
    AspectRatioValue {
        auto,
        ratio: Some((width, height)),
    }
}

#[test]
fn aspect_ratio_cascade_starts_at_auto_and_takes_the_last_valid_declaration() {
    for (source, expected) in [
        ("", AspectRatioValue::AUTO),
        ("aspect-ratio:16/9", ratio(false, 16.0, 9.0)),
        ("aspect-ratio:2", ratio(false, 2.0, 1.0)),
        ("aspect-ratio:calc(1 + 1) / 4", ratio(false, 2.0, 4.0)),
        ("aspect-ratio:auto calc(3)", ratio(true, 3.0, 1.0)),
        ("aspect-ratio:auto 4/3", ratio(true, 4.0, 3.0)),
        (
            "aspect-ratio:16/9;aspect-ratio:auto",
            AspectRatioValue::AUTO,
        ),
        ("aspect-ratio:16/9;aspect-ratio:-1", ratio(false, 16.0, 9.0)),
        (
            "aspect-ratio:16/9;aspect-ratio:bogus",
            ratio(false, 16.0, 9.0),
        ),
        (
            "aspect-ratio:16/9;aspect-ratio:initial",
            AspectRatioValue::AUTO,
        ),
        (
            "aspect-ratio:16/9;aspect-ratio:unset",
            AspectRatioValue::AUTO,
        ),
        (
            "aspect-ratio:16/9;aspect-ratio:revert",
            AspectRatioValue::AUTO,
        ),
        (
            "--ratio:3/2;aspect-ratio:var(--ratio)",
            ratio(false, 3.0, 2.0),
        ),
        (
            "--ratio:initial;aspect-ratio:2;aspect-ratio:var(--ratio)",
            AspectRatioValue::AUTO,
        ),
    ] {
        assert_eq!(
            aspect_ratio(&cascade_doc("", "div", Some(source))),
            expected,
            "{source}"
        );
    }
}

#[test]
fn aspect_ratio_does_not_inherit_without_explicit_inherit() {
    let mut doc = TestDoc::new();
    let parent = doc.push_element(0, "div", Some("aspect-ratio:16/9;--inherit:inherit"));
    let ordinary = doc.push_element(parent, "div", None);
    let inherited = doc.push_element(parent, "div", Some("aspect-ratio:inherit"));
    let unset = doc.push_element(parent, "div", Some("aspect-ratio:unset"));
    let variable = doc.push_element(parent, "div", Some("aspect-ratio:var(--inherit)"));
    let result = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    let sixteen_by_nine = ratio(false, 16.0, 9.0);
    assert_eq!(aspect_ratio(&result.computed[parent]), sixteen_by_nine);
    assert_eq!(
        aspect_ratio(&result.computed[ordinary]),
        AspectRatioValue::AUTO
    );
    assert_eq!(aspect_ratio(&result.computed[inherited]), sixteen_by_nine);
    assert_eq!(
        aspect_ratio(&result.computed[unset]),
        AspectRatioValue::AUTO
    );
    assert_eq!(aspect_ratio(&result.computed[variable]), sixteen_by_nine);
}
