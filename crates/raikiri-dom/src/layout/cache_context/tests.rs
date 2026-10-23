use super::*;
use crate::layout::protocol::{
    self, AvailableSpace, LayoutInput, LayoutOutput, RequestedAxis, RunMode, SizingMode,
};
use crate::{Document, layout::context::LayoutSession, node::arena::BoxKey};
use raikiri_layout_geometry::{Line, Size};
use raikiri_layout_style::{PercentBasis, ViewContext};
use raikiri_style::property::WritingMode;

fn context(mode: WritingMode) -> CacheFingerprint {
    CacheFingerprint::new(
        ViewContext {
            layout_wm: mode,
            ..ViewContext::default()
        },
        0,
        0,
        0,
    )
}

fn store(cache: &mut LayoutCacheContext, key: BoxKey, fingerprint: CacheFingerprint) {
    let frame = cache.enter(key, fingerprint);
    assert!(cache.can_store(key));
    cache.mark_stored(key);
    cache.leave(frame);
}

#[test]
fn same_box_a_b_a_never_reuses_old_epoch() {
    let mut cache = LayoutCacheContext::default();
    let key = BoxKey::Source(0);
    let a = context(WritingMode::HorizontalTb);
    let b = context(WritingMode::VerticalRl);
    store(&mut cache, key, a);
    let first = cache.epoch();
    let frame = cache.enter(key, b);
    assert!(!cache.can_read(key));
    let second = cache.epoch();
    cache.leave(frame);
    let frame = cache.enter(key, a);
    assert!(!cache.can_read(key));
    assert!(first < second && second < cache.epoch());
    cache.leave(frame);
}

#[test]
fn different_boxes_do_not_change_epoch_for_each_other() {
    let mut cache = LayoutCacheContext::default();
    let first = cache.epoch();
    store(
        &mut cache,
        BoxKey::Source(0),
        context(WritingMode::HorizontalTb),
    );
    store(
        &mut cache,
        BoxKey::Source(1),
        context(WritingMode::VerticalRl),
    );
    assert_eq!(cache.epoch(), first);
    assert!(cache.can_read(BoxKey::Source(0)));
    assert!(cache.can_read(BoxKey::Source(1)));
}

#[test]
fn child_change_prevents_parent_hit() {
    let mut cache = LayoutCacheContext::default();
    let parent = BoxKey::Source(0);
    let child = BoxKey::Source(1);
    store(&mut cache, parent, context(WritingMode::HorizontalTb));
    store(&mut cache, child, context(WritingMode::HorizontalTb));
    let frame = cache.enter(child, context(WritingMode::VerticalRl));
    assert!(!cache.can_read(parent));
    cache.leave(frame);
}

#[test]
fn ancestor_late_store_is_discarded() {
    let mut cache = LayoutCacheContext::default();
    let parent = BoxKey::Source(0);
    let child = BoxKey::Source(1);
    store(&mut cache, child, context(WritingMode::HorizontalTb));
    let outer = cache.enter(parent, context(WritingMode::HorizontalTb));
    let inner = cache.enter(child, context(WritingMode::VerticalRl));
    assert!(cache.can_store(child));
    cache.mark_stored(child);
    cache.leave(inner);
    assert!(!cache.can_store(parent));
    cache.leave(outer);
    assert!(!cache.can_read(parent));
}

#[test]
fn reentrant_same_node_keeps_distinct_computation_frames() {
    let mut cache = LayoutCacheContext::default();
    let node = BoxKey::Source(0);
    let outer = cache.enter(node, context(WritingMode::HorizontalTb));
    let inner = cache.enter(node, context(WritingMode::VerticalRl));
    assert!(cache.can_store(node));
    cache.mark_stored(node);
    cache.leave(inner);
    assert!(!cache.can_store(node));
    cache.leave(outer);
    assert!(cache.can_read(node));
}

#[test]
fn new_session_does_not_relabel_old_cache() {
    let mut doc = Document::new();
    let node = BoxKey::Source(0);
    store(
        &mut doc.layout_cache_context,
        node,
        context(WritingMode::HorizontalTb),
    );
    doc.layout_cache_context.invalidate();
    let epoch = doc.layout_cache_context.epoch();
    let session_cascade = crate::layout::test_support::cascaded(&doc);
    let session = LayoutSession::with_cascade(&mut doc, &session_cascade);
    assert!(session.layout_cache_context.epoch() > epoch);
    assert!(!session.layout_cache_context.can_read(node));
}

