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
        block_space: None,
        container_style: transpose_style(tree.layout_style_for_key(container).unwrap(), true),
        child_styles: HashMap::from([(child, style)]),
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
