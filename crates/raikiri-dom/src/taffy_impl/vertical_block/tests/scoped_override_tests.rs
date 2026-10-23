use super::super::*;
use crate::layout::style_views::tests::fixture_with_styles;
use raikiri_layout_geometry::Edges;
use raikiri_layout_style::{LengthProperty, OwnedPropertyOverride};
use raikiri_style::ComputedLengthPercentageOrAuto as L;
use taffy::Rect as StyleRect; // taffy-boundary-lint:ignore: Rect<LengthPercentage> and its relatives are style input
use taffy::{CoreStyle, Dimension, LengthPercentage};

/// A document whose child has a fixed size and a padding the frames carry.
fn fixture() -> (crate::Document, raikiri_style::CascadeResult, usize, usize) {
    fixture_with_styles("display:flex", "width:31px;height:7px;padding-left:73px")
}

fn frame(tree: &mut LayoutSession<'_>, container: usize, child: usize) -> TransposedBlock {
    TransposedBlock {
        id: next_transposed_frame_id(tree),
        container,
        right_to_left: true,
        block_space: None,
        children: vec![child],
        container_view: None,
        child_views: HashMap::default(),
        placed: Vec::new(),
        own_axes: None,
    }
}

/// The transposed padding of `child` in every pushed frame.
fn frame_paddings(tree: &LayoutSession<'_>, child: usize) -> Vec<StyleRect<LengthPercentage>> {
    tree.context
        .transposed_blocks
        .iter()
        .map(|block| block.child_views[&child].view().padding())
        .collect()
}

#[test]
fn scoped_override_updates_existing_transposed_snapshots_and_preserves_padding() {
    let (mut doc, cascade, child, _) = fixture();
    let parent = doc.nodes[child].parent.unwrap();
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let outer = frame(&mut tree, parent, child);
    let inner = frame(&mut tree, child, child);
    push_transposed_frame(&mut tree, outer);
    push_transposed_frame(&mut tree, inner);
    let paddings = frame_paddings(&tree, child);
    let zero = LengthPercentage::length(0.0);
    let unpadded = Edges {
        left: zero,
        right: zero,
        top: zero,
        bottom: zero,
    };
    assert_ne!(paddings[0], unpadded, "the fixture pads the child");
    let key = tree.nodes.box_key(child).unwrap();
    tree.with_overrides(
        vec![
            (
                key,
                OwnedPropertyOverride::length(LengthProperty::Width, L::Px(17.0), true).unwrap(),
            ),
            (key, OwnedPropertyOverride::aspect_ratio(Some(3.0))),
        ],
        |tree| {
            for block in &tree.context.transposed_blocks {
                let view = block.child_views.get(&child).unwrap().view();
                assert_eq!(view.size().height, Dimension::length(17.0));
                assert_eq!(view.size().width, Dimension::length(7.0));
                assert_eq!(view.aspect_ratio(), Some(1.0 / 3.0));
            }
            assert_eq!(frame_paddings(tree, child), paddings);
            assert_eq!(
                tree.transposed_container_view(child).unwrap().size().height,
                Dimension::length(17.0)
            );
            let detached = tree.context.transposed_blocks.pop().unwrap();
            let replacement = frame(tree, parent, parent);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                with_transposed_frame(tree, replacement, |tree| {
                    let replacement_size = tree.context.transposed_blocks[1]
                        .container_view
                        .as_ref()
                        .unwrap()
                        .view()
                        .size();
                    tree.with_overrides(
                        vec![(
                            key,
                            OwnedPropertyOverride::length(LengthProperty::Width, L::Px(23.0), true)
                                .unwrap(),
                        )],
                        |tree| {
                            assert_eq!(
                                tree.context.transposed_blocks[0].child_views[&child]
                                    .view()
                                    .size()
                                    .height,
                                Dimension::length(23.0)
                            );
                            assert_eq!(
                                tree.context.transposed_blocks[1]
                                    .container_view
                                    .as_ref()
                                    .unwrap()
                                    .view()
                                    .size(),
                                replacement_size
                            );
                            panic!(
                                "nested override while another frame occupies the detached slot"
                            );
                        },
                    );
                });
            }));
            assert!(result.is_err());
            tree.context.transposed_blocks.push(detached);
            assert_eq!(
                tree.transposed_child_view(child).unwrap().size().height,
                Dimension::length(17.0)
            );
        },
    );
    for block in &tree.context.transposed_blocks {
        let view = block.child_views.get(&child).unwrap().view();
        assert_eq!(view.size().height, Dimension::length(31.0));
        assert_eq!(view.aspect_ratio(), None);
    }
    assert_eq!(frame_paddings(&tree, child), paddings);
    assert_eq!(
        tree.transposed_container_view(child).unwrap().size().height,
        Dimension::length(31.0)
    );
}

