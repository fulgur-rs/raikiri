use super::*;
use crate::Document;
use crate::layout::test_support::cascaded;
use raikiri_layout_geometry::Size;

fn scoped<R>(
    tree: &mut LayoutSession<'_>,
    node: NodeId,
    inputs: LayoutInput,
    definite: bool,
    f: impl FnOnce(&mut LayoutSession<'_>) -> R,
) -> R {
    let entry = constraints(tree, node, inputs, definite).unwrap();
    with_entry(tree, entry, f)
}

#[test]
fn nested_cross_constraints_restore_the_outer_owner_after_an_unwind() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let outer = doc.append_element(
        Some(html),
        "div",
        Some("display:flex;flex-direction:column;max-height:100px"),
    );
    let inner = doc.append_element(
        Some(outer),
        "div",
        Some("display:flex;flex-direction:column;max-height:40px"),
    );
    let leaf = doc.append_element(Some(inner), "div", Some("display:block;margin:5px"));
    doc.mark_in_document_flags();
    let cascade = cascaded(&doc);
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let inputs = LayoutInput {
        parent_size: Size {
            width: Some(800.0),
            height: None,
        },
        ..LayoutInput::HIDDEN
    };
    let measure = LayoutInput {
        parent_size: Size {
            width: Some(70.0),
            height: Some(60.0),
        },
        available_space: Size {
            width: AvailableSpace::MinContent,
            height: AvailableSpace::Definite(60.0),
        },
        ..inputs
    };
    let epoch = tree.layout_cache_context.epoch();
    let value = scoped(&mut tree, outer.into(), inputs, false, |tree| {
        let mut constrained = measure;
        constrain_inputs(tree, inner, outer, false, &mut constrained);
        assert_eq!(
            constrained.parent_size,
            Size {
                width: Some(70.0),
                height: None
            }
        );
        assert_eq!(
            constrained.available_space.height,
            AvailableSpace::Definite(100.0)
        );
        assert_eq!(
            constrained.available_space.width,
            AvailableSpace::MinContent
        );
        let failed = catch_unwind(AssertUnwindSafe(|| {
            scoped(tree, inner.into(), inputs, false, |tree| {
                let mut constrained = measure;
                constrain_inputs(tree, leaf, inner, false, &mut constrained);
                assert_eq!(constrained.parent_size.height, None);
                assert_eq!(
                    constrained.available_space.height,
                    AvailableSpace::Definite(30.0)
                );
                panic!("nested cross measurement unwind");
            });
        }));
        assert!(failed.is_err());
        assert_eq!(tree.context.column_cross_constraints.len(), 1);
        let mut constrained = measure;
        constrain_inputs(tree, inner, outer, false, &mut constrained);
        assert_eq!(
            constrained.available_space.height,
            AvailableSpace::Definite(100.0)
        );
        let mut unrelated = measure;
        constrain_inputs(tree, leaf, inner, false, &mut unrelated);
        assert_eq!(unrelated.parent_size, measure.parent_size);
        assert_eq!(unrelated.available_space.height, AvailableSpace::MaxContent);
        42
    });
    assert_eq!(value, 42);
    assert!(tree.context.column_cross_constraints.is_empty());
    assert_eq!(tree.layout_cache_context.epoch(), epoch);
}

#[test]
fn a_phase_only_exposes_the_saved_child_caches_and_clears_them_on_unwind() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let root = doc.append_element(
        Some(html),
        "div",
        Some("display:inline-flex;flex-direction:column"),
    );
    let item = doc.append_element(
        Some(root),
        "div",
        Some("display:block;writing-mode:vertical-rl;padding:10%"),
    );
    doc.append_text(item, "x");
    let other = doc.append_element(Some(html), "div", Some("display:block"));
    doc.append_text(other, "x");
    let replaced = doc.append_element(
        Some(root),
        "img",
        Some("display:block;width:15px;height:20px;padding:5%"),
    );
    doc.mark_in_document_flags();
    let cascade = cascaded(&doc);
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let epoch = tree.layout_cache_context.epoch();
    let inputs = LayoutInput {
        run_mode: RunMode::ComputeSize,
        axis: RequestedAxis::Both,
        ..LayoutInput::HIDDEN
    };
    let failed = catch_unwind(AssertUnwindSafe(|| {
        layout(&mut tree, root.into(), inputs, false, |tree, _, _| {
            assert!(item_cache_isolated(tree, item));
            assert!(item_cache_isolated(tree, replaced));
            assert!(!item_cache_isolated(tree, other));
            panic!("isolated column phase unwind");
        });
    }));
    assert!(failed.is_err());
    assert!(tree.context.column_cross_constraints.is_empty());
    assert!(!item_cache_isolated(&tree, item));
    assert_eq!(tree.override_checkpoint(), 0);
    assert_eq!(tree.layout_cache_context.epoch(), epoch);
}
