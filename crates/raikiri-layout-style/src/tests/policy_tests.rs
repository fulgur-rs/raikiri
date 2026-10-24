use super::*;
use raikiri_style::{
    ComputedFlexBasis as F, ComputedLengthPercentageOrCalc as P,
    ComputedLengthPercentageOrNormal as N, property::*,
};

#[test]
fn preparation_only_slots_cannot_be_used_as_physical_overrides() {
    for property in [
        LengthProperty::LogicalWidth,
        LengthProperty::LogicalHeight,
        LengthProperty::MinBlock,
    ] {
        assert!(PropertyOverride::length(property, LengthValue::resolved(3.0)).is_err());
    }
}

#[test]
fn box_identity_position_containment_and_overflow_policies_are_explicit() {
    let mut cv = ComputedValues::initial();
    let flags = StyleFlags::HIDDEN
        .union(StyleFlags::REPLACED)
        .union(StyleFlags::COMPRESSIBLE_REPLACED);
    for (position, expected) in [
        (PositionValue::Static, taffy::Position::Static),
        (PositionValue::Relative, taffy::Position::Relative),
        (PositionValue::Sticky, taffy::Position::Sticky),
        (PositionValue::Absolute, taffy::Position::Absolute),
        (PositionValue::Fixed, taffy::Position::Absolute),
    ] {
        cv.position = position;
        for (contain, expected_contain) in [
            (Contain::NONE, taffy::Contain::NONE),
            (Contain::LAYOUT, taffy::Contain::LAYOUT),
            (Contain::PAINT, taffy::Contain::PAINT),
            (
                Contain::LAYOUT.union(Contain::PAINT),
                taffy::Contain::CONTENT,
            ),
        ] {
            cv.contain = contain;
            let p = PreparedStyle::new(
                &cv,
                StyleMetadata {
                    flags,
                    ..StyleMetadata::default()
                },
                &[],
            )
            .unwrap();
            let v = LayoutStyleView::new(&p, ViewContext::default(), &[], &[]);
            assert_eq!(v.position(), expected);
            assert_eq!(v.contain(), expected_contain);
            assert_eq!(v.box_generation_mode(), taffy::BoxGenerationMode::None);
            assert!(v.is_replaced() && v.is_compressible_replaced());
        }
    }
    for (overflow, expected) in [
        (OverflowValue::Visible, taffy::Overflow::Visible),
        (OverflowValue::Hidden, taffy::Overflow::Hidden),
        (OverflowValue::Clip, taffy::Overflow::Clip),
        (OverflowValue::Auto, taffy::Overflow::Scroll),
        (OverflowValue::Scroll, taffy::Overflow::Scroll),
    ] {
        cv.overflow.x = overflow;
        cv.overflow.y = overflow;
        let p = PreparedStyle::new(&cv, StyleMetadata::default(), &[]).unwrap();
        let v = LayoutStyleView::new(&p, ViewContext::default(), &[], &[]);
        assert_eq!(
            v.overflow(),
            taffy::Point {
                x: expected,
                y: expected
            }
        );
    }
    cv.box_sizing = BoxSizing::BorderBox;
    let p = PreparedStyle::new(&cv, StyleMetadata::default(), &[]).unwrap();
    assert_eq!(
        LayoutStyleView::new(&p, ViewContext::default(), &[], &[]).box_sizing(),
        taffy::BoxSizing::BorderBox
    );
}

