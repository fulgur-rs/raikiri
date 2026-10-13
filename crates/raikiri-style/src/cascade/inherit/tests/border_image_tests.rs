use super::*;
use crate::computed::ComputedBorderImage;
use crate::property::{
    BackgroundImage, BorderImageOutsetSide, BorderImageRepeatKeyword, BorderImageSliceOffset,
    BorderImageWidthSide,
};
use crate::resolve::ComputedLengthPercentageOrCalc;

const BORDER_IMAGE: &str = "url(https://images.test/a.png) 7 / 2px 10% 3 auto / 1px 2 round";

#[test]
fn variable_border_image_shorthands_set_every_longhand() {
    let values = cascade_doc(
        "",
        "div",
        Some(&format!("--b:{BORDER_IMAGE};border-image:var(--b)")),
    );
    let image = &values.border_image;
    assert!(matches!(image.source, BackgroundImage::Url(_)));
    assert_eq!(image.slice.offsets.top, BorderImageSliceOffset::Number(7.0));
    assert_eq!(
        image.width.top,
        BorderImageWidthSide::LengthPercentage(ComputedLengthPercentageOrCalc::Px(2.0))
    );
    assert_eq!(image.width.bottom, BorderImageWidthSide::Number(3.0));
    assert_eq!(image.width.left, BorderImageWidthSide::Auto);
    assert_eq!(image.outset.top, BorderImageOutsetSide::Length(1.0));
    assert_eq!(image.outset.right, BorderImageOutsetSide::Number(2.0));
    assert_eq!(image.repeat.horizontal, BorderImageRepeatKeyword::Round);
}

#[test]
fn variable_border_shorthands_reset_the_border_image() {
    let values = cascade_doc(
        "",
        "div",
        Some(&format!(
            "border-image:{BORDER_IMAGE};--b:1px solid;border:var(--b)"
        )),
    );
    assert_eq!(values.border_image, ComputedBorderImage::initial());
}

#[test]
fn variable_css_wide_border_images_default_every_longhand() {
    let mut doc = TestDoc::new();
    let parent = doc.push_element(0, "div", Some(&format!("border-image:{BORDER_IMAGE}")));
    let inherited = doc.push_element(parent, "div", Some("--k:inherit;border-image:var(--k)"));
    let initial = doc.push_element(
        parent,
        "div",
        Some(&format!(
            "border-image:{BORDER_IMAGE};--k:initial;border-image:var(--k)"
        )),
    );
    let invalid = doc.push_element(
        parent,
        "div",
        Some(&format!(
            "border-image:{BORDER_IMAGE};border-image:var(--missing)"
        )),
    );
    let result = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    assert_eq!(
        result.computed[inherited].border_image,
        result.computed[parent].border_image
    );
    for node in [initial, invalid] {
        assert_eq!(
            result.computed[node].border_image,
            ComputedBorderImage::initial()
        );
    }
}

#[test]
fn direct_border_image_values_reach_the_specified_longhands() {
    let mut values = crate::specified::SpecifiedValues::initial();
    let shorthand = crate::property::BorderImageShorthand::initial();
    apply_value(
        PropertyValue::BorderImage(Box::new(shorthand.clone())),
        &mut values,
    );
    assert_eq!(values.border_image_repeat, shorthand.repeat);
    apply_value(
        PropertyValue::BorderImageSource(BackgroundImage::None),
        &mut values,
    );
    assert_eq!(values.border_image_source, BackgroundImage::None);
}

#[test]
fn variable_css_wide_borders_carry_the_keyword_to_the_border_image() {
    let mut doc = TestDoc::new();
    let parent = doc.push_element(0, "div", Some(&format!("border-image:{BORDER_IMAGE}")));
    let inherited = doc.push_element(parent, "div", Some("--k:inherit;border:var(--k)"));
    let result = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    assert_eq!(
        result.computed[inherited].border_image,
        result.computed[parent].border_image
    );
}

