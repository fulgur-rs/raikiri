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
fn cell_minimum_height_is_auto_but_other_boxes_keep_it() {
    for display in [
        DisplayValue::TableCell,
        DisplayValue::Table,
        DisplayValue::Block,
    ] {
        let mut cv = ComputedValues::initial();
        cv.display = display;
        cv.min_width = L::Px(12.0);
        cv.min_height = L::Px(40.0);
        cv.height = L::Px(20.0);
        let flags = if display == DisplayValue::TableCell {
            StyleFlags::TABLE_CELL
        } else {
            StyleFlags::default()
        };
        query(&cv, flags, |v| {
            assert_eq!(v.min_size().width, LengthPercentageAuto::length(12.0));
            assert_eq!(
                v.min_size().height,
                if display == DisplayValue::TableCell {
                    LengthPercentageAuto::auto()
                } else {
                    LengthPercentageAuto::length(40.0)
                }
            );
            assert_eq!(v.size().height, Dimension::length(20.0));
            let transposed = v.with_context(ViewContext {
                bridge_projection: BridgeProjection::BlockTranspose {
                    right_to_left: false,
                },
                ..ViewContext::default()
            });
            assert_eq!(transposed.min_size().width, v.min_size().height);
            assert_eq!(transposed.min_size().height, v.min_size().width);
        });
    }
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
    assert_eq!(v.flex_basis(), values::Dimension::length(33.0));
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
            assert_eq!(v.size().width, values::Dimension::length(expected));
            assert_eq!(
                v.margin().left,
                values::LengthPercentageAuto::percent(expected)
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
        cv.width = value.clone();
        cv.min_width = value.clone();
        cv.max_height = value.clone();
        cv.margin.left = value;
        query(&cv, StyleFlags::default(), |v| {
            assert_ne!(v.size().width, values::Dimension::auto());
            assert_eq!(v.min_size().width, values::LengthPercentageAuto::auto());
            assert_eq!(v.max_size().height, values::LengthPercentageAuto::auto());
            // A margin never computes to a keyword; one that gets here is no margin.
            assert_eq!(v.margin().left, values::LengthPercentageAuto::length(0.0));
        });
        query(&cv, StyleFlags::IGNORE_WIDTH_KEYWORD, |v| {
            assert_eq!(v.size().width, values::Dimension::auto())
        });
    }
}

#[test]
fn the_main_axis_of_a_flex_container_runs_along_rows_for_row_directions() {
    for (input, expected) in [
        (FlexDirectionValue::Row, true),
        (FlexDirectionValue::RowReverse, true),
        (FlexDirectionValue::Column, false),
        (FlexDirectionValue::ColumnReverse, false),
    ] {
        let mut cv = ComputedValues::initial();
        cv.flex_direction = input;
        query(&cv, StyleFlags::default(), |v| {
            assert_eq!(v.flex_direction_is_row(), expected)
        });
    }
}