#[test]
fn scoped_vertical_frames_restore_stack_after_nested_panic() {
    let (mut doc, cascade, child, _) = fixture();
    let parent = doc.nodes[child].parent.unwrap();
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let base = frame(&mut tree, parent, child);
    push_transposed_frame(&mut tree, base);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let outer = frame(&mut tree, child, child);
        with_transposed_frame(&mut tree, outer, |tree| {
            let inner = frame(tree, child, child);
            with_transposed_frame(tree, inner, |_| panic!("nested vertical layout"));
        });
    }));
    assert!(result.is_err());
    assert_eq!(tree.context.transposed_blocks.len(), 1);
    assert_eq!(tree.context.transposed_blocks[0].container, parent);
    assert_eq!(
        tree.context
            .transposed_style_index
            .0
            .values()
            .map(Vec::len)
            .sum::<usize>(),
        2
    );
}

#[test]
fn scoped_snapshot_restores_an_applied_setup_prefix() {
    use std::cell::Cell;
    thread_local! { static OBSERVED: Cell<bool> = const { Cell::new(false) }; }
    fn fail_after_apply(tree: &LayoutSession<'_>, index: usize, count: usize) {
        if count == 1 {
            assert_eq!(tree.style_view(index).size().width, Dimension::length(11.0));
            assert_eq!(
                tree.context.transposed_blocks[0].child_views[&index]
                    .view()
                    .size()
                    .height,
                Dimension::length(11.0)
            );
            OBSERVED.set(true);
            panic!("after applying the first snapshot override");
        }
    }
    let (mut doc, cascade, child, _) = fixture();
    let parent = doc.nodes[child].parent.unwrap();
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let block = frame(&mut tree, parent, child);
    push_transposed_frame(&mut tree, block);
    let key = tree.nodes.box_key(child).unwrap();
    OBSERVED.set(false);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        crate::layout::style_views::tests::with_setup_inspector(fail_after_apply, || {
            tree.with_overrides(
                vec![
                    (
                        key,
                        OwnedPropertyOverride::length(LengthProperty::Width, L::Px(11.0), true)
                            .unwrap(),
                    ),
                    (
                        key,
                        OwnedPropertyOverride::length(LengthProperty::Height, L::Px(19.0), true)
                            .unwrap(),
                    ),
                ],
                |_| panic!("body must not run"),
            );
        });
    }));
    assert!(result.is_err());
    assert!(OBSERVED.get());
    assert_eq!(tree.override_checkpoint(), 0);
    assert_eq!(tree.style_override_depth(child), 0);
    assert_eq!(tree.style_view(child).size().width, Dimension::length(31.0));
    assert_eq!(
        tree.transposed_child_view(child).unwrap().size().height,
        Dimension::length(31.0)
    );
}

#[test]
#[should_panic(expected = "snapshot registrations restore in reverse order")]
fn snapshot_index_rejects_out_of_order_deregistration() {
    let (mut doc, cascade, child, _) = fixture();
    let parent = doc.nodes[child].parent.unwrap();
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let outer = frame(&mut tree, parent, child);
    let inner = frame(&mut tree, child, child);
    let mut index = TransposedStyleIndex::default();
    index.register(&outer, 0);
    index.register(&inner, 1);
    index.unregister(&outer, 0);
}

