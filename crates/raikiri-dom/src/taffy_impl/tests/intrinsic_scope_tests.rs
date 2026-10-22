use super::super::{with_flex_intrinsic_probe, with_flex_item_basis_bounds};
use crate::layout::{
    context::LayoutSession,
    style_views::tests::{fixture_with_styles, with_setup_inspector},
};
use raikiri_layout_style::{LengthProperty, OwnedPropertyOverride};
use raikiri_style::ComputedLengthPercentageOrAuto as L;
use taffy::{CoreStyle, Dimension, FlexboxItemStyle, LengthPercentageAuto as Lpa, NodeId};

#[test]
fn intrinsic_scope_preserves_unrelated_updates() {
    let (mut doc, cascade, child, _) =
        fixture_with_styles("display:flex;max-width:50px", "flex:1 0 100px;width:auto");
    let parent = doc.nodes[child].parent.unwrap();
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    with_flex_intrinsic_probe(&mut tree, NodeId::from(parent), |tree| {
        assert!(tree.style_view(parent).max_size().width.is_auto());
        with_flex_item_basis_bounds(tree, NodeId::from(parent), |tree| {
            let view = tree.style_view(child);
            assert!(view.size().width.is_auto());
            assert_eq!(view.min_size().width, Lpa::length(100.0));
            assert_eq!(view.flex_shrink(), 1.0);
            tree.record_pass_override(
                child,
                OwnedPropertyOverride::length(LengthProperty::Width, L::Px(91.0), true).unwrap(),
            );
        });
    });
    assert_eq!(tree.style_view(child).size().width, Dimension::length(91.0));
    assert_eq!(tree.style_view(child).flex_shrink(), 0.0);
    assert_eq!(tree.style_view(parent).max_size().width, Lpa::length(50.0));
    assert_eq!(tree.override_checkpoint(), 0);
}

#[test]
fn intrinsic_scope_restores_marker_and_original_panic() {
    fn fail(tree: &LayoutSession<'_>, child: usize, count: usize) {
        if count == 2 {
            assert_eq!(tree.style_view(child).min_size().width, Lpa::length(100.0));
            panic!("intrinsic setup failure");
        }
    }
    for setup in [false, true] {
        let (mut doc, cascade, child, _) =
            fixture_with_styles("display:flex;max-width:50px", "flex:1 0 100px;width:auto");
        let parent = doc.nodes[child].parent.unwrap();
        let mut tree = LayoutSession::bridged(&mut doc, &cascade);
        tree.context.flex_intrinsic_probe = Some(child);
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut run = || {
                with_flex_intrinsic_probe(&mut tree, NodeId::from(parent), |tree| {
                    assert_eq!(tree.context.flex_intrinsic_probe, Some(parent));
                    with_flex_item_basis_bounds(tree, NodeId::from(parent), |_| {
                        panic!("intrinsic body failure")
                    });
                })
            };
            if setup {
                with_setup_inspector(fail, run);
            } else {
                run();
            }
        }))
        .unwrap_err();
        assert_eq!(
            panic.downcast_ref::<&str>(),
            Some(&if setup {
                "intrinsic setup failure"
            } else {
                "intrinsic body failure"
            })
        );
        assert_eq!(tree.context.flex_intrinsic_probe, Some(child));
        assert_eq!(tree.override_checkpoint(), 0);
        assert_eq!(tree.style_view(child).flex_shrink(), 0.0);
        assert!(tree.style_view(child).min_size().width.is_auto());
        assert_eq!(tree.style_view(parent).max_size().width, Lpa::length(50.0));
    }
}

thread_local! {
    static CHILD_PAIR: std::cell::Cell<Option<(usize, usize)>> = const { std::cell::Cell::new(None) };
    static FAIL_LATER_CHILD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static SAW_PRIOR_BOUND: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub(in crate::taffy_impl) fn before_child_bounds(tree: &LayoutSession<'_>, idx: usize) {
    if let Some((first, second)) = CHILD_PAIR.get()
        && idx == second
    {
        assert_eq!(tree.style_view(first).min_size().width, Lpa::length(100.0));
        assert_eq!(tree.style_view(first).flex_shrink(), 1.0);
        SAW_PRIOR_BOUND.set(true);
        if FAIL_LATER_CHILD.get() {
            panic!("later child setup failure");
        }
    }
}

#[test]
fn intrinsic_scope_keeps_earlier_child_bounds() {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            CHILD_PAIR.set(None);
        }
    }
    let (mut doc, _, first, _) = fixture_with_styles("display:flex", "flex:1 0 100px;width:auto");
    let parent = doc.nodes[first].parent.unwrap();
    let second = doc.append_element(Some(parent), "div", Some("flex:1 0 200px;width:auto"));
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).unwrap();
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    CHILD_PAIR.set(Some((first, second)));
    SAW_PRIOR_BOUND.set(false);
    let _reset = Reset;
    with_flex_item_basis_bounds(&mut tree, NodeId::from(parent), |tree| {
        assert_eq!(tree.style_view(first).min_size().width, Lpa::length(100.0));
        assert_eq!(tree.style_view(second).min_size().width, Lpa::length(200.0));
    });
    assert!(SAW_PRIOR_BOUND.get());
    for idx in [first, second] {
        assert!(tree.style_view(idx).min_size().width.is_auto());
        assert_eq!(tree.style_view(idx).flex_shrink(), 0.0);
    }
    assert_eq!(tree.override_checkpoint(), 0);
}

