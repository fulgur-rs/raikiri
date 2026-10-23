use super::*;

#[test]
fn old_saved_cache_is_not_restored_into_new_epoch() {
    let mut doc = Document::new();
    let session_cascade = crate::layout::test_support::cascaded(&doc);
    let mut tree = LayoutSession::bridged(&mut doc, &session_cascade);
    let id = protocol::NodeId::from(0usize);
    let input = layout_input(10.0);
    tree.with_cache_frame(0, ViewContext::default(), |tree| {
        tree.cache_store(id, &input, LayoutOutput::HIDDEN)
    });
    let saved = tree.take_layout_cache(0);
    tree.layout_cache_context.invalidate();
    tree.restore_layout_cache(0, saved);
    assert!(tree.cache_get(id, &input).is_none());
}

#[test]
fn stale_cache_capture_in_current_epoch_stays_stale_after_restore() {
    let mut doc = Document::new();
    let session_cascade = crate::layout::test_support::cascaded(&doc);
    let mut tree = LayoutSession::bridged(&mut doc, &session_cascade);
    let id = protocol::NodeId::from(0usize);
    let input = layout_input(10.0);
    tree.with_cache_frame(0, ViewContext::default(), |tree| {
        tree.cache_store(id, &input, LayoutOutput::HIDDEN)
    });
    tree.layout_cache_context.invalidate();
    let saved = tree.take_layout_cache(0);
    tree.restore_layout_cache(0, saved);
    assert!(tree.cache_get(id, &input).is_none());
}

#[test]
fn valid_saved_cache_restores_only_its_own_original_entries() {
    let mut doc = Document::new();
    let session_cascade = crate::layout::test_support::cascaded(&doc);
    let mut tree = LayoutSession::bridged(&mut doc, &session_cascade);
    let id = protocol::NodeId::from(0usize);
    let a = layout_input(10.0);
    let b = layout_input(20.0);
    tree.with_cache_frame(0, ViewContext::default(), |tree| {
        tree.cache_store(id, &a, LayoutOutput::HIDDEN)
    });
    let saved = tree.take_layout_cache(0);
    tree.with_cache_frame(0, ViewContext::default(), |tree| {
        tree.cache_store(id, &b, LayoutOutput::HIDDEN)
    });
    tree.restore_layout_cache(0, saved);
    assert!(tree.cache_get(id, &a).is_some());
    assert!(tree.cache_get(id, &b).is_none());
}

#[test]
fn synthetic_rebuild_invalidates_parent() {
    let mut doc = Document::new();
    let session_cascade = crate::layout::test_support::cascaded(&doc);
    let mut tree = LayoutSession::bridged(&mut doc, &session_cascade);
    let id = protocol::NodeId::from(0usize);
    let input = layout_input(10.0);
    for kind in 0..3 {
        tree.with_cache_frame(0, ViewContext::default(), |tree| {
            tree.cache_store(id, &input, LayoutOutput::HIDDEN)
        });
        assert!(tree.cache_get(id, &input).is_some());
        match kind {
            0 => tree.document.set_anonymous_tables(Vec::new()),
            1 => tree.document.set_anonymous_cells(Vec::new()),
            _ => {
                crate::layout::initial_containing_block::prepare(
                    tree.document,
                    &session_cascade,
                    100.0,
                    100.0,
                );
            }
        }
        assert!(tree.cache_get(id, &input).is_none());
    }
}

#[test]
fn nested_scope_success_error_unwind_restores_values() {
    use taffy::CoreStyle;
    let (mut doc, cascade, child, _) = crate::layout::style_views::tests::fixture();
    let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
    crate::layout::bridge::apply_computed_to_style_in_session(&mut tree, &cascade).unwrap();
    let parent = 0;
    let id = protocol::NodeId::from(parent);
    let input = layout_input(10.0);
    let original = tree.style_view(child).size();
    let override_width = |width| {
        vec![
            raikiri_layout_style::OwnedPropertyOverride::length(
                raikiri_layout_style::LengthProperty::Width,
                raikiri_style::ComputedLengthPercentageOrAuto::Px(width),
                true,
            )
            .unwrap(),
        ]
    };
    tree.with_cache_frame(parent, ViewContext::default(), |tree| {
        tree.cache_store(id, &input, LayoutOutput::HIDDEN)
    });
    tree.with_style_overrides(child, override_width(20.0), |tree| {
        assert!(tree.cache_get(id, &input).is_none());
        let error: Result<(), ()> =
            tree.with_style_overrides(child, override_width(30.0), |_| Err(()));
        assert!(error.is_err());
        assert_eq!(
            tree.style_view(child).size().width,
            taffy::Dimension::length(20.0)
        );
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            tree.with_style_overrides(child, override_width(40.0), |_| {
                panic!("nested cache scope")
            })
        }));
        assert!(panic.is_err());
        assert_eq!(
            tree.style_view(child).size().width,
            taffy::Dimension::length(20.0)
        );
    });
    assert_eq!(tree.style_view(child).size(), original);
    assert!(tree.cache_get(id, &input).is_none());
}

