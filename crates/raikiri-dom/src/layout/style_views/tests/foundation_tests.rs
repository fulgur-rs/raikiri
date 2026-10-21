use super::*;
use crate::layout::box_style_data::LayoutBase;
use crate::node::arena::BoxKey;

#[test]
fn all_live_boxes_have_document_foundations_without_expanding_query_targets() {
    let (mut doc, cascade, child, _) = fixture_with_styles("display:block", "width:31px");
    let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
    assert!(session.box_style_data.is_ready());
    for idx in session.nodes.layout_keys() {
        let key = session.nodes.box_key(idx).unwrap();
        let record = session
            .box_style_data
            .get(key)
            .expect("every live box has a foundation");
        assert_eq!(record.stamp, session.box_style_data.stamp());
        if idx == child {
            assert!(matches!(record.base, LayoutBase::Source(source) if source == child));
        }
    }
    assert!(!session.has_style_view(child));
    session.record_pass_override(child, OwnedPropertyOverride::aspect_ratio(Some(2.0)));
    assert_eq!(
        session
            .box_style_data
            .get(BoxKey::Source(child))
            .unwrap()
            .pass
            .len(),
        1
    );
}

#[test]
fn new_session_rejects_cache_queries_until_explicit_binding() {
    let (mut doc, cascade, child, _) = fixture();
    {
        let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
        apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
    }
    let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
    let query = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        session.with_cache_frame(child, Default::default(), |_| {});
    }));
    assert!(query.is_err());
    assert!(!session.box_style_data.is_ready());
    session.prepare_style_views();
    assert!(session.box_style_data.is_ready());
    session.with_cache_frame(child, Default::default(), |_| {});
}

#[test]
fn same_generation_and_address_rebind_changed_scalar_and_calc_values() {
    for (first, second) in [
        (L::Px(10.0), L::Px(20.0)),
        (
            L::Calc(raikiri_style::property::CalcLengthPercentage {
                px: 10.0,
                percent: 0.5,
            }),
            L::Calc(raikiri_style::property::CalcLengthPercentage {
                px: 20.0,
                percent: 0.25,
            }),
        ),
    ] {
        let (mut doc, mut cascade, child, _) = fixture();
        let generation = cascade.generation();
        let address = &cascade as *const _;
        cascade.computed[child].width = first;
        {
            let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
            apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
        }
        cascade.computed[child].width = second;
        assert_eq!(cascade.generation(), generation);
        assert_eq!(&cascade as *const _, address);
        let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
        session.prepare_style_views();
        let width = session.style_view(child).size().width;
        match second {
            L::Px(value) => assert_eq!(width, taffy::Dimension::length(value)),
            L::Calc(value) => assert_eq!(
                crate::taffy_impl::resolve_calc(width.into_raw().calc_value(), 100.0),
                value.px + value.percent
            ),
            _ => unreachable!(),
        }
    }
}

#[test]
fn new_session_discards_previous_pass_entries_before_rebinding() {
    let (mut doc, cascade, child, _) = fixture();
    {
        let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
        apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
        session.record_pass_override(
            child,
            OwnedPropertyOverride::length(LengthProperty::Width, L::Px(99.0), false).unwrap(),
        );
        assert_eq!(
            session.style_view(child).size().width,
            taffy::Dimension::length(99.0)
        );
    }
    let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
    session.prepare_style_views();
    assert!(
        session
            .box_style_data
            .get(BoxKey::Source(child))
            .unwrap()
            .pass
            .is_empty()
    );
    assert_eq!(
        session.style_view(child).size().width,
        taffy::Dimension::length(31.0)
    );
}

#[test]
fn metadata_refresh_shares_document_prepared_data_and_pass_entries() {
    use std::sync::Arc;
    let (mut doc, cascade, child, _) =
        fixture_with_styles("display:flex", "width:calc(50% + 10px)");
    let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
    let data = session
        .box_style_data
        .get(BoxKey::Source(child))
        .unwrap()
        .prepared
        .clone();
    assert!(Arc::strong_count(&data) >= 3);
    session.record_pass_override(child, OwnedPropertyOverride::aspect_ratio(Some(2.0)));
    session.document.layout_cache_context.publish_metadata();
    session.with_cache_frame(child, Default::default(), |_| {});
    let record = session.box_style_data.get(BoxKey::Source(child)).unwrap();
    assert!(Arc::ptr_eq(&data, &record.prepared));
    assert_eq!(record.pass.len(), 1);
    assert_eq!(session.style_view(child).aspect_ratio(), Some(2.0));
}