#[test]
fn source_and_metadata_publication_invalidate_before_any_child_read() {
    let mut cache = LayoutCacheContext::default();
    let parent = BoxKey::Source(0);
    store(&mut cache, parent, context(WritingMode::HorizontalTb));
    cache.publish_source();
    assert!(!cache.can_read(parent));
    let fingerprint = cache.fingerprint(parent, ViewContext::default());
    store(&mut cache, parent, fingerprint);
    cache.publish_metadata();
    assert!(!cache.can_read(parent));
}

#[test]
fn nan_payloads_and_signed_zero_do_not_churn_epochs() {
    let mut cache = LayoutCacheContext::default();
    let node = BoxKey::Source(0);
    let fingerprint = |basis| {
        CacheFingerprint::new(
            ViewContext {
                percent_basis: PercentBasis::Definite(basis),
                ..ViewContext::default()
            },
            0,
            0,
            0,
        )
    };
    store(&mut cache, node, fingerprint(f32::from_bits(0x7fc0_0001)));
    let epoch = cache.epoch();
    let frame = cache.enter(node, fingerprint(f32::from_bits(0x7fc0_0002)));
    assert_eq!(cache.epoch(), epoch);
    assert!(cache.can_read(node));
    cache.leave(frame);
    store(&mut cache, node, fingerprint(-0.0));
    let epoch = cache.epoch();
    let frame = cache.enter(node, fingerprint(0.0));
    assert_eq!(cache.epoch(), epoch);
    cache.leave(frame);
}

#[test]
fn epoch_overflow_does_not_revive_stamps_or_active_frames() {
    let mut cache = LayoutCacheContext::default();
    let node = BoxKey::Source(0);
    store(&mut cache, node, context(WritingMode::HorizontalTb));
    let first = cache.epoch();
    let frame = cache.enter(node, context(WritingMode::HorizontalTb));
    cache.epoch.value = u64::MAX;
    cache.invalidate();
    assert!(cache.epoch() > first);
    assert!(!cache.can_read(node));
    assert!(!cache.can_store(node));
    cache.leave(frame);
}

#[test]
fn source_publication_does_not_invalidate_once_per_box() {
    let mut cache = LayoutCacheContext::default();
    for id in 0..20 {
        store(
            &mut cache,
            BoxKey::Source(id),
            context(WritingMode::HorizontalTb),
        );
    }
    cache.publish_source();
    let epoch = cache.epoch();
    for id in 0..20 {
        let node = BoxKey::Source(id);
        let fingerprint = cache.fingerprint(node, ViewContext::default());
        store(&mut cache, node, fingerprint);
        assert_eq!(cache.epoch(), epoch);
    }
}

#[test]
fn namespace_exhaustion_keeps_all_cache_access_disabled() {
    let mut cache = LayoutCacheContext::default();
    let node = BoxKey::Source(0);
    cache.epoch.namespace = u64::MAX;
    cache.epoch.value = u64::MAX;
    store(&mut cache, node, context(WritingMode::HorizontalTb));
    cache.invalidate();
    let frame = cache.enter(node, context(WritingMode::HorizontalTb));
    assert!(!cache.can_read(node));
    assert!(!cache.can_store(node));
    cache.leave(frame);
    cache.invalidate();
    assert!(!cache.can_read(node));
}

#[test]
fn clone_does_not_copy_active_frames_or_relabel_node_caches() {
    let mut cache = LayoutCacheContext::default();
    let node = BoxKey::Source(0);
    store(&mut cache, node, context(WritingMode::HorizontalTb));
    let frame = cache.enter(node, context(WritingMode::HorizontalTb));
    let cloned = cache.clone();
    assert!(cloned.active_frame.is_none());
    assert!(!cloned.can_read(node));
    assert!(!cloned.can_store(node));
    cache.leave(frame);
}

#[test]
fn override_push_and_pop_each_invalidate_the_parent_before_lookup() {
    let mut cache = LayoutCacheContext::default();
    let parent = BoxKey::Source(0);
    let child = BoxKey::Source(1);
    store(&mut cache, parent, context(WritingMode::HorizontalTb));
    cache.override_changed(child);
    assert!(!cache.can_read(parent));
    let first = cache.epoch();
    let fingerprint = cache.fingerprint(parent, ViewContext::default());
    store(&mut cache, parent, fingerprint);
    cache.override_changed(child);
    assert!(!cache.can_read(parent));
    assert!(cache.epoch() > first);
}

