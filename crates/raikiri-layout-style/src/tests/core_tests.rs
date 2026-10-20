use super::*;
use raikiri_style::property::*;

fn query(cv: &ComputedValues, flags: StyleFlags, f: impl FnOnce(LayoutStyleView<'_>)) {
    let p = PreparedStyle::new(
        cv,
        StyleMetadata {
            flags,
            ..StyleMetadata::default()
        },
        &[],
    )
    .unwrap();
    f(LayoutStyleView::new(&p, ViewContext::default(), &[], &[]));
}

#[test]
fn flex_basis_override_can_replace_content() {
    let mut cv = ComputedValues::initial();
    cv.flex_basis = raikiri_style::ComputedFlexBasis::Content;
    let p = PreparedStyle::new(&cv, StyleMetadata::default(), &[]).unwrap();
    let overrides = [PropertyOverride::length(
        LengthProperty::FlexBasis,
        LengthValue::computed(L::Px(33.0)).unwrap(),
    )
    .unwrap()];
    let v = LayoutStyleView::new(&p, ViewContext::default(), &[], &overrides);
    assert_eq!(v.flex_basis(), taffy::Dimension::length(33.0));
}

#[test]
fn sizing_keywords_and_non_finite_numbers_follow_bridge_policy() {
    for (input, expected) in [
        (f32::NAN, 0.0),
        (f32::INFINITY, 1e7),
        (f32::NEG_INFINITY, -1e7),
        (2e7, 1e7),
        (-2e7, -1e7),
        (3.0, 3.0),
    ] {
        let mut cv = ComputedValues::initial();
        cv.width = L::Px(input);
        cv.margin.left = L::Percent(input * 100.0);
        query(&cv, StyleFlags::default(), |v| {
            assert_eq!(v.size().width, taffy::Dimension::length(expected));
            assert_eq!(
                v.margin().left,
                taffy::LengthPercentageAuto::percent(expected)
            );
        });
    }
    for value in [
        L::MinContent,
        L::MaxContent,
        L::FitContent,
        L::FitContentOf(raikiri_style::ComputedLengthPercentageOrCalc::Percent(25.0)),
    ] {
        let mut cv = ComputedValues::initial();
        cv.width = value;
        cv.min_width = value;
        cv.max_height = value;
        query(&cv, StyleFlags::default(), |v| {
            assert_ne!(v.size().width, taffy::Dimension::auto());
            assert_eq!(v.min_size().width, taffy::LengthPercentageAuto::auto());
            assert_eq!(v.max_size().height, taffy::LengthPercentageAuto::auto());
        });
        query(&cv, StyleFlags::IGNORE_WIDTH_KEYWORD, |v| {
            assert_eq!(v.size().width, taffy::Dimension::auto())
        });
    }
}

#[test]
fn all_flex_and_alignment_keywords_have_explicit_results() {
    for (input, expected) in [
        (FlexDirectionValue::Row, taffy::FlexDirection::Row),
        (
            FlexDirectionValue::RowReverse,
            taffy::FlexDirection::RowReverse,
        ),
        (FlexDirectionValue::Column, taffy::FlexDirection::Column),
        (
            FlexDirectionValue::ColumnReverse,
            taffy::FlexDirection::ColumnReverse,
        ),
    ] {
        let mut cv = ComputedValues::initial();
        cv.flex_direction = input;
        query(&cv, StyleFlags::default(), |v| {
            assert_eq!(v.flex_direction(), expected)
        });
    }
    for (input, expected) in [
        (FlexWrapValue::NoWrap, taffy::FlexWrap::NoWrap),
        (FlexWrapValue::Wrap, taffy::FlexWrap::Wrap),
        (FlexWrapValue::WrapReverse, taffy::FlexWrap::WrapReverse),
    ] {
        let mut cv = ComputedValues::initial();
        cv.flex_wrap = input;
        query(&cv, StyleFlags::default(), |v| {
            assert_eq!(v.flex_wrap(), expected)
        });
    }
    for (input, expected) in [
        (ContentAlignmentValue::Normal, taffy::AlignContent::NORMAL),
        (ContentAlignmentValue::Stretch, taffy::AlignContent::STRETCH),
        (
            ContentAlignmentValue::SpaceBetween,
            taffy::AlignContent::SPACE_BETWEEN,
        ),
        (
            ContentAlignmentValue::SpaceAround,
            taffy::AlignContent::SPACE_AROUND,
        ),
        (
            ContentAlignmentValue::SpaceEvenly,
            taffy::AlignContent::SPACE_EVENLY,
        ),
        (ContentAlignmentValue::Center, taffy::AlignContent::CENTER),
        (ContentAlignmentValue::Start, taffy::AlignContent::START),
        (ContentAlignmentValue::End, taffy::AlignContent::END),
        (
            ContentAlignmentValue::FlexStart,
            taffy::AlignContent::FLEX_START,
        ),
        (
            ContentAlignmentValue::FlexEnd,
            taffy::AlignContent::FLEX_END,
        ),
    ] {
        let mut cv = ComputedValues::initial();
        cv.align_content = input;
        cv.justify_content = input;
        query(&cv, StyleFlags::default(), |v| {
            assert_eq!(v.align_content(), expected);
            assert_eq!(v.justify_content(), expected);
        });
    }
    for (input, expected) in [
        (SelfAlignmentValue::Normal, taffy::AlignItems::NORMAL),
        (SelfAlignmentValue::Stretch, taffy::AlignItems::STRETCH),
        (SelfAlignmentValue::Center, taffy::AlignItems::CENTER),
        (SelfAlignmentValue::Start, taffy::AlignItems::START),
        (SelfAlignmentValue::End, taffy::AlignItems::END),
        (SelfAlignmentValue::FlexStart, taffy::AlignItems::FLEX_START),
        (SelfAlignmentValue::FlexEnd, taffy::AlignItems::FLEX_END),
        (SelfAlignmentValue::Baseline, taffy::AlignItems::BASELINE),
        (
            SelfAlignmentValue::LastBaseline,
            taffy::AlignItems::BASELINE,
        ),
    ] {
        let mut cv = ComputedValues::initial();
        cv.align_items = input;
        cv.align_self = AlignSelfValue::Value(input);
        query(&cv, StyleFlags::default(), |v| {
            assert_eq!(v.align_items(), expected);
            assert_eq!(v.align_self(), Some(expected));
        });
    }
}

#[test]
fn direction_uses_layout_axes_and_preserves_upright_sideways_rules() {
    for mode in [
        WritingMode::HorizontalTb,
        WritingMode::VerticalRl,
        WritingMode::VerticalLr,
        WritingMode::SidewaysRl,
        WritingMode::SidewaysLr,
    ] {
        for rtl in [false, true] {
            for upright in [false, true] {
                let mut cv = ComputedValues::initial();
                cv.cssom_writing_mode = mode;
                cv.direction = if rtl { Direction::Rtl } else { Direction::Ltr };
                cv.text_orientation = if upright {
                    TextOrientation::Upright
                } else {
                    TextOrientation::Mixed
                };
                let p = PreparedStyle::new(&cv, StyleMetadata::default(), &[]).unwrap();
                let context = ViewContext {
                    layout_wm: mode,
                    ..ViewContext::default()
                };
                let v = LayoutStyleView::new(&p, context, &[], &[]);
                let forced_ltr =
                    upright && matches!(mode, WritingMode::VerticalRl | WritingMode::VerticalLr);
                let expected_rtl = (rtl && !forced_ltr) != (mode == WritingMode::SidewaysLr);
                assert_eq!(
                    v.direction(),
                    if expected_rtl {
                        taffy::Direction::Rtl
                    } else {
                        taffy::Direction::Ltr
                    }
                );
            }
        }
    }
}

#[test]
fn explicit_auto_alignment_and_wrong_range_calc_are_distinct() {
    let mut cv = ComputedValues::initial();
    cv.align_self = AlignSelfValue::Value(SelfAlignmentValue::LastBaseline);
    cv.position = PositionValue::Absolute;
    let p = PreparedStyle::new(&cv, StyleMetadata::default(), &[]).unwrap();
    let v = LayoutStyleView::new(&p, ViewContext::default(), &[], &[]);
    assert_eq!(v.align_self(), Some(taffy::AlignItems::SAFE_SELF_END));
    let overrides = [PropertyOverride::align_self(None)];
    let v = LayoutStyleView::new(&p, ViewContext::default(), &[], &overrides);
    assert_eq!(v.align_self(), None);
    let payload = AlignedCalc::new(
        CalcLengthPercentage {
            px: -10.0,
            percent: 10.0,
        },
        false,
    );
    assert!(PropertyOverride::length(LengthProperty::Width, LengthValue::calc(&payload)).is_err());
    assert!(
        PropertyOverride::length(LengthProperty::MarginLeft, LengthValue::calc(&payload)).is_ok()
    );
}

#[test]
fn physical_edges_and_logical_minimum_are_not_confused() {
    let mut cv = ComputedValues::initial();
    cv.margin = Sides {
        top: L::Px(1.0),
        right: L::Px(2.0),
        bottom: L::Px(3.0),
        left: L::Px(4.0),
    };
    cv.cssom_writing_mode = WritingMode::VerticalRl;
    cv.min_block_size = Some(L::Px(17.0));
    let p = PreparedStyle::new(&cv, StyleMetadata::default(), &[]).unwrap();
    let physical = LayoutStyleView::new(&p, ViewContext::default(), &[], &[]);
    assert_eq!(
        physical.min_size().height,
        taffy::LengthPercentageAuto::length(17.0)
    );
    assert_eq!(
        physical.margin().left,
        taffy::LengthPercentageAuto::length(4.0)
    );
    let v = LayoutStyleView::new(
        &p,
        ViewContext {
            layout_wm: WritingMode::VerticalRl,
            ..ViewContext::default()
        },
        &[],
        &[],
    );
    assert_eq!(
        v.margin(),
        taffy::Rect {
            left: taffy::LengthPercentageAuto::length(1.0),
            right: taffy::LengthPercentageAuto::length(3.0),
            top: taffy::LengthPercentageAuto::length(2.0),
            bottom: taffy::LengthPercentageAuto::length(4.0)
        }
    );
}
