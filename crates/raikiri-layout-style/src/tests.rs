use super::*;
use crate::values::{
    CalcRef, Dimension, ExpandedDimension, ExpandedLengthPercentage, ExpandedLengthPercentageAuto,
    LengthPercentage, LengthPercentageAuto,
};
use raikiri_layout_geometry::{Edges, Point, Size};
use raikiri_style::ComputedLengthPercentageOrAuto as L;
use raikiri_style::{ComputedValues, property::CalcLengthPercentage};
mod calc_handle_tests;
mod core_tests;
mod inline_box_tests;
mod math_tests;
mod owned_tests;
mod physical_sizing_tests;
mod policy_tests;
mod pr_review_tests;
mod prepared_data_tests;
mod resolve_tests;
mod resolved_length_tests;
mod review_tests;

/// The calc reference of a length that holds one.
trait CalcOf: Copy {
    fn calc_ref(self) -> CalcRef;
}

impl CalcOf for Dimension {
    fn calc_ref(self) -> CalcRef {
        match self.expand() {
            ExpandedDimension::Calc(calc) | ExpandedDimension::FitContentCalc(calc) => calc,
            other => panic!("not a calc: {other:?}"),
        }
    }
}

impl CalcOf for LengthPercentage {
    fn calc_ref(self) -> CalcRef {
        match self.expand() {
            ExpandedLengthPercentage::Calc(calc) => calc,
            other => panic!("not a calc: {other:?}"),
        }
    }
}

impl CalcOf for LengthPercentageAuto {
    fn calc_ref(self) -> CalcRef {
        match self.expand() {
            ExpandedLengthPercentageAuto::Calc(calc) => calc,
            other => panic!("not a calc: {other:?}"),
        }
    }
}

#[test]
fn initial_values_match_bridge() {
    let cv = ComputedValues::initial();
    let p = PreparedStyle::new(&cv, StyleMetadata::default(), &[]).unwrap();
    let v = LayoutStyleView::new(&p, ViewContext::default(), &[], &[]);
    assert_eq!(v.box_sizing(), values::BoxSizing::ContentBox);
    assert_eq!(v.flex_grow(), 0.0);
    assert_eq!(v.flex_shrink(), 1.0);
    assert_eq!(v.flex_basis(), Dimension::auto());
    assert_eq!(
        v.gap(),
        Size {
            width: LengthPercentage::ZERO,
            height: LengthPercentage::ZERO,
        }
    );
}

#[test]
fn scoped_overrides_distinguish_absence_and_last_wins() {
    let mut cv = ComputedValues::initial();
    cv.width = L::Px(100.0);
    let p = PreparedStyle::new(&cv, StyleMetadata::default(), &[]).unwrap();
    let length = |value| LengthValue::computed(value).unwrap();
    let pass = [
        PropertyOverride::length(LengthProperty::Width, length(L::Px(10.0))).unwrap(),
        PropertyOverride::length(LengthProperty::Width, length(L::Px(30.0))).unwrap(),
        PropertyOverride::aspect_ratio(Some(2.0)),
    ];
    let plain = LayoutStyleView::new(&p, ViewContext::default(), &pass, &[]);
    assert_eq!(plain.size().width, Dimension::length(30.0));
    assert_eq!(plain.aspect_ratio(), Some(2.0));
    let scoped = [
        PropertyOverride::length(LengthProperty::Width, length(L::Auto)).unwrap(),
        PropertyOverride::aspect_ratio(None),
    ];
    let v = LayoutStyleView::new(&p, ViewContext::default(), &pass, &scoped);
    assert_eq!(v.size().width, Dimension::auto());
    assert_eq!(v.aspect_ratio(), None);
}

#[test]
fn a_table_that_owns_its_height_hides_the_vertical_sizes_from_the_library() {
    let mut cv = ComputedValues::initial();
    cv.width = L::Px(20.0);
    cv.height = L::Px(40.0);
    cv.min_height = L::Px(10.0);
    cv.max_height = L::Px(50.0);
    cv.top = L::Px(1.0);
    cv.bottom = L::Px(3.0);
    let p = PreparedStyle::new(&cv, StyleMetadata::default(), &[]).unwrap();
    let plain = LayoutStyleView::new(&p, ViewContext::default(), &[], &[]);
    assert_eq!(plain.size().height, Dimension::length(40.0));
    assert_eq!(plain.min_size().height, LengthPercentageAuto::length(10.0));
    assert_eq!(plain.max_size().height, LengthPercentageAuto::length(50.0));
    let hidden = plain.with_hidden_table_height();
    assert_eq!(
        hidden.size(),
        Size {
            width: Dimension::length(20.0),
            height: Dimension::auto()
        }
    );
    assert_eq!(hidden.min_size().height, LengthPercentageAuto::auto());
    assert_eq!(hidden.max_size().height, LengthPercentageAuto::auto());
    // The insets and the width limits are read as ever.
    assert_eq!(hidden.inset(), plain.inset());
    assert_eq!(hidden.min_size().width, plain.min_size().width);
}