#[test]
fn basis_tags_and_alignment_axes_remain_distinct() {
    let mut cache = LayoutCacheContext::default();
    let node = BoxKey::Source(0);
    for view in [
        ViewContext::default(),
        ViewContext {
            percent_basis: PercentBasis::Indefinite,
            ..ViewContext::default()
        },
        ViewContext {
            percent_basis: PercentBasis::Definite(0.0),
            ..ViewContext::default()
        },
        ViewContext {
            percent_basis: PercentBasis::Definite(0.0),
            align_axis_is_inline: true,
            ..ViewContext::default()
        },
    ] {
        let fingerprint = cache.fingerprint(node, view);
        let frame = cache.enter(node, fingerprint);
        assert!(!cache.can_read(node));
        cache.mark_stored(node);
        cache.leave(frame);
    }
}

fn layout_input(width: f32) -> LayoutInput {
    LayoutInput {
        run_mode: RunMode::ComputeSize,
        sizing_mode: SizingMode::InherentSize,
        axis: RequestedAxis::Both,
        known_dimensions: Size {
            width: Some(width),
            height: None,
        },
        known_dimensions_are_definite: Size {
            width: true,
            height: false,
        },
        parent_size: Size {
            width: None,
            height: None,
        },
        available_space: Size {
            width: AvailableSpace::MaxContent,
            height: AvailableSpace::MaxContent,
        },
        vertical_margins_are_collapsible: Line::TRUE,
    }
}

#[test]
fn first_store_in_new_epoch_clears_every_old_input() {
    use taffy::CacheTree;
    let mut doc = Document::new();
    let session_cascade = crate::layout::test_support::cascaded(&doc);
    let mut tree = LayoutSession::bridged(&mut doc, &session_cascade);
    let id = protocol::NodeId::from(0usize);
    let a = layout_input(10.0);
    let b = layout_input(20.0);
    tree.with_cache_frame(0, ViewContext::default(), |tree| {
        tree.cache_store(id, &a, LayoutOutput::HIDDEN);
    });
    tree.layout_cache_context.invalidate();
    tree.with_cache_frame(0, ViewContext::default(), |tree| {
        tree.cache_store(id, &b, LayoutOutput::HIDDEN);
        assert!(tree.cache_get(id, &a).is_none());
        assert!(tree.cache_get(id, &b).is_some());
    });
}

#[test]
fn stale_outer_store_keeps_valid_same_node_inner_cache() {
    use taffy::CacheTree;
    let mut doc = Document::new();
    let session_cascade = crate::layout::test_support::cascaded(&doc);
    let mut tree = LayoutSession::bridged(&mut doc, &session_cascade);
    let id = protocol::NodeId::from(0usize);
    let a = layout_input(10.0);
    let b = layout_input(20.0);
    tree.with_cache_frame(0, ViewContext::default(), |tree| {
        tree.layout_cache_context.invalidate();
        tree.with_cache_frame(0, ViewContext::default(), |tree| {
            tree.cache_store(id, &b, LayoutOutput::HIDDEN);
        });
        tree.cache_store(id, &a, LayoutOutput::HIDDEN);
        assert!(tree.cache_get(id, &a).is_none());
        assert!(tree.cache_get(id, &b).is_some());
    });
}

#[test]
fn cache_frame_leaves_after_hit_error_and_unwind() {
    let mut doc = Document::new();
    let session_cascade = crate::layout::test_support::cascaded(&doc);
    let mut tree = LayoutSession::bridged(&mut doc, &session_cascade);
    let result: Result<(), ()> = tree.with_cache_frame(0, ViewContext::default(), |_| Err(()));
    assert!(result.is_err());
    assert!(tree.layout_cache_context.active_frame.is_none());
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        tree.with_cache_frame(0, ViewContext::default(), |_| panic!("frame test"));
    }));
    assert!(result.is_err());
    assert!(tree.layout_cache_context.active_frame.is_none());
}

#[test]
fn pending_document_mutation_rejects_parent_cache_before_child_read() {
    use taffy::CacheTree;
    let mut doc = Document::new();
    let session_cascade = crate::layout::test_support::cascaded(&doc);
    let mut tree = LayoutSession::bridged(&mut doc, &session_cascade);
    let input = layout_input(10.0);
    let id = protocol::NodeId::from(0usize);
    tree.with_cache_frame(0, ViewContext::default(), |tree| {
        tree.cache_store(id, &input, LayoutOutput::HIDDEN);
    });
    tree.layout_dirty = true;
    assert!(tree.cache_get(id, &input).is_none());
}

mod restore_tests;

