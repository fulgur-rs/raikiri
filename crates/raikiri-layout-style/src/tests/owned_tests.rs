use super::*;
use std::sync::Arc;
use taffy::{GenericRepetition, GridContainerStyle};

#[test]
fn owned_and_borrowed_getters_agree() {
    let mut cv = ComputedValues::initial();
    let expression = CalcLengthPercentage {
        px: -10.0,
        percent: 25.0,
    };
    cv.width = L::Calc(expression);
    cv.margin.left = L::Calc(expression);
    let positive = AlignedCalc::new(expression, true);
    let signed = AlignedCalc::new(expression, false);
    let bindings = [
        PreparedCalc {
            slot: CalcSlot::Property(LengthProperty::Width),
            payload: &positive,
        },
        PreparedCalc {
            slot: CalcSlot::Property(LengthProperty::MarginLeft),
            payload: &signed,
        },
    ];
    let borrowed = PreparedStyle::new(&cv, StyleMetadata::default(), &bindings).unwrap();
    let owned = PreparedStyle::prepare(&cv, StyleMetadata::default());
    let a = LayoutStyleView::new(&borrowed, ViewContext::default(), &[], &[]);
    let b = LayoutStyleView::new(&owned, ViewContext::default(), &[], &[]);
    for basis in [0.0, 1.0, 37.0, 100.0, 1000.0] {
        assert_eq!(
            borrowed
                .payload(CalcSlot::Property(LengthProperty::Width))
                .resolve(basis),
            owned
                .payload(CalcSlot::Property(LengthProperty::Width))
                .resolve(basis)
        );
        assert_eq!(
            borrowed
                .payload(CalcSlot::Property(LengthProperty::MarginLeft))
                .resolve(basis),
            owned
                .payload(CalcSlot::Property(LengthProperty::MarginLeft))
                .resolve(basis)
        );
    }
    assert_eq!(a.min_size(), b.min_size());
    assert_eq!(a.box_sizing(), b.box_sizing());
    assert_eq!(a.flex_basis(), b.flex_basis());
    assert_eq!(a.flex_grow(), b.flex_grow());
    assert_eq!(a.overflow(), b.overflow());
    macro_rules! same {
        ($trait:ident, $($getter:ident),+ $(,)?) => { $(
            assert_eq!($trait::$getter(&a), $trait::$getter(&b));
        )+ };
    }
    same!(
        CoreStyle,
        box_generation_mode,
        is_block,
        is_compressible_replaced,
        is_replaced,
        box_sizing,
        direction,
        overflow,
        scrollbar_width,
        position,
        inset,
        is_containing_block,
        min_size,
        max_size,
        aspect_ratio,
        padding,
        border,
        contain
    );
    same!(
        FlexboxContainerStyle,
        flex_direction,
        flex_wrap,
        gap,
        align_content,
        align_items,
        justify_content
    );
    same!(
        FlexboxItemStyle,
        flex_basis,
        flex_grow,
        flex_shrink,
        align_self
    );
    for basis in [0.0, 37.0, 100.0] {
        // SAFETY: Each view's prepared owner stays alive throughout these calls.
        #[allow(unsafe_code)]
        unsafe {
            assert_eq!(
                resolve_calc(a.size().width.into_raw().calc_value(), basis),
                resolve_calc(b.size().width.into_raw().calc_value(), basis)
            );
            assert_eq!(
                resolve_calc(a.margin().left.into_raw().calc_value(), basis),
                resolve_calc(b.margin().left.into_raw().calc_value(), basis)
            );
        }
    }
}

