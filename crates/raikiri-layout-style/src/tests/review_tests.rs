use super::*;
use raikiri_style::property::*;

#[test]
fn explicit_normal_alignment_does_not_inherit_like_auto() {
    let mut cv = ComputedValues::initial();
    cv.align_self = AlignSelfValue::Value(SelfAlignmentValue::Normal);
    cv.justify_self = cv.align_self;
    let p = PreparedStyle::new(&cv, StyleMetadata::default(), &[]).unwrap();
    let v = LayoutStyleView::new(&p, ViewContext::default(), &[], &[]);
    assert_eq!(v.align_self(), Some(values::AlignItems::NORMAL));
}

fn side_border(cv: &mut ComputedValues) {
    let mut border = raikiri_style::SpecifiedValues::initial().border.left;
    border.width = Length::Px(4.0);
    border.style = BorderStyle::Solid;
    cv.border.left = raikiri_style::resolve_border(
        border,
        raikiri_style::ComputedLength(16.0),
        None,
        &raikiri_style::ResolveContext::initial(),
    );
}

#[test]
fn ratio_uses_logical_sizes_before_pass_overrides() {
    let mut cv = ComputedValues::initial();
    cv.aspect_ratio.auto = false;
    cv.aspect_ratio.ratio = Some((2.0, 1.0));
    cv.vertical_logical_size = Some(raikiri_style::VerticalLogicalSize {
        width: L::Px(100.0),
        height: L::Px(50.0),
    });
    let p = PreparedStyle::new(
        &cv,
        StyleMetadata {
            flags: StyleFlags::VERTICAL_LOGICAL_SIZE,
            ..StyleMetadata::default()
        },
        &[],
    )
    .unwrap();
    let pass = [PropertyOverride::length(
        LengthProperty::Width,
        LengthValue::computed(L::Auto).unwrap(),
    )
    .unwrap()];
    let v = LayoutStyleView::new(&p, ViewContext::default(), &pass, &[]);
    assert_eq!(v.size().width, values::Dimension::auto());
    assert_eq!(v.aspect_ratio(), None);
}

#[test]
fn box_sizing_uses_logical_sizes_and_keyword_filtering_before_overrides() {
    let mut cv = ComputedValues::initial();
    side_border(&mut cv);
    cv.vertical_logical_size = Some(raikiri_style::VerticalLogicalSize {
        width: L::Px(100.0),
        height: L::Px(50.0),
    });
    let p = PreparedStyle::new(
        &cv,
        StyleMetadata {
            flags: StyleFlags::VERTICAL_LOGICAL_SIZE,
            ..StyleMetadata::default()
        },
        &[],
    )
    .unwrap();
    let v = LayoutStyleView::new(&p, ViewContext::default(), &[], &[]);
    assert_eq!(v.box_sizing(), values::BoxSizing::ContentBox);
    cv.width = L::MaxContent;
    cv.vertical_logical_size = None;
    let p = PreparedStyle::new(
        &cv,
        StyleMetadata {
            flags: StyleFlags::IGNORE_WIDTH_KEYWORD,
            ..StyleMetadata::default()
        },
        &[],
    )
    .unwrap();
    let pass = [PropertyOverride::length(
        LengthProperty::Width,
        LengthValue::computed(L::Px(88.0)).unwrap(),
    )
    .unwrap()];
    let v = LayoutStyleView::new(&p, ViewContext::default(), &pass, &[]);
    assert_eq!(v.box_sizing(), values::BoxSizing::BorderBox);
}

#[test]
fn replaced_without_a_natural_ratio_still_uses_authored_content_keyword_ratio() {
    let mut cv = ComputedValues::initial();
    cv.width = L::MaxContent;
    cv.height = L::Px(40.0);
    cv.aspect_ratio.auto = false;
    cv.aspect_ratio.ratio = Some((2.0, 1.0));
    let p = PreparedStyle::new(
        &cv,
        StyleMetadata {
            flags: StyleFlags::REPLACED,
            ..StyleMetadata::default()
        },
        &[],
    )
    .unwrap();
    let v = LayoutStyleView::new(&p, ViewContext::default(), &[], &[]);
    assert_eq!(v.aspect_ratio(), Some(2.0));
}
