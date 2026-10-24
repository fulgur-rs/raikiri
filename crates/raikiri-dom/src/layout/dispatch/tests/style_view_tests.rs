use crate::layout::protocol::{LayoutInput, NodeId, RunMode};
use crate::layout::{context::LayoutSession, style_views::tests::fixture_with_styles};
use raikiri_layout_geometry::Size;

#[test]
fn flex_intrinsic_probe_records_and_restores_its_temporary_limit() {
    let (mut doc, cascade, child, _) =
        fixture_with_styles("display:flex;max-width:50px", "width:10px");
    let parent = doc.nodes[child].parent.unwrap();
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let original = tree.style_view(parent).max_size();
    super::super::with_flex_intrinsic_probe(&mut tree, NodeId::from(parent), |tree| {
        assert!(tree.style_view(parent).max_size().width.is_auto());
        assert_eq!(
            tree.style_view(parent).max_size(),
            tree.style_view(parent).max_size()
        );
    });
    assert_eq!(tree.style_view(parent).max_size(), original);
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        super::super::with_flex_intrinsic_probe(&mut tree, NodeId::from(parent), |_| {
            panic!("flex probe unwind")
        });
    }));
    assert!(panic.is_err());
    assert_eq!(tree.style_view(parent).max_size(), original);
    assert!(tree.context.flex_intrinsic_probe.is_none());
}

#[test]
fn flex_basis_bounds_record_only_changed_values_and_restore_on_error_and_unwind() {
    let (mut doc, cascade, child, _) = fixture_with_styles(
        "display:flex",
        "flex:1 0 20px;width:10px;min-width:3px;max-width:30px",
    );
    let parent = doc.nodes[child].parent.unwrap();
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let original = (
        tree.style_view(child).size(),
        tree.style_view(child).min_size(),
        tree.style_view(child).max_size(),
        tree.style_view(child).flex_shrink(),
    );
    let result: Result<(), ()> =
        super::super::with_flex_item_basis_bounds(&mut tree, NodeId::from(parent), |tree| {
            let view = tree.style_view(child);
            assert_eq!(view.size().width, taffy::Dimension::length(20.0));
            assert_eq!(
                view.min_size().width,
                taffy::LengthPercentageAuto::length(20.0)
            );
            assert_eq!(view.max_size(), original.2);
            assert_eq!(view.flex_shrink(), 1.0);
            Err(())
        });
    assert!(result.is_err());
    let restored = |tree: &LayoutSession<'_>| {
        (
            tree.style_view(child).size(),
            tree.style_view(child).min_size(),
            tree.style_view(child).max_size(),
            tree.style_view(child).flex_shrink(),
        )
    };
    assert_eq!(restored(&tree), original);
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        super::super::with_flex_item_basis_bounds(&mut tree, NodeId::from(parent), |_| {
            panic!("flex bound unwind")
        });
    }));
    assert!(panic.is_err());
    assert_eq!(restored(&tree), original);
}

#[test]
fn flex_item_container_resolves_edges_during_indefinite_main_axis_measurement() {
    let (mut doc, cascade, child, _) = fixture_with_styles(
        "display:flex;width:100px",
        "display:flex;width:20px;height:10px;padding-left:10%",
    );
    crate::layout::test_support::with_ahem(&mut doc);
    let parent = doc.nodes[child].parent.unwrap();
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let padding = tree.style_view(child).padding();
    tree.context.flex_item_percent_bases.push((
        parent,
        Size {
            width: Some(100.0),
            height: Some(100.0),
        },
    ));
    let output = tree.compute_child_layout_with_block_ctx(
        NodeId::from(child),
        LayoutInput {
            run_mode: RunMode::ComputeSize,
            parent_size: Size {
                width: None,
                height: Some(100.0),
            },
            ..LayoutInput::HIDDEN
        },
        None,
    );
    assert_eq!(output.size.width, 30.0);
    assert_eq!(tree.style_view(child).padding(), padding);
    assert_eq!(tree.style_view(child).padding().left, padding.left);
}