#[test]
fn snapshot_apply_rejects_a_different_frame_before_changing_fields() {
    let (mut doc, cascade, child, _) = fixture();
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let mut block = frame(&mut tree, child, child);
    prepare_frame_views(&mut tree, &mut block);
    let original = block.container_view.as_ref().unwrap().view().size();
    let mut undo = TransposedSizingUndo {
        frame: 0,
        id: block.id + 1,
        child: None,
        view_previous: None,
    };
    let entry = OwnedPropertyOverride::length(LengthProperty::Width, L::Px(11.0), true).unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        undo.apply(std::slice::from_mut(&mut block), &entry)
    }));
    let payload = result.expect_err("a different frame must be rejected");
    let message = payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied());
    assert!(
        message
            .unwrap()
            .contains("scope restores the same transposed frame")
    );
    assert_eq!(
        block.container_view.as_ref().unwrap().view().size(),
        original
    );
    assert!(block.container_view.as_ref().unwrap().late.is_empty());
    assert!(undo.view_previous.is_none());
}

#[test]
fn border_scope_transposes_opposite_frames() {
    let (mut doc, cascade, child, _) = fixture();
    let parent = doc.nodes[child].parent.unwrap();
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let outer = frame(&mut tree, parent, child);
    let mut inner = frame(&mut tree, child, child);
    inner.right_to_left = false;
    push_transposed_frame(&mut tree, outer);
    push_transposed_frame(&mut tree, inner);
    let originals: Vec<_> = tree
        .context
        .transposed_blocks
        .iter()
        .map(|b| b.child_views[&child].view().border())
        .collect();
    let key = tree.nodes.box_key(child).unwrap();
    tree.with_overrides(
        vec![
            (
                key,
                OwnedPropertyOverride::length(LengthProperty::BorderTop, L::Px(11.0), true)
                    .unwrap(),
            ),
            (
                key,
                OwnedPropertyOverride::length(LengthProperty::BorderRight, L::Px(12.0), true)
                    .unwrap(),
            ),
            (
                key,
                OwnedPropertyOverride::length(LengthProperty::BorderBottom, L::Px(13.0), true)
                    .unwrap(),
            ),
            (
                key,
                OwnedPropertyOverride::length(LengthProperty::BorderLeft, L::Px(14.0), true)
                    .unwrap(),
            ),
            (key, OwnedPropertyOverride::flex_shrink(0.5)),
        ],
        |tree| {
            let physical = tree.style_view(child).border();
            assert_eq!(
                physical,
                StyleRect {
                    top: LengthPercentage::length(11.0),
                    right: LengthPercentage::length(12.0),
                    bottom: LengthPercentage::length(13.0),
                    left: LengthPercentage::length(14.0)
                }
            );
            for (block, (top, bottom)) in tree
                .context
                .transposed_blocks
                .iter()
                .zip([(12.0, 14.0), (14.0, 12.0)])
            {
                let view = block.child_views[&child].view();
                assert_eq!(
                    view.border(),
                    StyleRect {
                        left: LengthPercentage::length(11.0),
                        right: LengthPercentage::length(13.0),
                        top: LengthPercentage::length(top),
                        bottom: LengthPercentage::length(bottom)
                    }
                );
                assert_eq!(taffy::FlexboxItemStyle::flex_shrink(&view), 0.5);
            }
            assert_eq!(
                tree.context.transposed_blocks[1]
                    .container_view
                    .as_ref()
                    .unwrap()
                    .view()
                    .border(),
                crate::taffy_impl::transposed_axes::transpose_rect(physical, false)
            );
        },
    );
    assert_eq!(tree.override_checkpoint(), 0);
    for (block, original) in tree.context.transposed_blocks.iter().zip(originals) {
        let view = block.child_views[&child].view();
        assert_eq!(view.border(), original);
        assert_eq!(taffy::FlexboxItemStyle::flex_shrink(&view), 1.0);
    }
}
