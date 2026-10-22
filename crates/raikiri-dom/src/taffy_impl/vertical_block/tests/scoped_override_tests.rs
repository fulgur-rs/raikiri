use super::super::*;
use crate::layout::style_views::tests::fixture;
use raikiri_layout_style::{LengthProperty, OwnedPropertyOverride};
use raikiri_style::ComputedLengthPercentageOrAuto as L;
use taffy::Dimension;

fn frame(tree: &mut LayoutSession<'_>, container: usize, child: usize) -> TransposedBlock {
    let mut style = transpose_style(tree.layout_style_for_key(child).unwrap(), true);
    style.padding.left = LengthPercentage::length(73.0);
    TransposedBlock {
        id: next_transposed_frame_id(tree),
        container,
        right_to_left: true,
        block_space: None,
        container_style: transpose_style(tree.layout_style_for_key(container).unwrap(), true),
        child_styles: HashMap::from([(child, style)]),
        container_view: None,
        child_views: HashMap::default(),
        placed: Vec::new(),
    }
}

#[test]
fn scoped_override_updates_existing_transposed_snapshots_and_preserves_padding() {
    let (mut doc, cascade, child, _) = fixture();
    crate::layout::apply_computed_to_style(&mut doc, &cascade).unwrap();
    let parent = doc.nodes[child].parent.unwrap();
    let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
    tree.prepare_style_views();
    let outer = frame(&mut tree, parent, child);
    let inner = frame(&mut tree, child, child);
    push_transposed_frame(&mut tree, outer);
    push_transposed_frame(&mut tree, inner);
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
                let style = block.child_styles.get(&child).unwrap();
                assert_eq!(style.size.height, Dimension::length(17.0));
                assert_eq!(style.size.width, Dimension::length(7.0));
                assert_eq!(style.aspect_ratio, Some(1.0 / 3.0));
                assert_eq!(style.padding.left, LengthPercentage::length(73.0));
            }
            assert_eq!(
                tree.transposed_container_style(child).unwrap().size.height,
                Dimension::length(17.0)
            );
            let detached = tree.context.transposed_blocks.pop().unwrap();
            let replacement = frame(tree, parent, parent);
            let replacement_size = replacement.container_style.size;
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                with_transposed_frame(tree, replacement, |tree| {
                    tree.with_overrides(
                        vec![(
                            key,
                            OwnedPropertyOverride::length(LengthProperty::Width, L::Px(23.0), true)
                                .unwrap(),
                        )],
                        |tree| {
                            assert_eq!(
                                tree.context.transposed_blocks[0].child_styles[&child]
                                    .size
                                    .height,
                                Dimension::length(23.0)
                            );
                            assert_eq!(
                                tree.context.transposed_blocks[1].container_style.size,
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
                tree.transposed_child_style(child).unwrap().size.height,
                Dimension::length(17.0)
            );
        },
    );
    for block in &tree.context.transposed_blocks {
        let style = block.child_styles.get(&child).unwrap();
        assert_eq!(style.size.height, Dimension::length(31.0));
        assert_eq!(style.aspect_ratio, None);
        assert_eq!(style.padding.left, LengthPercentage::length(73.0));
    }
    assert_eq!(
        tree.transposed_container_style(child).unwrap().size.height,
        Dimension::length(31.0)
    );
}

#[test]
fn scoped_vertical_frames_restore_stack_after_nested_panic() {
    let (mut doc, cascade, child, _) = fixture();
    crate::layout::apply_computed_to_style(&mut doc, &cascade).unwrap();
    let parent = doc.nodes[child].parent.unwrap();
    let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
    tree.prepare_style_views();
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
            assert_eq!(
                tree.layout_style_for_key(index).unwrap().size.width,
                Dimension::length(11.0)
            );
            assert_eq!(
                tree.context.transposed_blocks[0].child_styles[&index]
                    .size
                    .height,
                Dimension::length(11.0)
            );
            OBSERVED.set(true);
            panic!("after applying the first snapshot override");
        }
    }
    let (mut doc, cascade, child, _) = fixture();
    crate::layout::apply_computed_to_style(&mut doc, &cascade).unwrap();
    let parent = doc.nodes[child].parent.unwrap();
    let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
    tree.prepare_style_views();
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
    assert_eq!(
        tree.layout_style_for_key(child).unwrap().size.width,
        Dimension::length(31.0)
    );
    assert_eq!(
        tree.transposed_child_style(child).unwrap().size.height,
        Dimension::length(31.0)
    );
    assert_eq!(
        tree.transposed_child_style(child).unwrap().padding.left,
        LengthPercentage::length(73.0)
    );
}

#[test]
#[should_panic(expected = "snapshot registrations restore in reverse order")]
fn snapshot_index_rejects_out_of_order_deregistration() {
    let (mut doc, cascade, child, _) = fixture();
    crate::layout::apply_computed_to_style(&mut doc, &cascade).unwrap();
    let parent = doc.nodes[child].parent.unwrap();
    let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
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
    crate::layout::apply_computed_to_style(&mut doc, &cascade).unwrap();
    let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
    let mut block = frame(&mut tree, child, child);
    let original = block.container_style.size;
    let mut undo = TransposedSizingUndo {
        frame: 0,
        id: block.id + 1,
        child: None,
        previous: None,
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
    assert_eq!(block.container_style.size, original);
    assert!(undo.previous.is_none());
}

#[test]
fn border_scope_transposes_opposite_frames() {
    let (mut doc, cascade, child, _) = fixture();
    crate::layout::apply_computed_to_style(&mut doc, &cascade).unwrap();
    let parent = doc.nodes[child].parent.unwrap();
    let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
    tree.prepare_style_views();
    let outer = frame(&mut tree, parent, child);
    let mut inner = frame(&mut tree, child, child);
    inner.right_to_left = false;
    inner.container_style = transpose_style(tree.layout_style_for_key(child).unwrap(), false);
    inner.child_styles.insert(
        child,
        transpose_style(tree.layout_style_for_key(child).unwrap(), false),
    );
    push_transposed_frame(&mut tree, outer);
    push_transposed_frame(&mut tree, inner);
    let originals: Vec<_> = tree
        .context
        .transposed_blocks
        .iter()
        .map(|b| b.child_styles[&child].border)
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
            let physical = tree.layout_style_for_key(child).unwrap().border;
            assert_eq!(
                physical,
                Rect {
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
                assert_eq!(
                    block.child_styles[&child].border,
                    Rect {
                        left: LengthPercentage::length(11.0),
                        right: LengthPercentage::length(13.0),
                        top: LengthPercentage::length(top),
                        bottom: LengthPercentage::length(bottom)
                    }
                );
                assert_eq!(block.child_styles[&child].flex_shrink, 0.5);
            }
            assert_eq!(
                tree.context.transposed_blocks[1].container_style.border,
                transpose_rect(physical, false)
            );
        },
    );
    assert_eq!(tree.override_checkpoint(), 0);
    for (block, original) in tree.context.transposed_blocks.iter().zip(originals) {
        assert_eq!(block.child_styles[&child].border, original);
        assert_eq!(block.child_styles[&child].flex_shrink, 1.0);
    }
}