#[test]
fn cold_warm_uncached_outputs_agree() {
    for display in ["flex", "grid"] {
        let (mut doc, _, child, _) = crate::layout::style_views::tests::fixture();
        doc.set_element_inline_style(
            child,
            Some(format!("display:{display};width:80px;height:30px").into()),
        );
        doc.append_element(Some(child), "div", Some("width:50%;height:10px"));
        let rules = raikiri_style::build_rule_tree(&doc);
        let cascade = raikiri_style::cascade(&doc, &rules).unwrap();
        let mut tree = LayoutSession::bridged(&mut doc, &cascade);
        crate::layout::bridge::apply_computed_to_style_in_session(&mut tree, &cascade).unwrap();
        let id = protocol::NodeId::from(child);
        let input = layout_input(80.0);
        let cold = tree.compute_child_layout_with_block_ctx(id, input, None);
        assert!(tree.cache_get(id, &input).is_some());
        let warm = tree.compute_child_layout_with_block_ctx(id, input, None);
        tree.layout_cache_context.invalidate();
        assert!(tree.cache_get(id, &input).is_none());
        let uncached = tree.compute_child_layout_with_block_ctx(id, input, None);
        assert_eq!(format!("{cold:?}"), format!("{warm:?}"));
        assert_eq!(format!("{cold:?}"), format!("{uncached:?}"));
    }
}

#[test]
fn legacy_session_size_scope_restores_values_after_unwind() {
    use taffy::CoreStyle;
    let (mut doc, mut cascade, child, _) = crate::layout::style_views::tests::fixture();
    cascade.computed[child].min_width = raikiri_style::ComputedLengthPercentageOrAuto::MaxContent;
    crate::layout::bridge::apply_computed_to_style(&mut doc, &cascade).unwrap();
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let old = tree.style_view(child).min_size();
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        crate::taffy_impl::content_limits::with_width_limits(
            &mut tree,
            child,
            Size {
                width: Some(100.0),
                height: None,
            },
            100.0,
            |_| panic!("legacy size scope"),
        );
    }));
    assert!(panic.is_err());
    assert_eq!(tree.style_view(child).min_size(), old);
    assert_eq!(tree.override_checkpoint(), 0);
    assert!(tree.layout_cache_context.active_frame.is_none());
}

#[test]
fn resolved_edge_scopes_publish_exactly_once_per_change() {
    for checked in [false, true] {
        let (mut doc, cascade, child, _) = crate::layout::style_views::tests::fixture();
        let mut tree = if checked {
            LayoutSession::with_cascade(&mut doc, &cascade)
        } else {
            LayoutSession::bridged(&mut doc, &cascade)
        };
        crate::layout::bridge::apply_computed_to_style_in_session(&mut tree, &cascade).unwrap();
        // Count scope changes separately from the pending document publication.
        tree.sync_layout_cache_mutations();
        let before = tree.layout_cache_context.epoch();
        tree.with_resolved_edges(child, 100.0, true, |tree| {
            assert_eq!(tree.layout_cache_context.epoch().value, before.value + 1);
            assert_eq!(tree.resolved_edge_basis(child), Some(100.0));
            tree.with_resolved_edges(child, 200.0, false, |tree| {
                assert_eq!(tree.resolved_edge_basis(child), Some(100.0));
            });
            assert_eq!(tree.resolved_edge_basis(child), Some(100.0));
        });
        assert_eq!(tree.layout_cache_context.epoch().value, before.value + 4);
        assert!(tree.resolved_edge_basis(child).is_none());
    }
}