#[test]
fn nested_frame_tokens_preserve_unbounded_reentrant_order() {
    let mut cache = LayoutCacheContext::default();
    let mut tokens = Vec::new();
    for idx in 0..20 {
        let key = BoxKey::Source(idx);
        tokens.push(cache.enter(key, context(WritingMode::HorizontalTb)));
    }
    assert_eq!(cache.active_frame.unwrap().depth, 19);
    for token in tokens.into_iter().rev() {
        assert!(cache.can_store(token.key));
        cache.leave(token);
    }
    assert!(cache.active_frame.is_none());
}

#[test]
fn later_session_does_not_reuse_unfinished_child_walk() {
    let mut doc = Document::new();
    {
        let session_cascade = crate::layout::test_support::cascaded(&doc);
        let tree = LayoutSession::bridged(&mut doc, &session_cascade);
        tree.node_state(0).unwrap().taffy_child_walks = Some(vec![protocol::NodeId::from(0usize)]);
    }
    let session_cascade = crate::layout::test_support::cascaded(&doc);
    let tree = LayoutSession::bridged(&mut doc, &session_cascade);
    assert!(tree.node_state(0).unwrap().taffy_child_walks.is_none());
}

#[test]
fn context_change_at_rollover_keeps_the_new_last_context() {
    let mut cache = LayoutCacheContext::default();
    let key = BoxKey::Source(0);
    cache.epoch.value = u64::MAX;
    store(&mut cache, key, context(WritingMode::HorizontalTb));
    store(&mut cache, key, context(WritingMode::VerticalRl));
    let frame = cache.enter(key, context(WritingMode::HorizontalTb));
    assert!(!cache.can_read(key));
    cache.leave(frame);
}

#[test]
fn walk_session_rollover_does_not_revive_a_previous_walk() {
    let mut cache = LayoutCacheContext::default();
    let key = BoxKey::Source(0);
    store(&mut cache, key, context(WritingMode::HorizontalTb));
    cache.begin_session();
    cache.node_state(key, false).taffy_child_walks = Some(vec![protocol::NodeId::from(0usize)]);
    cache.walk_session = u64::MAX;
    cache.begin_session();
    assert!(cache.node_state(key, false).taffy_child_walks.is_none());
    assert!(cache.can_read(key));
}

fn store_view(cache: &mut LayoutCacheContext, key: BoxKey, view: ViewContext) {
    let frame = cache.enter_view(key, view);
    assert!(cache.can_store(key));
    cache.mark_stored(key);
    cache.leave(frame);
}

#[test]
fn growing_workset_preserves_existing_cache_stamps_and_contexts() {
    let mut cache = LayoutCacheContext::default();
    let epoch = cache.epoch();
    for idx in 0..9 {
        let view = ViewContext {
            layout_wm: if idx % 2 == 0 {
                WritingMode::HorizontalTb
            } else {
                WritingMode::VerticalRl
            },
            ..ViewContext::default()
        };
        store_view(&mut cache, BoxKey::Source(idx), view);
        for previous in 0..=idx {
            assert!(cache.can_read(BoxKey::Source(previous)));
        }
        assert_eq!(cache.epoch(), epoch);
    }
    let frame = cache.enter_view(
        BoxKey::Source(0),
        ViewContext {
            layout_wm: WritingMode::VerticalRl,
            ..ViewContext::default()
        },
    );
    for idx in 0..9 {
        assert!(!cache.can_read(BoxKey::Source(idx)));
    }
    cache.leave(frame);
}

#[test]
fn publications_preserve_key_identity_across_large_and_small_worksets() {
    let mut cache = LayoutCacheContext::default();
    let view = ViewContext::default();
    for idx in 0..6 {
        store_view(&mut cache, BoxKey::Source(idx), view);
        store_view(&mut cache, BoxKey::AnonymousCell(idx), view);
    }
    cache.publish_metadata();
    for idx in 0..6 {
        assert!(!cache.can_read(BoxKey::Source(idx)));
        assert!(!cache.can_read(BoxKey::AnonymousCell(idx)));
    }
    store_view(&mut cache, BoxKey::Source(0), view);
    store_view(&mut cache, BoxKey::AnonymousCell(0), view);
    assert!(cache.can_read(BoxKey::Source(0)));
    assert!(cache.can_read(BoxKey::AnonymousCell(0)));

    cache.publish_source();
    assert!(!cache.can_read(BoxKey::Source(0)));
    assert!(!cache.can_read(BoxKey::AnonymousCell(0)));
    for idx in 0..6 {
        store_view(&mut cache, BoxKey::AnonymousCell(idx), view);
    }
    cache.publish_metadata();
    for idx in 0..6 {
        assert!(!cache.can_read(BoxKey::AnonymousCell(idx)));
    }
    let epoch = cache.epoch();
    for idx in 0..6 {
        store_view(&mut cache, BoxKey::Source(idx), view);
    }
    for idx in 0..6 {
        assert!(cache.can_read(BoxKey::Source(idx)));
        assert!(!cache.can_read(BoxKey::AnonymousCell(idx)));
    }
    assert_eq!(cache.epoch(), epoch);
}