#[test]
fn logical_sources_keyword_policy_and_flex_basis_keep_their_meaning() {
    let mut cv = ComputedValues::initial();
    cv.width = L::MinContent;
    cv.vertical_logical_size = Some(raikiri_style::VerticalLogicalSize {
        width: L::Px(30.0),
        height: L::Px(50.0),
    });
    for (flags, width) in [
        (
            StyleFlags::VERTICAL_LOGICAL_SIZE,
            taffy::Dimension::length(30.0),
        ),
        (StyleFlags::MIN_CONTENT_WIDTH_AUTO, taffy::Dimension::auto()),
    ] {
        let p = PreparedStyle::new(
            &cv,
            StyleMetadata {
                flags,
                ..StyleMetadata::default()
            },
            &[],
        )
        .unwrap();
        let v = LayoutStyleView::new(&p, ViewContext::default(), &[], &[]);
        assert_eq!(v.size().width, width);
        let _ = v.min_size();
    }
    for (input, expected) in [
        (F::Content, taffy::Dimension::content()),
        (F::MinContent, taffy::Dimension::min_content()),
        (F::MaxContent, taffy::Dimension::max_content()),
        (F::FitContent, taffy::Dimension::fit_content()),
        (F::Px(4.0), taffy::Dimension::length(4.0)),
        (F::Percent(25.0), taffy::Dimension::percent(0.25)),
    ] {
        cv.flex_basis = input;
        let p = PreparedStyle::new(&cv, StyleMetadata::default(), &[]).unwrap();
        assert_eq!(
            LayoutStyleView::new(&p, ViewContext::default(), &[], &[]).flex_basis(),
            expected
        );
    }
    cv.width = L::Px(2.0);
    cv.height = L::Px(3.0);
    let p = PreparedStyle::new(&cv, StyleMetadata::default(), &[]).unwrap();
    assert_eq!(
        LayoutStyleView::new(&p, ViewContext::default(), &[], &[]).aspect_ratio(),
        None
    );
}

#[test]
fn properties_read_as_is_ignore_every_keyword_and_logical_size_policy() {
    let mut cv = ComputedValues::initial();
    cv.width = L::MinContent;
    cv.height = L::MaxContent;
    cv.min_width = L::FitContent;
    cv.max_height = L::MinContent;
    cv.margin.top = L::MaxContent;
    cv.top = L::FitContent;
    cv.flex_basis = F::MinContent;
    cv.vertical_logical_size = Some(raikiri_style::VerticalLogicalSize {
        width: L::Px(30.0),
        height: L::Px(50.0),
    });
    cv.min_block_size = Some(L::MinContent);
    let policies = StyleFlags::IGNORE_WIDTH_KEYWORD
        .union(StyleFlags::IGNORE_HEIGHT_KEYWORD)
        .union(StyleFlags::IGNORE_LIMIT_KEYWORDS)
        .union(StyleFlags::MIN_CONTENT_WIDTH_AUTO)
        .union(StyleFlags::IGNORE_LOGICAL_MIN_BLOCK)
        .union(StyleFlags::VERTICAL_LOGICAL_SIZE);
    for flags in [StyleFlags::EMPTY, policies] {
        let p = PreparedStyle::new(
            &cv,
            StyleMetadata {
                flags,
                ..StyleMetadata::default()
            },
            &[],
        )
        .unwrap();
        let v = LayoutStyleView::new(&p, ViewContext::default(), &[], &[]);
        for &property in LengthProperty::ALL {
            if property.reads_scalar_directly() {
                assert!(
                    v.base_length(property).value == p.scalar(property),
                    "{property:?} keeps its own scalar under {flags:?}"
                );
            }
        }
    }
}

#[test]
#[allow(unsafe_code)]
fn scalar_calc_slots_resolve_values_with_the_right_range() {
    let expr = CalcLengthPercentage {
        px: -10.0,
        percent: 25.0,
    };
    let positive = AlignedCalc::new(expr, true);
    let signed = AlignedCalc::new(expr, false);
    let mut cv = ComputedValues::initial();
    cv.width = L::FitContentOf(P::Calc(expr));
    cv.min_width = L::Calc(expr);
    cv.left = L::Calc(expr);
    cv.margin.top = L::Calc(expr);
    cv.padding.top = P::Calc(expr);
    cv.flex_basis = F::Calc(expr);
    cv.column_gap = N::Calc(expr);
    cv.row_gap = N::Percent(25.0);
    let bindings = [
        (LengthProperty::Width, &positive),
        (LengthProperty::MinWidth, &positive),
        (LengthProperty::Left, &signed),
        (LengthProperty::MarginTop, &signed),
        (LengthProperty::PaddingTop, &positive),
        (LengthProperty::FlexBasis, &positive),
        (LengthProperty::ColumnGap, &positive),
    ]
    .map(|(p, payload)| PreparedCalc {
        slot: CalcSlot::Property(p),
        payload,
    });
    let p = PreparedStyle::new(&cv, StyleMetadata::default(), &bindings).unwrap();
    let v = LayoutStyleView::new(&p, ViewContext::default(), &[], &[]);
    let handles = [
        (v.size().width.into_raw().calc_value(), true),
        (v.min_size().width.into_raw().calc_value(), true),
        (v.inset().left.into_raw().calc_value(), false),
        (v.margin().top.into_raw().calc_value(), false),
        (v.padding().top.into_raw().calc_value(), true),
        (v.flex_basis().into_raw().calc_value(), true),
        (v.gap().width.into_raw().calc_value(), true),
    ];
    for (handle, range) in handles {
        for basis in [0.0, 100.0] {
            let expected = if range {
                (basis * 0.25 - 10.0_f32).max(0.0)
            } else {
                basis * 0.25 - 10.0
            };
            assert_eq!(CalcHandle::resolve(handle, basis), expected);
        }
    }
    assert_eq!(
        AlignedCalc::new(
            CalcLengthPercentage {
                px: f32::INFINITY,
                percent: 0.0
            },
            true
        )
        .resolve(0.0),
        0.0
    );
    assert!(LengthValue::computed(L::Calc(expr)).is_err());
    assert!(format!("{}", OverrideError::UnpreparedCalc).contains("override"));
    assert!(format!("{}", PrepareError::Unexpected).contains("bindings"));
    assert!(
        PreparedStyle::new(
            &ComputedValues::initial(),
            StyleMetadata::default(),
            &bindings[..1]
        )
        .is_err()
    );
}

