use super::*;
use raikiri_layout_style::{
    AlignedCalc, LayoutStyleView, PreparedStyle, StyleMetadata, ViewContext,
};
use raikiri_style::{ComputedValues, property::CalcLengthPercentage};
use std::sync::Arc;
use taffy::CoreStyle;

#[test]
#[allow(unsafe_code)]
fn legacy_and_view_calc_handles_share_the_resolver() {
    for expression in [
        CalcLengthPercentage {
            px: -20.0,
            percent: 50.0,
        },
        CalcLengthPercentage {
            px: f32::INFINITY,
            percent: 25.0,
        },
        CalcLengthPercentage {
            px: 5.0,
            percent: f32::NAN,
        },
    ] {
        for non_negative in [true, false] {
            let mut owners: Vec<Arc<AlignedCalc>> = Vec::new();
            let legacy = crate::taffy_impl::push_calc(&mut owners, expression, non_negative);
            assert_eq!(owners[0].expression().px.to_bits(), expression.px.to_bits());
            assert_eq!(owners[0].non_negative(), non_negative);
            let mut values = ComputedValues::initial();
            if non_negative {
                values.width = L::Calc(expression);
            } else {
                values.margin.left = L::Calc(expression);
            }
            let prepared = PreparedStyle::prepare(&values, StyleMetadata::default());
            let view = LayoutStyleView::new(&prepared, ViewContext::default(), &[], &[]);
            let handle = if non_negative {
                view.size().width.into_raw().calc_value()
            } else {
                view.margin().left.into_raw().calc_value()
            };
            for basis in [0.0, 1.0, 37.0, 100.0, f32::INFINITY, f32::NAN] {
                let expected = owners[0].resolve(basis);
                assert_eq!(crate::taffy_impl::resolve_calc(legacy, basis), expected);
                assert_eq!(crate::taffy_impl::resolve_calc(handle, basis), expected);
                // SAFETY: The prepared view retains this payload throughout resolution.
                assert_eq!(
                    unsafe { raikiri_layout_style::resolve_calc(handle, basis) },
                    expected
                );
            }
        }
    }
}

#[test]
fn session_drop_preserves_retained_handle_owner() {
    let (mut doc, cascade, child, _) =
        fixture_with_styles("display:flex", "width:calc(50% + 10px);height:7px");
    let (handle, owner) = {
        let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
        apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
        let handle = session
            .style_view(child)
            .size()
            .width
            .into_raw()
            .calc_value();
        let owner = session
            .document
            .retained_layout_styles
            .calc_values
            .iter()
            .find(|owner| Arc::as_ptr(owner).cast::<()>() == handle)
            .map(Arc::downgrade)
            .expect("the document retains the exact payload before the handle escapes");
        (handle, owner)
    };
    let retained = owner.upgrade().expect("the document outlives its session");
    assert_eq!(crate::taffy_impl::resolve_calc(handle, 100.0), 60.0);
    drop(retained);
    drop(doc);
    assert!(owner.upgrade().is_none());
}

#[test]
fn rebridge_failure_keeps_old_handle_alive() {
    use crate::layout::ifc::test_support::ahem_fonts;
    use shodo::limits::Limits;
    let (mut doc, cascade, child, _) = fixture_with_styles(
        "display:flex;font-family:Ahem;font-size:10px",
        "width:calc(50% + 10px);height:7px",
    );
    doc.set_font_collection_with_limits(ahem_fonts(), Limits::default());
    let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
    let handle = session
        .style_view(child)
        .size()
        .width
        .into_raw()
        .calc_value();
    let owner = session
        .document
        .retained_layout_styles
        .calc_values
        .iter()
        .find(|owner| Arc::as_ptr(owner).cast::<()>() == handle)
        .map(Arc::downgrade)
        .expect("the published view has a document owner");
    session.document.set_font_collection_with_limits(
        ahem_fonts(),
        Limits {
            max_shaped_glyphs: Some(1),
            ..Limits::default()
        },
    );
    let result = apply_computed_to_style_in_session(&mut session, &cascade);
    assert!(result.is_err());
    let retained = owner
        .upgrade()
        .expect("a failed bridge retains the old owner prefix");
    assert_eq!(crate::taffy_impl::resolve_calc(handle, 100.0), 60.0);
    drop(retained);
    session
        .document
        .set_font_collection_with_limits(ahem_fonts(), Limits::default());
    apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
    assert!(owner.upgrade().is_none());
}

