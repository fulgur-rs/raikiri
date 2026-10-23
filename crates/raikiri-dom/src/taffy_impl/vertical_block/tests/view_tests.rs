use super::super::*;
use crate::layout::{
    context::LayoutSession,
    style_views::{ReadRole, tests::fixture_with_styles},
};
use taffy::Size as StyleSize; // taffy-boundary-lint:ignore: Size<Dimension> and its relatives are style input
use taffy::{CoreStyle, LengthPercentage};

fn frame(
    tree: &mut LayoutSession<'_>,
    container: usize,
    child: usize,
    right_to_left: bool,
) -> TransposedBlock {
    TransposedBlock {
        id: next_transposed_frame_id(tree),
        container,
        right_to_left,
        block_space: None,
        children: vec![child],
        container_view: None,
        child_views: HashMap::default(),
        placed: Vec::new(),
        own_axes: None,
    }
}

#[test]
fn opposite_frames_keep_independent_view_geometry() {
    let (mut doc, cascade, child, _) = fixture_with_styles(
        "display:block",
        "display:block;width:31px;height:7px;border-top:1px solid;border-right:2px solid;border-bottom:3px solid;border-left:4px solid",
    );
    let parent = doc.nodes[child].parent.unwrap();
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let outer = frame(&mut tree, parent, child, true);
    with_transposed_frame(&mut tree, outer, |tree| {
        let query = tree.query_style(child, ReadRole::BlockItem);
        assert_eq!(
            query.size(),
            StyleSize {
                width: taffy::Dimension::length(7.0),
                height: taffy::Dimension::length(31.0)
            }
        );
        assert_eq!(query.border().top, LengthPercentage::length(2.0));
        let inner = frame(tree, parent, child, false);
        with_transposed_frame(tree, inner, |tree| {
            assert_eq!(
                tree.query_style(child, ReadRole::BlockItem).border().top,
                LengthPercentage::length(4.0)
            );
            let key = tree.nodes.box_key(child).unwrap();
            tree.with_overrides(
                vec![(
                    key,
                    OwnedPropertyOverride::length(LengthProperty::BorderRight, L::Px(9.0), true)
                        .unwrap(),
                )],
                |tree| {
                    assert_eq!(
                        tree.query_style(child, ReadRole::BlockItem).border().bottom,
                        LengthPercentage::length(9.0)
                    );
                    let inner = tree.context.transposed_blocks.pop().unwrap();
                    assert_eq!(
                        tree.query_style(child, ReadRole::BlockItem).border().top,
                        LengthPercentage::length(9.0)
                    );
                    tree.context.transposed_blocks.push(inner);
                },
            );
            assert_eq!(
                tree.query_style(child, ReadRole::BlockItem).border().bottom,
                LengthPercentage::length(2.0)
            );
        });
        assert_eq!(
            tree.query_style(child, ReadRole::BlockItem).border().top,
            LengthPercentage::length(2.0)
        );
    });
    assert_eq!(
        tree.query_style(child, ReadRole::BlockItem).size().width,
        taffy::Dimension::length(31.0)
    );
}

#[test]
fn a_physical_read_ignores_the_frame_that_transposes_the_block_item_read() {
    let (mut doc, cascade, child, _) = fixture_with_styles(
        "display:block",
        "display:block;width:31px;height:7px;margin-left:5px;margin-top:9px",
    );
    let parent = doc.nodes[child].parent.unwrap();
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let physical_before = tree.physical_style(child).size();
    let block = frame(&mut tree, parent, child, true);
    with_transposed_frame(&mut tree, block, |tree| {
        assert_eq!(
            tree.query_style(child, ReadRole::BlockItem).size(),
            StyleSize {
                width: taffy::Dimension::length(7.0),
                height: taffy::Dimension::length(31.0)
            }
        );
        let physical = tree.physical_style(child);
        assert_eq!(physical.size(), physical_before);
        assert_eq!(
            physical.size(),
            StyleSize {
                width: taffy::Dimension::length(31.0),
                height: taffy::Dimension::length(7.0)
            }
        );
        assert_eq!(
            physical.margin().left,
            taffy::LengthPercentageAuto::length(5.0)
        );
        assert_eq!(
            physical.margin().top,
            taffy::LengthPercentageAuto::length(9.0)
        );
    });
}