#[test]
fn border_image_math_functions_clamp_to_the_range() {
    for (style, label) in [
        (
            "border-image-slice: calc(-1) calc(-10%) max(-5, -2) 4; \
             border-image-width: calc(-1) calc(-5px) calc(-10%) 2; \
             border-image-outset: calc(-1) calc(-5px) 1 1px",
            "longhands",
        ),
        (
            "--n: -1; border-image-slice: calc(var(--n)) calc(-10%) max(-5, -2) 4; \
             border-image-width: calc(var(--n)) calc(-5px) calc(-10%) 2; \
             border-image-outset: calc(var(--n)) calc(-5px) 1 1px",
            "variables",
        ),
        (
            "border-image: url(https://images.test/a.png) calc(-1) calc(-10%) max(-5, -2) 4 \
             / calc(-1) calc(-5px) calc(-10%) 2 / calc(-1) calc(-5px) 1 1px",
            "shorthand",
        ),
    ] {
        let values = cascade_doc("", "div", Some(style));
        let image = &values.border_image;
        assert_eq!(
            image.slice.offsets.top,
            BorderImageSliceOffset::Number(0.0),
            "{label}"
        );
        assert_eq!(
            image.slice.offsets.right,
            BorderImageSliceOffset::Percent(0.0),
            "{label}"
        );
        assert_eq!(
            image.slice.offsets.bottom,
            BorderImageSliceOffset::Number(0.0),
            "{label}"
        );
        assert_eq!(
            image.width.top,
            BorderImageWidthSide::Number(0.0),
            "{label}"
        );
        assert_eq!(
            image.width.right,
            BorderImageWidthSide::LengthPercentage(ComputedLengthPercentageOrCalc::Px(0.0)),
            "{label}"
        );
        assert!(
            matches!(
                image.width.bottom,
                BorderImageWidthSide::LengthPercentage(
                    ComputedLengthPercentageOrCalc::Px(0.0)
                        | ComputedLengthPercentageOrCalc::Percent(0.0)
                )
            ),
            "{label}: {:?}",
            image.width.bottom
        );
        assert_eq!(
            image.outset.top,
            BorderImageOutsetSide::Number(0.0),
            "{label}"
        );
        assert_eq!(
            image.outset.right,
            BorderImageOutsetSide::Length(0.0),
            "{label}"
        );
    }
}

#[test]
fn border_image_literal_negative_numbers_stay_invalid() {
    let values = cascade_doc(
        "",
        "div",
        Some("border-image-width: 2; border-image-width: -1"),
    );
    assert_eq!(
        values.border_image.width.top,
        BorderImageWidthSide::Number(2.0)
    );
}

#[test]
fn border_image_math_functions_in_range_keep_their_value() {
    let values = cascade_doc(
        "",
        "div",
        Some("border-image-slice: calc(1 + 1) calc(25% * 2); border-image-outset: calc(1px + 1em)"),
    );
    let image = &values.border_image;
    assert_eq!(image.slice.offsets.top, BorderImageSliceOffset::Number(2.0));
    let width = cascade_doc(
        "",
        "div",
        Some("border-image-width: min(-5px, -2px) max(-1%, -2%)"),
    );
    assert_eq!(
        width.border_image.width.top,
        BorderImageWidthSide::LengthPercentage(ComputedLengthPercentageOrCalc::Px(0.0))
    );
    assert!(matches!(
        width.border_image.width.right,
        BorderImageWidthSide::LengthPercentage(
            ComputedLengthPercentageOrCalc::Px(0.0) | ComputedLengthPercentageOrCalc::Percent(0.0)
        )
    ));
    assert_eq!(
        image.slice.offsets.right,
        BorderImageSliceOffset::Percent(50.0)
    );
    // The outset grammar keeps no mixed-unit calc, so a math function that
    // does not fold to one value leaves the initial outset.
    assert_eq!(image.outset.top, BorderImageOutsetSide::Number(0.0));
}