#[test]
fn source_wrapper_rebinds_when_the_same_key_becomes_a_spanner_recipe() {
    use taffy::GridContainerStyle;
    let (mut doc, _, wrapper, _) = fixture_with_styles(
        "display:block;column-count:2;width:120px",
        "display:block;grid-template-columns:7px 11px",
    );
    let spanner = doc.append_element(
        Some(wrapper),
        "div",
        LayoutStyle::default(),
        Some("display:block;height:20px"),
    );
    doc.mark_in_document_flags();
    let cascade = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
    super::prepare_for_inspection(&mut session);
    assert!(matches!(
        session
            .box_style_data
            .get(BoxKey::Source(wrapper))
            .unwrap()
            .base,
        LayoutBase::Source(_)
    ));
    assert_eq!(
        session
            .style_view(wrapper)
            .grid_template_columns()
            .unwrap()
            .count(),
        2
    );
    session.record_pass_override(wrapper, OwnedPropertyOverride::aspect_ratio(Some(2.0)));
    session.nodes[spanner].column_span_all = true;
    crate::layout::spanner_wrappers::lift_spanner_wrappers(&mut session, &cascade);
    super::prepare_for_inspection(&mut session);
    let record = session.box_style_data.get(BoxKey::Source(wrapper)).unwrap();
    assert!(matches!(record.base, LayoutBase::Synthetic(_)));
    assert_eq!(
        record.recipe,
        Some(crate::layout::box_style_data::recipes::RecipeKind::Spanner)
    );
    assert!(record.pass.is_empty());
    assert_eq!(
        session
            .style_view(wrapper)
            .grid_template_columns()
            .unwrap()
            .count(),
        0
    );
    assert_eq!(session.style_view(wrapper).aspect_ratio(), None);
    assert_neutral_getters(
        session.style_view(wrapper),
        session.layout_style_for_key(wrapper).unwrap(),
    );
}

fn assert_fresh_copy_rebridges(
    mut copy: Document,
    child: usize,
    old_data: &std::sync::Arc<raikiri_layout_style::PreparedData>,
) {
    assert!(!copy.box_style_data.is_ready());
    assert!(copy.box_style_data.get(BoxKey::Source(child)).is_none());
    copy.mark_in_document_flags();
    let cascade = cascade(&copy, &build_rule_tree(&copy)).unwrap();
    let mut session = LayoutSession::with_cascade(&mut copy, &cascade);
    apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
    assert!(session.box_style_data.is_ready());
    let record = session.box_style_data.get(BoxKey::Source(child)).unwrap();
    assert!(!std::sync::Arc::ptr_eq(&record.prepared, old_data));
    assert!(record.pass.is_empty());
    let pointer = session
        .style_view(child)
        .size()
        .width
        .into_raw()
        .calc_value();
    assert_eq!(crate::taffy_impl::resolve_calc(pointer, 100.0), 60.0);
}

#[test]
fn cloned_and_extracted_documents_rebuild_their_own_foundations() {
    let (mut doc, cascade, child, _) =
        fixture_with_styles("display:flex", "width:calc(50% + 10px)");
    let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
    session.record_pass_override(child, OwnedPropertyOverride::aspect_ratio(Some(2.0)));
    let data = session
        .box_style_data
        .get(BoxKey::Source(child))
        .unwrap()
        .prepared
        .clone();
    drop(session);
    let copies = [
        doc.clone(),
        doc.extract_nodes(&(0..doc.node_count()).collect::<Vec<_>>()),
    ];
    drop(doc);
    for copy in copies {
        assert_fresh_copy_rebridges(copy, child, &data);
    }
}

#[cfg(feature = "logical-snapshot")]
#[test]
fn snapshot_document_rebuilds_foundations_without_old_bindings() {
    let (mut doc, cascade, child, _) =
        fixture_with_styles("display:flex", "width:calc(50% + 10px)");
    let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
    let data = session
        .box_style_data
        .get(BoxKey::Source(child))
        .unwrap()
        .prepared
        .clone();
    drop(session);
    let copy =
        Document::from_logical_snapshot(doc.logical_snapshot(), doc.node_count() + 1).unwrap();
    drop(doc);
    assert_fresh_copy_rebridges(copy, child, &data);
}

#[test]
fn invalidated_box_identity_publishes_before_parent_cache_lookup() {
    use raikiri_style::property::{DisplayValue, PositionValue};
    let (mut doc, cascade, child, _) = fixture();
    let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
    session.with_cache_frame(child, Default::default(), |_| {});
    let publication = session.layout_cache_context.publication_epoch();
    session
        .document
        .set_box_properties(child, DisplayValue::None, PositionValue::Static);
    session.with_cache_frame(child, Default::default(), |session| {
        assert_ne!(
            session.layout_cache_context.publication_epoch(),
            publication
        );
        assert_eq!(
            session.style_view(child).box_generation_mode(),
            taffy::BoxGenerationMode::None
        );
    });
}

#[test]
fn failed_rebinding_keeps_replaced_prepared_data_until_full_recovery() {
    use std::sync::Arc;
    let (mut doc, _, child, _) = fixture_with_styles("display:flex", "width:calc(50% + 10px)");
    let body = doc.nodes[child].parent.unwrap();
    let later = doc.append_element(Some(body), "div", LayoutStyle::default(), Some("width:7px"));
    doc.mark_in_document_flags();
    let mut cascade = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    let old = {
        let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
        apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
        Arc::downgrade(
            &session
                .box_style_data
                .get(BoxKey::Source(child))
                .unwrap()
                .prepared,
        )
    };
    cascade.computed[child].width = L::Calc(raikiri_style::property::CalcLengthPercentage {
        px: 20.0,
        percent: 25.0,
    });
    // A later missing value interrupts preparation after the first calc rebind.
    let tail = cascade.computed.split_off(later);
    {
        let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
        let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            session.prepare_style_views()
        }));
        assert!(interrupted.is_err());
        assert!(!session.box_style_data.is_ready());
        assert!(old.upgrade().is_some());
    }
    cascade.computed.extend(tail);
    let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
    session.prepare_style_views();
    assert!(session.box_style_data.is_ready());
    assert!(old.upgrade().is_none());
    let pointer = session
        .style_view(child)
        .size()
        .width
        .into_raw()
        .calc_value();
    assert_eq!(crate::taffy_impl::resolve_calc(pointer, 100.0), 45.0);
}