use raikiri_layout_style::{ClaimsPolicy, LengthProperty, OwnedPropertyOverride};
use raikiri_style::ComputedLengthPercentageOrAuto as L;

#[test]
fn frame_snapshot_ignores_later_pass_and_metadata() {
    let (mut doc, cascade, child, _) =
        fixture_with_styles("display:block", "display:block;width:31px;height:7px");
    let parent = doc.nodes[child].parent.unwrap();
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let block = frame(&mut tree, parent, child, true);
    with_transposed_frame(&mut tree, block, |tree| {
        tree.record_pass_override(
            child,
            OwnedPropertyOverride::length(LengthProperty::Width, L::Px(99.0), true).unwrap(),
        );
        tree.nodes[child].claims_fixed_oof = true;
        tree.prepare_style_views();
        let live = tree.query_style(child, ReadRole::CoreContainer);
        assert_eq!(live.size().width, taffy::Dimension::length(99.0));
        assert!(live.is_containing_block().fixed);
        assert_eq!(
            tree.query_style(child, ReadRole::BlockItem).size().height,
            taffy::Dimension::length(31.0)
        );
        let frozen = tree.context.transposed_blocks.last().unwrap().child_views[&child].view();
        assert!(
            !frozen
                .with_context(raikiri_layout_style::ViewContext {
                    claims_policy: ClaimsPolicy::Prepared,
                    ..frozen.context()
                })
                .is_containing_block()
                .fixed
        );
    });
}

#[test]
fn late_override_preserves_unrelated_frame_property() {
    let (mut doc, cascade, child, _) = fixture_with_styles(
        "display:block",
        "display:block;width:31px;height:7px;padding:10%",
    );
    let parent = doc.nodes[child].parent.unwrap();
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let block = frame(&mut tree, parent, child, true);
    with_transposed_frame(&mut tree, block, |tree| {
        resolve_child_padding(tree, 100.0);
        let key = tree.nodes.box_key(child).unwrap();
        tree.with_overrides(
            vec![(
                key,
                OwnedPropertyOverride::length(LengthProperty::Width, L::Px(20.0), true).unwrap(),
            )],
            |tree| {
                assert_eq!(
                    tree.query_style(child, ReadRole::BlockItem).size().height,
                    taffy::Dimension::length(20.0)
                );
                let block = tree.context.transposed_blocks.last_mut().unwrap();
                OwnedPropertyOverride::length(LengthProperty::Height, L::Px(91.0), true)
                    .unwrap()
                    .install(&mut block.child_views.get_mut(&child).unwrap().late);
            },
        );
        let query = tree.query_style(child, ReadRole::BlockItem);
        assert_eq!(
            query.size(),
            StyleSize {
                width: taffy::Dimension::length(91.0),
                height: taffy::Dimension::length(31.0)
            }
        );
        assert_eq!(query.padding().left, LengthPercentage::length(10.0));
    });
}

