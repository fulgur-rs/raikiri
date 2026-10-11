use super::*;
use crate::computed::ComputedBorderImage;
use crate::property::{
    BackgroundImage, BorderImageOutsetSide, BorderImageRepeatKeyword, BorderImageSliceOffset,
    BorderImageWidthSide,
};
use crate::resolve::ComputedLengthPercentage;

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
        BorderImageWidthSide::LengthPercentage(ComputedLengthPercentage::Px(2.0))
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