#[test]
fn intrinsic_scope_isolates_probe_cache_and_rejects_late_outer_store() {
    use raikiri_layout_style::ViewContext;
    use taffy::CacheTree;
    let (mut doc, cascade, child, _) =
        fixture_with_styles("display:flex;max-width:50px", "flex:1 0 100px;width:auto");
    let parent = doc.nodes[child].parent.unwrap();
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let id = NodeId::from(child);
    let input = taffy::LayoutInput {
        run_mode: taffy::RunMode::ComputeSize,
        sizing_mode: taffy::SizingMode::InherentSize,
        axis: taffy::RequestedAxis::Both,
        known_dimensions: taffy::Size::NONE,
        known_dimensions_are_definite: taffy::Size {
            width: false,
            height: false,
        },
        parent_size: taffy::Size::NONE,
        available_space: taffy::Size {
            width: taffy::AvailableSpace::MaxContent,
            height: taffy::AvailableSpace::MaxContent,
        },
        vertical_margins_are_collapsible: taffy::Line::FALSE,
    };
    tree.with_cache_frame(child, ViewContext::default(), |tree| {
        tree.cache_store(id, &input, taffy::LayoutOutput::HIDDEN);
        assert!(tree.cache_get(id, &input).is_some());
        let ordinary_epoch = tree.layout_cache_context.epoch();
        with_flex_intrinsic_probe(tree, NodeId::from(parent), |tree| {
            with_flex_item_basis_bounds(tree, NodeId::from(parent), |tree| {
                assert!(tree.cache_get(id, &input).is_none());
                tree.with_cache_frame(child, ViewContext::default(), |tree| {
                    tree.cache_store(id, &input, taffy::LayoutOutput::HIDDEN);
                    assert!(tree.cache_get(id, &input).is_some());
                });
            });
        });
        assert!(tree.layout_cache_context.epoch() > ordinary_epoch);
        assert!(tree.cache_get(id, &input).is_none());
        tree.cache_store(id, &input, taffy::LayoutOutput::HIDDEN);
        assert!(tree.cache_get(id, &input).is_none());
    });
    tree.with_cache_frame(child, ViewContext::default(), |tree| {
        assert!(tree.cache_get(id, &input).is_none());
        tree.cache_store(id, &input, taffy::LayoutOutput::HIDDEN);
        assert!(tree.cache_get(id, &input).is_some());
    });
}

#[test]
fn intrinsic_scope_dirty_descendant_preserves_original_panic() {
    let (mut doc, cascade, child, _) =
        fixture_with_styles("display:flex;max-width:50px", "flex:1 0 100px;width:auto");
    let parent = doc.nodes[child].parent.unwrap();
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        with_flex_intrinsic_probe(&mut tree, NodeId::from(parent), |tree| {
            with_flex_item_basis_bounds(tree, NodeId::from(parent), |tree| {
                tree.layout_dirty = true;
                panic!("dirty intrinsic descendant");
            });
        });
    }))
    .unwrap_err();
    assert_eq!(
        panic.downcast_ref::<&str>(),
        Some(&"dirty intrinsic descendant")
    );
    assert!(tree.layout_dirty);
    assert_eq!(tree.override_checkpoint(), 0);
    assert!(tree.context.flex_intrinsic_probe.is_none());
    assert_eq!(tree.style_view(child).flex_shrink(), 0.0);
    assert_eq!(tree.style_view(parent).max_size().width, Lpa::length(50.0));
}

#[test]
fn intrinsic_scope_later_child_setup_failure_restores_prefix() {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            CHILD_PAIR.set(None);
            FAIL_LATER_CHILD.set(false);
        }
    }
    let (mut doc, _, first, _) =
        fixture_with_styles("display:flex;max-width:50px", "flex:1 0 100px;width:auto");
    let parent = doc.nodes[first].parent.unwrap();
    let second = doc.append_element(Some(parent), "div", Some("flex:1 0 200px;width:auto"));
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).unwrap();
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    CHILD_PAIR.set(Some((first, second)));
    FAIL_LATER_CHILD.set(true);
    let _reset = Reset;
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        with_flex_intrinsic_probe(&mut tree, NodeId::from(parent), |tree| {
            with_flex_item_basis_bounds(tree, NodeId::from(parent), |_| panic!("unreachable body"));
        });
    }))
    .unwrap_err();
    assert_eq!(
        panic.downcast_ref::<&str>(),
        Some(&"later child setup failure")
    );
    for idx in [first, second] {
        assert!(tree.style_view(idx).min_size().width.is_auto());
        assert_eq!(tree.style_view(idx).flex_shrink(), 0.0);
    }
    assert_eq!(tree.style_view(parent).max_size().width, Lpa::length(50.0));
    assert_eq!(tree.override_checkpoint(), 0);
    assert!(tree.context.flex_intrinsic_probe.is_none());
}

#[test]
fn intrinsic_scope_container_setup_failure_restores_marker() {
    fn fail(tree: &LayoutSession<'_>, idx: usize, count: usize) {
        assert_eq!(count, 1);
        assert_eq!(tree.context.flex_intrinsic_probe, Some(idx));
        assert!(tree.style_view(idx).max_size().width.is_auto());
        panic!("container setup failure");
    }
    let (mut doc, cascade, child, _) =
        fixture_with_styles("display:flex;max-width:50px", "flex:1 0 100px;width:auto");
    let parent = doc.nodes[child].parent.unwrap();
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    tree.context.flex_intrinsic_probe = Some(child);
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        with_setup_inspector(fail, || {
            with_flex_intrinsic_probe(&mut tree, NodeId::from(parent), |_| {
                panic!("unreachable body")
            });
        })
    }))
    .unwrap_err();
    assert_eq!(
        panic.downcast_ref::<&str>(),
        Some(&"container setup failure")
    );
    assert_eq!(tree.context.flex_intrinsic_probe, Some(child));
    assert_eq!(tree.override_checkpoint(), 0);
    assert_eq!(tree.style_view(parent).max_size().width, Lpa::length(50.0));
}
