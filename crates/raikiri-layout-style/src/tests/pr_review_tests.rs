use super::*;
use raikiri_style::{ComputedLengthPercentageOrCalc as P, property::WritingMode};

#[test]
fn ignoring_logical_minimum_preserves_explicit_physical_min_height() {
    let mut cv = ComputedValues::initial();
    cv.cssom_writing_mode = WritingMode::VerticalRl;
    cv.min_height = L::Px(19.0);
    cv.min_block_size = Some(L::MaxContent);
    let metadata = StyleMetadata {
        flags: StyleFlags::IGNORE_LOGICAL_MIN_BLOCK,
        ..StyleMetadata::default()
    };
    let prepared = PreparedStyle::new(&cv, metadata, &[]).unwrap();
    let v = LayoutStyleView::new(&prepared, ViewContext::default(), &[], &[]);
    assert_eq!(
        v.min_size().height,
        taffy::LengthPercentageAuto::length(19.0)
    );
}

#[test]
fn ignoring_logical_minimum_still_suppresses_the_logical_fallback() {
    let mut cv = ComputedValues::initial();
    cv.cssom_writing_mode = WritingMode::VerticalRl;
    cv.min_block_size = Some(L::MaxContent);
    let metadata = StyleMetadata {
        flags: StyleFlags::IGNORE_LOGICAL_MIN_BLOCK,
        ..StyleMetadata::default()
    };
    let prepared = PreparedStyle::new(&cv, metadata, &[]).unwrap();
    let v = LayoutStyleView::new(&prepared, ViewContext::default(), &[], &[]);
    assert_eq!(v.min_size().height, taffy::LengthPercentageAuto::auto());
}

#[test]
fn sideways_lr_reverses_direction_without_reversing_edges_twice() {
    let mut cv = ComputedValues::initial();
    cv.cssom_writing_mode = WritingMode::SidewaysLr;
    cv.margin.top = L::Px(1.0);
    cv.margin.right = L::Px(2.0);
    cv.margin.bottom = L::Px(3.0);
    cv.margin.left = L::Px(4.0);
    cv.padding.top = P::Px(1.0);
    cv.padding.right = P::Px(2.0);
    cv.padding.bottom = P::Px(3.0);
    cv.padding.left = P::Px(4.0);
    cv.top = L::Px(1.0);
    cv.right = L::Px(2.0);
    cv.bottom = L::Px(3.0);
    cv.left = L::Px(4.0);
    let prepared = PreparedStyle::new(&cv, StyleMetadata::default(), &[]).unwrap();
    let v = LayoutStyleView::new(
        &prepared,
        ViewContext {
            layout_wm: WritingMode::SidewaysLr,
            ..ViewContext::default()
        },
        &[],
        &[],
    );
    assert_eq!(v.direction(), taffy::Direction::Rtl);
    assert_eq!(
        v.margin(),
        taffy::Rect {
            left: taffy::LengthPercentageAuto::length(1.0),
            right: taffy::LengthPercentageAuto::length(3.0),
            top: taffy::LengthPercentageAuto::length(4.0),
            bottom: taffy::LengthPercentageAuto::length(2.0)
        }
    );
    assert_eq!(v.inset(), v.margin());
    assert_eq!(
        v.padding(),
        taffy::Rect {
            left: taffy::LengthPercentage::length(1.0),
            right: taffy::LengthPercentage::length(3.0),
            top: taffy::LengthPercentage::length(4.0),
            bottom: taffy::LengthPercentage::length(2.0)
        }
    );
}
