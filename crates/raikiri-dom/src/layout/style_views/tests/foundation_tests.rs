use super::*;
use crate::layout::box_style_data::LayoutBase;
use crate::node::arena::BoxKey;

#[test]
fn initial_explicit_preparation_fills_non_element_foundations_without_a_bridge() {
    let (mut doc, cascade, child, text) = fixture();
    let sources = cascade.computed.len();
    let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
    session.prepare_style_views();
    assert!(session.box_style_data.is_ready());
    for source in [0, text] {
        let record = session.box_style_data.get(BoxKey::Source(source)).unwrap();
        assert_eq!(
            record.recipe,
            Some(crate::layout::box_style_data::recipes::RecipeKind::NeutralSource)
        );
        assert!(matches!(record.base, LayoutBase::Synthetic(_)));
        assert_eq!(
            record.metadata.flags,
            raikiri_layout_style::StyleFlags::TEXT
        );
    }
    assert_eq!(
        session.style_view(child).size().width,
        taffy::Dimension::length(31.0)
    );
    assert_eq!(cascade.computed.len(), sources);
}

#[test]
fn new_session_refreshes_contents_recipe_from_current_source_direction() {
    use raikiri_style::property::Direction;
    let (mut doc, mut cascade, child, _) =
        fixture_with_styles("display:flex", "display:contents;direction:ltr");
    {
        let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
        apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
        let record = session.box_style_data.get(BoxKey::Source(child)).unwrap();
        let LayoutBase::Synthetic(values) = &record.base else {
            panic!("contents recipe")
        };
        assert_eq!(values.direction, Direction::Ltr);
    }
    let address = cascade.computed.as_ptr();
    let generation = cascade.generation();
    cascade.computed[child].direction = Direction::Rtl;
    let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
    session.prepare_style_views();
    assert_eq!(cascade.computed.as_ptr(), address);
    assert_eq!(cascade.generation(), generation);
    assert!(session.box_style_data.is_ready());
    let record = session.box_style_data.get(BoxKey::Source(child)).unwrap();
    let LayoutBase::Synthetic(values) = &record.base else {
        panic!("contents recipe")
    };
    assert_eq!(values.direction, Direction::Rtl);
}

#[test]
fn new_session_refreshes_spanner_recipe_from_current_source_direction() {
    use raikiri_style::property::Direction;
    let (mut doc, _, wrapper, _) = fixture_with_styles(
        "display:block;column-count:2;width:120px",
        "display:block;direction:ltr",
    );
    doc.append_element(
        Some(wrapper),
        "div",
        Some("display:block;column-span:all;height:20px"),
    );
    doc.mark_in_document_flags();
    let mut cascade = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    {
        let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
        apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
        assert_eq!(
            session
                .box_style_data
                .get(BoxKey::Source(wrapper))
                .unwrap()
                .recipe,
            Some(crate::layout::box_style_data::recipes::RecipeKind::Spanner)
        );
    }
    cascade.computed[wrapper].direction = Direction::Rtl;
    let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
    session.prepare_style_views();
    let LayoutBase::Synthetic(values) = &session
        .box_style_data
        .get(BoxKey::Source(wrapper))
        .unwrap()
        .base
    else {
        panic!("spanner recipe")
    };
    assert_eq!(values.direction, Direction::Rtl);
}

#[test]
fn new_session_refreshes_anonymous_recipes_from_their_explicit_owners() {
    use raikiri_style::property::{Direction, WritingMode};
    let (mut doc, _, owner, _) = fixture_with_styles("display:block", "display:block");
    let row = doc.append_element(Some(owner), "div", Some("display:table-row"));
    doc.append_text(row, "A");
    doc.mark_in_document_flags();
    let mut cascade = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    crate::layout::layout_single_page(
        crate::layout::test_support::with_ahem(&mut doc),
        &cascade,
        crate::layout::test_support::page_box_800x600(),
    )
    .unwrap();
    let owners = [
        doc.table_objects.tables[0].parent,
        doc.table_objects.cells[0].owner,
    ];
    for (i, &owner) in owners.iter().enumerate() {
        cascade.computed[owner].direction = Direction::Rtl;
        doc.nodes[owner].used_writing_mode = WritingMode::VerticalRl;
        doc.nodes[owner].border_spacing.horizontal = raikiri_style::ComputedLength(23.0 + i as f32);
    }
    let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
    session.prepare_style_views();
    for (i, key) in [BoxKey::AnonymousTable(0), BoxKey::AnonymousCell(0)]
        .into_iter()
        .enumerate()
    {
        let record = session.box_style_data.get(key).unwrap();
        let LayoutBase::Synthetic(values) = &record.base else {
            panic!("anonymous recipe")
        };
        assert_eq!(values.direction, Direction::Rtl);
        assert_eq!(values.border_spacing.horizontal.0, 23.0 + i as f32);
        assert_eq!(record.metadata.used_writing_mode, WritingMode::VerticalRl);
    }
}