#[test]
fn empty_grid_iterators_and_absent_areas_keep_the_legacy_representation() {
    let mut cv = ComputedValues::initial();
    cv.column_gap = N::Px(4.0);
    cv.align_self = AlignSelfValue::Auto;
    let p = PreparedStyle::new(&cv, StyleMetadata::default(), &[]).unwrap();
    let v = LayoutStyleView::new(&p, ViewContext::default(), &[], &[]);
    assert_eq!(v.grid_template_columns().unwrap().len(), 0);
    assert!(v.grid_template_areas().is_none());
    assert_eq!(
        (
            v.grid_template_area_row_count(),
            v.grid_template_area_column_count()
        ),
        (0, 0)
    );
    assert_eq!(v.align_self(), None);
    assert_eq!(v.gap().width, taffy::LengthPercentage::length(4.0));
    cv.grid_template_columns = raikiri_style::ComputedGridTemplateTracks::List(
        std::sync::Arc::new(raikiri_style::ComputedGridTrackList {
            components: vec![],
            line_names: vec![],
        }),
    );
    let p = PreparedStyle::new(&cv, StyleMetadata::default(), &[]).unwrap();
    let v = LayoutStyleView::new(&p, ViewContext::default(), &[], &[]);
    assert_eq!(v.grid_template_columns().unwrap().len(), 0);
    assert_eq!(v.grid_template_column_names().unwrap().len(), 0);
    for input in [L::Percent(25.0), L::FitContentOf(P::Px(7.0))] {
        cv.width = input;
        let p = PreparedStyle::new(&cv, StyleMetadata::default(), &[]).unwrap();
        let v = LayoutStyleView::new(&p, ViewContext::default(), &[], &[]);
        assert_ne!(v.size().width, taffy::Dimension::auto());
    }
}

#[test]
fn abspos_first_baseline_and_defensive_edge_fallbacks_are_stable() {
    let mut cv = ComputedValues::initial();
    cv.position = PositionValue::Fixed;
    cv.align_self = AlignSelfValue::Value(SelfAlignmentValue::Baseline);
    cv.margin.left = L::MinContent;
    let p = PreparedStyle::new(&cv, StyleMetadata::default(), &[]).unwrap();
    let overrides = [PropertyOverride::length(
        LengthProperty::PaddingTop,
        LengthValue::computed(L::Auto).unwrap(),
    )
    .unwrap()];
    for basis in [PercentBasis::Taffy, PercentBasis::Definite(200.0)] {
        let v = LayoutStyleView::new(
            &p,
            ViewContext {
                percent_basis: basis,
                ..ViewContext::default()
            },
            &overrides,
            &[],
        );
        assert_eq!(
            v.align_self(),
            Some(taffy::AlignItems {
                keyword: taffy::AlignItemsKeyword::Baseline,
                safety: taffy::AlignmentSafety::Safe
            })
        );
        assert_eq!(v.padding().top, taffy::LengthPercentage::length(0.0));
        assert_eq!(v.margin().left, taffy::LengthPercentageAuto::length(0.0));
    }
}