#[test]
fn frame_setup_failure_restores_original_payload() {
    fn fail_after_one(tree: &LayoutSession<'_>, index: usize, count: usize) {
        if count == 1 {
            assert_eq!(
                tree.query_style(index, ReadRole::BlockItem).size().height,
                taffy::Dimension::length(11.0)
            );
            std::panic::panic_any(719_u64);
        }
    }
    let (mut doc, cascade, child, _) =
        fixture_with_styles("display:block", "display:block;width:31px;height:7px");
    let parent = doc.nodes[child].parent.unwrap();
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let block = frame(&mut tree, parent, child, true);
    let key = tree.nodes.box_key(child).unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        with_transposed_frame(&mut tree, block, |tree| {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                crate::layout::style_views::tests::with_setup_inspector(fail_after_one, || {
                    tree.with_overrides(
                        vec![
                            (
                                key,
                                OwnedPropertyOverride::length(
                                    LengthProperty::Width,
                                    L::Px(11.0),
                                    true,
                                )
                                .unwrap(),
                            ),
                            (
                                key,
                                OwnedPropertyOverride::length(
                                    LengthProperty::Height,
                                    L::Px(19.0),
                                    true,
                                )
                                .unwrap(),
                            ),
                        ],
                        |_| panic!("body must not run"),
                    );
                });
            }));
            assert_eq!(
                tree.query_style(child, ReadRole::BlockItem).size().height,
                taffy::Dimension::length(31.0)
            );
            std::panic::resume_unwind(result.unwrap_err());
        });
    }));
    assert_eq!(*result.unwrap_err().downcast::<u64>().unwrap(), 719);
    assert!(tree.context.transposed_blocks.is_empty());
    assert!(tree.context.transposed_style_index.0.is_empty());
    assert_eq!(tree.override_checkpoint(), 0);
    assert_eq!(
        tree.query_style(child, ReadRole::BlockItem).size().width,
        taffy::Dimension::length(31.0)
    );
}

#[test]
fn frame_captures_edge_basis_without_reapplying_it_to_late_borders() {
    let (mut doc, cascade, child, _) = fixture_with_styles(
        "display:block",
        "display:block;width:31px;padding:10%;border:2px solid",
    );
    let parent = doc.nodes[child].parent.unwrap();
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    tree.with_resolved_edges(child, 200.0, false, |tree| {
        let block = frame(tree, parent, child, true);
        with_transposed_frame(tree, block, |tree| {
            assert_eq!(
                tree.query_style(child, ReadRole::BlockItem).padding().left,
                LengthPercentage::length(20.0)
            );
            let key = tree.nodes.box_key(child).unwrap();
            tree.with_overrides(
                vec![(
                    key,
                    OwnedPropertyOverride::length(
                        LengthProperty::BorderRight,
                        L::Percent(10.0),
                        false,
                    )
                    .unwrap(),
                )],
                |tree| {
                    assert_eq!(
                        tree.query_style(child, ReadRole::BlockItem).border().top,
                        LengthPercentage::percent(0.1)
                    );
                },
            );
            assert_eq!(
                tree.query_style(child, ReadRole::BlockItem).border().top,
                LengthPercentage::length(2.0)
            );
        });
    });
}

#[test]
fn frame_baked_edges_preserve_duplicate_override_precedence() {
    for pass_override in [false, true] {
        let (mut doc, cascade, child, _) = fixture_with_styles(
            "display:block",
            "display:block;width:31px;height:7px;border:2px solid",
        );
        let parent = doc.nodes[child].parent.unwrap();
        let mut tree = LayoutSession::bridged(&mut doc, &cascade);
        let key = tree.nodes.box_key(child).unwrap();
        let outer =
            OwnedPropertyOverride::length(LengthProperty::BorderRight, L::Px(3.0), true).unwrap();
        let outer_scope = if pass_override {
            tree.record_pass_override(child, outer);
            Vec::new()
        } else {
            vec![(key, outer)]
        };
        tree.with_overrides(outer_scope, |tree| {
            tree.with_overrides(
                vec![(
                    key,
                    OwnedPropertyOverride::length(
                        LengthProperty::BorderRight,
                        L::Percent(10.0),
                        false,
                    )
                    .unwrap(),
                )],
                |tree| {
                    tree.with_resolved_edges(child, 200.0, true, |tree| {
                        let block = frame(tree, parent, child, true);
                        with_transposed_frame(tree, block, |tree| {
                            assert_eq!(
                                tree.query_style(child, ReadRole::BlockItem).border().top,
                                LengthPercentage::length(20.0),
                            );
                            assert_eq!(
                                tree.query_style(child, ReadRole::BlockItem).border().top,
                                LengthPercentage::length(20.0),
                            );
                        });
                    });
                    assert_eq!(
                        tree.query_style(child, ReadRole::BlockItem).border().right,
                        LengthPercentage::percent(0.1),
                    );
                },
            );
            assert_eq!(
                tree.query_style(child, ReadRole::BlockItem).border().right,
                LengthPercentage::length(3.0),
            );
        });
        assert_eq!(tree.override_checkpoint(), 0);
        assert!(tree.context.transposed_blocks.is_empty());
        assert_eq!(
            tree.query_style(child, ReadRole::BlockItem).border().right,
            LengthPercentage::length(if pass_override { 3.0 } else { 2.0 }),
        );
    }
}