#[test]
fn publication_sync_preserves_new_walk_in_either_query_order() {
    let key = BoxKey::Source(0);
    let child = protocol::NodeId::from(2usize);
    for walk_first in [true, false] {
        let mut cache = LayoutCacheContext::default();
        cache.begin_session();
        store_view(&mut cache, key, ViewContext::default());
        cache.node_state(key, false).taffy_child_walks = Some(vec![protocol::NodeId::from(1usize)]);
        cache.publish_metadata();
        let epoch = cache.epoch();
        if walk_first {
            assert!(cache.node_state(key, false).taffy_child_walks.is_none());
            cache.node_state(key, false).taffy_child_walks = Some(vec![child]);
        }
        let frame = cache.enter_view(key, ViewContext::default());
        if !walk_first {
            assert!(cache.node_state(key, false).taffy_child_walks.is_none());
            cache.node_state(key, false).taffy_child_walks = Some(vec![child]);
        }
        assert_eq!(
            cache.node_state(key, false).taffy_child_walks,
            Some(vec![child])
        );
        cache.mark_stored(key);
        cache.leave(frame);
        assert_eq!(cache.epoch(), epoch);
        assert!(cache.can_read(key));
    }
}

#[test]
fn transient_override_keeps_walk_and_session_change_keeps_cache() {
    let mut cache = LayoutCacheContext::default();
    let key = BoxKey::Source(0);
    let child = protocol::NodeId::from(1usize);
    cache.begin_session();
    cache.publish_metadata();
    cache.override_changed(key);
    let fingerprint = cache.fingerprint(key, ViewContext::default());
    cache.node_state(key, false).taffy_child_walks = Some(vec![child]);
    assert_eq!(cache.fingerprint(key, ViewContext::default()), fingerprint);
    cache.override_changed(key);
    assert_eq!(
        cache.node_state(key, false).taffy_child_walks,
        Some(vec![child])
    );
    store_view(&mut cache, key, ViewContext::default());
    cache.begin_session();
    assert!(cache.node_state(key, false).taffy_child_walks.is_none());
    assert!(cache.can_read(key));
}

#[test]
fn metadata_publication_resets_context_even_after_epoch_exhaustion() {
    let mut cache = LayoutCacheContext::default();
    let key = BoxKey::Source(0);
    let view = ViewContext::default();
    cache.epoch.namespace = u64::MAX;
    cache.epoch.value = u64::MAX;
    cache.publish_metadata();
    cache.override_changed(key);
    let frame = cache.enter_view(key, view);
    cache.leave(frame);
    cache.node_state(key, false).taffy_child_walks = Some(vec![protocol::NodeId::from(1usize)]);
    cache.publish_metadata();
    assert_eq!(
        cache.fingerprint(key, view),
        CacheFingerprint::new(view, cache.source, cache.metadata, 0)
    );
    assert!(cache.node_state(key, false).taffy_child_walks.is_none());
    assert!(!cache.can_read(key));
    let frame = cache.enter_view(key, view);
    assert!(!cache.can_store(key));
    cache.leave(frame);
}

#[test]
fn metadata_publication_discards_inline_generated_identity() {
    let mut cache = LayoutCacheContext::default();
    let generated = BoxKey::AnonymousCell(0);
    store_view(&mut cache, generated, ViewContext::default());
    cache.publish_metadata();
    assert!(cache.entries.borrow().get(&generated).is_none());
    assert!(!cache.can_read(generated));
}

#[test]
#[should_panic(expected = "cache frames leave in reverse order")]
fn frames_reject_leaving_an_outer_computation_first() {
    let mut cache = LayoutCacheContext::default();
    let outer = cache.enter_view(BoxKey::Source(0), ViewContext::default());
    let _inner = cache.enter_view(BoxKey::Source(1), ViewContext::default());
    cache.leave(outer);
}

#[test]
#[should_panic(expected = "only a current active computation may stamp its cache")]
fn invalidated_computation_cannot_stamp_its_old_result() {
    let mut cache = LayoutCacheContext::default();
    let key = BoxKey::Source(0);
    let _frame = cache.enter_view(key, ViewContext::default());
    cache.invalidate();
    cache.mark_stored(key);
}
