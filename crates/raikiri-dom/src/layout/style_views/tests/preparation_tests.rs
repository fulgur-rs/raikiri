use super::*;
use crate::node::arena::BoxKey;
use taffy::CoreStyle;

#[test]
fn all_key_preparation_includes_hidden_oof_flattened_and_unrelated_boxes() {
    let (mut doc, _, oof, text) =
        fixture_with_styles("display:flex", "position:absolute;width:31px;height:7px");
    let body = doc.nodes[oof].parent.unwrap();
    let html = doc.nodes[body].parent.unwrap();
    let unrelated = doc.append_element(Some(html), "aside", Some("display:block"));
    let hidden = doc.append_element(Some(body), "div", Some("display:none;width:17px"));
    let contents = doc.append_element(Some(body), "div", Some("display:contents"));
    let flattened = doc.append_element(Some(contents), "div", Some("width:23px"));
    doc.mark_in_document_flags();
    let cascade = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
    assert_eq!(
        session.style_view(oof).size().width,
        taffy::Dimension::length(31.0)
    );
    assert_eq!(
        session.style_view(hidden).size().width,
        taffy::Dimension::length(17.0)
    );
    assert_eq!(
        session.style_view(flattened).size().width,
        taffy::Dimension::length(23.0)
    );
    assert!(
        session
            .style_views
            .boxes
            .contains_key(&BoxKey::Source(text))
    );
    assert!(
        session
            .style_views
            .boxes
            .contains_key(&BoxKey::Source(unrelated))
    );
    assert!(session.style_views.boxes.contains_key(&BoxKey::Source(0)));
}

#[test]
fn pass_overrides_survive_reparenting_between_block_and_flex() {
    let (mut doc, _, child, _) = fixture_with_styles("display:block", "width:31px;height:7px");
    let body = doc.nodes[child].parent.unwrap();
    let flex = doc.append_element(Some(body), "div", Some("display:flex"));
    doc.mark_in_document_flags();
    let cascade = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
    assert!(
        session
            .style_views
            .boxes
            .contains_key(&BoxKey::Source(child))
    );
    session.record_pass_override(
        child,
        OwnedPropertyOverride::length(LengthProperty::Width, L::Px(47.0), false).unwrap(),
    );
    session.document.append_child(flex, child).unwrap();
    session.with_cache_frame(child, raikiri_layout_style::ViewContext::default(), |_| {});
    assert_eq!(
        session.style_view(child).size().width,
        taffy::Dimension::length(47.0)
    );
    session.document.append_child(body, child).unwrap();
    session.with_cache_frame(child, raikiri_layout_style::ViewContext::default(), |_| {});
    assert!(
        session
            .style_views
            .boxes
            .contains_key(&BoxKey::Source(child))
    );
    session.record_pass_override(
        child,
        OwnedPropertyOverride::length(LengthProperty::Width, L::Px(53.0), false).unwrap(),
    );
    session.document.append_child(flex, child).unwrap();
    session.with_cache_frame(child, raikiri_layout_style::ViewContext::default(), |_| {});
    assert_eq!(
        session.style_view(child).size().width,
        taffy::Dimension::length(53.0)
    );
}

#[test]
fn prepared_edge_scopes_keep_both_query_restoration_and_epoch_notifications() {
    let (mut doc, cascade, child, _) =
        fixture_with_styles("display:block", "width:31px;height:7px;padding-left:10%");
    let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
    let marker = session.record_style_overrides(child, Vec::new());
    assert!(marker.is_some());
    session.record_scoped_length(child, LengthProperty::Width, L::Px(99.0), true);
    session.restore_recorded_style_overrides(marker);
    let epoch = session.layout_cache_context.epoch();
    session.with_resolved_edges(child, 200.0, false, |session| {
        assert_eq!(
            session.style_view(child).padding().left,
            taffy::LengthPercentage::length(20.0)
        );
    });
    assert_eq!(
        session.style_view(child).padding().left,
        taffy::LengthPercentage::percent(0.1)
    );
    assert_ne!(session.layout_cache_context.epoch(), epoch);
    assert!(session.has_style_view(child));
    assert_eq!(
        session.style_view(child).padding().left,
        taffy::LengthPercentage::percent(0.1)
    );
}