#[test]
fn new_session_refreshes_icb_principal_facts_and_retains_published_dimensions() {
    use raikiri_style::property::{Direction, WritingMode};
    let (mut doc, mut cascade, _, _) = fixture();
    let mut page = crate::layout::test_support::page_box_800x600();
    page.width = 80.0;
    page.height = 120.0;
    crate::layout::layout_single_page(
        crate::layout::test_support::with_ahem(&mut doc),
        &cascade,
        page,
    )
    .unwrap();
    for value in &mut cascade.computed {
        value.direction = Direction::Rtl;
    }
    for node in doc.nodes.iter_mut() {
        node.used_writing_mode = WritingMode::VerticalLr;
    }
    let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
    session.prepare_style_views();
    let record = session
        .box_style_data
        .get(BoxKey::InitialContainingBlock)
        .unwrap();
    let LayoutBase::Synthetic(values) = &record.base else {
        panic!("ICB recipe")
    };
    assert_eq!(values.direction, Direction::Rtl);
    assert_eq!(values.width, L::Px(80.0));
    assert_eq!(values.height, L::Px(120.0));
    assert_eq!(record.metadata.used_writing_mode, WritingMode::VerticalLr);
}

#[test]
fn neutral_recipe_refreshes_metadata_without_replacing_shared_initial_values() {
    use raikiri_style::property::WritingMode;
    let (mut doc, cascade, _, text) = fixture();
    let original = {
        let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
        apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
        let LayoutBase::Synthetic(values) = &session
            .box_style_data
            .get(BoxKey::Source(text))
            .unwrap()
            .base
        else {
            panic!("neutral recipe")
        };
        values.clone()
    };
    doc.nodes[text].used_writing_mode = WritingMode::VerticalRl;
    let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
    session.prepare_style_views();
    let record = session.box_style_data.get(BoxKey::Source(text)).unwrap();
    let LayoutBase::Synthetic(values) = &record.base else {
        panic!("neutral recipe")
    };
    assert!(std::sync::Arc::ptr_eq(values, &original));
    assert_eq!(record.metadata.used_writing_mode, WritingMode::VerticalRl);
}

#[test]
fn same_recipe_metadata_refresh_preserves_document_pass_entries() {
    use raikiri_style::property::WritingMode;
    let (mut doc, cascade, child, _) = fixture_with_styles("display:flex", "display:contents");
    let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
    session.record_pass_override(child, OwnedPropertyOverride::aspect_ratio(Some(2.0)));
    session.nodes[child].used_writing_mode = WritingMode::VerticalRl;
    session.document.layout_cache_context.publish_metadata();
    session.refresh_style_views();
    let record = session.box_style_data.get(BoxKey::Source(child)).unwrap();
    assert_eq!(record.pass.len(), 1);
    assert_eq!(record.metadata.used_writing_mode, WritingMode::VerticalRl);
}

#[test]
fn changed_anonymous_owner_discards_the_previous_identity_pass_entries() {
    let (mut doc, _, owner, _) = fixture_with_styles("display:block", "display:block");
    let row = doc.append_element(Some(owner), "div", Some("display:table-row"));
    doc.append_text(row, "A");
    doc.mark_in_document_flags();
    let cascade = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    crate::layout::layout_single_page(
        crate::layout::test_support::with_ahem(&mut doc),
        &cascade,
        crate::layout::test_support::page_box_800x600(),
    )
    .unwrap();
    let table = doc.nodes.anonymous_table_key(0);
    let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
    session.prepare_style_views();
    session.record_pass_override(table, OwnedPropertyOverride::aspect_ratio(Some(2.0)));
    assert_eq!(
        session
            .box_style_data
            .get(BoxKey::AnonymousTable(0))
            .unwrap()
            .pass
            .len(),
        1
    );
    session.table_objects.tables[0].parent = row;
    session.document.layout_cache_context.publish_metadata();
    session.refresh_style_views();
    let record = session
        .box_style_data
        .get(BoxKey::AnonymousTable(0))
        .unwrap();
    assert!(record.pass.is_empty());
    assert_eq!(
        record.recipe,
        Some(crate::layout::box_style_data::recipes::RecipeKind::AnonymousTable { owner: row })
    );
}