#[test]
fn materialized_calc_survives_frame_and_bridge_retirement() {
    use crate::layout::taffy_adapter::MaybeResolve;
    use std::sync::Arc;
    for batched in [false, true] {
        let (mut doc, cascade, child, _) =
            fixture_with_styles("display:block", "display:block;width:31px;height:7px");
        let parent = doc.nodes[child].parent.unwrap();
        let mut tree = LayoutSession::bridged(&mut doc, &cascade);
        tree.record_pass_override(
            child,
            OwnedPropertyOverride::length(
                LengthProperty::Width,
                L::Calc(raikiri_style::property::CalcLengthPercentage {
                    px: 1.0,
                    percent: 10.0,
                }),
                false,
            )
            .unwrap(),
        );
        let handle = tree.style_view(child).size().width.into_raw().calc_value();
        let owner = Arc::downgrade(
            tree.box_style_data
                .calc_values
                .iter()
                .find(|owner| Arc::as_ptr(owner).cast::<()>() == handle)
                .unwrap(),
        );
        let block = frame(&mut tree, parent, child, true);
        let (materialized, block) = with_transposed_frame(&mut tree, block, |tree| {
            crate::taffy_impl::block_flow::style_input::child(tree, child)
        });
        drop(block);
        tree.record_pass_override(
            child,
            OwnedPropertyOverride::length(LengthProperty::Width, L::Px(99.0), true).unwrap(),
        );
        if batched {
            tree.with_style_preparation_batch(|tree| {
                crate::layout::apply_computed_to_style_in_session(tree, &cascade)
            })
            .unwrap();
        } else {
            crate::layout::apply_computed_to_style_in_session(&mut tree, &cascade).unwrap();
        }
        assert!(
            tree.box_style_data
                .calc_values
                .iter()
                .any(|owner| Arc::as_ptr(owner).cast::<()>() == handle)
        );

        let _retained = owner
            .upgrade()
            .expect("escaped frame calc survives successful bridge retirement");
        assert_eq!(
            materialized
                .size
                .height
                .maybe_resolve(Some(100.0), crate::taffy_impl::resolve_calc),
            Some(11.0)
        );
        assert_eq!(
            materialized
                .size
                .height
                .maybe_resolve(Some(200.0), crate::taffy_impl::resolve_calc),
            Some(21.0)
        );
    }
}

#[test]
fn frame_capture_keeps_publication_and_foundation_stamps() {
    let (mut doc, cascade, child, _) = fixture_with_styles(
        "display:block",
        "display:block;width:calc(1px + 10%);height:7px",
    );
    let parent = doc.nodes[child].parent.unwrap();
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let publication = tree.layout_cache_context.publication_epoch();
    let stamp = tree.box_style_data.stamp();
    let dirty = tree.document.layout_dirty;
    for _ in 0..3 {
        let block = frame(&mut tree, parent, child, true);
        let (_, block) = with_transposed_frame(&mut tree, block, |tree| {
            let _ = tree.query_style(child, ReadRole::BlockItem).size();
        });
        drop(block);
        assert_eq!(tree.layout_cache_context.publication_epoch(), publication);
        assert_eq!(tree.box_style_data.stamp(), stamp);
        assert_eq!(tree.document.layout_dirty, dirty);
    }
}

