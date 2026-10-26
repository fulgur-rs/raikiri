use super::super::{with_flex_intrinsic_probe, with_flex_item_basis_bounds};
use crate::layout::protocol::{
    AvailableSpace, LayoutInput, LayoutOutput, NodeId, RequestedAxis, RunMode, SizingMode,
};
use crate::layout::{
    context::LayoutSession,
    style_views::tests::{fixture_with_styles, with_setup_inspector},
};
use raikiri_layout_geometry::{Edges, Line, Size};
use raikiri_layout_style::values::{Dimension, LengthPercentageAuto as Lpa};
use raikiri_layout_style::{LengthProperty, OwnedPropertyOverride};
use raikiri_style::ComputedLengthPercentageOrAuto as L;

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

pub(in crate::layout::dispatch) fn before_child_bounds(tree: &LayoutSession<'_>, idx: usize) {
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
fn intrinsic_scope_sets_the_bounded_item_cache_apart_and_keeps_the_generation() {
    use raikiri_layout_style::ViewContext;
    let (mut doc, cascade, child, _) =
        fixture_with_styles("display:flex;max-width:50px", "flex:1 0 100px;width:auto");
    let parent = doc.nodes[child].parent.unwrap();
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let id = NodeId::from(child);
    let input = LayoutInput {
        run_mode: RunMode::ComputeSize,
        sizing_mode: SizingMode::InherentSize,
        axis: RequestedAxis::Both,
        known_dimensions: Size::NONE,
        known_dimensions_are_definite: Size {
            width: false,
            height: false,
        },
        parent_size: Size::NONE,
        available_space: Size {
            width: AvailableSpace::MaxContent,
            height: AvailableSpace::MaxContent,
        },
        vertical_margins_are_collapsible: Line::FALSE,
    };
    let outside = LayoutOutput::HIDDEN;
    let inside = LayoutOutput::from_sizes(
        Size {
            width: 100.0,
            height: 10.0,
        },
        Edges::ZERO,
    );
    tree.with_cache_frame(child, ViewContext::default(), |tree| {
        tree.cache_store(id, &input, outside.clone());
        assert_eq!(tree.cache_get(id, &input), Some(outside.clone()));
        let ordinary_epoch = tree.layout_cache_context.epoch();
        with_flex_intrinsic_probe(tree, NodeId::from(parent), |tree| {
            with_flex_item_basis_bounds(tree, NodeId::from(parent), |tree| {
                // The item is measured under bounds of its own, so what it kept
                // for its ordinary style is not read, and what it keeps now is
                // not for the ordinary style.
                assert!(tree.cache_get(id, &input).is_none());
                tree.with_cache_frame(child, ViewContext::default(), |tree| {
                    tree.cache_store(id, &input, inside.clone());
                    assert_eq!(tree.cache_get(id, &input), Some(inside.clone()));
                });
            });
        });
        // The scopes left the generation alone: the answer of the ordinary
        // style is back and the frame that was open before can keep answers.
        assert_eq!(tree.layout_cache_context.epoch(), ordinary_epoch);
        assert_eq!(tree.cache_get(id, &input), Some(outside.clone()));
        tree.cache_store(id, &input, inside.clone());
        assert_eq!(tree.cache_get(id, &input), Some(inside.clone()));
    });
}

#[test]
fn intrinsic_scope_discards_the_saved_item_cache_when_the_generation_moves() {
    // An answer kept for the ordinary style is only restored while it is
    // still one the cache context vouches for; a scope inside that changes a
    // style the ordinary way moves the generation, and the item is measured
    // from nothing afterwards.
    use raikiri_layout_style::ViewContext;
    let (mut doc, cascade, child, _) =
        fixture_with_styles("display:flex;max-width:50px", "flex:1 0 100px;width:auto");
    let parent = doc.nodes[child].parent.unwrap();
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let id = NodeId::from(child);
    let input = LayoutInput {
        run_mode: RunMode::ComputeSize,
        sizing_mode: SizingMode::InherentSize,
        axis: RequestedAxis::Both,
        known_dimensions: Size::NONE,
        known_dimensions_are_definite: Size {
            width: false,
            height: false,
        },
        parent_size: Size::NONE,
        available_space: Size {
            width: AvailableSpace::MaxContent,
            height: AvailableSpace::MaxContent,
        },
        vertical_margins_are_collapsible: Line::FALSE,
    };
    tree.with_cache_frame(child, ViewContext::default(), |tree| {
        tree.cache_store(id, &input, LayoutOutput::HIDDEN);
        let ordinary_epoch = tree.layout_cache_context.epoch();
        with_flex_intrinsic_probe(tree, NodeId::from(parent), |tree| {
            with_flex_item_basis_bounds(tree, NodeId::from(parent), |tree| {
                tree.layout_cache_context.invalidate();
            });
        });
        assert!(tree.layout_cache_context.epoch() > ordinary_epoch);
        assert!(tree.cache_get(id, &input).is_none());
        // The frame opened before the generation moved cannot keep answers.
        tree.cache_store(id, &input, LayoutOutput::HIDDEN);
        assert!(tree.cache_get(id, &input).is_none());
    });
    tree.with_cache_frame(child, ViewContext::default(), |tree| {
        assert!(tree.cache_get(id, &input).is_none());
        tree.cache_store(id, &input, LayoutOutput::HIDDEN);
        assert!(tree.cache_get(id, &input).is_some());
    });
}

#[test]
fn a_probe_of_a_container_without_a_maximum_leaves_the_cache_generation_alone() {
    // With no `max-width` to drop, the probe changes nothing a box could be
    // measured differently by, so what is stored before it stays readable
    // inside it and after it.
    use raikiri_layout_style::ViewContext;
    let (mut doc, cascade, child, _) = fixture_with_styles("display:flex", "width:10px");
    let parent = doc.nodes[child].parent.unwrap();
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let id = NodeId::from(child);
    let input = LayoutInput {
        run_mode: RunMode::ComputeSize,
        sizing_mode: SizingMode::InherentSize,
        axis: RequestedAxis::Both,
        known_dimensions: Size::NONE,
        known_dimensions_are_definite: Size {
            width: false,
            height: false,
        },
        parent_size: Size::NONE,
        available_space: Size {
            width: AvailableSpace::MaxContent,
            height: AvailableSpace::MaxContent,
        },
        vertical_margins_are_collapsible: Line::FALSE,
    };
    tree.with_cache_frame(child, ViewContext::default(), |tree| {
        tree.cache_store(id, &input, LayoutOutput::HIDDEN);
        let epoch = tree.layout_cache_context.epoch();
        with_flex_intrinsic_probe(tree, NodeId::from(parent), |tree| {
            assert_eq!(tree.override_checkpoint(), 0);
            assert_eq!(tree.context.flex_intrinsic_probe, Some(parent));
            assert_eq!(tree.layout_cache_context.epoch(), epoch);
            assert!(tree.cache_get(id, &input).is_some());
        });
        assert_eq!(tree.layout_cache_context.epoch(), epoch);
        assert!(tree.cache_get(id, &input).is_some());
    });
}

#[test]
fn a_probe_of_a_container_with_a_maximum_drops_it_and_leaves_the_cache_generation_alone() {
    // The maximum is dropped for the probe only, and the probe is the one
    // measurement of the container that reads its style: what is kept for the
    // boxes inside it, and for the container itself, stays readable inside the
    // probe and after it.
    use raikiri_layout_style::ViewContext;
    let (mut doc, cascade, child, _) =
        fixture_with_styles("display:flex;max-width:50px", "width:10px");
    let parent = doc.nodes[child].parent.unwrap();
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let id = NodeId::from(child);
    let input = LayoutInput {
        run_mode: RunMode::ComputeSize,
        sizing_mode: SizingMode::InherentSize,
        axis: RequestedAxis::Both,
        known_dimensions: Size::NONE,
        known_dimensions_are_definite: Size {
            width: false,
            height: false,
        },
        parent_size: Size::NONE,
        available_space: Size {
            width: AvailableSpace::MaxContent,
            height: AvailableSpace::MaxContent,
        },
        vertical_margins_are_collapsible: Line::FALSE,
    };
    tree.with_cache_frame(child, ViewContext::default(), |tree| {
        tree.cache_store(id, &input, LayoutOutput::HIDDEN);
        let epoch = tree.layout_cache_context.epoch();
        with_flex_intrinsic_probe(tree, NodeId::from(parent), |tree| {
            assert_eq!(tree.override_checkpoint(), 1);
            assert!(tree.style_view(parent).max_size().width.is_auto());
            assert_eq!(tree.layout_cache_context.epoch(), epoch);
            assert!(tree.cache_get(id, &input).is_some());
        });
        assert_eq!(tree.override_checkpoint(), 0);
        assert_eq!(tree.style_view(parent).max_size().width, Lpa::length(50.0));
        assert_eq!(tree.layout_cache_context.epoch(), epoch);
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