#[test]
fn changed_wrapper_identity_requires_bridge_and_keeps_old_recipe_until_recovery() {
    use raikiri_style::property::DisplayValue;
    let (mut doc, mut cascade, child, _) = fixture_with_styles("display:flex", "display:contents");
    let old = {
        let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
        apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
        let LayoutBase::Synthetic(values) = &session
            .box_style_data
            .get(BoxKey::Source(child))
            .unwrap()
            .base
        else {
            panic!("contents recipe")
        };
        std::sync::Arc::downgrade(values)
    };
    cascade.computed[child].display = DisplayValue::Block;
    let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        session.prepare_style_views()
    }));
    assert!(failed.is_err());
    assert!(!session.box_style_data.is_ready());
    assert!(old.upgrade().is_some());
    apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
    assert!(session.box_style_data.is_ready());
    assert!(old.upgrade().is_none());
    assert!(
        matches!(session.box_style_data.get(BoxKey::Source(child)).unwrap().base, LayoutBase::Source(source) if source == child)
    );
}

#[test]
fn mismatched_recipe_provenance_cannot_publish_ready_foundations() {
    use crate::layout::box_style_data::recipes;
    for case in [
        "source-base",
        "missing-origin",
        "table-key",
        "cell-key",
        "wrapper-key",
        "icb-key",
        "neutral-kind",
    ] {
        let (mut doc, cascade, child, _) = fixture();
        {
            let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
            apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
            crate::layout::initial_containing_block::prepare(
                session.document,
                &cascade,
                80.0,
                120.0,
            );
            session.prepare_style_views();
        }
        let facts = recipes::BoxFacts {
            direction: taffy::Direction::Ltr,
            metadata: Default::default(),
        };
        match case {
            "missing-origin" => {
                let record = doc.box_style_data.get_mut(BoxKey::Source(child)).unwrap();
                record.base = LayoutBase::Synthetic(std::sync::Arc::new(
                    raikiri_style::ComputedValues::initial(),
                ));
            }
            "source-base" => {
                doc.box_style_data
                    .get_mut(BoxKey::Source(child))
                    .unwrap()
                    .recipe = Some(recipes::RecipeKind::Contents);
            }
            "table-key" | "cell-key" => {
                let table_facts = recipes::TableFacts::from_owner(&doc, &cascade, child, child);
                let recipe = if case == "table-key" {
                    recipes::anonymous_table(table_facts)
                } else {
                    recipes::anonymous_cell(table_facts)
                };
                doc.box_style_data
                    .publish_recipe(BoxKey::Source(child), recipe);
            }
            "wrapper-key" => doc
                .box_style_data
                .publish_recipe(BoxKey::InitialContainingBlock, recipes::contents(facts)),
            "icb-key" => doc.box_style_data.publish_recipe(
                BoxKey::Source(child),
                recipes::initial_containing_block(80.0, 120.0, facts),
            ),
            "neutral-kind" => doc
                .box_style_data
                .publish_recipe(BoxKey::Source(child), recipes::neutral_source(facts)),
            _ => unreachable!(),
        }
        let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
        let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            session.prepare_style_views()
        }));
        assert!(failed.is_err(), "{case}");
        assert!(!session.box_style_data.is_ready(), "{case}");
    }
}

#[test]
fn all_live_boxes_share_document_foundations_and_query_targets() {
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
    assert!(session.has_style_view(child));
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
    let spanner = doc.append_element(Some(wrapper), "div", Some("display:block;height:20px"));
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
    session.record_pass_override(
        wrapper,
        OwnedPropertyOverride::clear(raikiri_style::property::ClearValue::None),
    );
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
        &crate::LayoutStyle {
            display: taffy::Display::Block,
            ..crate::LayoutStyle::default()
        },
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
    let later = doc.append_element(Some(body), "div", Some("width:7px"));
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