#[test]
fn materialized_late_calc_survives_scope_frame_and_bridge_retirement() {
    use crate::layout::taffy_adapter::MaybeResolve;
    use std::sync::Arc;
    for batched in [false, true] {
        let (mut doc, cascade, child, _) =
            fixture_with_styles("display:block", "display:block;width:31px;height:7px");
        let parent = doc.nodes[child].parent.unwrap();
        let mut tree = LayoutSession::bridged(&mut doc, &cascade);
        let entry = OwnedPropertyOverride::length(
            LengthProperty::Width,
            L::Calc(raikiri_style::property::CalcLengthPercentage {
                px: 1.0,
                percent: 10.0,
            }),
            false,
        )
        .unwrap();
        let owner = Arc::downgrade(entry.calc_owner().unwrap());
        let key = tree.nodes.box_key(child).unwrap();
        let block = frame(&mut tree, parent, child, true);
        let (materialized, block) = with_transposed_frame(&mut tree, block, |tree| {
            tree.with_overrides(vec![(key, entry)], |tree| {
                crate::taffy_impl::block_flow::style_input::child(tree, child)
            })
        });
        drop(block);
        if batched {
            tree.with_style_preparation_batch(|tree| {
                crate::layout::apply_computed_to_style_in_session(tree, &cascade)
            })
            .unwrap();
        } else {
            crate::layout::apply_computed_to_style_in_session(&mut tree, &cascade).unwrap();
        }
        let _retained = owner
            .upgrade()
            .expect("escaped late calc survives scope, frame and bridge retirement");
        for (basis, expected) in [(100.0, 11.0), (200.0, 21.0)] {
            assert_eq!(
                materialized
                    .size
                    .height
                    .maybe_resolve(Some(basis), crate::taffy_impl::resolve_calc),
                Some(expected)
            );
        }
    }
}

#[test]
fn frozen_sizing_undo_rejects_reused_frame_generation() {
    let (mut doc, cascade, child, _) =
        fixture_with_styles("display:block", "display:block;width:31px;height:7px");
    let parent = doc.nodes[child].parent.unwrap();
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let mut blocks = vec![frame(&mut tree, parent, child, true)];
    prepare_frame_views(&mut tree, &mut blocks[0]);
    let mut index = TransposedStyleIndex::default();
    index.register(&blocks[0], 0);
    let mut undo = TransposedSizingUndo::prepare(&blocks, &index, child)
        .pop()
        .unwrap();
    undo.apply(
        &mut blocks,
        &OwnedPropertyOverride::length(LengthProperty::Width, L::Px(20.0), true).unwrap(),
    );
    blocks[0].id += 1;
    let panic =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| undo.restore(&mut blocks)))
            .unwrap_err();
    assert!(
        panic
            .downcast_ref::<String>()
            .unwrap()
            .contains("scope restores the same transposed frame")
    );
    assert_eq!(
        blocks[0].child_views[&child].view().size().height,
        taffy::Dimension::length(20.0)
    );
}

#[test]
fn a_physical_read_of_a_vertical_box_keeps_the_physical_axes() {
    let (mut doc, cascade, child, _) = fixture_with_styles(
        "display:block",
        "display:block;writing-mode:vertical-rl;width:31px;height:7px;\
         margin-left:5px;margin-top:9px;padding-right:3px;border-bottom:2px solid",
    );
    let tree = LayoutSession::bridged(&mut doc, &cascade);
    let physical = tree.physical_style(child);
    assert_eq!(
        physical.size(),
        StyleSize {
            width: taffy::Dimension::length(31.0),
            height: taffy::Dimension::length(7.0)
        }
    );
    assert_eq!(
        physical.margin().left,
        taffy::LengthPercentageAuto::length(5.0)
    );
    assert_eq!(
        physical.margin().top,
        taffy::LengthPercentageAuto::length(9.0)
    );
    assert_eq!(physical.padding().right, LengthPercentage::length(3.0));
    assert_eq!(physical.border().bottom, LengthPercentage::length(2.0));
}
