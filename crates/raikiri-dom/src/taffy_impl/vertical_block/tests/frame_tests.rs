use super::super::*;
use crate::layout::{context::LayoutSession, style_views::tests::fixture_with_styles};

#[test]
fn a_frame_is_set_aside_for_its_child_and_back_afterwards() {
    let (mut doc, cascade, child, _) = fixture_with_styles("display:block", "width:31px");
    let parent = doc.nodes[child].parent.unwrap();
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let frame = TransposedBlock::new(&mut tree, parent, vec![child], true);
    let id = frame.id;
    let (result, _) = with_transposed_frame(&mut tree, frame, |tree| {
        let result = tree.with_frame_set_aside(child, |tree, block| {
            assert_eq!(block.id, id);
            assert!(tree.context.transposed_blocks.is_empty());
            "ran"
        });
        assert_eq!(tree.context.transposed_blocks.len(), 1);
        assert_eq!(tree.context.transposed_blocks[0].id, id);
        result
    });
    assert_eq!(result, Some("ran"));
}

#[test]
fn a_frame_that_does_not_hold_the_node_is_left_alone() {
    let (mut doc, cascade, child, _) = fixture_with_styles("display:block", "width:31px");
    let parent = doc.nodes[child].parent.unwrap();
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    assert!(
        tree.with_frame_set_aside(child, |_, _| unreachable!("there is no frame"))
            .is_none()
    );
    let frame = TransposedBlock::new(&mut tree, parent, vec![child], false);
    with_transposed_frame(&mut tree, frame, |tree| {
        assert!(
            tree.with_frame_set_aside(parent, |_, _| unreachable!("the frame holds the child"))
                .is_none()
        );
        assert_eq!(tree.context.transposed_blocks.len(), 1);
    });
}

#[test]
fn the_frame_returns_when_the_closure_unwinds() {
    let (mut doc, cascade, child, _) = fixture_with_styles("display:block", "width:31px");
    let parent = doc.nodes[child].parent.unwrap();
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let frame = TransposedBlock::new(&mut tree, parent, vec![child], true);
    let id = frame.id;
    with_transposed_frame(&mut tree, frame, |tree| {
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            tree.with_frame_set_aside(child, |_, _| std::panic::resume_unwind(Box::new(7u8)))
        }));
        assert_eq!(*unwound.unwrap_err().downcast::<u8>().unwrap(), 7);
        assert_eq!(tree.context.transposed_blocks.len(), 1);
        assert_eq!(tree.context.transposed_blocks[0].id, id);
    });
}
