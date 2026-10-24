use super::*;
use crate::layout::style_views::tests::fixture;
use raikiri_layout_style::{LengthProperty, OwnedPropertyOverride};
use raikiri_style::ComputedLengthPercentageOrAuto as L;

use raikiri_layout_style::values::StyleSize;

#[test]
fn batched_content_limits_preserve_unrelated_pass_updates_after_unwind() {
    for unwind in [false, true] {
        let (mut doc, mut cascade, child, _) = fixture();
        cascade.computed[child].min_width = L::MaxContent;
        let parent = doc.nodes[child].parent.unwrap();
        let mut tree = LayoutSession::bridged(&mut doc, &cascade);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            super::super::with_item_content_limits(
                &mut tree,
                NodeId::from(parent),
                super::style_view_tests::content_limit_inputs(),
                Size {
                    width: Some(100.0),
                    height: None,
                },
                |tree, _, _| {
                    assert!(!tree.style_view(child).min_size().width.is_auto());
                    tree.cache_override_changed(child);
                    tree.record_pass_override(
                        child,
                        OwnedPropertyOverride::length(LengthProperty::Height, L::Px(91.0), true)
                            .unwrap(),
                    );
                    if unwind {
                        panic!("after batch pass update");
                    }
                    LayoutOutput::HIDDEN
                },
            );
        }));
        assert_eq!(result.is_err(), unwind);
        assert_eq!(tree.style_override_depth(child), 0);
        assert!(tree.context.content_limit_probes.is_empty());
        assert_eq!(tree.context.content_limit_depth, 0);
        assert_eq!(
            tree.style_view(child).size().height,
            taffy::Dimension::length(91.0)
        );
    }
}

#[test]
fn batched_oof_scopes_restore_four_raw_constraints_and_keep_unrelated_updates() {
    for unwind in [false, true] {
        let (mut doc, _, first, _) = fixture();
        let parent = doc.nodes[first].parent.unwrap();
        let second = doc.append_element(Some(parent), "div", Some("width:41px;height:9px"));
        doc.mark_in_document_flags();
        let mut cascade =
            raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).unwrap();
        for child in [first, second] {
            cascade.computed[child].position = raikiri_style::property::PositionValue::Absolute;
            cascade.computed[child].min_width = L::Px(50.0);
            cascade.computed[child].max_width = L::Px(60.0);
            cascade.computed[child].min_height = L::Px(4.0);
            cascade.computed[child].max_height = L::MaxContent;
        }
        crate::layout::test_support::with_ahem(&mut doc);
        let mut tree = LayoutSession::bridged(&mut doc, &cascade);
        let mut calls = 0;
        let mut saw_raw = false;
        let mut saw_final = false;
        let inputs = LayoutInput {
            parent_size: Size {
                width: Some(100.0),
                height: Some(100.0),
            },
            ..LayoutInput::HIDDEN
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            with_oof_content_limits(
                &mut tree,
                Some(&[NodeId::from(first), NodeId::from(second)]),
                &[],
                |tree| {
                    calls += 1;
                    if let Some(capture) = tree.context.oof_content_limit_inputs.last_mut() {
                        for child in [first, second] {
                            capture.insert(child, inputs);
                        }
                    }
                    if calls == 2 {
                        for child in [first, second] {
                            assert_eq!(tree.style_view(child).min_size(), StyleSize::auto());
                            assert_eq!(tree.style_view(child).max_size(), StyleSize::auto());
                        }
                        saw_raw = true;
                    }
                    if calls == 3 {
                        assert!(tree.context.content_limit_probes.contains(&first));
                        assert!(tree.context.content_limit_probes.contains(&second));
                        tree.cache_override_changed(second);
                        tree.record_pass_override(
                            second,
                            OwnedPropertyOverride::length(
                                LengthProperty::Height,
                                L::Px(91.0),
                                true,
                            )
                            .unwrap(),
                        );
                        saw_final = true;
                        if unwind {
                            panic!("multiple OOF candidates");
                        }
                    }
                },
            );
        }));
        assert!(saw_raw && saw_final);
        assert_eq!(result.is_err(), unwind);
        assert_eq!(calls, 3);
        for child in [first, second] {
            let view = tree.style_view(child);
            assert_eq!(view.min_size().width, LengthPercentageAuto::length(50.0));
            assert_eq!(view.max_size().width, LengthPercentageAuto::length(60.0));
            assert_eq!(view.min_size().height, LengthPercentageAuto::length(4.0));
            assert!(view.max_size().height.is_auto());
            assert_eq!(tree.style_override_depth(child), 0);
        }
        assert_eq!(
            tree.style_view(second).size().height,
            taffy::Dimension::length(91.0)
        );
        assert!(tree.context.content_limit_probes.is_empty());
        assert!(tree.context.oof_content_limit_inputs.is_empty());
        assert_eq!(tree.context.oof_content_limit_prepass_depth, 0);
    }
}