#[test]
fn all_flex_and_alignment_keywords_have_explicit_results() {
    for (input, expected) in [
        (FlexWrapValue::NoWrap, false),
        (FlexWrapValue::Wrap, true),
        (FlexWrapValue::WrapReverse, true),
    ] {
        let mut cv = ComputedValues::initial();
        cv.flex_wrap = input;
        query(&cv, StyleFlags::default(), |v| {
            assert_eq!(v.flex_wraps(), expected);
        });
    }
    for (input, expected) in [
        (ContentAlignmentValue::Normal, values::AlignContent::NORMAL),
        (
            ContentAlignmentValue::Stretch,
            values::AlignContent::STRETCH,
        ),
        (
            ContentAlignmentValue::SpaceBetween,
            values::AlignContent::SPACE_BETWEEN,
        ),
        (
            ContentAlignmentValue::SpaceAround,
            values::AlignContent::SPACE_AROUND,
        ),
        (
            ContentAlignmentValue::SpaceEvenly,
            values::AlignContent::SPACE_EVENLY,
        ),
        (ContentAlignmentValue::Center, values::AlignContent::CENTER),
        (ContentAlignmentValue::Start, values::AlignContent::START),
        (ContentAlignmentValue::End, values::AlignContent::END),
        (
            ContentAlignmentValue::FlexStart,
            values::AlignContent::FLEX_START,
        ),
        (
            ContentAlignmentValue::FlexEnd,
            values::AlignContent::FLEX_END,
        ),
        (
            ContentAlignmentValue::Safe(ContentPosition::Center),
            values::AlignContent {
                safety: values::AlignmentSafety::Safe,
                ..values::AlignContent::CENTER
            },
        ),
        (
            ContentAlignmentValue::Unsafe(ContentPosition::FlexEnd),
            values::AlignContent {
                safety: values::AlignmentSafety::Unsafe,
                ..values::AlignContent::FLEX_END
            },
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
        (SelfAlignmentValue::Normal, values::AlignItems::NORMAL),
        (SelfAlignmentValue::Stretch, values::AlignItems::STRETCH),
        (SelfAlignmentValue::Center, values::AlignItems::CENTER),
        (SelfAlignmentValue::Start, values::AlignItems::START),
        (SelfAlignmentValue::End, values::AlignItems::END),
        (
            SelfAlignmentValue::FlexStart,
            values::AlignItems::FLEX_START,
        ),
        (SelfAlignmentValue::FlexEnd, values::AlignItems::FLEX_END),
        (SelfAlignmentValue::Baseline, values::AlignItems::BASELINE),
        (
            SelfAlignmentValue::LastBaseline,
            values::AlignItems::BASELINE,
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
                        values::Direction::Rtl
                    } else {
                        values::Direction::Ltr
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
    assert_eq!(v.align_self(), Some(values::AlignItems::SAFE_SELF_END));
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
        values::LengthPercentageAuto::length(17.0)
    );
    assert_eq!(
        physical.margin().left,
        values::LengthPercentageAuto::length(4.0)
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
        Edges {
            left: values::LengthPercentageAuto::length(1.0),
            right: values::LengthPercentageAuto::length(3.0),
            top: values::LengthPercentageAuto::length(2.0),
            bottom: values::LengthPercentageAuto::length(4.0)
        }
    );
}

#[test]
fn a_grid_item_defers_ratio_height_without_changing_its_own_ratio_or_alignment() {
    let mut cv = ComputedValues::initial();
    cv.aspect_ratio.auto = false;
    cv.aspect_ratio.ratio = Some((2.0, 1.0));
    query(&cv, StyleFlags::default(), |own| {
        assert_eq!(own.aspect_ratio(), Some(2.0));
        assert_eq!(own.bridge_align_self(), None);
        for alignment in [values::AlignSelf::START, values::AlignSelf::STRETCH] {
            let parent = own.with_context(ViewContext {
                grid_item_ratio: GridItemRatio::MeasuredByItem {
                    align_self: alignment,
                    justify_self: values::AlignSelf::START,
                },
                ..ViewContext::default()
            });
            assert_eq!(parent.aspect_ratio(), None);
            assert_eq!(parent.stated_aspect_ratio(), Some(2.0));
            assert_eq!(parent.block_aspect_ratio(), Some(2.0));
            assert_eq!(parent.bridge_align_self(), Some(alignment));
        }
        assert_eq!(own.aspect_ratio(), Some(2.0));
        assert_eq!(own.bridge_align_self(), None);
    });
}

#[test]
fn a_replaced_grid_item_keeps_its_identity_and_bounds_while_sizing_its_own_ratio() {
    let mut cv = ComputedValues::initial();
    cv.aspect_ratio.auto = false;
    cv.aspect_ratio.ratio = Some((2.0, 1.0));
    cv.height = L::Px(10.0);
    cv.min_width = L::Px(20.0);
    cv.max_width = L::Px(30.0);
    query(&cv, StyleFlags::REPLACED, |own| {
        let parent = own.with_context(ViewContext {
            grid_item_ratio: GridItemRatio::Replaced,
            ..ViewContext::default()
        });
        assert!(parent.is_replaced());
        assert_eq!(own.aspect_ratio(), Some(2.0));
        assert_eq!(parent.aspect_ratio(), None);
        assert_eq!(parent.block_aspect_ratio(), Some(2.0));
        assert_eq!(parent.size(), own.size());
        assert_eq!(parent.min_size(), own.min_size());
        assert_eq!(parent.max_size(), own.max_size());
        assert_eq!(parent.bridge_align_self(), own.bridge_align_self());
        assert_eq!(parent.bridge_justify_self(), own.bridge_justify_self());
    });
}

#[test]
fn late_padding_changes_only_its_projected_edge_of_resolved_padding() {
    let cv = ComputedValues::initial();
    let prepared = PreparedStyle::new(&cv, StyleMetadata::default(), &[]).unwrap();
    let resolved = raikiri_layout_geometry::Edges {
        top: 11.0,
        right: 13.0,
        bottom: 17.0,
        left: 19.0,
    };
    let baseline =
        [OwnedPropertyOverride::length(LengthProperty::PaddingTop, L::Px(91.0), true).unwrap()];
    let late =
        [OwnedPropertyOverride::length(LengthProperty::PaddingTop, L::Px(7.0), true).unwrap()];
    for (mode, projection, expected) in [
        (
            WritingMode::HorizontalTb,
            BridgeProjection::Identity,
            raikiri_layout_geometry::Edges {
                top: 7.0,
                ..resolved
            },
        ),
        (
            WritingMode::VerticalRl,
            BridgeProjection::Identity,
            raikiri_layout_geometry::Edges {
                left: 7.0,
                ..resolved
            },
        ),
        (
            WritingMode::VerticalLr,
            BridgeProjection::BlockTranspose {
                right_to_left: false,
            },
            raikiri_layout_geometry::Edges {
                top: 7.0,
                ..resolved
            },
        ),
        (
            WritingMode::VerticalRl,
            BridgeProjection::BlockTranspose {
                right_to_left: true,
            },
            raikiri_layout_geometry::Edges {
                bottom: 7.0,
                ..resolved
            },
        ),
    ] {
        let context = ViewContext {
            layout_wm: mode,
            bridge_projection: projection,
            resolved_padding: Some(resolved),
            ..ViewContext::default()
        };
        assert_eq!(
            LayoutStyleView::with_owned_overrides(&prepared, context, &baseline, &[]).padding(),
            resolved.map(LengthPercentage::length)
        );
        assert_eq!(
            LayoutStyleView::with_owned_overrides(&prepared, context, &baseline, &late).padding(),
            expected.map(LengthPercentage::length)
        );
    }
}
