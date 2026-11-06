use super::*;
use crate::Document;
use crate::layout::test_support::{cascaded, with_ahem};
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
    with_ahem(&mut doc);
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
    with_ahem(&mut doc);
    let cascade = cascaded(&doc);
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let epoch = tree.layout_cache_context.epoch();
    let inputs = LayoutInput {
        run_mode: RunMode::ComputeSize,
        axis: RequestedAxis::Both,
        ..LayoutInput::HIDDEN
    };
    let reached_unwind = std::cell::Cell::new(false);
    let failed = catch_unwind(AssertUnwindSafe(|| {
        layout(&mut tree, root.into(), inputs, false, |tree, _, _| {
            assert!(item_cache_isolated(tree, item));
            assert!(item_cache_isolated(tree, replaced));
            assert!(!item_cache_isolated(tree, other));
            reached_unwind.set(true);
            panic!("isolated column phase unwind");
        });
    }));
    assert!(failed.is_err());
    assert!(reached_unwind.get());
    assert!(tree.context.column_cross_constraints.is_empty());
    assert!(!item_cache_isolated(&tree, item));
    assert_eq!(tree.override_checkpoint(), 0);
    assert_eq!(tree.layout_cache_context.epoch(), epoch);
}

#[test]
fn an_unknown_horizontal_cross_size_is_measured_without_becoming_a_percentage_basis() {
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
        Some("display:block;writing-mode:vertical-rl;flex:0 0 80px"),
    );
    doc.append_text(item, "x");
    doc.mark_in_document_flags();
    with_ahem(&mut doc);
    let cascade = cascaded(&doc);
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let inputs = LayoutInput {
        run_mode: RunMode::PerformLayout,
        known_dimensions: Size {
            width: None,
            height: Some(80.0),
        },
        known_dimensions_are_definite: Size {
            width: true,
            height: false,
        },
        ..LayoutInput::HIDDEN
    };
    let epoch = tree.layout_cache_context.epoch();
    let calls = std::cell::Cell::new(0);
    let final_size = Size {
        width: 40.0,
        height: 80.0,
    };
    assert!(super::super::orthogonal_column_paragraph(&tree, item).is_some());
    let original = constraints(&tree, root.into(), inputs, false).unwrap();
    assert!(original.supported);
    assert!(original.measure_cross);
    let output = layout(
        &mut tree,
        root.into(),
        inputs,
        false,
        |tree, node, current| {
            assert_eq!(node, root.into());
            assert!(item_cache_isolated(tree, item));
            assert_eq!(current.parent_size, inputs.parent_size);
            assert_eq!(current.known_dimensions.height, Some(80.0));
            assert!(!current.known_dimensions_are_definite.height);
            let entry = tree.context.column_cross_constraints.last().unwrap();
            assert_eq!(entry.percentage_basis, None);
            match calls.get() {
                0 => {
                    assert_eq!(current.run_mode, RunMode::ComputeSize);
                    assert_eq!(current.axis, RequestedAxis::Horizontal);
                    assert_eq!(current.known_dimensions.width, None);
                    assert!(entry.measure_cross);
                }
                1 => {
                    assert_eq!(current.run_mode, RunMode::PerformLayout);
                    assert_eq!(current.axis, inputs.axis);
                    assert_eq!(current.known_dimensions.width, Some(40.0));
                    assert!(!current.known_dimensions_are_definite.width);
                    assert!(!entry.measure_cross);
                }
                _ => panic!("cross size must be measured only once"),
            }
            calls.set(calls.get() + 1);
            LayoutOutput::from_outer_size(final_size)
        },
    );
    assert_eq!(calls.get(), 2);
    assert_eq!(output.size, final_size);
    assert!(tree.context.column_cross_constraints.is_empty());
    assert!(!item_cache_isolated(&tree, item));
    assert_eq!(tree.override_checkpoint(), 0);
    assert_eq!(tree.layout_cache_context.epoch(), epoch);
}