#[test]
fn saved_cache_from_previous_namespace_cannot_be_restored() {
    let mut doc = Document::new();
    let session_cascade = crate::layout::test_support::cascaded(&doc);
    let mut tree = LayoutSession::bridged(&mut doc, &session_cascade);
    let id = protocol::NodeId::from(0usize);
    let input = layout_input(10.0);
    tree.with_cache_frame(0, ViewContext::default(), |tree| {
        tree.cache_store(id, &input, LayoutOutput::HIDDEN)
    });
    let saved = tree.take_layout_cache(0);
    tree.document.layout_cache_context.epoch.value = u64::MAX;
    tree.layout_cache_context.invalidate();
    assert_eq!(tree.layout_cache_context.epoch().value, 1);
    tree.restore_layout_cache(0, saved);
    assert!(tree.cache_get(id, &input).is_none());
}

#[test]
fn table_and_multicol_flex_descendants_agree_after_forced_miss() {
    for table in [false, true] {
        let (mut doc, _, child, _) = crate::layout::style_views::tests::fixture();
        let container_css = if table {
            "display:table;width:80px;border-collapse:collapse;border:2px solid"
        } else {
            "display:block;width:80px;columns:2;column-gap:4px"
        };
        doc.set_element_inline_style(child, Some(container_css.into()));
        let parent = if table {
            let row = doc.append_element(Some(child), "tr", Some("display:table-row"));
            doc.append_element(
                Some(row),
                "td",
                Some("display:table-cell;height:20px;border:1px solid"),
            )
        } else {
            child
        };
        let flex = doc.append_element(
            Some(parent),
            "div",
            Some("display:flex;width:100%;height:10px"),
        );
        doc.append_element(Some(flex), "div", Some("width:50%;height:10px"));
        let cascade = raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).unwrap();
        let mut tree = LayoutSession::bridged(&mut doc, &cascade);
        crate::layout::bridge::apply_computed_to_style_in_session(&mut tree, &cascade).unwrap();
        let id = protocol::NodeId::from(child);
        let mut input = layout_input(80.0);
        input.run_mode = RunMode::PerformLayout;
        let cold = tree.compute_child_layout_with_block_ctx(id, input, None);
        let warm = tree.compute_child_layout_with_block_ctx(id, input, None);
        tree.layout_cache_context.invalidate();
        let uncached = tree.compute_child_layout_with_block_ctx(id, input, None);
        assert_eq!(format!("{cold:?}"), format!("{warm:?}"));
        assert_eq!(format!("{cold:?}"), format!("{uncached:?}"));
        assert!(tree.layout_cache_context.active_frame.is_none());
    }
}

#[test]
fn dirty_saved_cache_is_discarded_without_publishing_inside_an_override() {
    use raikiri_layout_style::{LengthProperty, OwnedPropertyOverride};
    use raikiri_style::ComputedLengthPercentageOrAuto as L;
    let (mut doc, cascade, child, _) = crate::layout::style_views::tests::fixture();
    crate::layout::apply_computed_to_style(&mut doc, &cascade).unwrap();
    let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
    tree.prepare_style_views();
    let key = tree.nodes.box_key(child).unwrap();
    let input = layout_input(10.0);
    let id = protocol::NodeId::from(child);
    tree.with_overrides(
        vec![(
            key,
            OwnedPropertyOverride::length(LengthProperty::Width, L::Px(11.0), true).unwrap(),
        )],
        |tree| {
            tree.with_cache_frame(child, ViewContext::default(), |tree| {
                tree.cache_store(id, &input, LayoutOutput::HIDDEN)
            });
            let saved = tree.take_layout_cache(child);
            let publication = tree.layout_cache_context.publication_epoch();
            tree.layout_dirty = true;
            tree.restore_layout_cache(child, saved);
            assert!(tree.layout_dirty);
            assert_eq!(tree.layout_cache_context.publication_epoch(), publication);
            assert_eq!(tree.style_override_depth(child), 1);
        },
    );
    assert_eq!(tree.override_checkpoint(), 0);
    tree.with_cache_frame(child, ViewContext::default(), |tree| {
        assert!(tree.cache_get(id, &input).is_none());
        assert!(!tree.layout_dirty);
    });
}