#[test]
fn earlier_item_bounds_are_visible_while_the_next_item_is_measured() {
    use std::cell::Cell;
    thread_local! {
        static ITEMS: Cell<(usize, usize)> = const { Cell::new((usize::MAX, usize::MAX)) };
        static SAW_EARLIER_BOUND: Cell<bool> = const { Cell::new(false) };
    }
    fn inspect(tree: &LayoutSession<'_>, index: usize, _: usize) {
        let (first, second) = ITEMS.get();
        if index == second && tree.context.content_limit_probes.contains(&first) {
            assert!(!tree.style_view(first).min_size().width.is_auto());
            SAW_EARLIER_BOUND.set(true);
        }
    }
    let (mut doc, _, first, _) = fixture();
    let parent = doc.nodes[first].parent.unwrap();
    let second = doc.append_element(Some(parent), "div", Some("width:41px;height:9px"));
    doc.mark_in_document_flags();
    let mut cascade = raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).unwrap();
    for child in [first, second] {
        cascade.computed[child].min_width = L::MaxContent;
    }
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    ITEMS.set((first, second));
    SAW_EARLIER_BOUND.set(false);
    crate::layout::style_views::tests::with_setup_inspector(inspect, || {
        super::super::with_item_content_limits(
            &mut tree,
            NodeId::from(parent),
            super::style_view_tests::content_limit_inputs(),
            Size {
                width: Some(100.0),
                height: None,
            },
            |_, _, _| LayoutOutput::HIDDEN,
        );
    });
    assert!(SAW_EARLIER_BOUND.get());
    assert!(tree.style_view(first).min_size().width.is_auto());
    assert!(tree.style_view(second).min_size().width.is_auto());
}

#[test]
fn oof_child_scope_restores_after_dispatch_unwinds() {
    let (mut doc, mut cascade, child, _) = fixture();
    cascade.computed[child].position = raikiri_style::property::PositionValue::Absolute;
    cascade.computed[child].min_width = L::MaxContent;
    crate::layout::test_support::with_ahem(&mut doc);
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let original = tree.style_view(child).min_size();
    let mut observed = false;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        super::super::with_oof_child_content_limits(
            &mut tree,
            NodeId::from(child),
            super::style_view_tests::content_limit_inputs(),
            |tree, _, _| {
                assert!(!tree.style_view(child).min_size().width.is_auto());
                observed = true;
                panic!("OOF child dispatch unwind");
            },
        );
    }));
    assert!(result.is_err());
    assert!(observed);
    assert_eq!(tree.style_view(child).min_size(), original);
    assert_eq!(tree.override_checkpoint(), 0);
    assert!(tree.context.content_limit_probes.is_empty());
    assert_eq!(tree.context.content_limit_depth, 0);
    assert!(tree.layout_dirty);
}

#[test]
fn intrinsic_probe_restores_after_a_descendant_unwinds() {
    use std::cell::Cell;
    thread_local! {
        static ROOT: Cell<usize> = const { Cell::new(usize::MAX) };
        static OBSERVED: Cell<bool> = const { Cell::new(false) };
    }
    fn fail_in_descendant(tree: &LayoutSession<'_>, index: usize, _: usize) {
        let root = ROOT.get();
        if index != root && tree.context.content_limit_probes.contains(&root) {
            assert!(tree.style_view(root).size().width.is_auto());
            OBSERVED.set(true);
            panic!("descendant while the intrinsic probe computes");
        }
    }
    let (mut doc, _, root, _) = crate::layout::style_views::tests::fixture_with_styles(
        "display:block",
        "display:flex;width:31px;height:7px",
    );
    let child = doc.append_element(
        Some(root),
        "div",
        Some("width:23px;height:11px;min-width:max-content"),
    );
    doc.mark_in_document_flags();
    let mut cascade = raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).unwrap();
    cascade.computed[root].min_width = L::MaxContent;
    crate::layout::test_support::with_ahem(&mut doc);
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    ROOT.set(root);
    OBSERVED.set(false);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        crate::layout::style_views::tests::with_setup_inspector(fail_in_descendant, || {
            super::super::with_content_limits(
                &mut tree,
                NodeId::from(root),
                super::style_view_tests::content_limit_inputs(),
                |_, _, _| panic!("dispatch must not run"),
            );
        });
    }));
    let payload = result.expect_err("the descendant panic must propagate");
    let message = payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str));
    assert_eq!(
        message,
        Some("descendant while the intrinsic probe computes")
    );
    assert!(OBSERVED.get());
    assert_eq!(tree.override_checkpoint(), 0);
    assert_eq!(tree.style_override_depth(root), 0);
    assert_eq!(tree.style_override_depth(child), 0);
    assert_eq!(
        tree.style_view(root).size().width,
        taffy::Dimension::length(31.0)
    );
    assert_eq!(
        tree.style_view(child).size().width,
        taffy::Dimension::length(23.0)
    );
    assert!(tree.context.content_limit_probes.is_empty());
    assert!(
        tree.context
            .content_limit_percentage_height_bases
            .is_empty()
    );
    assert_eq!(tree.context.content_limit_depth, 0);
    assert!(tree.layout_dirty);
}