#[test]
fn preparation_rejects_live_override_tokens_without_losing_them() {
    let (mut doc, cascade, child, _) = fixture();
    let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
    let token = session.push_style_overrides(
        child,
        vec![OwnedPropertyOverride::length(LengthProperty::Width, L::Px(99.0), true).unwrap()],
    );
    session.with_cache_frame(child, raikiri_layout_style::ViewContext::default(), |_| {});
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        session.prepare_style_views()
    }));
    assert!(result.is_err());
    assert_eq!(
        session.style_view(child).size().width,
        taffy::Dimension::length(99.0)
    );
    session.restore_style_overrides(token);
    assert_eq!(
        session.style_view(child).size().width,
        taffy::Dimension::length(31.0)
    );
}

#[test]
fn publication_refreshes_metadata_before_parent_cache_lookup() {
    use raikiri_style::property::{DisplayValue, PositionValue};
    let (mut doc, cascade, child, _) = fixture();
    let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
    session.with_cache_frame(child, raikiri_layout_style::ViewContext::default(), |_| {});
    assert_eq!(
        session.style_view(child).box_generation_mode(),
        taffy::BoxGenerationMode::Normal
    );
    assert!(!session.document.layout_dirty);
    session
        .document
        .set_box_properties(child, DisplayValue::None, PositionValue::Static);
    session.document.layout_cache_context.publish_metadata();
    assert!(!session.document.layout_dirty);
    session.with_cache_frame(
        child,
        raikiri_layout_style::ViewContext::default(),
        |session| {
            assert_eq!(
                session.style_view(child).box_generation_mode(),
                taffy::BoxGenerationMode::None
            );
        },
    );
}

#[test]
fn publication_is_prepared_before_size_scope_target_selection() {
    let (mut doc, _, child, _) = fixture_with_styles("display:block", "width:31px");
    let body = doc.nodes[child].parent.unwrap();
    let flex = doc.append_element(Some(body), "div", Some("display:flex"));
    doc.mark_in_document_flags();
    let cascade = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
    session.document.append_child(flex, child).unwrap();
    let marker = session.record_style_overrides(child, Vec::new());
    assert!(marker.is_some());
    session.record_scoped_length(child, LengthProperty::Width, L::Px(99.0), true);
    session.with_cache_frame(
        child,
        raikiri_layout_style::ViewContext::default(),
        |session| {
            assert_eq!(
                session.style_view(child).size().width,
                taffy::Dimension::length(99.0)
            );
        },
    );
    session.restore_recorded_style_overrides(marker);
    assert_eq!(
        session.style_view(child).size().width,
        taffy::Dimension::length(31.0)
    );
}

#[test]
fn publication_is_prepared_before_resolved_edge_basis_is_installed() {
    let (mut doc, cascade, child, _) = fixture_with_styles("display:flex", "padding-left:10%");
    let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
    session.document.layout_cache_context.publish_metadata();
    session.with_resolved_edges(child, 200.0, false, |session| {
        session.with_cache_frame(
            child,
            raikiri_layout_style::ViewContext::default(),
            |session| {
                assert_eq!(
                    session.style_view(child).padding().left,
                    taffy::LengthPercentage::length(20.0)
                );
            },
        );
    });
    assert_eq!(
        session.style_view(child).padding().left,
        taffy::LengthPercentage::percent(0.1)
    );
}

#[test]
fn relayout_discards_old_oof_coordinates_after_style_and_owner_changes() {
    for (css, reparent) in [
        ("position:static;width:31px;height:7px", false),
        (
            "display:none;position:absolute;width:31px;height:7px",
            false,
        ),
        ("position:absolute;width:31px;height:7px", true),
    ] {
        let (mut doc, _, child, _) =
            fixture_with_styles("display:flex", "position:absolute;width:31px;height:7px");
        let body = doc.nodes[child].parent.unwrap();
        let owner = doc.append_element(
            Some(body),
            "div",
            Some("position:relative;display:block;width:100px;height:20px"),
        );
        doc.mark_in_document_flags();
        let first = cascade(&doc, &build_rule_tree(&doc)).unwrap();
        let page = crate::layout::test_support::page_box_800x600();
        crate::layout_single_page(&mut doc, &first, page).unwrap();
        assert!(doc.oof_coordinates.contains_key(&child));
        doc.set_element_attribute(child, "style", css).unwrap();
        if reparent {
            doc.append_child(owner, child).unwrap();
        }
        let second = cascade(&doc, &build_rule_tree(&doc)).unwrap();
        crate::layout_single_page(&mut doc, &second, page).unwrap();
        if reparent {
            assert_eq!(doc.oof_coordinates[&child].owner, owner);
        } else {
            assert!(
                !doc.oof_coordinates.contains_key(&child),
                "stale OOF entry: {css}"
            );
        }
    }
}