#[test]
fn shared_values_and_grid_payloads_survive_external_owner_drop() {
    use raikiri_style::{
        ComputedGridTemplateTracks, ComputedGridTrackBreadth as B, ComputedGridTrackList,
        ComputedGridTrackListComponent as C, ComputedGridTrackRepeat, ComputedGridTrackSize as S,
        property::GridRepeatCount,
    };
    let mut cv = ComputedValues::initial();
    cv.grid_template_columns = ComputedGridTemplateTracks::List(Arc::new(ComputedGridTrackList {
        line_names: vec![vec![], vec![]],
        components: vec![C::Repeat(ComputedGridTrackRepeat {
            count: GridRepeatCount::Count(2),
            line_names: vec![vec![], vec![]],
            tracks: vec![S::MinMax(
                B::Calc(CalcLengthPercentage {
                    px: -5.0,
                    percent: 20.0,
                }),
                B::Calc(CalcLengthPercentage {
                    px: 7.0,
                    percent: 40.0,
                }),
            )],
        })],
    }));
    let owner = Arc::new(cv);
    let weak = Arc::downgrade(&owner);
    let prepared = PreparedStyle::prepare_shared(owner.clone(), StyleMetadata::default());
    drop(owner);
    assert!(weak.upgrade().is_some());
    let view = LayoutStyleView::new(&prepared, ViewContext::default(), &[], &[]);
    let component = view.grid_template_columns().unwrap().next().unwrap();
    let taffy::GenericGridTemplateComponent::Repeat(repeat) = component else {
        panic!("repeat");
    };
    assert_eq!(repeat.count(), taffy::RepetitionCount::Count(2));
    assert_eq!(repeat.tracks().count(), 1);
    let slot = |part| CalcSlot::Grid {
        axis: Axis::X,
        list: GridList::Template,
        component: 0,
        repeat_track: Some(0),
        part,
    };
    assert_eq!(prepared.payload(slot(TrackPart::Min)).resolve(0.0), 0.0);
    assert_eq!(prepared.payload(slot(TrackPart::Min)).resolve(100.0), 15.0);
    assert_eq!(prepared.payload(slot(TrackPart::Max)).resolve(100.0), 47.0);
    drop(prepared);
    assert!(weak.upgrade().is_none());
}

#[test]
fn owned_overrides_keep_calc_range_and_explicit_none() {
    let cv = ComputedValues::initial();
    let prepared = PreparedStyle::prepare(&cv, StyleMetadata::default());
    let pass = [
        OwnedPropertyOverride::length(LengthProperty::Width, L::Px(10.0), false).unwrap(),
        OwnedPropertyOverride::length(LengthProperty::Width, L::Px(30.0), false).unwrap(),
        OwnedPropertyOverride::aspect_ratio(Some(2.0)),
    ];
    let scoped = [
        OwnedPropertyOverride::length(LengthProperty::Width, L::Auto, false).unwrap(),
        OwnedPropertyOverride::aspect_ratio(None),
    ];
    let v =
        LayoutStyleView::with_owned_overrides(&prepared, ViewContext::default(), &pass, &scoped);
    assert_eq!(v.size().width, taffy::Dimension::auto());
    assert_eq!(v.aspect_ratio(), None);
    let v = LayoutStyleView::with_owned_overrides(&prepared, ViewContext::default(), &pass, &[]);
    assert_eq!(v.size().width, taffy::Dimension::length(30.0));
    assert_eq!(v.aspect_ratio(), Some(2.0));
    assert!(
        OwnedPropertyOverride::length(LengthProperty::LogicalWidth, L::Px(1.0), false).is_err()
    );
    let calc = OwnedPropertyOverride::length(
        LengthProperty::Width,
        L::Calc(CalcLengthPercentage {
            px: -10.0,
            percent: 25.0,
        }),
        false,
    )
    .unwrap();
    let borrowed = calc.as_borrowed();
    let v = LayoutStyleView::new(
        &prepared,
        ViewContext::default(),
        std::slice::from_ref(&borrowed),
        &[],
    );
    let pointer = v.size().width.into_raw().calc_value();
    // SAFETY: The override owns this handle and outlives both resolutions.
    #[allow(unsafe_code)]
    unsafe {
        assert_eq!(resolve_calc(pointer, 0.0), 0.0);
        assert_eq!(resolve_calc(pointer, 100.0), 15.0);
    }
}

#[test]
fn resolved_overrides_require_pixel_values() {
    for value in [
        L::Auto,
        L::Percent(10.0),
        L::Calc(CalcLengthPercentage {
            px: 1.0,
            percent: 20.0,
        }),
    ] {
        assert!(OwnedPropertyOverride::length(LengthProperty::PaddingLeft, value, true).is_err());
    }
    assert!(OwnedPropertyOverride::length(LengthProperty::PaddingLeft, L::Px(7.0), true).is_ok());
}
