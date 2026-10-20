use crate::layout::{
    apply_computed_to_style, context::LayoutSession, style_views::tests::fixture_with_styles,
};
use taffy::{CoreStyle, FlexboxItemStyle, NodeId};

#[test]
fn flex_intrinsic_probe_records_and_restores_its_temporary_limit() {
    let (mut doc, cascade, child, _) =
        fixture_with_styles("display:flex;max-width:50px", "width:10px");
    let parent = doc.nodes[child].parent.unwrap();
    apply_computed_to_style(&mut doc, &cascade).unwrap();
    let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
    tree.prepare_style_views();
    let original = tree.style_view(parent).max_size();
    super::super::with_flex_intrinsic_probe(&mut tree, NodeId::from(parent), |tree| {
        assert!(
            tree.layout_style_for_key(parent)
                .unwrap()
                .max_size
                .width
                .is_auto()
        );
        assert_eq!(
            tree.style_view(parent).max_size(),
            tree.layout_style_for_key(parent).unwrap().max_size
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
    assert_eq!(
        tree.layout_style_for_key(parent).unwrap().max_size,
        original
    );
    assert!(tree.context.flex_intrinsic_probe.is_none());
}

#[test]
fn flex_basis_bounds_record_only_changed_values_and_restore_on_error_and_unwind() {
    let (mut doc, cascade, child, _) = fixture_with_styles(
        "display:flex",
        "flex:1 0 20px;width:10px;min-width:3px;max-width:30px",
    );
    let parent = doc.nodes[child].parent.unwrap();
    apply_computed_to_style(&mut doc, &cascade).unwrap();
    let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
    tree.prepare_style_views();
    let original = tree.layout_style_for_key(child).unwrap().clone();
    let result: Result<(), ()> =
        super::super::with_flex_item_basis_bounds(&mut tree, NodeId::from(parent), |tree| {
            let old = tree.layout_style_for_key(child).unwrap();
            let view = tree.style_view(child);
            assert_eq!(view.flex_shrink(), old.flex_shrink);
            assert_eq!(view.size(), old.size);
            assert_eq!(view.min_size(), old.min_size);
            assert_eq!(view.max_size(), old.max_size);
            Err(())
        });
    assert!(result.is_err());
    assert_eq!(tree.style_view(child).size(), original.size);
    assert_eq!(tree.style_view(child).min_size(), original.min_size);
    assert_eq!(tree.style_view(child).flex_shrink(), original.flex_shrink);
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        super::super::with_flex_item_basis_bounds(&mut tree, NodeId::from(parent), |_| {
            panic!("flex bound unwind")
        });
    }));
    assert!(panic.is_err());
    assert_eq!(tree.style_view(child).size(), original.size);
    assert_eq!(tree.style_view(child).min_size(), original.min_size);
    assert_eq!(tree.style_view(child).flex_shrink(), original.flex_shrink);
    let old = tree.layout_style_for_key(child).unwrap();
    assert_eq!(old.size, original.size);
    assert_eq!(old.min_size, original.min_size);
    assert_eq!(old.flex_shrink, original.flex_shrink);
}