#[test]
fn edge_resolution_is_not_repeated_and_axes_are_projected() {
    let mut cv = ComputedValues::initial();
    cv.width = L::Px(20.0);
    cv.height = L::Px(40.0);
    cv.padding.top = raikiri_style::ComputedLengthPercentageOrCalc::Percent(10.0);
    let p = PreparedStyle::new(&cv, StyleMetadata::default(), &[]).unwrap();
    let overrides =
        [
            PropertyOverride::length(LengthProperty::PaddingBottom, LengthValue::resolved(7.0))
                .unwrap(),
        ];
    let context = ViewContext {
        percent_basis: PercentBasis::Definite(200.0),
        ..ViewContext::default()
    };
    let v = LayoutStyleView::new(&p, context, &overrides, &[]);
    assert_eq!(v.padding().top, LengthPercentage::length(20.0));
    assert_eq!(v.padding().bottom, LengthPercentage::length(7.0));
    let context = ViewContext {
        layout_wm: raikiri_style::property::WritingMode::VerticalRl,
        ..context
    };
    let v = LayoutStyleView::new(&p, context, &overrides, &[]);
    assert_eq!(
        v.size(),
        Size {
            width: Dimension::length(40.0),
            height: Dimension::length(20.0)
        }
    );
    assert_eq!(v.padding().left, LengthPercentage::length(20.0));
    assert_eq!(v.padding().right, LengthPercentage::length(7.0));
}

#[test]
fn calc_respects_property_range() {
    let value = CalcLengthPercentage {
        px: -10.0,
        percent: 25.0,
    };
    let signed = AlignedCalc::new(value, false);
    let positive = AlignedCalc::new(value, true);
    assert_eq!((signed.resolve(0.0), signed.resolve(100.0)), (-10.0, 15.0));
    assert_eq!(
        (positive.resolve(0.0), positive.resolve(100.0)),
        (0.0, 15.0)
    );
    // The layout library keeps the handle with its low three bits clear.
    assert_eq!(signed.handle() & 0b111, 0);
    assert_ne!(signed.handle(), 0);
    assert_ne!(signed.handle(), positive.handle());
}

#[test]
fn prepared_style_rejects_wrong_bindings() {
    let mut cv = ComputedValues::initial();
    let expr = CalcLengthPercentage {
        px: 2.0,
        percent: 10.0,
    };
    cv.width = raikiri_style::ComputedLengthPercentageOrAuto::Calc(expr);
    let metadata = StyleMetadata::default();
    assert!(PreparedStyle::new(&cv, metadata, &[]).is_err());
    let payload = AlignedCalc::new(expr, true);
    let binding = PreparedCalc {
        slot: CalcSlot::Property(LengthProperty::Width),
        payload: &payload,
    };
    assert!(PreparedStyle::new(&cv, metadata, &[binding]).is_ok());
    assert!(PreparedStyle::new(&cv, metadata, &[binding, binding]).is_err());
    let signed = AlignedCalc::new(expr, false);
    assert!(
        PreparedStyle::new(
            &cv,
            metadata,
            &[PreparedCalc {
                payload: &signed,
                ..binding
            }]
        )
        .is_err()
    );
    let wrong = AlignedCalc::new(CalcLengthPercentage { px: 3.0, ..expr }, true);
    assert!(
        PreparedStyle::new(
            &cv,
            metadata,
            &[PreparedCalc {
                payload: &wrong,
                ..binding
            }]
        )
        .is_err()
    );
}
mod pass_tests;

mod block_tests;
mod bridge_projection_tests;

pub(crate) fn comparison(
    source: &str,
) -> std::sync::Arc<raikiri_style::property::ComputedLengthMath> {
    use raikiri_style::property::{LengthMath, LengthOrAuto};
    use raikiri_style::{ComputedLength, ResolveContext, resolve_size_length_percentage_or_auto};
    let mut input = cssparser::ParserInput::new(source);
    let mut parser = cssparser::Parser::new(&mut input);
    let specified = LengthMath::parse(&mut parser).expect("valid comparison fixture");
    parser.expect_exhausted().unwrap();
    let value = resolve_size_length_percentage_or_auto(
        LengthOrAuto::Math(specified),
        ComputedLength(16.0),
        None,
        &ResolveContext::new(ComputedLength(16.0)),
    );
    let L::Math(value) = value else {
        panic!("percentage tree must survive")
    };
    value
}