#[test]
fn metadata_publication_inside_edge_scope_is_rejected_and_restored() {
    let (mut doc, cascade, child, _) = fixture_with_styles("display:flex", "padding-left:10%");
    let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
    session.with_resolved_edges(child, 200.0, false, |session| {
        let direct = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            session.prepare_style_views();
        }));
        assert!(direct.is_err());
        session.document.layout_cache_context.publish_metadata();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            session.with_cache_frame(child, raikiri_layout_style::ViewContext::default(), |_| {});
        }));
        assert!(result.is_err());
        assert_eq!(
            session.style_view(child).padding().left,
            taffy::LengthPercentage::length(20.0)
        );
    });
    session.with_cache_frame(child, raikiri_layout_style::ViewContext::default(), |_| {});
    assert_eq!(
        session.style_view(child).padding().left,
        taffy::LengthPercentage::percent(0.1)
    );
}

#[test]
fn batch_defers_queries_until_pending_values_and_icb_are_ready() {
    let (mut doc, cascade, child, _) = fixture();
    let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
    session
        .with_style_preparation_batch(|session| {
            apply_computed_to_style_in_session(session, &cascade).unwrap();
            session.record_pass_override(
                child,
                OwnedPropertyOverride::length(LengthProperty::Width, L::Px(47.0), false).unwrap(),
            );
            let before = session.layout_cache_context.epoch();
            for operation in 0..3 {
                let result =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match operation {
                        0 => {
                            let _ = session.style_view(child);
                        }
                        1 => {
                            session.with_cache_frame(
                                child,
                                raikiri_layout_style::ViewContext::default(),
                                |_| {},
                            );
                        }
                        _ => {
                            session.with_resolved_edges(child, 200.0, false, |_| {});
                        }
                    }));
                assert!(result.is_err());
                assert_eq!(session.layout_cache_context.epoch(), before);
            }
            super::super::super::initial_containing_block::prepare(session, &cascade, 123.0, 456.0);
            session.begin_oof_layout();
            Ok::<_, ()>(())
        })
        .unwrap();
    assert!(!session.document.layout_dirty);
    assert_eq!(
        session.style_view(child).size().width,
        taffy::Dimension::length(47.0)
    );
}

#[test]
fn batch_error_and_unwind_keep_previous_calc_owners_until_repair() {
    use std::sync::Arc;
    for unwind in [false, true] {
        let (mut doc, cascade, child, _) =
            fixture_with_styles("display:flex", "width:calc(50% + 10px)");
        let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
        apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
        let handle = session
            .style_view(child)
            .size()
            .width
            .into_raw()
            .calc_value();
        let owner = Arc::downgrade(
            session
                .box_style_data
                .calc_values
                .iter()
                .find(|owner| Arc::as_ptr(owner).cast::<()>() == handle)
                .unwrap(),
        );
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            session.with_style_preparation_batch(|session| {
                apply_computed_to_style_in_session(session, &cascade).unwrap();
                if unwind {
                    panic!("batch interruption");
                }
                Err::<(), _>(())
            })
        }));
        if unwind {
            assert!(result.is_err());
        } else {
            assert!(result.unwrap().is_err());
        }
        assert!(owner.upgrade().is_some());
        assert!(!session.box_style_data.is_ready());
        assert!(session.style_views.prepared_stamp.is_none());
        let query = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = session.style_view(child);
        }));
        assert!(query.is_err());
        session
            .with_style_preparation_batch(|session| {
                apply_computed_to_style_in_session(session, &cascade).unwrap();
                assert!(owner.upgrade().is_some());
                session.begin_oof_layout();
                Ok::<_, ()>(())
            })
            .unwrap();
        assert!(owner.upgrade().is_none());
        assert_eq!(
            crate::layout::dispatch::resolve_calc(
                session
                    .style_view(child)
                    .size()
                    .width
                    .into_raw()
                    .calc_value(),
                100.0
            ),
            60.0
        );
    }
}
