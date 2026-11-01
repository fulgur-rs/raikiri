//! The syntax of the computed `color` (`ComputedValues::color_modern`) and the
//! default interpolation space of a gradient whose `currentcolor` stops take
//! it. Whether a color is written in a legacy or a modern syntax is what
//! Chromium reports for the default space: the gradients below were drawn
//! there and the middle of each was read back (sRGB mid `126,98,144`, Oklab
//! mid `157,117,153` for `#e53935` to `#1e88e5`).

use super::*;
use crate::property::{BackgroundImage, Gradient, GradientColorInterpolation, MixColorSpace};

/// `color` values and whether Chromium draws `linear-gradient(currentcolor,
/// #1e88e5)` in Oklab for them (`true`) or in sRGB (`false`).
const CHROMIUM_DEFAULT_SPACE: [(&str, bool); 15] = [
    ("#e53935", false),
    ("crimson", false),
    ("transparent", false),
    ("rgb(229 57 53)", false),
    ("rgba(229, 57, 53, 0.5)", false),
    ("hsl(1 75% 55%)", false),
    ("hwb(1 22% 10%)", false),
    ("light-dark(#e53935, #000)", false),
    ("oklch(0.6 0.2 25)", true),
    ("lab(50 60 40)", true),
    ("color(srgb 0.9 0.22 0.2)", true),
    ("rgb(229 none 53)", true),
    ("color-mix(in srgb, #e53935, #e53935)", true),
    ("light-dark(oklch(0.6 0.2 25), #000)", true),
    ("rgb(from #e53935 r g b)", true),
];

fn gradient_space(values: &ComputedValues) -> GradientColorInterpolation {
    let BackgroundImage::Gradient(gradient) = &values.background_layers()[0].image else {
        panic!("expected a gradient");
    };
    match &**gradient {
        Gradient::Linear(gradient) => gradient.interpolation,
        Gradient::Radial(gradient) => gradient.interpolation,
        Gradient::Conic(gradient) => gradient.interpolation,
    }
}

fn space_for(color: &str, gradient: &str) -> GradientColorInterpolation {
    gradient_space(&cascade_doc(
        "",
        "div",
        Some(&format!("color: {color}; background-image: {gradient}")),
    ))
}

fn expected_space(modern: bool) -> MixColorSpace {
    if modern {
        MixColorSpace::Oklab
    } else {
        MixColorSpace::Srgb
    }
}

#[test]
fn a_currentcolor_stop_takes_the_syntax_of_the_computed_color() {
    for (color, modern) in CHROMIUM_DEFAULT_SPACE {
        let values = cascade_doc(
            "",
            "div",
            Some(&format!(
                "color: {color}; background-image: linear-gradient(currentcolor, #1e88e5)"
            )),
        );
        assert_eq!(values.color_modern, modern, "{color}");
        let space = gradient_space(&values);
        assert_eq!(space.color_space, expected_space(modern), "{color}");
        assert!(!space.follows_current_color, "{color}: settled");
    }
}

#[test]
fn radial_and_conic_gradients_settle_the_same_way() {
    for gradient in [
        "radial-gradient(currentcolor, #1e88e5)",
        "conic-gradient(currentcolor, #1e88e5)",
        "repeating-linear-gradient(currentcolor, #1e88e5 10px)",
    ] {
        for (color, modern) in [("#e53935", false), ("oklch(0.6 0.2 25)", true)] {
            let space = space_for(color, gradient);
            assert_eq!(
                space.color_space,
                expected_space(modern),
                "{gradient} {color}"
            );
            assert!(!space.follows_current_color, "{gradient} {color}");
        }
    }
}

#[test]
fn a_gradient_with_a_space_or_a_modern_stop_ignores_the_color() {
    // An `in` clause names the space.
    assert_eq!(
        space_for(
            "oklch(0.6 0.2 25)",
            "linear-gradient(in srgb, currentcolor, #1e88e5)"
        )
        .color_space,
        MixColorSpace::Srgb
    );
    assert_eq!(
        space_for(
            "#e53935",
            "linear-gradient(in oklab, currentcolor, #1e88e5)"
        )
        .color_space,
        MixColorSpace::Oklab
    );
    // A stop in a modern syntax already makes the default Oklab.
    assert_eq!(
        space_for("#e53935", "linear-gradient(currentcolor, oklab(0.6 0 0))").color_space,
        MixColorSpace::Oklab
    );
    // Without a `currentcolor` stop the color has no say.
    assert_eq!(
        space_for("oklch(0.6 0.2 25)", "linear-gradient(#e53935, #1e88e5)").color_space,
        MixColorSpace::Srgb
    );
}