#[test]
fn popped_override_handle_outlives_the_session() {
    let (mut doc, cascade, child, _) = fixture();
    let (handle, owner) = {
        let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
        apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
        let token = session.push_style_overrides(
            child,
            vec![
                OwnedPropertyOverride::length(
                    LengthProperty::Width,
                    L::Calc(CalcLengthPercentage {
                        px: 2.0,
                        percent: 20.0,
                    }),
                    false,
                )
                .unwrap(),
            ],
        );
        let handle = session
            .style_view(child)
            .size()
            .width
            .into_raw()
            .calc_value();
        let owner = session
            .document
            .retained_layout_styles
            .calc_values
            .iter()
            .find(|owner| Arc::as_ptr(owner).cast::<()>() == handle)
            .map(Arc::downgrade)
            .expect("the document retains a scoped payload before publication");
        session.restore_style_overrides(token);
        assert_eq!(
            session.style_view(child).size().width,
            taffy::Dimension::length(31.0)
        );
        (handle, owner)
    };
    let retained = owner
        .upgrade()
        .expect("popping the scope does not invalidate an escaped handle");
    assert_eq!(crate::taffy_impl::resolve_calc(handle, 100.0), 22.0);
    drop(retained);
    drop(doc);
    assert!(owner.upgrade().is_none());
}

#[test]
fn rebridge_unwind_keeps_old_handle_alive() {
    let (mut doc, cascade, child, _) =
        fixture_with_styles("display:flex", "width:calc(50% + 10px);height:7px");
    let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
    let handle = session
        .style_view(child)
        .size()
        .width
        .into_raw()
        .calc_value();
    let owner = session
        .document
        .retained_layout_styles
        .calc_values
        .iter()
        .find(|owner| Arc::as_ptr(owner).cast::<()>() == handle)
        .map(Arc::downgrade)
        .unwrap();
    session.nodes[child].children.push(usize::MAX - 1);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
    }));
    assert!(result.is_err());
    let retained = owner
        .upgrade()
        .expect("unwinding leaves the old prefix retained");
    assert_eq!(crate::taffy_impl::resolve_calc(handle, 100.0), 60.0);
    drop(retained);
    session.nodes[child].children.pop();
    apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
    assert!(owner.upgrade().is_none());
}

#[test]
fn pass_and_recorded_scope_calcs_get_document_owners() {
    for scoped in [false, true] {
        let (mut doc, cascade, child, _) = fixture();
        let (handle, owner) = {
            let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
            apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
            let value = L::Calc(CalcLengthPercentage {
                px: 2.0,
                percent: 20.0,
            });
            let token = if scoped {
                let token = session.push_style_overrides(child, Vec::new());
                session.record_scoped_length(child, LengthProperty::Width, value, false);
                token
            } else {
                session.record_pass_override(
                    child,
                    OwnedPropertyOverride::length(LengthProperty::Width, value, false).unwrap(),
                );
                None
            };
            let handle = session
                .style_view(child)
                .size()
                .width
                .into_raw()
                .calc_value();
            let owner = session
                .document
                .retained_layout_styles
                .calc_values
                .iter()
                .find(|owner| Arc::as_ptr(owner).cast::<()>() == handle)
                .map(Arc::downgrade)
                .expect("each override writer retains its payload before publication");
            session.restore_style_overrides(token);
            (handle, owner)
        };
        let retained = owner.upgrade().unwrap();
        assert_eq!(crate::taffy_impl::resolve_calc(handle, 100.0), 22.0);
        drop(retained);
        drop(doc);
        assert!(owner.upgrade().is_none());
    }
}
