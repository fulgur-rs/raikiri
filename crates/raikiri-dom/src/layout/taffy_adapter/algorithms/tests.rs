use super::*;
use crate::Document;
use crate::layout::protocol::{OverflowOrigin, RunMode};
use crate::layout::test_support::cascaded;
use raikiri_layout_style::TaffyStyle;

#[test]
fn native_overflow_scopes_restore_the_owner_direction_and_saved_children() {
    let mut doc = Document::new();
    let cascade = cascaded(&doc);
    let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
    let owner = NodeId::from(0usize);
    with_overflow_direction(&mut tree, owner, true, |tree| {
        tree.context
            .native_overflow
            .as_mut()
            .unwrap()
            .physical_children
            .insert(7, Edges::ZERO);
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            with_overflow_direction(tree, NodeId::from(1usize), false, |tree| {
                let scope = tree.context.native_overflow.as_ref().unwrap();
                assert!(!scope.reversed);
                assert_eq!(scope.owner, NodeId::from(1usize));
                assert!(scope.physical_children.is_empty());
                panic!("test unwinding a nested native algorithm");
            });
        }));
        assert!(panic.is_err());
        let scope = tree.context.native_overflow.as_ref().unwrap();
        assert!(scope.reversed);
        assert_eq!(scope.owner, owner);
        assert_eq!(scope.physical_children.get(&7), Some(&Edges::ZERO));
    });
    assert!(tree.context.native_overflow.is_none());
}

#[test]
fn a_horizontal_rtl_leaf_reports_its_legacy_right_padding_origin() {
    let mut doc = Document::new();
    let child = doc.append_element(Some(0), "div", Some("direction:rtl"));
    let cascade = cascaded(&doc);
    let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
    tree.prepare_style_views();
    let style = TaffyStyle::new(tree.query_style(child, ReadRole::CoreContainer));
    let output = compute_leaf_layout(
        LayoutInput {
            run_mode: RunMode::PerformLayout,
            known_dimensions: Size {
                width: Some(100.0),
                height: Some(100.0),
            },
            ..LayoutInput::HIDDEN
        },
        &style,
        |_, _| 0.0,
        |_, _| Size {
            width: 150.0,
            height: 100.0,
        },
    );
    assert_eq!(output.overflow_origin, OverflowOrigin::ScrollOrigin);
    assert_eq!(
        (
            output.scrollable_overflow_rect.left,
            output.scrollable_overflow_rect.right
        ),
        (0.0, 150.0)
    );
    let physical = output.physical_overflow(100.0, true);
    assert_eq!((physical.left, physical.right), (-50.0, 100.0));
}