#[test]
fn only_a_bare_currentcolor_stop_follows_the_color() {
    // `currentcolor` inside a color function is a modern syntax of its own.
    assert_eq!(
        space_for(
            "#e53935",
            "linear-gradient(color-mix(in srgb, currentcolor, red), blue)"
        )
        .color_space,
        MixColorSpace::Oklab
    );
    // `light-dark()` is its light branch, so it can be `currentcolor`.
    assert_eq!(
        space_for(
            "oklch(0.6 0.2 25)",
            "linear-gradient(light-dark(currentcolor, red), #1e88e5)"
        )
        .color_space,
        MixColorSpace::Oklab
    );
    assert_eq!(
        space_for(
            "#e53935",
            "linear-gradient(light-dark(currentcolor, red), #1e88e5)"
        )
        .color_space,
        MixColorSpace::Srgb
    );
    // Every stop may be `currentcolor`.
    assert_eq!(
        space_for(
            "oklch(0.6 0.2 25)",
            "linear-gradient(currentcolor, currentcolor)"
        )
        .color_space,
        MixColorSpace::Oklab
    );
}

#[test]
fn the_syntax_of_the_color_is_inherited_with_the_color() {
    let gradient = "background-image: linear-gradient(currentcolor, #1e88e5)";
    // (parent `color`, the child's own declarations, whether Oklab results).
    for (parent, own, modern) in [
        ("oklch(0.6 0.2 25)", "", true),
        ("oklch(0.6 0.2 25)", "color: inherit", true),
        ("oklch(0.6 0.2 25)", "color: unset", true),
        ("oklch(0.6 0.2 25)", "all: unset", true),
        ("oklch(0.6 0.2 25)", "color: currentcolor", true),
        (
            "oklch(0.6 0.2 25)",
            "color: light-dark(currentcolor, red)",
            true,
        ),
        ("#e53935", "color: light-dark(currentcolor, red)", false),
        ("oklch(0.6 0.2 25)", "color: initial", false),
        ("oklch(0.6 0.2 25)", "color: #e53935", false),
        // `contrast-color()` picks white or black, which are legacy colors.
        (
            "oklch(0.6 0.2 25)",
            "color: contrast-color(currentcolor)",
            false,
        ),
        (
            "oklch(0.6 0.2 25)",
            "color: var(--c); --c: oklch(0.6 0.2 25)",
            true,
        ),
        ("#e53935", "color: oklch(0.6 0.2 25)", true),
        (
            "#e53935",
            "color: color-mix(in srgb, currentcolor, red)",
            true,
        ),
        ("#e53935", "color: rgb(from currentcolor r g b)", true),
        ("#e53935", "", false),
    ] {
        let mut doc = TestDoc::new();
        let parent_element = doc.push_element(0, "div", Some(&format!("color: {parent}")));
        let child = doc.push_element(parent_element, "div", Some(&format!("{own}; {gradient}")));
        let tree = build_rule_tree(&doc);
        let result = cascade(&doc, &tree).expect("cascade Ok");
        let values = &result.computed[child];
        assert_eq!(values.color_modern, modern, "{parent} / {own}");
        assert_eq!(
            gradient_space(values).color_space,
            expected_space(modern),
            "{parent} / {own}"
        );
    }
}

#[test]
fn the_syntax_of_a_color_resolved_against_the_inherited_one_is_its_own() {
    // `color-mix()` and relative colors are modern whatever they mix.
    let mut doc = TestDoc::new();
    let parent = doc.push_element(0, "div", Some("color: #e53935"));
    let child = doc.push_element(
        parent,
        "div",
        Some("color: color-mix(in srgb, currentcolor, currentcolor)"),
    );
    let grandchild = doc.push_element(child, "div", None);
    let tree = build_rule_tree(&doc);
    let result = cascade(&doc, &tree).expect("cascade Ok");
    assert!(!result.computed[parent].color_modern);
    assert!(result.computed[child].color_modern);
    assert!(result.computed[grandchild].color_modern);
}

#[test]
fn mask_and_border_image_gradients_settle_the_same_way() {
    let interpolation = |image: &BackgroundImage| match image {
        BackgroundImage::Gradient(gradient) => match &**gradient {
            Gradient::Linear(gradient) => gradient.interpolation,
            _ => panic!("expected a linear gradient"),
        },
        _ => panic!("expected a gradient"),
    };
    for (color, modern) in [("#e53935", false), ("oklch(0.6 0.2 25)", true)] {
        let values = cascade_doc(
            "",
            "div",
            Some(&format!(
                "color: {color}; mask-image: linear-gradient(currentcolor, transparent); \
                 border-image-source: linear-gradient(currentcolor, #1e88e5)"
            )),
        );
        for image in [&values.mask_image, &values.border_image.source] {
            let space = interpolation(image);
            assert_eq!(space.color_space, expected_space(modern), "{color}");
            assert!(!space.follows_current_color, "{color}");
        }
    }
}
